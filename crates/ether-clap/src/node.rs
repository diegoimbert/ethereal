//! Audio-thread half of an in-process CLAP plugin ([`ClapNode`]).
//!
//! Everything is allocated in [`ClapNode::new`] (main thread). `process` converts engine
//! events to CLAP events in a pre-allocated buffer, copies the main input into the plugin's
//! port buffers, calls the plugin, copies the main output back, and forwards the plugin's
//! output events: note/MIDI events to `ctx.out_events`, parameter values and gestures
//! (e.g. from the plugin GUI) to the controller through a lock-free ring.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use clack_host::events::event_types::{
    MidiEvent, NoteChokeEvent, NoteOffEvent, NoteOnEvent, ParamValueEvent, TransportEvent,
    TransportFlags,
};
use clack_host::events::io::{OutputEventBuffer, TryPushError};
use clack_host::events::spaces::CoreEventSpace;
use clack_host::events::{EventFlags, EventHeader, Match};
use clack_host::prelude::*;
use clack_host::utils::{BeatTime, SecondsTime};
use ether_core::buffer::AudioBuffers;
use ether_core::config::PrepareConfig;
use ether_core::event::{EventKind, ProcessEvent};
use ether_core::node::{Device, Node, ProcessContext, ProcessStatus};
use ether_core::plugin::PluginNode;
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::ParamId;
use ether_core::transport::TransportInfo;

use crate::host::EtherHost;

/// Messages from the audio thread to the controller.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum NodeMsg {
    Param { id: u32, value: f64 },
    GestureBegin(u32),
    GestureEnd(u32),
}

/// State shared between the node and its controller.
#[derive(Debug, Default)]
pub(crate) struct NodeShared {
    pub faulted: AtomicBool,
    pub latency: AtomicU32,
}

/// Audio port channel counts (from the audio-ports extension).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PortLayout {
    pub inputs: Vec<u32>,
    pub outputs: Vec<u32>,
    pub main_in: Option<usize>,
    pub main_out: Option<usize>,
    /// The sidechain port (CONTRACTS §12.14): the first input port that isn't the main one
    /// (`CLAP_PORT_IS_MAIN` unset) and has channels.
    pub aux_in: Option<usize>,
}

impl PortLayout {
    /// The sidechain port among `inputs` (see [`PortLayout::aux_in`]).
    pub fn find_aux(inputs: &[u32], main_in: Option<usize>) -> Option<usize> {
        (0..inputs.len()).find(|&i| Some(i) != main_in && inputs[i] > 0)
    }

    /// Channels the engine feeds the sidechain port with: its channel count capped at 2
    /// (the engine's sidechain is stereo). 0 = no sidechain.
    pub fn sidechain_channels(&self) -> u16 {
        self.aux_in
            .and_then(|i| self.inputs.get(i))
            .map_or(0, |ch| (*ch).min(2) as u16)
    }

    pub fn main_channels(&self) -> (u16, u16) {
        let ch = |ports: &[u32], main: Option<usize>| {
            main.and_then(|i| ports.get(i)).copied().unwrap_or(0) as u16
        };
        (
            ch(&self.inputs, self.main_in),
            ch(&self.outputs, self.main_out),
        )
    }
}

/// Output event buffer that refuses events instead of growing (no allocation on the audio
/// thread). Capacity was reserved up-front.
struct BoundedEvents {
    inner: EventBuffer,
    cap: usize,
}

impl OutputEventBuffer for BoundedEvents {
    fn try_push(&mut self, event: &UnknownEvent) -> Result<(), TryPushError> {
        if self.inner.len() as usize >= self.cap
            || event.header().size() as usize > std::mem::size_of::<TransportEvent>()
        {
            return Err(TryPushError::new());
        }
        self.inner.push(event);
        Ok(())
    }
}

pub struct ClapNode {
    processor: Option<PluginAudioProcessor<EtherHost>>,
    shared: Arc<NodeShared>,
    to_main: rtrb::Producer<NodeMsg>,
    descriptor: DeviceDescriptor,
    layout: PortLayout,
    max_frames: usize,
    in_bufs: Vec<Vec<f32>>,
    out_bufs: Vec<Vec<f32>>,
    in_ports: AudioPorts,
    out_ports: AudioPorts,
    in_events: EventBuffer,
    in_cap: usize,
    out_events: BoundedEvents,
    /// Current plain values, sorted by id.
    values: Vec<(u32, f64)>,
    /// `Device::set_param` calls not yet sent to the plugin.
    pending: Vec<(u32, f64)>,
    steady: u64,
}

pub(crate) struct NodeInit {
    pub processor: StoppedPluginAudioProcessor<EtherHost>,
    pub shared: Arc<NodeShared>,
    pub to_main: rtrb::Producer<NodeMsg>,
    pub descriptor: DeviceDescriptor,
    pub layout: PortLayout,
    pub max_frames: usize,
    pub max_events: usize,
    pub values: Vec<(u32, f64)>,
}

impl ClapNode {
    pub(crate) fn new(init: NodeInit) -> Self {
        let NodeInit {
            processor,
            shared,
            to_main,
            descriptor,
            layout,
            max_frames,
            max_events,
            mut values,
        } = init;
        values.sort_by_key(|(id, _)| *id);
        let alloc = |ports: &[u32]| -> Vec<Vec<f32>> {
            ports
                .iter()
                .map(|ch| vec![0.0; *ch as usize * max_frames])
                .collect()
        };
        let total = |ports: &[u32]| ports.iter().map(|c| *c as usize).sum::<usize>();
        let pending_cap = values.len().max(16);
        // Pending sets + engine events + a transport margin.
        let in_cap = max_events + pending_cap + 16;
        let out_cap = max_events.max(256);
        Self {
            processor: Some(processor.into()),
            shared,
            to_main,
            descriptor,
            in_bufs: alloc(&layout.inputs),
            out_bufs: alloc(&layout.outputs),
            in_ports: AudioPorts::with_capacity(total(&layout.inputs), layout.inputs.len()),
            out_ports: AudioPorts::with_capacity(total(&layout.outputs), layout.outputs.len()),
            layout,
            max_frames,
            in_events: EventBuffer::with_capacity(in_cap),
            in_cap,
            out_events: BoundedEvents {
                inner: EventBuffer::with_capacity(out_cap),
                cap: out_cap,
            },
            values,
            pending: Vec::with_capacity(pending_cap),
            steady: 0,
        }
    }

    fn fault(&mut self, audio: &mut AudioBuffers<'_, '_>) -> ProcessStatus {
        self.shared.faulted.store(true, Ordering::Release);
        audio.clear_outputs();
        ProcessStatus::Silent
    }

    fn set_value(&mut self, id: u32, value: f64) {
        if let Ok(i) = self.values.binary_search_by_key(&id, |(k, _)| *k) {
            self.values[i].1 = value;
        }
    }

    fn push_in(&mut self, event: &impl AsRef<UnknownEvent>) {
        if (self.in_events.len() as usize) < self.in_cap {
            self.in_events.push(event);
        }
    }

    fn convert_event(&mut self, e: &ProcessEvent) {
        let t = e.offset;
        match e.kind {
            EventKind::NoteOn {
                note_id,
                channel,
                key,
                velocity,
            } => self.push_in(&NoteOnEvent::new(
                t,
                Pckn::new(0u16, u16::from(channel), u16::from(key), note_id),
                f64::from(velocity),
            )),
            EventKind::NoteOff {
                note_id,
                channel,
                key,
                velocity,
            } => self.push_in(&NoteOffEvent::new(
                t,
                Pckn::new(0u16, u16::from(channel), u16::from(key), note_id),
                f64::from(velocity),
            )),
            EventKind::NoteChoke {
                note_id,
                channel,
                key,
            } => self.push_in(&NoteChokeEvent::new(
                t,
                Pckn::new(0u16, u16::from(channel), u16::from(key), note_id),
            )),
            EventKind::AllNotesOff => {
                // Wildcard note-off (CLAP dialect) + CC 123 "all notes off" (MIDI dialect).
                self.push_in(&NoteOffEvent::new(t, Pckn::match_all(), 0.0));
                self.push_in(&MidiEvent::new(t, 0, [0xB0, 123, 0]));
            }
            EventKind::Param { param, value } => {
                self.set_value(param.0, value);
                self.push_in(&ParamValueEvent::new(
                    t,
                    ClapId::new(param.0),
                    Pckn::match_all(),
                    value,
                ));
            }
            EventKind::Midi { data } => self.push_in(&MidiEvent::new(t, 0, data)),
            // v0.3: `midi-expression`/`mpe` forward this as `clap_event_note_expression`.
            EventKind::NoteExpression { .. } => {}
        }
    }
}

fn pckn_parts(p: Pckn) -> (u32, u8, u8) {
    let note_id = match p.note_id {
        Match::Specific(n) => n,
        Match::All => 0,
    };
    let channel = match p.channel {
        Match::Specific(c) => c.min(15) as u8,
        Match::All => 0,
    };
    let key = match p.key {
        Match::Specific(k) => k.min(127) as u8,
        Match::All => 0,
    };
    (note_id, channel, key)
}

pub(crate) fn transport_event(t: &TransportInfo) -> TransportEvent {
    let mut flags = TransportFlags::HAS_TEMPO
        | TransportFlags::HAS_BEATS_TIMELINE
        | TransportFlags::HAS_SECONDS_TIMELINE
        | TransportFlags::HAS_TIME_SIGNATURE;
    if t.playing {
        flags |= TransportFlags::IS_PLAYING;
    }
    if t.recording {
        flags |= TransportFlags::IS_RECORDING;
    }
    if t.loop_active {
        flags |= TransportFlags::IS_LOOP_ACTIVE;
    }
    let sig = t.time_signature;
    let beats_per_bar = f64::from(sig.numerator.max(1)) * 4.0 / f64::from(sig.denominator.max(1));
    TransportEvent {
        header: EventHeader::new_core(0, EventFlags::empty()),
        flags,
        song_pos_beats: BeatTime::from_float(t.position),
        song_pos_seconds: SecondsTime::from_float(t.seconds),
        tempo: t.bpm,
        tempo_inc: 0.0,
        loop_start_beats: BeatTime::from_float(t.loop_start),
        loop_end_beats: BeatTime::from_float(t.loop_end),
        loop_start_seconds: SecondsTime::from_float(0.0),
        loop_end_seconds: SecondsTime::from_float(0.0),
        bar_start: BeatTime::from_float(t.bar_start),
        bar_number: (t.bar_start / beats_per_bar).round() as i32,
        time_signature_numerator: u16::from(sig.numerator),
        time_signature_denominator: u16::from(sig.denominator),
    }
}

impl Node for ClapNode {
    fn prepare(&mut self, config: &PrepareConfig) {
        // The plugin was activated for `max_frames`; the engine uses the same EngineConfig
        // for both, so a mismatch is a host bug. Larger blocks are rejected in `process`.
        if config.max_block_size > self.max_frames {
            tracing::warn!(
                "CLAP node prepared for {} frames but activated for {}",
                config.max_block_size,
                self.max_frames
            );
        }
    }

    fn reset(&mut self) {
        if let Some(p) = self.processor.as_mut() {
            p.reset();
        }
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        self.run(ctx, audio, &[])
    }

    fn sidechain_inputs(&self) -> u16 {
        self.layout.sidechain_channels()
    }

    fn process_sidechain(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        sidechain: &[&[f32]],
    ) -> ProcessStatus {
        self.run(ctx, audio, sidechain)
    }

    fn latency(&self) -> u32 {
        self.shared.latency.load(Ordering::Relaxed)
    }

    fn channels(&self) -> (u16, u16) {
        self.layout.main_channels()
    }
}

/// The source of channel `c` of a sidechain port from the engine's sidechain `sc`
/// (CONTRACTS §12.14): mono ports get the left channel, wider ones the first two (a mono
/// source feeds both), extra channels are silent. `None` = silence.
pub(crate) fn sidechain_source<'a>(sc: &[&'a [f32]], c: usize) -> Option<&'a [f32]> {
    match c {
        0 => sc.first().copied(),
        1 => sc.get(1).or(sc.first()).copied(),
        _ => None,
    }
}

impl ClapNode {
    /// `process` (no sidechain source: `sidechain` is empty, the aux port gets silence) and
    /// `process_sidechain`.
    fn run(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        sidechain: &[&[f32]],
    ) -> ProcessStatus {
        let frames = ctx.frames;
        if self.shared.faulted.load(Ordering::Relaxed) || frames > self.max_frames {
            audio.clear_outputs();
            return ProcessStatus::Silent;
        }
        if frames == 0 {
            return ProcessStatus::Continue;
        }

        // Input events: pending immediate sets (offset 0), then the engine's sorted events.
        self.in_events.clear();
        for i in 0..self.pending.len() {
            let (id, value) = self.pending[i];
            self.push_in(&ParamValueEvent::new(
                0,
                ClapId::new(id),
                Pckn::match_all(),
                value,
            ));
        }
        self.pending.clear();
        for e in ctx.events {
            self.convert_event(e);
        }
        let transport = transport_event(ctx.transport);

        // Main input → plugin's main input port (mono input feeds every channel); the
        // sidechain → the aux port; other ports get silence.
        let max = self.max_frames;
        for (p, buf) in self.in_bufs.iter_mut().enumerate() {
            let is_main = self.layout.main_in == Some(p);
            let is_aux = self.layout.aux_in == Some(p);
            for (c, dst) in buf.chunks_exact_mut(max).enumerate() {
                let dst = &mut dst[..frames];
                let src = if is_main {
                    audio.inputs.get(c).or(audio.inputs.last()).copied()
                } else if is_aux {
                    sidechain_source(sidechain, c)
                } else {
                    None
                };
                match src {
                    Some(src) => dst.copy_from_slice(&src[..frames]),
                    None => dst.fill(0.0),
                }
            }
        }

        let Some(processor) = self.processor.as_mut() else {
            return self.fault(audio);
        };
        let Ok(started) = processor.ensure_processing_started() else {
            return self.fault(audio);
        };

        self.out_events.inner.clear();
        let result = {
            let ins = self
                .in_ports
                .with_input_buffers(self.in_bufs.iter_mut().map(|buf| AudioPortBuffer {
                    latency: 0,
                    channels: AudioPortBufferType::f32_input_only(buf.chunks_exact_mut(max).map(
                        |b| InputChannel {
                            buffer: &mut b[..frames],
                            is_constant: false,
                        },
                    )),
                }));
            let mut outs = self
                .out_ports
                .with_output_buffers(self.out_bufs.iter_mut().map(|buf| AudioPortBuffer {
                    latency: 0,
                    channels: AudioPortBufferType::f32_output_only(
                        buf.chunks_exact_mut(max).map(|b| &mut b[..frames]),
                    ),
                }));
            let input_events = InputEvents::from_buffer(&self.in_events);
            let mut output_events = OutputEvents::from_buffer(&mut self.out_events);
            started.process(
                &ins,
                &mut outs,
                &input_events,
                &mut output_events,
                Some(self.steady),
                Some(&transport),
            )
        };
        self.steady = self.steady.wrapping_add(frames as u64);

        let status = match result {
            Ok(status) => status,
            Err(_) => return self.fault(audio),
        };

        // Plugin's main output → node outputs (mono output feeds every channel).
        match self.layout.main_out {
            Some(p) if self.layout.outputs[p] > 0 => {
                let ch = self.layout.outputs[p] as usize;
                let src = &self.out_bufs[p];
                for (c, dst) in audio.outputs.iter_mut().enumerate() {
                    let c = c.min(ch - 1);
                    dst[..frames].copy_from_slice(&src[c * max..c * max + frames]);
                }
            }
            _ => audio.clear_outputs(),
        }

        // Plugin output events.
        let last = (frames - 1) as u32;
        for i in 0..self.out_events.inner.len() {
            let Some(event) = self.out_events.inner.get(i) else {
                continue;
            };
            let offset = event.header().time().min(last);
            let out = |kind| ProcessEvent { offset, kind };
            match event.as_core_event() {
                Some(CoreEventSpace::ParamValue(e)) => {
                    if let Some(id) = e.param_id() {
                        let (id, value) = (id.get(), e.value());
                        if let Ok(i) = self.values.binary_search_by_key(&id, |(k, _)| *k) {
                            self.values[i].1 = value;
                        }
                        let _ = self.to_main.push(NodeMsg::Param { id, value });
                    }
                }
                Some(CoreEventSpace::ParamGestureBegin(e)) => {
                    if let Some(id) = e.param_id() {
                        let _ = self.to_main.push(NodeMsg::GestureBegin(id.get()));
                    }
                }
                Some(CoreEventSpace::ParamGestureEnd(e)) => {
                    if let Some(id) = e.param_id() {
                        let _ = self.to_main.push(NodeMsg::GestureEnd(id.get()));
                    }
                }
                Some(CoreEventSpace::NoteOn(e)) => {
                    let (note_id, channel, key) = pckn_parts(e.pckn());
                    ctx.out_events.push(out(EventKind::NoteOn {
                        note_id,
                        channel,
                        key,
                        velocity: e.velocity() as f32,
                    }));
                }
                Some(CoreEventSpace::NoteOff(e)) => {
                    let (note_id, channel, key) = pckn_parts(e.pckn());
                    ctx.out_events.push(out(EventKind::NoteOff {
                        note_id,
                        channel,
                        key,
                        velocity: e.velocity() as f32,
                    }));
                }
                Some(CoreEventSpace::NoteChoke(e)) => {
                    let (note_id, channel, key) = pckn_parts(e.pckn());
                    ctx.out_events.push(out(EventKind::NoteChoke {
                        note_id,
                        channel,
                        key,
                    }));
                }
                Some(CoreEventSpace::Midi(e)) => {
                    ctx.out_events.push(out(EventKind::Midi { data: e.data() }));
                }
                _ => {}
            }
        }

        match status {
            clack_host::process::ProcessStatus::Sleep => ProcessStatus::Silent,
            _ => ProcessStatus::Continue,
        }
    }
}

impl Device for ClapNode {
    fn descriptor(&self) -> DeviceDescriptor {
        self.descriptor.clone()
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.values
            .binary_search_by_key(&id.0, |(k, _)| *k)
            .ok()
            .map(|i| self.values[i].1)
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.set_value(id.0, value);
        if self.pending.len() < self.pending.capacity() {
            self.pending.push((id.0, value));
        }
    }
}

impl PluginNode for ClapNode {
    fn is_faulted(&self) -> bool {
        self.shared.faulted.load(Ordering::Relaxed)
    }
}

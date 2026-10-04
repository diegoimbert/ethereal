//! Audio-thread half of an in-process VST2 plugin ([`Vst2Node`]).
//!
//! Everything is allocated in [`Vst2Node::new`] (main thread): one planar buffer per plugin
//! channel (and `f64` copies for `processDoubleReplacing`), the channel-pointer arrays, the
//! `VstMidiEvent`s with their `VstEvents` list, the `VstTimeInfo` and the MIDI-out buffer.
//!
//! # Sample accuracy
//! - MIDI (notes, raw MIDI, MPE expression as MIDI) goes through `effProcessEvents` with
//!   `deltaFrames` = the event's offset in the (sub-)block.
//! - VST2 has no timed parameter changes (`setParameter` applies "now"), so the block is
//!   split at param events: each sub-block starts with its params set, then its MIDI, then
//!   `processReplacing`. Param events closer than [`MIN_SPLIT`] samples to the current
//!   sub-block start are applied at that start instead of splitting again (plugins dislike
//!   tiny blocks).
//! - `audioMasterGetTime` during a sub-block reports that sub-block's position.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use ether_core::buffer::AudioBuffers;
use ether_core::config::PrepareConfig;
use ether_core::event::{EventKind, ProcessEvent};
use ether_core::expression::mpe::MpeOut;
use ether_core::node::{Device, Node, ProcessContext, ProcessStatus};
use ether_core::plugin::PluginNode;
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::ParamId;
use ether_core::transport::TransportInfo;

use crate::abi::*;
use crate::host::{AudioScope, MidiOut};
use crate::module::Effect;

/// MIDI events per sub-block (extra ones are dropped).
pub(crate) const MAX_EVENTS: usize = 1024;
/// Smallest sub-block a param event splits off (see the module docs).
pub(crate) const MIN_SPLIT: u32 = 16;

/// How the node calls the plugin's audio processing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Precision {
    /// `processReplacing` (32-bit float).
    Single,
    /// `processDoubleReplacing`, for plugins that only do 64-bit (converted from/to f32).
    Double,
    /// The deprecated accumulating `process` (outputs cleared first).
    Accumulate,
}

impl Precision {
    /// The best mode the plugin offers. The engine is f32: `processReplacing` is preferred
    /// even when the plugin can also do double.
    pub(crate) fn choose(effect: &Effect) -> Option<Self> {
        if effect.has_flag(effFlagsCanReplacing) && effect.process_replacing().is_some() {
            Some(Self::Single)
        } else if effect.has_flag(effFlagsCanDoubleReplacing)
            && effect.process_double_replacing().is_some()
        {
            Some(Self::Double)
        } else if effect.process_replacing().is_some() {
            // Some plugins forget the flag but set the pointer.
            Some(Self::Single)
        } else {
            effect.process_accumulating().map(|_| Self::Accumulate)
        }
    }
}

/// State shared between the node and its controller.
#[derive(Debug, Default)]
pub(crate) struct NodeShared {
    pub latency: AtomicU32,
}

pub struct Vst2Node {
    effect: Arc<Effect>,
    shared: Arc<NodeShared>,
    descriptor: DeviceDescriptor,
    precision: Precision,
    max_frames: usize,
    sample_rate: f64,
    n_in: usize,
    n_out: usize,
    main: (u16, u16),
    ins: Vec<f32>,
    outs: Vec<f32>,
    ins64: Vec<f64>,
    outs64: Vec<f64>,
    in_ptrs: Vec<*mut f32>,
    out_ptrs: Vec<*mut f32>,
    in_ptrs64: Vec<*mut f64>,
    out_ptrs64: Vec<*mut f64>,
    midi: Box<[VstMidiEvent]>,
    events: Box<VstEventsBuf<MAX_EVENTS>>,
    midi_len: usize,
    time: Box<VstTimeInfo>,
    was_playing: bool,
    midi_out: Box<MidiOut>,
    /// Sounding notes per channel (bit = key), for `AllNotesOff` / `reset`.
    held: [u128; 16],
    release_all: bool,
    mpe_out: MpeOut,
}

// SAFETY: the raw pointers point into buffers this node owns; the effect is shared with the
// controller per the VST2 threading split (see `Effect`).
unsafe impl Send for Vst2Node {}

pub(crate) struct NodeInit {
    pub effect: Arc<Effect>,
    pub shared: Arc<NodeShared>,
    pub descriptor: DeviceDescriptor,
    pub precision: Precision,
    pub config: PrepareConfig,
}

impl Vst2Node {
    pub(crate) fn new(init: NodeInit) -> Self {
        let NodeInit {
            effect,
            shared,
            descriptor,
            precision,
            config,
        } = init;
        let max_frames = config.max_block_size.max(1);
        let n_in = effect.num_inputs() as usize;
        let n_out = effect.num_outputs() as usize;
        let main = (
            descriptor.audio_inputs,
            descriptor.audio_outputs,
        );
        let double = precision == Precision::Double;
        let wide = |n: usize| if double { n * max_frames } else { 0 };
        let mut midi = vec![VstMidiEvent::default(); MAX_EVENTS].into_boxed_slice();
        let mut events = Box::new(VstEventsBuf::<MAX_EVENTS> {
            num_events: 0,
            reserved: 0,
            events: [std::ptr::null_mut(); MAX_EVENTS],
        });
        for (slot, ev) in events.events.iter_mut().zip(midi.iter_mut()) {
            *slot = (ev as *mut VstMidiEvent).cast();
        }
        Self {
            shared,
            descriptor,
            precision,
            max_frames,
            sample_rate: f64::from(config.sample_rate),
            n_in,
            n_out,
            main,
            ins: vec![0.0; n_in * max_frames],
            outs: vec![0.0; n_out * max_frames],
            ins64: vec![0.0; wide(n_in)],
            outs64: vec![0.0; wide(n_out)],
            in_ptrs: vec![std::ptr::null_mut(); n_in.max(1)],
            out_ptrs: vec![std::ptr::null_mut(); n_out.max(1)],
            in_ptrs64: vec![std::ptr::null_mut(); n_in.max(1)],
            out_ptrs64: vec![std::ptr::null_mut(); n_out.max(1)],
            midi,
            events,
            midi_len: 0,
            time: Box::new(VstTimeInfo::default()),
            was_playing: false,
            midi_out: Box::new(MidiOut::with_capacity(MAX_EVENTS)),
            held: [0; 16],
            release_all: false,
            mpe_out: MpeOut::default(),
            effect,
        }
    }

    pub fn precision(&self) -> Precision {
        self.precision
    }

    fn scope(&mut self) -> AudioScope {
        AudioScope {
            time: &mut *self.time,
            midi_out: &mut *self.midi_out,
        }
    }

    fn push_midi(&mut self, delta: u32, data: [u8; 3]) {
        let Some(ev) = self.midi.get_mut(self.midi_len) else {
            return;
        };
        *ev = VstMidiEvent {
            event_type: kVstMidiType,
            byte_size: size_of::<VstMidiEvent>() as i32,
            delta_frames: delta as i32,
            flags: kVstMidiEventIsRealtime,
            midi_data: [data[0], data[1], data[2], 0],
            ..VstMidiEvent::default()
        };
        self.midi_len += 1;
    }

    fn note(&mut self, delta: u32, on: bool, channel: u8, key: u8, velocity: u8) {
        let ch = channel.min(15);
        let key = key.min(127);
        let bit = 1u128 << key;
        if on {
            self.held[usize::from(ch)] |= bit;
            self.push_midi(delta, [0x90 | ch, key, velocity.clamp(1, 127)]);
        } else {
            self.held[usize::from(ch)] &= !bit;
            self.push_midi(delta, [0x80 | ch, key, velocity.min(127)]);
        }
    }

    /// Note-off for every sounding note, then "all notes off" (CC 123) on every channel.
    fn release_held(&mut self, delta: u32) {
        for ch in 0..16u8 {
            while self.held[usize::from(ch)] != 0 {
                let key = self.held[usize::from(ch)].trailing_zeros() as u8;
                self.note(delta, false, ch, key, 0);
            }
            self.push_midi(delta, [0xB0 | ch, 123, 0]);
        }
    }

    fn convert_event(&mut self, delta: u32, kind: &EventKind) {
        // MPE expression without per-note support in VST2: as MPE MIDI (member channels).
        let mut out = std::mem::take(&mut self.mpe_out);
        out.translate(kind, |kind| self.convert_one(delta, kind));
        self.mpe_out = out;
    }

    fn convert_one(&mut self, delta: u32, kind: EventKind) {
        let vel = |v: f32| (v.clamp(0.0, 1.0) * 127.0).round() as u8;
        match kind {
            EventKind::NoteOn {
                channel,
                key,
                velocity,
                ..
            } => self.note(delta, true, channel, key, vel(velocity)),
            EventKind::NoteOff {
                channel,
                key,
                velocity,
                ..
            } => self.note(delta, false, channel, key, vel(velocity)),
            EventKind::NoteChoke { channel, key, .. } => self.note(delta, false, channel, key, 0),
            EventKind::AllNotesOff => self.release_held(delta),
            EventKind::Midi { data } => {
                let (status, ch) = (data[0] & 0xF0, data[0] & 0x0F);
                match status {
                    0x90 if data[2] & 0x7F > 0 => {
                        self.held[usize::from(ch)] |= 1u128 << (data[1] & 0x7F);
                    }
                    0x80 | 0x90 => self.held[usize::from(ch)] &= !(1u128 << (data[1] & 0x7F)),
                    _ => {}
                }
                self.push_midi(delta, data);
            }
            // Params are applied at sub-block starts (see `run`); expression came in as MIDI.
            EventKind::Param { .. } | EventKind::NoteExpression { .. } => {}
        }
    }

    fn fill_time(&mut self, t: &TransportInfo, pos: u32) {
        let sig = t.time_signature;
        let mut flags = kVstNanosValid
            | kVstPpqPosValid
            | kVstTempoValid
            | kVstBarsValid
            | kVstCyclePosValid
            | kVstTimeSigValid;
        if t.playing {
            flags |= kVstTransportPlaying;
        }
        if t.recording {
            flags |= kVstTransportRecording;
        }
        if t.loop_active {
            flags |= kVstTransportCycleActive;
        }
        if pos == 0 && t.playing != self.was_playing {
            flags |= kVstTransportChanged;
        }
        let at = f64::from(pos);
        let sr = self.sample_rate;
        *self.time = VstTimeInfo {
            sample_pos: (t.seconds * sr).round() + at,
            sample_rate: sr,
            nano_seconds: (t.sample_time as f64 + at) / sr * 1e9,
            ppq_pos: t.position + at * t.beats_per_sample,
            tempo: t.bpm,
            bar_start_pos: t.bar_start,
            cycle_start_pos: t.loop_start,
            cycle_end_pos: t.loop_end,
            time_sig_numerator: i32::from(sig.numerator),
            time_sig_denominator: i32::from(sig.denominator),
            smpte_offset: 0,
            smpte_frame_rate: 0,
            samples_to_next_clock: 0,
            flags,
        };
    }

    /// One plugin process call over `[pos, pos + n)` of the node's buffers.
    fn process_segment(&mut self, pos: usize, n: usize) {
        let max = self.max_frames;
        let raw = self.effect.raw();
        match self.precision {
            Precision::Single | Precision::Accumulate => {
                for (c, p) in self.in_ptrs.iter_mut().enumerate().take(self.n_in) {
                    // SAFETY: `c < n_in`, `pos + n <= max`: inside `ins`.
                    *p = unsafe { self.ins.as_mut_ptr().add(c * max + pos) };
                }
                for (c, p) in self.out_ptrs.iter_mut().enumerate().take(self.n_out) {
                    // SAFETY: as above, inside `outs`.
                    *p = unsafe { self.outs.as_mut_ptr().add(c * max + pos) };
                }
                let f = if self.precision == Precision::Single {
                    self.effect.process_replacing()
                } else {
                    for c in 0..self.n_out {
                        self.outs[c * max + pos..c * max + pos + n].fill(0.0);
                    }
                    self.effect.process_accumulating()
                };
                if let Some(f) = f {
                    // SAFETY: channel pointers to `n` valid samples each.
                    unsafe {
                        f(raw, self.in_ptrs.as_mut_ptr(), self.out_ptrs.as_mut_ptr(), n as i32)
                    };
                }
            }
            Precision::Double => {
                for c in 0..self.n_in {
                    let r = c * max + pos..c * max + pos + n;
                    for (d, s) in self.ins64[r.clone()].iter_mut().zip(&self.ins[r]) {
                        *d = f64::from(*s);
                    }
                    // SAFETY: inside `ins64`.
                    self.in_ptrs64[c] = unsafe { self.ins64.as_mut_ptr().add(c * max + pos) };
                }
                for c in 0..self.n_out {
                    // SAFETY: inside `outs64`.
                    self.out_ptrs64[c] = unsafe { self.outs64.as_mut_ptr().add(c * max + pos) };
                }
                if let Some(f) = self.effect.process_double_replacing() {
                    // SAFETY: channel pointers to `n` valid samples each.
                    unsafe {
                        f(
                            raw,
                            self.in_ptrs64.as_mut_ptr(),
                            self.out_ptrs64.as_mut_ptr(),
                            n as i32,
                        )
                    };
                }
                for c in 0..self.n_out {
                    let r = c * max + pos..c * max + pos + n;
                    for (d, s) in self.outs[r.clone()].iter_mut().zip(&self.outs64[r]) {
                        *d = *s as f32;
                    }
                }
            }
        }
    }

    fn send_midi(&mut self) {
        if self.midi_len == 0 {
            return;
        }
        self.events.num_events = self.midi_len as i32;
        let ptr = (&mut *self.events as *mut VstEventsBuf<MAX_EVENTS>).cast();
        self.effect.dispatch(effProcessEvents, 0, 0, ptr, 0.0);
    }

    fn run(&mut self, ctx: &mut ProcessContext<'_>, audio: &mut AudioBuffers<'_, '_>) -> ProcessStatus {
        let frames = ctx.frames;
        if frames > self.max_frames {
            audio.clear_outputs();
            return ProcessStatus::Silent;
        }
        if frames == 0 {
            return ProcessStatus::Continue;
        }
        let max = self.max_frames;
        // Main input → the first plugin inputs (a mono source feeds both); others silent.
        let main_in = usize::from(self.main.0);
        for c in 0..self.n_in {
            let dst = &mut self.ins[c * max..c * max + frames];
            let src = if c < main_in {
                audio.inputs.get(c).or(audio.inputs.last()).copied()
            } else {
                None
            };
            match src {
                Some(src) => dst.copy_from_slice(&src[..frames]),
                None => dst.fill(0.0),
            }
        }
        self.midi_out.len = 0;
        let scope = self.scope();
        let _guard = scope.enter();

        let events = ctx.events;
        let mut ei = 0;
        let mut pos = 0u32;
        let frames_u = frames as u32;
        while pos < frames_u {
            let mut end = frames_u;
            for e in &events[ei..] {
                if matches!(e.kind, EventKind::Param { .. }) && e.offset >= pos + MIN_SPLIT {
                    end = e.offset.min(frames_u);
                    break;
                }
            }
            self.midi_len = 0;
            if std::mem::take(&mut self.release_all) {
                self.release_held(0);
            }
            while ei < events.len() && events[ei].offset < end {
                let e = &events[ei];
                let delta = e.offset.saturating_sub(pos);
                match e.kind {
                    EventKind::Param { param, value } => {
                        self.effect.set_parameter(param.0 as i32, value as f32)
                    }
                    ref kind => self.convert_event(delta, kind),
                }
                ei += 1;
            }
            self.fill_time(ctx.transport, pos);
            self.midi_out.base = pos;
            self.send_midi();
            self.process_segment(pos as usize, (end - pos) as usize);
            pos = end;
        }
        self.was_playing = ctx.transport.playing;

        // First plugin outputs → node outputs (a mono plugin feeds every channel).
        if self.n_out == 0 {
            audio.clear_outputs();
        } else {
            for (c, dst) in audio.outputs.iter_mut().enumerate() {
                let c = c.min(self.n_out - 1);
                dst[..frames].copy_from_slice(&self.outs[c * max..c * max + frames]);
            }
        }
        // Plugin MIDI out.
        let last = frames_u - 1;
        for &(offset, data) in &self.midi_out.events[..self.midi_out.len] {
            let (status, ch) = (data[0] & 0xF0, data[0] & 0x0F);
            let (key, v) = (data[1] & 0x7F, data[2] & 0x7F);
            let kind = match status {
                0x90 if v > 0 => EventKind::NoteOn {
                    note_id: u32::MAX,
                    channel: ch,
                    key,
                    velocity: f32::from(v) / 127.0,
                },
                0x80 | 0x90 => EventKind::NoteOff {
                    note_id: u32::MAX,
                    channel: ch,
                    key,
                    velocity: f32::from(v) / 127.0,
                },
                _ => EventKind::Midi { data },
            };
            ctx.out_events.push(ProcessEvent {
                offset: offset.min(last),
                kind,
            });
        }
        ProcessStatus::Continue
    }
}

impl Node for Vst2Node {
    fn prepare(&mut self, config: &PrepareConfig) {
        if config.max_block_size > self.max_frames {
            tracing::warn!(
                "VST2 node prepared for {} frames but activated for {}",
                config.max_block_size,
                self.max_frames
            );
        }
    }

    fn reset(&mut self) {
        self.release_all = true;
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        self.run(ctx, audio)
    }

    fn latency(&self) -> u32 {
        self.shared.latency.load(Ordering::Relaxed)
    }

    fn channels(&self) -> (u16, u16) {
        self.main
    }
}

impl Device for Vst2Node {
    fn descriptor(&self) -> DeviceDescriptor {
        self.descriptor.clone()
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        let i = i32::try_from(id.0).ok()?;
        (i < self.effect.num_params()).then(|| f64::from(self.effect.get_parameter(i)))
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        let Ok(i) = i32::try_from(id.0) else {
            return;
        };
        let scope = self.scope();
        let _guard = scope.enter();
        self.effect.set_parameter(i, value as f32);
    }
}

impl PluginNode for Vst2Node {
    fn is_faulted(&self) -> bool {
        // VST2 has no error channel from processing; a crash takes the process down (use the
        // sandbox for isolation).
        false
    }
}

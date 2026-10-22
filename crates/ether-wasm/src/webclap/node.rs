//! [`WebclapNode`]: a WebCLAP plugin instance as an engine node.
//!
//! The node owns no plugin code: each block it fills an [`Exchange`] (the engine's channel
//! pointers, its events converted to [`EventRec`]s, the transport) and hands it to the
//! realm's [`Guest`](super::guest::Guest), which copies the audio into the plugin's own
//! memory, builds the CLAP events and calls `clap_plugin.process`. Everything is allocated
//! at construction: `process` only writes into pre-sized buffers.
//!
//! The exchange is read by JS through the engine's wasm memory, so its layout is fixed
//! (`#[repr(C)]`, offsets asserted for wasm32 below and mirrored in
//! `apps/web/src/engine/webclap/layout.ts`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor, DeviceTypeRef};
use ether_core::protocol::model::{NoteExpressionKind, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PluginNode, PrepareConfig, ProcessContext, ProcessEvent,
    ProcessStatus,
};

use super::catalog::Layout;
use super::guest::{GuestSpec, status, with_guest};

/// Events per block, in and out (more are dropped, like `ether-clap`'s bounded buffers).
pub const EVENT_CAPACITY: usize = 1024;
/// Immediate sets (`Device::set_param`) kept for the next block.
const PENDING_CAPACITY: usize = 256;

/// [`EventRec::kind`] values.
pub mod kind {
    pub const NOTE_ON: u8 = 1;
    pub const NOTE_OFF: u8 = 2;
    pub const NOTE_CHOKE: u8 = 3;
    /// Release every note (the guest sends a wildcard note-off and MIDI CC 123).
    pub const ALL_NOTES_OFF: u8 = 4;
    pub const PARAM: u8 = 5;
    /// `id` = the three MIDI bytes, little endian.
    pub const MIDI: u8 = 6;
    /// `extra` = the CLAP note expression id (`clap_note_expression`).
    pub const NOTE_EXPRESSION: u8 = 7;
}

/// CLAP note expression ids.
const CLAP_NOTE_EXPRESSION_TUNING: u8 = 2;
const CLAP_NOTE_EXPRESSION_BRIGHTNESS: u8 = 5;
const CLAP_NOTE_EXPRESSION_PRESSURE: u8 = 6;

/// One event, engine ↔ guest (24 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EventRec {
    pub offset: u32,
    pub kind: u8,
    pub channel: u8,
    pub key: u8,
    pub extra: u8,
    /// Note id, param id, or the MIDI bytes.
    pub id: u32,
    pub _pad: u32,
    /// Velocity (0..=1), plain param value, expression value.
    pub value: f64,
}

/// Transport flags of [`Exchange::flags`].
pub mod flags {
    pub const PLAYING: u32 = 1;
    pub const RECORDING: u32 = 2;
    pub const LOOP_ACTIVE: u32 = 4;
}

/// One block, as the guest sees it. Pointers are into this (the engine's) wasm memory.
#[repr(C)]
#[derive(Debug)]
pub struct Exchange {
    pub frames: u32,
    /// Engine channels at `inputs` / `outputs` / `sidechain` (each at most 2).
    pub in_channels: u32,
    pub out_channels: u32,
    pub sc_channels: u32,
    pub n_events: u32,
    /// Written by the guest: events at `out_events`.
    pub n_out_events: u32,
    pub event_capacity: u32,
    pub flags: u32,
    pub inputs: [*const f32; 2],
    pub outputs: [*mut f32; 2],
    pub sidechain: [*const f32; 2],
    pub events: *const EventRec,
    pub out_events: *mut EventRec,
    pub steady_time: f64,
    pub position: f64,
    pub seconds: f64,
    pub bpm: f64,
    pub tempo_inc: f64,
    pub bar_start: f64,
    pub loop_start: f64,
    pub loop_end: f64,
    pub bar_number: i32,
    pub ts_num: u32,
    pub ts_den: u32,
    pub _pad: u32,
}

// The JS side reads these offsets (layout.ts).
#[cfg(target_arch = "wasm32")]
const _: () = {
    use std::mem::{offset_of, size_of};
    assert!(size_of::<EventRec>() == 24);
    assert!(offset_of!(EventRec, kind) == 4);
    assert!(offset_of!(EventRec, id) == 8);
    assert!(offset_of!(EventRec, value) == 16);
    assert!(offset_of!(Exchange, inputs) == 32);
    assert!(offset_of!(Exchange, outputs) == 40);
    assert!(offset_of!(Exchange, sidechain) == 48);
    assert!(offset_of!(Exchange, events) == 56);
    assert!(offset_of!(Exchange, out_events) == 60);
    assert!(offset_of!(Exchange, steady_time) == 64);
    assert!(offset_of!(Exchange, loop_end) == 120);
    assert!(offset_of!(Exchange, bar_number) == 128);
    assert!(offset_of!(Exchange, ts_den) == 136);
    assert!(size_of::<Exchange>() == 144);
};

// Only raw pointers into buffers the node owns; the node moves to the audio thread whole.
unsafe impl Send for Exchange {}

/// What a node instantiates (from the controller's catalog).
#[derive(Clone, Debug, PartialEq)]
pub struct NodeSpec {
    pub module: String,
    pub plugin_id: String,
    pub name: String,
    pub device: String,
    pub layout: Layout,
    pub category: DeviceCategory,
    pub note_expressions: bool,
    /// Every param's id and default (the node's value table, sorted by id).
    pub params: Vec<(u32, f64)>,
    pub always_process: bool,
}

pub struct WebclapNode {
    /// 0: no guest in this realm (the node stays silent).
    handle: u32,
    spec: NodeSpec,
    max_frames: usize,
    /// Current plain values, sorted by id (UI readback; updated by events both ways).
    values: Vec<(u32, f64)>,
    pending: Vec<(u32, f64)>,
    events: Box<[EventRec]>,
    out_events: Box<[EventRec]>,
    exchange: Box<Exchange>,
    always: Arc<AtomicBool>,
    faulted: bool,
    steady: u64,
}

impl WebclapNode {
    /// Non-RT. Instantiates through the realm's guest (the plugin is activated at
    /// `sample_rate` for blocks up to `max_frames`).
    pub fn new(spec: NodeSpec, state: Option<&[u8]>, sample_rate: f64, max_frames: usize) -> Self {
        let handle = with_guest(|g| {
            g.create(&GuestSpec {
                module: &spec.module,
                plugin_id: &spec.plugin_id,
                device: &spec.device,
                sample_rate,
                max_frames: max_frames as u32,
                state,
            })
        })
        .unwrap_or(0);
        let mut values = spec.params.clone();
        values.sort_by_key(|(id, _)| *id);
        values.dedup_by_key(|(id, _)| *id);
        let mut events = vec![EventRec::default(); EVENT_CAPACITY].into_boxed_slice();
        let mut out_events = vec![EventRec::default(); EVENT_CAPACITY].into_boxed_slice();
        let exchange = Box::new(Exchange {
            frames: 0,
            in_channels: 0,
            out_channels: 0,
            sc_channels: 0,
            n_events: 0,
            n_out_events: 0,
            event_capacity: EVENT_CAPACITY as u32,
            flags: 0,
            inputs: [std::ptr::null(); 2],
            outputs: [std::ptr::null_mut(); 2],
            sidechain: [std::ptr::null(); 2],
            events: events.as_mut_ptr(),
            out_events: out_events.as_mut_ptr(),
            steady_time: 0.0,
            position: 0.0,
            seconds: 0.0,
            bpm: 120.0,
            tempo_inc: 0.0,
            bar_start: 0.0,
            loop_start: 0.0,
            loop_end: 0.0,
            bar_number: 0,
            ts_num: 4,
            ts_den: 4,
            _pad: 0,
        });
        Self {
            handle,
            always: Arc::new(AtomicBool::new(spec.always_process)),
            spec,
            max_frames,
            values,
            pending: Vec::with_capacity(PENDING_CAPACITY),
            events,
            out_events,
            exchange,
            faulted: handle == 0,
            steady: 0,
        }
    }

    /// The guest's handle of this instance (0: none).
    pub fn handle(&self) -> u32 {
        self.handle
    }

    fn set_value(&mut self, id: u32, value: f64) {
        if let Ok(i) = self.values.binary_search_by_key(&id, |(k, _)| *k) {
            self.values[i].1 = value;
        }
    }

    fn push(&mut self, n: &mut usize, rec: EventRec) {
        if *n < self.events.len() {
            self.events[*n] = rec;
            *n += 1;
        }
    }

    fn convert(&mut self, n: &mut usize, e: &ProcessEvent) {
        let note = |kind, note_id: u32, channel: u8, key: u8, value: f64| EventRec {
            offset: e.offset,
            kind,
            channel,
            key,
            id: note_id,
            value,
            ..EventRec::default()
        };
        let rec = match e.kind {
            EventKind::NoteOn {
                note_id,
                channel,
                key,
                velocity,
            } => note(kind::NOTE_ON, note_id, channel, key, f64::from(velocity)),
            EventKind::NoteOff {
                note_id,
                channel,
                key,
                velocity,
            } => note(kind::NOTE_OFF, note_id, channel, key, f64::from(velocity)),
            EventKind::NoteChoke {
                note_id,
                channel,
                key,
            } => note(kind::NOTE_CHOKE, note_id, channel, key, 0.0),
            EventKind::AllNotesOff => note(kind::ALL_NOTES_OFF, 0, 0, 0, 0.0),
            EventKind::Param { param, value } => {
                self.set_value(param.0, value);
                EventRec {
                    offset: e.offset,
                    kind: kind::PARAM,
                    id: param.0,
                    value,
                    ..EventRec::default()
                }
            }
            EventKind::Midi { data } => EventRec {
                offset: e.offset,
                kind: kind::MIDI,
                id: u32::from_le_bytes([data[0], data[1], data[2], 0]),
                ..EventRec::default()
            },
            EventKind::NoteExpression {
                note_id,
                channel,
                key,
                expression,
                value,
            } => {
                if !self.spec.note_expressions {
                    // Without the CLAP note dialect: pressure as poly aftertouch, the
                    // rest is dropped (MPE MIDI is not translated here).
                    match expression {
                        NoteExpressionKind::Pressure => EventRec {
                            offset: e.offset,
                            kind: kind::MIDI,
                            id: u32::from_le_bytes([
                                0xA0 | (channel & 0x0F),
                                key & 0x7F,
                                (value.clamp(0.0, 1.0) * 127.0).round() as u8,
                                0,
                            ]),
                            ..EventRec::default()
                        },
                        _ => return,
                    }
                } else {
                    let (extra, value) = match expression {
                        NoteExpressionKind::Pitch => (CLAP_NOTE_EXPRESSION_TUNING, value),
                        NoteExpressionKind::Pressure => (CLAP_NOTE_EXPRESSION_PRESSURE, value),
                        NoteExpressionKind::Timbre => (CLAP_NOTE_EXPRESSION_BRIGHTNESS, value),
                    };
                    EventRec {
                        extra,
                        ..note(
                            kind::NOTE_EXPRESSION,
                            note_id,
                            channel,
                            key,
                            f64::from(value),
                        )
                    }
                }
            }
        };
        self.push(n, rec);
    }

    fn run(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        sidechain: &[&[f32]],
    ) -> ProcessStatus {
        let frames = ctx.frames;
        if self.faulted || frames > self.max_frames {
            audio.clear_outputs();
            return ProcessStatus::Silent;
        }
        if frames == 0 {
            return ProcessStatus::Continue;
        }

        // Pending immediate sets at offset 0, then the engine's (sorted) events.
        let mut n = 0;
        for i in 0..self.pending.len() {
            let (id, value) = self.pending[i];
            self.push(
                &mut n,
                EventRec {
                    kind: kind::PARAM,
                    id,
                    value,
                    ..EventRec::default()
                },
            );
        }
        self.pending.clear();
        for e in ctx.events {
            self.convert(&mut n, e);
        }

        let t = ctx.transport;
        let x = &mut *self.exchange;
        x.frames = frames as u32;
        x.n_events = n as u32;
        x.n_out_events = 0;
        x.in_channels = audio.inputs.len().min(2) as u32;
        x.out_channels = audio.outputs.len().min(2) as u32;
        x.sc_channels = sidechain.len().min(2) as u32;
        for c in 0..2 {
            x.inputs[c] = audio.inputs.get(c).map_or(std::ptr::null(), |s| s.as_ptr());
            x.outputs[c] = audio
                .outputs
                .get_mut(c)
                .map_or(std::ptr::null_mut(), |s| s.as_mut_ptr());
            x.sidechain[c] = sidechain.get(c).map_or(std::ptr::null(), |s| s.as_ptr());
        }
        let flag = |on: bool, f: u32| if on { f } else { 0 };
        x.flags = flag(t.playing, flags::PLAYING)
            | flag(t.recording, flags::RECORDING)
            | flag(t.loop_active, flags::LOOP_ACTIVE);
        x.steady_time = self.steady as f64;
        x.position = t.position;
        x.seconds = t.seconds;
        x.bpm = t.bpm;
        x.tempo_inc = t.tempo_inc;
        x.bar_start = t.bar_start;
        x.loop_start = t.loop_start;
        x.loop_end = t.loop_end;
        x.bar_number = t.bar_number;
        x.ts_num = u32::from(t.time_signature.numerator);
        x.ts_den = u32::from(t.time_signature.denominator);

        let handle = self.handle;
        let code = with_guest(|g| g.process(handle, x)).unwrap_or(status::FAULTED);
        self.steady = self.steady.wrapping_add(frames as u64);

        match code {
            status::PENDING => {
                // Not instantiated yet: effects pass the dry signal, instruments are silent.
                audio.pass_through();
                for o in audio.outputs.iter_mut() {
                    o[frames..].fill(0.0);
                }
                return ProcessStatus::Continue;
            }
            status::CONTINUE..=status::SLEEP => {}
            _ => {
                self.faulted = true;
                audio.clear_outputs();
                return ProcessStatus::Silent;
            }
        }

        // The plugin's output events: its own param changes (values), notes and MIDI out.
        let last = (frames - 1) as u32;
        let count = (self.exchange.n_out_events as usize).min(self.out_events.len());
        for i in 0..count {
            let r = self.out_events[i];
            let offset = r.offset.min(last);
            let out = |kind| ProcessEvent { offset, kind };
            match r.kind {
                kind::PARAM => self.set_value(r.id, r.value),
                kind::NOTE_ON => {
                    ctx.out_events.push(out(EventKind::NoteOn {
                        note_id: r.id,
                        channel: r.channel,
                        key: r.key,
                        velocity: r.value as f32,
                    }));
                }
                kind::NOTE_OFF => {
                    ctx.out_events.push(out(EventKind::NoteOff {
                        note_id: r.id,
                        channel: r.channel,
                        key: r.key,
                        velocity: r.value as f32,
                    }));
                }
                kind::NOTE_CHOKE => {
                    ctx.out_events.push(out(EventKind::NoteChoke {
                        note_id: r.id,
                        channel: r.channel,
                        key: r.key,
                    }));
                }
                kind::MIDI => {
                    let [a, b, c, _] = r.id.to_le_bytes();
                    ctx.out_events
                        .push(out(EventKind::Midi { data: [a, b, c] }));
                }
                _ => {}
            }
        }

        // CLAP sleep rules, as `ether-clap`: `SLEEP` sleeps, `CONTINUE_IF_NOT_QUIET` once
        // the output is quiet (-120 dBFS), `TAIL` and `CONTINUE` keep running.
        match code {
            _ if self.always.load(Ordering::Relaxed) => ProcessStatus::Continue,
            status::SLEEP => ProcessStatus::Silent,
            status::CONTINUE_IF_NOT_QUIET
                if audio
                    .outputs
                    .iter()
                    .all(|o| o[..frames.min(o.len())].iter().all(|s| s.abs() < 1e-6)) =>
            {
                ProcessStatus::Silent
            }
            _ => ProcessStatus::Continue,
        }
    }
}

impl Drop for WebclapNode {
    fn drop(&mut self) {
        if self.handle != 0 {
            let handle = self.handle;
            with_guest(|g| g.destroy(handle));
        }
    }
}

impl Node for WebclapNode {
    fn prepare(&mut self, config: &PrepareConfig) {
        // The instance was activated for `max_frames`; larger blocks are refused in
        // `process` (the engine uses the same config for both).
        let _ = config;
    }

    fn reset(&mut self) {
        let handle = self.handle;
        if handle != 0 && !self.faulted {
            with_guest(|g| g.reset(handle));
        }
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        self.run(ctx, audio, &[])
    }

    fn process_sidechain(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        sidechain: &[&[f32]],
    ) -> ProcessStatus {
        self.run(ctx, audio, sidechain)
    }

    fn sidechain_inputs(&self) -> u16 {
        self.spec.layout.sidechain
    }

    fn wake_requested(&mut self) -> bool {
        let handle = self.handle;
        self.always.load(Ordering::Relaxed)
            || (handle != 0 && with_guest(|g| g.take_wake(handle)).unwrap_or(false))
    }

    fn latency(&self) -> u32 {
        let handle = self.handle;
        if handle == 0 {
            return 0;
        }
        with_guest(|g| g.latency(handle)).unwrap_or(0)
    }

    fn channels(&self) -> (u16, u16) {
        (self.spec.layout.inputs, self.spec.layout.outputs)
    }
}

impl Device for WebclapNode {
    fn descriptor(&self) -> DeviceDescriptor {
        // The full descriptor (param infos) lives with the controller's catalog.
        DeviceDescriptor {
            device_type: DeviceTypeRef::Plugin {
                plugin_id: self.spec.plugin_id.clone(),
            },
            name: self.spec.name.clone(),
            category: self.spec.category,
            params: Vec::new(),
            audio_inputs: self.spec.layout.inputs,
            audio_outputs: self.spec.layout.outputs,
            midi_input: self.spec.layout.midi,
            sidechain_inputs: self.spec.layout.sidechain,
            layout: None,
            implemented: true,
        }
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

impl PluginNode for WebclapNode {
    fn is_faulted(&self) -> bool {
        self.faulted
    }

    fn always_process(&self) -> Option<Arc<AtomicBool>> {
        Some(self.always.clone())
    }
}

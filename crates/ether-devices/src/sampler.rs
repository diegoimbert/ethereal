//! Sampler: one-shot and pitched (root key) playback of a media sample.
//!
//! The sample is an [`AudioSource`] supplied by the host (already at the engine sample
//! rate). Modes:
//! - **One-shot**: every note plays the whole sample at its original pitch (plus
//!   `Transpose`); note-offs are ignored, the voice ends with the sample.
//! - **Pitched**: playback rate follows `key - Root Key` (+ `Transpose`); note-off starts
//!   the release.
//!
//! Reads go through the source in chunks into pre-allocated scratch buffers and are
//! linearly interpolated, so `process` never allocates.

use std::sync::Arc;

use ether_core::protocol::devices::{
    DeviceCategory, DeviceDescriptor, DeviceTypeRef, ParamInfo, ParamScale, ParamUnit,
};
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, AudioSource, Device, EventKind, Node, PrepareConfig, ProcessContext,
    ProcessStatus, Smoother,
};

use crate::util::{self, Adsr, AdsrRates};

/// Maximum simultaneous voices.
pub const MAX_VOICES: usize = 8;
/// Maximum playback-rate ratio (4 octaves up).
const MAX_RATE: f64 = 16.0;
/// Render chunk size in frames (bounds the scratch buffers).
const CHUNK: usize = 64;
const SCRATCH: usize = CHUNK * MAX_RATE as usize + 2;

/// Parameter ids (stable: stored in documents and automation).
pub mod params {
    use ether_core::protocol::model::ParamId;
    pub const MODE: ParamId = ParamId(0);
    pub const ROOT_KEY: ParamId = ParamId(1);
    pub const TRANSPOSE: ParamId = ParamId(2);
    pub const ATTACK: ParamId = ParamId(3);
    pub const RELEASE: ParamId = ParamId(4);
    pub const VOLUME: ParamId = ParamId(5);
}

const NUM_PARAMS: usize = 6;

/// Mode labels in plain-value order.
pub const MODES: [&str; 2] = ["One-shot", "Pitched"];
const MODE_ONE_SHOT: usize = 0;

/// Parameter list (identical for every instance).
pub fn param_infos() -> Vec<ParamInfo> {
    use util::{choice, param};
    let time = ParamScale::Power { exponent: 3.0 };
    vec![
        choice(0, "Mode", "Playback", ParamUnit::None, &MODES, 1),
        param(
            1,
            "Root Key",
            "Playback",
            ParamUnit::None,
            (0.0, 127.0, 60.0),
            ParamScale::Linear,
        ),
        param(
            2,
            "Transpose",
            "Playback",
            ParamUnit::Semitones,
            (-24.0, 24.0, 0.0),
            ParamScale::Linear,
        ),
        param(
            3,
            "Attack",
            "Envelope",
            ParamUnit::Milliseconds,
            (0.0, 5000.0, 1.0),
            time,
        ),
        param(
            4,
            "Release",
            "Envelope",
            ParamUnit::Milliseconds,
            (1.0, 10000.0, 100.0),
            time,
        ),
        param(
            5,
            "Volume",
            "Output",
            ParamUnit::Decibels,
            (-60.0, 6.0, 0.0),
            ParamScale::Linear,
        ),
    ]
}

/// Descriptor of the sampler type.
pub fn descriptor() -> DeviceDescriptor {
    DeviceDescriptor {
        device_type: DeviceTypeRef::Builtin {
            device: BuiltinDeviceType::Sampler,
        },
        name: "Sampler".to_owned(),
        category: DeviceCategory::Instrument,
        params: param_infos(),
        audio_inputs: 0,
        audio_outputs: 2,
        midi_input: true,
    }
}

#[derive(Clone, Copy, Debug)]
struct Voice {
    note_id: u32,
    channel: u8,
    key: u8,
    velocity: f32,
    age: u64,
    /// Read position in source frames.
    pos: f64,
    env: Adsr,
}

impl Voice {
    const IDLE: Self = Self {
        note_id: 0,
        channel: 0,
        key: 0,
        velocity: 0.0,
        age: 0,
        pos: 0.0,
        env: Adsr::IDLE,
    };
}

/// The built-in sampler.
pub struct Sampler {
    source: Option<Arc<dyn AudioSource>>,
    values: [f64; NUM_PARAMS],
    infos: Vec<ParamInfo>,
    sample_rate: f32,
    voices: [Voice; MAX_VOICES],
    next_age: u64,
    rates: AdsrRates,
    volume: Smoother,
    /// Per source channel (max 2) read scratch.
    scratch: [Vec<f32>; 2],
    gains: [f32; CHUNK],
}

impl std::fmt::Debug for Sampler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sampler")
            .field("has_source", &self.source.is_some())
            .field("values", &self.values)
            .finish()
    }
}

impl Default for Sampler {
    fn default() -> Self {
        Self::new(None)
    }
}

impl Sampler {
    /// Non-RT. A sampler with default parameters playing `source` (`None` = silent).
    pub fn new(source: Option<Arc<dyn AudioSource>>) -> Self {
        let infos = param_infos();
        let mut values = [0.0; NUM_PARAMS];
        for (v, info) in values.iter_mut().zip(&infos) {
            *v = info.default;
        }
        let mut s = Self {
            source,
            values,
            infos,
            sample_rate: 48_000.0,
            voices: [Voice::IDLE; MAX_VOICES],
            next_age: 0,
            rates: AdsrRates::new(1.0, 1.0, 1.0, 100.0, 48_000.0),
            volume: Smoother::new(1.0, 20.0, 48_000.0),
            scratch: [vec![0.0; SCRATCH], vec![0.0; SCRATCH]],
            gains: [0.0; CHUNK],
        };
        s.sync_all();
        s
    }

    /// Non-RT (drops the previous source). Replace the sample; stops all voices.
    pub fn set_source(&mut self, source: Option<Arc<dyn AudioSource>>) {
        self.source = source;
        self.reset();
    }

    fn value(&self, id: ParamId) -> f64 {
        self.values[id.0 as usize]
    }

    fn sync_all(&mut self) {
        self.volume = Smoother::new(
            util::db_to_amp(self.value(params::VOLUME) as f32),
            20.0,
            self.sample_rate,
        );
        self.update_rates();
    }

    fn update_rates(&mut self) {
        self.rates = AdsrRates::new(
            self.value(params::ATTACK) as f32,
            1.0,
            1.0,
            self.value(params::RELEASE) as f32,
            self.sample_rate,
        );
    }

    fn apply_param(&mut self, id: ParamId, value: f64, smooth: bool) {
        let Some(info) = self.infos.get(id.0 as usize) else {
            return;
        };
        let v = util::clamp(value, info.min, info.max);
        self.values[id.0 as usize] = v;
        match id {
            params::VOLUME => {
                let amp = util::db_to_amp(v as f32);
                if smooth {
                    self.volume.set_target(amp);
                } else {
                    self.volume.set_immediate(amp);
                }
            }
            params::ATTACK | params::RELEASE => self.update_rates(),
            _ => {}
        }
    }

    fn one_shot(&self) -> bool {
        util::index(self.value(params::MODE), MODES.len()) == MODE_ONE_SHOT
    }

    /// Playback rate for `key` in the current mode.
    fn rate(&self, key: u8) -> f64 {
        let mut semis = self.value(params::TRANSPOSE);
        if !self.one_shot() {
            semis += key as f64 - self.value(params::ROOT_KEY).round();
        }
        2f64.powf(semis / 12.0).min(MAX_RATE)
    }

    fn note_on(&mut self, note_id: u32, channel: u8, key: u8, velocity: f32) {
        let Some(source) = &self.source else {
            return;
        };
        source.prefetch_hint(0);
        let slot = self
            .voices
            .iter()
            .position(|v| !v.env.is_active())
            .or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, v)| (!v.env.is_releasing(), v.age))
                    .map(|(i, _)| i)
            })
            .unwrap_or(0);
        self.next_age += 1;
        let v = &mut self.voices[slot];
        *v = Voice {
            note_id,
            channel,
            key,
            velocity: velocity.clamp(0.0, 1.0),
            age: self.next_age,
            ..Voice::IDLE
        };
        v.env.trigger();
    }

    fn for_matching(&mut self, note_id: u32, channel: u8, key: u8, f: impl Fn(&mut Voice)) {
        let by_id = self
            .voices
            .iter()
            .any(|v| v.env.is_active() && v.note_id == note_id);
        for v in self.voices.iter_mut().filter(|v| v.env.is_active()) {
            let hit = if by_id {
                v.note_id == note_id
            } else {
                v.key == key && v.channel == channel
            };
            if hit {
                f(v);
            }
        }
    }

    fn handle_event(&mut self, kind: &EventKind) {
        match *kind {
            EventKind::NoteOn {
                note_id,
                channel,
                key,
                velocity,
            } => self.note_on(note_id, channel, key, velocity),
            EventKind::NoteOff {
                note_id,
                channel,
                key,
                ..
            } => {
                if !self.one_shot() {
                    self.for_matching(note_id, channel, key, |v| v.env.release());
                }
            }
            EventKind::NoteChoke {
                note_id,
                channel,
                key,
            } => self.for_matching(note_id, channel, key, |v| v.env.kill()),
            EventKind::AllNotesOff => {
                for v in &mut self.voices {
                    v.env.release();
                }
            }
            EventKind::Param { param, value } => self.apply_param(param, value, true),
            EventKind::Midi { .. } => {}
        }
    }

    fn render(&mut self, outputs: &mut [&mut [f32]], start: usize, end: usize) {
        for ch in outputs.iter_mut() {
            ch[start..end].fill(0.0);
        }
        let Some(source) = self.source.as_deref() else {
            return;
        };
        let src_channels = (source.channels() as usize).min(2);
        let src_frames = source.frames();
        if src_channels == 0 {
            return;
        }
        let mut c0 = start;
        while c0 < end {
            let n = CHUNK.min(end - c0);
            for g in &mut self.gains[..n] {
                *g = self.volume.tick();
            }
            for vi in 0..MAX_VOICES {
                if !self.voices[vi].env.is_active() {
                    continue;
                }
                let rate = self.rate(self.voices[vi].key);
                let v = &mut self.voices[vi];
                let first = v.pos.floor();
                // Frames needed for `n` outputs, +1 for interpolation.
                let len = ((v.pos + (n - 1) as f64 * rate).floor() - first) as usize + 2;
                let len = len.min(SCRATCH);
                for (ch, buf) in self.scratch[..src_channels].iter_mut().enumerate() {
                    source.read(ch as u16, first as u64, &mut buf[..len]);
                }
                for k in 0..n {
                    if v.pos >= src_frames as f64 {
                        v.env.kill();
                        break;
                    }
                    let idx = v.pos - first;
                    let i0 = (idx as usize).min(len - 2);
                    let frac = (idx - i0 as f64) as f32;
                    let amp = v.env.tick(&self.rates) * v.velocity * self.gains[k];
                    for (o, out) in outputs.iter_mut().enumerate() {
                        let buf = &self.scratch[o.min(src_channels - 1)];
                        let s = buf[i0] + (buf[i0 + 1] - buf[i0]) * frac;
                        out[c0 + k] += s * amp;
                    }
                    if !v.env.is_active() {
                        break;
                    }
                    v.pos += rate;
                }
            }
            c0 += n;
        }
    }

    fn any_active(&self) -> bool {
        self.voices.iter().any(|v| v.env.is_active())
    }
}

impl Node for Sampler {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.sync_all();
        self.reset();
    }

    fn reset(&mut self) {
        self.voices = [Voice::IDLE; MAX_VOICES];
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let has_notes = ctx
            .events
            .iter()
            .any(|e| matches!(e.kind, EventKind::NoteOn { .. }));
        if !self.any_active() && !(has_notes && self.source.is_some()) {
            for e in ctx.events {
                self.handle_event(&e.kind);
            }
            audio.clear_outputs();
            return ProcessStatus::Silent;
        }
        let outputs = &mut *audio.outputs;
        util::split_at_events(
            self,
            ctx.events,
            ctx.frames,
            |s, a, b| s.render(outputs, a, b),
            |s, kind| s.handle_event(kind),
        );
        ProcessStatus::Continue
    }

    fn channels(&self) -> (u16, u16) {
        (0, 2)
    }
}

impl Device for Sampler {
    fn descriptor(&self) -> DeviceDescriptor {
        descriptor()
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.values.get(id.0 as usize).copied()
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.apply_param(id, value, false);
    }
}

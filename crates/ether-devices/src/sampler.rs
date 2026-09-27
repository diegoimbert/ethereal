//! Sampler: one-shot and pitched (root key) playback of a media sample.
//!
//! The sample is an [`AudioSource`] supplied by the host (already at the engine sample
//! rate). Modes:
//! - **One-shot**: every note plays the whole sample at its original pitch (plus
//!   `Transpose`); note-offs are ignored, the voice ends with the sample.
//! - **Pitched**: playback rate follows `key - Root Key` (+ `Transpose`); note-off starts
//!   the release.
//!
//! - **Sample range**: `Start`/`End` (percent of the sample) bound what every note plays
//!   (slice pads of a drum rack use this, `SliceCommand::ToDrumRack`).
//! - **Slice mode** (roadmap v2, `drum-rack`; [`SliceSettings`] in the device kind): note
//!   `base_note + i` plays slice `i` (`[markers[i], markers[i + 1])`, the last one to the
//!   end of the sample) at its original pitch (+ `Transpose`); other keys are ignored. The
//!   markers reach a live sampler in place through [`Node::set_data`] (a boxed
//!   [`SliceSettings`]), so an edit never cuts sounding notes.
//!
//! Reads go through the source in chunks into pre-allocated scratch buffers and are
//! linearly interpolated, so `process` never allocates.

use std::sync::Arc;

use ether_core::node::NodeData;
use ether_core::protocol::devices::{
    DeviceCategory, DeviceDescriptor, DeviceTypeRef, ParamInfo, ParamScale, ParamUnit,
};
use ether_core::protocol::model::{BuiltinDeviceType, ParamId, SliceSettings};
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
    /// Start of the played range, percent of the sample (roadmap v2, `drum-rack`).
    pub const START: ParamId = ParamId(6);
    /// End of the played range, percent of the sample (roadmap v2, `drum-rack`).
    pub const END: ParamId = ParamId(7);
}

const NUM_PARAMS: usize = 8;
/// Fade at a range/slice end that is not the end of the sample (declick), in ms.
const END_FADE_MS: f64 = 2.0;

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
        )
        .with_step(1.0),
        param(
            2,
            "Transpose",
            "Playback",
            ParamUnit::Semitones,
            (-24.0, 24.0, 0.0),
            ParamScale::Linear,
        )
        .with_step(1.0),
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
        param(
            6,
            "Start",
            "Playback",
            ParamUnit::Percent,
            (0.0, 100.0, 0.0),
            ParamScale::Linear,
        ),
        param(
            7,
            "End",
            "Playback",
            ParamUnit::Percent,
            (0.0, 100.0, 100.0),
            ParamScale::Linear,
        ),
    ]
}

/// Descriptor of the sampler type.
pub fn descriptor() -> DeviceDescriptor {
    DeviceDescriptor {
        layout: Some(layout()),
        device_type: DeviceTypeRef::Builtin {
            device: BuiltinDeviceType::Sampler,
        },
        name: "Sampler".to_owned(),
        category: DeviceCategory::Instrument,
        params: param_infos(),
        audio_inputs: 0,
        audio_outputs: 2,
        midi_input: true,
        sidechain_inputs: 0,
    }
}

/// Declarative panel (v0.2, `device-ui`; drawn by the shared device renderer).
pub fn layout() -> ether_core::protocol::layout::DeviceLayout {
    use crate::contract::{item, knob, layout, section};
    use ether_core::protocol::layout::{Widget, WidgetSize::*};
    let mut wave = item(
        Widget::SampleWaveform {
            start: Some(params::START),
            end: Some(params::END),
        },
        Medium,
    );
    wave.colspan = 4;
    layout(vec![
        section("sample", Some("Sample"), 4, 4, vec![wave]),
        section(
            "playback",
            Some("Playback"),
            2,
            3,
            vec![
                item(Widget::Choice { param: params::MODE }, Small),
                item(
                    Widget::Number {
                        param: params::ROOT_KEY,
                    },
                    Small,
                ),
                knob(params::TRANSPOSE, Medium),
            ],
        ),
        section(
            "envelope",
            Some("Envelope"),
            1,
            2,
            vec![knob(params::ATTACK, Medium), knob(params::RELEASE, Medium)],
        ),
        section(
            "output",
            Some("Output"),
            1,
            1,
            vec![knob(params::VOLUME, Large)],
        ),
    ])
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
    /// End of the played range in source frames (exclusive).
    end: f64,
    /// Transpose by key (pitched mode, outside slice mode).
    tracks_key: bool,
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
        end: 0.0,
        tracks_key: false,
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
    /// Slice markers + slice mode (swapped in place by `set_data`).
    slices: Box<SliceSettings>,
}

impl std::fmt::Debug for Sampler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sampler")
            .field("has_source", &self.source.is_some())
            .field("values", &self.values)
            .field("slices", &self.slices)
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
        Self::with_slices(source, SliceSettings::default())
    }

    /// Non-RT. A sampler with default parameters and the given slice settings.
    pub fn with_slices(source: Option<Arc<dyn AudioSource>>, slices: SliceSettings) -> Self {
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
            slices: Box::new(slices),
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

    /// Current slice settings.
    pub fn slices(&self) -> &SliceSettings {
        &self.slices
    }

    /// Playback rate of a voice on `key`.
    fn rate(&self, key: u8, tracks_key: bool) -> f64 {
        let mut semis = self.value(params::TRANSPOSE);
        if tracks_key {
            semis += key as f64 - self.value(params::ROOT_KEY).round();
        }
        2f64.powf(semis / 12.0).min(MAX_RATE)
    }

    /// Source frame range `[start, end)` a note on `key` plays (`None` = no sound: a key
    /// without a slice in slice mode, or an empty range).
    fn range(&self, key: u8, frames: u64) -> Option<(f64, f64)> {
        let len = frames as f64;
        let sr = self.sample_rate as f64;
        let (start, end) = if self.slices.enabled {
            let i = (key as usize).checked_sub(self.slices.base_note as usize)?;
            let m = &self.slices.markers;
            let start = m.get(i)?.0 * sr;
            let end = m.get(i + 1).map_or(len, |e| e.0 * sr);
            (start, end)
        } else {
            (
                self.value(params::START) * 0.01 * len,
                self.value(params::END) * 0.01 * len,
            )
        };
        let (start, end) = (start.clamp(0.0, len), end.clamp(0.0, len));
        (start.is_finite() && end > start).then_some((start, end))
    }

    fn note_on(&mut self, note_id: u32, channel: u8, key: u8, velocity: f32) {
        let Some(source) = &self.source else {
            return;
        };
        let Some((start, end)) = self.range(key, source.frames()) else {
            return;
        };
        let tracks_key = !self.slices.enabled && !self.one_shot();
        source.prefetch_hint(start as u64);
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
            pos: start,
            end,
            tracks_key,
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
        let src_frames = source.frames() as f64;
        if src_channels == 0 {
            return;
        }
        let fade = (END_FADE_MS * 0.001 * self.sample_rate as f64).max(1.0);
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
                let rate = self.rate(self.voices[vi].key, self.voices[vi].tracks_key);
                let v = &mut self.voices[vi];
                let end = v.end.min(src_frames);
                // Declick a range end inside the sample.
                let fade_from = if end < src_frames {
                    end - fade * rate
                } else {
                    end
                };
                let first = v.pos.floor();
                // Frames needed for `n` outputs, +1 for interpolation.
                let len = ((v.pos + (n - 1) as f64 * rate).floor() - first) as usize + 2;
                let len = len.min(SCRATCH);
                for (ch, buf) in self.scratch[..src_channels].iter_mut().enumerate() {
                    source.read(ch as u16, first as u64, &mut buf[..len]);
                }
                for k in 0..n {
                    if v.pos >= end {
                        v.env.kill();
                        break;
                    }
                    let idx = v.pos - first;
                    let i0 = (idx as usize).min(len - 2);
                    let frac = (idx - i0 as f64) as f32;
                    let mut amp = v.env.tick(&self.rates) * v.velocity * self.gains[k];
                    if v.pos > fade_from {
                        amp *= ((end - v.pos) / (end - fade_from)) as f32;
                    }
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

    /// Takes a boxed [`SliceSettings`] (new markers / slice mode) without touching the
    /// voices; returns the previous settings (dropped off the audio thread).
    fn set_data(&mut self, data: NodeData) -> Option<NodeData> {
        match data.downcast::<SliceSettings>() {
            Ok(new) => Some(std::mem::replace(&mut self.slices, new) as NodeData),
            Err(data) => Some(data),
        }
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

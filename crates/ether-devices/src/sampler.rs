//! Sampler: one-shot and pitched (root key) playback of a media sample.
//!
//! The sample is an [`AudioSource`] supplied by the host (already at the engine sample
//! rate). Modes:
//! - **One-shot**: every note plays the whole sample at its original pitch (plus
//!   `Transpose`); note-offs are ignored, the voice ends with the sample.
//! - **Pitched**: playback rate follows `key - Root Key` (+ `Transpose`, `Fine`); note-off
//!   starts the release.
//!
//! - **Sample range**: `Start`/`End` (percent of the sample) bound what every note plays
//!   (slice pads of a drum rack use this, `SliceCommand::ToDrumRack`).
//! - **Slice mode** (roadmap v2, `drum-rack`; [`SliceSettings`] in the device kind): note
//!   `base_note + i` plays slice `i` (`[markers[i], markers[i + 1])`, the last one to the
//!   end of the sample) at its original pitch (+ `Transpose`); other keys are ignored. The
//!   markers reach a live sampler in place through [`Node::set_data`] (a boxed
//!   [`SliceSettings`]), so an edit never cuts sounding notes.
//!
//! The instruments pass appends (all defaulting to the sound before, bit for bit):
//! - a full ADSR (`Decay`, `Sustain`), `Fine` (cents), `Velocity` sensitivity and `Voices`
//!   (1..=32, default the original 8);
//! - `Reverse`: notes play the range backwards, from `End` down to `Start`;
//! - `Loop` (pitched mode, outside slice mode): `Forward` or `Ping-Pong` between
//!   `Loop Start` and `Loop End` (percent of the sample, inside the range), through the
//!   release; `Crossfade` (percent of the loop) blends the loop's end into the material
//!   before its start (forward loops), so a loop of a non-periodic sample doesn't click;
//! - a per-voice state-variable `Filter` (`Off` / low-pass / high-pass / band-pass, 12 dB)
//!   with `Cutoff` and `Resonance`.
//!
//! Reads go through the source in chunks into pre-allocated scratch buffers, so `process`
//! never allocates. They are band-limited ([`crate::dsp::resample`]): a windowed
//! sinc whose cutoff follows the playback rate, so a sample pitched up doesn't alias (and an
//! unpitched one plays bit-exact).

use std::f32::consts::PI;
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

use crate::dsp::resample::{Kernel, MAX_HALF, Taps, Weights};
use crate::util::{self, Adsr, AdsrRates};

/// Maximum simultaneous voices (the `Voices` param picks how many are used).
pub const MAX_VOICES: usize = 32;
/// Maximum playback-rate ratio (4 octaves up).
const MAX_RATE: f64 = 16.0;
/// Render chunk size in frames (bounds the scratch buffers).
const CHUNK: usize = 64;
/// Frames read per chunk: `CHUNK · MAX_RATE`, the kernel's span on both sides and one more
/// for a fractional start.
const SCRATCH: usize = CHUNK * MAX_RATE as usize + 2 * (MAX_HALF + 1) + 2;

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
    // The instruments pass (appended):
    pub const DECAY: ParamId = ParamId(8);
    pub const SUSTAIN: ParamId = ParamId(9);
    /// Fine tuning, cents.
    pub const FINE: ParamId = ParamId(10);
    /// Velocity → level, percent (100 = the original response).
    pub const VELOCITY: ParamId = ParamId(11);
    pub const REVERSE: ParamId = ParamId(12);
    /// [`super::LOOP_MODES`].
    pub const LOOP: ParamId = ParamId(13);
    /// Loop start / end, percent of the sample.
    pub const LOOP_START: ParamId = ParamId(14);
    pub const LOOP_END: ParamId = ParamId(15);
    /// Crossfade at the loop point, percent of the loop length.
    pub const CROSSFADE: ParamId = ParamId(16);
    /// [`super::FILTER_TYPES`].
    pub const FILTER_TYPE: ParamId = ParamId(17);
    pub const CUTOFF: ParamId = ParamId(18);
    pub const RESONANCE: ParamId = ParamId(19);
    /// Polyphony (1..=[`super::MAX_VOICES`]).
    pub const VOICES: ParamId = ParamId(20);
}

const NUM_PARAMS: usize = 21;
/// Fade at a range/slice end that is not the end of the sample (declick), in ms.
const END_FADE_MS: f64 = 2.0;
/// Extra voice slots where a stolen voice fades out over [`END_FADE_MS`] while the new note
/// starts in its slot (a hard cut clicked).
const STEAL_SLOTS: usize = 8;
/// Physical voice slots.
const SLOTS: usize = MAX_VOICES + STEAL_SLOTS;
/// Polyphony before `Voices` existed (its default).
const DEFAULT_VOICES: usize = 8;

/// Mode labels in plain-value order.
pub const MODES: [&str; 2] = ["One-shot", "Pitched"];
const MODE_ONE_SHOT: usize = 0;
/// Loop mode labels in plain-value order.
pub const LOOP_MODES: [&str; 3] = ["Off", "Forward", "Ping-Pong"];
/// Filter labels in plain-value order (12 dB/oct state-variable filter).
pub const FILTER_TYPES: [&str; 4] = ["Off", "Low-pass", "High-pass", "Band-pass"];

/// Parameter list (identical for every instance).
pub fn param_infos() -> Vec<ParamInfo> {
    use util::{choice, param};
    let time = ParamScale::Power { exponent: 3.0 };
    let percent = |id, name, group, default| {
        param(
            id,
            name,
            group,
            ParamUnit::Percent,
            (0.0, 100.0, default),
            ParamScale::Linear,
        )
    };
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
        percent(6, "Start", "Playback", 0.0),
        percent(7, "End", "Playback", 100.0),
        param(
            8,
            "Decay",
            "Envelope",
            ParamUnit::Milliseconds,
            (1.0, 10000.0, 300.0),
            time,
        ),
        percent(9, "Sustain", "Envelope", 100.0),
        param(
            10,
            "Fine",
            "Playback",
            ParamUnit::Cents,
            (-100.0, 100.0, 0.0),
            ParamScale::Linear,
        ),
        percent(11, "Velocity", "Output", 100.0),
        choice(
            12,
            "Reverse",
            "Playback",
            ParamUnit::Toggle,
            &["Off", "On"],
            0,
        ),
        choice(13, "Loop", "Loop", ParamUnit::None, &LOOP_MODES, 0),
        percent(14, "Loop Start", "Loop", 0.0),
        percent(15, "Loop End", "Loop", 100.0),
        percent(16, "Crossfade", "Loop", 0.0),
        choice(17, "Filter", "Filter", ParamUnit::None, &FILTER_TYPES, 0),
        param(
            18,
            "Cutoff",
            "Filter",
            ParamUnit::Hertz,
            (20.0, 20000.0, 20000.0),
            ParamScale::Log,
        ),
        percent(19, "Resonance", "Filter", 0.0),
        param(
            20,
            "Voices",
            "Playback",
            ParamUnit::None,
            (1.0, MAX_VOICES as f64, DEFAULT_VOICES as f64),
            ParamScale::Linear,
        )
        .with_step(1.0),
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
        implemented: true,
    }
}

/// Declarative panel (v0.2; drawn by the shared device renderer): the sample
/// with its range, playback, loop, the ADSR, the filter curve and the output.
pub fn layout() -> ether_core::protocol::layout::DeviceLayout {
    use crate::contract::{item, knob, layout, section};
    use ether_core::protocol::layout::{LayoutItem, Widget, WidgetSize::*};
    let wide = |mut it: LayoutItem, span: u8| {
        it.colspan = span;
        it
    };
    let choice = |param| item(Widget::Choice { param }, Small);
    let number = |param| item(Widget::Number { param }, Small);
    let toggle = |param| item(Widget::Toggle { param }, Small);
    layout(vec![
        section(
            "sample",
            Some("Sample"),
            4,
            4,
            vec![wide(
                item(
                    Widget::SampleWaveform {
                        start: Some(params::START),
                        end: Some(params::END),
                    },
                    Medium,
                ),
                4,
            )],
        ),
        section(
            "playback",
            Some("Playback"),
            1,
            3,
            vec![
                wide(choice(params::MODE), 2),
                toggle(params::REVERSE),
                number(params::ROOT_KEY),
                number(params::VOICES),
                knob(params::TRANSPOSE, Small),
                knob(params::FINE, Small),
            ],
        ),
        section(
            "loop",
            Some("Loop"),
            1,
            3,
            vec![
                wide(choice(params::LOOP), 3),
                knob(params::LOOP_START, Small),
                knob(params::LOOP_END, Small),
                knob(params::CROSSFADE, Small),
            ],
        ),
        section(
            "envelope",
            Some("Envelope"),
            2,
            2,
            vec![wide(
                item(
                    Widget::Envelope {
                        attack: params::ATTACK,
                        decay: params::DECAY,
                        sustain: params::SUSTAIN,
                        release: params::RELEASE,
                        delay: None,
                        hold: None,
                    },
                    Medium,
                ),
                2,
            )],
        ),
        section(
            "filter",
            Some("Filter"),
            1,
            2,
            vec![wide(
                item(
                    Widget::FilterCurve {
                        cutoff: params::CUTOFF,
                        resonance: params::RESONANCE,
                        mode: Some(params::FILTER_TYPE),
                        drive: None,
                        gain: None,
                    },
                    Medium,
                ),
                2,
            )],
        ),
        section(
            "output",
            Some("Output"),
            1,
            1,
            vec![knob(params::VOLUME, Large), knob(params::VELOCITY, Small)],
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
    /// Played range in source frames: `[lo, end)`.
    lo: f64,
    end: f64,
    /// Direction: +1 forwards, -1 backwards (reverse, or a ping-pong loop's way back).
    dir: f64,
    /// Transpose by key (pitched mode, outside slice mode).
    tracks_key: bool,
    /// May loop (pitched mode, outside slice mode).
    loops: bool,
    env: Adsr,
    /// Filter integrators `[ic1, ic2]` per output channel.
    filt: [[f32; 2]; 2],
    /// Samples left of a stolen voice's fade-out (0 = not stolen).
    dying: u32,
    /// Its length.
    dying_len: u32,
}

impl Voice {
    const IDLE: Self = Self {
        note_id: 0,
        channel: 0,
        key: 0,
        velocity: 0.0,
        age: 0,
        pos: 0.0,
        lo: 0.0,
        end: 0.0,
        dir: 1.0,
        tracks_key: false,
        loops: false,
        env: Adsr::IDLE,
        filt: [[0.0; 2]; 2],
        dying: 0,
        dying_len: 1,
    };

    /// The fade-out gain of a stolen voice for this sample (1 otherwise); ends the voice
    /// when the fade does.
    #[inline]
    fn steal_gain(&mut self) -> f32 {
        if self.dying == 0 {
            return 1.0;
        }
        let g = self.dying as f32 / self.dying_len as f32;
        self.dying -= 1;
        if self.dying == 0 {
            self.env.kill();
        }
        g
    }
}

/// Per-chunk filter: TPT state-variable coefficients and the output mix.
#[derive(Clone, Copy, Debug)]
struct Filter {
    /// 0 off, 1 low, 2 high, 3 band.
    mode: usize,
    k: f32,
    a: [f32; 3],
}

impl Filter {
    fn new(mode: usize, cutoff: f32, res: f32, sr: f32) -> Self {
        let g = (PI * cutoff.clamp(10.0, sr * 0.45) / sr).tan();
        // Butterworth (k = √2) at 0 %, near self-oscillation at 100 %.
        let k = std::f32::consts::SQRT_2 * (1.0 - res) + 0.05 * res;
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        Self {
            mode,
            k,
            a: [a1, a2, g * a2],
        }
    }

    #[inline]
    fn tick(&self, s: &mut [f32; 2], x: f32) -> f32 {
        let [a1, a2, a3] = self.a;
        let v3 = x - s[1];
        let v1 = a1 * s[0] + a2 * v3;
        let v2 = s[1] + a2 * s[0] + a3 * v3;
        s[0] = 2.0 * v1 - s[0];
        s[1] = 2.0 * v2 - s[1];
        match self.mode {
            1 => v2,
            2 => x - self.k * v1 - v2,
            _ => v1,
        }
    }
}

/// Loop of a voice for one chunk, in source frames.
#[derive(Clone, Copy, Debug)]
struct Loop {
    start: f64,
    end: f64,
    ping_pong: bool,
    /// Crossfade length (frames; forward loops only, 0 = none).
    xfade: f64,
}

/// What a voice runs into next while playing (in its direction).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Edge {
    /// The end of what it plays: the voice ends.
    Stop,
    /// The loop point: it wraps (forward) or turns (ping-pong).
    Wrap,
}

/// The built-in sampler.
pub struct Sampler {
    source: Option<Arc<dyn AudioSource>>,
    values: [f64; NUM_PARAMS],
    infos: Vec<ParamInfo>,
    sample_rate: f32,
    voices: [Voice; SLOTS],
    next_age: u64,
    rates: AdsrRates,
    volume: Smoother,
    cutoff: Smoother,
    /// Per source channel (max 2) read scratch, and the loop crossfade's second read.
    scratch: [Vec<f32>; 2],
    scratch_xf: [Vec<f32>; 2],
    gains: [f32; CHUNK],
    /// Filter coefficients of each sample of the chunk (the cutoff glides per sample: per
    /// chunk, a jump stepped audibly).
    filters: [Filter; CHUNK],
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
        // Build the shared resampling kernel here, never first on the audio thread.
        let _ = Kernel::get();
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
            voices: [Voice::IDLE; SLOTS],
            next_age: 0,
            rates: AdsrRates::new(1.0, 1.0, 1.0, 100.0, 48_000.0),
            volume: Smoother::new(1.0, 20.0, 48_000.0),
            cutoff: Smoother::new(20_000.0, 20.0, 48_000.0),
            scratch: [vec![0.0; SCRATCH], vec![0.0; SCRATCH]],
            scratch_xf: [vec![0.0; SCRATCH], vec![0.0; SCRATCH]],
            gains: [0.0; CHUNK],
            filters: [Filter::new(0, 20_000.0, 0.0, 48_000.0); CHUNK],
            slices: Box::new(slices),
        };
        s.sync_all();
        s
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
        self.cutoff = Smoother::new(self.value(params::CUTOFF) as f32, 20.0, self.sample_rate);
        self.update_rates();
    }

    fn update_rates(&mut self) {
        self.rates = AdsrRates::new(
            self.value(params::ATTACK) as f32,
            self.value(params::DECAY) as f32,
            self.value(params::SUSTAIN) as f32 * 0.01,
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
            params::CUTOFF => {
                if smooth {
                    self.cutoff.set_target(v as f32);
                } else {
                    self.cutoff.set_immediate(v as f32);
                }
            }
            params::ATTACK | params::DECAY | params::SUSTAIN | params::RELEASE => {
                self.update_rates();
            }
            _ => {}
        }
    }

    fn one_shot(&self) -> bool {
        util::index(self.value(params::MODE), MODES.len()) == MODE_ONE_SHOT
    }

    fn reversed(&self) -> bool {
        self.value(params::REVERSE) >= 0.5
    }

    /// Polyphony (the `Voices` param).
    fn polyphony(&self) -> usize {
        (self.value(params::VOICES).round().max(1.0) as usize).min(MAX_VOICES)
    }

    /// Current slice settings.
    pub fn slices(&self) -> &SliceSettings {
        &self.slices
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
        let reverse = self.reversed();
        let pos = if reverse {
            (end - 1.0).max(start)
        } else {
            start
        };
        source.prefetch_hint(pos as u64);
        let limit = self.polyphony();
        let slot = self.voices[..limit]
            .iter()
            .position(|v| !v.env.is_active())
            .or_else(|| {
                self.voices[..limit]
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, v)| (!v.env.is_releasing(), v.age))
                    .map(|(i, _)| i)
            })
            .unwrap_or(0);
        if self.voices[slot].env.is_active() && self.voices[slot].dying == 0 {
            // Stealing: the old note fades out in a spare slot.
            if let Some(spare) = (limit..SLOTS).find(|&i| !self.voices[i].env.is_active()) {
                let len = (END_FADE_MS * 0.001 * f64::from(self.sample_rate)).max(1.0) as u32;
                self.voices[spare] = Voice {
                    dying: len,
                    dying_len: len,
                    ..self.voices[slot]
                };
            }
        }
        self.next_age += 1;
        let v = &mut self.voices[slot];
        *v = Voice {
            note_id,
            channel,
            key,
            velocity: velocity.clamp(0.0, 1.0),
            age: self.next_age,
            pos,
            lo: start,
            end,
            dir: if reverse { -1.0 } else { 1.0 },
            tracks_key,
            loops: tracks_key,
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
            EventKind::Param { param, value, step } => self.apply_param(param, value, !step),
            EventKind::Midi { .. } | EventKind::NoteExpression { .. } => {}
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
        let sens = self.value(params::VELOCITY) as f32 * 0.01;
        let filter_mode = util::index(self.value(params::FILTER_TYPE), FILTER_TYPES.len());
        let res = self.value(params::RESONANCE) as f32 * 0.01;
        let mut c0 = start;
        while c0 < end {
            let n = CHUNK.min(end - c0);
            for g in &mut self.gains[..n] {
                *g = self.volume.tick();
            }
            if filter_mode != 0 {
                if self.cutoff.is_smoothing() {
                    for f in &mut self.filters[..n] {
                        *f = Filter::new(filter_mode, self.cutoff.tick(), res, self.sample_rate);
                    }
                } else {
                    let f = Filter::new(filter_mode, self.cutoff.current(), res, self.sample_rate);
                    self.filters[..n].fill(f);
                }
            } else {
                for f in &mut self.filters[..n] {
                    self.cutoff.tick();
                    f.mode = 0;
                }
            }
            let filters = &self.filters;
            for v in self.voices.iter_mut() {
                if !v.env.is_active() {
                    continue;
                }
                let rate = rate_of(&self.values, v.key, v.tracks_key);
                let looped = loop_of(&self.values, v, source.frames());
                let chunk = Chunk {
                    c0,
                    n,
                    rate,
                    sens,
                    fade,
                    src_frames,
                    src_channels,
                    filters,
                    looped,
                    taps: Taps::new(rate),
                    kernel: Kernel::get(),
                };
                if v.dir > 0.0 && looped.is_none() {
                    forward(
                        v,
                        &chunk,
                        source,
                        &mut self.scratch,
                        &self.gains,
                        &self.rates,
                        outputs,
                    );
                } else {
                    let scratch = (&mut self.scratch, &mut self.scratch_xf);
                    general(
                        v,
                        &chunk,
                        source,
                        scratch,
                        &self.gains,
                        &self.rates,
                        outputs,
                    );
                }
            }
            c0 += n;
        }
    }

    fn any_active(&self) -> bool {
        self.voices.iter().any(|v| v.env.is_active())
    }
}

/// A param's plain value.
#[inline]
fn value(values: &[f64; NUM_PARAMS], id: ParamId) -> f64 {
    values[id.0 as usize]
}

/// Playback rate of a voice on `key`.
fn rate_of(values: &[f64; NUM_PARAMS], key: u8, tracks_key: bool) -> f64 {
    let mut semis = value(values, params::TRANSPOSE);
    if tracks_key {
        semis += key as f64 - value(values, params::ROOT_KEY).round();
    }
    let fine = value(values, params::FINE);
    if fine != 0.0 {
        semis += fine * 0.01;
    }
    2f64.powf(semis / 12.0).min(MAX_RATE)
}

/// The loop a looping voice plays this chunk (`None`: loop off, or empty inside the
/// voice's range). Live: moving the loop points moves sounding notes' loops.
fn loop_of(values: &[f64; NUM_PARAMS], v: &Voice, frames: u64) -> Option<Loop> {
    let mode = util::index(value(values, params::LOOP), LOOP_MODES.len());
    if mode == 0 || !v.loops {
        return None;
    }
    let len = frames as f64;
    let start = (value(values, params::LOOP_START) * 0.01 * len).clamp(v.lo, v.end);
    let end = (value(values, params::LOOP_END) * 0.01 * len).clamp(v.lo, v.end);
    let span = end - start;
    if span.is_nan() || span < 2.0 {
        return None;
    }
    let ping_pong = mode == 2;
    // The crossfade reads the material before the loop start (forward), or after its
    // end (backward): never more than the sample has.
    let room = if v.dir > 0.0 { start } else { len - end };
    let xfade = if ping_pong {
        0.0
    } else {
        (value(values, params::CROSSFADE) * 0.01 * span)
            .min(room)
            .max(0.0)
    };
    Some(Loop {
        start,
        end,
        ping_pong,
        xfade: if xfade >= 1.0 { xfade } else { 0.0 },
    })
}

/// What every voice shares in one render chunk.
struct Chunk<'a> {
    c0: usize,
    n: usize,
    rate: f64,
    sens: f32,
    /// Declick fade length (frames of output).
    fade: f64,
    src_frames: f64,
    src_channels: usize,
    /// Per-sample filter coefficients (`mode` 0: no filter).
    filters: &'a [Filter; CHUNK],
    looped: Option<Loop>,
    /// The band-limited read setup for `rate`.
    taps: Taps,
    kernel: &'static Kernel,
}

impl Chunk<'_> {
    #[inline]
    fn velocity(&self, v: &Voice) -> f32 {
        if self.sens >= 1.0 {
            v.velocity
        } else {
            1.0 - self.sens + self.sens * v.velocity
        }
    }
}

/// Add one voice sample (per source channel `s`) to the outputs at `at`, filtered.
#[inline]
fn emit(
    v: &mut Voice,
    c: &Chunk,
    outputs: &mut [&mut [f32]],
    at: usize,
    amp: f32,
    sample: impl Fn(usize) -> f32,
) {
    // One read per source channel (a mono sample feeds both outputs).
    let mut read = [0.0f32; 2];
    for (ch, r) in read[..c.src_channels.min(2)].iter_mut().enumerate() {
        *r = sample(ch);
    }
    for (o, out) in outputs.iter_mut().enumerate() {
        let mut s = read[o.min(c.src_channels - 1).min(1)];
        let f = &c.filters[at - c.c0];
        if f.mode != 0 {
            s = f.tick(&mut v.filt[o.min(1)], s);
        }
        out[at] += s * amp;
    }
}

/// A voice playing forwards without a loop.
fn forward(
    v: &mut Voice,
    c: &Chunk,
    source: &dyn AudioSource,
    scratch: &mut [Vec<f32>; 2],
    gains: &[f32; CHUNK],
    rates: &AdsrRates,
    outputs: &mut [&mut [f32]],
) {
    let (n, rate) = (c.n, c.rate);
    let end = v.end.min(c.src_frames);
    // Declick a range end inside the sample.
    let fade_from = if end < c.src_frames {
        end - c.fade * rate
    } else {
        end
    };
    // Frames needed for `n` outputs, with the kernel's span on both sides.
    let margin = c.taps.margin() as f64;
    let first = v.pos.floor() - margin;
    let len = ((v.pos + (n - 1) as f64 * rate).floor() - first) as usize + 1 + margin as usize;
    let len = len.min(SCRATCH);
    fetch(source, c.src_channels, first, len, scratch);
    let vel = c.velocity(v);
    let mut w = Weights::new();
    for (k, &gain) in gains[..n].iter().enumerate() {
        if v.pos >= end {
            v.env.kill();
            break;
        }
        c.kernel.weights(v.pos - first, &c.taps, &mut w);
        let mut amp = v.env.tick(rates) * vel * gain * v.steal_gain();
        if v.pos > fade_from {
            amp *= ((end - v.pos) / (end - fade_from)) as f32;
        }
        emit(v, c, outputs, c.c0 + k, amp, |ch| {
            w.apply(&scratch[ch][..len])
        });
        if !v.env.is_active() {
            break;
        }
        v.pos += rate;
    }
}

/// Read source frames `[first, first + len)` of every channel into `bufs` (frames before
/// the sample's start are 0).
fn fetch(
    source: &dyn AudioSource,
    channels: usize,
    first: f64,
    len: usize,
    bufs: &mut [Vec<f32>; 2],
) {
    let skip = ((-first).max(0.0) as usize).min(len);
    for (ch, buf) in bufs[..channels].iter_mut().enumerate() {
        buf[..skip].fill(0.0);
        source.read(ch as u16, first.max(0.0) as u64, &mut buf[skip..len]);
    }
}

/// A voice playing backwards, or looping: in segments that never cross a loop point, a
/// crossfade boundary or the end of what it plays, each read in one go.
fn general(
    v: &mut Voice,
    c: &Chunk,
    source: &dyn AudioSource,
    (scratch, scratch_xf): (&mut [Vec<f32>; 2], &mut [Vec<f32>; 2]),
    gains: &[f32; CHUNK],
    rates: &AdsrRates,
    outputs: &mut [&mut [f32]],
) {
    let rate = c.rate;
    let vel = c.velocity(v);
    let mut k = 0;
    // Each pass plays at least one sample or crosses an edge (bounded: a wrap lands inside
    // the loop, a stop ends the voice).
    let mut guard = 0;
    while k < c.n && v.env.is_active() && guard < 4 * CHUNK {
        guard += 1;
        let fwd = v.dir > 0.0;
        // The next edge in the voice's direction.
        // (A looping voice always heads for a loop point: forwards it starts at or before
        // the loop start, backwards at or after the loop end.)
        let (edge, at) = match c.looped {
            Some(l) if fwd => (Edge::Wrap, l.end),
            Some(l) => (Edge::Wrap, l.start),
            None if fwd => (Edge::Stop, v.end.min(c.src_frames)),
            None => (Edge::Stop, v.lo),
        };
        // Samples before the edge: forwards while pos < at, backwards while pos >= at.
        let steps = if fwd {
            ((at - v.pos) / rate).ceil().max(0.0)
        } else {
            ((v.pos - at) / rate).floor() + 1.0
        };
        if steps < 1.0 || !steps.is_finite() {
            match edge {
                Edge::Stop => v.env.kill(),
                Edge::Wrap => cross(v, c.looped.unwrap()),
            }
            continue;
        }
        // Crossfade window of a forward loop: the last `xfade` frames before the loop
        // point in the voice's direction.
        let (xf, xf_from) = match c.looped {
            Some(l) if edge == Edge::Wrap && l.xfade > 0.0 => {
                if fwd {
                    (l.xfade, l.end - l.xfade)
                } else {
                    (l.xfade, l.start + l.xfade)
                }
            }
            _ => (0.0, 0.0),
        };
        let mut seg = (steps as usize).min(c.n - k);
        let in_xf = xf > 0.0
            && if fwd {
                v.pos >= xf_from
            } else {
                v.pos < xf_from
            };
        if xf > 0.0 && !in_xf {
            let to_xf = if fwd {
                ((xf_from - v.pos) / rate).ceil()
            } else {
                ((v.pos - xf_from) / rate).floor() + 1.0
            };
            seg = seg.min((to_xf.max(1.0)) as usize);
        }
        // Read the segment (and its crossfade partner one loop away).
        let last = v.pos + v.dir * rate * (seg - 1) as f64;
        let margin = c.taps.margin() as f64;
        let first = v.pos.min(last).floor() - margin;
        let len = ((v.pos.max(last).floor() - first + margin) as usize + 1).min(SCRATCH);
        fetch(source, c.src_channels, first, len, scratch);
        let shift = match c.looped {
            Some(l) if fwd => l.start - l.end,
            Some(l) => l.end - l.start,
            None => 0.0,
        };
        // The partner's frames start on their own whole frame (the loop length may be
        // fractional); one more frame covers the offset.
        let first_xf = (first + shift).floor();
        let len_xf = (len + 1).min(SCRATCH);
        if in_xf {
            fetch(source, c.src_channels, first_xf, len_xf, scratch_xf);
        }
        // Declick the end of a range inside the sample (no loop to continue into).
        let stop_inside = edge == Edge::Stop && if fwd { at < c.src_frames } else { at > 0.0 };
        let fade_len = c.fade * rate;
        let (mut w, mut w_xf) = (Weights::new(), Weights::new());
        for j in 0..seg {
            let mut amp = v.env.tick(rates) * vel * gains[k + j] * v.steal_gain();
            if stop_inside {
                let d = if fwd { at - v.pos } else { v.pos - at };
                if d < fade_len {
                    amp *= (d / fade_len).max(0.0) as f32;
                }
            }
            let pos = v.pos;
            let t = if in_xf {
                (if fwd { pos - xf_from } else { xf_from - pos } / xf).clamp(0.0, 1.0) as f32
            } else {
                0.0
            };
            c.kernel.weights(pos - first, &c.taps, &mut w);
            if in_xf {
                c.kernel.weights(pos + shift - first_xf, &c.taps, &mut w_xf);
            }
            emit(v, c, outputs, c.c0 + k + j, amp, |ch| {
                let a = w.apply(&scratch[ch][..len]);
                if in_xf {
                    let b = w_xf.apply(&scratch_xf[ch][..len_xf]);
                    a + (b - a) * t
                } else {
                    a
                }
            });
            if !v.env.is_active() {
                return;
            }
            v.pos += v.dir * rate;
        }
        k += seg;
    }
}

/// A voice at its loop point: wrap (forward loop) or turn back (ping-pong).
fn cross(v: &mut Voice, l: Loop) {
    let span = l.end - l.start;
    if l.ping_pong {
        if v.dir > 0.0 {
            v.pos = (2.0 * l.end - 1.0 - v.pos).max(l.start);
        } else {
            v.pos = (2.0 * l.start - 1.0 - v.pos).min(l.end - 1.0);
        }
        v.dir = -v.dir;
        v.pos = v.pos.clamp(l.start, l.end - 1.0);
    } else if v.dir > 0.0 {
        v.pos -= span * ((v.pos - l.end) / span + 1.0).floor().max(1.0);
    } else {
        v.pos += span * ((l.start - v.pos) / span + 1.0).floor().max(1.0);
    }
}

impl Node for Sampler {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.sync_all();
        self.reset();
    }

    fn reset(&mut self) {
        self.voices = [Voice::IDLE; SLOTS];
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

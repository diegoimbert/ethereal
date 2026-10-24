//! Built-in `PitchCorrection`: monophonic pitch correction for voices and solo instruments.
//! The pitch is detected ([`crate::dsp::yin`], pYIN or YIN), a target note picked (the
//! nearest note of `Key` / `Scale`, or the held note of the note input), and the voice
//! shifted towards it ([`crate::dsp::psola`]: PSOLA, formants preserved by default).
//!
//! - **Detection**: on the mid signal, every ~4 ms; `Sensitivity` sets how readily a
//!   frame counts as voiced. Unvoiced stretches (breaths, consonants, silence) pass
//!   unshifted.
//! - **Target**: `Scale`, the scale note nearest to the detected pitch (a 20-cent
//!   hysteresis keeps a pitch half way between two notes from flipping); `MIDI`, the last
//!   held note of the note input (e.g. a MIDI sidechain from a MIDI track), falling back to
//!   the scale while no note is held.
//! - **Correction**: the shift needed (target − detected, semitones) glides with a
//!   one-pole of `Retune` ms (0 = instant, the hard-tuned effect; slower keeps vibrato and
//!   slides). `Humanize` lengthens the retune of held notes (up to 4× after half a second
//!   on the same target) so sustained notes keep their natural movement while note changes
//!   stay quick. `Amount` scales the correction; `Transpose` adds semitones.
//! - **Formants**: with `Preserve Formants` (on by default) the voice keeps its character
//!   whatever the shift; `Formant` moves the formants on their own (±5 semitones, deeper
//!   or brighter voice). Off, the formants follow the pitch (the classic chipmunk shift).
//! - **Latency**: fixed, [`latency_ms`] (34 ms at 48 kHz: a grain of the lowest pitch,
//!   70 Hz, read at the largest formant shift); compensated in playback, audible when
//!   monitoring live.
//! - Analysis: `AnalysisKind::Levels`, see [`Frame`].
//!
//! # Parameter ids (stable, append-only)
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Scale | `Key` | C ..= B, default C |
//! | 1 | Scale | `Scale` | Chromatic, Major, … Whole Tone (as Scale Quantize), default Chromatic |
//! | 2 | Scale | `Target` | Scale / MIDI, default Scale |
//! | 3 | Correction | `Retune` | 0 ..= 400 ms (power 2), default 40 |
//! | 4 | Correction | `Humanize` | 0 ..= 100 %, default 0 |
//! | 5 | Correction | `Amount` | 0 ..= 100 %, default 100 |
//! | 6 | Correction | `Transpose` | -12 ..= 12 semitones, default 0 |
//! | 7 | Formant | `Formant` | -5 ..= 5 semitones (steps of 1), default 0 |
//! | 8 | Formant | `Preserve Formants` | toggle, default on |
//! | 9 | Detection | `Detector` | pYIN / YIN, default pYIN |
//! | 10 | Detection | `Sensitivity` | 0 ..= 100 %, default 50 |
//! | 11 | Output | `Mix` | 0 ..= 100 %, default 100 |

use ether_core::protocol::devices::{
    DeviceCategory, DeviceDescriptor, ParamInfo, ParamScale, ParamUnit,
};
use ether_core::protocol::layout::{DeviceLayout, LayoutItem, Widget, WidgetSize};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, ParamId};
use ether_core::{
    AnalysisKind, AnalysisSink, AudioBuffers, Device, EventKind, Node, PrepareConfig,
    ProcessContext, ProcessStatus, Smoother,
};

use crate::contract::{
    FactoryPreset, choice, descriptor as build, item, knob, layout, param, section, stepped, toggle,
};
use crate::dsp::delay_line::DelayLine;
use crate::dsp::psola::{Control, Psola};
use crate::dsp::yin::{self, Detector, Method};
use crate::midi_fx::{SCALE_KINDS, scale_mask};
use crate::util;

/// Parameter ids (stable: stored in documents and automation).
pub mod params {
    use ether_core::protocol::model::ParamId;
    pub const KEY: ParamId = ParamId(0);
    pub const SCALE: ParamId = ParamId(1);
    pub const TARGET: ParamId = ParamId(2);
    pub const RETUNE: ParamId = ParamId(3);
    pub const HUMANIZE: ParamId = ParamId(4);
    pub const AMOUNT: ParamId = ParamId(5);
    pub const TRANSPOSE: ParamId = ParamId(6);
    pub const FORMANT: ParamId = ParamId(7);
    pub const PRESERVE_FORMANTS: ParamId = ParamId(8);
    pub const DETECTOR: ParamId = ParamId(9);
    pub const SENSITIVITY: ParamId = ParamId(10);
    pub const MIX: ParamId = ParamId(11);
    /// Number of params.
    pub const COUNT: usize = 12;
}
use params as p;

/// `Target` values.
pub mod target {
    pub const SCALE: usize = 0;
    pub const MIDI: usize = 1;
}
/// `Detector` values.
pub mod detector {
    pub const PYIN: usize = 0;
    pub const YIN: usize = 1;
}

const KEYS: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];
/// As Scale Quantize's `Kind` (the order of `SCALE_KINDS`).
const SCALES: [&str; 14] = [
    "Chromatic",
    "Major",
    "Minor",
    "Harmonic Minor",
    "Melodic Minor",
    "Major Pentatonic",
    "Minor Pentatonic",
    "Blues",
    "Dorian",
    "Phrygian",
    "Lydian",
    "Mixolydian",
    "Locrian",
    "Whole Tone",
];
/// Largest formant shift (semitones), and so the largest grain read speed.
pub const MAX_FORMANT_SEMITONES: f32 = 5.0;
/// Hysteresis of the scale target (semitones).
const HYSTERESIS: f32 = 0.2;
/// History: one point per this many ms, [`HISTORY`] points (4 s).
pub const HISTORY_STEP_MS: f32 = 20.0;
pub const HISTORY: usize = 200;
/// Held notes the MIDI target remembers.
const HELD: usize = 16;

/// Layout of the `Levels` frame (indices into its values).
pub struct Frame;

impl Frame {
    /// Detected pitch now (MIDI note, fractional; -1 when unvoiced).
    pub const DETECTED: usize = 0;
    /// Pitch played now (MIDI note, fractional; -1 when unvoiced).
    pub const OUTPUT: usize = 1;
    /// Target note (MIDI note; -1 when none).
    pub const TARGET: usize = 2;
    /// Correction now (cents, output − detected).
    pub const CORRECTION: usize = 3;
    /// Detection confidence 0 ..= 1.
    pub const CONFIDENCE: usize = 4;
    /// Seconds per history point.
    pub const STEP: usize = 5;
    /// `n`, the history points that follow: `n` detected, then `n` output pitches (MIDI
    /// notes, -1 = unvoiced), oldest first.
    pub const POINTS: usize = 6;
    pub const HISTORY: usize = 7;
}

/// The latency at `sample_rate` (ms).
pub fn latency_ms(sample_rate: f32) -> f32 {
    let p = psola(sample_rate);
    p.latency() as f32 * 1000.0 / sample_rate
}

fn psola(sample_rate: f32) -> Psola {
    let max_period = (sample_rate / yin::MIN_HZ).ceil() as usize;
    Psola::new(
        sample_rate,
        max_period,
        2f32.powf(MAX_FORMANT_SEMITONES / 12.0),
    )
}

/// Descriptor of the pitch correction.
pub fn descriptor(ty: BuiltinDeviceType) -> DeviceDescriptor {
    assert_eq!(ty, BuiltinDeviceType::PitchCorrection);
    let pct = |id, name: &str, group: &str, default| {
        param(
            id,
            name,
            group,
            ParamUnit::Percent,
            (0.0, 100.0, default),
            ParamScale::Linear,
        )
    };
    let mut d = build(
        BuiltinDeviceType::PitchCorrection,
        "Pitch Correction",
        DeviceCategory::AudioEffect,
        vec![
            choice(0, "Key", "Scale", &KEYS, 0),
            choice(1, "Scale", "Scale", &SCALES, 0),
            choice(2, "Target", "Scale", &["Scale", "MIDI"], target::SCALE),
            ParamInfo {
                scale: ParamScale::Power { exponent: 2.0 },
                ..param(
                    3,
                    "Retune",
                    "Correction",
                    ParamUnit::Milliseconds,
                    (0.0, 400.0, 40.0),
                    ParamScale::Linear,
                )
            },
            pct(4, "Humanize", "Correction", 0.0),
            pct(5, "Amount", "Correction", 100.0),
            stepped(
                6,
                "Transpose",
                "Correction",
                ParamUnit::Semitones,
                -12,
                12,
                0,
            ),
            stepped(
                7,
                "Formant",
                "Formant",
                ParamUnit::Semitones,
                -(MAX_FORMANT_SEMITONES as i32),
                MAX_FORMANT_SEMITONES as i32,
                0,
            ),
            toggle(8, "Preserve Formants", "Formant", true),
            choice(9, "Detector", "Detection", &["pYIN", "YIN"], detector::PYIN),
            pct(10, "Sensitivity", "Detection", 50.0),
            pct(11, "Mix", "Output", 100.0),
        ],
        2,
        2,
        // A note input: the MIDI target (usually a MIDI sidechain).
        true,
        0,
    );
    d.layout = Some(panel());
    d
}

fn panel() -> DeviceLayout {
    use WidgetSize::*;
    layout(vec![
        section(
            "pitch",
            None,
            4,
            4,
            vec![LayoutItem {
                colspan: 4,
                ..item(
                    Widget::PitchCorrection {
                        key: p::KEY,
                        scale: p::SCALE,
                    },
                    Large,
                )
            }],
        ),
        section(
            "scale",
            Some("Scale"),
            1,
            1,
            vec![
                item(Widget::Choice { param: p::KEY }, Small),
                item(Widget::Choice { param: p::SCALE }, Small),
                item(Widget::Choice { param: p::TARGET }, Small),
            ],
        ),
        section(
            "correction",
            Some("Correction"),
            2,
            4,
            vec![
                knob(p::RETUNE, Large),
                knob(p::HUMANIZE, Medium),
                knob(p::AMOUNT, Small),
                knob(p::TRANSPOSE, Small),
            ],
        ),
        section(
            "formant",
            Some("Formant"),
            1,
            2,
            vec![
                knob(p::FORMANT, Medium),
                item(
                    Widget::Toggle {
                        param: p::PRESERVE_FORMANTS,
                    },
                    Small,
                ),
            ],
        ),
        section(
            "detection",
            Some("Detection"),
            1,
            3,
            vec![
                item(Widget::Choice { param: p::DETECTOR }, Small),
                knob(p::SENSITIVITY, Small),
                knob(p::MIX, Small),
            ],
        ),
    ])
}

/// Non-RT. A new instance.
pub fn create(device: &BuiltinDevice) -> Box<dyn Device> {
    assert_eq!(device.device_type(), BuiltinDeviceType::PitchCorrection);
    Box::new(PitchCorrection::new())
}

/// `FactoryPreset` entries for `presets/pitch-correction/<slug>.etherpreset` files.
macro_rules! presets {
    ($($slug:literal),* $(,)?) => {
        &[$(FactoryPreset {
            id: concat!("pitch-correction/", $slug),
            json: include_str!(concat!("../presets/pitch-correction/", $slug, ".etherpreset")),
        }),*]
    };
}

/// Factory presets (embedded).
pub fn factory_presets(ty: BuiltinDeviceType) -> &'static [FactoryPreset] {
    match ty {
        BuiltinDeviceType::PitchCorrection => presets!(
            "natural-vocal",
            "hard-tune",
            "midi-melody",
            "deep-voice",
            "octave-down"
        ),
        _ => &[],
    }
}

/// MIDI note of a frequency.
fn note_of(hz: f32) -> f32 {
    69.0 + 12.0 * (hz / 440.0).log2()
}

/// The held notes of the note input, newest last.
#[derive(Clone, Copy)]
struct Held {
    notes: [(u32, u8); HELD],
    len: usize,
}

impl Held {
    fn event(&mut self, kind: &EventKind) {
        match *kind {
            EventKind::NoteOn { note_id, key, .. } => {
                self.release(note_id);
                if self.len == HELD {
                    self.notes.copy_within(1.., 0);
                    self.len -= 1;
                }
                self.notes[self.len] = (note_id, key);
                self.len += 1;
            }
            EventKind::NoteOff { note_id, .. } | EventKind::NoteChoke { note_id, .. } => {
                self.release(note_id);
            }
            EventKind::AllNotesOff => self.len = 0,
            _ => {}
        }
    }

    fn release(&mut self, note_id: u32) {
        if let Some(i) = self.notes[..self.len].iter().position(|h| h.0 == note_id) {
            self.notes.copy_within(i + 1..self.len, i);
            self.len -= 1;
        }
    }

    fn last(&self) -> Option<u8> {
        self.notes[..self.len].last().map(|h| h.1)
    }
}

/// The built-in pitch correction.
pub struct PitchCorrection {
    values: [f64; p::COUNT],
    ranges: [(f64, f64); p::COUNT],
    sample_rate: f32,
    detector: Detector,
    psola: Psola,
    dry: [DelayLine; 2],
    mix: Smoother,
    held: Held,
    /// The current target note and how long it has been the target (seconds).
    target: Option<i32>,
    target_age: f32,
    /// Smoothed correction (semitones), and whether the last estimate was voiced.
    correction: f32,
    was_voiced: bool,
    // Readouts and history.
    detected: f32,
    output: f32,
    confidence: f32,
    hist_detected: Vec<f32>,
    hist_output: Vec<f32>,
    hist_at: usize,
    hist_len: usize,
    point_len: usize,
    point_pos: usize,
    tail: util::Tail,
}

impl Default for PitchCorrection {
    fn default() -> Self {
        Self::new()
    }
}

impl PitchCorrection {
    /// Non-RT.
    pub fn new() -> Self {
        let d = descriptor(BuiltinDeviceType::PitchCorrection);
        let mut values = [0.0; p::COUNT];
        let mut ranges = [(0.0, 0.0); p::COUNT];
        for info in &d.params {
            values[info.id.0 as usize] = info.default;
            ranges[info.id.0 as usize] = (info.min, info.max);
        }
        let mut s = Self {
            values,
            ranges,
            sample_rate: 48_000.0,
            detector: Detector::new(48_000.0, Method::PYin),
            psola: psola(48_000.0),
            dry: [DelayLine::default(), DelayLine::default()],
            mix: Smoother::new(1.0, 20.0, 48_000.0),
            held: Held {
                notes: [(0, 0); HELD],
                len: 0,
            },
            target: None,
            target_age: 0.0,
            correction: 0.0,
            was_voiced: false,
            detected: -1.0,
            output: -1.0,
            confidence: 0.0,
            hist_detected: vec![-1.0; HISTORY],
            hist_output: vec![-1.0; HISTORY],
            hist_at: 0,
            hist_len: 0,
            point_len: 960,
            point_pos: 0,
            tail: util::Tail::default(),
        };
        s.allocate();
        s
    }

    fn value(&self, id: ParamId) -> f32 {
        self.values[id.0 as usize] as f32
    }

    fn index(&self, id: ParamId, n: usize) -> usize {
        util::index(self.values[id.0 as usize], n)
    }

    fn allocate(&mut self) {
        let sr = self.sample_rate;
        self.detector = Detector::new(sr, self.method());
        self.detector
            .set_sensitivity(self.value(p::SENSITIVITY) * 0.01);
        self.psola = psola(sr);
        self.psola.wait_for_controls(true);
        let l = self.psola.latency();
        self.dry = [DelayLine::new(l + 1), DelayLine::new(l + 1)];
        self.mix = Smoother::new(self.value(p::MIX) * 0.01, 20.0, sr);
        self.point_len = ((HISTORY_STEP_MS * 0.001 * sr).round() as usize).max(1);
        self.reset_state();
    }

    fn method(&self) -> Method {
        if self.index(p::DETECTOR, 2) == detector::YIN {
            Method::Yin
        } else {
            Method::PYin
        }
    }

    fn reset_state(&mut self) {
        self.detector.reset();
        self.psola.reset();
        for d in &mut self.dry {
            d.clear();
        }
        self.target = None;
        self.target_age = 0.0;
        self.correction = 0.0;
        self.was_voiced = false;
        self.detected = -1.0;
        self.output = -1.0;
        self.confidence = 0.0;
    }

    fn apply(&mut self, id: ParamId, value: f64, smooth: bool) {
        let Some(&(min, max)) = self.ranges.get(id.0 as usize) else {
            return;
        };
        self.values[id.0 as usize] = util::clamp(value, min, max);
        match id {
            p::MIX => {
                let v = self.value(p::MIX) * 0.01;
                if smooth {
                    self.mix.set_target(v);
                } else {
                    self.mix.set_immediate(v);
                }
            }
            p::DETECTOR => self.detector.set_method(self.method()),
            p::SENSITIVITY => self
                .detector
                .set_sensitivity(self.value(p::SENSITIVITY) * 0.01),
            _ => {}
        }
    }

    /// The scale note nearest to `note` (with hysteresis around the current target).
    fn scale_target(&self, note: f32) -> i32 {
        let key = self.index(p::KEY, 12) as i32;
        let mask = scale_mask(SCALE_KINDS[self.index(p::SCALE, SCALE_KINDS.len())]);
        let in_scale = |n: i32| mask & (1 << (n - key).rem_euclid(12)) != 0;
        let base = note.round() as i32;
        let mut best = base;
        let mut dist = f32::INFINITY;
        for n in base - 6..=base + 6 {
            if in_scale(n) {
                let d = (note - n as f32).abs();
                if d < dist {
                    dist = d;
                    best = n;
                }
            }
        }
        match self.target {
            Some(t) if in_scale(t) && (note - t as f32).abs() < dist + HYSTERESIS => t,
            _ => best,
        }
    }

    /// RT. A new estimate: pick the target, move the correction, hand the shifter its
    /// control from the analysed span's centre on.
    fn on_estimate(&mut self, est: yin::Estimate) {
        let hop = self.detector.hop_samples() as f32 / self.sample_rate;
        let delay = if est.voiced {
            est.delay as u64
        } else {
            self.detector.centre_delay() as u64
        };
        let at = self.psola.position().saturating_sub(delay);
        self.confidence = est.confidence;
        if !est.voiced {
            self.was_voiced = false;
            self.detected = -1.0;
            self.output = -1.0;
            self.psola.control(Control {
                at,
                period: 0.0,
                voiced: false,
                ratio: 1.0,
                formant: 1.0,
            });
            return;
        }
        let note = note_of(est.hz);
        let midi = (self.index(p::TARGET, 2) == target::MIDI)
            .then(|| self.held.last())
            .flatten();
        let target = midi.map_or_else(|| self.scale_target(note), i32::from);
        if Some(target) == self.target {
            self.target_age += hop;
        } else {
            self.target = Some(target);
            self.target_age = 0.0;
        }
        let wanted = target as f32 - note;
        if !self.was_voiced {
            // A new phrase starts from its own pitch and glides in.
            self.correction = if self.value(p::RETUNE) <= 0.0 {
                wanted
            } else {
                0.0
            };
        }
        self.was_voiced = true;
        let humanize = self.value(p::HUMANIZE) * 0.01;
        let retune =
            self.value(p::RETUNE) * (1.0 + 3.0 * humanize * (self.target_age / 0.5).min(1.0));
        self.correction = if retune <= 0.0 {
            wanted
        } else {
            let k = (-hop * 1000.0 / retune).exp();
            wanted + (self.correction - wanted) * k
        };
        let shift = self.correction * self.value(p::AMOUNT) * 0.01 + self.value(p::TRANSPOSE);
        let ratio = 2f32.powf(shift / 12.0);
        let formant = 2f32.powf(self.value(p::FORMANT) / 12.0);
        let formant = if self.value(p::PRESERVE_FORMANTS) >= 0.5 {
            formant
        } else {
            formant * ratio
        };
        self.detected = note;
        self.output = note + shift;
        self.psola.control(Control {
            at,
            period: est.period,
            voiced: true,
            ratio,
            formant,
        });
    }

    fn render(&mut self, audio: &mut AudioBuffers<'_, '_>, start: usize, end: usize) {
        let inputs = audio.inputs;
        let input = |c: usize, i: usize| {
            inputs
                .get(c)
                .or_else(|| inputs.first())
                .map_or(0.0, |ch| if ch[i].is_finite() { ch[i] } else { 0.0 })
        };
        let l = self.psola.latency();
        for i in start..end {
            let x = [input(0, i), input(1, i)];
            if let Some(est) = self.detector.push(0.5 * (x[0] + x[1])) {
                self.on_estimate(est);
            }
            let wet = self.psola.process(x);
            let mix = self.mix.tick();
            for (c, out) in audio.outputs.iter_mut().enumerate() {
                if c >= 2 {
                    out[i] = 0.0;
                    continue;
                }
                let dry = self.dry[c].tap(l);
                self.dry[c].push(x[c]);
                out[i] = dry + (wet[c] - dry) * mix;
            }
            self.point_pos += 1;
            if self.point_pos >= self.point_len {
                self.point_pos = 0;
                self.hist_detected[self.hist_at] = self.detected;
                self.hist_output[self.hist_at] = self.output;
                self.hist_at = (self.hist_at + 1) % HISTORY;
                self.hist_len = (self.hist_len + 1).min(HISTORY);
            }
        }
    }
}

impl Node for PitchCorrection {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(8_000.0);
        self.allocate();
    }

    fn reset(&mut self) {
        self.reset_state();
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let frames = ctx.frames;
        util::split_at_events(
            self,
            ctx.events,
            frames,
            |s, a, b| s.render(audio, a, b),
            |s, kind| match *kind {
                EventKind::Param { param, value, step } => s.apply(param, value, !step),
                ref other => s.held.event(other),
            },
        );
        let hold = self.psola.latency() + util::samples(0.1, self.sample_rate);
        self.tail
            .status_of(ctx, audio, !self.mix.is_smoothing(), hold)
    }

    fn latency(&self) -> u32 {
        self.psola.latency() as u32
    }

    fn has_analysis(&self) -> bool {
        true
    }

    fn analysis(&mut self, out: &mut AnalysisSink<'_>) {
        let Some(f) = out.frame(AnalysisKind::Levels) else {
            return;
        };
        let target = if self.detected >= 0.0 {
            self.target.map_or(-1.0, |t| t as f32)
        } else {
            -1.0
        };
        let correction = if self.detected >= 0.0 {
            (self.output - self.detected) * 100.0
        } else {
            0.0
        };
        for v in [
            self.detected,
            self.output,
            target,
            correction,
            self.confidence,
            HISTORY_STEP_MS * 0.001,
            self.hist_len as f32,
        ] {
            f.push(v);
        }
        for ring in [&self.hist_detected, &self.hist_output] {
            for k in (0..self.hist_len).rev() {
                f.push(ring[(self.hist_at + HISTORY - 1 - k) % HISTORY]);
            }
        }
    }
}

impl Device for PitchCorrection {
    fn descriptor(&self) -> DeviceDescriptor {
        descriptor(BuiltinDeviceType::PitchCorrection)
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.values.get(id.0 as usize).copied()
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.apply(id, value, false);
    }
}

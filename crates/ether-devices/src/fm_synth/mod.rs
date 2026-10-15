//! The FM Synth (`BuiltinDeviceType::FmSynth`, node `fm-synth`): four operators A–D in one
//! of 11 algorithms ([`algorithms`]), each a band-limited oscillator (sine, triangle, saw,
//! square, noise) at a frequency ratio of the note (or a fixed frequency) with its own ADSR,
//! level, velocity sensitivity and self-feedback; a voice filter with its own envelope, one
//! LFO (pitch / filter / volume), glide, pitch bend, a tone control and 2x oversampling
//! against FM aliasing. Modelled on the architecture of classic 4-operator FM instruments;
//! the DSP is our own (phase modulation, see `engine.rs`).
//!
//! **Param ids are stable and append-only** (documents, automation and presets store
//! them): never renumber, only append.
//!
//! # Params
//!
//! Operator `o` (0 = A … 3 = D) owns ids `13·o + field` ([`fm_synth::op`]):
//!
//! | field | name | range |
//! |-------|------|-------|
//! | 0 | `On` | toggle, default on |
//! | 1 | `Wave` | Sine / Triangle / Saw / Square / Noise (default Sine) |
//! | 2 | `Coarse` | frequency ratio 0.5, 1 ..= 48 (stepped; plain 0 = 0.5), default 1 |
//! | 3 | `Fine` | 0 ..= 1000 thousandths of the ratio, default 0 |
//! | 4 | `Fixed` | toggle: play `Frequency` whatever the note, default off |
//! | 5 | `Frequency` | 1 ..= 10000 Hertz (log), default 440 |
//! | 6 | `Level` | -70 ..= 0 dB (fader; -70 = silent), default 0 (A) / -70 (B, C, D) |
//! | 7 | `Attack` | 0 ..= 20000 Milliseconds, default 2 |
//! | 8 | `Decay` | 1 ..= 60000 Milliseconds, default 600 |
//! | 9 | `Sustain` | 0 ..= 100 Percent, default 100 |
//! | 10 | `Release` | 1 ..= 60000 Milliseconds, default 300 |
//! | 11 | `Velocity` | 0 ..= 100 Percent (level velocity sensitivity), default 50 |
//! | 12 | `Feedback` | 0 ..= 100 Percent (self phase modulation), default 0 |
//!
//! Global params:
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 52 | Algorithm | `Algorithm` | the 11 routings of [`algorithms::ALGORITHMS`] (default 1) |
//! | 53 | Pitch | `Transpose` | -48 ..= 48 Semitones (stepped), default 0 |
//! | 54 | Pitch | `Fine` | -100 ..= 100 Cents, default 0 |
//! | 55 | Voice | `Voices` | 1 ..= 32 (stepped), default 8 |
//! | 56 | Voice | `Glide` | 0 ..= 2000 Milliseconds, default 0 |
//! | 57 | Voice | `Bend Range` | 0 ..= 24 Semitones (stepped), default 2 |
//! | 58 | LFO | `Shape` | Sine / Triangle / Saw / Square / Sample & Hold (default Sine) |
//! | 59 | LFO | `Rate` | 0.01 ..= 50 Hertz (log), default 5 |
//! | 60 | LFO | `To Pitch` | 0 ..= 100 Percent (squared; 100 % = ±12 semitones), default 0 |
//! | 61 | LFO | `To Filter` | -100 ..= 100 Percent (±4 octaves), default 0 |
//! | 62 | LFO | `To Volume` | 0 ..= 100 Percent (tremolo depth), default 0 |
//! | 63 | Filter | `On` | toggle, default off |
//! | 64 | Filter | `Type` | Low-pass 12 / Low-pass 24 / High-pass 12 / High-pass 24 / Band-pass 12 / Band-pass 24 (default Low-pass 24) |
//! | 65 | Filter | `Cutoff` | 20 ..= 20000 Hertz (log), default 8000 |
//! | 66 | Filter | `Resonance` | 0 ..= 100 Percent, default 10 |
//! | 67 | Filter | `Env Amount` | -100 ..= 100 Percent (±8 octaves), default 0 |
//! | 68 | Filter | `Key Tracking` | 0 ..= 100 Percent, default 0 |
//! | 69–72 | Filter Envelope | `Attack` / `Decay` / `Sustain` / `Release` | as the operators', defaults 5 ms / 400 ms / 0 % / 300 ms |
//! | 73 | Output | `Volume` | -70 ..= 6 dB (fader), default -6 |
//! | 74 | Output | `Tone` | 0 ..= 100 Percent (output low-pass 1 kHz … open), default 100 |
//! | 75 | Output | `Antialias` | toggle: 2x oversampled operators, default on |
//! | 76 | Voice | `Mode` | Poly / Mono / Legato (default Poly) |
//!
//! `Mode` (as on the Poly Synth): `Mono` plays one voice and retriggers the envelopes on
//! every note (gliding when `Glide` > 0); `Legato` plays one voice, an overlapping note
//! takes it over without retriggering (and glides), a detached note retriggers without
//! gliding. Both use last-note priority: releasing the newest key returns to the most
//! recent key still held.

pub mod algorithms;
mod engine;

pub use engine::{FmSynth, MAX_POLYPHONY};

use ether_core::Device;
use ether_core::protocol::devices::{
    DeviceCategory, DeviceDescriptor, ParamInfo, ParamScale, ParamUnit,
};
use ether_core::protocol::layout::{DeviceLayout, LayoutItem};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, ParamId};

use crate::contract::{
    FactoryPreset, choice, descriptor as build, item, knob, layout as layout_of, param, section,
    stepped, toggle,
};

/// Param ids of `FmSynth` (stable, append-only).
#[allow(clippy::module_inception)]
pub mod fm_synth {
    use ether_core::protocol::model::ParamId;

    /// Fields of an operator's params: its id is `op_param(o, field)`.
    pub mod op {
        pub const ON: u32 = 0;
        pub const WAVE: u32 = 1;
        pub const COARSE: u32 = 2;
        pub const FINE: u32 = 3;
        pub const FIXED: u32 = 4;
        pub const FREQUENCY: u32 = 5;
        pub const LEVEL: u32 = 6;
        pub const ATTACK: u32 = 7;
        pub const DECAY: u32 = 8;
        pub const SUSTAIN: u32 = 9;
        pub const RELEASE: u32 = 10;
        pub const VELOCITY: u32 = 11;
        pub const FEEDBACK: u32 = 12;
        /// Params per operator.
        pub const STRIDE: u32 = 13;
    }

    /// Param `field` (see [`op`]) of operator `o` (0 = A … 3 = D).
    pub const fn op_param(o: usize, field: u32) -> ParamId {
        ParamId(o as u32 * op::STRIDE + field)
    }

    pub const ALGORITHM: ParamId = ParamId(52);
    pub const TRANSPOSE: ParamId = ParamId(53);
    pub const FINE: ParamId = ParamId(54);
    pub const VOICES: ParamId = ParamId(55);
    pub const GLIDE: ParamId = ParamId(56);
    pub const BEND_RANGE: ParamId = ParamId(57);
    pub const LFO_SHAPE: ParamId = ParamId(58);
    pub const LFO_RATE: ParamId = ParamId(59);
    pub const LFO_PITCH: ParamId = ParamId(60);
    pub const LFO_FILTER: ParamId = ParamId(61);
    pub const LFO_VOLUME: ParamId = ParamId(62);
    pub const FILTER_ON: ParamId = ParamId(63);
    pub const FILTER_TYPE: ParamId = ParamId(64);
    pub const CUTOFF: ParamId = ParamId(65);
    pub const RESONANCE: ParamId = ParamId(66);
    pub const FILTER_ENV_AMOUNT: ParamId = ParamId(67);
    pub const KEY_TRACKING: ParamId = ParamId(68);
    pub const FILTER_ATTACK: ParamId = ParamId(69);
    pub const FILTER_DECAY: ParamId = ParamId(70);
    pub const FILTER_SUSTAIN: ParamId = ParamId(71);
    pub const FILTER_RELEASE: ParamId = ParamId(72);
    pub const VOLUME: ParamId = ParamId(73);
    pub const TONE: ParamId = ParamId(74);
    pub const ANTIALIAS: ParamId = ParamId(75);
    /// Voice mode: Poly / Mono / Legato ([`super::VOICE_MODES`]).
    pub const VOICE_MODE: ParamId = ParamId(76);
    /// Number of params.
    pub const COUNT: usize = 77;
}

/// Operator names, by index.
pub const OP_NAMES: [&str; algorithms::OPS] = ["A", "B", "C", "D"];

/// Operator waveforms (the `Wave` choices).
pub const WAVES: [&str; 5] = ["Sine", "Triangle", "Saw", "Square", "Noise"];

/// Voice modes (the `Mode` choices; as the Poly Synth's).
pub const VOICE_MODES: [&str; 3] = ["Poly", "Mono", "Legato"];

/// Filter types (the filter `Type` choices).
pub const FILTER_TYPES: [&str; 6] = [
    "Low-pass 12",
    "Low-pass 24",
    "High-pass 12",
    "High-pass 24",
    "Band-pass 12",
    "Band-pass 24",
];

fn envelope_params(first: u32, group: &str, defaults: (f64, f64, f64, f64)) -> [ParamInfo; 4] {
    let time = |id, name, (min, max), default| {
        param(
            id,
            name,
            group,
            ParamUnit::Milliseconds,
            (min, max, default),
            ParamScale::Power { exponent: 3.0 },
        )
    };
    [
        time(first, "Attack", (0.0, 20_000.0), defaults.0),
        time(first + 1, "Decay", (1.0, 60_000.0), defaults.1),
        param(
            first + 2,
            "Sustain",
            group,
            ParamUnit::Percent,
            (0.0, 100.0, defaults.2),
            ParamScale::Linear,
        ),
        time(first + 3, "Release", (1.0, 60_000.0), defaults.3),
    ]
}

fn operator_params(o: usize) -> Vec<ParamInfo> {
    use fm_synth::op;
    let id = |field: u32| fm_synth::op_param(o, field).0;
    let group = format!("Operator {}", OP_NAMES[o]);
    let g = group.as_str();
    let mut coarse = stepped(id(op::COARSE), "Coarse", g, ParamUnit::Ratio, 0, 48, 1);
    if let Some(labels) = coarse.labels.as_mut() {
        labels[0] = "0.5".to_owned();
    }
    let mut out = vec![
        toggle(id(op::ON), "On", g, true),
        choice(id(op::WAVE), "Wave", g, &WAVES, 0),
        coarse,
        param(
            id(op::FINE),
            "Fine",
            g,
            ParamUnit::None,
            (0.0, 1000.0, 0.0),
            ParamScale::Linear,
        ),
        toggle(id(op::FIXED), "Fixed", g, false),
        param(
            id(op::FREQUENCY),
            "Frequency",
            g,
            ParamUnit::Hertz,
            (1.0, 10_000.0, 440.0),
            ParamScale::Log,
        ),
        param(
            id(op::LEVEL),
            "Level",
            g,
            ParamUnit::Decibels,
            (-70.0, 0.0, if o == 0 { 0.0 } else { -70.0 }),
            ParamScale::Fader,
        ),
    ];
    out.extend(envelope_params(
        id(op::ATTACK),
        g,
        (2.0, 600.0, 100.0, 300.0),
    ));
    out.push(param(
        id(op::VELOCITY),
        "Velocity",
        g,
        ParamUnit::Percent,
        (0.0, 100.0, 50.0),
        ParamScale::Linear,
    ));
    out.push(param(
        id(op::FEEDBACK),
        "Feedback",
        g,
        ParamUnit::Percent,
        (0.0, 100.0, 0.0),
        ParamScale::Linear,
    ));
    out
}

/// Descriptor of the FM Synth.
///
/// # Panics
/// For another type.
pub fn descriptor(ty: BuiltinDeviceType) -> DeviceDescriptor {
    assert_eq!(
        ty,
        BuiltinDeviceType::FmSynth,
        "{ty:?} is not an `fm-synth` device"
    );
    let mut params: Vec<ParamInfo> = (0..algorithms::OPS).flat_map(operator_params).collect();
    params.push(choice(
        52,
        "Algorithm",
        "Algorithm",
        &algorithms::labels(),
        0,
    ));
    params.push(stepped(
        53,
        "Transpose",
        "Pitch",
        ParamUnit::Semitones,
        -48,
        48,
        0,
    ));
    params.push(param(
        54,
        "Fine",
        "Pitch",
        ParamUnit::Cents,
        (-100.0, 100.0, 0.0),
        ParamScale::Linear,
    ));
    params.push(stepped(55, "Voices", "Voice", ParamUnit::None, 1, 32, 8));
    params.push(param(
        56,
        "Glide",
        "Voice",
        ParamUnit::Milliseconds,
        (0.0, 2000.0, 0.0),
        ParamScale::Power { exponent: 3.0 },
    ));
    params.push(stepped(
        57,
        "Bend Range",
        "Voice",
        ParamUnit::Semitones,
        0,
        24,
        2,
    ));
    params.push(choice(
        58,
        "Shape",
        "LFO",
        &["Sine", "Triangle", "Saw", "Square", "Sample & Hold"],
        0,
    ));
    params.push(param(
        59,
        "Rate",
        "LFO",
        ParamUnit::Hertz,
        (0.01, 50.0, 5.0),
        ParamScale::Log,
    ));
    params.push(param(
        60,
        "To Pitch",
        "LFO",
        ParamUnit::Percent,
        (0.0, 100.0, 0.0),
        ParamScale::Linear,
    ));
    params.push(param(
        61,
        "To Filter",
        "LFO",
        ParamUnit::Percent,
        (-100.0, 100.0, 0.0),
        ParamScale::Linear,
    ));
    params.push(param(
        62,
        "To Volume",
        "LFO",
        ParamUnit::Percent,
        (0.0, 100.0, 0.0),
        ParamScale::Linear,
    ));
    params.push(toggle(63, "On", "Filter", false));
    params.push(choice(64, "Type", "Filter", &FILTER_TYPES, 1));
    params.push(param(
        65,
        "Cutoff",
        "Filter",
        ParamUnit::Hertz,
        (20.0, 20_000.0, 8000.0),
        ParamScale::Log,
    ));
    params.push(param(
        66,
        "Resonance",
        "Filter",
        ParamUnit::Percent,
        (0.0, 100.0, 10.0),
        ParamScale::Linear,
    ));
    params.push(param(
        67,
        "Env Amount",
        "Filter",
        ParamUnit::Percent,
        (-100.0, 100.0, 0.0),
        ParamScale::Linear,
    ));
    params.push(param(
        68,
        "Key Tracking",
        "Filter",
        ParamUnit::Percent,
        (0.0, 100.0, 0.0),
        ParamScale::Linear,
    ));
    params.extend(envelope_params(
        69,
        "Filter Envelope",
        (5.0, 400.0, 0.0, 300.0),
    ));
    params.push(param(
        73,
        "Volume",
        "Output",
        ParamUnit::Decibels,
        // -14 dB (#520): at -6 a fresh five-note chord peaked at +5.6 dBFS; now -2.4,
        // level with the synth and the poly synth.
        (-70.0, 6.0, -14.0),
        ParamScale::Fader,
    ));
    params.push(param(
        74,
        "Tone",
        "Output",
        ParamUnit::Percent,
        (0.0, 100.0, 100.0),
        ParamScale::Linear,
    ));
    params.push(toggle(75, "Antialias", "Output", true));
    params.push(choice(76, "Mode", "Voice", &VOICE_MODES, 0));
    debug_assert_eq!(params.len(), fm_synth::COUNT);
    let mut d = build(
        BuiltinDeviceType::FmSynth,
        "FM Synth",
        DeviceCategory::Instrument,
        params,
        0,
        2,
        true,
        0,
    );
    d.layout = Some(layout());
    d
}

/// Declarative panel: the four operators side by side (a column each: wave, tuning,
/// level, envelope), then the algorithm picker, pitch / voice, LFO, filter with its
/// envelope and output. Typed widgets own the params they bind (their controls row).
pub fn layout() -> DeviceLayout {
    use ether_core::protocol::layout::{Widget, WidgetSize::*};
    use fm_synth::{self as p, op, op_param};
    let wide = |mut it: LayoutItem, span: u8| {
        it.colspan = span;
        it
    };
    let named = |mut it: LayoutItem, label: &str| {
        it.label = Some(label.to_owned());
        it
    };
    let choice = |param: ParamId| item(Widget::Choice { param }, Small);
    let toggle = |param: ParamId| item(Widget::Toggle { param }, Small);
    let number = |param: ParamId| item(Widget::Number { param }, Small);
    let envelope = |a: ParamId| {
        item(
            Widget::Envelope {
                attack: a,
                decay: ParamId(a.0 + 1),
                sustain: ParamId(a.0 + 2),
                release: ParamId(a.0 + 3),
                delay: None,
                hold: None,
            },
            Medium,
        )
    };
    let operator = |o: usize| {
        let f = |field| op_param(o, field);
        section(
            &format!("op-{}", OP_NAMES[o].to_ascii_lowercase()),
            Some(&format!("Operator {}", OP_NAMES[o])),
            1,
            4,
            vec![
                toggle(f(op::ON)),
                wide(choice(f(op::WAVE)), 3),
                number(f(op::COARSE)),
                knob(f(op::FINE), Small),
                toggle(f(op::FIXED)),
                named(knob(f(op::FREQUENCY), Small), "Freq"),
                knob(f(op::LEVEL), Medium),
                knob(f(op::VELOCITY), Small),
                knob(f(op::FEEDBACK), Small),
                wide(envelope(f(op::ATTACK)), 4),
            ],
        )
    };
    // The operators side by side first, then the algorithm beside the global sections.
    let mut sections: Vec<_> = (0..algorithms::OPS).map(operator).collect();
    sections.push(section(
        "algorithm",
        Some("Algorithm"),
        1,
        1,
        vec![item(
            Widget::FmAlgorithm {
                algorithm: p::ALGORITHM,
                algorithms: algorithms::ALGORITHMS.iter().map(|a| a.routing()).collect(),
            },
            Medium,
        )],
    ));
    sections.extend([
        section(
            "pitch-voice",
            Some("Pitch / Voice"),
            1,
            2,
            vec![
                number(p::TRANSPOSE),
                knob(p::FINE, Small),
                wide(choice(p::VOICE_MODE), 2),
                named(number(p::VOICES), "Polyphony"),
                number(p::BEND_RANGE),
                wide(knob(p::GLIDE, Small), 2),
            ],
        ),
        section(
            "lfo",
            Some("LFO"),
            1,
            3,
            vec![
                wide(
                    item(
                        Widget::Lfo {
                            shape: p::LFO_SHAPE,
                            rate: p::LFO_RATE,
                            amount: None,
                        },
                        Medium,
                    ),
                    3,
                ),
                named(knob(p::LFO_PITCH, Small), "Pitch"),
                named(knob(p::LFO_FILTER, Small), "Filter"),
                named(knob(p::LFO_VOLUME, Small), "Volume"),
            ],
        ),
        section(
            "filter",
            Some("Filter"),
            2,
            3,
            vec![
                wide(
                    item(
                        Widget::FilterCurve {
                            cutoff: p::CUTOFF,
                            resonance: p::RESONANCE,
                            mode: Some(p::FILTER_TYPE),
                            drive: None,
                            gain: None,
                        },
                        Large,
                    ),
                    3,
                ),
                toggle(p::FILTER_ON),
                knob(p::FILTER_ENV_AMOUNT, Small),
                knob(p::KEY_TRACKING, Small),
            ],
        ),
        section(
            "filter-env",
            Some("Filter Envelope"),
            1,
            2,
            vec![wide(envelope(p::FILTER_ATTACK), 2)],
        ),
        section(
            "output",
            Some("Output"),
            1,
            2,
            vec![
                wide(knob(p::VOLUME, Large), 2),
                knob(p::TONE, Small),
                toggle(p::ANTIALIAS),
            ],
        ),
    ]);
    layout_of(sections)
}

/// Non-RT. A new instance.
pub fn create(device: &BuiltinDevice) -> Box<dyn Device> {
    debug_assert_eq!(device.device_type(), BuiltinDeviceType::FmSynth);
    Box::new(FmSynth::new())
}

/// `preset!("e-piano")` → the embedded `presets/fm-synth/e-piano.etherpreset`.
macro_rules! preset {
    ($slug:literal) => {
        FactoryPreset {
            id: concat!("fm-synth/", $slug),
            json: include_str!(concat!("../../presets/fm-synth/", $slug, ".etherpreset")),
        }
    };
}

/// Factory presets (keys, bell, bass, brass, pad), embedded.
static FACTORY_PRESETS: [FactoryPreset; 5] = [
    preset!("e-piano"),
    preset!("bell"),
    preset!("bass"),
    preset!("brass"),
    preset!("pad"),
];

/// Factory presets of the FM Synth.
pub fn factory_presets(ty: BuiltinDeviceType) -> &'static [FactoryPreset] {
    match ty {
        BuiltinDeviceType::FmSynth => &FACTORY_PRESETS,
        _ => &[],
    }
}

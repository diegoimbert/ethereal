//! v0.2 devices owned by the `synth-2` node (contracts-3 froze the param tables; see
//! docs/ROADMAP.md "v0.2" and the device agent guide there).
//!
//! The polyphonic synth: 2 oscillators (virtual-analog shapes + wavetable position), sub, noise, multimode filter with drive, amp/filter/mod envelopes, 2 LFOs, unison, glide, voice modes.
//!
//! Every device here starts as a [`Placeholder`] (pass-through / silent / MIDI-thru) with its
//! final descriptor. **Param ids are stable and append-only** (documents, automation and
//! presets store them): never renumber, only append. Split this module into files as you like.
//!
//! # Poly Synth (`BuiltinDeviceType::PolySynth`)
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Osc 1 | `Type` | Saw / Square / Triangle / Sine / Wavetable (default Saw) |
//! | 1 | Osc 1 | `Position` | 0 ..= 100 Percent, default 0 |
//! | 2 | Osc 1 | `Octave` | -3 ..= 3 (stepped), default 0 |
//! | 3 | Osc 1 | `Semitones` | -12 ..= 12 (stepped), default 0 |
//! | 4 | Osc 1 | `Fine` | -1 ..= 1 Semitones, default 0 |
//! | 5 | Osc 1 | `Pulse Width` | 5 ..= 95 Percent, default 50 |
//! | 6 | Osc 1 | `Level` | -70 ..= 6 dB (fader), default 0 |
//! | 7 | Osc 2 | `Type` | Saw / Square / Triangle / Sine / Wavetable (default Saw) |
//! | 8 | Osc 2 | `Position` | 0 ..= 100 Percent, default 0 |
//! | 9 | Osc 2 | `Octave` | -3 ..= 3 (stepped), default 0 |
//! | 10 | Osc 2 | `Semitones` | -12 ..= 12 (stepped), default 0 |
//! | 11 | Osc 2 | `Fine` | -1 ..= 1 Semitones, default 0 |
//! | 12 | Osc 2 | `Pulse Width` | 5 ..= 95 Percent, default 50 |
//! | 13 | Osc 2 | `Level` | -70 ..= 6 dB (fader), default -70 |
//! | 14 | Sub / Noise | `Sub Level` | -70 ..= 6 dB (fader), default -70 |
//! | 15 | Sub / Noise | `Sub Octave` | -1 / -2 (default -1) |
//! | 16 | Sub / Noise | `Noise Level` | -70 ..= 6 dB (fader), default -70 |
//! | 17 | Sub / Noise | `Noise Color` | 0 ..= 100 Percent, default 50 |
//! | 18 | Unison | `Unison` | 1 ..= 8 (stepped), default 1 |
//! | 19 | Unison | `Detune` | 0 ..= 100 Percent, default 20 |
//! | 20 | Unison | `Spread` | 0 ..= 100 Percent, default 50 |
//! | 21 | Voice | `Voices` | 1 ..= 32 (stepped), default 16 |
//! | 22 | Voice | `Mode` | Poly / Mono / Legato (default Poly) |
//! | 23 | Voice | `Glide` | 0 ..= 2000 Milliseconds, default 0 |
//! | 24 | Voice | `Bend Range` | 0 ..= 24 (stepped), default 2 |
//! | 25 | Filter | `Type` | Low-pass 24 / Low-pass 12 / High-pass 12 / Band-pass 12 / Notch (default Low-pass 24) |
//! | 26 | Filter | `Cutoff` | 20 ..= 20000 Hertz (log), default 8000 |
//! | 27 | Filter | `Resonance` | 0 ..= 100 Percent, default 10 |
//! | 28 | Filter | `Drive` | 0 ..= 24 Decibels, default 0 |
//! | 29 | Filter | `Key Tracking` | 0 ..= 100 Percent, default 0 |
//! | 30 | Filter | `Env Amount` | -100 ..= 100 Percent, default 0 |
//! | 31 | Amp Envelope | `Attack` | 0 ..= 10000 Milliseconds, default 5 |
//! | 32 | Amp Envelope | `Decay` | 1 ..= 10000 Milliseconds, default 200 |
//! | 33 | Amp Envelope | `Sustain` | 0 ..= 100 Percent, default 70 |
//! | 34 | Amp Envelope | `Release` | 1 ..= 20000 Milliseconds, default 300 |
//! | 35 | Filter Envelope | `Attack` | 0 ..= 10000 Milliseconds, default 5 |
//! | 36 | Filter Envelope | `Decay` | 1 ..= 10000 Milliseconds, default 400 |
//! | 37 | Filter Envelope | `Sustain` | 0 ..= 100 Percent, default 0 |
//! | 38 | Filter Envelope | `Release` | 1 ..= 20000 Milliseconds, default 300 |
//! | 39 | Mod Envelope | `Attack` | 0 ..= 10000 Milliseconds, default 5 |
//! | 40 | Mod Envelope | `Decay` | 1 ..= 10000 Milliseconds, default 400 |
//! | 41 | Mod Envelope | `Sustain` | 0 ..= 100 Percent, default 0 |
//! | 42 | Mod Envelope | `Release` | 1 ..= 20000 Milliseconds, default 300 |
//! | 43 | Mod Envelope | `Target` | Off / Pitch / Osc 2 Pitch / Wavetable Position / Pulse Width / Cutoff (default Off) |
//! | 44 | Mod Envelope | `Amount` | -100 ..= 100 Percent, default 0 |
//! | 45 | LFO 1 | `Shape` | Sine / Triangle / Saw / Square / Sample & Hold (default Sine) |
//! | 46 | LFO 1 | `Rate` | 0.01 ..= 50 Hertz (log), default 2 |
//! | 47 | LFO 1 | `Sync` | toggle, default off |
//! | 48 | LFO 1 | `Sync Rate` | sync rate (default index 8) |
//! | 49 | LFO 1 | `Target` | Off / Pitch / Cutoff / Amp / Pan / Wavetable Position / Pulse Width (default Off) |
//! | 50 | LFO 1 | `Amount` | -100 ..= 100 Percent, default 0 |
//! | 51 | LFO 2 | `Shape` | Sine / Triangle / Saw / Square / Sample & Hold (default Sine) |
//! | 52 | LFO 2 | `Rate` | 0.01 ..= 50 Hertz (log), default 2 |
//! | 53 | LFO 2 | `Sync` | toggle, default off |
//! | 54 | LFO 2 | `Sync Rate` | sync rate (default index 8) |
//! | 55 | LFO 2 | `Target` | Off / Pitch / Cutoff / Amp / Pan / Wavetable Position / Pulse Width (default Off) |
//! | 56 | LFO 2 | `Amount` | -100 ..= 100 Percent, default 0 |
//! | 57 | Output | `Volume` | -70 ..= 6 dB (fader), default -6 |
//! | 58 | Output | `Velocity` | 0 ..= 100 Percent, default 50 |

use ether_core::Device;
use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor, ParamScale, ParamUnit};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType};

#[allow(unused_imports)]
use crate::contract::{
    FactoryPreset, Placeholder, PlaceholderMode, SYNC_RATES, choice, descriptor as build, param,
    stepped, toggle,
};

/// Param ids of `PolySynth` (stable, append-only).
#[allow(clippy::module_inception)]
pub mod poly_synth {
    use ether_core::protocol::model::ParamId;
    pub const OSC1_TYPE: ParamId = ParamId(0);
    pub const OSC1_POSITION: ParamId = ParamId(1);
    pub const OSC1_OCTAVE: ParamId = ParamId(2);
    pub const OSC1_SEMITONES: ParamId = ParamId(3);
    pub const OSC1_FINE: ParamId = ParamId(4);
    pub const OSC1_PULSE_WIDTH: ParamId = ParamId(5);
    pub const OSC1_LEVEL: ParamId = ParamId(6);
    pub const OSC2_TYPE: ParamId = ParamId(7);
    pub const OSC2_POSITION: ParamId = ParamId(8);
    pub const OSC2_OCTAVE: ParamId = ParamId(9);
    pub const OSC2_SEMITONES: ParamId = ParamId(10);
    pub const OSC2_FINE: ParamId = ParamId(11);
    pub const OSC2_PULSE_WIDTH: ParamId = ParamId(12);
    pub const OSC2_LEVEL: ParamId = ParamId(13);
    pub const SUB_LEVEL: ParamId = ParamId(14);
    pub const SUB_OCTAVE: ParamId = ParamId(15);
    pub const NOISE_LEVEL: ParamId = ParamId(16);
    pub const NOISE_COLOR: ParamId = ParamId(17);
    pub const UNISON_VOICES: ParamId = ParamId(18);
    pub const UNISON_DETUNE: ParamId = ParamId(19);
    pub const UNISON_SPREAD: ParamId = ParamId(20);
    pub const VOICES: ParamId = ParamId(21);
    pub const VOICE_MODE: ParamId = ParamId(22);
    pub const GLIDE: ParamId = ParamId(23);
    pub const BEND_RANGE: ParamId = ParamId(24);
    pub const FILTER_TYPE: ParamId = ParamId(25);
    pub const CUTOFF: ParamId = ParamId(26);
    pub const RESONANCE: ParamId = ParamId(27);
    pub const DRIVE: ParamId = ParamId(28);
    pub const KEY_TRACKING: ParamId = ParamId(29);
    pub const FILTER_ENV_AMOUNT: ParamId = ParamId(30);
    pub const AMP_ATTACK: ParamId = ParamId(31);
    pub const AMP_DECAY: ParamId = ParamId(32);
    pub const AMP_SUSTAIN: ParamId = ParamId(33);
    pub const AMP_RELEASE: ParamId = ParamId(34);
    pub const FILTER_ATTACK: ParamId = ParamId(35);
    pub const FILTER_DECAY: ParamId = ParamId(36);
    pub const FILTER_SUSTAIN: ParamId = ParamId(37);
    pub const FILTER_RELEASE: ParamId = ParamId(38);
    pub const MOD_ATTACK: ParamId = ParamId(39);
    pub const MOD_DECAY: ParamId = ParamId(40);
    pub const MOD_SUSTAIN: ParamId = ParamId(41);
    pub const MOD_RELEASE: ParamId = ParamId(42);
    pub const MOD_ENV_TARGET: ParamId = ParamId(43);
    pub const MOD_ENV_AMOUNT: ParamId = ParamId(44);
    pub const LFO1_SHAPE: ParamId = ParamId(45);
    pub const LFO1_RATE: ParamId = ParamId(46);
    pub const LFO1_SYNC: ParamId = ParamId(47);
    pub const LFO1_SYNC_RATE: ParamId = ParamId(48);
    pub const LFO1_TARGET: ParamId = ParamId(49);
    pub const LFO1_AMOUNT: ParamId = ParamId(50);
    pub const LFO2_SHAPE: ParamId = ParamId(51);
    pub const LFO2_RATE: ParamId = ParamId(52);
    pub const LFO2_SYNC: ParamId = ParamId(53);
    pub const LFO2_SYNC_RATE: ParamId = ParamId(54);
    pub const LFO2_TARGET: ParamId = ParamId(55);
    pub const LFO2_AMOUNT: ParamId = ParamId(56);
    pub const VOLUME: ParamId = ParamId(57);
    pub const VELOCITY: ParamId = ParamId(58);
    /// Number of params.
    pub const COUNT: usize = 59;
}

/// Descriptor of a type of this group.
///
/// # Panics
/// For a type of another group.
pub fn descriptor(ty: BuiltinDeviceType) -> DeviceDescriptor {
    match ty {
        BuiltinDeviceType::PolySynth => build(
            BuiltinDeviceType::PolySynth,
            "Poly Synth",
            DeviceCategory::Instrument,
            vec![
                choice(
                    0,
                    "Type",
                    "Osc 1",
                    &["Saw", "Square", "Triangle", "Sine", "Wavetable"],
                    0,
                ),
                param(
                    1,
                    "Position",
                    "Osc 1",
                    ParamUnit::Percent,
                    (0.0, 100.0, 0.0),
                    ParamScale::Linear,
                ),
                stepped(2, "Octave", "Osc 1", ParamUnit::None, -3, 3, 0),
                stepped(3, "Semitones", "Osc 1", ParamUnit::Semitones, -12, 12, 0),
                param(
                    4,
                    "Fine",
                    "Osc 1",
                    ParamUnit::Semitones,
                    (-1.0, 1.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    5,
                    "Pulse Width",
                    "Osc 1",
                    ParamUnit::Percent,
                    (5.0, 95.0, 50.0),
                    ParamScale::Linear,
                ),
                param(
                    6,
                    "Level",
                    "Osc 1",
                    ParamUnit::Decibels,
                    (-70.0, 6.0, 0.0),
                    ParamScale::Fader,
                ),
                choice(
                    7,
                    "Type",
                    "Osc 2",
                    &["Saw", "Square", "Triangle", "Sine", "Wavetable"],
                    0,
                ),
                param(
                    8,
                    "Position",
                    "Osc 2",
                    ParamUnit::Percent,
                    (0.0, 100.0, 0.0),
                    ParamScale::Linear,
                ),
                stepped(9, "Octave", "Osc 2", ParamUnit::None, -3, 3, 0),
                stepped(10, "Semitones", "Osc 2", ParamUnit::Semitones, -12, 12, 0),
                param(
                    11,
                    "Fine",
                    "Osc 2",
                    ParamUnit::Semitones,
                    (-1.0, 1.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    12,
                    "Pulse Width",
                    "Osc 2",
                    ParamUnit::Percent,
                    (5.0, 95.0, 50.0),
                    ParamScale::Linear,
                ),
                param(
                    13,
                    "Level",
                    "Osc 2",
                    ParamUnit::Decibels,
                    (-70.0, 6.0, -70.0),
                    ParamScale::Fader,
                ),
                param(
                    14,
                    "Sub Level",
                    "Sub / Noise",
                    ParamUnit::Decibels,
                    (-70.0, 6.0, -70.0),
                    ParamScale::Fader,
                ),
                choice(15, "Sub Octave", "Sub / Noise", &["-1", "-2"], 0),
                param(
                    16,
                    "Noise Level",
                    "Sub / Noise",
                    ParamUnit::Decibels,
                    (-70.0, 6.0, -70.0),
                    ParamScale::Fader,
                ),
                param(
                    17,
                    "Noise Color",
                    "Sub / Noise",
                    ParamUnit::Percent,
                    (0.0, 100.0, 50.0),
                    ParamScale::Linear,
                ),
                stepped(18, "Unison", "Unison", ParamUnit::None, 1, 8, 1),
                param(
                    19,
                    "Detune",
                    "Unison",
                    ParamUnit::Percent,
                    (0.0, 100.0, 20.0),
                    ParamScale::Linear,
                ),
                param(
                    20,
                    "Spread",
                    "Unison",
                    ParamUnit::Percent,
                    (0.0, 100.0, 50.0),
                    ParamScale::Linear,
                ),
                stepped(21, "Voices", "Voice", ParamUnit::None, 1, 32, 16),
                choice(22, "Mode", "Voice", &["Poly", "Mono", "Legato"], 0),
                param(
                    23,
                    "Glide",
                    "Voice",
                    ParamUnit::Milliseconds,
                    (0.0, 2000.0, 0.0),
                    ParamScale::Power { exponent: 3.0 },
                ),
                stepped(24, "Bend Range", "Voice", ParamUnit::Semitones, 0, 24, 2),
                choice(
                    25,
                    "Type",
                    "Filter",
                    &[
                        "Low-pass 24",
                        "Low-pass 12",
                        "High-pass 12",
                        "Band-pass 12",
                        "Notch",
                    ],
                    0,
                ),
                param(
                    26,
                    "Cutoff",
                    "Filter",
                    ParamUnit::Hertz,
                    (20.0, 20000.0, 8000.0),
                    ParamScale::Log,
                ),
                param(
                    27,
                    "Resonance",
                    "Filter",
                    ParamUnit::Percent,
                    (0.0, 100.0, 10.0),
                    ParamScale::Linear,
                ),
                param(
                    28,
                    "Drive",
                    "Filter",
                    ParamUnit::Decibels,
                    (0.0, 24.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    29,
                    "Key Tracking",
                    "Filter",
                    ParamUnit::Percent,
                    (0.0, 100.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    30,
                    "Env Amount",
                    "Filter",
                    ParamUnit::Percent,
                    (-100.0, 100.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    31,
                    "Attack",
                    "Amp Envelope",
                    ParamUnit::Milliseconds,
                    (0.0, 10000.0, 5.0),
                    ParamScale::Power { exponent: 3.0 },
                ),
                param(
                    32,
                    "Decay",
                    "Amp Envelope",
                    ParamUnit::Milliseconds,
                    (1.0, 10000.0, 200.0),
                    ParamScale::Power { exponent: 3.0 },
                ),
                param(
                    33,
                    "Sustain",
                    "Amp Envelope",
                    ParamUnit::Percent,
                    (0.0, 100.0, 70.0),
                    ParamScale::Linear,
                ),
                param(
                    34,
                    "Release",
                    "Amp Envelope",
                    ParamUnit::Milliseconds,
                    (1.0, 20000.0, 300.0),
                    ParamScale::Power { exponent: 3.0 },
                ),
                param(
                    35,
                    "Attack",
                    "Filter Envelope",
                    ParamUnit::Milliseconds,
                    (0.0, 10000.0, 5.0),
                    ParamScale::Power { exponent: 3.0 },
                ),
                param(
                    36,
                    "Decay",
                    "Filter Envelope",
                    ParamUnit::Milliseconds,
                    (1.0, 10000.0, 400.0),
                    ParamScale::Power { exponent: 3.0 },
                ),
                param(
                    37,
                    "Sustain",
                    "Filter Envelope",
                    ParamUnit::Percent,
                    (0.0, 100.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    38,
                    "Release",
                    "Filter Envelope",
                    ParamUnit::Milliseconds,
                    (1.0, 20000.0, 300.0),
                    ParamScale::Power { exponent: 3.0 },
                ),
                param(
                    39,
                    "Attack",
                    "Mod Envelope",
                    ParamUnit::Milliseconds,
                    (0.0, 10000.0, 5.0),
                    ParamScale::Power { exponent: 3.0 },
                ),
                param(
                    40,
                    "Decay",
                    "Mod Envelope",
                    ParamUnit::Milliseconds,
                    (1.0, 10000.0, 400.0),
                    ParamScale::Power { exponent: 3.0 },
                ),
                param(
                    41,
                    "Sustain",
                    "Mod Envelope",
                    ParamUnit::Percent,
                    (0.0, 100.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    42,
                    "Release",
                    "Mod Envelope",
                    ParamUnit::Milliseconds,
                    (1.0, 20000.0, 300.0),
                    ParamScale::Power { exponent: 3.0 },
                ),
                choice(
                    43,
                    "Target",
                    "Mod Envelope",
                    &[
                        "Off",
                        "Pitch",
                        "Osc 2 Pitch",
                        "Wavetable Position",
                        "Pulse Width",
                        "Cutoff",
                    ],
                    0,
                ),
                param(
                    44,
                    "Amount",
                    "Mod Envelope",
                    ParamUnit::Percent,
                    (-100.0, 100.0, 0.0),
                    ParamScale::Linear,
                ),
                choice(
                    45,
                    "Shape",
                    "LFO 1",
                    &["Sine", "Triangle", "Saw", "Square", "Sample & Hold"],
                    0,
                ),
                param(
                    46,
                    "Rate",
                    "LFO 1",
                    ParamUnit::Hertz,
                    (0.01, 50.0, 2.0),
                    ParamScale::Log,
                ),
                toggle(47, "Sync", "LFO 1", false),
                choice(48, "Sync Rate", "LFO 1", &SYNC_RATES, 8),
                choice(
                    49,
                    "Target",
                    "LFO 1",
                    &[
                        "Off",
                        "Pitch",
                        "Cutoff",
                        "Amp",
                        "Pan",
                        "Wavetable Position",
                        "Pulse Width",
                    ],
                    0,
                ),
                param(
                    50,
                    "Amount",
                    "LFO 1",
                    ParamUnit::Percent,
                    (-100.0, 100.0, 0.0),
                    ParamScale::Linear,
                ),
                choice(
                    51,
                    "Shape",
                    "LFO 2",
                    &["Sine", "Triangle", "Saw", "Square", "Sample & Hold"],
                    0,
                ),
                param(
                    52,
                    "Rate",
                    "LFO 2",
                    ParamUnit::Hertz,
                    (0.01, 50.0, 2.0),
                    ParamScale::Log,
                ),
                toggle(53, "Sync", "LFO 2", false),
                choice(54, "Sync Rate", "LFO 2", &SYNC_RATES, 8),
                choice(
                    55,
                    "Target",
                    "LFO 2",
                    &[
                        "Off",
                        "Pitch",
                        "Cutoff",
                        "Amp",
                        "Pan",
                        "Wavetable Position",
                        "Pulse Width",
                    ],
                    0,
                ),
                param(
                    56,
                    "Amount",
                    "LFO 2",
                    ParamUnit::Percent,
                    (-100.0, 100.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    57,
                    "Volume",
                    "Output",
                    ParamUnit::Decibels,
                    (-70.0, 6.0, -6.0),
                    ParamScale::Fader,
                ),
                param(
                    58,
                    "Velocity",
                    "Output",
                    ParamUnit::Percent,
                    (0.0, 100.0, 50.0),
                    ParamScale::Linear,
                ),
            ],
            0,
            2,
            true,
            0,
        ),
        other => unreachable!("{other:?} is not a `synth-2` device"),
    }
}

/// Non-RT. A new instance (placeholder until implemented).
pub fn create(device: &BuiltinDevice) -> Box<dyn Device> {
    let ty = device.device_type();
    let mode = PlaceholderMode::Silent;
    Box::new(Placeholder::new(descriptor(ty), mode))
}

/// Factory presets of a type of this group (embedded; add `FactoryPreset { id, json:
/// include_str!("../../presets/<device-key>/<slug>.etherpreset") }` entries).
pub fn factory_presets(ty: BuiltinDeviceType) -> &'static [FactoryPreset] {
    let _ = ty;
    &[]
}

//! v0.2 devices owned by the `multisampler` node (contracts-3 froze the param tables; see
//! docs/ROADMAP.md "v0.2" and the device agent guide there).
//!
//! The multisampler: zones (`BuiltinDevice::MultiSampler::zones`, `ether_model::multisampler`) with key/velocity ranges, round robin, loop points; global ADSR and filter.
//!
//! Implementation in progress (node `multisampler`): zones, velocity layers, round robin.
//! final descriptor. **Param ids are stable and append-only** (documents, automation and
//! presets store them): never renumber, only append. Split this module into files as you like.
//!
//! # Multisampler (`BuiltinDeviceType::MultiSampler`)
//!
//! Zones are device-kind data (see `ether_model::multisampler` for the zone selection rules); live zone edits arrive through `Node::set_data` (`Vec<SampleZone>` + resolved sources).
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Output | `Volume` | -70 ..= 6 dB (fader), default -6 |
//! | 1 | Pitch | `Transpose` | -48 ..= 48 (stepped), default 0 |
//! | 2 | Pitch | `Fine` | -1 ..= 1 Semitones, default 0 |
//! | 3 | Voice | `Voices` | 1 ..= 64 (stepped), default 32 |
//! | 4 | Voice | `Glide` | 0 ..= 2000 Milliseconds, default 0 |
//! | 5 | Voice | `Velocity` | 0 ..= 100 Percent, default 100 |
//! | 6 | Amp Envelope | `Attack` | 0 ..= 10000 Milliseconds, default 1 |
//! | 7 | Amp Envelope | `Decay` | 1 ..= 10000 Milliseconds, default 200 |
//! | 8 | Amp Envelope | `Sustain` | 0 ..= 100 Percent, default 100 |
//! | 9 | Amp Envelope | `Release` | 1 ..= 20000 Milliseconds, default 100 |
//! | 10 | Filter | `Type` | Off / Low-pass 24 / Low-pass 12 / High-pass 12 / Band-pass 12 (default Off) |
//! | 11 | Filter | `Cutoff` | 20 ..= 20000 Hertz (log), default 20000 |
//! | 12 | Filter | `Resonance` | 0 ..= 100 Percent, default 0 |
//! | 13 | Filter | `Key Tracking` | 0 ..= 100 Percent, default 0 |
//! | 14 | Filter | `Env Amount` | -100 ..= 100 Percent, default 0 |
//! | 15 | Filter Envelope | `Attack` | 0 ..= 10000 Milliseconds, default 1 |
//! | 16 | Filter Envelope | `Decay` | 1 ..= 10000 Milliseconds, default 400 |
//! | 17 | Filter Envelope | `Sustain` | 0 ..= 100 Percent, default 0 |
//! | 18 | Filter Envelope | `Release` | 1 ..= 20000 Milliseconds, default 300 |
//! | 19 | Voice | `Round Robin` | Cycle / Random (default Cycle) |
//! | 20 | Output | `Pan` | -1 ..= 1 Pan, default 0 |

use ether_core::Device;
use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor, ParamScale, ParamUnit};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType};

#[allow(unused_imports)]
use crate::contract::{
    FactoryPreset, Placeholder, PlaceholderMode, SYNC_RATES, choice, descriptor as build, param,
    stepped, toggle,
};

/// Param ids of `MultiSampler` (stable, append-only).
pub mod multi_sampler {
    use ether_core::protocol::model::ParamId;
    pub const VOLUME: ParamId = ParamId(0);
    pub const TRANSPOSE: ParamId = ParamId(1);
    pub const FINE: ParamId = ParamId(2);
    pub const VOICES: ParamId = ParamId(3);
    pub const GLIDE: ParamId = ParamId(4);
    pub const VELOCITY: ParamId = ParamId(5);
    pub const AMP_ATTACK: ParamId = ParamId(6);
    pub const AMP_DECAY: ParamId = ParamId(7);
    pub const AMP_SUSTAIN: ParamId = ParamId(8);
    pub const AMP_RELEASE: ParamId = ParamId(9);
    pub const FILTER_TYPE: ParamId = ParamId(10);
    pub const CUTOFF: ParamId = ParamId(11);
    pub const RESONANCE: ParamId = ParamId(12);
    pub const KEY_TRACKING: ParamId = ParamId(13);
    pub const FILTER_ENV_AMOUNT: ParamId = ParamId(14);
    pub const FILTER_ATTACK: ParamId = ParamId(15);
    pub const FILTER_DECAY: ParamId = ParamId(16);
    pub const FILTER_SUSTAIN: ParamId = ParamId(17);
    pub const FILTER_RELEASE: ParamId = ParamId(18);
    pub const ROUND_ROBIN: ParamId = ParamId(19);
    pub const PAN: ParamId = ParamId(20);
    /// Number of params.
    pub const COUNT: usize = 21;
}

/// Descriptor of a type of this group.
///
/// # Panics
/// For a type of another group.
pub fn descriptor(ty: BuiltinDeviceType) -> DeviceDescriptor {
    match ty {
        BuiltinDeviceType::MultiSampler => build(
            BuiltinDeviceType::MultiSampler,
            "Multisampler",
            DeviceCategory::Instrument,
            vec![
                param(
                    0,
                    "Volume",
                    "Output",
                    ParamUnit::Decibels,
                    (-70.0, 6.0, -6.0),
                    ParamScale::Fader,
                ),
                stepped(1, "Transpose", "Pitch", ParamUnit::Semitones, -48, 48, 0),
                param(
                    2,
                    "Fine",
                    "Pitch",
                    ParamUnit::Semitones,
                    (-1.0, 1.0, 0.0),
                    ParamScale::Linear,
                ),
                stepped(3, "Voices", "Voice", ParamUnit::None, 1, 64, 32),
                param(
                    4,
                    "Glide",
                    "Voice",
                    ParamUnit::Milliseconds,
                    (0.0, 2000.0, 0.0),
                    ParamScale::Power { exponent: 3.0 },
                ),
                param(
                    5,
                    "Velocity",
                    "Voice",
                    ParamUnit::Percent,
                    (0.0, 100.0, 100.0),
                    ParamScale::Linear,
                ),
                param(
                    6,
                    "Attack",
                    "Amp Envelope",
                    ParamUnit::Milliseconds,
                    (0.0, 10000.0, 1.0),
                    ParamScale::Power { exponent: 3.0 },
                ),
                param(
                    7,
                    "Decay",
                    "Amp Envelope",
                    ParamUnit::Milliseconds,
                    (1.0, 10000.0, 200.0),
                    ParamScale::Power { exponent: 3.0 },
                ),
                param(
                    8,
                    "Sustain",
                    "Amp Envelope",
                    ParamUnit::Percent,
                    (0.0, 100.0, 100.0),
                    ParamScale::Linear,
                ),
                param(
                    9,
                    "Release",
                    "Amp Envelope",
                    ParamUnit::Milliseconds,
                    (1.0, 20000.0, 100.0),
                    ParamScale::Power { exponent: 3.0 },
                ),
                choice(
                    10,
                    "Type",
                    "Filter",
                    &[
                        "Off",
                        "Low-pass 24",
                        "Low-pass 12",
                        "High-pass 12",
                        "Band-pass 12",
                    ],
                    0,
                ),
                param(
                    11,
                    "Cutoff",
                    "Filter",
                    ParamUnit::Hertz,
                    (20.0, 20000.0, 20000.0),
                    ParamScale::Log,
                ),
                param(
                    12,
                    "Resonance",
                    "Filter",
                    ParamUnit::Percent,
                    (0.0, 100.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    13,
                    "Key Tracking",
                    "Filter",
                    ParamUnit::Percent,
                    (0.0, 100.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    14,
                    "Env Amount",
                    "Filter",
                    ParamUnit::Percent,
                    (-100.0, 100.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    15,
                    "Attack",
                    "Filter Envelope",
                    ParamUnit::Milliseconds,
                    (0.0, 10000.0, 1.0),
                    ParamScale::Power { exponent: 3.0 },
                ),
                param(
                    16,
                    "Decay",
                    "Filter Envelope",
                    ParamUnit::Milliseconds,
                    (1.0, 10000.0, 400.0),
                    ParamScale::Power { exponent: 3.0 },
                ),
                param(
                    17,
                    "Sustain",
                    "Filter Envelope",
                    ParamUnit::Percent,
                    (0.0, 100.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    18,
                    "Release",
                    "Filter Envelope",
                    ParamUnit::Milliseconds,
                    (1.0, 20000.0, 300.0),
                    ParamScale::Power { exponent: 3.0 },
                ),
                choice(19, "Round Robin", "Voice", &["Cycle", "Random"], 0),
                param(
                    20,
                    "Pan",
                    "Output",
                    ParamUnit::Pan,
                    (-1.0, 1.0, 0.0),
                    ParamScale::Linear,
                ),
            ],
            0,
            2,
            true,
            0,
        ),
        other => unreachable!("{other:?} is not a `multisampler` device"),
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

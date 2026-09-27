//! v0.2 devices owned by the `fx-analysis` node (contracts-3 froze the param tables; see
//! docs/ROADMAP.md "v0.2" and the device agent guide there).
//!
//! Analysis devices (audio passes through untouched): spectrum analyzer and tuner. They publish `AnalysisKind::{Spectrum, Tuner}` frames through `Node::{has_analysis, analysis}` (`ether_core::analysis`).
//!
//! Every device here starts as a [`Placeholder`] (pass-through / silent / MIDI-thru) with its
//! final descriptor. **Param ids are stable and append-only** (documents, automation and
//! presets store them): never renumber, only append. Split this module into files as you like.
//!
//! # Spectrum (`BuiltinDeviceType::SpectrumAnalyzer`)
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Analyzer | `Block Size` | 1024 / 2048 / 4096 / 8192 (default 4096) |
//! | 1 | Analyzer | `Averaging` | 0 ..= 100 Percent, default 60 |
//! | 2 | Analyzer | `Channel` | Stereo / Left / Right / Mid / Side (default Stereo) |
//! | 3 | Display | `Range` | -120 ..= -24 Decibels, default -90 |
//! | 4 | Display | `Slope` | 0 ..= 6 None, default 3 |
//! | 5 | Display | `Peak Hold` | toggle, default off |
//!
//! # Tuner (`BuiltinDeviceType::Tuner`)
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Tuner | `Reference` | 415 ..= 466 Hertz, default 440 |
//! | 1 | Tuner | `Input` | Stereo / Left / Right (default Stereo) |
//! | 2 | Tuner | `Mute Output` | toggle, default off |

use ether_core::Device;
use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor, ParamScale, ParamUnit};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType};

#[allow(unused_imports)]
use crate::contract::{
    FactoryPreset, Placeholder, PlaceholderMode, SYNC_RATES, choice, descriptor as build, param,
    stepped, toggle,
};

/// Param ids of `SpectrumAnalyzer` (stable, append-only).
pub mod spectrum_analyzer {
    use ether_core::protocol::model::ParamId;
    pub const BLOCK_SIZE: ParamId = ParamId(0);
    pub const AVERAGING: ParamId = ParamId(1);
    pub const CHANNEL: ParamId = ParamId(2);
    pub const RANGE: ParamId = ParamId(3);
    pub const SLOPE: ParamId = ParamId(4);
    pub const PEAK_HOLD: ParamId = ParamId(5);
    /// Number of params.
    pub const COUNT: usize = 6;
}

/// Param ids of `Tuner` (stable, append-only).
pub mod tuner {
    use ether_core::protocol::model::ParamId;
    pub const REFERENCE: ParamId = ParamId(0);
    pub const INPUT: ParamId = ParamId(1);
    pub const MUTE: ParamId = ParamId(2);
    /// Number of params.
    pub const COUNT: usize = 3;
}

/// Descriptor of a type of this group.
///
/// # Panics
/// For a type of another group.
pub fn descriptor(ty: BuiltinDeviceType) -> DeviceDescriptor {
    match ty {
        BuiltinDeviceType::SpectrumAnalyzer => build(
            BuiltinDeviceType::SpectrumAnalyzer,
            "Spectrum",
            DeviceCategory::AudioEffect,
            vec![
                choice(
                    0,
                    "Block Size",
                    "Analyzer",
                    &["1024", "2048", "4096", "8192"],
                    2,
                ),
                param(
                    1,
                    "Averaging",
                    "Analyzer",
                    ParamUnit::Percent,
                    (0.0, 100.0, 60.0),
                    ParamScale::Linear,
                ),
                choice(
                    2,
                    "Channel",
                    "Analyzer",
                    &["Stereo", "Left", "Right", "Mid", "Side"],
                    0,
                ),
                param(
                    3,
                    "Range",
                    "Display",
                    ParamUnit::Decibels,
                    (-120.0, -24.0, -90.0),
                    ParamScale::Linear,
                ),
                param(
                    4,
                    "Slope",
                    "Display",
                    ParamUnit::None,
                    (0.0, 6.0, 3.0),
                    ParamScale::Linear,
                ),
                toggle(5, "Peak Hold", "Display", false),
            ],
            2,
            2,
            false,
            0,
        ),
        BuiltinDeviceType::Tuner => build(
            BuiltinDeviceType::Tuner,
            "Tuner",
            DeviceCategory::AudioEffect,
            vec![
                param(
                    0,
                    "Reference",
                    "Tuner",
                    ParamUnit::Hertz,
                    (415.0, 466.0, 440.0),
                    ParamScale::Linear,
                ),
                choice(1, "Input", "Tuner", &["Stereo", "Left", "Right"], 0),
                toggle(2, "Mute Output", "Tuner", false),
            ],
            2,
            2,
            false,
            0,
        ),
        other => unreachable!("{other:?} is not a `fx-analysis` device"),
    }
}

/// Non-RT. A new instance (placeholder until implemented).
pub fn create(device: &BuiltinDevice) -> Box<dyn Device> {
    let ty = device.device_type();
    let mode = PlaceholderMode::PassThrough;
    Box::new(Placeholder::new(descriptor(ty), mode))
}

/// Factory presets of a type of this group (embedded; add `FactoryPreset { id, json:
/// include_str!("../../presets/<device-key>/<slug>.etherpreset") }` entries).
pub fn factory_presets(ty: BuiltinDeviceType) -> &'static [FactoryPreset] {
    let _ = ty;
    &[]
}

//! v0.2 devices owned by the `fx-modulation` node (contracts-3 froze the param tables; see
//! docs/ROADMAP.md "v0.2" and the device agent guide there).
//!
//! Modulation effects: chorus, phaser, flanger, tremolo/auto-pan.
//!
//! Every device here starts as a [`Placeholder`] (pass-through / silent / MIDI-thru) with its
//! final descriptor. **Param ids are stable and append-only** (documents, automation and
//! presets store them): never renumber, only append. Split this module into files as you like.
//!
//! # Chorus (`BuiltinDeviceType::Chorus`)
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Chorus | `Mode` | Chorus / Ensemble / Vibrato (default Chorus) |
//! | 1 | Chorus | `Rate` | 0.01 ..= 10 Hertz (log), default 0.8 |
//! | 2 | Chorus | `Depth` | 0 ..= 100 Percent, default 50 |
//! | 3 | Chorus | `Delay` | 1 ..= 40 Milliseconds, default 7 |
//! | 4 | Chorus | `Voices` | 1 ..= 4 (stepped), default 2 |
//! | 5 | Chorus | `Spread` | 0 ..= 100 Percent, default 100 |
//! | 6 | Chorus | `Feedback` | 0 ..= 95 Percent, default 0 |
//! | 7 | Chorus | `High Cut` | 1000 ..= 20000 Hertz (log), default 20000 |
//! | 8 | Output | `Mix` | 0 ..= 100 Percent, default 50 |
//! | 9 | Output | `Output` | -24 ..= 24 Decibels, default 0 |
//!
//! # Phaser (`BuiltinDeviceType::Phaser`)
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Phaser | `Stages` | 2 / 4 / 6 / 8 / 12 (default 4) |
//! | 1 | Phaser | `Rate` | 0.01 ..= 20 Hertz (log), default 0.3 |
//! | 2 | Phaser | `Sync` | toggle, default off |
//! | 3 | Phaser | `Sync Rate` | sync rate (default index 6) |
//! | 4 | Phaser | `Depth` | 0 ..= 100 Percent, default 50 |
//! | 5 | Phaser | `Center` | 100 ..= 10000 Hertz (log), default 800 |
//! | 6 | Phaser | `Feedback` | -95 ..= 95 Percent, default 30 |
//! | 7 | Phaser | `Stereo Phase` | 0 ..= 180 None, default 90 |
//! | 8 | Output | `Mix` | 0 ..= 100 Percent, default 50 |
//! | 9 | Output | `Output` | -24 ..= 24 Decibels, default 0 |
//!
//! # Flanger (`BuiltinDeviceType::Flanger`)
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Flanger | `Rate` | 0.01 ..= 20 Hertz (log), default 0.2 |
//! | 1 | Flanger | `Sync` | toggle, default off |
//! | 2 | Flanger | `Sync Rate` | sync rate (default index 6) |
//! | 3 | Flanger | `Depth` | 0 ..= 100 Percent, default 50 |
//! | 4 | Flanger | `Delay` | 0.1 ..= 20 Milliseconds (log), default 2 |
//! | 5 | Flanger | `Feedback` | -95 ..= 95 Percent, default 50 |
//! | 6 | Flanger | `Stereo Phase` | 0 ..= 180 None, default 90 |
//! | 7 | Output | `Mix` | 0 ..= 100 Percent, default 50 |
//! | 8 | Output | `Output` | -24 ..= 24 Decibels, default 0 |
//!
//! # Tremolo (`BuiltinDeviceType::Tremolo`)
//!
//! Auto-Pan mode modulates the balance (Stereo Phase is ignored).
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Tremolo | `Mode` | Tremolo / Auto-Pan (default Tremolo) |
//! | 1 | Tremolo | `Shape` | Sine / Triangle / Square / Saw Up / Saw Down (default Sine) |
//! | 2 | Tremolo | `Rate` | 0.1 ..= 40 Hertz (log), default 4 |
//! | 3 | Tremolo | `Sync` | toggle, default off |
//! | 4 | Tremolo | `Sync Rate` | sync rate (default index 6) |
//! | 5 | Tremolo | `Depth` | 0 ..= 100 Percent, default 50 |
//! | 6 | Tremolo | `Stereo Phase` | 0 ..= 180 None, default 0 |
//! | 7 | Output | `Output` | -24 ..= 24 Decibels, default 0 |

use ether_core::Device;
use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor, ParamScale, ParamUnit};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType};

#[allow(unused_imports)]
use crate::contract::{
    FactoryPreset, Placeholder, PlaceholderMode, SYNC_RATES, choice, descriptor as build, param,
    stepped, toggle,
};

/// Param ids of `Chorus` (stable, append-only).
pub mod chorus {
    use ether_core::protocol::model::ParamId;
    pub const MODE: ParamId = ParamId(0);
    pub const RATE: ParamId = ParamId(1);
    pub const DEPTH: ParamId = ParamId(2);
    pub const DELAY: ParamId = ParamId(3);
    pub const VOICES: ParamId = ParamId(4);
    pub const SPREAD: ParamId = ParamId(5);
    pub const FEEDBACK: ParamId = ParamId(6);
    pub const HIGH_CUT: ParamId = ParamId(7);
    pub const MIX: ParamId = ParamId(8);
    pub const OUTPUT: ParamId = ParamId(9);
    /// Number of params.
    pub const COUNT: usize = 10;
}

/// Param ids of `Phaser` (stable, append-only).
pub mod phaser {
    use ether_core::protocol::model::ParamId;
    pub const STAGES: ParamId = ParamId(0);
    pub const RATE: ParamId = ParamId(1);
    pub const SYNC: ParamId = ParamId(2);
    pub const SYNC_RATE: ParamId = ParamId(3);
    pub const DEPTH: ParamId = ParamId(4);
    pub const CENTER: ParamId = ParamId(5);
    pub const FEEDBACK: ParamId = ParamId(6);
    pub const STEREO_PHASE: ParamId = ParamId(7);
    pub const MIX: ParamId = ParamId(8);
    pub const OUTPUT: ParamId = ParamId(9);
    /// Number of params.
    pub const COUNT: usize = 10;
}

/// Param ids of `Flanger` (stable, append-only).
pub mod flanger {
    use ether_core::protocol::model::ParamId;
    pub const RATE: ParamId = ParamId(0);
    pub const SYNC: ParamId = ParamId(1);
    pub const SYNC_RATE: ParamId = ParamId(2);
    pub const DEPTH: ParamId = ParamId(3);
    pub const DELAY: ParamId = ParamId(4);
    pub const FEEDBACK: ParamId = ParamId(5);
    pub const STEREO_PHASE: ParamId = ParamId(6);
    pub const MIX: ParamId = ParamId(7);
    pub const OUTPUT: ParamId = ParamId(8);
    /// Number of params.
    pub const COUNT: usize = 9;
}

/// Param ids of `Tremolo` (stable, append-only).
pub mod tremolo {
    use ether_core::protocol::model::ParamId;
    pub const MODE: ParamId = ParamId(0);
    pub const SHAPE: ParamId = ParamId(1);
    pub const RATE: ParamId = ParamId(2);
    pub const SYNC: ParamId = ParamId(3);
    pub const SYNC_RATE: ParamId = ParamId(4);
    pub const DEPTH: ParamId = ParamId(5);
    pub const STEREO_PHASE: ParamId = ParamId(6);
    pub const OUTPUT: ParamId = ParamId(7);
    /// Number of params.
    pub const COUNT: usize = 8;
}

/// Descriptor of a type of this group.
///
/// # Panics
/// For a type of another group.
pub fn descriptor(ty: BuiltinDeviceType) -> DeviceDescriptor {
    match ty {
        BuiltinDeviceType::Chorus => build(
            BuiltinDeviceType::Chorus,
            "Chorus",
            DeviceCategory::AudioEffect,
            vec![
                choice(0, "Mode", "Chorus", &["Chorus", "Ensemble", "Vibrato"], 0),
                param(
                    1,
                    "Rate",
                    "Chorus",
                    ParamUnit::Hertz,
                    (0.01, 10.0, 0.8),
                    ParamScale::Log,
                ),
                param(
                    2,
                    "Depth",
                    "Chorus",
                    ParamUnit::Percent,
                    (0.0, 100.0, 50.0),
                    ParamScale::Linear,
                ),
                param(
                    3,
                    "Delay",
                    "Chorus",
                    ParamUnit::Milliseconds,
                    (1.0, 40.0, 7.0),
                    ParamScale::Linear,
                ),
                stepped(4, "Voices", "Chorus", ParamUnit::None, 1, 4, 2),
                param(
                    5,
                    "Spread",
                    "Chorus",
                    ParamUnit::Percent,
                    (0.0, 100.0, 100.0),
                    ParamScale::Linear,
                ),
                param(
                    6,
                    "Feedback",
                    "Chorus",
                    ParamUnit::Percent,
                    (0.0, 95.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    7,
                    "High Cut",
                    "Chorus",
                    ParamUnit::Hertz,
                    (1000.0, 20000.0, 20000.0),
                    ParamScale::Log,
                ),
                param(
                    8,
                    "Mix",
                    "Output",
                    ParamUnit::Percent,
                    (0.0, 100.0, 50.0),
                    ParamScale::Linear,
                ),
                param(
                    9,
                    "Output",
                    "Output",
                    ParamUnit::Decibels,
                    (-24.0, 24.0, 0.0),
                    ParamScale::Linear,
                ),
            ],
            2,
            2,
            false,
            0,
        ),
        BuiltinDeviceType::Phaser => build(
            BuiltinDeviceType::Phaser,
            "Phaser",
            DeviceCategory::AudioEffect,
            vec![
                choice(0, "Stages", "Phaser", &["2", "4", "6", "8", "12"], 1),
                param(
                    1,
                    "Rate",
                    "Phaser",
                    ParamUnit::Hertz,
                    (0.01, 20.0, 0.3),
                    ParamScale::Log,
                ),
                toggle(2, "Sync", "Phaser", false),
                choice(3, "Sync Rate", "Phaser", &SYNC_RATES, 6),
                param(
                    4,
                    "Depth",
                    "Phaser",
                    ParamUnit::Percent,
                    (0.0, 100.0, 50.0),
                    ParamScale::Linear,
                ),
                param(
                    5,
                    "Center",
                    "Phaser",
                    ParamUnit::Hertz,
                    (100.0, 10000.0, 800.0),
                    ParamScale::Log,
                ),
                param(
                    6,
                    "Feedback",
                    "Phaser",
                    ParamUnit::Percent,
                    (-95.0, 95.0, 30.0),
                    ParamScale::Linear,
                ),
                param(
                    7,
                    "Stereo Phase",
                    "Phaser",
                    ParamUnit::None,
                    (0.0, 180.0, 90.0),
                    ParamScale::Linear,
                ),
                param(
                    8,
                    "Mix",
                    "Output",
                    ParamUnit::Percent,
                    (0.0, 100.0, 50.0),
                    ParamScale::Linear,
                ),
                param(
                    9,
                    "Output",
                    "Output",
                    ParamUnit::Decibels,
                    (-24.0, 24.0, 0.0),
                    ParamScale::Linear,
                ),
            ],
            2,
            2,
            false,
            0,
        ),
        BuiltinDeviceType::Flanger => build(
            BuiltinDeviceType::Flanger,
            "Flanger",
            DeviceCategory::AudioEffect,
            vec![
                param(
                    0,
                    "Rate",
                    "Flanger",
                    ParamUnit::Hertz,
                    (0.01, 20.0, 0.2),
                    ParamScale::Log,
                ),
                toggle(1, "Sync", "Flanger", false),
                choice(2, "Sync Rate", "Flanger", &SYNC_RATES, 6),
                param(
                    3,
                    "Depth",
                    "Flanger",
                    ParamUnit::Percent,
                    (0.0, 100.0, 50.0),
                    ParamScale::Linear,
                ),
                param(
                    4,
                    "Delay",
                    "Flanger",
                    ParamUnit::Milliseconds,
                    (0.1, 20.0, 2.0),
                    ParamScale::Log,
                ),
                param(
                    5,
                    "Feedback",
                    "Flanger",
                    ParamUnit::Percent,
                    (-95.0, 95.0, 50.0),
                    ParamScale::Linear,
                ),
                param(
                    6,
                    "Stereo Phase",
                    "Flanger",
                    ParamUnit::None,
                    (0.0, 180.0, 90.0),
                    ParamScale::Linear,
                ),
                param(
                    7,
                    "Mix",
                    "Output",
                    ParamUnit::Percent,
                    (0.0, 100.0, 50.0),
                    ParamScale::Linear,
                ),
                param(
                    8,
                    "Output",
                    "Output",
                    ParamUnit::Decibels,
                    (-24.0, 24.0, 0.0),
                    ParamScale::Linear,
                ),
            ],
            2,
            2,
            false,
            0,
        ),
        BuiltinDeviceType::Tremolo => build(
            BuiltinDeviceType::Tremolo,
            "Tremolo",
            DeviceCategory::AudioEffect,
            vec![
                choice(0, "Mode", "Tremolo", &["Tremolo", "Auto-Pan"], 0),
                choice(
                    1,
                    "Shape",
                    "Tremolo",
                    &["Sine", "Triangle", "Square", "Saw Up", "Saw Down"],
                    0,
                ),
                param(
                    2,
                    "Rate",
                    "Tremolo",
                    ParamUnit::Hertz,
                    (0.1, 40.0, 4.0),
                    ParamScale::Log,
                ),
                toggle(3, "Sync", "Tremolo", false),
                choice(4, "Sync Rate", "Tremolo", &SYNC_RATES, 6),
                param(
                    5,
                    "Depth",
                    "Tremolo",
                    ParamUnit::Percent,
                    (0.0, 100.0, 50.0),
                    ParamScale::Linear,
                ),
                param(
                    6,
                    "Stereo Phase",
                    "Tremolo",
                    ParamUnit::None,
                    (0.0, 180.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    7,
                    "Output",
                    "Output",
                    ParamUnit::Decibels,
                    (-24.0, 24.0, 0.0),
                    ParamScale::Linear,
                ),
            ],
            2,
            2,
            false,
            0,
        ),
        other => unreachable!("{other:?} is not a `fx-modulation` device"),
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

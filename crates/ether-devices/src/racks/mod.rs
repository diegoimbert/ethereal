//! v0.2 devices owned by the `racks-modulation` node (contracts-3 froze the param tables; see
//! docs/ROADMAP.md "v0.2" and the device agent guide there).
//!
//! Racks (`ether_model::rack`): the rack node's params are the 8 macros and the chain selector (shared by the three types, `ether_model::rack_macro_param`, `RACK_SELECTOR_PARAM`). The node itself passes audio/events through: chains are run by the engine before it (`ether_core::rack_chains`); macros are modulation sources (`ether_core::modulation`).
//!
//! Every device here starts as a [`Placeholder`] (pass-through / silent / MIDI-thru) with its
//! final descriptor. **Param ids are stable and append-only** (documents, automation and
//! presets store them): never renumber, only append. Split this module into files as you like.
//!
//! # Instrument Rack (`BuiltinDeviceType::InstrumentRack`)
//!
//! Its node takes the chains' mix as input (`channels() == (2, 2)`), like the drum rack.
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Macros | `Macro 1` | 0 ..= 1 None, default 0 |
//! | 1 | Macros | `Macro 2` | 0 ..= 1 None, default 0 |
//! | 2 | Macros | `Macro 3` | 0 ..= 1 None, default 0 |
//! | 3 | Macros | `Macro 4` | 0 ..= 1 None, default 0 |
//! | 4 | Macros | `Macro 5` | 0 ..= 1 None, default 0 |
//! | 5 | Macros | `Macro 6` | 0 ..= 1 None, default 0 |
//! | 6 | Macros | `Macro 7` | 0 ..= 1 None, default 0 |
//! | 7 | Macros | `Macro 8` | 0 ..= 1 None, default 0 |
//! | 8 | Chains | `Chain Selector` | 0 ..= 127 (stepped), default 0 |
//!
//! # Audio Effect Rack (`BuiltinDeviceType::AudioEffectRack`)
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Macros | `Macro 1` | 0 ..= 1 None, default 0 |
//! | 1 | Macros | `Macro 2` | 0 ..= 1 None, default 0 |
//! | 2 | Macros | `Macro 3` | 0 ..= 1 None, default 0 |
//! | 3 | Macros | `Macro 4` | 0 ..= 1 None, default 0 |
//! | 4 | Macros | `Macro 5` | 0 ..= 1 None, default 0 |
//! | 5 | Macros | `Macro 6` | 0 ..= 1 None, default 0 |
//! | 6 | Macros | `Macro 7` | 0 ..= 1 None, default 0 |
//! | 7 | Macros | `Macro 8` | 0 ..= 1 None, default 0 |
//! | 8 | Chains | `Chain Selector` | 0 ..= 127 (stepped), default 0 |
//!
//! # MIDI Effect Rack (`BuiltinDeviceType::MidiEffectRack`)
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Macros | `Macro 1` | 0 ..= 1 None, default 0 |
//! | 1 | Macros | `Macro 2` | 0 ..= 1 None, default 0 |
//! | 2 | Macros | `Macro 3` | 0 ..= 1 None, default 0 |
//! | 3 | Macros | `Macro 4` | 0 ..= 1 None, default 0 |
//! | 4 | Macros | `Macro 5` | 0 ..= 1 None, default 0 |
//! | 5 | Macros | `Macro 6` | 0 ..= 1 None, default 0 |
//! | 6 | Macros | `Macro 7` | 0 ..= 1 None, default 0 |
//! | 7 | Macros | `Macro 8` | 0 ..= 1 None, default 0 |
//! | 8 | Chains | `Chain Selector` | 0 ..= 127 (stepped), default 0 |

use ether_core::Device;
use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor, ParamScale, ParamUnit};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType};

#[allow(unused_imports)]
use crate::contract::{
    FactoryPreset, Placeholder, PlaceholderMode, SYNC_RATES, choice, descriptor as build, param,
    stepped, toggle,
};

/// Param ids of `InstrumentRack` (stable, append-only).
pub mod instrument_rack {
    use ether_core::protocol::model::ParamId;
    pub const MACRO_1: ParamId = ParamId(0);
    pub const MACRO_2: ParamId = ParamId(1);
    pub const MACRO_3: ParamId = ParamId(2);
    pub const MACRO_4: ParamId = ParamId(3);
    pub const MACRO_5: ParamId = ParamId(4);
    pub const MACRO_6: ParamId = ParamId(5);
    pub const MACRO_7: ParamId = ParamId(6);
    pub const MACRO_8: ParamId = ParamId(7);
    pub const CHAIN_SELECTOR: ParamId = ParamId(8);
    /// Number of params.
    pub const COUNT: usize = 9;
}

/// Param ids of `AudioEffectRack` (stable, append-only).
pub mod audio_effect_rack {
    use ether_core::protocol::model::ParamId;
    pub const MACRO_1: ParamId = ParamId(0);
    pub const MACRO_2: ParamId = ParamId(1);
    pub const MACRO_3: ParamId = ParamId(2);
    pub const MACRO_4: ParamId = ParamId(3);
    pub const MACRO_5: ParamId = ParamId(4);
    pub const MACRO_6: ParamId = ParamId(5);
    pub const MACRO_7: ParamId = ParamId(6);
    pub const MACRO_8: ParamId = ParamId(7);
    pub const CHAIN_SELECTOR: ParamId = ParamId(8);
    /// Number of params.
    pub const COUNT: usize = 9;
}

/// Param ids of `MidiEffectRack` (stable, append-only).
pub mod midi_effect_rack {
    use ether_core::protocol::model::ParamId;
    pub const MACRO_1: ParamId = ParamId(0);
    pub const MACRO_2: ParamId = ParamId(1);
    pub const MACRO_3: ParamId = ParamId(2);
    pub const MACRO_4: ParamId = ParamId(3);
    pub const MACRO_5: ParamId = ParamId(4);
    pub const MACRO_6: ParamId = ParamId(5);
    pub const MACRO_7: ParamId = ParamId(6);
    pub const MACRO_8: ParamId = ParamId(7);
    pub const CHAIN_SELECTOR: ParamId = ParamId(8);
    /// Number of params.
    pub const COUNT: usize = 9;
}

/// Descriptor of a type of this group.
///
/// # Panics
/// For a type of another group.
pub fn descriptor(ty: BuiltinDeviceType) -> DeviceDescriptor {
    match ty {
        BuiltinDeviceType::InstrumentRack => build(
            BuiltinDeviceType::InstrumentRack,
            "Instrument Rack",
            DeviceCategory::Instrument,
            vec![
                param(
                    0,
                    "Macro 1",
                    "Macros",
                    ParamUnit::None,
                    (0.0, 1.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    1,
                    "Macro 2",
                    "Macros",
                    ParamUnit::None,
                    (0.0, 1.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    2,
                    "Macro 3",
                    "Macros",
                    ParamUnit::None,
                    (0.0, 1.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    3,
                    "Macro 4",
                    "Macros",
                    ParamUnit::None,
                    (0.0, 1.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    4,
                    "Macro 5",
                    "Macros",
                    ParamUnit::None,
                    (0.0, 1.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    5,
                    "Macro 6",
                    "Macros",
                    ParamUnit::None,
                    (0.0, 1.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    6,
                    "Macro 7",
                    "Macros",
                    ParamUnit::None,
                    (0.0, 1.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    7,
                    "Macro 8",
                    "Macros",
                    ParamUnit::None,
                    (0.0, 1.0, 0.0),
                    ParamScale::Linear,
                ),
                stepped(8, "Chain Selector", "Chains", ParamUnit::None, 0, 127, 0),
            ],
            0,
            2,
            true,
            0,
        ),
        BuiltinDeviceType::AudioEffectRack => build(
            BuiltinDeviceType::AudioEffectRack,
            "Audio Effect Rack",
            DeviceCategory::AudioEffect,
            vec![
                param(
                    0,
                    "Macro 1",
                    "Macros",
                    ParamUnit::None,
                    (0.0, 1.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    1,
                    "Macro 2",
                    "Macros",
                    ParamUnit::None,
                    (0.0, 1.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    2,
                    "Macro 3",
                    "Macros",
                    ParamUnit::None,
                    (0.0, 1.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    3,
                    "Macro 4",
                    "Macros",
                    ParamUnit::None,
                    (0.0, 1.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    4,
                    "Macro 5",
                    "Macros",
                    ParamUnit::None,
                    (0.0, 1.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    5,
                    "Macro 6",
                    "Macros",
                    ParamUnit::None,
                    (0.0, 1.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    6,
                    "Macro 7",
                    "Macros",
                    ParamUnit::None,
                    (0.0, 1.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    7,
                    "Macro 8",
                    "Macros",
                    ParamUnit::None,
                    (0.0, 1.0, 0.0),
                    ParamScale::Linear,
                ),
                stepped(8, "Chain Selector", "Chains", ParamUnit::None, 0, 127, 0),
            ],
            2,
            2,
            false,
            0,
        ),
        BuiltinDeviceType::MidiEffectRack => build(
            BuiltinDeviceType::MidiEffectRack,
            "MIDI Effect Rack",
            DeviceCategory::NoteEffect,
            vec![
                param(
                    0,
                    "Macro 1",
                    "Macros",
                    ParamUnit::None,
                    (0.0, 1.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    1,
                    "Macro 2",
                    "Macros",
                    ParamUnit::None,
                    (0.0, 1.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    2,
                    "Macro 3",
                    "Macros",
                    ParamUnit::None,
                    (0.0, 1.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    3,
                    "Macro 4",
                    "Macros",
                    ParamUnit::None,
                    (0.0, 1.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    4,
                    "Macro 5",
                    "Macros",
                    ParamUnit::None,
                    (0.0, 1.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    5,
                    "Macro 6",
                    "Macros",
                    ParamUnit::None,
                    (0.0, 1.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    6,
                    "Macro 7",
                    "Macros",
                    ParamUnit::None,
                    (0.0, 1.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    7,
                    "Macro 8",
                    "Macros",
                    ParamUnit::None,
                    (0.0, 1.0, 0.0),
                    ParamScale::Linear,
                ),
                stepped(8, "Chain Selector", "Chains", ParamUnit::None, 0, 127, 0),
            ],
            0,
            0,
            true,
            0,
        ),
        other => unreachable!("{other:?} is not a `racks-modulation` device"),
    }
}

/// Non-RT. A new instance (placeholder until implemented).
pub fn create(device: &BuiltinDevice) -> Box<dyn Device> {
    let ty = device.device_type();
    let mode = match ty {
        BuiltinDeviceType::InstrumentRack | BuiltinDeviceType::AudioEffectRack => {
            PlaceholderMode::PassThrough
        }
        _ => PlaceholderMode::MidiThru,
    };
    Box::new(Placeholder::new(descriptor(ty), mode))
}

/// Factory presets of a type of this group (embedded; add `FactoryPreset { id, json:
/// include_str!("../../presets/<device-key>/<slug>.etherpreset") }` entries).
pub fn factory_presets(ty: BuiltinDeviceType) -> &'static [FactoryPreset] {
    let _ = ty;
    &[]
}

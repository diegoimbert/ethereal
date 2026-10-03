//! v0.3 devices owned by the `external-instrument` node (contracts-4 froze the param
//! tables; see docs/ROADMAP.md "v0.3" and CONTRACTS.md §13.7).
//!
//! The nodes read and write hardware through the engine's `ether_core::hw_io` buffers (the
//! routing is in the device kind, `ether_model::ExternalRouting`, compiled into
//! `TrackDesc::hw_io`), report `LATENCY` (ms → samples at the prepared rate) as
//! `Node::latency` for PDC, and send an External Instrument's input notes/MIDI to the
//! hardware MIDI ring. Routing changes (`External::SetRouting`) reach live nodes through
//! `EngineBridge::update_builtin` → `Node::set_data`.
//!
//! **Param ids are stable and append-only.**
//!
//! # External Instrument (`BuiltinDeviceType::ExternalInstrument`)
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Return | `Gain` | -60 ..= 24 Decibels, default 0 |
//! | 1 | Hardware | `Latency` | 0 ..= 500 Milliseconds, default 0 |
//!
//! # External Audio Effect (`BuiltinDeviceType::ExternalAudioEffect`)
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Send | `Send Gain` | -24 ..= 24 Decibels, default 0 |
//! | 1 | Return | `Return Gain` | -24 ..= 24 Decibels, default 0 |
//! | 2 | Output | `Mix` | 0 ..= 100 Percent, default 100 |
//! | 3 | Hardware | `Latency` | 0 ..= 500 Milliseconds, default 0 |
//! | 4 | Return | `Invert Phase` | toggle, default off |

use ether_core::Device;
use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor, ParamScale, ParamUnit};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType};

#[allow(unused_imports)]
use crate::contract::{
    FactoryPreset, Placeholder, PlaceholderMode, descriptor as build, param, toggle,
};

/// Param ids of `ExternalInstrument` (stable, append-only).
pub mod external_instrument {
    use ether_core::protocol::model::ParamId;
    pub const GAIN: ParamId = ParamId(0);
    pub const LATENCY: ParamId = ParamId(1);
    /// Number of params.
    pub const COUNT: usize = 2;
}

/// Param ids of `ExternalAudioEffect` (stable, append-only).
pub mod external_audio_effect {
    use ether_core::protocol::model::ParamId;
    pub const SEND_GAIN: ParamId = ParamId(0);
    pub const RETURN_GAIN: ParamId = ParamId(1);
    pub const MIX: ParamId = ParamId(2);
    pub const LATENCY: ParamId = ParamId(3);
    pub const INVERT_PHASE: ParamId = ParamId(4);
    /// Number of params.
    pub const COUNT: usize = 5;
}

fn latency(id: u32) -> ether_core::protocol::devices::ParamInfo {
    param(
        id,
        "Latency",
        "Hardware",
        ParamUnit::Milliseconds,
        (0.0, 500.0, 0.0),
        ParamScale::Linear,
    )
}

/// Descriptor of a type of this group.
///
/// # Panics
/// For a type of another group.
pub fn descriptor(ty: BuiltinDeviceType) -> DeviceDescriptor {
    match ty {
        BuiltinDeviceType::ExternalInstrument => build(
            BuiltinDeviceType::ExternalInstrument,
            "External Instrument",
            DeviceCategory::Instrument,
            vec![
                param(
                    0,
                    "Gain",
                    "Return",
                    ParamUnit::Decibels,
                    (-60.0, 24.0, 0.0),
                    ParamScale::Linear,
                ),
                latency(1),
            ],
            0,
            2,
            true,
            0,
        ),
        BuiltinDeviceType::ExternalAudioEffect => build(
            BuiltinDeviceType::ExternalAudioEffect,
            "External Audio Effect",
            DeviceCategory::AudioEffect,
            vec![
                param(
                    0,
                    "Send Gain",
                    "Send",
                    ParamUnit::Decibels,
                    (-24.0, 24.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    1,
                    "Return Gain",
                    "Return",
                    ParamUnit::Decibels,
                    (-24.0, 24.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    2,
                    "Mix",
                    "Output",
                    ParamUnit::Percent,
                    (0.0, 100.0, 100.0),
                    ParamScale::Linear,
                ),
                latency(3),
                toggle(4, "Invert Phase", "Return", false),
            ],
            2,
            2,
            false,
            0,
        ),
        other => panic!("{other:?} is not an external device"),
    }
}

/// Non-RT. Placeholders until `external-instrument` lands: the instrument is silent, the
/// effect passes through.
pub fn create(device: &BuiltinDevice) -> Box<dyn Device> {
    let ty = device.device_type();
    let mode = match ty {
        BuiltinDeviceType::ExternalInstrument => PlaceholderMode::Silent,
        _ => PlaceholderMode::PassThrough,
    };
    Box::new(Placeholder::new(descriptor(ty), mode))
}

/// Factory presets of a type of this group (none expected: hardware differs per user).
pub fn factory_presets(ty: BuiltinDeviceType) -> &'static [FactoryPreset] {
    let _ = ty;
    &[]
}

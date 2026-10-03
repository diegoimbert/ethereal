//! v0.3 devices owned by the `fx-space` node (contracts-4 froze the param table; see
//! docs/ROADMAP.md "v0.3" and the device agent guide in its "v0.2" section).
//!
//! # Convolution Reverb (`BuiltinDeviceType::ConvolutionReverb`)
//!
//! Partitioned convolution (uniform or non-uniform partitions; a zero-latency head is
//! preferred, else report the partition latency with `Node::latency`) of the input with the
//! impulse response of `BuiltinDevice::ConvolutionReverb { ir }`:
//! - `IrSource::Factory { id }`: one of [`FACTORY_IRS`] (shipped with the device; generated
//!   or embedded, the `fx-space` node decides; ids are append-only);
//! - `IrSource::Media { media }`: project media (imported or referenced in place) resolved
//!   through the `SampleResolver` like the sampler's sample; the whole IR is read and
//!   transformed (FFT partitions) in `create` / `Node::set_data`, never on the audio thread.
//! - `None`: dry only.
//!
//! IR changes (`Device::SetIr`) reach the live node through `EngineBridge::update_builtin` →
//! `Node::set_data` (crossfade to the new IR) instead of re-creating it.
//!
//! **Param ids are stable and append-only.**
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Output | `Mix` | 0 ..= 100 Percent, default 30 |
//! | 1 | Time | `Pre-delay` | 0 ..= 250 Milliseconds, default 0 |
//! | 2 | Time | `Decay` | 10 ..= 100 Percent (IR tail shortening), default 100 |
//! | 3 | Time | `Size` | 50 ..= 150 Percent (IR time stretch), default 100 |
//! | 4 | EQ | `Low Cut` | 20 ..= 2000 Hertz (log), default 20 |
//! | 5 | EQ | `High Cut` | 1000 ..= 20000 Hertz (log), default 20000 |
//! | 6 | Output | `Width` | 0 ..= 200 Percent, default 100 |
//! | 7 | Output | `Gain` | -24 ..= 24 Decibels (wet), default 0 |
//! | 8 | Time | `Reverse` | toggle, default off |

use ether_core::Device;
use ether_core::protocol::devices::{
    DeviceCategory, DeviceDescriptor, FactoryIr, ParamScale, ParamUnit,
};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, Seconds};

#[allow(unused_imports)]
use crate::contract::{
    FactoryPreset, Placeholder, PlaceholderMode, choice, descriptor as build, param, toggle,
};

/// Param ids of `ConvolutionReverb` (stable, append-only).
pub mod convolution_reverb {
    use ether_core::protocol::model::ParamId;
    pub const MIX: ParamId = ParamId(0);
    pub const PRE_DELAY: ParamId = ParamId(1);
    pub const DECAY: ParamId = ParamId(2);
    pub const SIZE: ParamId = ParamId(3);
    pub const LOW_CUT: ParamId = ParamId(4);
    pub const HIGH_CUT: ParamId = ParamId(5);
    pub const WIDTH: ParamId = ParamId(6);
    pub const GAIN: ParamId = ParamId(7);
    pub const REVERSE: ParamId = ParamId(8);
    /// Number of params.
    pub const COUNT: usize = 9;
}

/// A factory impulse response (static data; [`factory_irs`] gives the wire form).
#[derive(Clone, Copy, Debug)]
pub struct FactoryIrSpec {
    /// `IrSource::Factory { id }` (stable, append-only).
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub length_seconds: f64,
    pub channels: u16,
}

/// The factory IRs (ids frozen by contracts-4; `fx-space` may append and tune the rest).
pub const FACTORY_IRS: &[FactoryIrSpec] = &[
    FactoryIrSpec {
        id: "room",
        name: "Small Room",
        category: "Room",
        length_seconds: 0.6,
        channels: 2,
    },
    FactoryIrSpec {
        id: "chamber",
        name: "Chamber",
        category: "Room",
        length_seconds: 1.2,
        channels: 2,
    },
    FactoryIrSpec {
        id: "plate",
        name: "Plate",
        category: "Plate",
        length_seconds: 1.8,
        channels: 2,
    },
    FactoryIrSpec {
        id: "hall",
        name: "Concert Hall",
        category: "Hall",
        length_seconds: 2.8,
        channels: 2,
    },
];

/// `Device::ListFactoryIrs` reply.
pub fn factory_irs() -> Vec<FactoryIr> {
    FACTORY_IRS
        .iter()
        .map(|s| FactoryIr {
            id: s.id.to_owned(),
            name: s.name.to_owned(),
            category: s.category.to_owned(),
            length: Seconds(s.length_seconds),
            channels: s.channels,
        })
        .collect()
}

/// Descriptor of a type of this group.
///
/// # Panics
/// For a type of another group.
pub fn descriptor(ty: BuiltinDeviceType) -> DeviceDescriptor {
    match ty {
        BuiltinDeviceType::ConvolutionReverb => build(
            BuiltinDeviceType::ConvolutionReverb,
            "Convolution Reverb",
            DeviceCategory::AudioEffect,
            vec![
                param(
                    0,
                    "Mix",
                    "Output",
                    ParamUnit::Percent,
                    (0.0, 100.0, 30.0),
                    ParamScale::Linear,
                ),
                param(
                    1,
                    "Pre-delay",
                    "Time",
                    ParamUnit::Milliseconds,
                    (0.0, 250.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    2,
                    "Decay",
                    "Time",
                    ParamUnit::Percent,
                    (10.0, 100.0, 100.0),
                    ParamScale::Linear,
                ),
                param(
                    3,
                    "Size",
                    "Time",
                    ParamUnit::Percent,
                    (50.0, 150.0, 100.0),
                    ParamScale::Linear,
                ),
                param(
                    4,
                    "Low Cut",
                    "EQ",
                    ParamUnit::Hertz,
                    (20.0, 2000.0, 20.0),
                    ParamScale::Log,
                ),
                param(
                    5,
                    "High Cut",
                    "EQ",
                    ParamUnit::Hertz,
                    (1000.0, 20000.0, 20000.0),
                    ParamScale::Log,
                ),
                param(
                    6,
                    "Width",
                    "Output",
                    ParamUnit::Percent,
                    (0.0, 200.0, 100.0),
                    ParamScale::Linear,
                ),
                param(
                    7,
                    "Gain",
                    "Output",
                    ParamUnit::Decibels,
                    (-24.0, 24.0, 0.0),
                    ParamScale::Linear,
                ),
                toggle(8, "Reverse", "Time", false),
            ],
            2,
            2,
            false,
            0,
        ),
        other => panic!("{other:?} is not an fx-space device"),
    }
}

/// Non-RT. The device for `device` (a placeholder until `fx-space` lands: pass-through).
pub fn create(device: &BuiltinDevice) -> Box<dyn Device> {
    let ty = device.device_type();
    Box::new(Placeholder::new(
        descriptor(ty),
        PlaceholderMode::PassThrough,
    ))
}

/// Factory presets of a type of this group (embedded; add `FactoryPreset { id, json:
/// include_str!("../../presets/<device-key>/<slug>.etherpreset") }` entries).
pub fn factory_presets(ty: BuiltinDeviceType) -> &'static [FactoryPreset] {
    let _ = ty;
    &[]
}

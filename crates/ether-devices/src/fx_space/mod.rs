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
//!
//! # Implementation (`fx-space`)
//!
//! - [`convolver`]: zero-latency non-uniform partitioned convolution (direct 128-tap head,
//!   FFT stages of 128 and 2048 with the large stage's work spread evenly over 128-sample
//!   ticks); `Node::latency` is 0. Shaping params (`Decay`, `Size`, `Reverse`) rebuild the
//!   spare kernel a few partitions per tick and crossfade, sharing the input history.
//! - [`ir`]: base IR loading (media, at the engine rate, truncated to
//!   [`ir::MAX_IR_SECONDS`], energy-normalized) and shaping.
//! - [`factory_ir`]: the factory IRs, synthesized (no recorded or third-party IRs).
//! - [`device`]: the node (pre-delay, convolver, low/high cut, width, gain, mix) and the
//!   IR swap through `Node::set_data` ([`ir_swap`], built off the audio thread by the
//!   bridges' `update_builtin`).

mod convolver;
mod device;
mod factory_ir;
pub mod ir;

use ether_core::protocol::devices::{
    DeviceCategory, DeviceDescriptor, FactoryIr, ParamScale, ParamUnit,
};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, IrSource, ParamId, Seconds};
use ether_core::Device;
use ether_core::node::NodeData;

use crate::SampleResolver;
#[allow(unused_imports)]
use crate::contract::{
    FactoryPreset, Placeholder, PlaceholderMode, choice, descriptor as build, layout, param,
    section, toggle,
};

pub use convolver::{B as HEAD_BLOCK, Convolver, T as TAIL_BLOCK};
pub use device::{ConvolutionReverb, IrSwap, SWAP_MS};
pub use ir::{IrBase, IrInput, Shaping};

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
    FactoryIrSpec {
        id: "cathedral",
        name: "Cathedral",
        category: "Hall",
        length_seconds: 5.0,
        channels: 2,
    },
    FactoryIrSpec {
        id: "ambience",
        name: "Ambience",
        category: "Room",
        length_seconds: 0.35,
        channels: 2,
    },
];

/// `(min, max, default)` per param id (mirrors [`descriptor`]; RT lookups).
const RANGES: [(f64, f64, f64); convolution_reverb::COUNT] = [
    (0.0, 100.0, 30.0),
    (0.0, 250.0, 0.0),
    (10.0, 100.0, 100.0),
    (50.0, 150.0, 100.0),
    (20.0, 2000.0, 20.0),
    (1000.0, 20000.0, 20000.0),
    (0.0, 200.0, 100.0),
    (-24.0, 24.0, 0.0),
    (0.0, 1.0, 0.0),
];

/// Range of a param (allocation-free).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Range {
    pub min: f64,
    pub max: f64,
    pub default: f64,
}

pub(crate) fn param_info(id: ParamId) -> Option<Range> {
    RANGES
        .get(id.0 as usize)
        .map(|&(min, max, default)| Range { min, max, default })
}

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
        BuiltinDeviceType::ConvolutionReverb => DeviceDescriptor {
            layout: Some(reverb_layout()),
            ..build(
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
        )
        },
        other => panic!("{other:?} is not an fx-space device"),
    }
}

/// Panel: the IR (picker + waveform, the `SampleWaveform` data widget, which the shared
/// renderer draws as the IR widget for this device) with its shaping, the EQ and the output.
fn reverb_layout() -> ether_core::protocol::layout::DeviceLayout {
    use crate::contract::{item, knob};
    use convolution_reverb as p;
    use ether_core::protocol::layout::Widget;
    use ether_core::protocol::layout::WidgetSize::*;
    let mut ir = item(
        Widget::SampleWaveform {
            start: None,
            end: None,
        },
        Large,
    );
    ir.colspan = 4;
    ir.label = Some("Impulse Response".to_owned());
    layout(vec![
        section(
            "ir",
            Some("Impulse Response"),
            3,
            4,
            vec![
                ir,
                knob(p::DECAY, Large),
                knob(p::SIZE, Medium),
                knob(p::PRE_DELAY, Medium),
                item(Widget::Toggle { param: p::REVERSE }, Small),
            ],
        ),
        section(
            "eq",
            Some("EQ"),
            1,
            1,
            vec![knob(p::LOW_CUT, Medium), knob(p::HIGH_CUT, Medium)],
        ),
        section(
            "output",
            Some("Output"),
            1,
            2,
            vec![
                knob(p::MIX, Large),
                knob(p::GAIN, Medium),
                knob(p::WIDTH, Medium),
            ],
        ),
    ])
}

/// What `ir` points at, resolved (`None`: no IR, unknown factory id or media not loaded).
pub fn resolve(ir: Option<&IrSource>, samples: &dyn SampleResolver) -> Option<IrInput> {
    match ir? {
        IrSource::Factory { id } => FACTORY_IRS
            .iter()
            .position(|s| s.id == id)
            .map(IrInput::Factory),
        IrSource::Media { media } => samples.resolve(*media).map(IrInput::Media),
    }
}

/// Non-RT. The device for `device` (its IR is built in `prepare`, at the engine rate).
pub fn create(device: &BuiltinDevice, samples: &dyn SampleResolver) -> Box<dyn Device> {
    let ir = match device {
        BuiltinDevice::ConvolutionReverb { ir } => ir.as_ref(),
        other => panic!("{other:?} is not an fx-space device"),
    };
    Box::new(ConvolutionReverb::new(resolve(ir, samples)))
}

/// Whether a live node built from `old` takes `new` in place (`Node::set_data` with
/// [`ir_swap`], crossfading): only the IR of a convolution reverb changed.
pub fn updatable_in_place(old: &BuiltinDevice, new: &BuiltinDevice) -> bool {
    matches!(
        (old, new),
        (
            BuiltinDevice::ConvolutionReverb { ir: a },
            BuiltinDevice::ConvolutionReverb { ir: b },
        ) if a != b
    )
}

/// Non-RT (bridges' `update_builtin`). Read and partition the IR of `device` at
/// `sample_rate` and wrap it for `Node::set_data` (an [`IrSwap`]); `None` for another
/// device type. The node crossfades to it.
pub fn ir_swap(
    device: &BuiltinDevice,
    samples: &dyn SampleResolver,
    sample_rate: f32,
) -> Option<NodeData> {
    let BuiltinDevice::ConvolutionReverb { ir } = device else {
        return None;
    };
    let next = resolve(ir.as_ref(), samples).and_then(|input| {
        IrBase::load(&input, sample_rate)
            .map(|base| Box::new(Convolver::new(input, base, Shaping::default())))
    });
    Some(Box::new(IrSwap::new(next)))
}

/// Factory presets of a type of this group (embedded; add `FactoryPreset { id, json:
/// include_str!("../../presets/<device-key>/<slug>.etherpreset") }` entries).
pub fn factory_presets(ty: BuiltinDeviceType) -> &'static [FactoryPreset] {
    let _ = ty;
    &[]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_mirror_the_descriptor() {
        let d = descriptor(BuiltinDeviceType::ConvolutionReverb);
        for p in &d.params {
            let r = param_info(p.id).unwrap();
            assert_eq!((r.min, r.max, r.default), (p.min, p.max, p.default), "{}", p.name);
        }
    }
}

//! v0.2 devices owned by the `fx-color` node (contracts-3 froze the param tables; see
//! docs/ROADMAP.md "v0.2" and the device agent guide there).
//!
//! Color effects: saturator (several curves), bitcrusher, auto-filter (multimode + envelope follower + LFO).
//!
//! Implementations: [`saturator_device`], [`bitcrusher_device`], [`auto_filter_device`]
//! (DSP notes in each). **Param ids are stable and append-only** (documents, automation and
//! presets store them): never renumber, only append.
//!
//! # Saturator (`BuiltinDeviceType::Saturator`)
//!
//! Oversampling adds latency: the device always reports the 4x round trip
//! ([`saturator_device::LATENCY`] = 29 samples) so a mode change never moves PDC.
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Shape | `Curve` | Soft / Hard / Tube / Tape / Fold / Rectify (default Soft) |
//! | 1 | Shape | `Drive` | 0 ..= 36 Decibels, default 6 |
//! | 2 | Shape | `Bias` | -100 ..= 100 Percent, default 0 |
//! | 3 | Shape | `Tone` | -100 ..= 100 Percent, default 0 |
//! | 4 | Quality | `Oversampling` | Off / 2x / 4x (default 2x) |
//! | 5 | Output | `Auto Gain` | toggle, default on |
//! | 6 | Output | `Output` | -24 ..= 24 Decibels, default 0 |
//! | 7 | Output | `Mix` | 0 ..= 100 Percent, default 100 |
//!
//! # Bitcrusher (`BuiltinDeviceType::Bitcrusher`)
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Crush | `Bits` | 1 ..= 24 None, default 8 |
//! | 1 | Crush | `Rate` | 100 ..= 48000 Hertz (log), default 11025 |
//! | 2 | Crush | `Jitter` | 0 ..= 100 Percent, default 0 |
//! | 3 | Crush | `Dither` | toggle, default off |
//! | 4 | Output | `Output` | -24 ..= 24 Decibels, default 0 |
//! | 5 | Output | `Mix` | 0 ..= 100 Percent, default 100 |
//!
//! # Auto Filter (`BuiltinDeviceType::AutoFilter`)
//!
//! The envelope follower listens to the sidechain input when one is set (`sidechain_inputs = 2`), else to the main input.
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Filter | `Type` | Low-pass 12 / Low-pass 24 / High-pass 12 / High-pass 24 / Band-pass / Notch / Peak (default Low-pass 12) |
//! | 1 | Filter | `Cutoff` | 20 ..= 20000 Hertz (log), default 1000 |
//! | 2 | Filter | `Resonance` | 0 ..= 100 Percent, default 20 |
//! | 3 | Filter | `Drive` | 0 ..= 24 Decibels, default 0 |
//! | 4 | Envelope | `Amount` | -100 ..= 100 Percent, default 0 |
//! | 5 | Envelope | `Attack` | 0.1 ..= 100 Milliseconds (log), default 5 |
//! | 6 | Envelope | `Release` | 10 ..= 2000 Milliseconds (log), default 200 |
//! | 7 | LFO | `Amount` | 0 ..= 100 Percent, default 0 |
//! | 8 | LFO | `Shape` | Sine / Triangle / Saw Up / Saw Down / Square / Sample & Hold (default Sine) |
//! | 9 | LFO | `Rate` | 0.01 ..= 20 Hertz (log), default 1 |
//! | 10 | LFO | `Sync` | toggle, default off |
//! | 11 | LFO | `Sync Rate` | sync rate (default index 8) |
//! | 12 | LFO | `Stereo Phase` | 0 ..= 180 None, default 0 |
//! | 13 | Output | `Mix` | 0 ..= 100 Percent, default 100 |
//! | 14 | Output | `Output` | -24 ..= 24 Decibels, default 0 |

use ether_core::Device;
use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor, ParamScale, ParamUnit};
use ether_core::protocol::layout::{DeviceLayout, Widget, WidgetSize};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType};

use crate::contract::{
    FactoryPreset, SYNC_RATES, choice, descriptor as build, item, knob, layout, param, section,
    toggle,
};

pub mod auto_filter_device;
pub mod bitcrusher_device;
mod oversample;
pub mod saturator_device;
mod shared;

pub use auto_filter_device::AutoFilter;
pub use bitcrusher_device::Bitcrusher;
pub use saturator_device::Saturator;

/// Param ids of `Saturator` (stable, append-only).
pub mod saturator {
    use ether_core::protocol::model::ParamId;
    pub const CURVE: ParamId = ParamId(0);
    pub const DRIVE: ParamId = ParamId(1);
    pub const BIAS: ParamId = ParamId(2);
    pub const TONE: ParamId = ParamId(3);
    pub const OVERSAMPLING: ParamId = ParamId(4);
    pub const AUTO_GAIN: ParamId = ParamId(5);
    pub const OUTPUT: ParamId = ParamId(6);
    pub const MIX: ParamId = ParamId(7);
    /// Number of params.
    pub const COUNT: usize = 8;
}

/// Param ids of `Bitcrusher` (stable, append-only).
pub mod bitcrusher {
    use ether_core::protocol::model::ParamId;
    pub const BITS: ParamId = ParamId(0);
    pub const RATE: ParamId = ParamId(1);
    pub const JITTER: ParamId = ParamId(2);
    pub const DITHER: ParamId = ParamId(3);
    pub const OUTPUT: ParamId = ParamId(4);
    pub const MIX: ParamId = ParamId(5);
    /// Number of params.
    pub const COUNT: usize = 6;
}

/// Param ids of `AutoFilter` (stable, append-only).
pub mod auto_filter {
    use ether_core::protocol::model::ParamId;
    pub const TYPE: ParamId = ParamId(0);
    pub const CUTOFF: ParamId = ParamId(1);
    pub const RESONANCE: ParamId = ParamId(2);
    pub const DRIVE: ParamId = ParamId(3);
    pub const ENV_AMOUNT: ParamId = ParamId(4);
    pub const ENV_ATTACK: ParamId = ParamId(5);
    pub const ENV_RELEASE: ParamId = ParamId(6);
    pub const LFO_AMOUNT: ParamId = ParamId(7);
    pub const LFO_SHAPE: ParamId = ParamId(8);
    pub const LFO_RATE: ParamId = ParamId(9);
    pub const LFO_SYNC: ParamId = ParamId(10);
    pub const LFO_SYNC_RATE: ParamId = ParamId(11);
    pub const LFO_PHASE: ParamId = ParamId(12);
    pub const MIX: ParamId = ParamId(13);
    pub const OUTPUT: ParamId = ParamId(14);
    /// Number of params.
    pub const COUNT: usize = 15;
}

/// Descriptor of a type of this group.
///
/// # Panics
/// For a type of another group.
pub fn descriptor(ty: BuiltinDeviceType) -> DeviceDescriptor {
    let mut d = params_descriptor(ty);
    d.layout = Some(device_layout(ty));
    d
}

fn params_descriptor(ty: BuiltinDeviceType) -> DeviceDescriptor {
    match ty {
        BuiltinDeviceType::Saturator => build(
            BuiltinDeviceType::Saturator,
            "Saturator",
            DeviceCategory::AudioEffect,
            vec![
                choice(
                    0,
                    "Curve",
                    "Shape",
                    &["Soft", "Hard", "Tube", "Tape", "Fold", "Rectify"],
                    0,
                ),
                param(
                    1,
                    "Drive",
                    "Shape",
                    ParamUnit::Decibels,
                    (0.0, 36.0, 6.0),
                    ParamScale::Linear,
                ),
                param(
                    2,
                    "Bias",
                    "Shape",
                    ParamUnit::Percent,
                    (-100.0, 100.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    3,
                    "Tone",
                    "Shape",
                    ParamUnit::Percent,
                    (-100.0, 100.0, 0.0),
                    ParamScale::Linear,
                ),
                choice(4, "Oversampling", "Quality", &["Off", "2x", "4x"], 1),
                toggle(5, "Auto Gain", "Output", true),
                param(
                    6,
                    "Output",
                    "Output",
                    ParamUnit::Decibels,
                    (-24.0, 24.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    7,
                    "Mix",
                    "Output",
                    ParamUnit::Percent,
                    (0.0, 100.0, 100.0),
                    ParamScale::Linear,
                ),
            ],
            2,
            2,
            false,
            0,
        ),
        BuiltinDeviceType::Bitcrusher => build(
            BuiltinDeviceType::Bitcrusher,
            "Bitcrusher",
            DeviceCategory::AudioEffect,
            vec![
                param(
                    0,
                    "Bits",
                    "Crush",
                    ParamUnit::None,
                    (1.0, 24.0, 8.0),
                    ParamScale::Linear,
                ),
                param(
                    1,
                    "Rate",
                    "Crush",
                    ParamUnit::Hertz,
                    (100.0, 48000.0, 11025.0),
                    ParamScale::Log,
                ),
                param(
                    2,
                    "Jitter",
                    "Crush",
                    ParamUnit::Percent,
                    (0.0, 100.0, 0.0),
                    ParamScale::Linear,
                ),
                toggle(3, "Dither", "Crush", false),
                param(
                    4,
                    "Output",
                    "Output",
                    ParamUnit::Decibels,
                    (-24.0, 24.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    5,
                    "Mix",
                    "Output",
                    ParamUnit::Percent,
                    (0.0, 100.0, 100.0),
                    ParamScale::Linear,
                ),
            ],
            2,
            2,
            false,
            0,
        ),
        BuiltinDeviceType::AutoFilter => build(
            BuiltinDeviceType::AutoFilter,
            "Auto Filter",
            DeviceCategory::AudioEffect,
            vec![
                choice(
                    0,
                    "Type",
                    "Filter",
                    &[
                        "Low-pass 12",
                        "Low-pass 24",
                        "High-pass 12",
                        "High-pass 24",
                        "Band-pass",
                        "Notch",
                        "Peak",
                    ],
                    0,
                ),
                param(
                    1,
                    "Cutoff",
                    "Filter",
                    ParamUnit::Hertz,
                    (20.0, 20000.0, 1000.0),
                    ParamScale::Log,
                ),
                param(
                    2,
                    "Resonance",
                    "Filter",
                    ParamUnit::Percent,
                    (0.0, 100.0, 20.0),
                    ParamScale::Linear,
                ),
                param(
                    3,
                    "Drive",
                    "Filter",
                    ParamUnit::Decibels,
                    (0.0, 24.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    4,
                    "Amount",
                    "Envelope",
                    ParamUnit::Percent,
                    (-100.0, 100.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    5,
                    "Attack",
                    "Envelope",
                    ParamUnit::Milliseconds,
                    (0.1, 100.0, 5.0),
                    ParamScale::Log,
                ),
                param(
                    6,
                    "Release",
                    "Envelope",
                    ParamUnit::Milliseconds,
                    (10.0, 2000.0, 200.0),
                    ParamScale::Log,
                ),
                param(
                    7,
                    "Amount",
                    "LFO",
                    ParamUnit::Percent,
                    (0.0, 100.0, 0.0),
                    ParamScale::Linear,
                ),
                choice(
                    8,
                    "Shape",
                    "LFO",
                    &[
                        "Sine",
                        "Triangle",
                        "Saw Up",
                        "Saw Down",
                        "Square",
                        "Sample & Hold",
                    ],
                    0,
                ),
                param(
                    9,
                    "Rate",
                    "LFO",
                    ParamUnit::Hertz,
                    (0.01, 20.0, 1.0),
                    ParamScale::Log,
                ),
                toggle(10, "Sync", "LFO", false),
                choice(11, "Sync Rate", "LFO", &SYNC_RATES, 8),
                param(
                    12,
                    "Stereo Phase",
                    "LFO",
                    ParamUnit::None,
                    (0.0, 180.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    13,
                    "Mix",
                    "Output",
                    ParamUnit::Percent,
                    (0.0, 100.0, 100.0),
                    ParamScale::Linear,
                ),
                param(
                    14,
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
            2,
        ),
        other => unreachable!("{other:?} is not a `fx-color` device"),
    }
}

/// Declarative panel of a type of this group (drawn by the shared device renderer). Typed
/// widgets own the params they bind (the transfer curve shows Curve/Drive/Bias, the filter
/// curve Type/Cutoff/Resonance/Drive, the LFO preview Shape/Rate/Amount).
fn device_layout(ty: BuiltinDeviceType) -> DeviceLayout {
    use WidgetSize::*;
    let wide = |w: Widget, size: WidgetSize, colspan: u8| {
        let mut i = item(w, size);
        i.colspan = colspan;
        i
    };
    match ty {
        BuiltinDeviceType::Saturator => {
            use saturator as p;
            layout(vec![
                section(
                    "shape",
                    Some("Shape"),
                    2,
                    3,
                    vec![
                        wide(
                            Widget::TransferCurve {
                                drive: p::DRIVE,
                                curve: Some(p::CURVE),
                                bias: Some(p::BIAS),
                            },
                            Large,
                            2,
                        ),
                        knob(p::TONE, Medium),
                    ],
                ),
                section(
                    "output",
                    Some("Output"),
                    2,
                    3,
                    vec![
                        knob(p::OUTPUT, Medium),
                        knob(p::MIX, Large),
                        item(
                            Widget::Toggle {
                                param: p::AUTO_GAIN,
                            },
                            Small,
                        ),
                        wide(
                            Widget::Choice {
                                param: p::OVERSAMPLING,
                            },
                            Small,
                            3,
                        ),
                    ],
                ),
            ])
        }
        BuiltinDeviceType::Bitcrusher => {
            use bitcrusher as p;
            layout(vec![
                section(
                    "crush",
                    Some("Crush"),
                    2,
                    4,
                    vec![
                        knob(p::BITS, Large),
                        knob(p::RATE, Large),
                        knob(p::JITTER, Medium),
                        item(Widget::Toggle { param: p::DITHER }, Small),
                    ],
                ),
                section(
                    "output",
                    Some("Output"),
                    1,
                    2,
                    vec![knob(p::OUTPUT, Medium), knob(p::MIX, Large)],
                ),
            ])
        }
        BuiltinDeviceType::AutoFilter => {
            use auto_filter as p;
            layout(vec![
                section(
                    "filter",
                    Some("Filter"),
                    2,
                    2,
                    vec![wide(
                        Widget::FilterCurve {
                            cutoff: p::CUTOFF,
                            resonance: p::RESONANCE,
                            mode: Some(p::TYPE),
                            drive: Some(p::DRIVE),
                            gain: None,
                        },
                        Large,
                        2,
                    )],
                ),
                section(
                    "envelope",
                    Some("Envelope"),
                    1,
                    3,
                    vec![
                        knob(p::ENV_AMOUNT, Medium),
                        knob(p::ENV_ATTACK, Small),
                        knob(p::ENV_RELEASE, Small),
                    ],
                ),
                section(
                    "lfo",
                    Some("LFO"),
                    2,
                    3,
                    vec![
                        wide(
                            Widget::Lfo {
                                shape: p::LFO_SHAPE,
                                rate: p::LFO_RATE,
                                amount: Some(p::LFO_AMOUNT),
                            },
                            Medium,
                            3,
                        ),
                        item(Widget::Toggle { param: p::LFO_SYNC }, Small),
                        item(
                            Widget::Choice {
                                param: p::LFO_SYNC_RATE,
                            },
                            Small,
                        ),
                        knob(p::LFO_PHASE, Small),
                    ],
                ),
                section(
                    "output",
                    Some("Output"),
                    1,
                    1,
                    vec![knob(p::MIX, Large), knob(p::OUTPUT, Medium)],
                ),
            ])
        }
        other => unreachable!("{other:?} is not a `fx-color` device"),
    }
}

/// Non-RT. A new instance.
pub fn create(device: &BuiltinDevice) -> Box<dyn Device> {
    match device.device_type() {
        BuiltinDeviceType::Saturator => Box::new(Saturator::new()),
        BuiltinDeviceType::Bitcrusher => Box::new(Bitcrusher::new()),
        BuiltinDeviceType::AutoFilter => Box::new(AutoFilter::new()),
        other => unreachable!("{other:?} is not a `fx-color` device"),
    }
}

/// `FactoryPreset` entries for `presets/<key>/<slug>.etherpreset` files.
macro_rules! presets {
    ($key:literal: $($slug:literal),* $(,)?) => {
        &[$(FactoryPreset {
            id: concat!($key, "/", $slug),
            json: include_str!(concat!("../../presets/", $key, "/", $slug, ".etherpreset")),
        }),*]
    };
}

/// Factory presets of a type of this group (embedded).
pub fn factory_presets(ty: BuiltinDeviceType) -> &'static [FactoryPreset] {
    match ty {
        BuiltinDeviceType::Saturator => presets!(
            "saturator": "warm-tape", "tube-drive", "hard-clip", "wavefolder", "octave-fuzz"
        ),
        BuiltinDeviceType::Bitcrusher => presets!(
            "bitcrusher": "8-bit-console", "lo-fi-sampler", "radio-grit", "broken-clock"
        ),
        BuiltinDeviceType::AutoFilter => presets!(
            "auto-filter": "envelope-wah", "sidechain-pump", "slow-sweep", "tempo-wobble",
            "resonant-rise"
        ),
        _ => &[],
    }
}

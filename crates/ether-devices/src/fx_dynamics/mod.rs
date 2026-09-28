//! v0.2 devices owned by the `fx-dynamics` node (contracts-3 froze the param tables; see
//! docs/ROADMAP.md "v0.2" and the device agent guide there).
//!
//! Dynamics: gate/expander, 3-band multiband compressor, transient shaper.
//!
//! Implementations: [`Gate`] (`gate_device.rs`), [`MultibandCompressor`]
//! (`multiband_device.rs`), [`TransientShaper`] (`shaper_device.rs`), shared DSP in
//! `shared.rs`. **Param ids are stable and append-only** (documents, automation and presets
//! store them): never renumber, only append. Each descriptor carries its declarative layout
//! (drawn by the shared device renderer); gain reduction reaches the layout meters as
//! `AnalysisKind::Levels` (values in dB, ≤ 0 = reduction).
//!
//! # Gate (`BuiltinDeviceType::Gate`)
//!
//! Sidechain input (`sidechain_inputs = 2`): when a source is set it keys the detector (`Node::process_sidechain`, filtered by `Sidechain HPF`), like the compressor. Lookahead is latency (`Node::latency`). Gate state for the layout meter: `AnalysisKind::Levels` `[gain reduction dB]`.
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Gate | `Threshold` | -80 ..= 0 Decibels, default -40 |
//! | 1 | Gate | `Return` | 0 ..= 24 Decibels, default 3 |
//! | 2 | Gate | `Floor` | -80 ..= 0 Decibels, default -80 |
//! | 3 | Gate | `Ratio` | 1 ..= 20 Ratio (log), default 4 |
//! | 4 | Gate | `Mode` | Gate / Expander (default Gate) |
//! | 5 | Timing | `Attack` | 0.01 ..= 100 Milliseconds (log), default 0.5 |
//! | 6 | Timing | `Hold` | 0 ..= 500 Milliseconds, default 10 |
//! | 7 | Timing | `Release` | 1 ..= 2000 Milliseconds (log), default 100 |
//! | 8 | Timing | `Lookahead` | 0 ..= 10 Milliseconds, default 0 |
//! | 9 | Sidechain | `Sidechain HPF` | 20 ..= 2000 Hertz (log), default 20 |
//! | 10 | Gate | `Flip` | toggle, default off |
//! | 11 | Output | `Output` | -24 ..= 24 Decibels, default 0 |
//!
//! # Multiband Compressor (`BuiltinDeviceType::MultibandCompressor`)
//!
//! Sidechain input (`sidechain_inputs = 2`): when a source is set, its full-band signal keys the detectors of all three bands (`Node::process_sidechain`). Linkwitz-Riley crossovers (flat sum when all bands are neutral). Per-band gain reduction for the layout meters: `AnalysisKind::Levels` `[low, mid, high]` in dB.
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Crossover | `Low / Mid` | 20 ..= 1000 Hertz (log), default 200 |
//! | 1 | Crossover | `Mid / High` | 500 ..= 16000 Hertz (log), default 2500 |
//! | 2 | Low | `Threshold` | -60 ..= 0 Decibels, default -20 |
//! | 3 | Low | `Ratio` | 1 ..= 20 Ratio (log), default 3 |
//! | 4 | Low | `Attack` | 0.1 ..= 200 Milliseconds (log), default 10 |
//! | 5 | Low | `Release` | 5 ..= 2000 Milliseconds (log), default 150 |
//! | 6 | Low | `Makeup` | 0 ..= 24 Decibels, default 0 |
//! | 7 | Low | `Solo` | toggle, default off |
//! | 8 | Low | `Bypass` | toggle, default off |
//! | 9 | Mid | `Threshold` | -60 ..= 0 Decibels, default -20 |
//! | 10 | Mid | `Ratio` | 1 ..= 20 Ratio (log), default 3 |
//! | 11 | Mid | `Attack` | 0.1 ..= 200 Milliseconds (log), default 10 |
//! | 12 | Mid | `Release` | 5 ..= 2000 Milliseconds (log), default 150 |
//! | 13 | Mid | `Makeup` | 0 ..= 24 Decibels, default 0 |
//! | 14 | Mid | `Solo` | toggle, default off |
//! | 15 | Mid | `Bypass` | toggle, default off |
//! | 16 | High | `Threshold` | -60 ..= 0 Decibels, default -20 |
//! | 17 | High | `Ratio` | 1 ..= 20 Ratio (log), default 3 |
//! | 18 | High | `Attack` | 0.1 ..= 200 Milliseconds (log), default 10 |
//! | 19 | High | `Release` | 5 ..= 2000 Milliseconds (log), default 150 |
//! | 20 | High | `Makeup` | 0 ..= 24 Decibels, default 0 |
//! | 21 | High | `Solo` | toggle, default off |
//! | 22 | High | `Bypass` | toggle, default off |
//! | 23 | Output | `Output` | -24 ..= 24 Decibels, default 0 |
//! | 24 | Output | `Mix` | 0 ..= 100 Percent, default 100 |
//!
//! # Transient Shaper (`BuiltinDeviceType::TransientShaper`)
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Shaper | `Attack` | -100 ..= 100 Percent, default 0 |
//! | 1 | Shaper | `Sustain` | -100 ..= 100 Percent, default 0 |
//! | 2 | Shaper | `Attack Time` | 1 ..= 50 Milliseconds (log), default 10 |
//! | 3 | Shaper | `Release Time` | 10 ..= 500 Milliseconds (log), default 100 |
//! | 4 | Output | `Clip` | toggle, default off |
//! | 5 | Output | `Output` | -24 ..= 24 Decibels, default 0 |
//! | 6 | Output | `Mix` | 0 ..= 100 Percent, default 100 |

use ether_core::Device;
use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor, ParamScale, ParamUnit};
use ether_core::protocol::layout::{DeviceLayout, Widget, WidgetSize};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType};

use crate::contract::{
    FactoryPreset, choice, descriptor as build, item, knob, layout, param, section, toggle,
};

mod gate_device;
mod multiband_device;
mod shaper_device;
mod shared;

pub use gate_device::Gate;
pub use multiband_device::MultibandCompressor;
pub use shaper_device::TransientShaper;

/// Param ids of `Gate` (stable, append-only).
pub mod gate {
    use ether_core::protocol::model::ParamId;
    pub const THRESHOLD: ParamId = ParamId(0);
    pub const HYSTERESIS: ParamId = ParamId(1);
    pub const RANGE: ParamId = ParamId(2);
    pub const RATIO: ParamId = ParamId(3);
    pub const MODE: ParamId = ParamId(4);
    pub const ATTACK: ParamId = ParamId(5);
    pub const HOLD: ParamId = ParamId(6);
    pub const RELEASE: ParamId = ParamId(7);
    pub const LOOKAHEAD: ParamId = ParamId(8);
    pub const SIDECHAIN_HPF: ParamId = ParamId(9);
    pub const FLIP: ParamId = ParamId(10);
    pub const OUTPUT: ParamId = ParamId(11);
    /// Number of params.
    pub const COUNT: usize = 12;
}

/// Param ids of `MultibandCompressor` (stable, append-only).
pub mod multiband_compressor {
    use ether_core::protocol::model::ParamId;
    pub const LOW_MID_FREQ: ParamId = ParamId(0);
    pub const MID_HIGH_FREQ: ParamId = ParamId(1);
    pub const LOW_THRESHOLD: ParamId = ParamId(2);
    pub const LOW_RATIO: ParamId = ParamId(3);
    pub const LOW_ATTACK: ParamId = ParamId(4);
    pub const LOW_RELEASE: ParamId = ParamId(5);
    pub const LOW_MAKEUP: ParamId = ParamId(6);
    pub const LOW_SOLO: ParamId = ParamId(7);
    pub const LOW_BYPASS: ParamId = ParamId(8);
    pub const MID_THRESHOLD: ParamId = ParamId(9);
    pub const MID_RATIO: ParamId = ParamId(10);
    pub const MID_ATTACK: ParamId = ParamId(11);
    pub const MID_RELEASE: ParamId = ParamId(12);
    pub const MID_MAKEUP: ParamId = ParamId(13);
    pub const MID_SOLO: ParamId = ParamId(14);
    pub const MID_BYPASS: ParamId = ParamId(15);
    pub const HIGH_THRESHOLD: ParamId = ParamId(16);
    pub const HIGH_RATIO: ParamId = ParamId(17);
    pub const HIGH_ATTACK: ParamId = ParamId(18);
    pub const HIGH_RELEASE: ParamId = ParamId(19);
    pub const HIGH_MAKEUP: ParamId = ParamId(20);
    pub const HIGH_SOLO: ParamId = ParamId(21);
    pub const HIGH_BYPASS: ParamId = ParamId(22);
    pub const OUTPUT: ParamId = ParamId(23);
    pub const MIX: ParamId = ParamId(24);
    /// Number of params.
    pub const COUNT: usize = 25;
}

/// Param ids of `TransientShaper` (stable, append-only).
pub mod transient_shaper {
    use ether_core::protocol::model::ParamId;
    pub const ATTACK: ParamId = ParamId(0);
    pub const SUSTAIN: ParamId = ParamId(1);
    pub const ATTACK_TIME: ParamId = ParamId(2);
    pub const RELEASE_TIME: ParamId = ParamId(3);
    pub const CLIP: ParamId = ParamId(4);
    pub const OUTPUT: ParamId = ParamId(5);
    pub const MIX: ParamId = ParamId(6);
    /// Number of params.
    pub const COUNT: usize = 7;
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

/// Declarative panel of a type (CONTRACTS.md §12.4.2; drawn by the shared renderer).
fn device_layout(ty: BuiltinDeviceType) -> DeviceLayout {
    use WidgetSize::{Large, Medium, Small};
    let meter = |index: u8, min_db: f32, max_db: f32, label: &str| {
        let mut it = item(
            Widget::Meter {
                index,
                min_db,
                max_db,
            },
            Medium,
        );
        it.label = Some(label.to_owned());
        it
    };
    match ty {
        BuiltinDeviceType::Gate => {
            use gate::*;
            layout(vec![
                section(
                    "gate",
                    Some("Gate"),
                    2,
                    4,
                    vec![
                        knob(THRESHOLD, Large),
                        knob(HYSTERESIS, Medium),
                        knob(RANGE, Medium),
                        knob(RATIO, Medium),
                        item(Widget::Choice { param: MODE }, Small),
                        item(Widget::Toggle { param: FLIP }, Small),
                    ],
                ),
                section(
                    "meter",
                    Some("Gain"),
                    1,
                    1,
                    vec![meter(0, -60.0, 0.0, "Gain reduction")],
                ),
                section(
                    "timing",
                    Some("Timing"),
                    2,
                    4,
                    vec![
                        knob(ATTACK, Medium),
                        knob(HOLD, Medium),
                        knob(RELEASE, Medium),
                        knob(LOOKAHEAD, Small),
                    ],
                ),
                section(
                    "sidechain",
                    Some("Sidechain"),
                    1,
                    1,
                    vec![knob(SIDECHAIN_HPF, Small)],
                ),
                section("output", Some("Output"), 1, 1, vec![knob(OUTPUT, Medium)]),
            ])
        }
        BuiltinDeviceType::MultibandCompressor => {
            use multiband_compressor::*;
            let band = |id: &str, title: &str, first: u32, index: u8| {
                let pid = |o: u32| ether_core::protocol::model::ParamId(first + o);
                section(
                    id,
                    Some(title),
                    1,
                    4,
                    vec![
                        knob(pid(0), Large),
                        knob(pid(1), Medium),
                        knob(pid(2), Small),
                        knob(pid(3), Small),
                        knob(pid(4), Small),
                        meter(index, -24.0, 0.0, "GR"),
                        item(Widget::Toggle { param: pid(5) }, Small),
                        item(Widget::Toggle { param: pid(6) }, Small),
                    ],
                )
            };
            layout(vec![
                section(
                    "crossover",
                    Some("Crossover"),
                    4,
                    1,
                    vec![item(
                        Widget::Crossover {
                            frequencies: vec![LOW_MID_FREQ, MID_HIGH_FREQ],
                        },
                        Large,
                    )],
                ),
                band("low", "Low", LOW_THRESHOLD.0, 0),
                band("mid", "Mid", MID_THRESHOLD.0, 1),
                band("high", "High", HIGH_THRESHOLD.0, 2),
                section(
                    "output",
                    Some("Output"),
                    1,
                    1,
                    vec![knob(OUTPUT, Medium), knob(MIX, Medium)],
                ),
            ])
        }
        BuiltinDeviceType::TransientShaper => {
            use transient_shaper::*;
            layout(vec![
                section(
                    "shaper",
                    Some("Shaper"),
                    2,
                    2,
                    vec![
                        knob(ATTACK, Large),
                        knob(SUSTAIN, Large),
                        knob(ATTACK_TIME, Medium),
                        knob(RELEASE_TIME, Medium),
                    ],
                ),
                section(
                    "meter",
                    Some("Gain"),
                    1,
                    1,
                    vec![meter(
                        0,
                        -shaper_device::MAX_GAIN_DB,
                        shaper_device::MAX_GAIN_DB,
                        "Shaping",
                    )],
                ),
                section(
                    "output",
                    Some("Output"),
                    1,
                    1,
                    vec![
                        knob(OUTPUT, Medium),
                        knob(MIX, Medium),
                        item(Widget::Toggle { param: CLIP }, Small),
                    ],
                ),
            ])
        }
        other => unreachable!("{other:?} is not a `fx-dynamics` device"),
    }
}

/// Param table of a type (frozen by contracts-3, append-only).
fn params_descriptor(ty: BuiltinDeviceType) -> DeviceDescriptor {
    match ty {
        BuiltinDeviceType::Gate => build(
            BuiltinDeviceType::Gate,
            "Gate",
            DeviceCategory::AudioEffect,
            vec![
                param(
                    0,
                    "Threshold",
                    "Gate",
                    ParamUnit::Decibels,
                    (-80.0, 0.0, -40.0),
                    ParamScale::Linear,
                ),
                param(
                    1,
                    "Return",
                    "Gate",
                    ParamUnit::Decibels,
                    (0.0, 24.0, 3.0),
                    ParamScale::Linear,
                ),
                param(
                    2,
                    "Floor",
                    "Gate",
                    ParamUnit::Decibels,
                    (-80.0, 0.0, -80.0),
                    ParamScale::Linear,
                ),
                param(
                    3,
                    "Ratio",
                    "Gate",
                    ParamUnit::Ratio,
                    (1.0, 20.0, 4.0),
                    ParamScale::Log,
                ),
                choice(4, "Mode", "Gate", &["Gate", "Expander"], 0),
                param(
                    5,
                    "Attack",
                    "Timing",
                    ParamUnit::Milliseconds,
                    (0.01, 100.0, 0.5),
                    ParamScale::Log,
                ),
                param(
                    6,
                    "Hold",
                    "Timing",
                    ParamUnit::Milliseconds,
                    (0.0, 500.0, 10.0),
                    ParamScale::Linear,
                ),
                param(
                    7,
                    "Release",
                    "Timing",
                    ParamUnit::Milliseconds,
                    (1.0, 2000.0, 100.0),
                    ParamScale::Log,
                ),
                param(
                    8,
                    "Lookahead",
                    "Timing",
                    ParamUnit::Milliseconds,
                    (0.0, 10.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    9,
                    "Sidechain HPF",
                    "Sidechain",
                    ParamUnit::Hertz,
                    (20.0, 2000.0, 20.0),
                    ParamScale::Log,
                ),
                toggle(10, "Flip", "Gate", false),
                param(
                    11,
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
        BuiltinDeviceType::MultibandCompressor => build(
            BuiltinDeviceType::MultibandCompressor,
            "Multiband Compressor",
            DeviceCategory::AudioEffect,
            vec![
                param(
                    0,
                    "Low / Mid",
                    "Crossover",
                    ParamUnit::Hertz,
                    (20.0, 1000.0, 200.0),
                    ParamScale::Log,
                ),
                param(
                    1,
                    "Mid / High",
                    "Crossover",
                    ParamUnit::Hertz,
                    (500.0, 16000.0, 2500.0),
                    ParamScale::Log,
                ),
                param(
                    2,
                    "Threshold",
                    "Low",
                    ParamUnit::Decibels,
                    (-60.0, 0.0, -20.0),
                    ParamScale::Linear,
                ),
                param(
                    3,
                    "Ratio",
                    "Low",
                    ParamUnit::Ratio,
                    (1.0, 20.0, 3.0),
                    ParamScale::Log,
                ),
                param(
                    4,
                    "Attack",
                    "Low",
                    ParamUnit::Milliseconds,
                    (0.1, 200.0, 10.0),
                    ParamScale::Log,
                ),
                param(
                    5,
                    "Release",
                    "Low",
                    ParamUnit::Milliseconds,
                    (5.0, 2000.0, 150.0),
                    ParamScale::Log,
                ),
                param(
                    6,
                    "Makeup",
                    "Low",
                    ParamUnit::Decibels,
                    (0.0, 24.0, 0.0),
                    ParamScale::Linear,
                ),
                toggle(7, "Solo", "Low", false),
                toggle(8, "Bypass", "Low", false),
                param(
                    9,
                    "Threshold",
                    "Mid",
                    ParamUnit::Decibels,
                    (-60.0, 0.0, -20.0),
                    ParamScale::Linear,
                ),
                param(
                    10,
                    "Ratio",
                    "Mid",
                    ParamUnit::Ratio,
                    (1.0, 20.0, 3.0),
                    ParamScale::Log,
                ),
                param(
                    11,
                    "Attack",
                    "Mid",
                    ParamUnit::Milliseconds,
                    (0.1, 200.0, 10.0),
                    ParamScale::Log,
                ),
                param(
                    12,
                    "Release",
                    "Mid",
                    ParamUnit::Milliseconds,
                    (5.0, 2000.0, 150.0),
                    ParamScale::Log,
                ),
                param(
                    13,
                    "Makeup",
                    "Mid",
                    ParamUnit::Decibels,
                    (0.0, 24.0, 0.0),
                    ParamScale::Linear,
                ),
                toggle(14, "Solo", "Mid", false),
                toggle(15, "Bypass", "Mid", false),
                param(
                    16,
                    "Threshold",
                    "High",
                    ParamUnit::Decibels,
                    (-60.0, 0.0, -20.0),
                    ParamScale::Linear,
                ),
                param(
                    17,
                    "Ratio",
                    "High",
                    ParamUnit::Ratio,
                    (1.0, 20.0, 3.0),
                    ParamScale::Log,
                ),
                param(
                    18,
                    "Attack",
                    "High",
                    ParamUnit::Milliseconds,
                    (0.1, 200.0, 10.0),
                    ParamScale::Log,
                ),
                param(
                    19,
                    "Release",
                    "High",
                    ParamUnit::Milliseconds,
                    (5.0, 2000.0, 150.0),
                    ParamScale::Log,
                ),
                param(
                    20,
                    "Makeup",
                    "High",
                    ParamUnit::Decibels,
                    (0.0, 24.0, 0.0),
                    ParamScale::Linear,
                ),
                toggle(21, "Solo", "High", false),
                toggle(22, "Bypass", "High", false),
                param(
                    23,
                    "Output",
                    "Output",
                    ParamUnit::Decibels,
                    (-24.0, 24.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    24,
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
            2,
        ),
        BuiltinDeviceType::TransientShaper => build(
            BuiltinDeviceType::TransientShaper,
            "Transient Shaper",
            DeviceCategory::AudioEffect,
            vec![
                param(
                    0,
                    "Attack",
                    "Shaper",
                    ParamUnit::Percent,
                    (-100.0, 100.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    1,
                    "Sustain",
                    "Shaper",
                    ParamUnit::Percent,
                    (-100.0, 100.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    2,
                    "Attack Time",
                    "Shaper",
                    ParamUnit::Milliseconds,
                    (1.0, 50.0, 10.0),
                    ParamScale::Log,
                ),
                param(
                    3,
                    "Release Time",
                    "Shaper",
                    ParamUnit::Milliseconds,
                    (10.0, 500.0, 100.0),
                    ParamScale::Log,
                ),
                toggle(4, "Clip", "Output", false),
                param(
                    5,
                    "Output",
                    "Output",
                    ParamUnit::Decibels,
                    (-24.0, 24.0, 0.0),
                    ParamScale::Linear,
                ),
                param(
                    6,
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
        other => unreachable!("{other:?} is not a `fx-dynamics` device"),
    }
}

/// Non-RT. A new instance.
///
/// # Panics
/// For a type of another group.
pub fn create(device: &BuiltinDevice) -> Box<dyn Device> {
    match device.device_type() {
        BuiltinDeviceType::Gate => Box::new(Gate::new()),
        BuiltinDeviceType::MultibandCompressor => Box::new(MultibandCompressor::new()),
        BuiltinDeviceType::TransientShaper => Box::new(TransientShaper::new()),
        other => unreachable!("{other:?} is not a `fx-dynamics` device"),
    }
}

macro_rules! presets {
    ($key:literal: $($slug:literal),* $(,)?) => {
        &[$(FactoryPreset {
            id: concat!($key, "/", $slug),
            json: include_str!(concat!("../../presets/", $key, "/", $slug, ".etherpreset")),
        }),*]
    };
}

/// Factory presets of a type of this group (embedded from `presets/<device-key>/`).
pub fn factory_presets(ty: BuiltinDeviceType) -> &'static [FactoryPreset] {
    match ty {
        BuiltinDeviceType::Gate => {
            presets!("gate": "tight-drums", "gentle-expander", "noise-cleanup", "sidechain-trigger")
        }
        BuiltinDeviceType::MultibandCompressor => {
            presets!("multiband-compressor": "gentle-glue", "tame-lows", "de-harsh", "heavy-squash")
        }
        BuiltinDeviceType::TransientShaper => {
            presets!("transient-shaper": "punchy-drums", "tight-room", "soft-attack", "snappy-snare")
        }
        _ => &[],
    }
}

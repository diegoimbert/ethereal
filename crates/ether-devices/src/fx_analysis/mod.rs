//! v0.2 devices owned by the `fx-analysis` node (contracts-3 froze the param tables; see
//! docs/ROADMAP.md "v0.2" and the device agent guide there).
//!
//! Analysis devices (audio passes through untouched): spectrum analyzer and tuner. They publish `AnalysisKind::{Spectrum, Tuner}` frames through `Node::{has_analysis, analysis}` (`ether_core::analysis`).
//!
//! DSP in [`spectrum`] and [`tuner_dsp`] (FFT in `fft`). **Param ids are stable and
//! append-only** (documents, automation and presets store them): never renumber, only append.
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

mod fft;
pub mod spectrum;
pub mod tuner_dsp;

use ether_core::Device;
use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor, ParamScale, ParamUnit};
use ether_core::protocol::layout::{DeviceLayout, Widget, WidgetSize};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType};

use crate::contract::{
    FactoryPreset, choice, descriptor as build, item, knob, layout, param, section, toggle,
};

pub use spectrum::SpectrumAnalyzer;
pub use tuner_dsp::Tuner;

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
    let mut d = params_descriptor(ty);
    d.layout = Some(match ty {
        BuiltinDeviceType::SpectrumAnalyzer => spectrum_layout(),
        _ => tuner_layout(),
    });
    d
}

/// Spectrum: the plot across the panel, then the analysis and display settings.
fn spectrum_layout() -> DeviceLayout {
    use spectrum_analyzer as p;
    let mut plot = item(Widget::Spectrum, WidgetSize::Large);
    plot.colspan = 6;
    layout(vec![
        section("spectrum", None, 4, 6, vec![plot]),
        section(
            "analyzer",
            Some("Analyzer"),
            2,
            3,
            vec![
                item(
                    Widget::Choice {
                        param: p::BLOCK_SIZE,
                    },
                    WidgetSize::Small,
                ),
                knob(p::AVERAGING, WidgetSize::Medium),
                item(Widget::Choice { param: p::CHANNEL }, WidgetSize::Small),
            ],
        ),
        section(
            "display",
            Some("Display"),
            2,
            3,
            vec![
                knob(p::RANGE, WidgetSize::Medium),
                knob(p::SLOPE, WidgetSize::Medium),
                item(
                    Widget::Toggle {
                        param: p::PEAK_HOLD,
                    },
                    WidgetSize::Small,
                ),
            ],
        ),
    ])
}

/// Tuner: the note/needle display, with reference, input and mute beside it.
fn tuner_layout() -> DeviceLayout {
    use tuner as p;
    layout(vec![
        section(
            "tuner",
            None,
            3,
            1,
            vec![item(Widget::Tuner, WidgetSize::Large)],
        ),
        section(
            "settings",
            Some("Tuner"),
            1,
            1,
            vec![
                knob(p::REFERENCE, WidgetSize::Medium),
                item(Widget::Choice { param: p::INPUT }, WidgetSize::Small),
                item(Widget::Toggle { param: p::MUTE }, WidgetSize::Small),
            ],
        ),
    ])
}

/// The frozen param tables (contracts-3).
fn params_descriptor(ty: BuiltinDeviceType) -> DeviceDescriptor {
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

/// Non-RT. A new instance.
///
/// # Panics
/// For a type of another group.
pub fn create(device: &BuiltinDevice) -> Box<dyn Device> {
    match device.device_type() {
        BuiltinDeviceType::SpectrumAnalyzer => Box::new(spectrum::create()),
        BuiltinDeviceType::Tuner => Box::new(tuner_dsp::create()),
        other => unreachable!("{other:?} is not a `fx-analysis` device"),
    }
}

macro_rules! preset {
    ($key:literal, $slug:literal) => {
        FactoryPreset {
            id: concat!($key, "/", $slug),
            json: include_str!(concat!("../../presets/", $key, "/", $slug, ".etherpreset")),
        }
    };
}

const SPECTRUM_PRESETS: &[FactoryPreset] = &[
    preset!("spectrum-analyzer", "mix-balance"),
    preset!("spectrum-analyzer", "fast-transients"),
    preset!("spectrum-analyzer", "fine-resolution"),
    preset!("spectrum-analyzer", "stereo-side"),
    preset!("spectrum-analyzer", "flat-tilt"),
];

const TUNER_PRESETS: &[FactoryPreset] = &[
    preset!("tuner", "concert-440"),
    preset!("tuner", "orchestra-442"),
    preset!("tuner", "baroque-415"),
    preset!("tuner", "silent-tuning"),
];

/// Factory presets of a type of this group (embedded).
pub fn factory_presets(ty: BuiltinDeviceType) -> &'static [FactoryPreset] {
    match ty {
        BuiltinDeviceType::SpectrumAnalyzer => SPECTRUM_PRESETS,
        BuiltinDeviceType::Tuner => TUNER_PRESETS,
        _ => &[],
    }
}

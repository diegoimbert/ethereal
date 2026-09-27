//! Declarative device layouts (v0.2, frozen by contracts-3; renderer: `device-ui` node;
//! CONTRACTS.md §12.4.2).
//!
//! Every built-in device ships a [`DeviceLayout`] in its `DeviceDescriptor::layout` (written
//! by the device's node in its `ether-devices` module; the mock descriptors carry the same JSON).
//! **One shared renderer** (`ui/src/features/devices/layout/`, kit components + tokens only)
//! renders every built-in device from it, so the visual design is done once. Device nodes
//! write specs and widget data only, never bespoke panels. Plugins and devices without a
//! layout get the generic layout (params grouped by `ParamInfo::group`).
//!
//! Layout model: a panel is a row of [`LayoutSection`]s (wrapping when narrow); a section is
//! a titled grid of [`LayoutItem`]s placed in reading order. Sizes are semantic
//! ([`WidgetSize`]); the renderer maps them to tokens. Every param referenced must exist in
//! the descriptor; params not referenced anywhere are still reachable in the generic
//! "all parameters" view. MIDI-learn targets, modulation drop targets and depth rings are
//! added by the renderer on every param widget.
//!
//! The widget catalog is append-only: new widgets need a BCR (renderer + this enum).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::eq_response::EqShape;
use crate::model::ParamId;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct DeviceLayout {
    pub sections: Vec<LayoutSection>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct LayoutSection {
    /// Stable id (unique in the layout; used for collapse state).
    pub id: String,
    pub title: Option<String>,
    /// Relative width weight in the panel row, `1..=4`.
    pub span: u8,
    /// Grid columns inside the section (`1..=8`); items flow row by row.
    pub columns: u8,
    pub items: Vec<LayoutItem>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct LayoutItem {
    pub widget: Widget,
    pub size: WidgetSize,
    /// Grid cells spanned horizontally (`1..=columns`).
    pub colspan: u8,
    /// `None` = the param's name (or the widget's default caption).
    pub label: Option<String>,
}

/// Semantic prominence; the renderer maps it to token sizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum WidgetSize {
    Small,
    Medium,
    /// The device's hero controls (cutoff, drive, mix...).
    Large,
}

/// Widget catalog (append-only). Param widgets bind one param; typed widgets bind several
/// and/or read widget data (device kind data or `AnalysisData`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum Widget {
    // --- param widgets ---
    Knob {
        param: ParamId,
    },
    Slider {
        param: ParamId,
        vertical: bool,
    },
    Toggle {
        param: ParamId,
    },
    /// Stepped/enum param: segmented control (≤ 5 labels) or dropdown.
    Choice {
        param: ParamId,
    },
    /// Numeric field (drag/type), e.g. voices, semitones.
    Number {
        param: ParamId,
    },
    // --- typed widgets (bind several params) ---
    /// ADSR(+delay/hold) graph with draggable breakpoints.
    Envelope {
        attack: ParamId,
        decay: ParamId,
        sustain: ParamId,
        release: ParamId,
        delay: Option<ParamId>,
        hold: Option<ParamId>,
    },
    /// Filter response curve; drag = cutoff (x) + resonance (y).
    FilterCurve {
        cutoff: ParamId,
        resonance: ParamId,
        mode: Option<ParamId>,
        drive: Option<ParamId>,
        gain: Option<ParamId>,
    },
    /// Waveshaper transfer curve (saturator/bitcrusher).
    TransferCurve {
        drive: ParamId,
        curve: Option<ParamId>,
        bias: Option<ParamId>,
    },
    /// Oscillator shape preview (virtual-analog shape or wavetable position).
    Oscillator {
        shape: ParamId,
        position: Option<ParamId>,
    },
    /// LFO shape preview with rate.
    Lfo {
        shape: ParamId,
        rate: ParamId,
        amount: Option<ParamId>,
    },
    /// Step sequence editor over params `first..first+count` (one step per param).
    StepEditor {
        first: ParamId,
        count: u16,
    },
    /// Two params on one pad.
    XyPad {
        x: ParamId,
        y: ParamId,
    },
    /// Crossover/band display for multiband devices (band split frequencies).
    Crossover {
        frequencies: Vec<ParamId>,
    },
    // --- data widgets ---
    /// The sample waveform of the device (sampler: `start`/`end` params as handles).
    SampleWaveform {
        start: Option<ParamId>,
        end: Option<ParamId>,
    },
    /// Multisampler zone map (keys × velocities) from `BuiltinDevice::MultiSampler::zones`.
    ZoneMap,
    /// `AnalysisData::Spectrum` (requires `Analysis::Watch`).
    Spectrum,
    /// `AnalysisData::Tuner`.
    Tuner,
    /// `AnalysisData::Levels::values[index]` as a meter, in dB between `min_db` and `max_db`
    /// (e.g. gain reduction).
    Meter {
        index: u8,
        min_db: f32,
        max_db: f32,
    },
    /// Rack view: chains list with mix/zones, and the macro bank.
    RackChains,
    /// The 8 macro knobs of a rack (params 0..8).
    Macros,
    /// Interactive EQ curve (v0.2, `graphical-eq`): the combined magnitude response of
    /// `bands` (`ether_protocol::eq_response`, mirrored in TS) on a log-frequency / dB grid,
    /// with a draggable handle per band: drag = freq (x) + gain (y), wheel or Alt-drag = Q,
    /// double-click = toggle `on`, context menu = `kind`. Each drag is one gesture (one undo
    /// step). `crossovers` draws vertical handles (drag = frequency), e.g. multiband
    /// crossovers. `spectrum` overlays `AnalysisData::Spectrum` frames (watched device).
    /// Serves the EQ (8 bands), the auto filter (1 band) and the multiband compressor
    /// (crossovers only).
    EqCurve {
        bands: Vec<EqBandBinding>,
        crossovers: Vec<ParamId>,
        spectrum: SpectrumOverlay,
    },
}

/// One band of an [`Widget::EqCurve`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct EqBandBinding {
    /// On/off toggle param (`None` = always on).
    pub on: Option<ParamId>,
    /// Enum param selecting the shape (`None` = `shapes[0]`).
    pub kind: Option<ParamId>,
    /// Response shape for each plain value of `kind` (index = value).
    pub shapes: Vec<EqShape>,
    pub freq: ParamId,
    /// `None` = 0 dB (cuts, band-pass, notch).
    pub gain: Option<ParamId>,
    /// `None` = Q 1/√2.
    pub q: Option<ParamId>,
}

/// Spectrum drawn behind an [`Widget::EqCurve`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum SpectrumOverlay {
    None,
    /// `SpectrumStage::Post` frames.
    Post,
    /// Both `Pre` (input) and `Post` (output) frames.
    PrePost,
}

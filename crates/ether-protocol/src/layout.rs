//! Declarative device layouts (v0.2, frozen by the v0.2 contracts; renderer: the device UI;
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
//! The widget catalog is append-only: a new widget is a protocol change (renderer + this enum).

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
    /// The macro knobs of a rack: macros 1..8 are params `first..first + 8`, macros 9..16
    /// `upper..upper + 8` (a chain rack's 0 and 9, after its chain selector; a drum rack's
    /// after its volume and pan, 2 and 10; `ether_model::rack::macro_param`). It shows the
    /// rack's `RackMacros::visible` macros under their names, with show/hide buttons.
    /// `first` defaults to 0 and `upper` to 9 when absent.
    Macros {
        #[serde(default)]
        first: ParamId,
        #[serde(default = "default_macros_upper")]
        upper: ParamId,
    },
    /// Interactive EQ curve (v0.2): the combined magnitude response of
    /// `bands` (`ether_protocol::eq_response`, mirrored in TS) on a log-frequency / dB grid,
    /// with a draggable handle per band: drag = freq (x) + gain (y), wheel or Alt-drag = Q,
    /// double-click = toggle `on`, context menu = `kind`. Each drag is one gesture (one undo
    /// step). `crossovers` draws vertical handles (drag = frequency), e.g. multiband
    /// crossovers. `spectrum` overlays `AnalysisData::Spectrum` frames (watched device).
    /// v0.2: the EQ only (8 bands). Other filters (auto filter, multiband crossovers) use
    /// `FilterCurve` / `Crossover` for now and may adopt it later with an explicit param
    /// mapping.
    EqCurve {
        bands: Vec<EqBandBinding>,
        crossovers: Vec<ParamId>,
        spectrum: SpectrumOverlay,
    },
    /// v0.3 (`external-instrument`): the hardware routing of an External Instrument /
    /// External Audio Effect (`BuiltinDevice::{ExternalInstrument, ExternalAudioEffect}::
    /// routing`): MIDI output + channel, audio send / return channels, missing ports, and
    /// the `External::MeasureLatency` button. No params.
    HardwareRouting,
    // --- effect window visualisations ---
    /// Static transfer curve of a dynamics device (input dB → output dB on a -60..0 dB
    /// square), drag = threshold (x) and ratio (y). Behaviour by the `mode` param's current
    /// label: "Expand…" = downward expander, "Gate" = gate (to `floor`), else a compressor
    /// (`ratio: None` = brickwall limiter). `knee` in dB (`None` = hard). `input_gain` shifts
    /// the input (a limiter's Gain). `level`: index of the detector level (dB) in the device's
    /// `AnalysisData::Levels`, drawn as a live dot on the curve.
    DynamicsCurve {
        threshold: ParamId,
        ratio: Option<ParamId>,
        knee: Option<ParamId>,
        mode: Option<ParamId>,
        floor: Option<ParamId>,
        input_gain: Option<ParamId>,
        level: Option<u8>,
    },
    /// Echo display of a delay: the taps at multiples of the delay time (ms, or the synced
    /// `division` while `sync` is on, at the project tempo), their level decaying by
    /// `feedback` (percent), alternating sides with `ping_pong`. Drag = time (x) + feedback (y).
    DelayTaps {
        time: ParamId,
        feedback: ParamId,
        sync: Option<ParamId>,
        division: Option<ParamId>,
        ping_pong: Option<ParamId>,
        freeze: Option<ParamId>,
    },
    /// Decay display of a reverb: pre-delay gap, then the tail falling by 60 dB over `decay`
    /// (seconds, or percent of the impulse response), its shading by `damping` (percent).
    /// Drag = decay (x). `freeze` (toggle) draws an endless tail.
    ReverbDecay {
        decay: ParamId,
        pre_delay: Option<ParamId>,
        size: Option<ParamId>,
        damping: Option<ParamId>,
        freeze: Option<ParamId>,
    },
    /// Velocity mapping curve of a MIDI velocity effect (input 1..127 → output 1..127):
    /// the input range, the `drive` (power, ±100 %) and `compand` (S-curve, ±100 %) shaping
    /// and the output range; `mode` "Fixed" draws a flat line at `out_high`, "Gate" greys
    /// what falls outside the input range; `random` shades the ± spread. Drag = drive (y).
    VelocityCurve {
        drive: ParamId,
        compand: Option<ParamId>,
        in_low: Option<ParamId>,
        in_high: Option<ParamId>,
        out_low: Option<ParamId>,
        out_high: Option<ParamId>,
        mode: Option<ParamId>,
        random: Option<ParamId>,
    },
    // --- fm-synth ---
    /// Algorithm picker of an FM synth: one small routing diagram per algorithm, the current
    /// one (the plain value of the `algorithm` choice param) highlighted; click = select.
    /// `algorithms[i]` is the routing of plain value `i` (one entry per label).
    FmAlgorithm {
        algorithm: ParamId,
        algorithms: Vec<FmRouting>,
    },
    // --- fx-modulation ---
    /// Editor of a Volume LFO's drawn cycle (`BuiltinDevice::VolumeLfo::curve`, written with
    /// `Device::SetLfoCurve`): breakpoints on a grid of the cycle's divisions and a playhead.
    /// `sync` / `sync_rate` / `rate` set the grid and the playhead's speed, `phase` its
    /// offset; `depth` scales the drawn gain.
    LfoCurve {
        sync: ParamId,
        sync_rate: ParamId,
        rate: ParamId,
        phase: ParamId,
        depth: ParamId,
    },
    // --- fx-analysis ---
    /// A loudness meter's readouts and history (`AnalysisData::Levels` of a `Loudness`:
    /// momentary, short-term, integrated, range, true peak max / now, window and step in
    /// seconds, `n`, then `n` momentary and `n` short-term points, oldest first). `target`
    /// (dB, read as LUFS) draws the reference line.
    Loudness {
        target: ParamId,
    },
    // --- maximizer ---
    /// A maximizer's history: input level, output level and gain reduction over the last
    /// seconds, with the `threshold` and `ceiling` lines and the readouts
    /// (`AnalysisData::Levels` of a `Maximizer`, layout in `ether_devices::maximizer::Frame`).
    Maximizer {
        threshold: ParamId,
        ceiling: ParamId,
    },
}

/// Operator routing of one algorithm of a [`Widget::FmAlgorithm`]. Operators are numbered
/// from 0 (drawn "A", at the bottom) upward.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct FmRouting {
    /// Per operator, the bit mask of the operators that modulate it (bit `j` = operator `j`).
    /// Its length is the number of operators.
    pub modulators: Vec<u8>,
    /// Bit mask of the operators heard at the output (carriers).
    pub carriers: u8,
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

/// [`Widget::Macros::upper`] when absent: a chain rack's macro 9 (after its chain selector).
fn default_macros_upper() -> ParamId {
    ParamId(9)
}

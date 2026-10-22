//! Compressor: threshold, ratio, attack, release, makeup gain, sidechain HPF, detector.
//!
//! Feed-forward, stereo-linked detector with a fixed 6 dB soft knee. The gain reduction (in
//! dB) is smoothed with separate attack/release time constants by a smooth decoupled peak
//! detector ([`ballistics`]): the reduction a peak asks for is held between the peaks of a
//! waveform (released with `Release`), then approached with `Attack`, so a steady tone gets
//! the static curve's reduction (the branching smoother let it sag between the peaks
//! of a sine, 1 dB short of 4:1 at 0 dBFS).
//!
//! `Detector` (param 6, appended by a later pass):
//! - **Peak** (default): the per-sample peak of the louder channel.
//! - **RMS**: the root of the mean square of the louder channel, averaged by a one-pole
//!   filter with an [`RMS_WINDOW_MS`] time constant, so short transients count less than
//!   sustained level.
//! - **Expand**: downward expander on the peak detector: below the threshold the gain drops
//!   by `ratio - 1` dB per dB under it (at most [`MAX_EXPAND_DB`]); above it, unity. Attack
//!   is how fast it opens, release how fast it closes.
//!
//! Sidechain (roadmap v2, `sidechain`): the device has a stereo sidechain input
//! (`sidechain_inputs = 2`). When the chain entry has a sidechain source, the engine calls
//! [`Node::process_sidechain`] with the (latency-aligned) source signal and the detector
//! listens to it instead of the main input; the gain is still applied to the main input.
//! The detector signal of the sidechain goes through a high-pass (`Sidechain HPF`, off at
//! its 20 Hz minimum; one-pole 6 dB/oct, or 12 dB/oct with `Sidechain Slope`) so a kick's
//! sub doesn't dominate.
//!
//! The effects pass appends (defaults = the sound before them, bit-exact):
//! - `Knee` (7, 0..=24 dB, default the former fixed 6 dB): quadratic soft knee width.
//! - `Mix` (8, 0..=100 %, default 100): parallel compression (dry/wet, latency-aligned).
//! - `Lookahead` (9, 0..=10 ms, default 0): the audio is delayed so the gain is already down
//!   when a transient arrives; reported as latency (`Node::latency`).
//! - `Auto Makeup` (10, toggle, default off): adds half the static gain reduction at 0 dBFS
//!   to `Makeup` (follows threshold, ratio and knee).
//!
//! Appended for Live's Glue Compressor (defaults bit-exact):
//! - `Range` (11, 0..=80 dB, default 80 = no limit): the most gain reduction applied (the
//!   static curve's target is capped; 0 = no compression). Compressing detectors only.
//! - `Soft Clip` (12, toggle, default off): the output (after the mix) goes through
//!   [`soft_clip`]: linear up to -6 dBFS, then a tanh curve towards -0.5 dBFS.
//!
//! Appended for Live's sidechain EQ (default bit-exact):
//! - `Sidechain Slope` (13, choice, default 6 dB/oct = the one-pole above): 12 dB/oct makes
//!   the sidechain high-pass a 2nd-order Butterworth (Q = 1/√2, Live's sidechain EQ
//!   default), -3 dB at the cutoff. Both filters run while the high-pass is on and the
//!   detector crossfades between them ([`SLOPE_FADE_MS`]), so switching doesn't click.
//!
//! Analysis (the window's meter and transfer-curve dot): `AnalysisKind::Levels`
//! `[gain reduction dB (<= 0), detector level dBFS]`, the deepest reduction and the loudest
//! detector peak since the last read.

use ether_core::protocol::devices::{
    DeviceCategory, DeviceDescriptor, DeviceTypeRef, ParamInfo, ParamScale, ParamUnit,
};
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus, Smoother,
};

use crate::dsp::delay_line::{DelayLine, LookaheadTap};
use crate::util;
use ether_core::{AnalysisKind, AnalysisSink};

/// Parameter ids (stable: stored in documents and automation).
pub mod params {
    use ether_core::protocol::model::ParamId;
    pub const THRESHOLD: ParamId = ParamId(0);
    pub const RATIO: ParamId = ParamId(1);
    pub const ATTACK: ParamId = ParamId(2);
    pub const RELEASE: ParamId = ParamId(3);
    pub const MAKEUP: ParamId = ParamId(4);
    /// Roadmap v2 (`sidechain`): high-pass cutoff of the sidechain detector signal (Hz; the
    /// 20 Hz minimum = off). Only used while a sidechain is connected.
    pub const SIDECHAIN_HPF: ParamId = ParamId(5);
    /// Detector mode, a choice of [`super::DETECTOR_LABELS`] (0 = Peak, the
    /// default, so documents without it behave as before).
    pub const DETECTOR: ParamId = ParamId(6);
    /// Soft-knee width (dB).
    pub const KNEE: ParamId = ParamId(7);
    /// Dry/wet (percent, 100 = fully compressed).
    pub const MIX: ParamId = ParamId(8);
    /// Lookahead (ms; latency).
    pub const LOOKAHEAD: ParamId = ParamId(9);
    /// Automatic makeup gain (toggle).
    pub const AUTO_MAKEUP: ParamId = ParamId(10);
    /// Most gain reduction the compressor applies (dB; the 80 dB maximum = no limit).
    pub const RANGE: ParamId = ParamId(11);
    /// A soft clipper on the output, peaking at [`super::SOFT_CLIP_CEILING_DB`] (toggle).
    pub const SOFT_CLIP: ParamId = ParamId(12);
    /// Appended (Live's sidechain EQ): slope of the sidechain high-pass, 0 = 6 dB/oct
    /// (one-pole, the default), 1 = 12 dB/oct (2nd-order Butterworth).
    pub const SIDECHAIN_SLOPE: ParamId = ParamId(13);
}

const NUM_PARAMS: usize = 14;
/// `Range` at this many dB (its maximum) doesn't limit the gain reduction.
pub const RANGE_OFF_DB: f32 = 80.0;
/// The soft clipper's ceiling (dBFS): Live's Glue Compressor's ("the maximum output will be
/// -0.5 dB", Live Reference Manual).
pub const SOFT_CLIP_CEILING_DB: f32 = -0.5;
/// The soft clipper is linear up to this level (linear amplitude, -6 dBFS).
const SOFT_CLIP_KNEE: f32 = 0.5;

/// The output soft clipper: linear up to [`SOFT_CLIP_KNEE`], then a tanh curve (slope 1 at
/// the knee) that approaches the ceiling.
#[inline]
pub fn soft_clip(x: f32) -> f32 {
    let a = x.abs();
    if a <= SOFT_CLIP_KNEE {
        return x;
    }
    let room = util::db_to_amp(SOFT_CLIP_CEILING_DB) - SOFT_CLIP_KNEE;
    (SOFT_CLIP_KNEE + room * ((a - SOFT_CLIP_KNEE) / room).tanh()).copysign(x)
}
/// Longest lookahead (ms).
pub const MAX_LOOKAHEAD_MS: f32 = 10.0;
/// Detector level reported when nothing was measured (dBFS).
const METER_FLOOR_DB: f32 = -90.0;
/// Labels of the `Detector` choice (indices are stable).
pub const DETECTOR_LABELS: [&str; 3] = ["Peak", "RMS", "Expand"];
const DETECTOR_PEAK: usize = 0;
const DETECTOR_RMS: usize = 1;
const DETECTOR_EXPAND: usize = 2;
/// Time constant of the RMS detector's mean-square average.
pub const RMS_WINDOW_MS: f32 = 10.0;
/// Deepest gain reduction of the expander (dB).
pub const MAX_EXPAND_DB: f32 = 60.0;
/// `Sidechain HPF` at or below this cutoff (Hz) is off.
const HPF_OFF_HZ: f32 = 20.0;
/// `Sidechain Slope` labels, in plain-value order.
pub const SLOPE_LABELS: [&str; 2] = ["6 dB/oct", "12 dB/oct"];
/// Crossfade between the two sidechain high-pass slopes when switching (ms).
pub const SLOPE_FADE_MS: f32 = 20.0;
/// Soft-knee width in dB.
pub(crate) const KNEE_DB: f32 = 6.0;

/// Parameter list (identical for every instance).
pub fn param_infos() -> Vec<ParamInfo> {
    use util::{choice, param};
    vec![
        param(
            0,
            "Threshold",
            "Compressor",
            ParamUnit::Decibels,
            (-60.0, 0.0, -18.0),
            ParamScale::Linear,
        ),
        param(
            1,
            "Ratio",
            "Compressor",
            ParamUnit::Ratio,
            (1.0, 20.0, 4.0),
            ParamScale::Log,
        ),
        param(
            2,
            "Attack",
            "Compressor",
            ParamUnit::Milliseconds,
            (0.1, 200.0, 10.0),
            ParamScale::Log,
        ),
        param(
            3,
            "Release",
            "Compressor",
            ParamUnit::Milliseconds,
            (5.0, 2000.0, 150.0),
            ParamScale::Log,
        ),
        param(
            4,
            "Makeup",
            "Output",
            ParamUnit::Decibels,
            (0.0, 24.0, 0.0),
            ParamScale::Linear,
        ),
        param(
            5,
            "Sidechain HPF",
            "Sidechain",
            ParamUnit::Hertz,
            (HPF_OFF_HZ as f64, 500.0, HPF_OFF_HZ as f64),
            ParamScale::Log,
        ),
        choice(
            6,
            "Detector",
            "Compressor",
            ParamUnit::None,
            &DETECTOR_LABELS,
            DETECTOR_PEAK,
        ),
        // --- devices pass effects ---
        param(
            7,
            "Knee",
            "Compressor",
            ParamUnit::Decibels,
            (0.0, 24.0, KNEE_DB as f64),
            ParamScale::Linear,
        ),
        param(
            8,
            "Mix",
            "Output",
            ParamUnit::Percent,
            (0.0, 100.0, 100.0),
            ParamScale::Linear,
        ),
        param(
            9,
            "Lookahead",
            "Compressor",
            ParamUnit::Milliseconds,
            (0.0, MAX_LOOKAHEAD_MS as f64, 0.0),
            ParamScale::Linear,
        ),
        choice(
            10,
            "Auto Makeup",
            "Output",
            ParamUnit::Toggle,
            &["Off", "On"],
            0,
        ),
        // --- appended (Ableton import: Live's Glue Compressor) ---
        param(
            11,
            "Range",
            "Compressor",
            ParamUnit::Decibels,
            (0.0, RANGE_OFF_DB as f64, RANGE_OFF_DB as f64),
            ParamScale::Linear,
        ),
        choice(
            12,
            "Soft Clip",
            "Output",
            ParamUnit::Toggle,
            &["Off", "On"],
            0,
        ),
        // --- appended (Ableton import: Live's sidechain EQ high-pass) ---
        choice(
            13,
            "Sidechain Slope",
            "Sidechain",
            ParamUnit::None,
            &SLOPE_LABELS,
            0,
        ),
    ]
}

/// Descriptor of the compressor type.
pub fn descriptor() -> DeviceDescriptor {
    DeviceDescriptor {
        layout: Some(layout()),
        device_type: DeviceTypeRef::Builtin {
            device: BuiltinDeviceType::Compressor,
        },
        name: "Compressor".to_owned(),
        category: DeviceCategory::AudioEffect,
        params: param_infos(),
        audio_inputs: 2,
        audio_outputs: 2,
        midi_input: false,
        sidechain_inputs: SIDECHAIN_CHANNELS as u16,
        implemented: true,
    }
}

/// Declarative panel (drawn by the shared renderer in the device's window): the transfer
/// curve (with its threshold / ratio / knee / detector controls and the live detector dot)
/// beside the gain-reduction meter, then the timing, output and sidechain controls.
pub fn layout() -> ether_core::protocol::layout::DeviceLayout {
    use crate::contract::{item, knob, layout, section};
    use ether_core::protocol::layout::{LayoutItem, Widget, WidgetSize::*};
    layout(vec![
        section(
            "curve",
            Some("Compressor"),
            3,
            4,
            vec![
                LayoutItem {
                    colspan: 3,
                    ..item(
                        Widget::DynamicsCurve {
                            threshold: params::THRESHOLD,
                            ratio: Some(params::RATIO),
                            knee: Some(params::KNEE),
                            mode: Some(params::DETECTOR),
                            floor: None,
                            input_gain: None,
                            level: Some(1),
                        },
                        Large,
                    )
                },
                LayoutItem {
                    label: Some("GR".to_owned()),
                    ..item(
                        Widget::Meter {
                            index: 0,
                            min_db: -24.0,
                            max_db: 0.0,
                        },
                        Medium,
                    )
                },
            ],
        ),
        section(
            "timing",
            Some("Timing"),
            2,
            3,
            vec![
                knob(params::ATTACK, Medium),
                knob(params::RELEASE, Medium),
                knob(params::LOOKAHEAD, Small),
            ],
        ),
        section(
            "output",
            Some("Output"),
            1,
            2,
            vec![
                knob(params::MAKEUP, Medium),
                knob(params::MIX, Medium),
                item(
                    Widget::Toggle {
                        param: params::AUTO_MAKEUP,
                    },
                    Small,
                ),
                knob(params::RANGE, Small),
                item(
                    Widget::Toggle {
                        param: params::SOFT_CLIP,
                    },
                    Small,
                ),
            ],
        ),
        section(
            "sidechain",
            Some("Sidechain"),
            1,
            1,
            vec![
                knob(params::SIDECHAIN_HPF, Small),
                item(
                    Widget::Choice {
                        param: params::SIDECHAIN_SLOPE,
                    },
                    Small,
                ),
            ],
        ),
    ])
}

/// Channels of the sidechain input.
const SIDECHAIN_CHANNELS: usize = 2;

/// Static gain computer: gain reduction in dB (>= 0) for an input level in dB, with the
/// default 6 dB knee.
#[inline]
pub(crate) fn gain_reduction_db(level_db: f32, threshold: f32, ratio: f32) -> f32 {
    gain_reduction_knee_db(level_db, threshold, ratio, KNEE_DB)
}

/// Static gain computer with a `knee` dB wide quadratic soft knee (0 = hard knee).
#[inline]
pub fn gain_reduction_knee_db(level_db: f32, threshold: f32, ratio: f32, knee: f32) -> f32 {
    let over = level_db - threshold;
    let slope = 1.0 - 1.0 / ratio;
    if 2.0 * over <= -knee {
        0.0
    } else if 2.0 * over >= knee {
        over * slope
    } else {
        let x = over + knee * 0.5;
        slope * x * x / (2.0 * knee)
    }
}

/// Gain-reduction ballistics shared by the compressors: a smooth decoupled peak
/// detector in the dB domain (Giannoulis, Massberg & Reiss, JAES 2012). `held` jumps up to
/// `target` and releases towards it (`release` one-pole coefficient), so the reduction a
/// waveform's peaks ask for holds between them; `gr` follows `held` with the `attack`
/// coefficient. Both snap to 0 near it (no subnormal tail). Returns the new `gr`.
#[inline]
pub(crate) fn ballistics(
    gr: &mut f32,
    held: &mut f32,
    target: f32,
    attack: f32,
    release: f32,
) -> f32 {
    *held = target.max(target + (*held - target) * release);
    if *held < 1e-6 {
        *held = 0.0;
    }
    *gr = *held + (*gr - *held) * attack;
    if *gr < 1e-6 {
        *gr = 0.0;
    }
    *gr
}

/// The built-in compressor.
#[derive(Debug)]
pub struct Compressor {
    values: [f64; NUM_PARAMS],
    ranges: [(f64, f64); NUM_PARAMS],
    sample_rate: f32,
    attack_coef: f32,
    release_coef: f32,
    makeup: Smoother,
    /// Current smoothed gain reduction in dB.
    gr_db: f32,
    /// The peak-held reduction [`ballistics`] attacks towards (dB).
    gr_held: f32,
    /// Detector mode index (`DETECTOR_*`).
    detector: usize,
    /// RMS detector: one-pole coefficient and running mean square.
    rms_coef: f32,
    mean_square: f32,
    /// One-pole high-pass coefficient of the sidechain detector (`None` = off).
    hpf_coef: Option<f32>,
    /// High-pass state per sidechain channel: (previous input, previous output).
    hpf_state: [(f32, f32); SIDECHAIN_CHANNELS],
    /// 12 dB/oct (Butterworth) high-pass of the sidechain detector and its state per
    /// channel; runs beside the one-pole while the high-pass is on.
    hpf2: crate::dsp::svf::Coefs,
    hpf2_state: [crate::dsp::svf::Svf; SIDECHAIN_CHANNELS],
    /// Weight of the 12 dB/oct output (0 = one-pole only, 1 = Butterworth only).
    slope: Smoother,
    /// Dry/wet (0..1).
    mix: Smoother,
    /// Lookahead delay per main channel and its length in samples (0 = off).
    delay: [DelayLine; 2],
    lookahead: LookaheadTap,
    /// Deepest gain reduction (dB) and loudest detector peak (linear) since the last
    /// analysis read.
    meter_gr: f32,
    meter_peak: f32,
    /// Silence reporting.
    tail: util::Tail,
    /// With lookahead the gain meets the audio `lookahead` samples into the line: every
    /// param but the lookahead itself waits that long (the detector looks ahead by design;
    /// a new threshold applies from the audio it arrived with on).
    late: util::LateEvents,
}

impl Default for Compressor {
    fn default() -> Self {
        Self::new()
    }
}

impl Compressor {
    /// Non-RT. A compressor with default parameters.
    pub fn new() -> Self {
        let infos = param_infos();
        let mut values = [0.0; NUM_PARAMS];
        let mut ranges = [(0.0, 0.0); NUM_PARAMS];
        for (i, info) in infos.iter().enumerate() {
            values[i] = info.default;
            ranges[i] = (info.min, info.max);
        }
        let mut c = Self {
            values,
            ranges,
            sample_rate: 48_000.0,
            attack_coef: 0.0,
            release_coef: 0.0,
            makeup: Smoother::new(1.0, 20.0, 48_000.0),
            gr_db: 0.0,
            gr_held: 0.0,
            detector: DETECTOR_PEAK,
            rms_coef: 0.0,
            mean_square: 0.0,
            hpf_coef: None,
            hpf_state: [(0.0, 0.0); SIDECHAIN_CHANNELS],
            hpf2: crate::dsp::svf::Coefs::BYPASS,
            hpf2_state: [crate::dsp::svf::Svf::ZERO; SIDECHAIN_CHANNELS],
            slope: Smoother::new(0.0, SLOPE_FADE_MS, 48_000.0),
            mix: Smoother::new(1.0, 20.0, 48_000.0),
            delay: [DelayLine::default(), DelayLine::default()],
            lookahead: LookaheadTap::default(),
            meter_gr: 0.0,
            meter_peak: 0.0,
            tail: util::Tail::default(),
            late: util::LateEvents::new(),
        };
        c.allocate();
        c
    }

    /// Non-RT: size the lookahead lines for the sample rate, then sync everything.
    fn allocate(&mut self) {
        let max = (MAX_LOOKAHEAD_MS * 0.001 * self.sample_rate).ceil() as usize + 1;
        self.delay = [DelayLine::new(max), DelayLine::new(max)];
        self.sync_all();
    }

    fn update_lookahead(&mut self, smooth: bool) {
        let n = (self.value(params::LOOKAHEAD) * 0.001 * self.sample_rate).round() as usize;
        self.lookahead.set(n.min(self.delay[0].max_delay()), smooth);
    }

    /// Makeup gain (linear): `Makeup`, plus half the reduction at 0 dBFS with `Auto Makeup`
    /// (compressing detectors only).
    fn makeup_amp(&self) -> f32 {
        let mut db = self.value(params::MAKEUP);
        if self.value(params::AUTO_MAKEUP) >= 0.5 && self.detector != DETECTOR_EXPAND {
            db += 0.5
                * gain_reduction_knee_db(
                    0.0,
                    self.value(params::THRESHOLD),
                    self.value(params::RATIO),
                    self.value(params::KNEE),
                );
        }
        util::db_to_amp(db)
    }

    fn value(&self, id: ParamId) -> f32 {
        self.values[id.0 as usize] as f32
    }

    fn sync_all(&mut self) {
        self.detector = util::index(
            self.values[params::DETECTOR.0 as usize],
            DETECTOR_LABELS.len(),
        );
        self.makeup = Smoother::new(self.makeup_amp(), 20.0, self.sample_rate);
        self.mix = Smoother::new(self.value(params::MIX) * 0.01, 20.0, self.sample_rate);
        self.slope = Smoother::new(self.slope_target(), SLOPE_FADE_MS, self.sample_rate);
        self.update_times();
        self.update_hpf();
        self.update_lookahead(false);
    }

    fn update_hpf(&mut self) {
        let fc = self.value(params::SIDECHAIN_HPF);
        self.hpf_coef = (fc > HPF_OFF_HZ).then(|| {
            // One-pole high-pass: y[n] = a * (y[n-1] + x[n] - x[n-1]).
            let rc = 1.0 / (2.0 * std::f32::consts::PI * fc);
            let dt = 1.0 / self.sample_rate;
            rc / (rc + dt)
        });
        if self.hpf_coef.is_some() {
            use crate::dsp::svf::{Coefs, Shape};
            self.hpf2 = Coefs::new(
                Shape::LowCut,
                f64::from(fc),
                0.0,
                std::f64::consts::FRAC_1_SQRT_2,
                f64::from(self.sample_rate),
            );
        }
    }

    /// Weight of the 12 dB/oct high-pass for the current `Sidechain Slope`.
    fn slope_target(&self) -> f32 {
        if util::index(
            f64::from(self.value(params::SIDECHAIN_SLOPE)),
            SLOPE_LABELS.len(),
        ) == 1
        {
            1.0
        } else {
            0.0
        }
    }

    fn update_times(&mut self) {
        self.attack_coef = util::tau_coef(self.value(params::ATTACK), self.sample_rate);
        self.release_coef = util::tau_coef(self.value(params::RELEASE), self.sample_rate);
        self.rms_coef = util::tau_coef(RMS_WINDOW_MS, self.sample_rate);
    }

    fn apply_param(&mut self, id: ParamId, value: f64, smooth: bool) {
        let Some(&(min, max)) = self.ranges.get(id.0 as usize) else {
            return;
        };
        self.values[id.0 as usize] = util::clamp(value, min, max);
        match id {
            params::ATTACK | params::RELEASE => self.update_times(),
            params::SIDECHAIN_HPF => self.update_hpf(),
            params::SIDECHAIN_SLOPE => {
                let w = self.slope_target();
                if smooth {
                    self.slope.set_target(w);
                } else {
                    self.slope.set_immediate(w);
                }
            }
            params::LOOKAHEAD => self.update_lookahead(smooth),
            params::DETECTOR => {
                self.detector = util::index(value, DETECTOR_LABELS.len());
            }
            params::MIX => {
                let v = self.value(params::MIX) * 0.01;
                if smooth {
                    self.mix.set_target(v);
                } else {
                    self.mix.set_immediate(v);
                }
            }
            _ => {}
        }
        let auto = self.value(params::AUTO_MAKEUP) >= 0.5;
        let moves_makeup = id == params::MAKEUP
            || id == params::AUTO_MAKEUP
            || (auto
                && matches!(
                    id,
                    params::THRESHOLD | params::RATIO | params::KNEE | params::DETECTOR
                ));
        if moves_makeup {
            let amp = self.makeup_amp();
            if smooth {
                self.makeup.set_target(amp);
            } else {
                self.makeup.set_immediate(amp);
            }
        }
    }

    /// Current gain reduction in dB (>= 0), e.g. for tests or a future meter.
    pub fn gain_reduction(&self) -> f32 {
        self.gr_db
    }

    /// Sidechain channel `c`'s high-pass of `x`, `w` the weight of the 12 dB/oct output.
    /// Both slopes keep running while the high-pass is on, so a switch crossfades between
    /// settled filters; at 6 dB/oct the one-pole's output is used as is (bit-exact).
    #[inline]
    fn high_pass(&mut self, c: usize, x: f32, w: f32) -> f32 {
        let Some(a) = self.hpf_coef else {
            return x;
        };
        let st = &mut self.hpf_state[c];
        let y = a * (st.1 + x - st.0);
        // Keep the state out of the subnormal range (no FTZ on the audio thread).
        *st = (x, if y.abs() < 1e-20 { 0.0 } else { y });
        let y2 = self.hpf2_state[c].tick(f64::from(x), &self.hpf2) as f32;
        if w == 0.0 {
            y
        } else if w == 1.0 {
            y2
        } else {
            y + (y2 - y) * w
        }
    }

    /// Detector level (linear peak) of frame `i`: the sidechain if connected (high-passed),
    /// else the main input.
    #[inline]
    fn detect(&mut self, inputs: &[&[f32]], sidechain: &[&[f32]], i: usize) -> f32 {
        if sidechain.is_empty() {
            return inputs.iter().fold(0.0f32, |m, ch| m.max(ch[i].abs()));
        }
        let w = self.slope.tick();
        let mut peak = 0.0f32;
        for (c, ch) in sidechain.iter().enumerate().take(SIDECHAIN_CHANNELS) {
            let y = self.high_pass(c, ch[i], w);
            peak = peak.max(y.abs());
        }
        peak
    }

    fn render(
        &mut self,
        audio: &mut AudioBuffers<'_, '_>,
        sidechain: &[&[f32]],
        start: usize,
        end: usize,
    ) {
        let threshold = self.value(params::THRESHOLD);
        let ratio = self.value(params::RATIO);
        let knee = self.value(params::KNEE);
        // A detector peak this far under the knee's lower edge needs no
        // log10 (its gain reduction is exactly 0 either way; the margin covers rounding).
        let below_knee = util::db_to_amp(threshold - knee * 0.5) * 0.999;
        let inputs = audio.inputs;
        let parallel = self.mix.is_smoothing() || self.mix.current() < 1.0;
        let shaped =
            self.value(params::RANGE) < RANGE_OFF_DB || self.value(params::SOFT_CLIP) >= 0.5;
        if self.detector != DETECTOR_PEAK || self.lookahead.active() || parallel || shaped {
            self.render_modes(audio, sidechain, start, end);
            return;
        }
        for i in start..end {
            // Keep the lookahead lines filled so switching it on doesn't replay stale audio.
            for (c, line) in self.delay.iter_mut().enumerate() {
                line.push(inputs.get(c).map_or(0.0, |ch| ch[i]));
            }
            let peak = self.detect(inputs, sidechain, i);
            self.meter_peak = self.meter_peak.max(peak);
            if peak < below_knee && self.gr_db == 0.0 && self.gr_held == 0.0 {
                // Released and under the knee: gain reduction stays exactly 0, so the gain
                // is the makeup alone (`db_to_amp(-0.0) == 1.0`): same bits, no powf.
                let gain = self.makeup.tick();
                for (o, out) in audio.outputs.iter_mut().enumerate() {
                    out[i] = inputs.get(o).map_or(0.0, |ch| ch[i]) * gain;
                }
                continue;
            }
            let target = if peak < below_knee {
                0.0
            } else {
                gain_reduction_knee_db(util::amp_to_db(peak), threshold, ratio, knee)
            };
            ballistics(
                &mut self.gr_db,
                &mut self.gr_held,
                target,
                self.attack_coef,
                self.release_coef,
            );
            self.meter_gr = self.meter_gr.max(self.gr_db);
            let gain = util::db_to_amp(-self.gr_db) * self.makeup.tick();
            for (o, out) in audio.outputs.iter_mut().enumerate() {
                out[i] = inputs.get(o).map_or(0.0, |ch| ch[i]) * gain;
            }
        }
    }

    /// RMS and Expand detectors, lookahead, parallel mix, `Range` and `Soft Clip` (the Peak
    /// detector without them stays on the original, bit-identical path in [`Self::render`]).
    fn render_modes(
        &mut self,
        audio: &mut AudioBuffers<'_, '_>,
        sidechain: &[&[f32]],
        start: usize,
        end: usize,
    ) {
        let threshold = self.value(params::THRESHOLD);
        let ratio = self.value(params::RATIO);
        let knee = self.value(params::KNEE);
        let expand = self.detector == DETECTOR_EXPAND;
        let rms = self.detector == DETECTOR_RMS;
        let range = self.value(params::RANGE);
        let limit = range < RANGE_OFF_DB && !expand;
        let clip = self.value(params::SOFT_CLIP) >= 0.5;
        let inputs = audio.inputs;
        for i in start..end {
            let peak = self.detect(inputs, sidechain, i);
            self.meter_peak = self.meter_peak.max(peak);
            let target = if expand {
                let under = threshold - util::amp_to_db(peak);
                if under > 0.0 {
                    (under * (ratio - 1.0)).min(MAX_EXPAND_DB)
                } else {
                    0.0
                }
            } else if rms {
                // Mean square of the louder channel (the peak's square), one-pole averaged.
                self.mean_square = peak * peak + (self.mean_square - peak * peak) * self.rms_coef;
                if self.mean_square < 1e-24 {
                    self.mean_square = 0.0;
                }
                gain_reduction_knee_db(
                    util::amp_to_db(self.mean_square.sqrt()),
                    threshold,
                    ratio,
                    knee,
                )
            } else {
                gain_reduction_knee_db(util::amp_to_db(peak), threshold, ratio, knee)
            };
            let target = if limit { target.min(range) } else { target };
            if expand {
                // Expander: attack while it opens, release while it closes.
                let coef = if target < self.gr_db {
                    self.attack_coef
                } else {
                    self.release_coef
                };
                self.gr_db = target + (self.gr_db - target) * coef;
                if self.gr_db < 1e-6 {
                    self.gr_db = 0.0;
                }
                self.gr_held = self.gr_db;
            } else {
                ballistics(
                    &mut self.gr_db,
                    &mut self.gr_held,
                    target,
                    self.attack_coef,
                    self.release_coef,
                );
            }
            self.meter_gr = self.meter_gr.max(self.gr_db);
            let gain = util::db_to_amp(-self.gr_db) * self.makeup.tick();
            let parallel = self.mix.is_smoothing() || self.mix.current() < 1.0;
            let mix = self.mix.tick();
            for (o, out) in audio.outputs.iter_mut().enumerate() {
                let x = inputs.get(o).map_or(0.0, |ch| ch[i]);
                // The lookahead delays the audio (dry and wet alike); the detector doesn't.
                let x = match self.delay.get_mut(o) {
                    Some(line) => {
                        line.push(x);
                        if self.lookahead.active() {
                            self.lookahead.read(line)
                        } else {
                            x
                        }
                    }
                    None => x,
                };
                let y = if parallel {
                    x * (1.0 - mix) + x * gain * mix
                } else {
                    x * gain
                };
                out[i] = if clip { soft_clip(y) } else { y };
            }
            self.lookahead.advance();
        }
    }

    fn run(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        sidechain: &[&[f32]],
    ) -> ProcessStatus {
        let frames = ctx.frames;
        // At most `SIDECHAIN_CHANNELS` channels, each at least `frames` long (else ignored).
        let n = sidechain.len().min(SIDECHAIN_CHANNELS);
        let sidechain = if sidechain[..n].iter().all(|c| c.len() >= frames) {
            &sidechain[..n]
        } else {
            &[]
        };
        let lookahead = self.lookahead.len();
        util::LateEvents::split(
            self,
            |s| &mut s.late,
            ctx,
            |kind| match kind {
                EventKind::Param {
                    param: params::LOOKAHEAD,
                    ..
                } => util::Route::Now,
                _ => util::Route::Late(lookahead),
            },
            |s, a, b| s.render(audio, sidechain, a, b),
            |s, kind| {
                if let EventKind::Param { param, value } = *kind {
                    s.apply_param(param, value, true);
                }
            },
        );
        // No look-ahead: silent input is silent output; settled once released.
        // (Released to under 0.001 dB: the rest of the release is inaudible.)
        // The expander rests fully closed on silence; the RMS average must have decayed.
        let detector_settled = match self.detector {
            DETECTOR_EXPAND => self.gr_db >= MAX_EXPAND_DB - 1e-3,
            DETECTOR_RMS => self.gr_db < 1e-3 && self.gr_held < 1e-3 && self.mean_square < 1e-12,
            _ => self.gr_db < 1e-3 && self.gr_held < 1e-3,
        };
        let settled = detector_settled
            && !self.makeup.is_smoothing()
            && !self.mix.is_smoothing()
            && !self.slope.is_smoothing()
            && self.late.is_idle();
        let hold = self.lookahead.len() + util::samples(util::BASE_HOLD_SECONDS, self.sample_rate);
        let silent_sc = util::all_zero(sidechain, frames);
        self.tail.status_of(ctx, audio, settled && silent_sc, hold)
    }
}

impl Node for Compressor {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.allocate();
        self.late.prepare(self.delay[0].max_delay(), config);
        self.reset();
    }

    fn reset(&mut self) {
        util::LateEvents::flush(
            self,
            |s| &mut s.late,
            |s, kind| {
                if let EventKind::Param { param, value } = *kind {
                    s.apply_param(param, value, false);
                }
            },
        );
        self.gr_db = 0.0;
        self.gr_held = 0.0;
        self.mean_square = 0.0;
        self.hpf_state = [(0.0, 0.0); SIDECHAIN_CHANNELS];
        self.hpf2_state = [crate::dsp::svf::Svf::ZERO; SIDECHAIN_CHANNELS];
        self.slope.settle();
        self.delay.iter_mut().for_each(DelayLine::clear);
        self.meter_gr = 0.0;
        self.meter_peak = 0.0;
    }

    fn latency(&self) -> u32 {
        self.lookahead.len() as u32
    }

    fn has_analysis(&self) -> bool {
        true
    }

    fn analysis(&mut self, out: &mut AnalysisSink<'_>) {
        if let Some(f) = out.frame(AnalysisKind::Levels) {
            f.push(-self.meter_gr);
            f.push(util::amp_to_db(self.meter_peak).max(METER_FLOOR_DB));
        }
        self.meter_gr = 0.0;
        self.meter_peak = 0.0;
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        self.run(ctx, audio, &[])
    }

    fn sidechain_inputs(&self) -> u16 {
        SIDECHAIN_CHANNELS as u16
    }

    fn process_sidechain(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        sidechain: &[&[f32]],
    ) -> ProcessStatus {
        self.run(ctx, audio, sidechain)
    }
}

impl Device for Compressor {
    fn descriptor(&self) -> DeviceDescriptor {
        descriptor()
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.late
            .readback(id, self.values.get(id.0 as usize).copied())
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.late.cancel(id);
        self.apply_param(id, value, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gain_computer() {
        assert_eq!(gain_reduction_db(-40.0, -20.0, 4.0), 0.0);
        // 12 dB over at 4:1 -> 9 dB reduction.
        assert!((gain_reduction_db(-8.0, -20.0, 4.0) - 9.0).abs() < 1e-5);
        // Continuous at the knee edges.
        let t = -20.0;
        let lo = gain_reduction_db(t - KNEE_DB / 2.0, t, 4.0);
        let hi = gain_reduction_db(t + KNEE_DB / 2.0, t, 4.0);
        assert!(lo.abs() < 1e-6);
        assert!((hi - KNEE_DB / 2.0 * 0.75).abs() < 1e-5);
    }

    const SR: f32 = 48_000.0;

    /// A compressor at `SR` with the sidechain high-pass at `fc` and slope index `slope`.
    fn with_hpf(fc: f64, slope: f64) -> Compressor {
        let mut c = Compressor::new();
        c.prepare(&PrepareConfig {
            sample_rate: SR,
            max_block_size: 512,
            max_events_per_block: 64,
        });
        c.set_param(params::SIDECHAIN_HPF, fc);
        c.set_param(params::SIDECHAIN_SLOPE, slope);
        c
    }

    /// Steady-state gain (dB) of the sidechain high-pass for a unit sine at `freq`: the
    /// detector's peak over the last second of two.
    fn hpf_gain_db(c: &mut Compressor, freq: f32) -> f32 {
        let n = (2.0 * SR) as usize;
        let mut peak = 0.0f32;
        for i in 0..n {
            let x = (std::f32::consts::TAU * freq * i as f32 / SR).sin();
            let sc = [x, x];
            let d = c.detect(&[], &[&sc[..1], &sc[1..]], 0);
            if i >= n / 2 {
                peak = peak.max(d);
            }
        }
        util::amp_to_db(peak)
    }

    #[test]
    fn sidechain_hpf_12_db_per_octave_is_butterworth() {
        let fc = 100.0;
        let g = |f| hpf_gain_db(&mut with_hpf(fc, 1.0), f);
        // -3 dB at the cutoff, flat an octave+ above, then 12 dB/oct below.
        let at_fc = g(100.0);
        assert!((at_fc + 3.01).abs() < 0.1, "{at_fc}");
        assert!(g(1000.0).abs() < 0.05);
        let (half, quarter, eighth) = (g(50.0), g(25.0), g(12.5));
        assert!((half + 12.3).abs() < 0.2, "{half}");
        assert!((quarter + 24.1).abs() < 0.2, "{quarter}");
        assert!(
            ((quarter - eighth) - 12.0).abs() < 0.3,
            "{quarter} {eighth}"
        );
        // The default stays the one-pole: ~6 dB/oct.
        let g6 = |f| hpf_gain_db(&mut with_hpf(fc, 0.0), f);
        let (q6, e6) = (g6(25.0), g6(12.5));
        assert!(((q6 - e6) - 6.0).abs() < 0.5, "{q6} {e6}");
    }

    #[test]
    fn switching_the_sidechain_slope_does_not_click() {
        let mut c = with_hpf(200.0, 0.0);
        let freq = 60.0;
        let mut prev = 0.0f32;
        let mut max_step = 0.0f32;
        let n = SR as usize;
        for i in 0..n {
            if i == n / 2 + 37 {
                c.apply_param(params::SIDECHAIN_SLOPE, 1.0, true);
            }
            let x = 0.5 * (std::f32::consts::TAU * freq * i as f32 / SR).sin();
            let w = c.slope.tick();
            let y = c.high_pass(0, x, w);
            if i > SR as usize / 10 {
                max_step = max_step.max((y - prev).abs());
            }
            prev = y;
        }
        assert_eq!(c.slope.current(), 1.0);
        // A 60 Hz sine at <= 0.5 moves < 0.004 per sample; a hard switch would jump ~0.1.
        assert!(max_step < 0.006, "{max_step}");
    }
}

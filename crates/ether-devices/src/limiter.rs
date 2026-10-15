//! Built-in `Limiter` device (roadmap v2, owned by the `devices-2` node).
//!
//! Stereo-linked lookahead brickwall limiter. The audio is delayed by a fixed
//! [`LOOKAHEAD_MS`] lookahead, reported through [`Node::latency`] so the engine's PDC
//! aligns it. Per sample:
//! 1. required gain `r = min(1, ceiling / max(|L|, |R|))` of the (input-gained) signal,
//! 2. minimum of `r` over the lookahead window (so the gain is already down when the
//!    peak leaves the delay),
//! 3. release: the envelope drops instantly and rises with the release time constant,
//! 4. a moving average over the same window, which turns the step into a smooth attack
//!    ramp while staying `<=` the required gain of every sample it covers,
//! 5. the delayed audio times the gain, then a final safety clamp at the (delayed)
//!    ceiling against float rounding. The output never exceeds the ceiling.
//!
//! The detector is **true-peak** (#520): besides each sample it estimates the three
//! inter-sample points between it and the next (4x, a [`ISP_TAPS`]-tap Kaiser-windowed
//! sinc), so peaks that a DAC or an encoder reconstructs between samples stay under the
//! ceiling too (sample-peak detection let them through by up to 0.8 dB at +12 dB of drive).
//! The estimate needs [`ISP_DELAY`] samples of the future, added to the lookahead delay and
//! the reported latency. Sidechain (roadmap v2, `sidechain`): a
//! stereo sidechain input (`sidechain_inputs = 2`). When connected, the (input-gained)
//! sidechain peak replaces the detector signal of step 1 (`Limiter::step`'s `detect`
//! argument); the gain path and the final ceiling clamp stay the same, so the output still
//! never exceeds the ceiling.
//!
//! Analysis (`devices-pass-effects`, the window's meter and transfer-curve dot):
//! `AnalysisKind::Levels [gain reduction dB (<= 0), detector peak dBFS]`, the deepest
//! reduction and the loudest (input-gained) detector peak since the last read.
//!
//! # Parameter ids (stable, append-only)
//!
//! | id | name      | range                    |
//! |----|-----------|--------------------------|
//! | 0  | `Gain`    | -12 ..= 24 dB (input)    |
//! | 1  | `Ceiling` | -24 ..= 0 dB             |
//! | 2  | `Release` | 1 ..= 1000 ms (log)      |

use ether_core::protocol::devices::{
    DeviceCategory, DeviceDescriptor, DeviceTypeRef, ParamInfo, ParamScale, ParamUnit,
};
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus, Smoother,
};

use crate::dsp::delay_line::DelayLine;
use crate::dsp::sliding_min::SlidingMin;
use crate::util;
use ether_core::{AnalysisKind, AnalysisSink};

/// Parameter ids (stable: stored in documents and automation).
pub mod params {
    use ether_core::protocol::model::ParamId;
    pub const GAIN: ParamId = ParamId(0);
    pub const CEILING: ParamId = ParamId(1);
    pub const RELEASE: ParamId = ParamId(2);
}

const NUM_PARAMS: usize = 3;
/// Channels of the sidechain (detector) input.
const SIDECHAIN_CHANNELS: usize = 2;
/// Lookahead in milliseconds (the latency is this plus [`ISP_DELAY`] samples).
pub const LOOKAHEAD_MS: f32 = 5.0;
/// Taps of the true-peak interpolator.
const ISP_TAPS: usize = 16;
/// Samples the true-peak detector looks ahead of the sample it measures.
pub const ISP_DELAY: usize = ISP_TAPS / 2;
/// Detector level reported when nothing was measured (dBFS).
const METER_FLOOR_DB: f32 = -90.0;

/// Parameter list (identical for every instance).
pub fn param_infos() -> Vec<ParamInfo> {
    use util::param;
    vec![
        param(
            0,
            "Gain",
            "Limiter",
            ParamUnit::Decibels,
            (-12.0, 24.0, 0.0),
            ParamScale::Linear,
        ),
        param(
            1,
            "Ceiling",
            "Limiter",
            ParamUnit::Decibels,
            (-24.0, 0.0, -0.3),
            ParamScale::Linear,
        ),
        param(
            2,
            "Release",
            "Limiter",
            ParamUnit::Milliseconds,
            (1.0, 1000.0, 100.0),
            ParamScale::Log,
        ),
    ]
}

/// Descriptor of the `Limiter` type.
pub fn descriptor() -> DeviceDescriptor {
    DeviceDescriptor {
        layout: Some(layout()),
        device_type: DeviceTypeRef::Builtin {
            device: BuiltinDeviceType::Limiter,
        },
        name: "Limiter".to_owned(),
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
/// curve (input gain, ceiling; live detector dot) beside the gain-reduction meter, and the
/// release.
pub fn layout() -> ether_core::protocol::layout::DeviceLayout {
    use crate::contract::{item, knob, layout, section};
    use ether_core::protocol::layout::{LayoutItem, Widget, WidgetSize::*};
    layout(vec![
        section(
            "curve",
            Some("Limiter"),
            3,
            4,
            vec![
                LayoutItem {
                    colspan: 3,
                    ..item(
                        Widget::DynamicsCurve {
                            threshold: params::CEILING,
                            ratio: None,
                            knee: None,
                            mode: None,
                            floor: None,
                            input_gain: Some(params::GAIN),
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
            "release",
            Some("Release"),
            1,
            1,
            vec![knob(params::RELEASE, Medium)],
        ),
    ])
}

/// Non-RT. A new instance with default params.
pub fn create() -> Box<dyn Device> {
    Box::new(Limiter::new())
}

/// The true-peak interpolator: for `p` in 1/4, 1/2, 3/4, the weights of the history
/// (oldest first) for the point `p` after the sample [`ISP_DELAY`] back.
fn isp_coefficients() -> [[f32; ISP_TAPS]; 3] {
    let half = ISP_TAPS as f64 / 2.0;
    let bessel = |x: f64| {
        let (mut sum, mut term) = (1.0, 1.0);
        for k in 1..40 {
            term *= (x / 2.0) * (x / 2.0) / f64::from(k * k);
            sum += term;
        }
        sum
    };
    let beta = 6.0;
    let mut out = [[0.0f32; ISP_TAPS]; 3];
    for (j, row) in out.iter_mut().enumerate() {
        let p = (j + 1) as f64 / 4.0;
        let t = (ISP_DELAY - 1) as f64 + p;
        let mut sum = 0.0;
        for (k, c) in row.iter_mut().enumerate() {
            let d = t - k as f64;
            let sinc = (std::f64::consts::PI * d).sin() / (std::f64::consts::PI * d);
            let r = d / half;
            let w = bessel(beta * (1.0 - r * r).max(0.0).sqrt()) / bessel(beta);
            *c = (sinc * w) as f32;
            sum += sinc * w;
        }
        // Unity DC gain.
        row.iter_mut().for_each(|c| *c /= sum as f32);
    }
    out
}

/// Lookahead in samples at `sample_rate` (>= 1).
pub fn lookahead_samples(sample_rate: f32) -> u32 {
    ((LOOKAHEAD_MS * 0.001 * sample_rate).round() as u32).max(1)
}

/// The built-in limiter.
#[derive(Debug)]
pub struct Limiter {
    values: [f64; NUM_PARAMS],
    ranges: [(f64, f64); NUM_PARAMS],
    sample_rate: f32,
    /// Lookahead `L` in samples. Windows span `L + 1` samples; the audio is delayed by
    /// `L + ISP_DELAY` (the latency).
    lookahead: usize,
    /// Detector history per channel (oldest first) and the interpolator's three phases.
    isp: [[f32; ISP_TAPS]; 2],
    isp_coef: [[f32; ISP_TAPS]; 3],
    input_gain: Smoother,
    ceiling: Smoother,
    release_coef: f64,
    delay: [DelayLine; 2],
    /// The ceiling travels with the audio so the safety clamp matches the gain path.
    ceiling_delay: DelayLine,
    min: SlidingMin,
    env: f64,
    box_ring: Vec<f32>,
    box_pos: usize,
    box_sum: f64,
    /// Last applied gain (linear), for tests and the meter.
    gain: f32,
    /// Lowest gain (linear) and loudest detector peak since the last analysis read.
    meter_gain: f32,
    meter_peak: f32,
    /// Silence reporting (perf audit #5).
    tail: util::Tail,
}

impl Default for Limiter {
    fn default() -> Self {
        Self::new()
    }
}

impl Limiter {
    /// Non-RT. A limiter with default parameters (buffers sized for 48 kHz until
    /// `prepare`).
    pub fn new() -> Self {
        let infos = param_infos();
        let mut values = [0.0; NUM_PARAMS];
        let mut ranges = [(0.0, 0.0); NUM_PARAMS];
        for (i, info) in infos.iter().enumerate() {
            values[i] = info.default;
            ranges[i] = (info.min, info.max);
        }
        let mut l = Self {
            values,
            ranges,
            sample_rate: 48_000.0,
            lookahead: 1,
            isp: [[0.0; ISP_TAPS]; 2],
            isp_coef: isp_coefficients(),
            input_gain: Smoother::new(1.0, 20.0, 48_000.0),
            ceiling: Smoother::new(1.0, 20.0, 48_000.0),
            release_coef: 0.0,
            delay: [DelayLine::default(), DelayLine::default()],
            ceiling_delay: DelayLine::default(),
            min: SlidingMin::default(),
            env: 1.0,
            box_ring: Vec::new(),
            box_pos: 0,
            box_sum: 0.0,
            gain: 1.0,
            meter_gain: 1.0,
            meter_peak: 0.0,
            tail: util::Tail::default(),
        };
        l.allocate();
        l
    }

    fn value(&self, id: ParamId) -> f32 {
        self.values[id.0 as usize] as f32
    }

    fn allocate(&mut self) {
        self.lookahead = lookahead_samples(self.sample_rate) as usize;
        let window = self.lookahead + 1;
        let delay = window + ISP_DELAY;
        self.delay = [DelayLine::new(delay), DelayLine::new(delay)];
        self.ceiling_delay = DelayLine::new(delay);
        self.min = SlidingMin::new(window);
        self.box_ring = vec![1.0; window];
        self.input_gain = Smoother::new(
            util::db_to_amp(self.value(params::GAIN)),
            20.0,
            self.sample_rate,
        );
        self.ceiling = Smoother::new(
            util::db_to_amp(self.value(params::CEILING)),
            20.0,
            self.sample_rate,
        );
        self.release_coef = util::tau_coef(self.value(params::RELEASE), self.sample_rate) as f64;
        self.reset();
    }

    fn apply_param(&mut self, id: ParamId, value: f64, smooth: bool) {
        let Some(&(min, max)) = self.ranges.get(id.0 as usize) else {
            return;
        };
        self.values[id.0 as usize] = util::clamp(value, min, max);
        let smoother = match id {
            params::GAIN => &mut self.input_gain,
            params::CEILING => &mut self.ceiling,
            _ => {
                self.release_coef =
                    util::tau_coef(self.value(params::RELEASE), self.sample_rate) as f64;
                return;
            }
        };
        let amp = util::db_to_amp(self.values[id.0 as usize] as f32);
        if smooth {
            smoother.set_target(amp);
        } else {
            smoother.set_immediate(amp);
        }
    }

    /// Current gain reduction in dB (>= 0).
    pub fn gain_reduction(&self) -> f32 {
        -util::amp_to_db(self.gain).min(0.0)
    }

    /// One frame: `x` is the input-gained stereo frame, `detect` the detector level.
    /// Returns the output frame.
    #[inline]
    fn step(&mut self, x: [f32; 2], detect: f32) -> [f32; 2] {
        let ceiling = self.ceiling.tick();
        let required = if detect > ceiling {
            ceiling / detect
        } else {
            1.0
        };
        let m = self.min.push(required) as f64;
        self.env = if m < self.env || m - self.env < 1e-6 {
            // Attack is instant here (the moving average below shapes it); the release
            // snaps when settled so the gain returns to exactly `m` (unity).
            m
        } else {
            m - (m - self.env) * self.release_coef
        };
        // Moving average over the window (running sum, re-summed on every wrap so f64
        // rounding never accumulates).
        let old = self.box_ring[self.box_pos];
        self.box_ring[self.box_pos] = self.env as f32;
        self.box_sum += (self.env as f32) as f64 - old as f64;
        self.box_pos += 1;
        if self.box_pos == self.box_ring.len() {
            self.box_pos = 0;
            self.box_sum = self.box_ring.iter().map(|&v| v as f64).sum();
        }
        let gain = (self.box_sum / self.box_ring.len() as f64) as f32;
        self.gain = gain;
        self.meter_gain = self.meter_gain.min(gain);
        self.meter_peak = self.meter_peak.max(detect);

        self.ceiling_delay.push(ceiling);
        let tap = self.lookahead + ISP_DELAY + 1;
        let ceil_d = self.ceiling_delay.tap(tap);
        let mut out = [0.0; 2];
        for (c, line) in self.delay.iter_mut().enumerate() {
            line.push(x[c]);
            let y = line.tap(tap) * gain;
            out[c] = y.clamp(-ceil_d, ceil_d);
        }
        out
    }

    fn render(
        &mut self,
        audio: &mut AudioBuffers<'_, '_>,
        sidechain: &[&[f32]],
        start: usize,
        end: usize,
    ) {
        let inputs = audio.inputs;
        let input = |c: usize, i: usize| {
            inputs
                .get(c)
                .or_else(|| inputs.first())
                .map_or(0.0, |ch| ch[i])
        };
        for i in start..end {
            let g = self.input_gain.tick();
            let x = [input(0, i) * g, input(1, i) * g];
            let key = if sidechain.is_empty() {
                x
            } else {
                let first = sidechain[0][i];
                [first * g, sidechain.get(1).map_or(first, |ch| ch[i]) * g]
            };
            let detect = self.true_peak(key);
            let y = self.step(x, detect);
            for (c, out) in audio.outputs.iter_mut().enumerate() {
                out[i] = if c < 2 { y[c] } else { 0.0 };
            }
        }
    }
}

impl Limiter {
    /// RT. Push one detector frame; the true peak (sample and three inter-sample points) of
    /// the frame [`ISP_DELAY`] samples back, over both channels.
    #[inline]
    fn true_peak(&mut self, frame: [f32; 2]) -> f32 {
        let mut m = 0.0f32;
        for (h, &x) in self.isp.iter_mut().zip(&frame) {
            h.copy_within(1.., 0);
            h[ISP_TAPS - 1] = x;
            m = m.max(h[ISP_DELAY - 1].abs());
            for c in &self.isp_coef {
                let v: f32 = h.iter().zip(c).map(|(a, b)| a * b).sum();
                m = m.max(v.abs());
            }
        }
        m
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
        util::split_at_events(
            self,
            ctx.events,
            frames,
            |s, a, b| s.render(audio, sidechain, a, b),
            |s, kind| {
                if let EventKind::Param { param, value } = *kind {
                    s.apply_param(param, value, true);
                }
            },
        );
        // Silent input leaves the look-ahead line after `lookahead` samples; settled once
        // the gain released (stale reduction would duck the next note).
        let settled = (1.0 - self.env).abs() < 1e-6
            && (1.0 - self.gain).abs() < 1e-6
            && !self.input_gain.is_smoothing()
            && !self.ceiling.is_smoothing();
        let hold = self.lookahead
            + ISP_DELAY
            + 1
            + util::samples(util::BASE_HOLD_SECONDS, self.sample_rate);
        let silent_sc = util::all_zero(sidechain, frames);
        self.tail.status_of(ctx, audio, settled && silent_sc, hold)
    }
}

impl Node for Limiter {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.allocate();
    }

    fn reset(&mut self) {
        for line in &mut self.delay {
            line.clear();
        }
        // Delayed ceiling starts at the current ceiling (the audio there is silent).
        self.ceiling_delay.clear();
        let c = self.ceiling.current();
        for _ in 0..=self.lookahead + ISP_DELAY + 1 {
            self.ceiling_delay.push(c);
        }
        self.isp = [[0.0; ISP_TAPS]; 2];
        self.min.clear();
        self.env = 1.0;
        self.box_ring.fill(1.0);
        self.box_pos = 0;
        self.box_sum = self.box_ring.len() as f64;
        self.gain = 1.0;
        self.meter_gain = 1.0;
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

    fn latency(&self) -> u32 {
        (self.lookahead + ISP_DELAY) as u32
    }

    fn has_analysis(&self) -> bool {
        true
    }

    fn analysis(&mut self, out: &mut AnalysisSink<'_>) {
        if let Some(f) = out.frame(AnalysisKind::Levels) {
            f.push(util::amp_to_db(self.meter_gain).min(0.0));
            f.push(util::amp_to_db(self.meter_peak).max(METER_FLOOR_DB));
        }
        self.meter_gain = 1.0;
        self.meter_peak = 0.0;
    }
}

impl Device for Limiter {
    fn descriptor(&self) -> DeviceDescriptor {
        descriptor()
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.values.get(id.0 as usize).copied()
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.apply_param(id, value, false);
    }
}

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
//! Sample peaks only (no true-peak oversampling). Sidechain: the descriptor reports
//! `sidechain_inputs: 0`; a sidechain input would only replace the detector signal of
//! step 1 (`Limiter::step`'s `detect` argument), the gain path stays the same.
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

/// Parameter ids (stable: stored in documents and automation).
pub mod params {
    use ether_core::protocol::model::ParamId;
    pub const GAIN: ParamId = ParamId(0);
    pub const CEILING: ParamId = ParamId(1);
    pub const RELEASE: ParamId = ParamId(2);
}

const NUM_PARAMS: usize = 3;
/// Lookahead (= reported latency) in milliseconds.
pub const LOOKAHEAD_MS: f32 = 5.0;

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
        device_type: DeviceTypeRef::Builtin {
            device: BuiltinDeviceType::Limiter,
        },
        name: "Limiter".to_owned(),
        category: DeviceCategory::AudioEffect,
        params: param_infos(),
        audio_inputs: 2,
        audio_outputs: 2,
        midi_input: false,
        sidechain_inputs: 0,
    }
}

/// Non-RT. A new instance with default params.
pub fn create() -> Box<dyn Device> {
    Box::new(Limiter::new())
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
    /// Lookahead `L` in samples (the latency). Windows span `L + 1` samples.
    lookahead: usize,
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
    /// Last applied gain (linear), for tests and a future meter.
    gain: f32,
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
        self.delay = [DelayLine::new(window), DelayLine::new(window)];
        self.ceiling_delay = DelayLine::new(window);
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

        self.ceiling_delay.push(ceiling);
        let ceil_d = self.ceiling_delay.tap(self.lookahead + 1);
        let mut out = [0.0; 2];
        for (c, line) in self.delay.iter_mut().enumerate() {
            line.push(x[c]);
            let y = line.tap(self.lookahead + 1) * gain;
            out[c] = y.clamp(-ceil_d, ceil_d);
        }
        out
    }

    fn render(&mut self, audio: &mut AudioBuffers<'_, '_>, start: usize, end: usize) {
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
            let detect = x[0].abs().max(x[1].abs());
            let y = self.step(x, detect);
            for (c, out) in audio.outputs.iter_mut().enumerate() {
                out[i] = if c < 2 { y[c] } else { 0.0 };
            }
        }
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
        for _ in 0..=self.lookahead + 1 {
            self.ceiling_delay.push(c);
        }
        self.min.clear();
        self.env = 1.0;
        self.box_ring.fill(1.0);
        self.box_pos = 0;
        self.box_sum = self.box_ring.len() as f64;
        self.gain = 1.0;
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        util::split_at_events(
            self,
            ctx.events,
            ctx.frames,
            |s, a, b| s.render(audio, a, b),
            |s, kind| {
                if let EventKind::Param { param, value } = *kind {
                    s.apply_param(param, value, true);
                }
            },
        );
        ProcessStatus::Continue
    }

    fn latency(&self) -> u32 {
        self.lookahead as u32
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

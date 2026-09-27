//! Built-in `Reverb` device (roadmap v2, owned by the `devices-2` node).
//!
//! Algorithmic stereo reverb: stereo pre-delay → two series allpass diffusers per channel
//! → an 8-line feedback delay network (FDN) with a normalized Hadamard feedback matrix.
//! Each line has a one-pole low-pass (damping) and a gain `10^(-3·len/(fs·RT60))`, so
//! the tail decays by 60 dB in `Decay` seconds at low frequencies regardless of `Size`.
//! The matrix is orthogonal and every loop gain is < 1, so the network is stable at every
//! setting (20 s decay, zero damping included).
//!
//! Left input feeds the even lines and right the odd ones; the wet outputs are two
//! orthogonal sign patterns over all lines (decorrelated L/R), then mid/side `Width`.
//! `Size` scales the line lengths (read with fractional delays, gliding on change), the
//! other params are smoothed; buffers are allocated in `prepare`.
//!
//! # Parameter ids (stable, append-only)
//!
//! | id | name        | range                  |
//! |----|-------------|------------------------|
//! | 0  | `Pre-Delay` | 0 ..= 250 ms           |
//! | 1  | `Size`      | 0 ..= 100 %            |
//! | 2  | `Decay`     | 0.2 ..= 20 s (log)     |
//! | 3  | `Damping`   | 0 ..= 100 %            |
//! | 4  | `Width`     | 0 ..= 100 %            |
//! | 5  | `Mix`       | 0 ..= 100 %            |

use ether_core::protocol::devices::{
    DeviceCategory, DeviceDescriptor, DeviceTypeRef, ParamInfo, ParamScale, ParamUnit,
};
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus, Smoother,
};

use crate::dsp::delay_line::DelayLine;
use crate::dsp::flush32;
use crate::util;

/// Parameter ids (stable: stored in documents and automation).
pub mod params {
    use ether_core::protocol::model::ParamId;
    pub const PRE_DELAY: ParamId = ParamId(0);
    pub const SIZE: ParamId = ParamId(1);
    pub const DECAY: ParamId = ParamId(2);
    pub const DAMPING: ParamId = ParamId(3);
    pub const WIDTH: ParamId = ParamId(4);
    pub const MIX: ParamId = ParamId(5);
}

const NUM_PARAMS: usize = 6;
const LINES: usize = 8;
/// FDN line lengths at 48 kHz for a size factor of 1 (mutually prime).
const BASE_LENGTHS: [f32; LINES] = [
    1433.0, 1601.0, 1867.0, 2053.0, 2251.0, 2399.0, 2617.0, 2797.0,
];
/// Size factor range (`Size` 0 % .. 100 %).
const SIZE_MIN: f32 = 0.2;
const SIZE_MAX: f32 = 2.0;
/// Allpass diffuser lengths at 48 kHz (left, right) and feedback.
const DIFFUSERS: [[f32; 2]; 2] = [[155.0, 413.0], [163.0, 421.0]];
const DIFFUSION: f32 = 0.6;
const MAX_PRE_DELAY_MS: f32 = 250.0;
/// Glide time constant for size and pre-delay changes.
const GLIDE_MS: f32 = 80.0;
/// Coefficient update interval (loop gains) in samples.
const CONTROL_INTERVAL: usize = 32;
/// Wet output scale (roughly unity loudness for broadband input at mid settings).
const WET_GAIN: f32 = 0.35;

/// Parameter list (identical for every instance).
pub fn param_infos() -> Vec<ParamInfo> {
    use util::param;
    vec![
        param(
            0,
            "Pre-Delay",
            "Reverb",
            ParamUnit::Milliseconds,
            (0.0, MAX_PRE_DELAY_MS as f64, 20.0),
            ParamScale::Power { exponent: 2.0 },
        ),
        param(
            1,
            "Size",
            "Reverb",
            ParamUnit::Percent,
            (0.0, 100.0, 50.0),
            ParamScale::Linear,
        ),
        param(
            2,
            "Decay",
            "Reverb",
            ParamUnit::Seconds,
            (0.2, 20.0, 2.0),
            ParamScale::Log,
        ),
        param(
            3,
            "Damping",
            "Reverb",
            ParamUnit::Percent,
            (0.0, 100.0, 50.0),
            ParamScale::Linear,
        ),
        param(
            4,
            "Width",
            "Output",
            ParamUnit::Percent,
            (0.0, 100.0, 100.0),
            ParamScale::Linear,
        ),
        param(
            5,
            "Mix",
            "Output",
            ParamUnit::Percent,
            (0.0, 100.0, 30.0),
            ParamScale::Linear,
        ),
    ]
}

/// Descriptor of the `Reverb` type.
pub fn descriptor() -> DeviceDescriptor {
    DeviceDescriptor {
        device_type: DeviceTypeRef::Builtin {
            device: BuiltinDeviceType::Reverb,
        },
        name: "Reverb".to_owned(),
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
    Box::new(Reverb::new())
}

/// Schroeder allpass on a delay line.
#[derive(Clone, Debug, Default)]
struct Allpass {
    line: DelayLine,
    len: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Self {
            line: DelayLine::new(len),
            len,
        }
    }

    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let d = self.line.tap(self.len);
        let v = x - DIFFUSION * d;
        self.line.push(v);
        d + DIFFUSION * v
    }
}

/// In-place normalized 8-point Walsh-Hadamard transform (orthogonal).
#[inline]
fn hadamard8(x: &mut [f32; LINES]) {
    let mut h = 1;
    while h < LINES {
        let mut i = 0;
        while i < LINES {
            for j in i..i + h {
                let (a, b) = (x[j], x[j + h]);
                x[j] = a + b;
                x[j + h] = a - b;
            }
            i += 2 * h;
        }
        h *= 2;
    }
    let norm = 1.0 / (LINES as f32).sqrt();
    x.iter_mut().for_each(|v| *v *= norm);
}

/// Output sign patterns (two orthogonal Hadamard rows).
const OUT_L: [f32; LINES] = [1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0];
const OUT_R: [f32; LINES] = [1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, -1.0];

/// The built-in reverb.
#[derive(Debug)]
pub struct Reverb {
    values: [f64; NUM_PARAMS],
    ranges: [(f64, f64); NUM_PARAMS],
    sample_rate: f32,
    pre_delay: [DelayLine; 2],
    diffusers: [[Allpass; 2]; 2],
    lines: [DelayLine; LINES],
    /// Damping low-pass state per line.
    lp: [f32; LINES],
    /// Per-line loop gain (updated every `CONTROL_INTERVAL` samples).
    gains: [f32; LINES],
    control_countdown: usize,
    glide_coef: f32,
    /// Current (gliding) size factor and pre-delay in samples.
    size: f32,
    pre: f32,
    snap: bool,
    decay: Smoother,
    damping: Smoother,
    width: Smoother,
    mix: Smoother,
}

impl Default for Reverb {
    fn default() -> Self {
        Self::new()
    }
}

impl Reverb {
    /// Non-RT. A reverb with default parameters (buffers sized for 48 kHz until
    /// `prepare`).
    pub fn new() -> Self {
        let infos = param_infos();
        let mut values = [0.0; NUM_PARAMS];
        let mut ranges = [(0.0, 0.0); NUM_PARAMS];
        for (i, info) in infos.iter().enumerate() {
            values[i] = info.default;
            ranges[i] = (info.min, info.max);
        }
        let s = || Smoother::new(0.0, 20.0, 48_000.0);
        let mut r = Self {
            values,
            ranges,
            sample_rate: 48_000.0,
            pre_delay: Default::default(),
            diffusers: Default::default(),
            lines: Default::default(),
            lp: [0.0; LINES],
            gains: [0.0; LINES],
            control_countdown: 0,
            glide_coef: 0.0,
            size: 1.0,
            pre: 0.0,
            snap: true,
            decay: s(),
            damping: s(),
            width: s(),
            mix: s(),
        };
        r.allocate();
        r
    }

    fn value(&self, id: ParamId) -> f32 {
        self.values[id.0 as usize] as f32
    }

    fn scale(&self) -> f32 {
        self.sample_rate / 48_000.0
    }

    fn allocate(&mut self) {
        let scale = self.scale();
        let pre_len = (MAX_PRE_DELAY_MS * 0.001 * self.sample_rate).ceil() as usize + 2;
        self.pre_delay = [DelayLine::new(pre_len), DelayLine::new(pre_len)];
        self.diffusers = DIFFUSERS
            .map(|pair| pair.map(|len| Allpass::new(((len * scale).round() as usize).max(1))));
        self.lines =
            BASE_LENGTHS.map(|len| DelayLine::new((len * SIZE_MAX * scale).ceil() as usize + 2));
        self.glide_coef = util::tau_coef(GLIDE_MS, self.sample_rate);
        let sr = self.sample_rate;
        let mk = |v: f32| Smoother::new(v, 20.0, sr);
        self.decay = mk(self.value(params::DECAY));
        self.damping = mk(self.damping_coef());
        self.width = mk(self.value(params::WIDTH) * 0.01);
        self.mix = mk(self.value(params::MIX) * 0.01);
        self.reset();
    }

    /// One-pole low-pass coefficient for the `Damping` param: 0 % ≈ 20 kHz, 100 % ≈ 1 kHz.
    fn damping_coef(&self) -> f32 {
        let d = self.value(params::DAMPING) * 0.01;
        let fc = (20_000.0 * 0.05f32.powf(d)).min(self.sample_rate * 0.45);
        (-std::f32::consts::TAU * fc / self.sample_rate).exp()
    }

    fn target_size(&self) -> f32 {
        SIZE_MIN + (SIZE_MAX - SIZE_MIN) * self.value(params::SIZE) * 0.01
    }

    fn target_pre(&self) -> f32 {
        self.value(params::PRE_DELAY) * 0.001 * self.sample_rate
    }

    fn apply_param(&mut self, id: ParamId, value: f64, smooth: bool) {
        let Some(&(min, max)) = self.ranges.get(id.0 as usize) else {
            return;
        };
        self.values[id.0 as usize] = util::clamp(value, min, max);
        let v = match id {
            params::DECAY => self.value(params::DECAY),
            params::DAMPING => self.damping_coef(),
            params::WIDTH => self.value(params::WIDTH) * 0.01,
            params::MIX => self.value(params::MIX) * 0.01,
            _ => {
                // Size / pre-delay glide in `render`.
                if !smooth {
                    self.snap = true;
                }
                return;
            }
        };
        let smoother = match id {
            params::DECAY => &mut self.decay,
            params::DAMPING => &mut self.damping,
            params::WIDTH => &mut self.width,
            _ => &mut self.mix,
        };
        if smooth {
            smoother.set_target(v);
        } else {
            smoother.set_immediate(v);
        }
    }

    fn update_gains(&mut self) {
        let rt60 = self.decay.current().max(0.01);
        let scale = self.scale() * self.size;
        for (g, base) in self.gains.iter_mut().zip(BASE_LENGTHS) {
            let seconds = base * scale / self.sample_rate;
            *g = 10f32.powf(-3.0 * seconds / rt60);
        }
    }

    fn render(&mut self, audio: &mut AudioBuffers<'_, '_>, start: usize, end: usize) {
        let (size_t, pre_t) = (self.target_size(), self.target_pre());
        if self.snap {
            self.size = size_t;
            self.pre = pre_t;
            self.snap = false;
            self.control_countdown = 0;
        }
        let inputs = audio.inputs;
        let input = |c: usize, i: usize| {
            inputs
                .get(c)
                .or_else(|| inputs.first())
                .map_or(0.0, |ch| ch[i])
        };
        let scale = self.scale();
        for i in start..end {
            self.size = size_t + (self.size - size_t) * self.glide_coef;
            self.pre = pre_t + (self.pre - pre_t) * self.glide_coef;
            self.decay.tick();
            if self.control_countdown == 0 {
                self.update_gains();
                self.control_countdown = CONTROL_INTERVAL;
            }
            self.control_countdown -= 1;
            let damp = self.damping.tick();

            let dry = [input(0, i), input(1, i)];
            // Pre-delay (0 = straight through) and diffusion.
            let mut feed = [0.0f32; 2];
            for c in 0..2 {
                let line = &mut self.pre_delay[c];
                line.push(dry[c]);
                let x = if self.pre < 1.0 {
                    dry[c]
                } else {
                    line.read(self.pre + 1.0)
                };
                let [a, b] = &mut self.diffusers[c];
                feed[c] = b.tick(a.tick(x));
            }

            // FDN.
            let mut o = [0.0f32; LINES];
            let mut wet = [0.0f32; 2];
            for (k, line) in self.lines.iter().enumerate() {
                let y = line.read(BASE_LENGTHS[k] * scale * self.size);
                o[k] = y;
                wet[0] += y * OUT_L[k];
                wet[1] += y * OUT_R[k];
            }
            for k in 0..LINES {
                self.lp[k] = flush32(o[k] * (1.0 - damp) + self.lp[k] * damp);
                o[k] = self.lp[k] * self.gains[k];
            }
            hadamard8(&mut o);
            for (k, line) in self.lines.iter_mut().enumerate() {
                line.push(o[k] + feed[k & 1]);
            }

            let width = self.width.tick();
            let mix = self.mix.tick();
            let (wl, wr) = (wet[0] * WET_GAIN, wet[1] * WET_GAIN);
            let mid = (wl + wr) * 0.5;
            let side = (wl - wr) * 0.5 * width;
            let out = [
                dry[0] * (1.0 - mix) + (mid + side) * mix,
                dry[1] * (1.0 - mix) + (mid - side) * mix,
            ];
            for (c, ch) in audio.outputs.iter_mut().enumerate() {
                ch[i] = if c < 2 { out[c] } else { 0.0 };
            }
        }
    }
}

impl Node for Reverb {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.allocate();
    }

    fn reset(&mut self) {
        self.pre_delay.iter_mut().for_each(DelayLine::clear);
        for pair in &mut self.diffusers {
            pair.iter_mut().for_each(|a| a.line.clear());
        }
        self.lines.iter_mut().for_each(DelayLine::clear);
        self.lp = [0.0; LINES];
        self.snap = true;
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
}

impl Device for Reverb {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hadamard_is_orthogonal() {
        let mut x = [1.0, 2.0, -3.0, 0.5, 0.0, 4.0, -1.0, 2.5];
        let e0: f32 = x.iter().map(|v| v * v).sum();
        hadamard8(&mut x);
        let e1: f32 = x.iter().map(|v| v * v).sum();
        assert!((e0 - e1).abs() < 1e-4);
        hadamard8(&mut x);
        assert!((x[2] + 3.0).abs() < 1e-5, "involution");
    }
}

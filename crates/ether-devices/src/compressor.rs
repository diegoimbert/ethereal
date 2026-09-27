//! Compressor: threshold, ratio, attack, release, makeup gain, sidechain HPF.
//!
//! Feed-forward, stereo-linked peak detector with a fixed 6 dB soft knee. The gain
//! reduction (in dB) is smoothed with separate attack/release time constants.
//!
//! Sidechain (roadmap v2, `sidechain`): the device has a stereo sidechain input
//! (`sidechain_inputs = 2`). When the chain entry has a sidechain source, the engine calls
//! [`Node::process_sidechain`] with the (latency-aligned) source signal and the detector
//! listens to it instead of the main input; the gain is still applied to the main input.
//! The detector signal of the sidechain goes through a one-pole high-pass
//! (`Sidechain HPF`, off at its 20 Hz minimum) so a kick's sub doesn't dominate.

use ether_core::protocol::devices::{
    DeviceCategory, DeviceDescriptor, DeviceTypeRef, ParamInfo, ParamScale, ParamUnit,
};
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus, Smoother,
};

use crate::util;

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
}

const NUM_PARAMS: usize = 6;
/// `Sidechain HPF` at or below this cutoff (Hz) is off.
const HPF_OFF_HZ: f32 = 20.0;
/// Soft-knee width in dB.
const KNEE_DB: f32 = 6.0;

/// Parameter list (identical for every instance).
pub fn param_infos() -> Vec<ParamInfo> {
    use util::param;
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
    ]
}

/// Descriptor of the compressor type.
pub fn descriptor() -> DeviceDescriptor {
    DeviceDescriptor {
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
    }
}

/// Channels of the sidechain input.
const SIDECHAIN_CHANNELS: usize = 2;

/// Static gain computer: gain reduction in dB (>= 0) for an input level in dB.
#[inline]
pub(crate) fn gain_reduction_db(level_db: f32, threshold: f32, ratio: f32) -> f32 {
    let over = level_db - threshold;
    let slope = 1.0 - 1.0 / ratio;
    if 2.0 * over <= -KNEE_DB {
        0.0
    } else if 2.0 * over >= KNEE_DB {
        over * slope
    } else {
        let x = over + KNEE_DB * 0.5;
        slope * x * x / (2.0 * KNEE_DB)
    }
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
    /// One-pole high-pass coefficient of the sidechain detector (`None` = off).
    hpf_coef: Option<f32>,
    /// High-pass state per sidechain channel: (previous input, previous output).
    hpf_state: [(f32, f32); SIDECHAIN_CHANNELS],
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
            hpf_coef: None,
            hpf_state: [(0.0, 0.0); SIDECHAIN_CHANNELS],
        };
        c.sync_all();
        c
    }

    fn value(&self, id: ParamId) -> f32 {
        self.values[id.0 as usize] as f32
    }

    fn sync_all(&mut self) {
        self.makeup = Smoother::new(
            util::db_to_amp(self.value(params::MAKEUP)),
            20.0,
            self.sample_rate,
        );
        self.update_times();
        self.update_hpf();
    }

    fn update_hpf(&mut self) {
        let fc = self.value(params::SIDECHAIN_HPF);
        self.hpf_coef = (fc > HPF_OFF_HZ).then(|| {
            // One-pole high-pass: y[n] = a * (y[n-1] + x[n] - x[n-1]).
            let rc = 1.0 / (2.0 * std::f32::consts::PI * fc);
            let dt = 1.0 / self.sample_rate;
            rc / (rc + dt)
        });
    }

    fn update_times(&mut self) {
        self.attack_coef = util::tau_coef(self.value(params::ATTACK), self.sample_rate);
        self.release_coef = util::tau_coef(self.value(params::RELEASE), self.sample_rate);
    }

    fn apply_param(&mut self, id: ParamId, value: f64, smooth: bool) {
        let Some(&(min, max)) = self.ranges.get(id.0 as usize) else {
            return;
        };
        self.values[id.0 as usize] = util::clamp(value, min, max);
        match id {
            params::MAKEUP => {
                let amp = util::db_to_amp(self.value(params::MAKEUP));
                if smooth {
                    self.makeup.set_target(amp);
                } else {
                    self.makeup.set_immediate(amp);
                }
            }
            params::ATTACK | params::RELEASE => self.update_times(),
            params::SIDECHAIN_HPF => self.update_hpf(),
            _ => {}
        }
    }

    /// Current gain reduction in dB (>= 0), e.g. for tests or a future meter.
    pub fn gain_reduction(&self) -> f32 {
        self.gr_db
    }

    /// Detector level (linear peak) of frame `i`: the sidechain if connected (high-passed),
    /// else the main input.
    #[inline]
    fn detect(&mut self, inputs: &[&[f32]], sidechain: &[&[f32]], i: usize) -> f32 {
        if sidechain.is_empty() {
            return inputs.iter().fold(0.0f32, |m, ch| m.max(ch[i].abs()));
        }
        let mut peak = 0.0f32;
        for (ch, st) in sidechain.iter().zip(self.hpf_state.iter_mut()) {
            let x = ch[i];
            let y = match self.hpf_coef {
                Some(a) => {
                    let y = a * (st.1 + x - st.0);
                    // Keep the state out of the subnormal range (no FTZ on the audio
                    // thread).
                    *st = (x, if y.abs() < 1e-20 { 0.0 } else { y });
                    y
                }
                None => x,
            };
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
        let inputs = audio.inputs;
        for i in start..end {
            let peak = self.detect(inputs, sidechain, i);
            let target = gain_reduction_db(util::amp_to_db(peak), threshold, ratio);
            let coef = if target > self.gr_db {
                self.attack_coef
            } else {
                self.release_coef
            };
            self.gr_db = target + (self.gr_db - target) * coef;
            // Snap the decaying tail to 0 so it never goes subnormal (no FTZ on the
            // audio thread).
            if self.gr_db < 1e-6 {
                self.gr_db = 0.0;
            }
            let gain = util::db_to_amp(-self.gr_db) * self.makeup.tick();
            for (o, out) in audio.outputs.iter_mut().enumerate() {
                out[i] = inputs.get(o).map_or(0.0, |ch| ch[i]) * gain;
            }
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
        ProcessStatus::Continue
    }
}

impl Node for Compressor {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.sync_all();
        self.reset();
    }

    fn reset(&mut self) {
        self.gr_db = 0.0;
        self.hpf_state = [(0.0, 0.0); SIDECHAIN_CHANNELS];
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
}

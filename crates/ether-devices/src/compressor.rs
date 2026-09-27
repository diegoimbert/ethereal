//! Compressor: threshold, ratio, attack, release, makeup gain.
//!
//! Feed-forward, stereo-linked peak detector with a fixed 6 dB soft knee. The gain
//! reduction (in dB) is smoothed with separate attack/release time constants.

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
}

const NUM_PARAMS: usize = 5;
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
        sidechain_inputs: 0,
    }
}

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
            _ => {}
        }
    }

    /// Current gain reduction in dB (>= 0), e.g. for tests or a future meter.
    pub fn gain_reduction(&self) -> f32 {
        self.gr_db
    }

    fn render(&mut self, audio: &mut AudioBuffers<'_, '_>, start: usize, end: usize) {
        let threshold = self.value(params::THRESHOLD);
        let ratio = self.value(params::RATIO);
        let inputs = audio.inputs;
        for i in start..end {
            let peak = inputs.iter().fold(0.0f32, |m, ch| m.max(ch[i].abs()));
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
}

impl Node for Compressor {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.sync_all();
        self.reset();
    }

    fn reset(&mut self) {
        self.gr_db = 0.0;
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

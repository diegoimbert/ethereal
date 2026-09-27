//! Built-in `Utility` device (roadmap v2, owned by the `devices-2` node).
//!
//! Gain, balance pan, stereo width, phase invert (L/R) and mono. Per sample:
//! 1. phase invert per channel,
//! 2. mid/side width: `M = (L+R)/2`, `S = (L-R)/2 · width` (`Mono` forces width 0),
//! 3. balance pan: panning right attenuates the left channel linearly (and vice versa),
//!    centre is unity,
//! 4. gain.
//!
//! Every stage is smoothed (≈20 ms), toggles included, so switching never clicks.
//!
//! # Parameter ids (stable, append-only)
//!
//! | id | name      | range                    |
//! |----|-----------|--------------------------|
//! | 0  | `Gain`    | -36 ..= 36 dB            |
//! | 1  | `Pan`     | -1 ..= 1                 |
//! | 2  | `Width`   | 0 ..= 200 %              |
//! | 3  | `Invert L`| toggle                   |
//! | 4  | `Invert R`| toggle                   |
//! | 5  | `Mono`    | toggle                   |

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
    pub const GAIN: ParamId = ParamId(0);
    pub const PAN: ParamId = ParamId(1);
    pub const WIDTH: ParamId = ParamId(2);
    pub const INVERT_L: ParamId = ParamId(3);
    pub const INVERT_R: ParamId = ParamId(4);
    pub const MONO: ParamId = ParamId(5);
}

const NUM_PARAMS: usize = 6;
const SMOOTH_MS: f32 = 20.0;

/// Parameter list (identical for every instance).
pub fn param_infos() -> Vec<ParamInfo> {
    use util::{choice, param};
    vec![
        param(
            0,
            "Gain",
            "Utility",
            ParamUnit::Decibels,
            (-36.0, 36.0, 0.0),
            ParamScale::Linear,
        ),
        param(
            1,
            "Pan",
            "Utility",
            ParamUnit::Pan,
            (-1.0, 1.0, 0.0),
            ParamScale::Linear,
        ),
        param(
            2,
            "Width",
            "Stereo",
            ParamUnit::Percent,
            (0.0, 200.0, 100.0),
            ParamScale::Linear,
        ),
        choice(3, "Invert L", "Phase", ParamUnit::Toggle, &["Off", "On"], 0),
        choice(4, "Invert R", "Phase", ParamUnit::Toggle, &["Off", "On"], 0),
        choice(5, "Mono", "Stereo", ParamUnit::Toggle, &["Off", "On"], 0),
    ]
}

/// Descriptor of the `Utility` type.
pub fn descriptor() -> DeviceDescriptor {
    DeviceDescriptor {
        device_type: DeviceTypeRef::Builtin {
            device: BuiltinDeviceType::Utility,
        },
        name: "Utility".to_owned(),
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
    Box::new(Utility::new())
}

/// Smoothed per-sample targets.
#[derive(Debug)]
struct Smoothers {
    gain: Smoother,
    /// Left/right balance gains.
    bal: [Smoother; 2],
    width: Smoother,
    /// Polarity factors (+1 / -1).
    sign: [Smoother; 2],
}

/// The built-in utility.
#[derive(Debug)]
pub struct Utility {
    values: [f64; NUM_PARAMS],
    ranges: [(f64, f64); NUM_PARAMS],
    sample_rate: f32,
    s: Smoothers,
}

impl Default for Utility {
    fn default() -> Self {
        Self::new()
    }
}

impl Utility {
    /// Non-RT. A utility with default parameters.
    pub fn new() -> Self {
        let infos = param_infos();
        let mut values = [0.0; NUM_PARAMS];
        let mut ranges = [(0.0, 0.0); NUM_PARAMS];
        for (i, info) in infos.iter().enumerate() {
            values[i] = info.default;
            ranges[i] = (info.min, info.max);
        }
        let mk = |v| Smoother::new(v, SMOOTH_MS, 48_000.0);
        let mut u = Self {
            values,
            ranges,
            sample_rate: 48_000.0,
            s: Smoothers {
                gain: mk(1.0),
                bal: [mk(1.0), mk(1.0)],
                width: mk(1.0),
                sign: [mk(1.0), mk(1.0)],
            },
        };
        u.sync_all();
        u
    }

    fn value(&self, id: ParamId) -> f64 {
        self.values[id.0 as usize]
    }

    /// Plain targets: gain, balance L/R, effective width, signs L/R.
    fn targets(&self) -> [f32; 6] {
        let pan = self.value(params::PAN) as f32;
        let width = if self.value(params::MONO) >= 0.5 {
            0.0
        } else {
            self.value(params::WIDTH) as f32 * 0.01
        };
        let sign = |id| if self.value(id) >= 0.5 { -1.0 } else { 1.0 };
        [
            util::db_to_amp(self.value(params::GAIN) as f32),
            1.0 - pan.max(0.0),
            1.0 + pan.min(0.0),
            width,
            sign(params::INVERT_L),
            sign(params::INVERT_R),
        ]
    }

    fn smoothers(&mut self) -> [&mut Smoother; 6] {
        let s = &mut self.s;
        let [bl, br] = &mut s.bal;
        let [sl, sr] = &mut s.sign;
        [&mut s.gain, bl, br, &mut s.width, sl, sr]
    }

    fn sync_all(&mut self) {
        let targets = self.targets();
        let sr = self.sample_rate;
        for (s, t) in self.smoothers().into_iter().zip(targets) {
            *s = Smoother::new(t, SMOOTH_MS, sr);
        }
    }

    fn apply_param(&mut self, id: ParamId, value: f64, smooth: bool) {
        let Some(&(min, max)) = self.ranges.get(id.0 as usize) else {
            return;
        };
        self.values[id.0 as usize] = util::clamp(value, min, max);
        let targets = self.targets();
        for (s, t) in self.smoothers().into_iter().zip(targets) {
            if !smooth {
                s.set_immediate(t);
            } else if s.target() != t {
                s.set_target(t);
            }
        }
    }

    fn render(&mut self, audio: &mut AudioBuffers<'_, '_>, start: usize, end: usize) {
        let inputs = audio.inputs;
        let input = |c: usize, i: usize| inputs.get(c).map_or(0.0, |ch| ch[i]);
        let mono_in = inputs.len() == 1;
        for i in start..end {
            let l = input(0, i) * self.s.sign[0].tick();
            let r = if mono_in { input(0, i) } else { input(1, i) } * self.s.sign[1].tick();
            let mid = (l + r) * 0.5;
            let side = (l - r) * 0.5 * self.s.width.tick();
            let gain = self.s.gain.tick();
            let out_l = (mid + side) * self.s.bal[0].tick() * gain;
            let out_r = (mid - side) * self.s.bal[1].tick() * gain;
            match audio.outputs {
                [ol, or, rest @ ..] => {
                    ol[i] = out_l;
                    or[i] = out_r;
                    rest.iter_mut().for_each(|o| o[i] = 0.0);
                }
                [o] => o[i] = (out_l + out_r) * 0.5,
                [] => {}
            }
        }
    }
}

impl Node for Utility {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.sync_all();
    }

    fn reset(&mut self) {}

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

impl Device for Utility {
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

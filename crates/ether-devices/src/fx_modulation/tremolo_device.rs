//! Tremolo / Auto-Pan: an LFO (sine, triangle, square, saw up, saw down; free or
//! tempo-synced) drives the gain or the balance.
//!
//! - **Tremolo**: gain `1 - Depth × (1 - u)` with `u = (lfo + 1) / 2`, so 100 % depth dips
//!   to silence. The right channel's LFO leads by `Stereo Phase` degrees (180° = the classic
//!   stereo "ping-pong" tremolo).
//! - **Auto-Pan**: balance `Depth × lfo` (`-1` = left). The louder side stays at unity and
//!   the other follows an equal-power curve (no level boost); `Stereo Phase` is ignored.
//!
//! The gain signal is smoothed by a 1 ms one-pole so square and saw edges don't click.

use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus,
};

use super::shared::{Glide, Lfo, LfoStep, Params, Ramp, Rate, Shape, input};
use super::tremolo as p;
use crate::util::{self, db_to_amp};

const N: usize = p::COUNT;
/// Edge smoothing of the gain signal (ms).
const EDGE_MS: f32 = 1.0;

pub(super) struct Tremolo {
    params: Params<N>,
    sample_rate: f32,
    lfo: Lfo,
    depth: Ramp,
    stereo: Glide,
    gain: Ramp,
    /// Smoothed per-channel modulation gains.
    smoothed: [f32; 2],
    edge_coef: f32,
}

impl Tremolo {
    pub(super) fn new(desc: &DeviceDescriptor) -> Self {
        let mut s = Self {
            params: Params::<N>::new(desc),
            sample_rate: 48_000.0,
            lfo: Lfo::default(),
            depth: Ramp::new(0.0),
            stereo: Glide::new(0.0),
            gain: Ramp::new(1.0),
            smoothed: [1.0; 2],
            edge_coef: 0.0,
        };
        s.init();
        s
    }

    fn init(&mut self) {
        self.stereo.set_time(15.0, self.sample_rate);
        self.edge_coef = util::tau_coef(EDGE_MS, self.sample_rate);
        for id in 0..N as u32 {
            self.sync_param(ParamId(id), false);
        }
    }

    fn sync_param(&mut self, id: ParamId, smooth: bool) {
        let v = self.params.get(id);
        match id {
            p::DEPTH => self.depth.set(v * 0.01, smooth),
            p::STEREO_PHASE => self.stereo.set(v / 360.0, smooth),
            p::OUTPUT => self.gain.set(db_to_amp(v), smooth),
            _ => {}
        }
    }

    fn apply(&mut self, id: ParamId, value: f64, smooth: bool) {
        if self.params.set(id, value) {
            self.sync_param(id, smooth);
        }
    }

    /// Modulation gains `[left, right]` for the LFO phase `phase`.
    #[inline]
    fn gains(auto_pan: bool, shape: Shape, phase: f64, depth: f32, stereo: f32) -> [f32; 2] {
        if auto_pan {
            let pan = (depth * shape.at(phase)).clamp(-1.0, 1.0);
            let theta = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
            let s2 = std::f32::consts::SQRT_2;
            [(s2 * theta.cos()).min(1.0), (s2 * theta.sin()).min(1.0)]
        } else {
            let g = |ph: f64| 1.0 - depth * (1.0 - (shape.at(ph) + 1.0) * 0.5);
            [g(phase), g(phase + f64::from(stereo))]
        }
    }

    fn render(
        &mut self,
        ctx: &ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        start: usize,
        end: usize,
    ) {
        let rate = Rate::of(
            self.params.on(p::SYNC),
            self.params.get(p::RATE),
            self.params
                .index(p::SYNC_RATE, crate::contract::SYNC_RATES.len()),
        );
        let step = LfoStep::new(rate, ctx.transport, self.sample_rate);
        let auto_pan = self.params.index(p::MODE, 2) == 1;
        let shape = Shape::ALL[self.params.index(p::SHAPE, Shape::ALL.len())];
        let channels = audio.outputs.len().min(2);
        let inputs = audio.inputs;
        let k = self.edge_coef;
        for i in start..end {
            self.lfo.advance(&step, i);
            let depth = self.depth.tick();
            let stereo = self.stereo.tick();
            let gain = self.gain.tick();
            let g = Self::gains(auto_pan, shape, self.lfo.phase(), depth, stereo);
            for ch in 0..channels {
                let sm = &mut self.smoothed[ch];
                *sm = crate::dsp::flush32(g[ch] + (*sm - g[ch]) * k);
                audio.outputs[ch][i] = input(inputs, ch, i) * *sm * gain;
            }
            for out in audio.outputs.iter_mut().skip(channels) {
                out[i] = 0.0;
            }
        }
    }
}

impl Node for Tremolo {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.init();
        self.reset();
    }

    fn reset(&mut self) {
        let auto_pan = self.params.index(p::MODE, 2) == 1;
        let shape = Shape::ALL[self.params.index(p::SHAPE, Shape::ALL.len())];
        self.smoothed = Self::gains(
            auto_pan,
            shape,
            self.lfo.phase(),
            self.params.get(p::DEPTH) * 0.01,
            self.params.get(p::STEREO_PHASE) / 360.0,
        );
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let c: &ProcessContext<'_> = ctx;
        util::split_at_events(
            self,
            c.events,
            c.frames,
            |s, a, b| s.render(c, audio, a, b),
            |s, kind| {
                if let EventKind::Param { param, value } = *kind {
                    s.apply(param, value, true);
                }
            },
        );
        ProcessStatus::Continue
    }
}

impl Device for Tremolo {
    fn descriptor(&self) -> DeviceDescriptor {
        super::descriptor(BuiltinDeviceType::Tremolo)
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.params.plain(id)
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.apply(id, value, false);
    }
}

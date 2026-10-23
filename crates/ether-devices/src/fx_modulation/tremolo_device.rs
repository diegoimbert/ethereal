//! Tremolo / Auto-Pan: an LFO (sine, triangle, square, saw up, saw down; free or
//! tempo-synced) drives the gain or the balance.
//!
//! - **Tremolo**: gain `1 - Depth × (1 - u)` with `u = (lfo + 1) / 2`, so 100 % depth dips
//!   to silence. The right channel's LFO leads by `Stereo Phase` degrees (180° = the classic
//!   stereo "ping-pong" tremolo).
//! - **Auto-Pan**: balance `Depth × lfo` (`-1` = left). The louder side stays at unity and
//!   the other follows an equal-power curve (no level boost); `Stereo Phase` is ignored.
//!
//! The waveform starts `Phase` degrees in (against the song position when synced), is
//! pushed to its limits by `Hardness` (`tanh(k·x) / tanh(k)`) and flipped by `Invert` (Live's
//! Auto Pan Offset, Shape and Invert). The gain signal is smoothed by a 1 ms one-pole so
//! square and saw edges don't click (and so a jump of `Phase` doesn't either).

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
/// `Hardness` 100 % as the waveshaper's `k` (`tanh(k·x) / tanh(k)`: a sine ends up close to
/// a square with rounded edges).
const HARDNESS_K: f32 = 8.0;

/// The LFO waveform with `Phase`, `Hardness` and `Invert` applied.
#[derive(Clone, Copy)]
struct Wave {
    shape: Shape,
    /// Start offset, cycles.
    offset: f64,
    /// Waveshaper `k` (0 = off).
    hardness: f32,
    sign: f32,
}

impl Wave {
    #[inline]
    fn at(self, phase: f64) -> f32 {
        let x = self.shape.at(phase + self.offset);
        let x = if self.hardness > 1e-3 {
            (self.hardness * x).tanh() / self.hardness.tanh()
        } else {
            x
        };
        self.sign * x
    }
}

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
    /// Silence reporting.
    tail: crate::util::Tail,
}

impl Tremolo {
    pub(super) fn new(desc: &DeviceDescriptor) -> Self {
        let mut s = Self {
            tail: crate::util::Tail::default(),
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
    fn gains(auto_pan: bool, wave: Wave, phase: f64, depth: f32, stereo: f32) -> [f32; 2] {
        if auto_pan {
            let pan = (depth * wave.at(phase)).clamp(-1.0, 1.0);
            let theta = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
            let s2 = std::f32::consts::SQRT_2;
            [(s2 * theta.cos()).min(1.0), (s2 * theta.sin()).min(1.0)]
        } else {
            let g = |ph: f64| 1.0 - depth * (1.0 - (wave.at(ph) + 1.0) * 0.5);
            [g(phase), g(phase + f64::from(stereo))]
        }
    }

    /// The waveform as the params shape it.
    fn wave(&self) -> Wave {
        Wave {
            shape: Shape::ALL[self.params.index(p::SHAPE, Shape::ALL.len())],
            offset: f64::from(self.params.get(p::PHASE)) / 360.0,
            hardness: self.params.get(p::HARDNESS) * 0.01 * HARDNESS_K,
            sign: if self.params.on(p::INVERT) { -1.0 } else { 1.0 },
        }
    }

    fn rate(&self) -> Rate {
        Rate::of(
            self.params.on(p::SYNC),
            self.params.get(p::RATE),
            self.params
                .index(p::SYNC_RATE, crate::contract::SYNC_RATES.len()),
        )
    }

    fn render(
        &mut self,
        ctx: &ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        start: usize,
        end: usize,
    ) {
        let step = LfoStep::new(self.rate(), ctx.transport, self.sample_rate);
        let auto_pan = self.params.index(p::MODE, 2) == 1;
        let wave = self.wave();
        let channels = audio.outputs.len().min(2);
        let inputs = audio.inputs;
        let k = self.edge_coef;
        for i in start..end {
            self.lfo.advance(&step, i);
            let depth = self.depth.tick();
            let stereo = self.stereo.tick();
            let gain = self.gain.tick();
            let g = Self::gains(auto_pan, wave, self.lfo.phase(), depth, stereo);
            for (ch, &target) in g.iter().enumerate().take(channels) {
                let sm = &mut self.smoothed[ch];
                *sm = crate::dsp::flush32(target + (*sm - target) * k);
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
        self.smoothed = Self::gains(
            auto_pan,
            self.wave(),
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
        // Asleep for a while: the LFO goes on as if it had run, and the
        // edge smoothing starts from its gains there (silence came through either way).
        let skipped = self.tail.skipped(c);
        if skipped > 0 {
            let step = LfoStep::new(self.rate(), c.transport, self.sample_rate);
            self.lfo.skip(&step, skipped);
            self.reset();
        }
        util::split_at_events(
            self,
            c.events,
            c.frames,
            |s, a, b| s.render(c, audio, a, b),
            |s, kind| {
                if let EventKind::Param { param, value, step } = *kind {
                    s.apply(param, value, !step);
                }
            },
        );
        // Memoryless on the audio (input x gain): silent in, silent out.
        let settled =
            !self.stereo.is_moving() && !self.depth.is_smoothing() && !self.gain.is_smoothing();
        self.tail.status_of(c, audio, settled, 1)
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

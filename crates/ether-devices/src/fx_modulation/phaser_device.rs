//! Phaser: 2-12 first-order allpass stages per channel whose break frequency sweeps
//! `Center × 2^(±2.5 oct × Depth × lfo)` (sine LFO, free or tempo-synced), with feedback
//! from the last stage to the input. Mixed with the dry signal the allpass phase shift makes
//! `Stages / 2` moving notches (deepest at `Mix` 50 %). The right channel's LFO leads by
//! `Stereo Phase` degrees. Coefficients are recomputed every sample (no zipper noise).

use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus,
};

use super::phaser as p;
use super::shared::{
    CONTROL, CtlRamp, Glide, Lfo, LfoStep, Params, Ramp, Rate, Shape, input, lfo_at, soft_clip,
};
use crate::util::{self, db_to_amp};

const N: usize = p::COUNT;
/// Stage counts of the `Stages` choice.
const STAGES: [usize; 5] = [2, 4, 6, 8, 12];
const MAX_STAGES: usize = 12;
/// Sweep half-range at 100 % depth, in octaves.
const SWEEP_OCTAVES: f32 = 2.5;
/// Lowest / highest break frequency (Hz; the top is also capped below Nyquist).
const MIN_HZ: f32 = 20.0;
const MAX_HZ: f32 = 20_000.0;

pub(super) struct Phaser {
    params: Params<N>,
    sample_rate: f32,
    lfo: Lfo,
    /// Allpass states (`z1`) per channel and stage.
    state: [[f32; MAX_STAGES]; 2],
    /// Last stage output of the previous sample, per channel (feedback).
    last: [f32; 2],
    center: Glide,
    depth: Glide,
    stereo: Glide,
    feedback: Ramp,
    mix: Ramp,
    gain: Ramp,
    /// Allpass coefficient per channel at control rate and the samples
    /// left in the current chunk.
    coef: [CtlRamp; 2],
    ctl_left: usize,
    /// Silence reporting.
    tail: crate::util::Tail,
}

impl Phaser {
    pub(super) fn new(desc: &DeviceDescriptor) -> Self {
        let mut s = Self {
            tail: crate::util::Tail::default(),
            coef: [CtlRamp::default(); 2],
            ctl_left: 0,
            params: Params::<N>::new(desc),
            sample_rate: 48_000.0,
            lfo: Lfo::default(),
            state: [[0.0; MAX_STAGES]; 2],
            last: [0.0; 2],
            center: Glide::new(0.0),
            depth: Glide::new(0.0),
            stereo: Glide::new(0.0),
            feedback: Ramp::new(0.0),
            mix: Ramp::new(0.0),
            gain: Ramp::new(1.0),
        };
        s.init();
        s
    }

    fn init(&mut self) {
        for g in [&mut self.center, &mut self.depth, &mut self.stereo] {
            g.set_time(15.0, self.sample_rate);
        }
        for id in 0..N as u32 {
            self.sync_param(ParamId(id), false);
        }
    }

    fn sync_param(&mut self, id: ParamId, smooth: bool) {
        let v = self.params.get(id);
        match id {
            p::DEPTH => self.depth.set(v * 0.01, smooth),
            p::CENTER => self.center.set(v.max(1.0).log2(), smooth),
            p::STEREO_PHASE => self.stereo.set(v / 360.0, smooth),
            p::FEEDBACK => self.feedback.set(v * 0.01, smooth),
            p::MIX => self.mix.set(v * 0.01, smooth),
            p::OUTPUT => self.gain.set(db_to_amp(v), smooth),
            _ => {}
        }
    }

    fn apply(&mut self, id: ParamId, value: f64, smooth: bool) {
        if self.params.set(id, value) {
            self.sync_param(id, smooth);
        }
    }

    /// After skipped blocks (asleep): catch the LFO up and restart the control ramps.
    fn wake(&mut self, ctx: &ProcessContext<'_>) {
        let skipped = self.tail.skipped(ctx);
        if skipped > 0 {
            let rate = Rate::of(
                self.params.on(p::SYNC),
                self.params.get(p::RATE),
                self.params
                    .index(p::SYNC_RATE, crate::contract::SYNC_RATES.len()),
            );
            self.lfo.skip(
                &LfoStep::new(rate, ctx.transport, self.sample_rate),
                skipped,
            );
            self.coef.iter_mut().for_each(CtlRamp::unprime);
            self.ctl_left = 0;
        }
    }

    fn render(
        &mut self,
        ctx: &ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        start: usize,
        end: usize,
    ) {
        let sr = self.sample_rate;
        let rate = Rate::of(
            self.params.on(p::SYNC),
            self.params.get(p::RATE),
            self.params
                .index(p::SYNC_RATE, crate::contract::SYNC_RATES.len()),
        );
        let step = LfoStep::new(rate, ctx.transport, sr);
        let stages = STAGES[self.params.index(p::STAGES, STAGES.len())];
        let shape = Shape::ALL[self.params.index(p::LFO_SHAPE, Shape::ALL.len())];
        let top = MAX_HZ.min(sr * 0.45);
        let pi_sr = std::f32::consts::PI / sr;
        let channels = audio.outputs.len().min(2);
        let inputs = audio.inputs;
        for i in start..end {
            self.lfo.advance(&step, i);
            let center = self.center.tick();
            let sweep = self.depth.tick() * SWEEP_OCTAVES;
            let stereo = self.stereo.tick();
            let fb = self.feedback.tick();
            let mix = self.mix.tick();
            let gain = self.gain.tick();
            // The sweep's sin/exp2/tan once per control chunk.
            if self.ctl_left == 0 {
                self.ctl_left = CONTROL;
                let coef_at = |phase: f64, ch: usize| {
                    let ph = phase + if ch == 1 { f64::from(stereo) } else { 0.0 };
                    let m = lfo_at(shape, ph);
                    let hz = (center + sweep * m).exp2().clamp(MIN_HZ, top);
                    let t = (pi_sr * hz).tan();
                    (t - 1.0) / (t + 1.0)
                };
                let (now, end) = (self.lfo.phase(), self.lfo.ahead(&step, CONTROL));
                for ch in 0..channels {
                    self.coef[ch].segment(coef_at(end, ch), || coef_at(now, ch));
                }
            }
            self.ctl_left -= 1;
            for ch in 0..channels {
                let a = self.coef[ch].tick();
                let dry = input(inputs, ch, i);
                let mut x = dry + fb * soft_clip(self.last[ch]);
                for z in &mut self.state[ch][..stages] {
                    let y = a * x + *z;
                    *z = crate::dsp::flush32(x - a * y);
                    x = y;
                }
                self.last[ch] = crate::dsp::flush32(x);
                audio.outputs[ch][i] = (dry * (1.0 - mix) + x * mix) * gain;
            }
            for out in audio.outputs.iter_mut().skip(channels) {
                out[i] = 0.0;
            }
        }
    }
}

impl Node for Phaser {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.init();
        self.reset();
    }

    fn reset(&mut self) {
        self.state = [[0.0; MAX_STAGES]; 2];
        self.last = [0.0; 2];
        self.coef.iter_mut().for_each(CtlRamp::unprime);
        self.ctl_left = 0;
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let c: &ProcessContext<'_> = ctx;
        self.wake(c);
        util::split_at_events(
            self,
            c.events,
            c.frames,
            |s, a, b| s.render(c, audio, a, b),
            |s, kind| {
                if let EventKind::Param { param, value, step } = *kind {
                    s.apply(param, value, !step);
                    // A fresh control chunk from the event on. Only events (at fixed
                    // song frames) restart the chunk grid, never block boundaries, so
                    // renders stay block-size independent (CONTRACTS §12.7).
                    s.ctl_left = 0;
                }
            },
        );
        // The allpass chain and its feedback ring out under the floor.
        let settled = ![&self.center, &self.depth, &self.stereo]
            .iter()
            .any(|g| g.is_moving())
            && ![&self.feedback, &self.mix, &self.gain]
                .iter()
                .any(|r| r.is_smoothing());
        let hold = util::samples(util::BASE_HOLD_SECONDS, self.sample_rate);
        self.tail.status_of(ctx, audio, settled, hold)
    }
}

impl Device for Phaser {
    fn descriptor(&self) -> DeviceDescriptor {
        super::descriptor(BuiltinDeviceType::Phaser)
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.params.plain(id)
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.apply(id, value, false);
    }
}

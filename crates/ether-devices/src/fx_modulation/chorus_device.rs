//! Chorus / Ensemble / Vibrato: 1-4 modulated delay voices per channel.
//!
//! - **Chorus**: `Voices` taps per channel, LFO phases spread evenly over the cycle and base
//!   delays staggered (+0..30 %), summed at equal power. `Spread` offsets the right channel's
//!   LFOs by up to a quarter cycle (0 % = mono-compatible, 100 % = widest).
//! - **Ensemble**: the string-machine chorus: three taps 120° apart, each driven by a slow
//!   LFO (`Rate`) plus a faster, shallower one (`Rate` × 7.3); `Voices` is ignored.
//! - **Vibrato**: one tap, wet only (pitch modulation; `Mix` and `Feedback` are ignored).
//!
//! The read position is `Delay + Depth × min(0.8 × Delay, 4 ms) × lfo`, interpolated with a
//! cubic Hermite read. `Feedback` feeds the voices' average back into the line (stable below
//! 100 %), `High Cut` is a one-pole low-pass on the wet signal.

use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus,
};

use super::chorus as p;
use super::shared::{
    DELAY_SLEW, Glide, Lfo, LfoStep, ModLine, OnePoleLp, Params, Ramp, Rate, input, ms, soft_clip,
};
use crate::util::{self, db_to_amp};

const N: usize = p::COUNT;
/// Longest base delay (ms) and modulation excursion (ms), for the line size.
const MAX_DELAY_MS: f32 = 40.0;
const MAX_EXCURSION_MS: f32 = 4.0;
/// Most voices per channel.
const MAX_VOICES: usize = 4;
/// Ensemble: fast LFO rate ratio and depth share.
const ENSEMBLE_FAST_RATIO: f64 = 7.3;
const ENSEMBLE_FAST_SHARE: f32 = 0.25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Chorus,
    Ensemble,
    Vibrato,
}

pub(super) struct Chorus {
    params: Params<N>,
    sample_rate: f32,
    lines: [ModLine; 2],
    lp: [OnePoleLp; 2],
    /// Slow and (Ensemble) fast LFO.
    lfo: Lfo,
    fast: Lfo,
    delay_ms: Glide,
    depth: Glide,
    spread: Glide,
    cutoff: Glide,
    feedback: Ramp,
    mix: Ramp,
    gain: Ramp,
    /// Wet feedback signal of the previous sample, per channel.
    fb: [f32; 2],
}

impl Chorus {
    pub(super) fn new(desc: &DeviceDescriptor) -> Self {
        let params = Params::<N>::new(desc);
        let mut c = Self {
            sample_rate: 48_000.0,
            lines: [ModLine::default(), ModLine::default()],
            lp: [OnePoleLp::default(); 2],
            lfo: Lfo::default(),
            fast: Lfo::default(),
            delay_ms: Glide::new(0.0),
            depth: Glide::new(0.0),
            spread: Glide::new(0.0),
            cutoff: Glide::new(0.0),
            feedback: Ramp::new(0.0),
            mix: Ramp::new(0.0),
            gain: Ramp::new(1.0),
            fb: [0.0; 2],
            params,
        };
        c.allocate();
        c
    }

    fn allocate(&mut self) {
        let max = ms(MAX_DELAY_MS * 1.3 + MAX_EXCURSION_MS, self.sample_rate).ceil() as usize + 4;
        self.lines = [ModLine::new(max), ModLine::new(max)];
        for g in [&mut self.delay_ms, &mut self.depth, &mut self.spread] {
            g.set_time(25.0, self.sample_rate);
        }
        self.cutoff.set_time(10.0, self.sample_rate);
        // `delay_ms` is in ms: DELAY_SLEW samples per sample.
        self.delay_ms
            .set_slew(DELAY_SLEW * 1000.0 / self.sample_rate);
        for id in 0..N as u32 {
            self.sync_param(ParamId(id), false);
        }
    }

    fn mode(&self) -> Mode {
        match self.params.index(p::MODE, 3) {
            0 => Mode::Chorus,
            1 => Mode::Ensemble,
            _ => Mode::Vibrato,
        }
    }

    fn sync_param(&mut self, id: ParamId, smooth: bool) {
        let v = self.params.get(id);
        match id {
            p::DEPTH => self.depth.set(v * 0.01, smooth),
            p::DELAY => self.delay_ms.set(v, smooth),
            p::SPREAD => self.spread.set(v * 0.01, smooth),
            p::FEEDBACK => self.feedback.set(v * 0.01, smooth),
            // Glide the cutoff in octaves (log).
            p::HIGH_CUT => self.cutoff.set(v.max(1.0).log2(), smooth),
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

    fn render(
        &mut self,
        ctx: &ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        start: usize,
        end: usize,
    ) {
        let sr = self.sample_rate;
        let mode = self.mode();
        let rate = f64::from(self.params.get(p::RATE));
        let step = LfoStep::new(Rate::Hz(rate), ctx.transport, sr);
        let fast_step = LfoStep::new(Rate::Hz(rate * ENSEMBLE_FAST_RATIO), ctx.transport, sr);
        let voices = match mode {
            Mode::Chorus => self.params.index(p::VOICES, 5).clamp(1, MAX_VOICES),
            Mode::Ensemble => 3,
            Mode::Vibrato => 1,
        };
        let inv_v = 1.0 / voices as f32;
        let norm = 1.0 / (voices as f32).sqrt();
        let channels = audio.outputs.len().min(2);
        let inputs = audio.inputs;
        for i in start..end {
            self.lfo.advance(&step, i);
            self.fast.advance(&fast_step, i);
            let base_ms = self.delay_ms.tick();
            let depth = self.depth.tick();
            let spread = self.spread.tick();
            let a = OnePoleLp::coef(self.cutoff.tick().exp2(), sr);
            let fbk = if mode == Mode::Vibrato {
                0.0
            } else {
                self.feedback.tick()
            };
            let mix = self.mix.tick();
            let gain = self.gain.tick();
            let base = ms(base_ms, sr);
            let excursion = depth * ms((0.8 * base_ms).min(MAX_EXCURSION_MS), sr);
            for ch in 0..channels {
                let dry = input(inputs, ch, i);
                self.lines[ch].push(dry + fbk * soft_clip(self.fb[ch]));
                // Right channel LFOs lag by up to a quarter cycle.
                let offset = if ch == 1 {
                    0.25 * f64::from(spread)
                } else {
                    0.0
                };
                let mut sum = 0.0;
                for v in 0..voices {
                    let ph = self.lfo.phase() + v as f64 * f64::from(inv_v) + offset;
                    let m = match mode {
                        Mode::Ensemble => {
                            let fast = self.fast.phase() + v as f64 / 3.0 + offset;
                            (1.0 - ENSEMBLE_FAST_SHARE) * sin(ph) + ENSEMBLE_FAST_SHARE * sin(fast)
                        }
                        _ => sin(ph),
                    };
                    let stagger = 1.0 + 0.3 * v as f32 * inv_v;
                    sum += self.lines[ch].read(base * stagger + excursion * m);
                }
                let wet = self.lp[ch].tick(sum * norm, a);
                // Average of the voices (`wet · norm` = sum / voices): loop gain < 1.
                self.fb[ch] = crate::dsp::flush32(wet * norm);
                let out = if mode == Mode::Vibrato {
                    wet
                } else {
                    dry * (1.0 - mix) + wet * mix
                };
                audio.outputs[ch][i] = out * gain;
            }
            for out in audio.outputs.iter_mut().skip(channels) {
                out[i] = 0.0;
            }
        }
    }
}

#[inline]
fn sin(phase: f64) -> f32 {
    (std::f64::consts::TAU * phase).sin() as f32
}

impl Node for Chorus {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.allocate();
        self.reset();
    }

    fn reset(&mut self) {
        for l in &mut self.lines {
            l.clear();
        }
        for f in &mut self.lp {
            f.reset();
        }
        self.fb = [0.0; 2];
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

impl Device for Chorus {
    fn descriptor(&self) -> DeviceDescriptor {
        super::descriptor(BuiltinDeviceType::Chorus)
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.params.plain(id)
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.apply(id, value, false);
    }
}

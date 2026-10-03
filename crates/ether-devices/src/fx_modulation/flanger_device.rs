//! Flanger: one short modulated delay per channel with feedback (positive or negative),
//! mixed with the dry signal.
//!
//! - **Normal**: the delay sweeps `Delay × (1 ± Depth × lfo)` (down to one sample).
//! - **Through Zero** (appended param 9): the dry path is delayed by a fixed
//!   [`TZ_MS`] (reported as latency, so PDC keeps the track aligned) and the wet delay sweeps
//!   `TZ_MS ± Depth × min(Delay, TZ_MS) × lfo` around it: the two paths cross (zero relative
//!   delay), the tape-flanging "through zero" cancellation.
//!
//! The sine LFO is free or tempo-synced; the right channel's leads by `Stereo Phase`
//! degrees. Reads use cubic Hermite interpolation, and the base delay / depth glide.

use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus,
};

use super::flanger as p;
use super::shared::{
    DELAY_SLEW, Glide, Lfo, LfoStep, ModLine, Params, Ramp, Rate, input, ms, soft_clip,
};
use crate::util::{self, db_to_amp};

const N: usize = p::COUNT;
const MAX_DELAY_MS: f32 = 20.0;
/// Fixed dry delay (and latency) in through-zero mode.
pub const TZ_MS: f32 = 10.0;
/// Crossfade time when through-zero is switched (ms).
const TZ_FADE_MS: f32 = 10.0;

pub(super) struct Flanger {
    params: Params<N>,
    sample_rate: f32,
    lfo: Lfo,
    /// Wet lines (input + feedback) and dry lines (input only, through-zero reference).
    wet: [ModLine; 2],
    dry: [ModLine; 2],
    last: [f32; 2],
    delay_ms: Glide,
    depth: Glide,
    stereo: Glide,
    /// 0 = normal, 1 = through zero (crossfaded when toggled).
    tz: Glide,
    feedback: Ramp,
    mix: Ramp,
    gain: Ramp,
    /// Whole samples of [`TZ_MS`].
    tz_samples: usize,
}

impl Flanger {
    pub(super) fn new(desc: &DeviceDescriptor) -> Self {
        let mut s = Self {
            params: Params::<N>::new(desc),
            sample_rate: 48_000.0,
            lfo: Lfo::default(),
            wet: [ModLine::default(), ModLine::default()],
            dry: [ModLine::default(), ModLine::default()],
            last: [0.0; 2],
            delay_ms: Glide::new(0.0),
            depth: Glide::new(0.0),
            stereo: Glide::new(0.0),
            tz: Glide::new(0.0),
            feedback: Ramp::new(0.0),
            mix: Ramp::new(0.0),
            gain: Ramp::new(1.0),
            tz_samples: 0,
        };
        s.allocate();
        s
    }

    fn allocate(&mut self) {
        let sr = self.sample_rate;
        let max = ms(MAX_DELAY_MS * 2.0 + TZ_MS, sr).ceil() as usize + 4;
        self.wet = [ModLine::new(max), ModLine::new(max)];
        self.tz_samples = ms(TZ_MS, sr).round() as usize;
        self.dry = [
            ModLine::new(self.tz_samples + 1),
            ModLine::new(self.tz_samples + 1),
        ];
        for g in [&mut self.delay_ms, &mut self.depth, &mut self.stereo] {
            g.set_time(20.0, sr);
        }
        self.tz.set_time(TZ_FADE_MS / 3.0, sr);
        self.delay_ms.set_slew(DELAY_SLEW * 1000.0 / sr);
        for id in 0..N as u32 {
            self.sync_param(ParamId(id), false);
        }
    }

    fn sync_param(&mut self, id: ParamId, smooth: bool) {
        let v = self.params.get(id);
        match id {
            p::DEPTH => self.depth.set(v * 0.01, smooth),
            p::DELAY => self.delay_ms.set(v, smooth),
            p::STEREO_PHASE => self.stereo.set(v / 360.0, smooth),
            p::FEEDBACK => self.feedback.set(v * 0.01, smooth),
            p::MIX => self.mix.set(v * 0.01, smooth),
            p::OUTPUT => self.gain.set(db_to_amp(v), smooth),
            p::THROUGH_ZERO => self.tz.set(if v >= 0.5 { 1.0 } else { 0.0 }, smooth),
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
        let rate = Rate::of(
            self.params.on(p::SYNC),
            self.params.get(p::RATE),
            self.params
                .index(p::SYNC_RATE, crate::contract::SYNC_RATES.len()),
        );
        let step = LfoStep::new(rate, ctx.transport, sr);
        let tz_len = self.tz_samples as f32;
        let channels = audio.outputs.len().min(2);
        let inputs = audio.inputs;
        for i in start..end {
            self.lfo.advance(&step, i);
            let base_ms = self.delay_ms.tick();
            let depth = self.depth.tick();
            let stereo = self.stereo.tick();
            let tz = self.tz.tick();
            let fb = self.feedback.tick();
            let mix = self.mix.tick();
            let gain = self.gain.tick();
            let base = ms(base_ms, sr);
            let tz_swing = depth * ms(base_ms.min(TZ_MS), sr);
            for ch in 0..channels {
                let ph = self.lfo.phase() + if ch == 1 { f64::from(stereo) } else { 0.0 };
                let m = (std::f64::consts::TAU * ph).sin() as f32;
                let x = input(inputs, ch, i);
                self.wet[ch].push(x + fb * soft_clip(self.last[ch]));
                self.dry[ch].push(x);
                let (wet, dry) = if tz <= 0.0 {
                    (self.wet[ch].read(base * (1.0 + depth * m)), x)
                } else {
                    let tz_wet = self.wet[ch].read(tz_len + tz_swing * m);
                    let tz_dry = self.dry[ch].tap(self.tz_samples);
                    if tz >= 1.0 {
                        (tz_wet, tz_dry)
                    } else {
                        let n_wet = self.wet[ch].read(base * (1.0 + depth * m));
                        (n_wet + (tz_wet - n_wet) * tz, x + (tz_dry - x) * tz)
                    }
                };
                self.last[ch] = crate::dsp::flush32(wet);
                audio.outputs[ch][i] = (dry * (1.0 - mix) + wet * mix) * gain;
            }
            for out in audio.outputs.iter_mut().skip(channels) {
                out[i] = 0.0;
            }
        }
    }
}

impl Node for Flanger {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.allocate();
        self.reset();
    }

    fn reset(&mut self) {
        for l in self.wet.iter_mut().chain(self.dry.iter_mut()) {
            l.clear();
        }
        self.last = [0.0; 2];
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

    /// Through-zero delays the dry path by [`TZ_MS`].
    fn latency(&self) -> u32 {
        if self.params.on(p::THROUGH_ZERO) {
            self.tz_samples as u32
        } else {
            0
        }
    }
}

impl Device for Flanger {
    fn descriptor(&self) -> DeviceDescriptor {
        super::descriptor(BuiltinDeviceType::Flanger)
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.params.plain(id)
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.apply(id, value, false);
    }
}

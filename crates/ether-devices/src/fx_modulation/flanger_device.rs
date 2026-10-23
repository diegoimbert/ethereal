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
    CONTROL, CtlRamp, DELAY_SLEW, Glide, Lfo, LfoStep, ModLine, Params, Ramp, Rate, Shape, input,
    lfo_at, ms, soft_clip,
};
use crate::util::{self, LateEvents, Route, db_to_amp};

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
    /// LFO value per channel at control rate, samples left in the chunk.
    lfo_m: [CtlRamp; 2],
    ctl_left: usize,
    /// Wet signal above the floor in this block.
    loud: bool,
    /// Silence reporting.
    tail: crate::util::Tail,
    /// Through zero, the output is the dry/wet pair read [`TZ_MS`] into the lines: every
    /// param but the switch itself waits that long (else heard 10 ms early).
    late: LateEvents,
}

impl Flanger {
    pub(super) fn new(desc: &DeviceDescriptor) -> Self {
        let mut s = Self {
            tail: crate::util::Tail::default(),
            late: LateEvents::new(),
            lfo_m: [CtlRamp::default(); 2],
            ctl_left: 0,
            loud: false,
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
        let sr = self.sample_rate;
        let step = LfoStep::new(self.rate(), ctx.transport, sr);
        let shape = Shape::ALL[self.params.index(p::LFO_SHAPE, Shape::ALL.len())];
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
            // The LFO's sin once per control chunk.
            if self.ctl_left == 0 {
                self.ctl_left = CONTROL;
                let m_at = |phase: f64, ch: usize| {
                    let ph = phase + if ch == 1 { f64::from(stereo) } else { 0.0 };
                    lfo_at(shape, ph)
                };
                let (now, end) = (self.lfo.phase(), self.lfo.ahead(&step, CONTROL));
                for ch in 0..channels {
                    self.lfo_m[ch].segment(m_at(end, ch), || m_at(now, ch));
                }
            }
            self.ctl_left -= 1;
            for ch in 0..channels {
                let m = self.lfo_m[ch].tick();
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
                self.loud |= wet.abs() >= util::SILENCE_FLOOR;
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
        self.late.prepare(self.tz_samples, config);
        self.reset();
    }

    fn reset(&mut self) {
        LateEvents::flush(
            self,
            |s| &mut s.late,
            |s, kind| {
                if let EventKind::Param { param, value, .. } = *kind {
                    s.apply(param, value, false);
                }
            },
        );
        for l in self.wet.iter_mut().chain(self.dry.iter_mut()) {
            l.clear();
        }
        self.last = [0.0; 2];
        self.lfo_m.iter_mut().for_each(CtlRamp::unprime);
        self.ctl_left = 0;
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let c: &ProcessContext<'_> = ctx;
        let skipped = self.tail.skipped(c);
        if skipped > 0 {
            let step = LfoStep::new(self.rate(), c.transport, self.sample_rate);
            self.lfo.skip(&step, skipped);
            self.lfo_m.iter_mut().for_each(CtlRamp::unprime);
            self.ctl_left = 0;
        }
        self.loud = false;
        let latency = self.latency() as usize;
        LateEvents::split(
            self,
            |s| &mut s.late,
            c,
            |kind| match kind {
                EventKind::Param {
                    param: p::THROUGH_ZERO,
                    ..
                } => Route::Now,
                _ => Route::Late(latency),
            },
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
        // Silent once the wet line (with its feedback) stayed under the floor for longer
        // than it is long.
        let n = c.frames;
        let silent_in = util::all_zero(audio.inputs, n);
        let quiet = silent_in
            && !self.loud
            && util::below_floor(audio.outputs.iter().map(|o| &o[..n.min(o.len())]));
        let settled = ![&self.delay_ms, &self.depth, &self.stereo, &self.tz]
            .iter()
            .any(|g| g.is_moving())
            && ![&self.feedback, &self.mix, &self.gain]
                .iter()
                .any(|r| r.is_smoothing())
            && self.late.is_idle();
        let hold = self.wet[0].max_delay() as usize
            + util::samples(util::BASE_HOLD_SECONDS, self.sample_rate);
        self.tail
            .status(!c.events.is_empty(), silent_in, quiet, settled, n, hold)
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
        self.late.readback(id, self.params.plain(id))
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.late.cancel(id);
        self.apply(id, value, false);
    }
}

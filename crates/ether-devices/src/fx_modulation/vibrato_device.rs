//! Vibrato: the pitch wobbles up and down with an LFO (vibrato, tape wow and flutter).
//!
//! A modulated delay line: reading a line at a moving delay `d(t)` plays it at speed
//! `1 - d'(t)`, so the pitch follows the delay's slope. The `Shape` is the **pitch** contour
//! (what is heard): the read delay is its integral, `d = C - A · W(φ)` with `W` the cycle
//! integral of the contour `m(φ)` (zero mean, so the delay stays bounded) and
//! `A = k · sr / f`: the speed is `1 + k · m(φ)` whatever the rate `f`. `k = sinh(c · ln2 /
//! 1200)` for a depth of `c` cents puts the up and down peaks at `+c · (1 - ε)` and
//! `-c · (1 + ε)` cents (their mean is `c` within 0.2 %; at 100 cents +97 / -103): a delay
//! line can't bend the pitch symmetrically in cents (its mean speed is 1).
//!
//! - Periodic shapes (sine, triangle, square, saw up, saw down) are tables of `W` (1024
//!   points, Hermite-read in `f64`). The square's edges are tanh-rounded and the saws' resets
//!   take 4 % of the cycle, so the pitch never jumps.
//! - `Random` holds a random pitch for each cycle (raised-cosine glides over the last 20 %
//!   between them); `Drift` glides (raised cosine) through a random pitch per cycle: tape
//!   wow at slow rates, flutter at fast ones. Each cycle's pitch steers the delay toward a
//!   random target, so it stays bounded with the pitch within the depth. Both are seeded:
//!   renders are deterministic.
//! - The excursion is at most [`MAX_EXCURSION_MS`] around the centre delay `C`, which is the
//!   reported latency (PDC realigns the track; the dry path is read at `C` too, so `Mix`
//!   blends aligned signals — chorus-like below 100 %). Very slow rates cap the depth:
//!   at 0.5 Hz a sine reaches ~110 cents, at 0.1 Hz ~22.
//! - `Phase` offsets the LFO in its cycle (the "offset"), `Stereo Phase` the right channel
//!   further; both glide at a bounded speed (a phase move bends the pitch by at most ~9
//!   cents). Depth and rate apply at once: the delay follows `W` by increments at the
//!   current amplitude (no jump), then recentres on `C - A · W` (also ≤ ~9 cents).
//! - A shape change, or `Retrigger` = Transport restarting the LFO (free: phase 0; synced:
//!   onto the song position) and the random sequence when the transport starts, crossfades
//!   from the frozen old read to the new one over [`XFADE_MS`] (no click, no pitch sweep).

use std::sync::LazyLock;

use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus,
};

use super::shared::{Glide, Lfo, LfoStep, ModLine, Params, Ramp, Rate, input, ms};
use super::vibrato as p;
use crate::util::{self, db_to_amp};

const N: usize = p::COUNT;
/// Largest delay excursion around the centre (ms); the centre (latency) is a bit more.
pub const MAX_EXCURSION_MS: f32 = 20.0;
/// Crossfade of a shape change or a retrigger (ms).
pub const XFADE_MS: f32 = 15.0;
/// Shape labels (the `Shape` param's order).
pub const SHAPES: [&str; 7] = [
    "Sine", "Triangle", "Square", "Saw Up", "Saw Down", "Random", "Drift",
];
const RANDOM: usize = 5;
const DRIFT: usize = 6;
/// Periodic shapes (tabulated).
const PERIODIC: usize = 5;
const TABLE: usize = 1024;
/// Square: tanh edge sharpness. Saws: share of the cycle the reset takes.
const SQUARE_K: f64 = 6.0;
const SAW_EDGE: f64 = 0.04;
/// Random: share of the cycle the pitch glides to the next value over.
const RANDOM_EDGE: f64 = 0.2;
/// Peak `|W|` of the random shapes (bounds of their constructions: the delay sits within
/// `±0.5` at each cycle start, a cycle moves it by at most `0.5 · RANDOM_EDGE` or `1 / π`
/// further).
const RANDOM_PEAK: f64 = 0.5 + RANDOM_EDGE / 2.0;
const DRIFT_PEAK: f64 = 0.5 + std::f64::consts::FRAC_1_PI;
/// Largest speed deviation a phase glide or a recentring adds (0.5 % ≈ 9 cents).
const GLIDE_SPEED: f64 = 0.005;
/// Time constant of the recentring after a depth or rate change.
const LEAK_SECONDS: f64 = 0.25;
const SEED: u64 = 0x9e37_79b9_7f4a_7c15;

/// Pitch contour of periodic shape `shape` at `phase` (`0..1`), `-1..=1`: sine, triangle,
/// rounded square, saw up / down with a smooth reset.
pub fn contour(shape: usize, phase: f64) -> f64 {
    let x = phase - phase.floor();
    let saw = |x: f64| {
        if x < 1.0 - SAW_EDGE {
            -1.0 + 2.0 * x / (1.0 - SAW_EDGE)
        } else {
            (std::f64::consts::PI * (x - (1.0 - SAW_EDGE)) / SAW_EDGE).cos()
        }
    };
    match shape {
        0 => (std::f64::consts::TAU * x).sin(),
        1 => {
            if x < 0.25 {
                4.0 * x
            } else if x < 0.75 {
                2.0 - 4.0 * x
            } else {
                4.0 * x - 4.0
            }
        }
        2 => (SQUARE_K * (std::f64::consts::TAU * x).sin()).tanh() / SQUARE_K.tanh(),
        3 => saw(x),
        _ => -saw(x),
    }
}

/// Cycle integrals `W` of the periodic contours (zero mean) and their peaks.
struct Tables {
    w: [[f64; TABLE]; PERIODIC],
    peak: [f64; PERIODIC],
}

static TABLES: LazyLock<Box<Tables>> = LazyLock::new(|| {
    const SUB: usize = 32;
    let mut t = Box::new(Tables {
        w: [[0.0; TABLE]; PERIODIC],
        peak: [0.0; PERIODIC],
    });
    let h = 1.0 / (TABLE * SUB) as f64;
    for s in 0..PERIODIC {
        let mut acc = 0.0;
        for j in 0..TABLE {
            t.w[s][j] = acc;
            for k in 0..SUB {
                acc += contour(s, ((j * SUB + k) as f64 + 0.5) * h) * h;
            }
        }
        let mean = t.w[s].iter().sum::<f64>() / TABLE as f64;
        t.w[s].iter_mut().for_each(|v| *v -= mean);
        t.peak[s] = t.w[s].iter().fold(0.0, |m, v| f64::max(m, v.abs()));
    }
    t
});

/// `W` of periodic shape `s` at `phase` (Catmull-Rom over the table).
#[inline]
fn table_w(t: &Tables, s: usize, phase: f64) -> f64 {
    let x = (phase - phase.floor()) * TABLE as f64;
    let i = x as usize;
    let f = x - i as f64;
    let w = &t.w[s];
    let at = |k: usize| w[k & (TABLE - 1)];
    let (x0, x1, x2, x3) = (at(i + TABLE - 1), at(i), at(i + 1), at(i + 2));
    let c1 = 0.5 * (x2 - x0);
    let c2 = x0 - 2.5 * x1 + 2.0 * x2 - 0.5 * x3;
    let c3 = 0.5 * (x3 - x0) + 1.5 * (x1 - x2);
    ((c3 * f + c2) * f + c1) * f + x1
}

/// Largest `|W|` of shape `s`.
fn peak(s: usize) -> f64 {
    match s {
        RANDOM => RANDOM_PEAK,
        DRIFT => DRIFT_PEAK,
        _ => TABLES.peak[s.min(PERIODIC - 1)],
    }
}

/// Per-channel state of the random shapes: one uniform draw per cycle feeds both the
/// `Random` delays (a reflected walk in `±0.5`, steps ≤ 1) and the `Drift` target.
#[derive(Clone, Copy, Debug)]
struct RandomVoice {
    rng: u64,
    /// `Random` delays `u[n..=n+2]`.
    walk: [f64; 3],
    /// `Drift`: `W` at the cycle start, pitches at the cycle's start and end.
    drift_w: f64,
    drift: [f64; 2],
    /// Position in the current cycle (`0..1`, never backwards past its start).
    pos: f64,
    /// Last LFO phase seen (`None` after a reseed).
    last: Option<f64>,
}

impl RandomVoice {
    fn new() -> Self {
        let mut v = Self {
            rng: SEED,
            walk: [0.0; 3],
            drift_w: 0.0,
            drift: [0.0; 2],
            pos: 0.0,
            last: None,
        };
        v.reseed();
        v
    }

    fn reseed(&mut self) {
        self.rng = SEED;
        self.walk = [0.0; 3];
        self.drift_w = 0.0;
        self.drift = [0.0; 2];
        for _ in 0..3 {
            self.step();
        }
        self.pos = 0.0;
        self.last = None;
    }

    /// Next knot of both sequences.
    fn step(&mut self) {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        let x = self.rng.wrapping_mul(0x2545_f491_4f6c_dd1d);
        let r = (x >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0;
        let u = self.walk[2];
        let next = if (u + r).abs() <= 0.5 {
            u + r
        } else if (u - r).abs() <= 0.5 {
            u - r
        } else {
            (u + r).clamp(-0.5, 0.5)
        };
        self.walk = [self.walk[1], self.walk[2], next];
        // Drift: the next pitch takes the delay to the target `r / 2` (as far as a pitch
        // within ±1 can), so the delay stays within ±0.5 at the cycle starts.
        let [s0, s1] = self.drift;
        self.drift_w += 0.5 * (s0 + s1);
        self.drift = [s1, (2.0 * (0.5 * r - self.drift_w) - s1).clamp(-1.0, 1.0)];
    }

    /// Follow the LFO to `phase` (its own cycle advances on each forward wrap).
    #[inline]
    fn follow(&mut self, phase: f64) {
        if let Some(last) = self.last {
            let mut d = phase - last;
            d -= d.round();
            self.pos += d;
            if self.pos >= 1.0 {
                self.pos -= 1.0;
                self.step();
            }
            self.pos = self.pos.clamp(0.0, 1.0 - 1e-12);
        }
        self.last = Some(phase);
    }

    /// `Random`: pitch `s_n` held through the cycle, raised-cosine glide to `s_{n+1}` over its
    /// last [`RANDOM_EDGE`]; `W` is its exact integral (continuous across cycles).
    #[inline]
    fn w_random(&self) -> f64 {
        let [u0, u1, u2] = self.walk;
        let (s0, s1) = (u1 - u0, u2 - u1);
        let x = self.pos;
        let mut w = u0 + 0.5 * RANDOM_EDGE * s0 + s0 * x;
        let q = x - (1.0 - RANDOM_EDGE);
        if q > 0.0 {
            let pi = std::f64::consts::PI;
            w += (s1 - s0) * (0.5 * q - RANDOM_EDGE / (2.0 * pi) * (pi * q / RANDOM_EDGE).sin());
        }
        w
    }

    /// `Drift`: pitch glides `s0 → s1` over the cycle (raised cosine); `W` is its integral.
    #[inline]
    fn w_drift(&self) -> f64 {
        let [s0, s1] = self.drift;
        let x = self.pos;
        let pi = std::f64::consts::PI;
        self.drift_w + s0 * x + 0.5 * (s1 - s0) * (x - (pi * x).sin() / pi)
    }
}

pub(super) struct Vibrato {
    params: Params<N>,
    sample_rate: f32,
    lines: [ModLine; 2],
    /// Centre delay (samples) = latency.
    center: usize,
    lfo: Lfo,
    /// Delay amplitude `A = k · sr / f` (samples).
    amp: f64,
    /// Delay offset from the centre per channel (`-A · W`, followed incrementally) and the
    /// `W` of the last sample; `fresh` = start from the exact value (no increment).
    offset: [f64; 2],
    last_w: [f64; 2],
    fresh: bool,
    /// Per-sample recentring gain of the offset.
    leak: f64,
    /// LFO phase offset and the right channel's extra offset (cycles).
    phase: Glide,
    stereo: Glide,
    mix: Ramp,
    gain: Ramp,
    random: [RandomVoice; 2],
    /// Shape being played (a change crossfades).
    shape: usize,
    /// Crossfade: frozen old read delays, progress `0..=1` (1 = none) and its step.
    frozen: [f64; 2],
    fade: f32,
    fade_step: f32,
    /// Read delay of the last sample, per channel.
    last_delay: [f64; 2],
    was_playing: bool,
    loud: bool,
    tail: crate::util::Tail,
}

impl Vibrato {
    pub(super) fn new(desc: &DeviceDescriptor) -> Self {
        LazyLock::force(&TABLES);
        let mut v = Self {
            params: Params::<N>::new(desc),
            sample_rate: 48_000.0,
            lines: [ModLine::default(), ModLine::default()],
            center: 0,
            lfo: Lfo::default(),
            amp: 0.0,
            offset: [0.0; 2],
            last_w: [0.0; 2],
            fresh: true,
            leak: 0.0,
            phase: Glide::new(0.0),
            stereo: Glide::new(0.0),
            mix: Ramp::new(1.0),
            gain: Ramp::new(1.0),
            random: [RandomVoice::new(); 2],
            shape: 0,
            frozen: [0.0; 2],
            fade: 1.0,
            fade_step: 0.0,
            last_delay: [0.0; 2],
            was_playing: false,
            loud: false,
            tail: crate::util::Tail::default(),
        };
        v.allocate();
        v
    }

    fn allocate(&mut self) {
        let sr = self.sample_rate;
        self.center = ms(MAX_EXCURSION_MS, sr).round() as usize + 4;
        self.lines = [ModLine::new(2 * self.center), ModLine::new(2 * self.center)];
        for g in [&mut self.phase, &mut self.stereo] {
            g.set_time(30.0, sr);
        }
        self.leak = 1.0 / (LEAK_SECONDS * f64::from(sr));
        self.fade_step = 1.0 / ms(XFADE_MS, sr).max(1.0);
        self.shape = self.shape_param();
        self.last_delay = [self.center as f64; 2];
        for id in 0..N as u32 {
            self.sync_param(ParamId(id), false);
        }
        self.amp = self.amp_target(120.0);
        self.fresh = true;
    }

    fn shape_param(&self) -> usize {
        self.params.index(p::SHAPE, SHAPES.len())
    }

    fn sync_param(&mut self, id: ParamId, smooth: bool) {
        let v = self.params.get(id);
        match id {
            p::PHASE => self.phase.set(v / 360.0, smooth),
            p::STEREO_PHASE => self.stereo.set(v / 360.0, smooth),
            p::MIX => self.mix.set(v * 0.01, smooth),
            p::OUTPUT => self.gain.set(db_to_amp(v), smooth),
            _ => {}
        }
    }

    fn apply(&mut self, id: ParamId, value: f64, smooth: bool) {
        if self.params.set(id, value) {
            self.sync_param(id, smooth);
            if !smooth && matches!(id, p::SHAPE | p::DEPTH | p::RATE | p::SYNC | p::SYNC_RATE) {
                self.shape = self.shape_param();
                self.amp = self.amp_target(120.0);
                self.fresh = true;
            }
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

    /// Delay amplitude for the current depth and rate (`bpm` for a synced rate), capped so
    /// the excursion stays within [`MAX_EXCURSION_MS`].
    fn amp_target(&self, bpm: f64) -> f64 {
        let hz = match self.rate() {
            Rate::Hz(hz) => hz,
            Rate::Beats(beats) => bpm / 60.0 / beats,
        }
        .max(1e-3);
        let cents = f64::from(self.params.get(p::DEPTH));
        let k = (cents * std::f64::consts::LN_2 / 1200.0).sinh();
        let sr = f64::from(self.sample_rate);
        let cap = (self.center - 2) as f64 / peak(self.shape);
        (k * sr / hz).min(cap)
    }

    /// Freeze the current reads and crossfade to the new trajectory.
    fn start_fade(&mut self) {
        // Mid-fade, keep whichever read dominates.
        if self.fade >= 0.5 {
            self.frozen = self.last_delay;
        }
        self.fade = 0.0;
        // The new trajectory starts at its exact delay (the crossfade hides the jump).
        self.fresh = true;
    }

    /// Transport started (`Retrigger` = Transport): restart the LFO and the random sequence.
    fn retrigger(&mut self, step: &LfoStep) {
        if self.params.on(p::SYNC) {
            self.lfo.skip(step, 1);
        } else {
            self.lfo.restart();
        }
        self.random.iter_mut().for_each(RandomVoice::reseed);
    }

    fn retriggers(&self) -> bool {
        self.params.index(p::RETRIGGER, 2) == 1
    }

    fn render(
        &mut self,
        ctx: &ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        start: usize,
        end: usize,
    ) {
        let sr = self.sample_rate;
        let transport = ctx.transport;
        let step = LfoStep::new(self.rate(), transport, sr);
        let shape = self.shape_param();
        if shape != self.shape {
            self.start_fade();
            self.shape = shape;
        }
        if transport.playing && !self.was_playing && self.retriggers() {
            self.start_fade();
            self.retrigger(&step);
        }
        self.was_playing = transport.playing;
        let bpm = if transport.bpm > 0.0 {
            transport.bpm
        } else {
            120.0
        };
        // Depth and rate changes apply at once: the offset follows `W` by increments at the
        // current amplitude (the pitch is right at once, the delay doesn't jump), then
        // recentres on `-A · W` at a bounded speed.
        self.amp = self.amp_target(bpm);
        let amp = self.amp;
        let phase_slew = (GLIDE_SPEED / amp.max(1.0)) as f32;
        self.phase.set_slew(phase_slew);
        self.stereo.set_slew(phase_slew);
        let leak = self.leak;
        let tables: &Tables = &TABLES;
        let center = self.center as f64;
        let max = (2 * self.center) as f64 - 2.0;
        let channels = audio.outputs.len().min(2);
        let inputs = audio.inputs;
        for i in start..end {
            self.lfo.advance(&step, i);
            let fresh = std::mem::take(&mut self.fresh);
            let offset = f64::from(self.phase.tick());
            let stereo = f64::from(self.stereo.tick());
            let mix = self.mix.tick();
            let gain = self.gain.tick();
            let fading = self.fade < 1.0;
            let g = if fading {
                self.fade = (self.fade + self.fade_step).min(1.0);
                0.5 - 0.5 * (std::f32::consts::PI * self.fade).cos()
            } else {
                1.0
            };
            for ch in 0..channels {
                let ph = self.lfo.phase() + offset + if ch == 1 { stereo } else { 0.0 };
                let r = &mut self.random[ch];
                r.follow(ph);
                let w = match shape {
                    RANDOM => r.w_random(),
                    DRIFT => r.w_drift(),
                    s => table_w(tables, s, ph),
                };
                let o = &mut self.offset[ch];
                if fresh {
                    *o = -amp * w;
                } else {
                    *o -= amp * (w - self.last_w[ch]);
                    *o += ((-amp * w - *o) * leak).clamp(-GLIDE_SPEED, GLIDE_SPEED);
                }
                self.last_w[ch] = w;
                let d = (center + *o).clamp(2.0, max);
                self.last_delay[ch] = d;
                let line = &mut self.lines[ch];
                line.push(input(inputs, ch, i));
                let mut wet = line.read_f64(d);
                if fading {
                    wet = line.read_f64(self.frozen[ch]) * (1.0 - g) + wet * g;
                }
                self.loud |= wet.abs() >= util::SILENCE_FLOOR;
                let dry = line.tap(self.center);
                audio.outputs[ch][i] = (dry + (wet - dry) * mix) * gain;
            }
            for out in audio.outputs.iter_mut().skip(channels) {
                out[i] = 0.0;
            }
        }
    }
}

impl Node for Vibrato {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.allocate();
        self.reset();
    }

    fn reset(&mut self) {
        for l in &mut self.lines {
            l.clear();
        }
        self.fade = 1.0;
        self.fresh = true;
        if self.retriggers() {
            self.lfo.restart();
            self.random.iter_mut().for_each(RandomVoice::reseed);
        }
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
            // The line is silent: the random voices pick up from the new phase.
            self.random.iter_mut().for_each(|r| r.last = None);
            self.fade = 1.0;
            self.fresh = true;
        }
        self.loud = false;
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
        let n = c.frames;
        let silent_in = util::all_zero(audio.inputs, n);
        let quiet = silent_in
            && !self.loud
            && util::below_floor(audio.outputs.iter().map(|o| &o[..n.min(o.len())]));
        let settled = !self.phase.is_moving()
            && !self.stereo.is_moving()
            && !self.mix.is_smoothing()
            && !self.gain.is_smoothing()
            && self.fade >= 1.0;
        let hold = 2 * self.center + util::samples(util::BASE_HOLD_SECONDS, self.sample_rate);
        self.tail
            .status(!c.events.is_empty(), silent_in, quiet, settled, n, hold)
    }

    /// The centre delay: the dry and wet paths both sit `center` samples late.
    fn latency(&self) -> u32 {
        self.center as u32
    }
}

impl Device for Vibrato {
    fn descriptor(&self) -> DeviceDescriptor {
        super::descriptor(BuiltinDeviceType::Vibrato)
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.params.plain(id)
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.apply(id, value, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_integrate_the_contours() {
        // Sine: W = -cos(2πφ) / 2π.
        let t: &Tables = &TABLES;
        for k in 0..50 {
            let ph = k as f64 / 50.0 + 0.003;
            let exact = -(std::f64::consts::TAU * ph).cos() / std::f64::consts::TAU;
            assert!((table_w(t, 0, ph) - exact).abs() < 1e-7, "{ph}");
        }
        assert!((t.peak[1] - 0.125).abs() < 1e-3);
        // Rounded square: a bit under the hard square's 0.25.
        assert!(t.peak[2] > 0.22 && t.peak[2] < 0.25, "{}", t.peak[2]);
        // Numeric slope of W = the contour (the pitch shape).
        for s in 0..PERIODIC {
            for k in 0..40 {
                let ph = k as f64 / 40.0 + 0.0071;
                let h = 1e-5;
                let slope = (table_w(t, s, ph + h) - table_w(t, s, ph - h)) / (2.0 * h);
                assert!((slope - contour(s, ph)).abs() < 2e-3, "{s} {ph}");
            }
        }
    }

    #[test]
    fn random_shapes_stay_bounded_and_continuous() {
        let mut r = RandomVoice::new();
        let mut prev = (r.w_random(), r.w_drift());
        let inc = 1.0 / 997.0;
        for i in 1..200_000 {
            r.follow((i as f64 * inc).fract());
            let w = (r.w_random(), r.w_drift());
            assert!(w.0.abs() <= RANDOM_PEAK + 1e-9 && w.1.abs() <= DRIFT_PEAK + 1e-9);
            // Slope ≤ 1 per cycle (peak pitch ≤ depth), so a step moves ≤ inc.
            assert!((w.0 - prev.0).abs() <= inc * 1.0001, "random {i}");
            assert!((w.1 - prev.1).abs() <= inc * 1.0001, "drift {i}");
            prev = w;
        }
    }
}

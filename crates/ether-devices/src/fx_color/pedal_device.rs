//! Pedal: a guitar-pedal distortion with three circuits (overdrive, distortion, fuzz), a
//! bass / mid / treble tone stack, a sub boost and dry/wet, in the spirit of Live's Pedal.
//!
//! Signal path per channel: type-specific pre-filters → `Gain` (the drive) → the type's
//! clipping curve, 4x oversampled through the linear-phase half-band filters of
//! [`super::oversample`] with first-order ADAA (see [`Aa`]) → DC blocker → the
//! type's post low-pass → level trim → tone stack → `Mix` with the aligned dry signal →
//! `Output`. The device always reports the 4x round trip ([`LATENCY`] samples).
//!
//! - **Overdrive** (warm, smooth): the clipped path is high-passed at 500 Hz before a
//!   gently asymmetric soft clipper (even harmonics), and the clean signal is summed back
//!   after it, so lows stay round and the drive sits in the mids; a 5.5 kHz low-pass after.
//!   Drive 4 ..= 34 dB.
//! - **Distortion** (tight, aggressive): high-passed at 120 Hz, a gain-dependent low-pass
//!   before the clipper (an op-amp running out of bandwidth: 12 kHz at no gain down to
//!   4 kHz at full gain), a hard-kneed polynomial clipper, a 2-pole low-pass at 5 kHz after.
//!   Drive 10 ..= 46 dB.
//! - **Fuzz** (unstable, broken): high-passed at 60 Hz only (fat lows), a biased tanh
//!   whose bias follows the signal's envelope: loud notes clip nearly symmetrically,
//!   decays starve the stage (the bias grows, the small-signal gain drops ~10 dB and the
//!   clipping turns one-sided), so tails sputter and gate like a dying transistor (the
//!   bias follows in ~10 ms, out over ~60 ms; a 30 Hz DC blocker takes the shift). A 7 kHz
//!   low-pass after. Drive 16 ..= 52 dB.
//! - `Gain` 0 % still distorts (like Live's). The level trim keeps a -12 dBFS sine near its
//!   input level across `Gain` and `Type` (computed from the curve, 32-point probe, on a
//!   change), so `Output` is a real trim, not a fight with the drive.
//! - Tone stack: `Bass` a bell at 100 Hz, `Mid` a bell at 500 Hz / 1 kHz / 2 kHz (`Mid
//!   Freq`; the lower, the narrower), `Treble` a high shelf at 3.3 kHz, each ±12 dB at
//!   ±100 %; `Sub` a +6 dB low shelf at 250 Hz. Coefficients glide (no zipper).
//! - A `Type` change dips the wet path over 2 ms, swaps the circuit, waits for the swap's
//!   edge to leave the oversampling filters (~1 ms) and fades back in over 2 ms (no click).

use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus, Smoother,
};

use super::oversample::{self, LATENCY_4X, Stage1, Stage2};
use super::pedal as ids;
use super::shared::{FixedDelay, Values, db, flush, glide};
use crate::dsp::adaa::ln_cosh;
use crate::dsp::svf::{Coefs, Shape, Svf};
use crate::util::{LateEvents, Route, untag};

/// Reported latency (samples): the 4x round trip.
pub const LATENCY: usize = LATENCY_4X;
/// Mid bell centres of `Mid Freq` and their Q (narrower when lower).
pub const MID_FREQS: [f64; 3] = [500.0, 1000.0, 2000.0];
const MID_QS: [f64; 3] = [1.6, 1.2, 0.9];
/// Tone stack ranges.
const EQ_DB: f64 = 12.0;
const SUB_DB: f64 = 6.0;
const BASS_HZ: f64 = 100.0;
const TREBLE_HZ: f64 = 3300.0;
const SUB_HZ: f64 = 250.0;
/// DC blocker corner (the fuzz's is higher: its moving bias shifts the DC, like the small
/// coupling capacitors of such circuits).
const DC_HZ: f32 = 10.0;
const FUZZ_DC_HZ: f32 = 30.0;
/// Coefficient glide time constant.
const GLIDE_MS: f64 = 8.0;
/// Type-change dip (each way).
const SWITCH_MS: f32 = 2.0;
/// Overdrive: negative-side knee of the soft clipper, clean blend.
const OD_K: f64 = 1.25;
const OD_CLEAN: f32 = 1.0;
/// Fuzz: bias when loud and when starved; envelope attack / release.
const FUZZ_BIAS_LOUD: f64 = 0.4;
const FUZZ_BIAS_STARVED: f64 = 1.15;
const FUZZ_ATTACK_MS: f32 = 10.0;
const FUZZ_RELEASE_MS: f32 = 60.0;
/// Level of the trim probe sine (-12 dBFS).
const PROBE_AMP: f64 = 0.25;

/// Pedal circuit (param `Type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Overdrive,
    Distortion,
    Fuzz,
}

impl Kind {
    fn from_index(i: usize) -> Self {
        match i {
            0 => Self::Overdrive,
            1 => Self::Distortion,
            _ => Self::Fuzz,
        }
    }

    /// Drive in dB at `Gain` `g` (0..=1).
    pub fn drive_db(self, g: f32) -> f32 {
        let (lo, hi) = match self {
            Self::Overdrive => (4.0, 34.0),
            Self::Distortion => (10.0, 46.0),
            Self::Fuzz => (16.0, 52.0),
        };
        lo + (hi - lo) * g.clamp(0.0, 1.0)
    }

    /// The clipping curve at bias `b` (fuzz only), `f(0) = 0`, bounded.
    #[inline]
    pub fn curve(self, u: f64, b: f64) -> f64 {
        match self {
            Self::Overdrive => {
                if u >= 0.0 {
                    u.tanh()
                } else {
                    (OD_K * u).tanh() / OD_K
                }
            }
            Self::Distortion => {
                let w = (u * (2.0 / 3.0)).clamp(-1.0, 1.0);
                1.5 * (w - w * w * w / 3.0)
            }
            Self::Fuzz => (u + b).tanh() - b.tanh(),
        }
    }

    /// Antiderivative of [`Self::curve`] in `u` (0 at 0), for ADAA.
    #[inline]
    fn integral(self, u: f64, b: f64) -> f64 {
        match self {
            Self::Overdrive => {
                if u >= 0.0 {
                    ln_cosh(u)
                } else {
                    ln_cosh(OD_K * u) / (OD_K * OD_K)
                }
            }
            Self::Distortion => {
                let a = u.abs();
                if a <= 1.5 {
                    let w = u * (2.0 / 3.0);
                    let w2 = w * w;
                    2.25 * (0.5 * w2 - w2 * w2 / 12.0)
                } else {
                    15.0 / 16.0 + (a - 1.5)
                }
            }
            Self::Fuzz => ln_cosh(u + b) - ln_cosh(b) - b.tanh() * u,
        }
    }
}

/// Gain that brings a -12 dBFS sine through `kind` at drive `g` (clean blend included,
/// fuzz at its loud bias) back to its input RMS. Bounded work (32 points), RT-safe.
pub fn level_trim(kind: Kind, g: f64) -> f32 {
    const N: usize = 32;
    let clean = if kind == Kind::Overdrive {
        f64::from(OD_CLEAN)
    } else {
        0.0
    };
    let mut ys = [0.0f64; N];
    let mut mean = 0.0;
    for (j, y) in ys.iter_mut().enumerate() {
        let x = PROBE_AMP * (std::f64::consts::TAU * (j as f64 + 0.5) / N as f64).sin();
        *y = kind.curve(g * x, FUZZ_BIAS_LOUD) + clean * x;
        mean += *y;
    }
    mean /= N as f64;
    let var = ys.iter().map(|y| (y - mean) * (y - mean)).sum::<f64>() / N as f64;
    let rms_in = PROBE_AMP * std::f64::consts::FRAC_1_SQRT_2;
    if var <= 1e-18 {
        return 1.0;
    }
    (rms_in / var.sqrt()).clamp(0.01, 4.0) as f32
}

/// Estimated mean of the fuzz curve at bias `b` for a signal whose driven envelope is
/// `env`: `-tanh(b)` once it clips hard (a square between `1 - tanh b` and `-1 - tanh b`),
/// `-tanh(b)·sech²(b)·A²/2` for a small sine of peak `A` (the curve's second-order term);
/// one smooth expression joins the two.
#[inline]
fn fuzz_dc(b: f64, env: f64) -> f64 {
    // The follower sits between the mean and the peak of |u|.
    let a = 1.4 * env;
    let t = b.tanh();
    let sech2 = 1.0 - t * t;
    -t * (1.0 - (-0.5 * sech2 * a * a).exp())
}

/// First-order antiderivative anti-aliasing (as [`crate::dsp::adaa`]) for a curve whose
/// shape moves (the fuzz bias): both ends of the segment are integrated with the *current*
/// bias, so a bias step never divides an antiderivative jump by a tiny segment. The last
/// end's value is reused while the bias holds (it changes once per base-rate sample).
#[derive(Clone, Copy, Debug)]
struct Aa {
    x1: f64,
    /// `F(x1, b1)`.
    f1: f64,
    b1: f64,
}

impl Default for Aa {
    fn default() -> Self {
        Self {
            x1: 0.0,
            f1: 0.0,
            b1: f64::NAN,
        }
    }
}

impl Aa {
    fn reset(&mut self) {
        *self = Self::default();
    }

    #[inline]
    fn tick(&mut self, kind: Kind, x: f64, b: f64) -> f32 {
        let fx = kind.integral(x, b);
        let f1 = if b == self.b1 {
            self.f1
        } else {
            kind.integral(self.x1, b)
        };
        let d = x - self.x1;
        let y = if d.abs() > 1e-6 {
            (fx - f1) / d
        } else {
            kind.curve(0.5 * (x + self.x1), b)
        };
        self.x1 = x;
        self.f1 = fx;
        self.b1 = b;
        y as f32
    }
}

/// One-pole low-pass coefficient for `hz`.
fn one_pole(hz: f32, sample_rate: f32) -> f32 {
    1.0 - (-std::f32::consts::TAU * hz.min(0.45 * sample_rate) / sample_rate).exp()
}

#[derive(Clone, Copy)]
struct Channel {
    /// Pre high-pass (one-pole low-pass state subtracted) and pre low-pass states.
    pre_hp: f32,
    pre_lp: f32,
    /// Fuzz envelope.
    env: f32,
    x4a: Stage1,
    x4b: Stage2,
    x4_pad: f32,
    adaa: Aa,
    clean_delay: FixedDelay<LATENCY>,
    dry_delay: FixedDelay<LATENCY>,
    dc_x1: f32,
    dc_y1: f32,
    post_lp: f32,
    post: Svf,
    eq: [Svf; 4],
}

impl Channel {
    fn new() -> Self {
        Self {
            pre_hp: 0.0,
            pre_lp: 0.0,
            env: 0.0,
            x4a: oversample::stage1(),
            x4b: oversample::stage2(),
            x4_pad: 0.0,
            adaa: Aa::default(),
            clean_delay: FixedDelay::new(),
            dry_delay: FixedDelay::new(),
            dc_x1: 0.0,
            dc_y1: 0.0,
            post_lp: 0.0,
            post: Svf::ZERO,
            eq: [Svf::ZERO; 4],
        }
    }

    fn reset(&mut self) {
        self.pre_hp = 0.0;
        self.pre_lp = 0.0;
        self.env = 0.0;
        self.x4a.reset();
        self.x4b.reset();
        self.x4_pad = 0.0;
        self.adaa.reset();
        self.clean_delay.reset();
        self.dry_delay.reset();
        self.dc_x1 = 0.0;
        self.dc_y1 = 0.0;
        self.post_lp = 0.0;
        self.post.reset();
        self.eq = [Svf::ZERO; 4];
    }
}

/// Per-type filter coefficients (one-pole `a`s at the base rate).
#[derive(Clone, Copy)]
struct Circuit {
    kind: Kind,
    pre_hp: f32,
    /// Pre low-pass at no gain and at full gain (distortion), else 1 (open).
    pre_lp: (f32, f32),
    /// One-pole post low-pass (overdrive, fuzz), or `None` for the 2-pole one.
    post_lp: Option<f32>,
    /// DC blocker pole.
    dc_r: f32,
}

/// The pedal device.
pub struct Pedal {
    values: Values<{ ids::COUNT }>,
    sample_rate: f32,
    circuit: Circuit,
    /// Type waiting for the dip to reach zero.
    pending: Option<Kind>,
    /// Samples the wet path stays muted after a swap (the oversampling filters flush the
    /// old circuit's last samples and the swap's edge) before it fades back in.
    hold: u32,
    switch: Smoother,
    drive: Smoother,
    /// Gain 0..=1 (the distortion's pre low-pass follows it).
    gain: Smoother,
    trim: Smoother,
    output: Smoother,
    mix: Smoother,
    env_attack: f32,
    env_release: f32,
    post: Coefs,
    eq: [Coefs; 4],
    eq_target: [Coefs; 4],
    eq_gliding: bool,
    glide_coef: f64,
    ch: [Channel; 2],
    /// Silence reporting.
    tail: crate::util::Tail,
    /// The params as the output stage sees them (`values` is the clipper's view): the
    /// level trim follows the gain of the audio it multiplies, [`LATENCY`] samples after
    /// it reaches the input.
    out_values: Values<{ ids::COUNT }>,
    /// The trim, tone stack, output and mix act on the clipped signal, [`LATENCY`]
    /// samples into the path; the gain drives the clipper where it sees the input (after
    /// the up-sampling filters). The type switch dips the whole wet path: at once.
    late: LateEvents,
}

impl Default for Pedal {
    fn default() -> Self {
        Self::new()
    }
}

impl Pedal {
    /// Non-RT. A pedal with default params.
    pub fn new() -> Self {
        let desc = descriptor();
        let mut s = Self {
            out_values: Values::new(&desc),
            late: LateEvents::new(),
            values: Values::new(&desc),
            sample_rate: 48_000.0,
            circuit: Circuit {
                kind: Kind::Overdrive,
                pre_hp: 0.0,
                pre_lp: (1.0, 1.0),
                post_lp: None,
                dc_r: 1.0,
            },
            pending: None,
            hold: 0,
            switch: Smoother::new(1.0, 0.0, 48_000.0),
            drive: Smoother::new(1.0, 0.0, 48_000.0),
            gain: Smoother::new(0.0, 0.0, 48_000.0),
            trim: Smoother::new(1.0, 0.0, 48_000.0),
            output: Smoother::new(1.0, 0.0, 48_000.0),
            mix: Smoother::new(1.0, 0.0, 48_000.0),
            env_attack: 0.0,
            env_release: 0.0,
            post: Coefs::BYPASS,
            eq: [Coefs::BYPASS; 4],
            eq_target: [Coefs::BYPASS; 4],
            eq_gliding: false,
            glide_coef: 0.0,
            ch: [Channel::new(); 2],
            tail: crate::util::Tail::default(),
        };
        s.sync_all();
        s
    }

    fn sync_all(&mut self) {
        let sr = self.sample_rate;
        self.env_attack = one_pole(1000.0 / (std::f32::consts::TAU * FUZZ_ATTACK_MS), sr);
        self.env_release = one_pole(1000.0 / (std::f32::consts::TAU * FUZZ_RELEASE_MS), sr);
        self.glide_coef = (-1.0 / (GLIDE_MS * 0.001 * f64::from(sr))).exp();
        self.pending = None;
        self.hold = 0;
        self.switch.set_immediate(1.0);
        for i in 0..ids::COUNT {
            self.apply(ParamId(i as u32), false);
        }
    }

    fn circuit(&self, kind: Kind) -> Circuit {
        let sr = self.sample_rate;
        let dc = |hz: f32| 1.0 - std::f32::consts::TAU * hz / sr;
        match kind {
            Kind::Overdrive => Circuit {
                kind,
                pre_hp: one_pole(500.0, sr),
                pre_lp: (1.0, 1.0),
                post_lp: Some(one_pole(5500.0, sr)),
                dc_r: dc(DC_HZ),
            },
            Kind::Distortion => Circuit {
                kind,
                pre_hp: one_pole(120.0, sr),
                pre_lp: (one_pole(12_000.0, sr), one_pole(4000.0, sr)),
                post_lp: None,
                dc_r: dc(DC_HZ),
            },
            Kind::Fuzz => Circuit {
                kind,
                pre_hp: one_pole(60.0, sr),
                pre_lp: (1.0, 1.0),
                post_lp: Some(one_pole(7000.0, sr)),
                dc_r: dc(FUZZ_DC_HZ),
            },
        }
    }

    fn set_circuit(&mut self, kind: Kind) {
        self.circuit = self.circuit(kind);
        self.post = Coefs::new(
            Shape::HighCut,
            5000.0,
            0.0,
            0.6,
            f64::from(self.sample_rate),
        );
    }

    fn kind_param(&self) -> Kind {
        Kind::from_index(self.values.index(ids::TYPE))
    }

    /// Drive and trim for the current `Gain` (as the clipper and the output stage see
    /// it) and `kind`.
    fn drive_targets(&self, kind: Kind) -> (f32, f32) {
        let g = db(kind.drive_db(self.values.f(ids::GAIN) * 0.01));
        let out = db(kind.drive_db(self.out_values.f(ids::GAIN) * 0.01));
        (g, level_trim(kind, f64::from(out)))
    }

    fn eq_targets(&self) -> [Coefs; 4] {
        let sr = f64::from(self.sample_rate);
        let pct = |id| f64::from(self.values.f(id)) * 0.01;
        let mid = self.values.index(ids::MID_FREQ).min(2);
        let sub = if self.values.on(ids::SUB) {
            SUB_DB
        } else {
            0.0
        };
        [
            Coefs::new(Shape::Bell, BASS_HZ, pct(ids::BASS) * EQ_DB, 0.7, sr),
            Coefs::new(
                Shape::Bell,
                MID_FREQS[mid],
                pct(ids::MID) * EQ_DB,
                MID_QS[mid],
                sr,
            ),
            Coefs::new(
                Shape::HighShelf,
                TREBLE_HZ,
                pct(ids::TREBLE) * EQ_DB,
                0.707,
                sr,
            ),
            Coefs::new(Shape::LowShelf, SUB_HZ, sub, 0.707, sr),
        ]
    }

    fn apply(&mut self, id: ParamId, smooth: bool) {
        match id {
            ids::TYPE => {
                let k = self.kind_param();
                if !smooth {
                    self.set_circuit(k);
                    self.pending = None;
                    let (d, t) = self.drive_targets(k);
                    glide(&mut self.drive, d, false);
                    glide(&mut self.trim, t, false);
                } else if k != self.circuit.kind || self.pending.is_some() {
                    self.pending = Some(k);
                    let n = ((SWITCH_MS * 0.001 * self.sample_rate) as u32).max(1);
                    self.switch.ramp_to(0.0, n);
                }
            }
            ids::GAIN => {
                self.apply_drive(smooth);
                self.apply_trim(smooth);
            }
            ids::BASS | ids::MID | ids::MID_FREQ | ids::TREBLE | ids::SUB => {
                self.eq_target = self.eq_targets();
                if smooth {
                    self.eq_gliding = true;
                } else {
                    self.eq = self.eq_target;
                    self.eq_gliding = false;
                }
            }
            ids::OUTPUT => {
                let v = db(self.values.f(ids::OUTPUT));
                glide(&mut self.output, v, smooth);
            }
            ids::MIX => {
                let v = self.values.f(ids::MIX) * 0.01;
                glide(&mut self.mix, v, smooth);
            }
            _ => {}
        }
    }

    /// The clipper's part of a gain change (`values` updated).
    fn apply_drive(&mut self, smooth: bool) {
        let k = self.pending.unwrap_or(self.circuit.kind);
        let (d, _) = self.drive_targets(k);
        glide(&mut self.drive, d, smooth);
        glide(&mut self.gain, self.values.f(ids::GAIN) * 0.01, smooth);
    }

    /// The trim's part (`out_values` updated).
    fn apply_trim(&mut self, smooth: bool) {
        let k = self.pending.unwrap_or(self.circuit.kind);
        let (_, t) = self.drive_targets(k);
        glide(&mut self.trim, t, smooth);
    }

    fn set(&mut self, id: ParamId, value: f64, smooth: bool) {
        let clipper = self.values.set(id, value);
        if self.out_values.set(id, value) || clipper {
            self.apply(id, smooth);
        }
    }

    /// A param event as [`LateEvents`] delivers it (see [`Self::route`]).
    fn on_event(&mut self, kind: &EventKind) {
        let EventKind::Param { param, value } = *kind else {
            return;
        };
        match untag(param) {
            (id, true) => {
                if self.out_values.set(id, value) {
                    self.apply_trim(true);
                }
            }
            (ids::GAIN, false) => {
                if self.values.set(ids::GAIN, value) {
                    self.apply_drive(true);
                }
            }
            (id, false) => self.set(id, value, true),
        }
    }

    /// Where each param acts (see `late`).
    fn route(kind: &EventKind) -> Route {
        match kind {
            EventKind::Param { param, .. } => match *param {
                ids::GAIN => Route::Split {
                    early: oversample::UP_DELAY_4X,
                    late: LATENCY,
                },
                ids::TYPE => Route::Now,
                _ => Route::Late(LATENCY),
            },
            _ => Route::Now,
        }
    }

    /// The dip reached zero: swap the circuit (clearing its states) and come back.
    fn finish_switch(&mut self) {
        let Some(k) = self.pending.take() else {
            return;
        };
        self.set_circuit(k);
        for c in &mut self.ch {
            c.pre_hp = 0.0;
            c.pre_lp = 0.0;
            c.adaa.reset();
            c.post_lp = 0.0;
            c.post.reset();
        }
        let (d, t) = self.drive_targets(k);
        self.drive.set_immediate(d);
        self.trim.set_immediate(t);
        // The swap's edge reaches the output through the oversampling filters: stay muted
        // until it has passed (their latency plus their half length).
        self.hold = (LATENCY + oversample::TAPS_1 / 2 + 1) as u32;
    }

    fn render(&mut self, audio: &mut AudioBuffers<'_, '_>, start: usize, end: usize) {
        let n_ch = audio.outputs.len().min(2);
        for i in start..end {
            if self.eq_gliding {
                let mut moving = false;
                for (c, t) in self.eq.iter_mut().zip(&self.eq_target) {
                    moving |= c.glide(t, self.glide_coef);
                }
                self.eq_gliding = moving;
            }
            let sw = self.switch.tick();
            if self.pending.is_some() && sw == 0.0 && !self.switch.is_smoothing() {
                self.finish_switch();
            } else if self.hold > 0 {
                self.hold -= 1;
                if self.hold == 0 {
                    let n = ((SWITCH_MS * 0.001 * self.sample_rate) as u32).max(1);
                    self.switch.ramp_to(1.0, n);
                }
            }
            let drive = f64::from(self.drive.tick());
            let gain = self.gain.tick();
            let trim = self.trim.tick() * sw;
            let out_gain = self.output.tick();
            let mix = self.mix.tick();
            let cir = self.circuit;
            let pre_lp = cir.pre_lp.0 + (cir.pre_lp.1 - cir.pre_lp.0) * gain;
            let (att, rel) = (self.env_attack, self.env_release);
            for (c, st) in self.ch.iter_mut().enumerate().take(n_ch) {
                let x = audio
                    .inputs
                    .get(c)
                    .or(audio.inputs.first())
                    .map_or(0.0, |b| b[i]);
                // Pre-filters (base rate).
                st.pre_hp = flush(st.pre_hp + cir.pre_hp * (x - st.pre_hp));
                let hp = x - st.pre_hp;
                st.pre_lp = flush(st.pre_lp + pre_lp * (hp - st.pre_lp));
                let v = st.pre_lp;
                // The driven signal's envelope (followed in every type, so a switch to the
                // fuzz starts from it) sets the fuzz bias.
                let a = (v * drive as f32).abs();
                let k = if a > st.env { att } else { rel };
                st.env = flush(st.env + k * (a - st.env));
                let (bias, fuzz_dc) = if cir.kind == Kind::Fuzz {
                    let loud = f64::from(st.env.min(1.0));
                    let b = FUZZ_BIAS_STARVED + (FUZZ_BIAS_LOUD - FUZZ_BIAS_STARVED) * loud;
                    (b, fuzz_dc(b, f64::from(st.env)) as f32)
                } else {
                    (0.0, 0.0)
                };
                // Clipper, 4x with ADAA.
                let kind = cir.kind;
                let shape = |adaa: &mut Aa, s: f32| adaa.tick(kind, drive * f64::from(s), bias);
                let [a, b] = st.x4a.up(v);
                let [a0, a1] = st.x4b.up(a);
                let [b0, b1] = st.x4b.up(b);
                let (a0, a1) = (shape(&mut st.adaa, a0), shape(&mut st.adaa, a1));
                let (b0, b1) = (shape(&mut st.adaa, b0), shape(&mut st.adaa, b1));
                let a = st.x4b.down([a0, a1]);
                let b = st.x4b.down([b0, b1]);
                let first = st.x4_pad;
                st.x4_pad = b;
                let mut y = st.x4a.down([first, a]);
                let clean = st.clean_delay.tick(x);
                if kind == Kind::Overdrive {
                    y += OD_CLEAN * clean;
                }
                // The fuzz's level-dependent offset, removed before the DC blocker (which
                // then only takes what the estimate misses: no thump as the bias moves).
                y -= fuzz_dc;
                // DC blocker, post low-pass, trim.
                let d = y - st.dc_x1 + cir.dc_r * st.dc_y1;
                st.dc_x1 = y;
                st.dc_y1 = flush(d);
                let y = match cir.post_lp {
                    Some(k) => {
                        st.post_lp = flush(st.post_lp + k * (d - st.post_lp));
                        st.post_lp
                    }
                    None => st.post.tick(f64::from(d), &self.post) as f32,
                };
                // Tone stack.
                let mut t = f64::from(y * trim);
                for (f, c) in st.eq.iter_mut().zip(&self.eq) {
                    t = f.tick(t, c);
                }
                let dry = st.dry_delay.tick(x);
                audio.outputs[c][i] = (dry + (t as f32 - dry) * mix) * out_gain;
            }
        }
    }
}

fn descriptor() -> DeviceDescriptor {
    super::descriptor(BuiltinDeviceType::Pedal)
}

impl Node for Pedal {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.sync_all();
        self.late.prepare(LATENCY, config);
        self.reset();
    }

    fn reset(&mut self) {
        LateEvents::flush(
            self,
            |s| &mut s.late,
            |s, kind| {
                if let EventKind::Param { param, value } = *kind {
                    let (id, out) = untag(param);
                    if out {
                        s.out_values.set(id, value);
                        s.apply_trim(false);
                    } else {
                        s.set(id, value, false);
                    }
                }
            },
        );
        for c in &mut self.ch {
            c.reset();
        }
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        LateEvents::split(
            self,
            |s| &mut s.late,
            ctx,
            Self::route,
            |s, a, b| s.render(audio, a, b),
            Self::on_event,
        );
        // Silence in gives silence out (every curve passes through 0); the fuzz envelope's
        // bias moves the DC blocker's input only while it releases.
        let settled = self.pending.is_none()
            && self.late.is_idle()
            && self.hold == 0
            && !self.eq_gliding
            && self.ch.iter().all(|c| c.env < 1e-4)
            && ![
                &self.switch,
                &self.drive,
                &self.gain,
                &self.trim,
                &self.output,
                &self.mix,
            ]
            .iter()
            .any(|s| s.is_smoothing());
        let hold = LATENCY + crate::util::samples(crate::util::BASE_HOLD_SECONDS, self.sample_rate);
        self.tail.status_of(ctx, audio, settled, hold)
    }

    fn latency(&self) -> u32 {
        LATENCY as u32
    }
}

impl Device for Pedal {
    fn descriptor(&self) -> DeviceDescriptor {
        descriptor()
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.late.readback(id, self.values.get(id))
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.late.cancel(id);
        self.set(id, value, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_are_zero_at_zero_bounded_and_match_their_integrals() {
        for kind in [Kind::Overdrive, Kind::Distortion, Kind::Fuzz] {
            for b in [0.0, FUZZ_BIAS_LOUD, FUZZ_BIAS_STARVED] {
                assert_eq!(kind.curve(0.0, b), 0.0, "{kind:?}");
                assert_eq!(kind.integral(0.0, b), 0.0, "{kind:?}");
                for u in [-1e6, -3.0, 3.0, 1e6] {
                    assert!(kind.curve(u, b).abs() <= 2.0, "{kind:?} bounded");
                }
                // The integral's slope is the curve.
                for u in [-4.0, -1.2, -0.3, 0.2, 0.9, 1.49, 2.5] {
                    let h = 1e-5;
                    let slope = (kind.integral(u + h, b) - kind.integral(u - h, b)) / (2.0 * h);
                    assert!(
                        (slope - kind.curve(u, b)).abs() < 1e-4,
                        "{kind:?} at {u}: {slope} vs {}",
                        kind.curve(u, b)
                    );
                }
            }
        }
    }

    #[test]
    fn level_trim_compensates_the_drive() {
        for kind in [Kind::Overdrive, Kind::Distortion, Kind::Fuzz] {
            let lo = level_trim(kind, f64::from(db(kind.drive_db(0.0))));
            let hi = level_trim(kind, f64::from(db(kind.drive_db(1.0))));
            assert!(hi < lo, "{kind:?}: {lo} {hi}");
        }
    }
}

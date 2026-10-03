//! Per-voice DSP blocks of the Poly Synth: PolyBLEP/BLAMP virtual-analog oscillators,
//! curved ADSR envelopes, a TPT state-variable filter and a zero-delay-feedback ladder (both
//! with a saturating input stage), LFO shapes, noise. All `Copy`, allocation-free.

use std::f32::consts::{PI, TAU};

/// Values below this are flushed to zero (the audio thread has no FTZ).
const DENORMAL: f32 = 1e-15;

#[inline]
pub(crate) fn flush(x: f32) -> f32 {
    if x.abs() < DENORMAL { 0.0 } else { x }
}

/// Smooth saturator: a rational `tanh` approximation, exact at 0, monotonic, `±1` beyond 3.
#[inline]
pub(crate) fn sat(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

/// `2^x` (plain `exp2`: only used at control rate).
#[inline]
pub(crate) fn exp2(x: f32) -> f32 {
    x.exp2()
}

// --- oscillators -----------------------------------------------------------------------

/// PolyBLEP residual of a unit-height upward step at phase 0 (`t` = phase, `dt` = increment).
#[inline]
fn blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        // After the step: -(1-x)²/2.
        -0.5 * (1.0 - x) * (1.0 - x)
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        // Before the step: (1+x)²/2.
        0.5 * (1.0 + x) * (1.0 + x)
    } else {
        0.0
    }
}

/// PolyBLAMP residual (integrated [`blep`]) of a unit slope change per sample at phase 0.
#[inline]
fn blamp(t: f32, dt: f32) -> f32 {
    let d = if t < dt {
        t / dt
    } else if t > 1.0 - dt {
        (1.0 - t) / dt
    } else {
        return 0.0;
    };
    let r = 1.0 - d;
    r * r * r * (1.0 / 6.0)
}

#[inline]
fn wrap(p: f32) -> f32 {
    if p >= 1.0 {
        p - 1.0
    } else if p < 0.0 {
        p + 1.0
    } else {
        p
    }
}

/// Virtual-analog shapes (plain values of the `Type` param, `Wavetable` excluded).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shape {
    Saw,
    Square,
    Triangle,
    Sine,
}

/// One band-limited sample of `shape` at `phase` (0..1) with increment `dt` and pulse
/// width `pw` (0..1, square only). `sine` is a lookup table.
#[inline]
pub(crate) fn va(shape: Shape, phase: f32, dt: f32, pw: f32, sine: &SineTable) -> f32 {
    match shape {
        Shape::Saw => {
            // Falls by 2 at phase 0.
            2.0 * phase - 1.0 - 2.0 * blep(phase, dt)
        }
        Shape::Square => {
            let naive = if phase < pw { 1.0 } else { -1.0 };
            // Rises by 2 at 0, falls by 2 at `pw`; centred (no DC for any width).
            naive + 2.0 * blep(phase, dt) - 2.0 * blep(wrap(phase - pw), dt) - (2.0 * pw - 1.0)
        }
        Shape::Triangle => {
            // -1 at 0, +1 at 0.5: slope ±4 per cycle, changes by 8·dt per sample at corners.
            let naive = 1.0 - 4.0 * (phase - 0.5).abs();
            let k = 8.0 * dt;
            naive + k * blamp(phase, dt) - k * blamp(wrap(phase - 0.5), dt)
        }
        Shape::Sine => sine.at(phase),
    }
}

/// A 1024-point sine cycle with linear interpolation (−100 dB error).
pub(crate) struct SineTable {
    t: [f32; 1025],
}

impl SineTable {
    pub(crate) fn new() -> Self {
        let mut t = [0.0; 1025];
        for (i, v) in t.iter_mut().enumerate() {
            *v = (TAU * i as f32 / 1024.0).sin();
        }
        Self { t }
    }

    #[inline]
    pub(crate) fn at(&self, phase: f32) -> f32 {
        let x = phase * 1024.0;
        let i = (x as usize).min(1023);
        let f = x - i as f32;
        let a = self.t[i];
        a + (self.t[i + 1] - a) * f
    }
}

// --- envelopes -------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// Per-sample rates of an envelope (shared by every voice).
#[derive(Clone, Copy, Debug)]
pub(crate) struct EnvRates {
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
}

/// Attack overshoot target: an RC charge towards 1.5 reaches 1 after `ln 3` time constants,
/// giving the convex "analog" attack curve.
const ATTACK_TARGET: f32 = 1.5;
/// Level below which a releasing envelope ends (-100 dB).
const ENV_FLOOR: f32 = 1e-5;
/// Shortest attack (click-free note starts on any waveform phase).
pub(crate) const MIN_ATTACK_MS: f32 = 0.5;
/// Shortest decay/release.
pub(crate) const MIN_RELEASE_MS: f32 = 1.0;

impl EnvRates {
    /// Times in ms, sustain 0..1.
    pub(crate) fn new(attack: f32, decay: f32, sustain: f32, release: f32, sr: f32) -> Self {
        let samples = |ms: f32| (ms * 0.001 * sr).max(1.0);
        Self {
            attack: 1.0 - (-(3f32.ln()) / samples(attack.max(MIN_ATTACK_MS))).exp(),
            // Exponential segments cover -80 dB in the given time.
            decay: (-9.21 / samples(decay.max(MIN_RELEASE_MS))).exp(),
            sustain: sustain.clamp(0.0, 1.0),
            release: (-9.21 / samples(release.max(MIN_RELEASE_MS))).exp(),
        }
    }
}

/// ADSR with a curved attack and exponential decay/release. Retriggers and releases from
/// the current level (never jumps).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Env {
    stage: Stage,
    pub level: f32,
}

impl Env {
    pub(crate) const IDLE: Self = Self {
        stage: Stage::Idle,
        level: 0.0,
    };

    pub(crate) fn trigger(&mut self) {
        self.stage = Stage::Attack;
    }

    pub(crate) fn release(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }

    pub(crate) fn is_active(&self) -> bool {
        self.stage != Stage::Idle
    }

    pub(crate) fn is_releasing(&self) -> bool {
        self.stage == Stage::Release
    }

    #[inline]
    pub(crate) fn tick(&mut self, r: &EnvRates) -> f32 {
        match self.stage {
            Stage::Idle => {}
            Stage::Attack => {
                self.level += (ATTACK_TARGET - self.level) * r.attack;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay | Stage::Sustain => {
                // Also follows sustain changes smoothly.
                self.level = r.sustain + (self.level - r.sustain) * r.decay;
                if self.stage == Stage::Decay && (self.level - r.sustain).abs() < ENV_FLOOR {
                    self.stage = Stage::Sustain;
                }
                if self.level < ENV_FLOOR && r.sustain == 0.0 {
                    self.level = 0.0;
                }
            }
            Stage::Release => {
                self.level *= r.release;
                if self.level < ENV_FLOOR {
                    *self = Self::IDLE;
                }
            }
        }
        self.level
    }
}

// --- filters ---------------------------------------------------------------------------

/// Filter modes (plain values of the filter `Type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FilterMode {
    Ladder24,
    Low12,
    High12,
    Band12,
    Notch,
}

impl FilterMode {
    pub(crate) fn from_index(i: usize) -> Self {
        match i {
            0 => Self::Ladder24,
            1 => Self::Low12,
            2 => Self::High12,
            3 => Self::Band12,
            _ => Self::Notch,
        }
    }
}

/// `g = tan(π·fc/fs)` with `fc` clamped below Nyquist.
#[inline]
pub(crate) fn prewarp(fc: f32, sr: f32) -> f32 {
    (PI * fc.clamp(10.0, sr * 0.46) / sr).tan()
}

/// SVF damping `k = 1/Q` for resonance 0..1: Butterworth (Q 0.707) at 0, Q 50 at 1.
#[inline]
pub(crate) fn svf_damping(res: f32) -> f32 {
    let r = 1.0 - res.clamp(0.0, 1.0);
    0.02 + (std::f32::consts::SQRT_2 - 0.02) * r * r.sqrt()
}

/// Ladder feedback for resonance 0..1 (self-oscillates just below 1).
#[inline]
pub(crate) fn ladder_feedback(res: f32) -> f32 {
    4.2 * res.clamp(0.0, 1.0)
}

/// One channel of filter state (SVF uses `s[0..2]`, ladder `s[0..4]`).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FilterState {
    s: [f32; 4],
}

impl FilterState {
    pub(crate) const ZERO: Self = Self { s: [0.0; 4] };

    pub(crate) fn flush(&mut self) {
        for v in &mut self.s {
            *v = flush(*v);
        }
    }

    /// One sample through `mode` with prewarped `g`, `k` (SVF damping or ladder feedback),
    /// and `drive` (input gain ≥ 1 into the saturator).
    #[inline]
    pub(crate) fn tick(&mut self, mode: FilterMode, x: f32, g: f32, k: f32, drive: f32) -> f32 {
        match mode {
            FilterMode::Ladder24 => self.ladder(x, g, k, drive),
            _ => {
                // TPT SVF (Simper), saturating input stage.
                let v0 = sat(x * drive);
                let a1 = 1.0 / (1.0 + g * (g + k));
                let a2 = g * a1;
                let a3 = g * a2;
                let v3 = v0 - self.s[1];
                let v1 = a1 * self.s[0] + a2 * v3;
                let v2 = self.s[1] + a2 * self.s[0] + a3 * v3;
                self.s[0] = 2.0 * v1 - self.s[0];
                self.s[1] = 2.0 * v2 - self.s[1];
                match mode {
                    FilterMode::Low12 => v2,
                    FilterMode::High12 => v0 - k * v1 - v2,
                    FilterMode::Band12 => k * v1,
                    _ => v0 - k * v1,
                }
            }
        }
    }

    /// Four TPT one-poles in a zero-delay feedback loop (Zavalishin), the loop input
    /// through the saturator, partial passband-gain compensation.
    #[inline]
    fn ladder(&mut self, x: f32, g: f32, k: f32, drive: f32) -> f32 {
        let big_g = g / (1.0 + g);
        let inv = 1.0 / (1.0 + g);
        // y4 = G⁴·u + Σ, with each stage y = G·x + s/(1+g).
        let s = &mut self.s;
        let sigma = big_g * big_g * big_g * s[0] * inv
            + big_g * big_g * s[1] * inv
            + big_g * s[2] * inv
            + s[3] * inv;
        let g4 = big_g * big_g * big_g * big_g;
        let xin = x * drive * (1.0 + 0.5 * k);
        let y4_est = (g4 * xin + sigma) / (1.0 + k * g4);
        let mut u = sat(xin - k * y4_est);
        for st in s.iter_mut() {
            let v = (u - *st) * big_g;
            let y = v + *st;
            *st = y + v;
            u = y;
        }
        u
    }
}

// --- LFO -------------------------------------------------------------------------------

/// LFO shapes (plain values of `Shape`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LfoShape {
    Sine,
    Triangle,
    Saw,
    Square,
    SampleHold,
}

impl LfoShape {
    pub(crate) fn from_index(i: usize) -> Self {
        match i {
            0 => Self::Sine,
            1 => Self::Triangle,
            2 => Self::Saw,
            3 => Self::Square,
            _ => Self::SampleHold,
        }
    }

    /// Bipolar value at `phase` (0..1); `held` is the current sample & hold value.
    #[inline]
    pub(crate) fn at(self, phase: f32, held: f32) -> f32 {
        match self {
            Self::Sine => (TAU * phase).sin(),
            Self::Triangle => {
                if phase < 0.25 {
                    4.0 * phase
                } else if phase < 0.75 {
                    2.0 - 4.0 * phase
                } else {
                    4.0 * phase - 4.0
                }
            }
            Self::Saw => 2.0 * phase - 1.0,
            Self::Square => {
                if phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Self::SampleHold => held,
        }
    }
}

// --- noise -----------------------------------------------------------------------------

/// xorshift32 white noise in -1..1.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Rng(u32);

impl Rng {
    pub(crate) const fn new(seed: u32) -> Self {
        Self(if seed == 0 { 1 } else { seed })
    }

    #[inline]
    pub(crate) fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// Uniform -1..1.
    #[inline]
    pub(crate) fn bipolar(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 * (2.0 / 16_777_216.0) - 1.0
    }

    /// Uniform 0..1.
    #[inline]
    pub(crate) fn unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 * (1.0 / 16_777_216.0)
    }
}

/// Noise color: one-pole low-pass (dark, color < 50 %) or high-pass (bright, > 50 %).
#[derive(Clone, Copy, Debug)]
pub(crate) struct NoiseColor {
    /// One-pole coefficient.
    a: f32,
    high: bool,
}

impl NoiseColor {
    /// `color` 0..1 (0.5 = white).
    pub(crate) fn new(color: f32, sr: f32) -> Self {
        let c = color.clamp(0.0, 1.0);
        if c <= 0.5 {
            // 200 Hz (dark) .. 20 kHz (white) low-pass.
            let fc = 200.0 * 100f32.powf(c * 2.0);
            Self {
                a: 1.0 - (-TAU * fc.min(sr * 0.49) / sr).exp(),
                high: false,
            }
        } else {
            // 20 Hz .. 6 kHz high-pass.
            let fc = 20.0 * 300f32.powf((c - 0.5) * 2.0);
            Self {
                a: 1.0 - (-TAU * fc / sr).exp(),
                high: true,
            }
        }
    }

    /// Filter white noise `x` with state `z`.
    #[inline]
    pub(crate) fn tick(&self, x: f32, z: &mut f32) -> f32 {
        *z += (x - *z) * self.a;
        if self.high {
            // Bright noise: boost to keep the level comparable.
            (x - *z) * 1.4
        } else {
            *z
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Power of the harmonics that alias (not multiples of f0 up to Nyquist), relative to
    /// the total, via a naive DFT at the alias frequencies.
    fn alias_ratio(f: impl Fn(f32, f32) -> f32, hz: f32) -> f32 {
        let sr = 48_000.0;
        let n = 4800; // 10 Hz bins
        let dt = hz / sr;
        let mut phase = 0.0f32;
        let mut x = vec![0.0f32; n];
        for (i, v) in x.iter_mut().enumerate() {
            // Hann window: keeps the harmonics' leakage out of the alias bins.
            let w = 0.5 - 0.5 * (TAU * i as f32 / n as f32).cos();
            *v = f(phase, dt) * w;
            phase = wrap(phase + dt);
        }
        let total: f32 = x.iter().map(|v| v * v).sum::<f32>() / n as f32;
        // Energy in bins 100 Hz .. 3 kHz that are not harmonics (aliases fold there).
        let mut alias = 0.0;
        let mut bin = 10;
        while bin < 300 {
            let fr = bin as f32 * 10.0;
            let near_harmonic = ((fr / hz) - (fr / hz).round()).abs() * hz < 30.0;
            if !near_harmonic {
                let (mut re, mut im) = (0.0f32, 0.0f32);
                for (i, v) in x.iter().enumerate() {
                    let a = TAU * fr * i as f32 / sr;
                    re += v * a.cos();
                    im += v * a.sin();
                }
                alias += (re * re + im * im) * 2.0 / (n as f32 * n as f32);
            }
            bin += 1;
        }
        alias / total
    }

    #[test]
    fn blep_oscillators_alias_far_less_than_naive() {
        let sine = SineTable::new();
        let hz = 3_517.0;
        for shape in [Shape::Saw, Shape::Square, Shape::Triangle] {
            let bl = alias_ratio(|p, dt| va(shape, p, dt, 0.5, &sine), hz);
            let naive = alias_ratio(|p, _| va(shape, p, 1e-9, 0.5, &sine), hz);
            assert!(bl < naive * 0.2, "{shape:?}: {bl} vs naive {naive}");
        }
    }

    #[test]
    fn va_shapes_are_bounded_and_centred() {
        let sine = SineTable::new();
        for shape in [Shape::Saw, Shape::Square, Shape::Triangle, Shape::Sine] {
            for pw in [0.05, 0.5, 0.95] {
                let dt = 0.013;
                let (mut sum, mut peak) = (0.0, 0.0f32);
                let n = 10_000;
                let mut p = 0.0;
                for _ in 0..n {
                    let v = va(shape, p, dt, pw, &sine);
                    sum += v;
                    peak = peak.max(v.abs());
                    p = wrap(p + dt);
                }
                assert!(peak < 2.05, "{shape:?} {pw}: {peak}");
                assert!((sum / n as f32).abs() < 0.02, "{shape:?} {pw}: dc");
            }
        }
    }

    #[test]
    fn envelope_is_continuous_on_retrigger_and_release() {
        let r = EnvRates::new(5.0, 50.0, 0.5, 20.0, 48_000.0);
        let mut e = Env::IDLE;
        e.trigger();
        let mut last = 0.0;
        for i in 0..20_000 {
            if i == 100 {
                e.release();
            }
            if i == 300 {
                e.trigger();
            }
            if i == 2000 {
                e.release();
            }
            let v = e.tick(&r);
            assert!((v - last).abs() < 0.02, "jump at {i}: {last} → {v}");
            last = v;
        }
        assert!(!e.is_active());
    }

    #[test]
    fn filters_are_stable_at_extremes() {
        for mode in [
            FilterMode::Ladder24,
            FilterMode::Low12,
            FilterMode::High12,
            FilterMode::Band12,
            FilterMode::Notch,
        ] {
            for (fc, res, drive) in [(20.0, 1.0, 16.0), (20_000.0, 1.0, 1.0), (1000.0, 0.0, 1.0)] {
                let g = prewarp(fc, 48_000.0);
                let k = if mode == FilterMode::Ladder24 {
                    ladder_feedback(res)
                } else {
                    svf_damping(res)
                };
                let mut st = FilterState::default();
                let mut rng = Rng::new(7);
                for _ in 0..48_000 {
                    let y = st.tick(mode, rng.bipolar(), g, k, drive);
                    assert!(y.is_finite() && y.abs() < 20.0, "{mode:?} {fc} {res}: {y}");
                }
            }
        }
    }

    #[test]
    fn low_pass_modes_pass_bass_and_cut_highs() {
        let sr = 48_000.0;
        for (mode, k) in [
            (FilterMode::Ladder24, ladder_feedback(0.0)),
            (FilterMode::Low12, svf_damping(0.0)),
        ] {
            let g = prewarp(1000.0, sr);
            let gain = |hz: f32| {
                let mut st = FilterState::default();
                let mut peak = 0.0f32;
                for i in 0..9600 {
                    let x = 0.05 * (TAU * hz * i as f32 / sr).sin();
                    let y = st.tick(mode, x, g, k, 1.0);
                    if i > 4800 {
                        peak = peak.max(y.abs());
                    }
                }
                peak / 0.05
            };
            assert!(gain(100.0) > 0.8, "{mode:?} bass {}", gain(100.0));
            let slope = if mode == FilterMode::Ladder24 {
                0.01
            } else {
                0.03
            };
            assert!(gain(8000.0) < slope, "{mode:?} highs {}", gain(8000.0));
        }
    }
}

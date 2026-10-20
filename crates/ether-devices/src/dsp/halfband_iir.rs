//! Polyphase IIR half-band filters for cheap 2x oversampling inside a voice (the Poly
//! Synth's filter saturator).
//!
//! The half-band low-pass is `H(z) = ½·(A0(z²) + z⁻¹·A1(z²))`, each `Ai` a chain of
//! first-order allpasses `(a + z⁻¹) / (1 + a·z⁻¹)` at the low rate (Valenzuela &
//! Constantinides; the elliptic design of Laurent de Soras' `hiir`). The upsampler runs one
//! branch per output sample, the downsampler one branch per input sample: [`COEFS`]
//! multiplies each way, no FIR history. The phase is not linear (an allpass's, like any
//! analog-style filter), the group delay at low frequencies is a couple of samples, and the
//! response is designed once ([`Coefs::get`]) for a pass band up to 20 kHz at 48 kHz with
//! the images and aliases above 28 kHz down by 80 dB or more (checked by the tests).

use std::f64::consts::PI;
use std::sync::OnceLock;

/// Allpass coefficients (shared by both branches, alternating).
pub(crate) const COEFS: usize = 8;
/// Transition band of the design, normalized to the high rate: 0.5 · (28 - 20) / 48.
const TRANSITION: f64 = 4.0 / 48.0;
/// Stop-band rejection the design reaches (dB).
#[cfg(test)]
const STOP_DB: f64 = 80.0;

/// The designed allpass coefficients.
#[derive(Debug)]
pub(crate) struct Coefs([f32; COEFS]);

impl Coefs {
    /// The shared design (computed on first use: non-RT, call it from `new` / `prepare`).
    pub(crate) fn get() -> &'static Coefs {
        static C: OnceLock<Coefs> = OnceLock::new();
        C.get_or_init(|| Coefs(design(COEFS, TRANSITION).map(|c| c as f32)))
    }
}

/// Elliptic half-band design (`hiir::PolyphaseIir2Designer::compute_coefs_spec_order_tbw`).
fn design(n: usize, transition: f64) -> [f64; COEFS] {
    let mut k = ((1.0 - transition * 2.0) * PI / 4.0).tan();
    k *= k;
    let kksqrt = (1.0 - k * k).powf(0.25);
    let e = 0.5 * (1.0 - kksqrt) / (1.0 + kksqrt);
    let e4 = e.powi(4);
    let q = e * (1.0 + e4 * (2.0 + e4 * (15.0 + 150.0 * e4)));
    let order = (2 * n + 1) as f64;
    let mut out = [0.0; COEFS];
    for (index, coef) in out.iter_mut().enumerate().take(n) {
        let c = (index + 1) as f64;
        let mut num = 0.0;
        let mut i = 0i32;
        let mut sign = 1.0;
        loop {
            let t = q.powi(i * (i + 1)) * ((2 * i + 1) as f64 * c * PI / order).sin() * sign;
            num += t;
            sign = -sign;
            i += 1;
            if t.abs() <= 1e-100 || i > 64 {
                break;
            }
        }
        let mut den = 0.0;
        let mut i = 1i32;
        let mut sign = -1.0;
        loop {
            let t = q.powi(i * i) * (2.0 * i as f64 * c * PI / order).cos() * sign;
            den += t;
            sign = -sign;
            i += 1;
            if t.abs() <= 1e-100 || i > 64 {
                break;
            }
        }
        let ww = num * q.powf(0.25) / (den + 0.5);
        let wwsq = ww * ww;
        let x = ((1.0 - wwsq * k) * (1.0 - wwsq / k)).sqrt() / (1.0 + wwsq);
        *coef = (1.0 - x) / (1.0 + x);
    }
    out
}

/// The two allpass chains' state: `x[i]` / `y[i]` are the previous input / output of
/// stage `i` (stage `i` of branch `i % 2`).
#[derive(Clone, Copy, Debug, Default)]
struct Branches {
    x: [f32; COEFS],
    y: [f32; COEFS],
}

impl Branches {
    const ZERO: Self = Self {
        x: [0.0; COEFS],
        y: [0.0; COEFS],
    };

    /// Run `s0` through branch 0 (even stages) and `s1` through branch 1 (odd stages).
    #[inline]
    fn run(&mut self, c: &Coefs, mut s0: f32, mut s1: f32) -> (f32, f32) {
        let mut i = 0;
        while i < COEFS {
            let y0 = (s0 - self.y[i]) * c.0[i] + self.x[i];
            self.x[i] = s0;
            self.y[i] = y0;
            s0 = y0;
            let y1 = (s1 - self.y[i + 1]) * c.0[i + 1] + self.x[i + 1];
            self.x[i + 1] = s1;
            self.y[i + 1] = y1;
            s1 = y1;
            i += 2;
        }
        (s0, s1)
    }

    fn flush(&mut self) {
        for v in self.x.iter_mut().chain(self.y.iter_mut()) {
            *v = super::flush32(*v);
        }
    }
}

/// A 2x up / down pair for one stream (`Copy`, no allocation).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Oversampler2x {
    up: Branches,
    down: Branches,
}

impl Oversampler2x {
    pub(crate) const ZERO: Self = Self {
        up: Branches::ZERO,
        down: Branches::ZERO,
    };

    /// One base-rate sample in, two high-rate samples out (in time order, unity gain).
    #[inline]
    pub(crate) fn up(&mut self, c: &Coefs, x: f32) -> [f32; 2] {
        let (a, b) = self.up.run(c, x, x);
        [a, b]
    }

    /// Two high-rate samples (in time order) in, one base-rate sample out.
    #[inline]
    pub(crate) fn down(&mut self, c: &Coefs, pair: [f32; 2]) -> f32 {
        let (a, b) = self.down.run(c, pair[1], pair[0]);
        0.5 * (a + b)
    }

    /// Flush state that decays towards the subnormal range (call once per block).
    pub(crate) fn flush(&mut self) {
        self.up.flush();
        self.down.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gain (dB) of `f` (a round trip) for a sine at `hz` (48 kHz base rate).
    fn sine_through(hz: f64, f: impl Fn(&mut Oversampler2x, f32) -> f32) -> f64 {
        let mut os = Oversampler2x::ZERO;
        let n = 48_000;
        let mut acc = 0.0;
        let mut ref_acc = 0.0;
        for i in 0..n {
            let x = (2.0 * PI * hz * i as f64 / 48_000.0).sin() as f32;
            let y = f(&mut os, x);
            if i > n / 2 {
                acc += f64::from(y) * f64::from(y);
                ref_acc += f64::from(x) * f64::from(x);
            }
        }
        10.0 * (acc / ref_acc).log10()
    }

    #[test]
    fn round_trip_is_flat_in_the_audio_band() {
        let c = Coefs::get();
        for hz in [50.0, 1000.0, 10_000.0, 18_000.0, 20_000.0] {
            let g = sine_through(hz, |os, x| {
                let p = os.up(c, x);
                os.down(c, p)
            });
            assert!(g.abs() < 0.01, "{hz} Hz: {g:.4} dB");
        }
    }

    #[test]
    fn upsampler_images_and_downsampler_aliases_are_rejected() {
        let c = Coefs::get();
        // Upsampled sine: what is left at its image (48 k - hz), via a high-rate sine fit.
        for hz in [1000.0, 10_000.0, 20_000.0] {
            let mut os = Oversampler2x::ZERO;
            let n = 96_000;
            let mut hi = Vec::with_capacity(n);
            for i in 0..n / 2 {
                let x = (2.0 * PI * hz * i as f64 / 48_000.0).sin() as f32;
                hi.extend(os.up(c, x));
            }
            let img = 48_000.0 - hz;
            let (mut re, mut im) = (0.0, 0.0);
            for (i, v) in hi.iter().enumerate().skip(n / 2) {
                let a = 2.0 * PI * img * i as f64 / 96_000.0;
                re += f64::from(*v) * a.cos();
                im += f64::from(*v) * a.sin();
            }
            let amp = 2.0 * (re * re + im * im).sqrt() / (n / 2) as f64;
            let db = 20.0 * amp.max(1e-12).log10();
            assert!(db < -STOP_DB, "{hz} Hz image: {db:.1} dB");
        }
        // A high-rate sine above 28 kHz decimated: what folds back.
        for hz in [28_500.0, 35_000.0, 47_000.0] {
            let mut os = Oversampler2x::ZERO;
            let n = 48_000;
            let mut acc = 0.0;
            for i in 0..n {
                let s = |k: usize| (2.0 * PI * hz * k as f64 / 96_000.0).sin() as f32;
                let y = os.down(c, [s(2 * i), s(2 * i + 1)]);
                if i > n / 2 {
                    acc += f64::from(y) * f64::from(y);
                }
            }
            let db = 10.0 * (acc / (n / 2) as f64 / 0.5).log10();
            assert!(db < -STOP_DB, "{hz} Hz alias: {db:.1} dB");
        }
    }
}

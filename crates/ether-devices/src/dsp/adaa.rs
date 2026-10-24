//! First-order antiderivative anti-aliasing (ADAA; Parker, Zavalishin & Le Bivic, DAFx
//! 2016) for static waveshapers.
//!
//! Instead of `f(x[n])`, output the mean of `f` over the segment from `x[n-1]` to `x[n]`:
//! `(F(x[n]) - F(x[n-1])) / (x[n] - x[n-1])` with `F` the antiderivative of `f` (falling
//! back to `f` at the midpoint when the segment is too short to divide by). That rejects
//! much of the aliasing a hard curve makes, at the price of a half-sample delay and the
//! gentle low-pass of a 2-tap average, so it is used on oversampled paths, where both
//! sit far above the audio band. Computed in `f64`: `F` grows like `|x|` and the difference
//! of two large values must stay exact.

/// State of one ADAA stream (one channel at one rate).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Adaa {
    x1: f64,
    /// `F(x1)`.
    big_f1: f64,
    /// [`Self::tick_rectify`]'s second-order state: `x[n-2]` and the divided difference
    /// `D(x[n-1], x[n-2])` of `|x|`'s second antiderivative.
    x2: f64,
    d1: f64,
}

/// The rectifier `|tanh x|` and its first two antiderivatives (0 at 0):
/// `F1 = sign(x)·ln cosh x`, `F2 = G(|x|)` with `G(a) = ∫₀ᵃ ln cosh t dt
/// = a²/2 - a·ln 2 + (Li₂(-e^{-2a}) + π²/12) / 2`.
#[inline]
fn rect_f(x: f64) -> f64 {
    x.tanh().abs()
}
#[inline]
fn rect_f1(x: f64) -> f64 {
    ln_cosh(x).copysign(x)
}
#[inline]
fn rect_f2(x: f64) -> f64 {
    let a = x.abs();
    0.5 * a * a - a * std::f64::consts::LN_2
        + 0.5 * (li2_neg((-2.0 * a).exp()) + std::f64::consts::PI.powi(2) / 12.0)
}

/// `Li₂(-y)` for `y` in `0..=1`, through `Li₂(z) = -Li₂(z/(z-1)) - ln²(1-z)/2`: the series
/// then runs on `w = y/(1+y) <= 1/2` (2⁻ᵏ/k², 40 terms below 1e-15).
#[inline]
fn li2_neg(y: f64) -> f64 {
    let w = y / (1.0 + y);
    let (mut term, mut sum) = (1.0, 0.0);
    for k in 1..=40 {
        term *= w;
        let kf = f64::from(k);
        sum += term / (kf * kf);
        if term < 1e-17 {
            break;
        }
    }
    let l = y.ln_1p();
    -sum - 0.5 * l * l
}

/// `(F2(a) - F2(b)) / (a - b)`, or `F1` at the midpoint when `a ≈ b`.
#[inline]
fn rect_d(a: f64, b: f64) -> f64 {
    let d = a - b;
    if d.abs() > EPS2 {
        (rect_f2(a) - rect_f2(b)) / d
    } else {
        rect_f1(0.5 * (a + b))
    }
}

/// Second-order segments shorter than this use the fallbacks.
const EPS2: f64 = 1e-3;

/// Segments shorter than this use `f` at their midpoint.
const EPS: f64 = 1e-6;

impl Adaa {
    /// Back to rest (`x = 0`; every antiderivative used here is 0 at 0).
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    /// One sample through the curve `f` with antiderivative `big_f`.
    #[inline]
    pub(crate) fn tick(
        &mut self,
        x: f64,
        f: impl Fn(f64) -> f64,
        big_f: impl Fn(f64) -> f64,
    ) -> f64 {
        let fx = big_f(x);
        let d = x - self.x1;
        let y = if d.abs() > EPS {
            (fx - self.big_f1) / d
        } else {
            f(0.5 * (x + self.x1))
        };
        self.x1 = x;
        self.big_f1 = fx;
        y
    }
}

impl Adaa {
    /// One sample of the rectifier `|tanh(x)|` with **second-order** ADAA (Bilbao,
    /// Esqueda, Parker & Välimäki, "Antiderivative antialiasing for memoryless
    /// nonlinearities", 2017): its kink's harmonics fall only 12 dB/oct, too slowly for
    /// first-order ADAA. One sample late (first-order: half).
    #[inline]
    pub(crate) fn tick_rectify(&mut self, x: f64) -> f64 {
        let (x1, x2) = (self.x1, self.x2);
        let d0 = rect_d(x, x1);
        let y = if (x - x2).abs() > EPS2 {
            2.0 * (d0 - self.d1) / (x - x2)
        } else {
            // x ≈ x[n-2]: expand around their mean.
            let m = 0.5 * (x + x2);
            let delta = m - x1;
            if delta.abs() > EPS2 {
                2.0 / delta * (rect_f1(m) + (rect_f2(x1) - rect_f2(m)) / delta)
            } else {
                rect_f(0.5 * (m + x1))
            }
        };
        self.x2 = x1;
        self.d1 = d0;
        self.x1 = x;
        // Keeps the first-order state coherent (the rectifier's own antiderivative).
        self.big_f1 = rect_f1(x);
        y
    }
}

/// `ln(cosh(x))`, without overflow (the antiderivative of `tanh`).
#[inline]
pub(crate) fn ln_cosh(x: f64) -> f64 {
    let a = x.abs();
    a + (-2.0 * a).exp().ln_1p() - std::f64::consts::LN_2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_a_slow_signal_and_averages_a_jump() {
        let mut a = Adaa::default();
        // On a slow ramp through tanh, ADAA is tanh half a sample late.
        let mut prev = 0.0f64;
        for n in 1..200 {
            let x = n as f64 * 0.01;
            let y = a.tick(x, f64::tanh, ln_cosh);
            let want = (0.5 * (x + prev)).tanh();
            assert!((y - want).abs() < 1e-5, "{y} vs {want}");
            prev = x;
        }
        // Across a jump of a hard clip from -10 to 10: the mean of the clip over the segment.
        let clip = |x: f64| x.clamp(-1.0, 1.0);
        let big = |x: f64| {
            if x.abs() <= 1.0 {
                0.5 * x * x
            } else {
                x.abs() - 0.5
            }
        };
        let mut a = Adaa::default();
        a.tick(-10.0, clip, big);
        assert!(a.tick(10.0, clip, big).abs() < 1e-12);
    }

    #[test]
    fn ln_cosh_is_exact_and_does_not_overflow() {
        for x in [-3.0, -0.5, 0.0, 0.25, 2.0] {
            assert!((ln_cosh(x) - f64::cosh(x).ln()).abs() < 1e-12);
        }
        assert!((ln_cosh(1000.0) - (1000.0 - std::f64::consts::LN_2)).abs() < 1e-9);
    }
}

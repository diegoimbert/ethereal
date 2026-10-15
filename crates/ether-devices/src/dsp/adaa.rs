//! First-order antiderivative anti-aliasing (ADAA; Parker, Zavalishin & Le Bivic, DAFx
//! 2016) for static waveshapers (#520).
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
}

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

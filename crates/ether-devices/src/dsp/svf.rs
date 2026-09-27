//! Topology-preserving-transform (zero-delay-feedback) state-variable filter
//! (A. Simper, "Linear Trapezoidal Integrated SVF", Cytomic 2013).
//!
//! Every response is a mix of the input and the band/low outputs:
//! `y = m0·v0 + m1·v1 + m2·v2`. Bypass is `(m0, m1, m2) = (1, 0, 0)`, so switching a band
//! on/off or changing its type is a smooth glide of five numbers `(g, k, m0, m1, m2)`
//! towards their targets. The TPT structure stays stable and click-free while `g`/`k`
//! move every sample (unlike direct-form biquads).

use std::f64::consts::PI;

/// Filter response of one band.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shape {
    LowCut,
    LowShelf,
    Bell,
    Notch,
    HighShelf,
    HighCut,
}

/// SVF coefficients: `g = tan(π·fc/fs)` (warped), damping `k = 1/Q`, output mix `m*`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Coefs {
    pub g: f64,
    pub k: f64,
    pub m0: f64,
    pub m1: f64,
    pub m2: f64,
}

impl Coefs {
    /// Pass-through, with a harmless `g`/`k` so the state keeps running.
    pub(crate) const BYPASS: Self = Self {
        g: 0.1,
        k: 1.0,
        m0: 1.0,
        m1: 0.0,
        m2: 0.0,
    };

    /// These coefficients switched to pass-through: same `g`/`k` (so an on/off glide only
    /// fades the output mix, without sweeping the filter), `m = (1, 0, 0)`.
    pub(crate) fn bypassed(self) -> Self {
        Self {
            m0: 1.0,
            m1: 0.0,
            m2: 0.0,
            ..self
        }
    }

    /// Output is exactly the input.
    pub(crate) fn is_bypass(&self) -> bool {
        self.m0 == 1.0 && self.m1 == 0.0 && self.m2 == 0.0
    }

    /// Coefficients for `shape` at `freq` Hz, `gain_db` (shelves/bell) and `q`: the shared
    /// definition in `ether_protocol::eq_response::svf_coefs` (v0.2), which the UI's EQ curve
    /// mirrors, so the drawn response is the device's response.
    pub(crate) fn new(shape: Shape, freq: f64, gain_db: f64, q: f64, sample_rate: f64) -> Self {
        use ether_core::protocol::eq_response::{EqShape, svf_coefs};
        let shape = match shape {
            Shape::LowCut => EqShape::LowCut,
            Shape::LowShelf => EqShape::LowShelf,
            Shape::Bell => EqShape::Bell,
            Shape::Notch => EqShape::Notch,
            Shape::HighShelf => EqShape::HighShelf,
            Shape::HighCut => EqShape::HighCut,
        };
        let c = svf_coefs(shape, freq, gain_db, q, sample_rate);
        Self {
            g: c.g,
            k: c.k,
            m0: c.m0,
            m1: c.m1,
            m2: c.m2,
        }
    }

    /// One-pole glide of every coefficient towards `target` (`coef` = retained fraction).
    /// Snaps when close so a settled filter uses exactly the target coefficients.
    #[inline]
    pub(crate) fn glide(&mut self, target: &Self, coef: f64) -> bool {
        let step = |c: &mut f64, t: f64| {
            let d = t - *c;
            if d.abs() < 1e-9 {
                *c = t;
                false
            } else {
                *c = t - d * coef;
                true
            }
        };
        // `g` glides in the log domain (frequency moves at a constant rate in octaves).
        let g_moving = if (target.g - self.g).abs() < 1e-9 * target.g {
            self.g = target.g;
            false
        } else {
            let (lt, lc) = (target.g.ln(), self.g.ln());
            self.g = (lt + (lc - lt) * coef).exp();
            true
        };
        // Evaluate all (no short-circuit).
        let a = g_moving;
        let b = step(&mut self.k, target.k);
        let c = step(&mut self.m0, target.m0);
        let d = step(&mut self.m1, target.m1);
        let e = step(&mut self.m2, target.m2);
        a | b | c | d | e
    }

    /// Complex magnitude response at `freq` Hz (for tests and a future curve display).
    pub(crate) fn magnitude(&self, freq: f64, sample_rate: f64) -> f64 {
        // Analog prototype evaluated at the prewarped frequency: bilinear transform maps
        // s = j·tan(π f / fs) / g in normalized units.
        let w = (PI * freq / sample_rate).tan() / self.g;
        // H(s) = m0 + m1·(s/(s²+ks+1)) + m2·(1/(s²+ks+1)), s = jw
        let (dr, di) = (1.0 - w * w, self.k * w);
        let den = dr * dr + di * di;
        // s/(den): (jw)(dr - j di)/den = (w·di + j w·dr)/den
        let bp = (w * di / den, w * dr / den);
        let lp = (dr / den, -di / den);
        let re = self.m0 + self.m1 * bp.0 + self.m2 * lp.0;
        let im = self.m1 * bp.1 + self.m2 * lp.1;
        (re * re + im * im).sqrt()
    }
}

/// Per-channel SVF state.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Svf {
    ic1: f64,
    ic2: f64,
}

impl Svf {
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    #[inline]
    pub(crate) fn tick(&mut self, v0: f64, c: &Coefs) -> f64 {
        let a1 = 1.0 / (1.0 + c.g * (c.g + c.k));
        let a2 = c.g * a1;
        let a3 = c.g * a2;
        let v3 = v0 - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = super::flush(2.0 * v1 - self.ic1);
        self.ic2 = super::flush(2.0 * v2 - self.ic2);
        c.m0 * v0 + c.m1 * v1 + c.m2 * v2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f64 = 48_000.0;

    fn db(x: f64) -> f64 {
        20.0 * x.log10()
    }

    #[test]
    fn analytic_responses() {
        let bell = Coefs::new(Shape::Bell, 1000.0, 12.0, 1.0, SR);
        assert!((db(bell.magnitude(1000.0, SR)) - 12.0).abs() < 1e-6);
        assert!(db(bell.magnitude(20.0, SR)).abs() < 0.1);
        let lc = Coefs::new(
            Shape::LowCut,
            1000.0,
            0.0,
            std::f64::consts::FRAC_1_SQRT_2,
            SR,
        );
        assert!((db(lc.magnitude(1000.0, SR)) + 3.0103).abs() < 1e-3);
        let ls = Coefs::new(Shape::LowShelf, 200.0, 6.0, 0.707, SR);
        assert!((db(ls.magnitude(10.0, SR)) - 6.0).abs() < 0.05);
        assert!(db(ls.magnitude(15_000.0, SR)).abs() < 0.05);
        let hs = Coefs::new(Shape::HighShelf, 2000.0, -6.0, 0.707, SR);
        assert!((db(hs.magnitude(20_000.0, SR)) + 6.0).abs() < 0.05);
        assert!(db(hs.magnitude(20.0, SR)).abs() < 0.05);
        // Zero gain bells/shelves are exact pass-throughs.
        for s in [Shape::Bell, Shape::LowShelf, Shape::HighShelf] {
            let c = Coefs::new(s, 500.0, 0.0, 2.0, SR);
            assert_eq!((c.m0, c.m1, c.m2), (1.0, 0.0, 0.0), "{s:?}");
        }
    }

    /// The device's filter (time domain) matches the shared response the UI draws
    /// (`ether_protocol::eq_response::svf_magnitude`) for every shape.
    #[test]
    fn device_filter_matches_the_shared_response() {
        use ether_core::protocol::eq_response::{SvfCoefs, svf_magnitude};
        for shape in [
            Shape::LowCut,
            Shape::LowShelf,
            Shape::Bell,
            Shape::Notch,
            Shape::HighShelf,
            Shape::HighCut,
        ] {
            let c = Coefs::new(shape, 1000.0, 6.0, 1.5, SR);
            let shared = SvfCoefs {
                g: c.g,
                k: c.k,
                m0: c.m0,
                m1: c.m1,
                m2: c.m2,
            };
            for f in [100.0, 700.0, 3000.0] {
                // Steady-state amplitude of a sine through the actual filter.
                let mut svf = Svf::default();
                let n = 48_000;
                let (mut sum, mut count) = (0.0, 0.0);
                for i in 0..n {
                    let x = (2.0 * PI * f * i as f64 / SR).sin();
                    let y = svf.tick(x, &c);
                    if i >= n / 2 {
                        sum += y * y;
                        count += 1.0;
                    }
                }
                // Sine amplitude from the RMS (the sample grid misses true peaks).
                let peak = (2.0 * sum / count).sqrt();
                let expect = svf_magnitude(&shared, f, SR);
                assert!(
                    (db(peak) - db(expect)).abs() < 0.05,
                    "{shape:?} at {f} Hz: {} vs {}",
                    db(peak),
                    db(expect)
                );
            }
        }
    }
}

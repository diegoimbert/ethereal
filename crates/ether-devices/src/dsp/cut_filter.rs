//! Stereo low-cut + high-cut pair: the tone filters of the delay's
//! feedback path and the reverb's input. Two 12 dB/oct TPT state-variable stages
//! (Butterworth Q) whose coefficients glide on change. A stage set to its "off" extreme
//! (low cut at or under [`LOW_OFF_HZ`], high cut at or over [`HIGH_OFF_HZ`]) fades to
//! pass-through and, once there, is skipped: with both off the signal is untouched
//! (bit-exact with the device before the filters existed).

use super::svf::{Coefs, Shape, Svf};

/// A low cut at or under this frequency (Hz) is off.
pub(crate) const LOW_OFF_HZ: f64 = 20.0;
/// A high cut at or over this frequency (Hz) is off.
pub(crate) const HIGH_OFF_HZ: f64 = 20_000.0;
const Q: f64 = std::f64::consts::FRAC_1_SQRT_2;
/// Coefficient glide time constant (ms).
const GLIDE_MS: f32 = 20.0;

#[derive(Clone, Copy, Debug)]
struct Stage {
    cur: Coefs,
    target: Coefs,
    state: [Svf; 2],
    /// `cur` is still gliding to `target`.
    moving: bool,
    /// Settled at pass-through (skipped).
    idle: bool,
}

impl Stage {
    const OFF: Self = Self {
        cur: Coefs::BYPASS,
        target: Coefs::BYPASS,
        state: [Svf::ZERO; 2],
        moving: false,
        idle: true,
    };

    fn set(&mut self, target: Coefs, smooth: bool) {
        self.target = target;
        if smooth {
            self.moving = self.cur != target;
        } else {
            self.cur = target;
            self.moving = false;
            if target.is_bypass() {
                self.state.iter_mut().for_each(Svf::reset);
            }
        }
        self.idle = !self.moving && self.cur.is_bypass();
    }

    #[inline]
    fn glide(&mut self, coef: f64) {
        if self.moving {
            self.moving = self.cur.glide(&self.target, coef);
            if !self.moving {
                self.cur = self.target;
                self.idle = self.cur.is_bypass();
            }
        }
    }
}

/// The filter pair (see the module docs).
#[derive(Clone, Copy, Debug)]
pub(crate) struct CutFilter {
    low: Stage,
    high: Stage,
    glide: f64,
    sample_rate: f64,
}

impl Default for CutFilter {
    fn default() -> Self {
        Self {
            low: Stage::OFF,
            high: Stage::OFF,
            glide: 0.0,
            sample_rate: 48_000.0,
        }
    }
}

impl CutFilter {
    /// Sample rate (call from `prepare`, then [`Self::set`] the cutoffs again).
    pub(crate) fn prepare(&mut self, sample_rate: f32) {
        self.sample_rate = f64::from(sample_rate.max(1.0));
        self.glide = f64::from(crate::util::tau_coef(GLIDE_MS, sample_rate));
    }

    /// Cutoffs in Hz (`low` <= [`LOW_OFF_HZ`] / `high` >= [`HIGH_OFF_HZ`] = that stage off);
    /// `smooth` glides there, else jumps.
    pub(crate) fn set(&mut self, low: f64, high: f64, smooth: bool) {
        let sr = self.sample_rate;
        // An "off" stage keeps its frequency, so switching it off fades the mix only.
        let lc = Coefs::new(Shape::LowCut, low.max(LOW_OFF_HZ), 0.0, Q, sr);
        let hc = Coefs::new(Shape::HighCut, high.min(HIGH_OFF_HZ), 0.0, Q, sr);
        self.low
            .set(if low <= LOW_OFF_HZ { lc.bypassed() } else { lc }, smooth);
        self.high.set(
            if high >= HIGH_OFF_HZ {
                hc.bypassed()
            } else {
                hc
            },
            smooth,
        );
    }

    /// Both stages off and settled (the signal passes untouched).
    #[inline]
    pub(crate) fn is_bypass(&self) -> bool {
        self.low.idle && self.high.idle
    }

    /// Advance the coefficient glides by one sample (once per frame, before the ticks).
    #[inline]
    pub(crate) fn glide(&mut self) {
        let g = self.glide;
        self.low.glide(g);
        self.high.glide(g);
    }

    /// Filter one sample of channel `ch` (0 or 1).
    #[inline]
    pub(crate) fn tick(&mut self, ch: usize, x: f32) -> f32 {
        let mut v = f64::from(x);
        for s in [&mut self.low, &mut self.high] {
            if !s.idle {
                v = s.state[ch].tick(v, &s.cur);
            }
        }
        v as f32
    }

    pub(crate) fn reset(&mut self) {
        for s in [&mut self.low, &mut self.high] {
            s.state.iter_mut().for_each(Svf::reset);
            s.cur = s.target;
            s.moving = false;
            s.idle = s.cur.is_bypass();
        }
    }

    /// Magnitude (linear) at `freq` Hz of the target response (tests).
    #[cfg(test)]
    pub(crate) fn magnitude(&self, freq: f64) -> f64 {
        self.low.target.magnitude(freq, self.sample_rate)
            * self.high.target.magnitude(freq, self.sample_rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_is_exact_and_cuts_cut() {
        let mut f = CutFilter::default();
        f.prepare(48_000.0);
        f.set(20.0, 20_000.0, false);
        assert!(f.is_bypass());
        for x in [0.1f32, -0.7, 1e-30, 0.33] {
            assert_eq!(f.tick(0, x).to_bits(), x.to_bits());
        }
        f.set(500.0, 2_000.0, false);
        assert!(!f.is_bypass());
        assert!(f.magnitude(50.0) < 0.02);
        assert!(f.magnitude(20_000.0) < 0.02);
        assert!((f.magnitude(1_000.0) - 1.0).abs() < 0.25);
        // Gliding back to off settles to the exact bypass.
        f.set(20.0, 20_000.0, true);
        for _ in 0..48_000 {
            f.glide();
            f.tick(0, 0.0);
        }
        assert!(f.is_bypass());
    }
}

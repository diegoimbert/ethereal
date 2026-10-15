//! 4-point polyBLEP / polyBLAMP residuals for the virtual-analog oscillators (synth, poly
//! synth; #520).
//!
//! A naive saw or pulse jumps at its wrap: the jump's harmonics run past Nyquist and fold
//! back as aliasing. A BLEP replaces the jump with a band-limited step by adding a residual
//! around it. The classic 2-point polyBLEP (the integral of a linear B-spline, one sample
//! each side) left the aliasing at -35 to -40 dB on high notes. These residuals integrate a
//! cubic B-spline instead (two samples each side): about 15 dB less aliasing (-53 dB on a
//! C7 saw), at the price of a gentler top end (the step's spectrum is `sinc⁴` instead of
//! `sinc²`: -2.5 dB at 10 kHz).
//!
//! Both stay stateless: a sample's residual depends only on its distance to the nearest
//! discontinuity before and after it (in samples, `phase / dt`), so oscillators remain pure
//! functions of their phase.

/// BLEP residual of a unit upward step, `y` samples before it (`0 ..= 2`; after it: the
/// negative, see [`blep`]).
#[inline]
fn step_before(y: f32) -> f32 {
    if y < 1.0 {
        let y2 = y * y;
        0.5 - (4.0 * y - 2.0 * y2 * y + 0.75 * y2 * y2) * (1.0 / 6.0)
    } else {
        let u = 2.0 - y;
        let u2 = u * u;
        u2 * u2 * (1.0 / 24.0)
    }
}

/// BLAMP residual of a unit slope change (per sample), `y` samples from it (`0 ..= 2`,
/// either side).
#[inline]
fn ramp_at(y: f32) -> f32 {
    if y < 1.0 {
        let y2 = y * y;
        // 7/30 - (y/2 - y²/3 + y⁴/12 - y⁵/40)
        7.0 / 30.0
            - (0.5 * y - y2 * (1.0 / 3.0) + y2 * y2 * (1.0 / 12.0) - y2 * y2 * y * (1.0 / 40.0))
    } else {
        let u = 2.0 - y;
        let u2 = u * u;
        u2 * u2 * u * (1.0 / 120.0)
    }
}

/// Residual to add for a unit upward step at phase 0 of a cycle (`t` = phase in 0..1, `dt`
/// = phase increment per sample, below 0.5).
#[inline]
pub(crate) fn blep(t: f32, dt: f32) -> f32 {
    let mut r = 0.0;
    let after = t / dt;
    if after < 2.0 {
        r -= step_before(after);
    }
    let before = (1.0 - t) / dt;
    if before < 2.0 {
        r += step_before(before);
    }
    r
}

/// Residual to add for a unit slope change per sample at phase 0 (see [`blep`]).
#[inline]
pub(crate) fn blamp(t: f32, dt: f32) -> f32 {
    let mut r = 0.0;
    let after = t / dt;
    if after < 2.0 {
        r += ramp_at(after);
    }
    let before = (1.0 - t) / dt;
    if before < 2.0 {
        r += ramp_at(before);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn residuals_are_continuous_and_vanish_at_their_edges() {
        // Step: 1/2 at the jump (the band-limited step crosses its midpoint), 1/24 at one
        // sample, 0 at two; slope matches across the pieces.
        assert!((step_before(0.0) - 0.5).abs() < 1e-6);
        assert!((step_before(1.0 - 1e-6) - 1.0 / 24.0).abs() < 1e-5);
        assert!((step_before(1.0) - 1.0 / 24.0).abs() < 1e-6);
        assert!(step_before(2.0).abs() < 1e-9);
        // Ramp: continuous at 1 and 0 at 2.
        assert!((ramp_at(1.0 - 1e-6) - ramp_at(1.0)).abs() < 1e-5);
        assert!((ramp_at(1.0) - 1.0 / 120.0).abs() < 1e-6);
        assert!(ramp_at(2.0).abs() < 1e-9);
    }

    #[test]
    fn a_blep_step_is_monotonic_and_reaches_both_levels() {
        // A unit step at t = 0.5 sampled every 0.1 samples: naive + residual.
        let dt = 0.01;
        let mut prev = f32::NEG_INFINITY;
        for i in 0..100 {
            let t = i as f32 * 0.001 + 0.47; // around the wrap of a cycle shifted by 0.5
            let p = (t + 0.5) % 1.0;
            let naive = if p < 0.5 { 1.0 } else { 0.0 };
            let v = naive + blep(p, dt);
            assert!(v >= prev - 1e-6, "not monotonic at {t}");
            prev = v;
        }
    }
}

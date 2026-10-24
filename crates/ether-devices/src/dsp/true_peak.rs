//! True-peak detection (ITU-R BS.1770-4 Annex 2): the signal's peak between its samples,
//! found by oversampling with a polyphase FIR interpolator and taking the largest magnitude
//! of every interpolated value.
//!
//! - Oversampling: 4x below 96 kHz, 2x below 192 kHz, none above (the Annex's
//!   recommendation: the interpolated rate reaches at least 192 kHz).
//! - Interpolator: a Kaiser-windowed sinc ([`TAPS`] taps per phase, beta [`BETA`]) cut at
//!   the input's Nyquist frequency, each phase normalized to unity DC gain. Phase 0 is the
//!   input sample itself (an exact delay), the other phases the in-between values. A sine
//!   reads within 0.1 dB of the bound sampling at the oversampled rate allows
//!   (`cos(pi f / (factor fs))`, under 0.6 dB at 20 kHz, as the Annex expects; the tests).
//! - The output lags the input by [`TruePeak::delay`] samples (half the filter span); a
//!   meter can ignore it, a limiter folds it into its look-ahead.
//!
//! Real-time: the coefficients and history are fixed-size arrays (no allocation at all).

/// Taps per phase (even: the filter's centre falls between two input samples).
pub(crate) const TAPS: usize = 16;
/// Largest oversampling factor.
pub(crate) const MAX_FACTOR: usize = 4;
/// Kaiser window shape.
const BETA: f64 = 7.0;

/// Oversampling factor at `sample_rate`.
pub(crate) fn factor_for(sample_rate: f32) -> usize {
    if sample_rate < 96_000.0 {
        4
    } else if sample_rate < 192_000.0 {
        2
    } else {
        1
    }
}

fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let q = x * x / 4.0;
    for k in 1..64 {
        term *= q / (k * k) as f64;
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

/// A one-channel true-peak detector.
#[derive(Clone)]
pub(crate) struct TruePeak {
    factor: usize,
    /// `coefs[k][j]`: phase `k` (`k / factor` of a sample after the centre), tap `j`
    /// (newest input first).
    coefs: [[f32; TAPS]; MAX_FACTOR],
    /// Input history, newest at `hist[at]`, written twice (`at` and `at + TAPS`) so a read
    /// of `TAPS` contiguous values never wraps.
    hist: [f32; 2 * TAPS],
    at: usize,
}

impl TruePeak {
    /// Non-RT (cheap, but computes Bessel functions). A detector for `sample_rate`.
    pub(crate) fn new(sample_rate: f32) -> Self {
        let factor = factor_for(sample_rate);
        let mut coefs = [[0.0f32; TAPS]; MAX_FACTOR];
        let half = TAPS as f64 / 2.0;
        let norm = bessel_i0(BETA);
        // History index j holds x[n - j]. Phase k > 0 interpolates `k / factor` of a sample
        // after x[n - TAPS/2], i.e. at `centre = TAPS/2 - 1 + (1 - k / factor)` taps back;
        // phase 0 is tap TAPS/2 - 1 itself (the input sample, delayed).
        for (k, phase) in coefs.iter_mut().enumerate().take(factor) {
            if k == 0 {
                phase[TAPS / 2 - 1] = 1.0;
                continue;
            }
            let centre = half - 1.0 + (1.0 - k as f64 / factor as f64);
            let mut row = [0.0f64; TAPS];
            for (j, c) in row.iter_mut().enumerate() {
                let t = j as f64 - centre;
                let sinc = if t.abs() < 1e-12 {
                    1.0
                } else {
                    (std::f64::consts::PI * t).sin() / (std::f64::consts::PI * t)
                };
                let u = t / half;
                let w = if u.abs() < 1.0 {
                    bessel_i0(BETA * (1.0 - u * u).sqrt()) / norm
                } else {
                    0.0
                };
                *c = sinc * w;
            }
            let sum: f64 = row.iter().sum();
            for (c, r) in phase.iter_mut().zip(row) {
                *c = (r / sum) as f32;
            }
        }
        Self {
            factor,
            coefs,
            hist: [0.0; 2 * TAPS],
            at: 0,
        }
    }

    /// The oversampling factor.
    #[allow(dead_code)]
    pub(crate) fn factor(&self) -> usize {
        self.factor
    }

    /// Input samples between a sample going in and its interpolated neighbourhood coming
    /// out of [`Self::push`].
    #[allow(dead_code)]
    pub(crate) fn delay(&self) -> usize {
        TAPS / 2
    }

    pub(crate) fn reset(&mut self) {
        self.hist = [0.0; 2 * TAPS];
        self.at = 0;
    }

    /// RT. Feed one sample; returns the largest magnitude among the interpolated values
    /// between the samples [`Self::delay`] and `delay - 1` back (the input sample included).
    #[inline]
    pub(crate) fn push(&mut self, x: f32) -> f32 {
        let x = if x.is_finite() { x } else { 0.0 };
        self.at = if self.at == 0 { TAPS - 1 } else { self.at - 1 };
        self.hist[self.at] = x;
        self.hist[self.at + TAPS] = x;
        let h = &self.hist[self.at..self.at + TAPS];
        let mut peak = h[TAPS / 2 - 1].abs();
        for phase in &self.coefs[1..self.factor] {
            let mut acc = 0.0f32;
            for (c, v) in phase.iter().zip(h) {
                acc += c * v;
            }
            peak = peak.max(acc.abs());
        }
        peak
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measure(sr: f32, hz: f32, phase: f32, amp: f32) -> (f32, f32) {
        let mut tp = TruePeak::new(sr);
        let mut sample_peak = 0.0f32;
        let mut true_peak = 0.0f32;
        let n = (sr * 0.5) as usize;
        for i in 0..n {
            let x = amp * (std::f32::consts::TAU * hz * i as f32 / sr + phase).sin();
            sample_peak = sample_peak.max(x.abs());
            let p = tp.push(x);
            if i > 4 * TAPS {
                true_peak = true_peak.max(p);
            }
        }
        (sample_peak, true_peak)
    }

    #[test]
    fn a_quarter_rate_sine_between_its_samples_reads_its_true_peak() {
        // fs/4 at 45 degrees: every sample is at ±0.707, the waveform reaches 1.
        let (sp, tp) = measure(48_000.0, 12_000.0, std::f32::consts::FRAC_PI_4, 1.0);
        assert!((20.0 * sp.log10() + 3.01).abs() < 0.05, "sample peak {sp}");
        assert!((20.0 * tp.log10()).abs() < 0.25, "true peak {tp}");
    }

    #[test]
    fn under_read_stays_small_up_to_the_top_of_the_band() {
        for sr in [44_100.0f32, 48_000.0, 96_000.0, 192_000.0] {
            for frac in [0.05f32, 0.2, 0.33, 0.45] {
                let hz = frac * sr;
                if hz > 20_000.0 {
                    continue;
                }
                let os = sr * factor_for(sr) as f32;
                let bound = 20.0 * (std::f32::consts::PI * hz / os).cos().log10();
                for phase in [0.0f32, 0.3, 0.77, 1.2] {
                    let (_, tp) = measure(sr, hz, phase, 0.5);
                    let err = 20.0 * (tp / 0.5).log10();
                    assert!(
                        err < 0.1 && err > bound - 0.1,
                        "{sr} Hz rate, {hz} Hz tone: {err:.3} dB (bound {bound:.3})"
                    );
                }
            }
        }
    }

    #[test]
    fn factor_follows_the_sample_rate() {
        assert_eq!(factor_for(44_100.0), 4);
        assert_eq!(factor_for(96_000.0), 2);
        assert_eq!(factor_for(192_000.0), 1);
    }
}

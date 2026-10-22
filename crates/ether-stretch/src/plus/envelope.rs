//! Spectral envelope by the "true envelope" method (Imai & Abe 1979; Röbel & Rodet,
//! "Efficient spectral envelope estimation and its application to pitch shifting and
//! envelope preservation", DAFx 2005): cepstral smoothing iterated so the smooth curve
//! rides on the harmonic peaks instead of averaging peaks and valleys.
//!
//! `A₀ = ln|X|`; `Cᵢ = lifter(Aᵢ)` (keep quefrencies below the cutoff); `Aᵢ₊₁ = max(A₀, Cᵢ)`,
//! until `A₀` exceeds `Cᵢ` by less than [`CONVERGED_DB`] anywhere (or after the iteration
//! budget). The cutoff quefrency sets the detail: a harmonic spectrum with fundamental `f0`
//! has its harmonics at quefrency `sr/f0`, so the cutoff must stay below that.

use std::sync::Arc;

use realfft::num_complex::Complex32 as C;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

/// Peaks above the envelope by less than this count as converged.
const CONVERGED_DB: f32 = 1.5;
/// Magnitudes floor relative to the frame's largest (dB).
const FLOOR_DB: f32 = -90.0;

pub(crate) struct EnvelopeEstimator {
    n: usize,
    fwd: Arc<dyn RealToComplex<f32>>,
    inv: Arc<dyn ComplexToReal<f32>>,
    spec: Vec<C>,
    cep: Vec<f32>,
    scratch: Vec<C>,
    a0: Vec<f32>,
    cur: Vec<f32>,
}

impl EnvelopeEstimator {
    /// Non-RT. For spectra of `n / 2 + 1` bins (`n` even).
    pub(crate) fn new(n: usize) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fwd = planner.plan_fft_forward(n);
        let inv = planner.plan_fft_inverse(n);
        let scratch_len = fwd.get_scratch_len().max(inv.get_scratch_len());
        let bins = n / 2 + 1;
        Self {
            n,
            fwd,
            inv,
            spec: vec![C::default(); bins],
            cep: vec![0.0; n],
            scratch: vec![C::default(); scratch_len],
            a0: vec![0.0; bins],
            cur: vec![0.0; bins],
        }
    }

    /// RT. Natural-log envelope of `mag` (`n/2 + 1` bins) into `out`, with cepstral cutoff
    /// `cutoff` (samples of quefrency) and at most `iters` iterations (1 = plain cepstral
    /// smoothing).
    pub(crate) fn estimate(&mut self, mag: &[f32], cutoff: usize, iters: usize, out: &mut [f32]) {
        let bins = self.n / 2 + 1;
        let max = mag[..bins].iter().fold(0.0f32, |m, v| m.max(*v));
        if max <= 0.0 || !max.is_finite() {
            out[..bins].fill(FLOOR_DB / 20.0 * std::f32::consts::LN_10);
            return;
        }
        let floor = max * 10f32.powf(FLOOR_DB / 20.0);
        for (a, m) in self.a0.iter_mut().zip(&mag[..bins]) {
            *a = m.max(floor).ln();
        }
        self.cur.copy_from_slice(&self.a0);
        let cutoff = cutoff.clamp(1, self.n / 2 - 1);
        let tol = CONVERGED_DB / 20.0 * std::f32::consts::LN_10;
        for i in 0..iters.max(1) {
            self.lifter(cutoff, out);
            let mut worst = 0.0f32;
            for ((c, a), o) in self.cur.iter_mut().zip(&self.a0).zip(&out[..bins]) {
                worst = worst.max(a - o);
                *c = a.max(*o);
            }
            if worst < tol || i + 1 == iters {
                break;
            }
        }
    }

    /// `out` = cepstral smoothing of `self.cur`.
    fn lifter(&mut self, cutoff: usize, out: &mut [f32]) {
        let n = self.n;
        for (s, a) in self.spec.iter_mut().zip(&self.cur) {
            *s = C::new(*a, 0.0);
        }
        let _ = self
            .inv
            .process_with_scratch(&mut self.spec, &mut self.cep, &mut self.scratch);
        // Real, even cepstrum: keep |q| < cutoff, a short taper at the edge (fewer ripples).
        let scale = 1.0 / n as f32;
        let taper = (cutoff / 4).max(1);
        for q in 0..=n / 2 {
            let w = if q < cutoff - taper.min(cutoff) {
                1.0
            } else if q < cutoff {
                let x = (q + taper - cutoff) as f32 / taper as f32;
                0.5 + 0.5 * (std::f32::consts::PI * x).cos()
            } else {
                0.0
            };
            self.cep[q] *= w * scale;
            if q > 0 && q < n / 2 {
                self.cep[n - q] *= w * scale;
            }
        }
        let _ = self
            .fwd
            .process_with_scratch(&mut self.cep, &mut self.spec, &mut self.scratch);
        for (o, s) in out.iter_mut().zip(&self.spec) {
            *o = s.re;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Log envelope with two "formants".
    fn formants(f: f32) -> f32 {
        let peak = |c: f32, bw: f32, db: f32| db * (-((f - c) / bw).powi(2)).exp();
        let db = -30.0 + peak(700.0, 150.0, 30.0) + peak(1800.0, 250.0, 22.0) - 12.0 * (f / 8000.0);
        db / 20.0 * std::f32::consts::LN_10
    }

    /// A harmonic magnitude spectrum (f0 = 180 Hz at 48 kHz, 2048-point) shaped by
    /// [`formants`]: the true envelope finds the shaping curve within a few dB at the
    /// harmonics, the plain cepstrum falls well below the peaks.
    #[test]
    fn true_envelope_rides_on_harmonic_peaks() {
        let n = 2048;
        let sr = 48_000.0;
        let f0 = 180.0;
        let bins = n / 2 + 1;
        let bin_hz = sr / n as f32;
        let mag: Vec<f32> = (0..bins)
            .map(|k| {
                let f = k as f32 * bin_hz;
                let h = (f / f0).round().max(1.0);
                // Hann main lobe around each harmonic.
                let d = (f - h * f0) / bin_hz;
                let lobe = if d.abs() < 2.0 {
                    0.5 + 0.5 * (std::f32::consts::PI * d / 2.0).cos()
                } else {
                    1e-4
                };
                formants(h * f0).exp() * lobe
            })
            .collect();
        let cutoff = (sr / f0 * 0.5) as usize;
        let mut est = EnvelopeEstimator::new(n);
        let mut te = vec![0.0; bins];
        est.estimate(&mag, cutoff, 40, &mut te);
        let mut cep = vec![0.0; bins];
        est.estimate(&mag, cutoff, 1, &mut cep);
        let db = |x: f32| x * 20.0 / std::f32::consts::LN_10;
        let (mut err_te, mut err_cep, mut count) = (0.0f32, 0.0f32, 0);
        for h in 2..40 {
            let k = (h as f32 * f0 / bin_hz).round() as usize;
            let want = db(formants(k as f32 * bin_hz));
            err_te += (db(te[k]) - want).abs();
            err_cep += (db(cep[k]) - want).abs();
            count += 1;
        }
        let (err_te, err_cep) = (err_te / count as f32, err_cep / count as f32);
        assert!(err_te < 3.0, "true envelope mean error {err_te} dB");
        assert!(
            err_cep > err_te + 3.0,
            "cepstrum {err_cep} dB vs {err_te} dB"
        );
    }

    /// A lone partial (the sparsest spectrum, where the iteration converges slowly): the
    /// envelope stays between the floor and the partial (no ringing above the source).
    #[test]
    fn lone_partial_stays_bounded() {
        let n = 2048;
        let bins = n / 2 + 1;
        let mag: Vec<f32> = (0..bins)
            .map(|k| {
                let d = (k as f32 - 9.4) / 2.0;
                if d.abs() < 1.0 {
                    0.5 + 0.5 * (std::f32::consts::PI * d).cos()
                } else {
                    0.0
                }
            })
            .collect();
        let mut est = EnvelopeEstimator::new(n);
        let mut env = vec![0.0; bins];
        est.estimate(&mag, 64, 8, &mut env);
        let db = |x: f32| x * 20.0 / std::f32::consts::LN_10;
        let peak = 0.0f32; // the partial's own level (magnitude 1)
        let top = env.iter().fold(f32::MIN, |m, v| m.max(db(*v)));
        assert!(
            top < peak + 1.0 && top.is_finite(),
            "rings above the source: {top} dB"
        );
        // Ripples below the floor are harmless; it stays bounded.
        assert!(
            env.iter().all(|v| db(*v) > FLOOR_DB - 20.0),
            "far below the floor"
        );
    }

    #[test]
    fn silence_gives_floor() {
        let mut est = EnvelopeEstimator::new(64);
        let mut out = vec![1.0; 33];
        est.estimate(&[0.0; 33], 8, 4, &mut out);
        assert!(out.iter().all(|v| *v < -5.0));
    }
}

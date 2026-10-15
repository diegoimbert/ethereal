//! The "modified" (half-bin shifted) real FFT of Signalsmith Linear (`fft.h`,
//! `RealFFT<Sample, false, true>`), on top of a `rustfft` complex FFT of half the size.
//!
//! Bin `k` (`0..N/2`) is at frequency `(k + 0.5)/N`; both directions are unnormalised, so
//! `ifft(fft(x)) = N·x`. Allocation happens in [`ModifiedRealFft::new`] only.

use std::sync::Arc;

use rustfft::num_complex::Complex32 as C;
use rustfft::{Fft, FftPlanner};

/// `SplitFFT::fastSizeAbove`: the smallest `2^k·m` (`m` in 1..=8, not 7) at least `size`.
fn complex_fast_size_above(size: usize) -> usize {
    let mut pow2 = 1;
    while pow2 < 16 && pow2 < size {
        pow2 *= 2;
    }
    while pow2 * 8 < size {
        pow2 *= 2;
    }
    let mut multiple = size.div_ceil(pow2);
    if multiple == 7 {
        multiple += 1;
    }
    multiple * pow2
}

/// `RealFFT::fastSizeAbove`.
pub(crate) fn fast_size_above(size: usize) -> usize {
    complex_fast_size_above(size.div_ceil(2)) * 2
}

pub(crate) struct ModifiedRealFft {
    half: usize,
    fwd: Arc<dyn Fft<f32>>,
    inv: Arc<dyn Fft<f32>>,
    buf: Vec<C>,
    scratch: Vec<C>,
    twiddles: Vec<C>,
    half_bin_twists: Vec<C>,
}

impl ModifiedRealFft {
    /// Non-RT. `size` must be even.
    pub(crate) fn new(size: usize) -> Self {
        let half = size / 2;
        let mut planner = FftPlanner::<f32>::new();
        let fwd = planner.plan_fft_forward(half);
        let inv = planner.plan_fft_inverse(half);
        let scratch_len = fwd
            .get_inplace_scratch_len()
            .max(inv.get_inplace_scratch_len());
        let n = size as f64;
        let twiddles = (0..half / 2 + 1)
            .map(|i| {
                let phase = (i as f64 + 0.5) * (-2.0 * std::f64::consts::PI / n)
                    - std::f64::consts::FRAC_PI_2;
                C::from_polar(1.0, phase as f32)
            })
            .collect();
        let half_bin_twists = (0..half)
            .map(|i| {
                let phase = -2.0 * std::f64::consts::PI * i as f64 / n;
                C::from_polar(1.0, phase as f32)
            })
            .collect();
        Self {
            half,
            fwd,
            inv,
            buf: vec![C::default(); half],
            scratch: vec![C::default(); scratch_len],
            twiddles,
            half_bin_twists,
        }
    }

    /// RT. `time` has `N` samples, `freq` `N/2` bins.
    pub(crate) fn fft(&mut self, time: &[f32], freq: &mut [C]) {
        let h = self.half;
        for (i, (b, tw)) in self.buf.iter_mut().zip(&self.half_bin_twists).enumerate() {
            *b = C::new(time[2 * i], time[2 * i + 1]) * tw;
        }
        self.fwd
            .process_with_scratch(&mut self.buf, &mut self.scratch);
        for i in 0..(h / 2 + 1).min(h) {
            let ci = h - 1 - i;
            let (fi, fc) = (self.buf[i], self.buf[ci].conj());
            let odd = (fi + fc) * 0.5;
            let even = (fi - fc) * 0.5 * self.twiddles[i];
            freq[i] = odd + even;
            freq[ci] = (odd - even).conj();
        }
    }

    /// RT. `freq` has `N/2` bins, `time` `N` samples.
    pub(crate) fn ifft(&mut self, freq: &[C], time: &mut [f32]) {
        let h = self.half;
        for i in 0..(h / 2 + 1).min(h) {
            let ci = h - 1 - i;
            let (fi, fc) = (freq[i], freq[ci].conj());
            let odd = fi + fc;
            let even = (fi - fc) * self.twiddles[i].conj();
            self.buf[i] = odd + even;
            self.buf[ci] = (odd - even).conj();
        }
        self.inv
            .process_with_scratch(&mut self.buf, &mut self.scratch);
        for (i, (t, tw)) in self.buf.iter().zip(&self.half_bin_twists).enumerate() {
            let v = t * tw.conj();
            time[2 * i] = v.re;
            time[2 * i + 1] = v.im;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_sizes_match_signalsmith() {
        // presetDefault / presetCheaper at 48 kHz (block 5760 / 4800 → FFT 6144 / 5120).
        assert_eq!(fast_size_above(5761 / 2) * 2, 6144);
        assert_eq!(fast_size_above(4801 / 2) * 2, 5120);
        assert_eq!(complex_fast_size_above(7 * 16), 128);
    }

    #[test]
    fn matches_naive_half_bin_dft_and_round_trips() {
        let n = 48;
        let x: Vec<f32> = (0..n)
            .map(|i| ((i * 7919) % 101) as f32 / 50.0 - 1.0)
            .collect();
        let mut fft = ModifiedRealFft::new(n);
        let mut freq = vec![C::default(); n / 2];
        fft.fft(&x, &mut freq);
        for (k, f) in freq.iter().enumerate() {
            let mut want = rustfft::num_complex::Complex64::default();
            for (i, v) in x.iter().enumerate() {
                let ph = -2.0 * std::f64::consts::PI * (k as f64 + 0.5) * i as f64 / n as f64;
                want += rustfft::num_complex::Complex64::from_polar(*v as f64, ph);
            }
            assert!((f.re as f64 - want.re).abs() < 1e-3, "bin {k}");
            assert!((f.im as f64 - want.im).abs() < 1e-3, "bin {k}");
        }
        let mut back = vec![0.0; n];
        fft.ifft(&freq, &mut back);
        for (a, b) in back.iter().zip(&x) {
            assert!((a / n as f32 - b).abs() < 1e-4);
        }
    }
}

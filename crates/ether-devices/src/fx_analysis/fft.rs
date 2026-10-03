//! Minimal in-place radix-2 complex FFT (no dependency; the analyzers only need forward
//! transforms of power-of-two sizes). Twiddles are computed once for the largest size and
//! strided for smaller ones, so [`Fft::forward`] never allocates.

use std::f64::consts::TAU;

pub(crate) struct Fft {
    n_max: usize,
    /// `cos(2πk/n_max)`, `k < n_max/2`.
    cos: Vec<f32>,
    /// `sin(2πk/n_max)`, `k < n_max/2`.
    sin: Vec<f32>,
}

impl Fft {
    /// Non-RT. Transforms of any power-of-two size up to `n_max` (a power of two).
    pub(crate) fn new(n_max: usize) -> Self {
        assert!(n_max.is_power_of_two() && n_max >= 2);
        let half = n_max / 2;
        let angle = |k: usize| TAU * k as f64 / n_max as f64;
        Self {
            n_max,
            cos: (0..half).map(|k| angle(k).cos() as f32).collect(),
            sin: (0..half).map(|k| angle(k).sin() as f32).collect(),
        }
    }

    /// RT. Forward DFT `X[k] = Σ x[n]·e^{-2πikn/N}` in place, `N = re.len() = im.len()`
    /// (a power of two `<= max_len`).
    pub(crate) fn forward(&self, re: &mut [f32], im: &mut [f32]) {
        let n = re.len();
        debug_assert!(n == im.len() && n.is_power_of_two() && n <= self.n_max);
        if n < 2 {
            return;
        }
        // Bit-reversal permutation.
        let bits = n.trailing_zeros();
        for i in 0..n {
            let j = i.reverse_bits() >> (usize::BITS - bits);
            if j > i {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let half = len / 2;
            let stride = self.n_max / len;
            for start in (0..n).step_by(len) {
                for j in 0..half {
                    let (c, s) = (self.cos[j * stride], self.sin[j * stride]);
                    let (a, b) = (start + j, start + j + half);
                    // (xr + i·xi)·(c − i·s)
                    let tr = re[b] * c + im[b] * s;
                    let ti = im[b] * c - re[b] * s;
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            len <<= 1;
        }
    }
}

/// Non-RT. Periodic Hann window of length `n`.
pub(crate) fn hann(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| (0.5 - 0.5 * (TAU * i as f64 / n as f64).cos()) as f32)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_a_naive_dft() {
        let fft = Fft::new(64);
        for n in [2usize, 8, 32, 64] {
            let x: Vec<f32> = (0..n).map(|i| ((i * 7 + 3) % 11) as f32 - 5.0).collect();
            let (mut re, mut im) = (x.clone(), vec![0.0; n]);
            fft.forward(&mut re, &mut im);
            for k in 0..n {
                let (mut sr, mut si) = (0.0f64, 0.0f64);
                for (i, v) in x.iter().enumerate() {
                    let a = -TAU * (k * i) as f64 / n as f64;
                    sr += f64::from(*v) * a.cos();
                    si += f64::from(*v) * a.sin();
                }
                assert!((f64::from(re[k]) - sr).abs() < 1e-3, "n={n} k={k}");
                assert!((f64::from(im[k]) - si).abs() < 1e-3, "n={n} k={k}");
            }
        }
    }
}

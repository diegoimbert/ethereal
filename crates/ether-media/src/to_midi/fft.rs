//! A small in-place radix-2 complex FFT (no dependency; sizes are powers of two) and the
//! analysis windows.

pub(crate) struct Fft {
    n: usize,
    /// `(cos, sin)` of `2πk/n`, `k < n/2`.
    twiddle: Vec<(f32, f32)>,
    rev: Vec<u32>,
}

impl Fft {
    pub(crate) fn new(n: usize) -> Self {
        assert!(n.is_power_of_two() && n >= 2);
        let twiddle = (0..n / 2)
            .map(|k| {
                let a = 2.0 * std::f64::consts::PI * k as f64 / n as f64;
                (a.cos() as f32, a.sin() as f32)
            })
            .collect();
        let bits = n.trailing_zeros();
        let rev = (0..n as u32)
            .map(|i| i.reverse_bits() >> (32 - bits))
            .collect();
        Self { n, twiddle, rev }
    }

    pub(crate) fn len(&self) -> usize {
        self.n
    }

    /// Forward transform (`X_k = Σ x_j e^{-2πijk/n}`), in place.
    pub(crate) fn forward(&self, re: &mut [f32], im: &mut [f32]) {
        let n = self.n;
        debug_assert!(re.len() == n && im.len() == n);
        for i in 0..n {
            let j = self.rev[i] as usize;
            if i < j {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let step = n / len;
            let half = len / 2;
            for start in (0..n).step_by(len) {
                for k in 0..half {
                    let (c, s) = self.twiddle[k * step];
                    let (a, b) = (start + k, start + k + half);
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

    /// Inverse transform, scaled by `1/n`, in place.
    pub(crate) fn inverse(&self, re: &mut [f32], im: &mut [f32]) {
        // ifft(x) = conj(fft(conj(x))) / n
        for v in im.iter_mut() {
            *v = -*v;
        }
        self.forward(re, im);
        let scale = 1.0 / self.n as f32;
        for (r, i) in re.iter_mut().zip(im.iter_mut()) {
            *r *= scale;
            *i = -*i * scale;
        }
    }
}

/// A periodic Hann window of `n` points.
pub(crate) fn hann(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| {
            let x = std::f64::consts::PI * 2.0 * i as f64 / n as f64;
            (0.5 - 0.5 * x.cos()) as f32
        })
        .collect()
}

/// A Blackman window of `n` points (sidelobes below -58 dB).
pub(crate) fn blackman(n: usize) -> Vec<f32> {
    let m = (n.max(2) - 1) as f64;
    (0..n)
        .map(|i| {
            let x = 2.0 * std::f64::consts::PI * i as f64 / m;
            (0.42 - 0.5 * x.cos() + 0.08 * (2.0 * x).cos()) as f32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_a_naive_dft_and_inverts() {
        let n = 64;
        let x: Vec<f32> = (0..n).map(|i| ((i * 7 % 13) as f32 - 6.0) / 6.0).collect();
        let fft = Fft::new(n);
        let (mut re, mut im) = (x.clone(), vec![0.0; n]);
        fft.forward(&mut re, &mut im);
        for k in [0, 1, 5, 31, 63] {
            let (mut sr, mut si) = (0.0f64, 0.0f64);
            for (j, v) in x.iter().enumerate() {
                let a = -2.0 * std::f64::consts::PI * (j * k) as f64 / n as f64;
                sr += *v as f64 * a.cos();
                si += *v as f64 * a.sin();
            }
            assert!((re[k] as f64 - sr).abs() < 1e-3, "re {k}");
            assert!((im[k] as f64 - si).abs() < 1e-3, "im {k}");
        }
        fft.inverse(&mut re, &mut im);
        for (a, b) in re.iter().zip(&x) {
            assert!((a - b).abs() < 1e-5);
        }
    }
}

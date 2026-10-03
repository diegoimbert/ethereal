//! Mono mix + decimation to the analysis rate, a chunk at a time (the first stage of every
//! detection). The low-pass is a linear-phase windowed sinc centred on each output sample,
//! so analysis times map back to source times exactly (`i · factor / rate`).

use crate::DecodedAudio;

pub(crate) struct Prep {
    pub factor: usize,
    taps: Vec<f32>,
    /// Next output sample to compute.
    next: usize,
    pub out: Vec<f32>,
    total: usize,
}

impl Prep {
    /// Decimate by `factor` (1 = mono mix only).
    pub(crate) fn new(audio: &DecodedAudio, factor: usize) -> Self {
        let factor = factor.max(1);
        let frames = audio.frames();
        let total = frames.div_ceil(factor);
        let taps = if factor == 1 {
            vec![1.0]
        } else {
            // Cutoff at 90 % of the output Nyquist.
            let fc = 0.45 / factor as f64;
            let len = 16 * factor + 1;
            let mid = (len / 2) as f64;
            let win = super::fft::blackman(len);
            let mut h: Vec<f64> = (0..len)
                .map(|i| {
                    let x = i as f64 - mid;
                    let s = if x == 0.0 {
                        2.0 * fc
                    } else {
                        (2.0 * std::f64::consts::PI * fc * x).sin() / (std::f64::consts::PI * x)
                    };
                    s * win[i] as f64
                })
                .collect();
            let sum: f64 = h.iter().sum();
            for v in &mut h {
                *v /= sum;
            }
            h.into_iter().map(|v| v as f32).collect()
        };
        Self {
            factor,
            taps,
            next: 0,
            out: Vec::with_capacity(total),
            total,
        }
    }

    pub(crate) fn done(&self) -> bool {
        self.next >= self.total
    }

    pub(crate) fn progress(&self) -> f32 {
        if self.total == 0 {
            1.0
        } else {
            self.next as f32 / self.total as f32
        }
    }

    /// Compute output samples worth up to `budget` input frames; returns the frames used.
    pub(crate) fn step(&mut self, audio: &DecodedAudio, budget: usize) -> usize {
        let chans = &audio.channels;
        let n_in = audio.frames();
        let gain = 1.0 / chans.len().max(1) as f32;
        let half = (self.taps.len() / 2) as isize;
        let mut used = 0;
        while self.next < self.total && used < budget {
            let center = (self.next * self.factor) as isize;
            let mut acc = 0.0f32;
            if self.factor == 1 {
                let i = center as usize;
                for ch in chans {
                    acc += ch[i];
                }
            } else {
                let lo = (center - half).max(0) as usize;
                let hi = ((center + half + 1) as usize).min(n_in);
                let t0 = (lo as isize - (center - half)) as usize;
                for ch in chans {
                    let mut s = 0.0f32;
                    for (x, h) in ch[lo..hi].iter().zip(&self.taps[t0..]) {
                        s += x * h;
                    }
                    acc += s;
                }
            }
            self.out.push(acc * gain);
            self.next += 1;
            used += self.factor;
        }
        used
    }
}

/// Short-time peak level of `x` in dB, in 1 ms blocks: block `j` holds the largest
/// `|x|` over the [`HOLD_SECONDS`] ending with it. A trailing peak rises the moment an
/// attack starts, yet stays flat over the cycles of a low note or beating partials (an
/// energy envelope over blocks this short would flutter with them).
pub(crate) struct Envelope {
    /// Seconds per block.
    pub block_sec: f64,
    pub db: Vec<f32>,
}

/// How long the envelope holds a peak (covers half a period down to ≈ 33 Hz).
const HOLD_SECONDS: f64 = 0.015;

impl Envelope {
    pub(crate) fn new(x: &[f32], rate: f64) -> Self {
        let block = ((rate * 0.001).round() as usize).max(1);
        let peaks: Vec<f32> = x
            .chunks(block)
            .map(|c| c.iter().fold(0.0f32, |m, v| m.max(v.abs())))
            .collect();
        Self::from_peaks(peaks, block, rate)
    }

    /// The envelope of the first difference of `x` (a +6 dB/octave tilt: attacks stand out
    /// over the low notes still ringing).
    pub(crate) fn emphasized(x: &[f32], rate: f64) -> Self {
        let block = ((rate * 0.001).round() as usize).max(1);
        let peaks: Vec<f32> = (0..x.len().div_ceil(block))
            .map(|j| {
                let (a, b) = (j * block, ((j + 1) * block).min(x.len()));
                (a..b).fold(0.0f32, |m, i| {
                    let prev = if i > 0 { x[i - 1] } else { 0.0 };
                    m.max((x[i] - prev).abs())
                })
            })
            .collect();
        Self::from_peaks(peaks, block, rate)
    }

    fn from_peaks(peaks: Vec<f32>, block: usize, rate: f64) -> Self {
        let hold = ((HOLD_SECONDS * rate) as usize / block).max(1);
        let db = (0..peaks.len())
            .map(|j| {
                let m = peaks[j.saturating_sub(hold - 1)..=j]
                    .iter()
                    .copied()
                    .fold(0.0f32, f32::max);
                20.0 * (m + 1e-6).log10()
            })
            .collect();
        Self {
            block_sec: block as f64 / rate,
            db,
        }
    }

    pub(crate) fn max_db(&self) -> f32 {
        self.db.iter().copied().fold(-120.0, f32::max)
    }

    fn index(&self, t: f64) -> isize {
        (t / self.block_sec).floor() as isize
    }

    /// Peak level in `[t0, t1)` seconds.
    pub(crate) fn peak(&self, t0: f64, t1: f64) -> f32 {
        let n = self.db.len() as isize;
        let a = self.index(t0).clamp(0, n);
        let b = (self.index(t1) + 1).clamp(a, n);
        self.db[a as usize..b as usize]
            .iter()
            .copied()
            .fold(-120.0, f32::max)
    }

    /// Sharpen an onset estimate `t` (seconds): when the level rises by at least 6 dB
    /// around `t`, the onset is the start of the first block within 6 dB of the attack's
    /// peak; otherwise `t` (e.g. a legato pitch change) is kept.
    pub(crate) fn refine_onset(&self, t: f64, early: f64, late: f64) -> f64 {
        let n = self.db.len() as isize;
        if n == 0 {
            return t;
        }
        let lo = self.index(t - early).clamp(0, n - 1) as usize;
        let hi = self.index(t + late).clamp(0, n - 1) as usize;
        let from = self.index(t - 0.01).clamp(lo as isize, hi as isize) as usize;
        let Some(pk) = (from..=hi).max_by(|&a, &b| self.db[a].total_cmp(&self.db[b])) else {
            return t;
        };
        let Some(mn) = (lo..=pk).min_by(|&a, &b| self.db[a].total_cmp(&self.db[b])) else {
            return t;
        };
        if self.db[pk] - self.db[mn] < 6.0 {
            return t;
        }
        // Halfway up the rise (in dB), at most 20 dB under the peak: early in the attack,
        // and robust to beating between partials.
        let thr = (0.5 * (self.db[pk] + self.db[mn])).max(self.db[pk] - 20.0);
        let j = (mn..=pk).find(|&j| self.db[j] >= thr).unwrap_or(pk);
        j as f64 * self.block_sec
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimation_keeps_low_tones_and_time() {
        let sr = 48_000;
        let n = 48_000;
        let tone: Vec<f32> = (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sr as f32).sin())
            .collect();
        let audio = DecodedAudio {
            sample_rate: sr,
            channels: vec![tone.clone(), tone],
        };
        let mut p = Prep::new(&audio, 4);
        while !p.done() {
            p.step(&audio, 1000);
        }
        assert_eq!(p.out.len(), 12_000);
        // Same phase at the same time (linear phase, centred).
        for k in [1000usize, 5000, 9000] {
            let want = (2.0 * std::f32::consts::PI * 440.0 * (k * 4) as f32 / sr as f32).sin();
            assert!((p.out[k] - want).abs() < 0.02, "{k}: {} vs {want}", p.out[k]);
        }
    }

    #[test]
    fn refine_finds_the_attack() {
        let rate = 12_000.0;
        let mut x = vec![0.0f32; 12_000];
        for (i, v) in x.iter_mut().enumerate().skip(6_000) {
            *v = (i as f32 * 0.3).sin() * 0.5;
        }
        let env = Envelope::new(&x, rate);
        let t = env.refine_onset(0.47, 0.045, 0.07);
        assert!((t - 0.5).abs() < 0.004, "{t}");
    }
}

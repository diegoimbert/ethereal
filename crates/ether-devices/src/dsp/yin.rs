//! Monophonic pitch detection: YIN (de Cheveigné & Kawahara 2002) and a causal pYIN
//! (Mauch & Dixon 2014), on a decimated copy of the input.
//!
//! - **Decimation**: the input is low-passed (a 4th-order Butterworth at 80 % of the
//!   detection rate's Nyquist frequency) and kept every `factor`-th sample, so detection
//!   runs at about [`DETECT_RATE`] whatever the sample rate.
//! - **Difference function** every [`HOP`] detection samples over a [`Detector::window`]
//!   window: `d(τ) = Σ (x_j - x_{j+τ})²`, from the cross-correlation computed with a real
//!   FFT (`O(N log N)`, not `O(N τmax)`), then the cumulative mean normalized difference
//!   `d'(τ) = d(τ) τ / Σ_{k<=τ} d(k)` over `τ` in the [`MIN_HZ`] ..= [`MAX_HZ`] range.
//! - **YIN**: the first dip of `d'` under the threshold (its local minimum), else the
//!   global minimum; voiced when that dip is under the voicing threshold. Parabolic
//!   interpolation gives a fractional period.
//! - **pYIN**: every local minimum of `d'` is a candidate, weighted by how much of a beta
//!   prior over thresholds picks it (the first dip under a threshold wins it; thresholds
//!   under every dip give a small weight to the global minimum). The candidates feed a
//!   hidden Markov model over [`BINS`] pitch bins (20 cents) × voiced / unvoiced, with
//!   triangular pitch transitions (at most ±[`MAX_JUMP`] bins per hop) and a small voicing
//!   switch probability, filtered causally (the forward algorithm, no lookahead): the
//!   estimate is the most probable state now. That trades pYIN's offline Viterbi for zero
//!   added latency and keeps most of its robustness (fewer octave jumps and dropouts).
//!
//! Real-time: every buffer and FFT plan is made in [`Detector::new`].

use std::sync::Arc;

use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

/// Detection sample rate (approximately: the input rate over an integer factor).
pub const DETECT_RATE: f32 = 16_000.0;
/// Lowest and highest detectable pitch (Hz).
pub const MIN_HZ: f32 = 70.0;
pub const MAX_HZ: f32 = 1400.0;
/// Detection samples between two estimates.
pub const HOP: usize = 64;
/// pYIN pitch bins: 20 cents from [`MIN_HZ`] to [`MAX_HZ`].
pub const BINS: usize = 260;
const BINS_PER_SEMITONE: f32 = 5.0;
/// Largest pitch move per hop the HMM allows (bins).
const MAX_JUMP: usize = 25;
/// Probability of switching voiced ↔ unvoiced per hop.
const SWITCH: f32 = 0.01;
/// Below this window level (RMS, dBFS) nothing is voiced.
pub const GATE_DB: f32 = -55.0;

/// Which detector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Yin,
    PYin,
}

/// One estimate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Estimate {
    /// Period in input samples (0 when unvoiced).
    pub period: f32,
    /// Pitch (Hz, 0 when unvoiced).
    pub hz: f32,
    pub voiced: bool,
    /// 0 ..= 1: `1 - d'` at the chosen period (YIN), the voiced probability (pYIN).
    pub confidence: f32,
    /// Input samples from the newest one back to the centre of what was measured.
    pub delay: u32,
}

impl Estimate {
    pub const UNVOICED: Self = Self {
        period: 0.0,
        hz: 0.0,
        voiced: false,
        confidence: 0.0,
        delay: 0,
    };
}

/// Direct-form-I biquad (decimation filter).
#[derive(Clone, Copy, Default)]
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    x: [f32; 2],
    y: [f32; 2],
}

impl Biquad {
    /// RBJ low-pass at `f` (Hz), Q `q`.
    fn low_pass(f: f32, q: f32, sr: f32) -> Self {
        let w = std::f32::consts::TAU * f / sr;
        let (s, c) = w.sin_cos();
        let alpha = s / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self {
            b: [(1.0 - c) / 2.0 / a0, (1.0 - c) / a0, (1.0 - c) / 2.0 / a0],
            a: [-2.0 * c / a0, (1.0 - alpha) / a0],
            ..Self::default()
        }
    }

    #[inline]
    fn tick(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.b[1] * self.x[0] + self.b[2] * self.x[1]
            - self.a[0] * self.y[0]
            - self.a[1] * self.y[1];
        self.x = [x, self.x[0]];
        self.y = [super::flush32(y), self.y[0]];
        y
    }
}

/// Bin of a pitch (Hz), fractional.
fn bin_of(hz: f32) -> f32 {
    12.0 * BINS_PER_SEMITONE * (hz / MIN_HZ).log2()
}

/// The streaming detector.
pub struct Detector {
    method: Method,
    sample_rate: f32,
    factor: usize,
    phase: usize,
    aa: [Biquad; 2],
    /// Detection rate.
    rate: f32,
    window: usize,
    min_tau: usize,
    max_tau: usize,
    /// The last input samples (full rate, for the final period refinement).
    raw: Vec<f32>,
    raw_at: usize,
    /// The last `window + max_tau` detection samples (a ring, unrolled into `frame`).
    ring: Vec<f32>,
    at: usize,
    since: usize,
    filled: usize,
    frame: Vec<f32>,
    // FFT cross-correlation.
    fwd: Arc<dyn RealToComplex<f32>>,
    inv: Arc<dyn ComplexToReal<f32>>,
    a_time: Vec<f32>,
    b_time: Vec<f32>,
    a_spec: Vec<Complex32>,
    b_spec: Vec<Complex32>,
    scratch: Vec<Complex32>,
    corr: Vec<f32>,
    /// `d'(τ)`, `τ < max_tau + 2`.
    cmnd: Vec<f32>,
    /// pYIN: beta-prior CDF over thresholds 0..=1 (101 points), HMM state.
    cdf: [f32; 101],
    voiced: Vec<f32>,
    unvoiced: Vec<f32>,
    next_v: Vec<f32>,
    next_u: Vec<f32>,
    obs: Vec<f32>,
    /// pYIN: the candidate period (detection samples) seen in each bin this hop.
    bin_tau: Vec<f32>,
    weights: [f32; 2 * MAX_JUMP + 1],
    /// The voiced observation's scale (sensitivity).
    voiced_scale: f32,
    /// YIN: dip threshold and voicing threshold.
    threshold: f32,
    last: Estimate,
}

impl Detector {
    /// Non-RT. A detector for input at `sample_rate`.
    pub fn new(sample_rate: f32, method: Method) -> Self {
        let sample_rate = sample_rate.max(8_000.0);
        let factor = ((sample_rate / DETECT_RATE).round() as usize).max(1);
        let rate = sample_rate / factor as f32;
        let min_tau = ((rate / MAX_HZ).floor() as usize).max(2);
        let max_tau = (rate / MIN_HZ).ceil() as usize + 2;
        let window = max_tau.next_power_of_two();
        let n = window + max_tau;
        let m = n.next_power_of_two();
        let mut planner = RealFftPlanner::<f32>::new();
        let fwd = planner.plan_fft_forward(m);
        let inv = planner.plan_fft_inverse(m);
        let scratch_len = fwd.get_scratch_len().max(inv.get_scratch_len());
        // Beta(2, 11.33) prior over thresholds (mean 0.15, pYIN's default), as a CDF.
        let mut cdf = [0.0f32; 101];
        let (alpha, beta) = (2.0f64, 34.0 / 3.0);
        let pdf = |x: f64| x.powf(alpha - 1.0) * (1.0 - x).powf(beta - 1.0);
        let mut acc = 0.0;
        for (i, c) in cdf.iter_mut().enumerate().skip(1) {
            acc += pdf((i as f64 - 0.5) / 100.0);
            *c = acc as f32;
        }
        let total = cdf[100];
        cdf.iter_mut().for_each(|c| *c /= total);
        let mut weights = [0.0f32; 2 * MAX_JUMP + 1];
        for (k, w) in weights.iter_mut().enumerate() {
            let d = (k as f32 - MAX_JUMP as f32).abs();
            *w = (MAX_JUMP as f32 + 1.0 - d).max(0.0);
        }
        let sum: f32 = weights.iter().sum();
        weights.iter_mut().for_each(|w| *w /= sum);
        let nyq = rate * 0.5;
        Self {
            method,
            sample_rate,
            factor,
            phase: 0,
            aa: [
                Biquad::low_pass(0.8 * nyq, 0.54, sample_rate),
                Biquad::low_pass(0.8 * nyq, 1.31, sample_rate),
            ],
            rate,
            window,
            min_tau,
            max_tau,
            raw: vec![0.0; n * factor + factor + 2],
            raw_at: 0,
            ring: vec![0.0; n],
            at: 0,
            since: 0,
            filled: 0,
            frame: vec![0.0; n],
            fwd,
            inv,
            a_time: vec![0.0; m],
            b_time: vec![0.0; m],
            a_spec: vec![Complex32::new(0.0, 0.0); m / 2 + 1],
            b_spec: vec![Complex32::new(0.0, 0.0); m / 2 + 1],
            scratch: vec![Complex32::new(0.0, 0.0); scratch_len],
            corr: vec![0.0; m],
            cmnd: vec![1.0; max_tau + 2],
            cdf,
            voiced: vec![0.0; BINS],
            unvoiced: vec![1.0 / BINS as f32; BINS],
            next_v: vec![0.0; BINS],
            next_u: vec![0.0; BINS],
            obs: vec![0.0; BINS],
            bin_tau: vec![0.0; BINS],
            weights,
            voiced_scale: 1.0,
            threshold: 0.15,
            last: Estimate::UNVOICED,
        }
    }

    /// RT. Switch method (the pYIN state restarts).
    pub fn set_method(&mut self, method: Method) {
        if method != self.method {
            self.method = method;
            self.reset_hmm();
        }
    }

    /// RT. Sensitivity 0 ..= 1 (0.5 = the defaults): how readily a frame counts as voiced.
    pub fn set_sensitivity(&mut self, s: f32) {
        let s = s.clamp(0.0, 1.0);
        // pYIN: the voiced observations ×0.25 ..= ×4; YIN: voicing threshold 0.05 ..= 0.45.
        self.voiced_scale = 4f32.powf(2.0 * s - 1.0);
        self.threshold = 0.05 + 0.4 * s;
    }

    /// Input samples between two estimates.
    pub fn hop_samples(&self) -> usize {
        HOP * self.factor
    }

    /// Input samples from the newest sample back to the centre of the analysed span (of
    /// an unvoiced estimate; a voiced one says its own, [`Estimate::delay`]).
    pub fn centre_delay(&self) -> usize {
        (self.window + self.max_tau) * self.factor / 2
    }

    /// Input samples back to the centre of what the difference function at lag `tau`
    /// (detection samples) compares: `x_j` against `x_{j+τ}`, `j < window`, from the
    /// start of the frame.
    fn delay_of(&self, tau: f32) -> usize {
        let n = (self.window + self.max_tau) as f32;
        ((n - (self.window as f32 + tau) * 0.5) * self.factor as f32).round() as usize
    }

    fn reset_hmm(&mut self) {
        self.voiced.fill(0.0);
        self.unvoiced.fill(1.0 / BINS as f32);
    }

    /// RT. Forget the signal.
    pub fn reset(&mut self) {
        self.ring.fill(0.0);
        self.raw.fill(0.0);
        self.raw_at = 0;
        self.at = 0;
        self.since = 0;
        self.filled = 0;
        self.phase = 0;
        for b in &mut self.aa {
            b.x = [0.0; 2];
            b.y = [0.0; 2];
        }
        self.reset_hmm();
        self.last = Estimate::UNVOICED;
    }

    /// RT. One input sample; `Some` when a new estimate is ready (every
    /// [`Self::hop_samples`]).
    #[inline]
    pub fn push(&mut self, x: f32) -> Option<Estimate> {
        let x = if x.is_finite() { x } else { 0.0 };
        self.raw[self.raw_at] = x;
        self.raw_at = (self.raw_at + 1) % self.raw.len();
        let first = self.aa[0].tick(x);
        let y = self.aa[1].tick(first);
        self.phase += 1;
        if self.phase < self.factor {
            return None;
        }
        self.phase = 0;
        let n = self.ring.len();
        self.ring[self.at] = y;
        self.at = (self.at + 1) % n;
        self.filled = (self.filled + 1).min(n);
        self.since += 1;
        if self.since < HOP {
            return None;
        }
        self.since = 0;
        self.last = if self.filled < n {
            Estimate::UNVOICED
        } else {
            self.estimate()
        };
        Some(self.last)
    }

    fn estimate(&mut self) -> Estimate {
        let n = self.ring.len();
        // Unroll the ring, oldest first.
        let (tail, head) = self.ring.split_at(self.at);
        self.frame[..head.len()].copy_from_slice(head);
        self.frame[head.len()..].copy_from_slice(tail);
        let rms = (self.frame[..n].iter().map(|v| v * v).sum::<f32>() / n as f32).sqrt();
        if rms <= 10f32.powf(GATE_DB / 20.0) {
            self.decay_hmm();
            return Estimate::UNVOICED;
        }
        self.difference();
        match self.method {
            Method::Yin => self.yin(),
            Method::PYin => self.pyin(),
        }
    }

    /// `d'(τ)` into `cmnd` (FFT cross-correlation).
    fn difference(&mut self) {
        let n = self.frame.len();
        let w = self.window;
        let m = self.a_time.len();
        self.a_time.fill(0.0);
        self.a_time[..w].copy_from_slice(&self.frame[..w]);
        self.b_time.fill(0.0);
        self.b_time[..n].copy_from_slice(&self.frame);
        let _ =
            self.fwd
                .process_with_scratch(&mut self.a_time, &mut self.a_spec, &mut self.scratch);
        let _ =
            self.fwd
                .process_with_scratch(&mut self.b_time, &mut self.b_spec, &mut self.scratch);
        for (a, b) in self.a_spec.iter_mut().zip(&self.b_spec) {
            *a = a.conj() * b;
        }
        self.a_spec[0].im = 0.0;
        if let Some(l) = self.a_spec.last_mut() {
            l.im = 0.0;
        }
        let _ = self
            .inv
            .process_with_scratch(&mut self.a_spec, &mut self.corr, &mut self.scratch);
        let scale = 1.0 / m as f32;
        // Energies: Σ_{j<w} x_j² and the sliding Σ_{j<w} x_{j+τ}².
        let e0: f32 = self.frame[..w].iter().map(|v| v * v).sum();
        let mut et = e0;
        let mut running = 0.0f32;
        self.cmnd[0] = 1.0;
        for tau in 1..self.cmnd.len().min(n - w + 1) {
            et += self.frame[tau + w - 1].powi(2) - self.frame[tau - 1].powi(2);
            let d = (e0 + et - 2.0 * self.corr[tau] * scale).max(0.0);
            running += d;
            self.cmnd[tau] = if running > 0.0 {
                d * tau as f32 / running
            } else {
                1.0
            };
        }
    }

    /// Parabolic interpolation of `d'` around `tau`.
    fn refine(&self, tau: usize) -> f32 {
        if tau < 1 || tau + 1 >= self.cmnd.len() {
            return tau as f32;
        }
        let (a, b, c) = (self.cmnd[tau - 1], self.cmnd[tau], self.cmnd[tau + 1]);
        let den = a - 2.0 * b + c;
        if den.abs() < 1e-9 {
            tau as f32
        } else {
            tau as f32 + (0.5 * (a - c) / den).clamp(-0.5, 0.5)
        }
    }

    fn estimate_of(&self, tau: f32, confidence: f32, voiced: bool) -> Estimate {
        if !voiced || tau <= 0.0 {
            return Estimate {
                confidence,
                ..Estimate::UNVOICED
            };
        }
        let delay = self.delay_of(tau);
        let period = self.refine_full(tau * self.factor as f32, delay);
        Estimate {
            period,
            hz: self.sample_rate / period,
            voiced,
            confidence,
            delay: delay as u32,
        }
    }

    /// The period (input samples) refined at the full rate: the raw difference function
    /// around the decimated estimate `t`, over four periods centred `delay` samples back,
    /// with parabolic interpolation (the decimated lag alone is a few cents off at high
    /// pitches).
    fn refine_full(&self, t: f32, delay: usize) -> f32 {
        let f = self.factor;
        if f == 1 {
            return t;
        }
        let len = self.raw.len();
        let center = t.round() as usize;
        let (lo, hi) = (center.saturating_sub(f).max(2), center + f);
        // Up to four periods (as many as the ring holds), centred on `delay`: sample
        // `base + j` back pairs with `base + j + lag` back.
        if hi + 2 >= len {
            return t;
        }
        let w = (4 * center).min(len - hi - 2);
        let base = delay.saturating_sub((w + center) / 2).min(len - 1 - hi - w);
        // Oldest-relative access: sample `k` back from the newest.
        let at = |k: usize| self.raw[(self.raw_at + len - 1 - k) % len];
        let d = |lag: usize| -> f32 {
            let mut acc = 0.0;
            for j in 0..w {
                let e = at(base + j) - at(base + j + lag);
                acc += e * e;
            }
            acc
        };
        let mut best = (lo, f32::INFINITY);
        let mut vals = [0.0f32; 64];
        let n = (hi - lo + 1).min(64);
        for (k, v) in vals.iter_mut().enumerate().take(n) {
            *v = d(lo + k);
            if *v < best.1 {
                best = (lo + k, *v);
            }
        }
        let k = best.0 - lo;
        if k == 0 || k + 1 >= n {
            return best.0 as f32;
        }
        let (a, b, c) = (vals[k - 1], vals[k], vals[k + 1]);
        let den = a - 2.0 * b + c;
        if den.abs() < 1e-12 {
            best.0 as f32
        } else {
            best.0 as f32 + (0.5 * (a - c) / den).clamp(-0.5, 0.5)
        }
    }

    fn yin(&mut self) -> Estimate {
        let (lo, hi) = (self.min_tau, self.max_tau.min(self.cmnd.len() - 2));
        let mut pick = None;
        let mut tau = lo;
        while tau <= hi {
            if self.cmnd[tau] < 0.1 {
                while tau < hi && self.cmnd[tau + 1] < self.cmnd[tau] {
                    tau += 1;
                }
                pick = Some(tau);
                break;
            }
            tau += 1;
        }
        let tau = pick.unwrap_or_else(|| {
            (lo..=hi)
                .min_by(|&a, &b| self.cmnd[a].total_cmp(&self.cmnd[b]))
                .unwrap_or(lo)
        });
        let v = self.cmnd[tau];
        self.estimate_of(
            self.refine(tau),
            (1.0 - v).clamp(0.0, 1.0),
            v < self.threshold,
        )
    }

    fn cdf_at(&self, s: f32) -> f32 {
        let x = (s.clamp(0.0, 1.0) * 100.0).max(0.0);
        let i = (x as usize).min(99);
        let f = x - i as f32;
        self.cdf[i] + (self.cdf[i + 1] - self.cdf[i]) * f
    }

    fn pyin(&mut self) -> Estimate {
        let (lo, hi) = (self.min_tau, self.max_tau.min(self.cmnd.len() - 2));
        self.obs.fill(0.0);
        self.bin_tau.fill(0.0);
        // Candidates: the local minima of d', in τ order; minimum k wins the thresholds
        // between its value and the lowest earlier minimum.
        let mut earlier = f32::INFINITY;
        let mut global = (lo, f32::INFINITY);
        let mut mass = 0.0f32;
        for tau in lo..=hi {
            let v = self.cmnd[tau];
            if v < global.1 {
                global = (tau, v);
            }
            let is_min = v <= self.cmnd[tau - 1] && v < self.cmnd[tau + 1];
            if !is_min || v >= earlier {
                continue;
            }
            let p = self.cdf_at(earlier.min(1.0)) - self.cdf_at(v);
            earlier = v;
            if p > 0.0 {
                mass += p;
                self.add_candidate(self.refine(tau), p);
            }
        }
        // Thresholds under every minimum: pYIN gives 1 % of their weight to the global one.
        let below = self.cdf_at(global.1) * 0.01;
        if below > 0.0 {
            mass += below;
            self.add_candidate(self.refine(global.0), below);
        }
        let mass = mass.min(1.0);
        for o in &mut self.obs {
            *o *= self.voiced_scale;
        }
        let unvoiced_obs = (1.0 - mass).max(0.0) / BINS as f32;
        // Forward step: triangular pitch transitions, voicing switch.
        for j in 0..BINS {
            let mut sv = 0.0;
            let mut su = 0.0;
            let i0 = j.saturating_sub(MAX_JUMP);
            let i1 = (j + MAX_JUMP).min(BINS - 1);
            for i in i0..=i1 {
                let w = self.weights[i + MAX_JUMP - j];
                sv += w * self.voiced[i];
                su += w * self.unvoiced[i];
            }
            self.next_v[j] = (sv * (1.0 - SWITCH) + su * SWITCH) * self.obs[j];
            self.next_u[j] = (su * (1.0 - SWITCH) + sv * SWITCH) * unvoiced_obs;
        }
        let total: f32 = self.next_v.iter().sum::<f32>() + self.next_u.iter().sum::<f32>();
        if total.is_nan() || total <= 1e-30 {
            self.reset_hmm();
            return Estimate::UNVOICED;
        }
        let k = 1.0 / total;
        let mut best = (0usize, 0.0f32, false);
        let mut voiced_p = 0.0;
        for j in 0..BINS {
            self.voiced[j] = self.next_v[j] * k;
            self.unvoiced[j] = self.next_u[j] * k;
            voiced_p += self.voiced[j];
            if self.voiced[j] > best.1 {
                best = (j, self.voiced[j], true);
            }
            if self.unvoiced[j] > best.1 {
                best = (j, self.unvoiced[j], false);
            }
        }
        let (j, _, voiced) = best;
        let tau = if self.bin_tau[j] > 0.0 {
            self.bin_tau[j]
        } else {
            // No candidate in the winning bin this hop: its centre.
            let hz = MIN_HZ * 2f32.powf((j as f32 + 0.5) / (12.0 * BINS_PER_SEMITONE));
            self.rate / hz
        };
        self.estimate_of(tau, voiced_p.clamp(0.0, 1.0), voiced)
    }

    fn add_candidate(&mut self, tau: f32, p: f32) {
        if tau <= 0.0 {
            return;
        }
        let b = bin_of(self.rate / tau).round();
        if !(0.0..BINS as f32).contains(&b) {
            return;
        }
        let b = b as usize;
        if p > self.obs[b] {
            self.bin_tau[b] = tau;
        }
        self.obs[b] += p;
    }

    /// Silence: the HMM drifts towards unvoiced.
    fn decay_hmm(&mut self) {
        let mut total = 0.0;
        for j in 0..BINS {
            let v = self.voiced[j];
            self.unvoiced[j] += v * 0.5;
            self.voiced[j] = v * 0.5;
            total += self.voiced[j] + self.unvoiced[j];
        }
        if total > 0.0 {
            let k = 1.0 / total;
            self.voiced.iter_mut().for_each(|v| *v *= k);
            self.unvoiced.iter_mut().for_each(|v| *v *= k);
        } else {
            self.reset_hmm();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saw(hz: f32, sr: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let ph = (hz * i as f32 / sr).fract();
                0.5 * (2.0 * ph - 1.0)
            })
            .collect()
    }

    fn detect(method: Method, x: &[f32], sr: f32) -> Vec<Estimate> {
        let mut d = Detector::new(sr, method);
        x.iter().filter_map(|&v| d.push(v)).collect()
    }

    #[test]
    fn finds_the_pitch_of_tones_across_the_range() {
        for method in [Method::Yin, Method::PYin] {
            for sr in [44_100.0f32, 48_000.0, 96_000.0] {
                for hz in [82.0f32, 110.0, 220.0, 440.0, 660.0, 1000.0] {
                    let est = detect(method, &saw(hz, sr, (sr * 0.5) as usize), sr);
                    let last = est.last().unwrap();
                    assert!(last.voiced, "{method:?} {sr} {hz}: unvoiced");
                    let cents = 1200.0 * (last.hz / hz).log2();
                    assert!(cents.abs() < 5.0, "{method:?} {sr} {hz}: {} Hz", last.hz);
                    assert!((last.period - sr / hz).abs() / (sr / hz) < 0.005);
                }
            }
        }
    }

    #[test]
    fn pyin_holds_a_tone_in_heavy_noise() {
        let sr = 48_000.0;
        let mut s = 9u32;
        let x: Vec<f32> = (0..96_000)
            .map(|i| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = ((s >> 8) as f32 / (1 << 24) as f32 - 0.5) * 1.6;
                let t = std::f32::consts::TAU * 196.0 * i as f32 / sr;
                0.2 * t.sin() + 0.6 * (2.0 * t).sin() + noise
            })
            .collect();
        let score = |method| {
            let est = detect(method, &x, sr);
            let tail = &est[40..];
            let wrong = tail
                .iter()
                .filter(|e| !e.voiced || (1200.0 * (e.hz / 196.0).log2()).abs() > 50.0)
                .count();
            (wrong, tail.len())
        };
        let (yin, n) = score(Method::Yin);
        let (pyin, _) = score(Method::PYin);
        eprintln!("wrong or unvoiced: yin {yin} / {n}, pyin {pyin} / {n}");
        assert!(
            pyin <= yin && pyin * 20 < n,
            "yin {yin}, pyin {pyin} of {n}"
        );
    }

    #[test]
    fn noise_and_silence_are_unvoiced() {
        let mut s = 1u32;
        let noise: Vec<f32> = (0..24_000)
            .map(|_| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (s >> 8) as f32 / (1 << 24) as f32 - 0.5
            })
            .collect();
        for method in [Method::Yin, Method::PYin] {
            let est = detect(method, &noise, 48_000.0);
            let voiced = est.iter().filter(|e| e.voiced).count();
            assert!(
                voiced * 10 < est.len(),
                "{method:?}: {voiced} of {}",
                est.len()
            );
            let est = detect(method, &vec![0.0; 24_000], 48_000.0);
            assert!(est.iter().all(|e| !e.voiced));
        }
    }
}

#[cfg(test)]
mod vibrato {
    use super::*;
    #[test]
    fn tracks_a_vibrato_at_the_measured_time() {
        let sr = 48_000.0f32;
        let mut phase = 0.0f64;
        let f = |t: f32| 220.0 * 2f32.powf(40.0 * (std::f32::consts::TAU * 5.5 * t).sin() / 1200.0);
        let x: Vec<f32> = (0..96_000)
            .map(|i| {
                phase = (phase + f64::from(f(i as f32 / sr) / sr)).fract();
                (0.4 * (2.0 * phase - 1.0)) as f32
            })
            .collect();
        for method in [Method::Yin, Method::PYin] {
            let mut d = Detector::new(sr, method);
            let mut errs = vec![];
            let mut unv = 0;
            for (i, &v) in x.iter().enumerate() {
                if let Some(e) = d.push(v) {
                    if i > 20_000 && !e.voiced {
                        unv += 1;
                    }
                    if i > 20_000 && e.voiced {
                        let t = (i - e.delay as usize) as f32 / sr;
                        errs.push(1200.0 * (e.hz / f(t)).log2());
                    }
                }
            }
            let m = errs.iter().sum::<f32>() / errs.len() as f32;
            let sd =
                (errs.iter().map(|e| (e - m) * (e - m)).sum::<f32>() / errs.len() as f32).sqrt();
            let mx = errs.iter().fold(0f32, |a, b| a.max(b.abs()));
            eprintln!(
                "{method:?}: mean {m:.2} sd {sd:.2} max {mx:.2} cents, unvoiced {unv} of {}",
                errs.len() + unv
            );
            // A ±40-cent vibrato, read at the centre of each window (`Estimate::delay`).
            assert!(unv == 0 && m.abs() < 1.0 && sd < 3.0 && mx < 8.0);
        }
    }
}

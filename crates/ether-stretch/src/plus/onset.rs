//! Onset (transient) detection on the input stream, ahead of the stretcher's analysis.
//!
//! - **Detection function**: rectified log-magnitude spectral flux (Bello et al., "A tutorial
//!   on onset detection in music signals", IEEE TSAP 2005; log compression as in Böck &
//!   Widmer's SuperFlux, DAFx 2013) of short frames (≈10 ms Hann, ≈2.7 ms hop), per band
//!   (low / mid / high, the stretcher's resolutions) and in total.
//! - **Picking**: a local maximum of the total flux above an adaptive threshold (a multiple of
//!   its recent mean plus a floor), at least [`MIN_GAP_SECONDS`] after the previous onset.
//! - **Location**: refined in the time domain to the start of the attack: the steepest rise of
//!   a pre-emphasised energy envelope near the frame, walked back to where the envelope was
//!   still low. Frame-level flux only says "somewhere in this 10 ms window".
//! - **Strength per band**: each band's flux over its own recent mean, so the stretcher can
//!   treat a hi-hat (high band only) differently from a kick (low band).

use std::sync::Arc;

use realfft::num_complex::Complex32 as C;
use realfft::{RealFftPlanner, RealToComplex};

/// Most onsets remembered (the stretcher only looks a few hundred ms around its cursor).
pub(crate) const MAX_ONSETS: usize = 64;
/// Bands: low, mid, high.
pub(crate) const BANDS: usize = 3;
/// Shortest gap between two onsets.
const MIN_GAP_SECONDS: f32 = 0.03;
/// Recent frames averaged for the adaptive threshold.
const HISTORY: usize = 24;
/// Threshold: `RATIO × recent mean + FLOOR` (total flux, per bin).
const RATIO: f32 = 1.8;
const FLOOR: f32 = 0.06;
/// Log compression `ln(1 + γ|X|)` with `|X|` normalised so a full-scale sine is ≈ 0.5.
const GAMMA: f32 = 2000.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Onset {
    /// Input sample of the start of the attack.
    pub pos: i64,
    /// Per band ([low, mid, high]): flux over the band's recent mean (≥ 1).
    pub strength: [f32; BANDS],
}

pub(crate) struct OnsetDetector {
    n: usize,
    hop: usize,
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    frame: Vec<f32>,
    spec: Vec<C>,
    scratch: Vec<C>,
    prev_log: Vec<f32>,
    /// Band edges (bins): `[0, b1)`, `[b1, b2)`, `[b2, n/2]`.
    edges: [usize; BANDS + 1],
    /// Mono input and its pre-emphasised energy envelope, rings of `mask + 1`.
    mono: Vec<f32>,
    env: Vec<f32>,
    mask: usize,
    /// Input samples received.
    count: i64,
    env_state: f32,
    last_x: f32,
    /// Centre of the next analysis frame.
    next_center: i64,
    primed: bool,
    /// Flux history (total + bands), ring of [`HISTORY`].
    hist: [[f32; BANDS + 1]; HISTORY],
    hist_len: usize,
    hist_at: usize,
    /// The previous two frames' (centre, total flux, band flux) for peak picking.
    back: [(i64, [f32; BANDS + 1], [f32; BANDS + 1]); 2],
    min_gap: i64,
    rise: usize,
    onsets: [Onset; MAX_ONSETS],
    onsets_len: usize,
    onsets_head: usize,
    last_onset: i64,
}

impl OnsetDetector {
    /// Non-RT. `n`/`hop`: frame size/hop (samples); `split_hz`: the band edges; `history`:
    /// input samples kept (≥ the stretcher's look-back).
    pub(crate) fn new(sr: f32, n: usize, hop: usize, split_hz: [f32; 2], history: usize) -> Self {
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(n);
        let scratch = vec![C::default(); fft.get_scratch_len()];
        let window = (0..n)
            .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos())
            .collect();
        let bin = |hz: f32| ((hz / sr * n as f32).round() as usize).clamp(1, n / 2);
        let cap = (history + 2 * n).next_power_of_two();
        Self {
            n,
            hop,
            fft,
            window,
            frame: vec![0.0; n],
            spec: vec![C::default(); n / 2 + 1],
            scratch,
            prev_log: vec![0.0; n / 2 + 1],
            edges: [0, bin(split_hz[0]), bin(split_hz[1]), n / 2 + 1],
            mono: vec![0.0; cap],
            env: vec![0.0; cap],
            mask: cap - 1,
            count: 0,
            env_state: 0.0,
            last_x: 0.0,
            next_center: 0,
            primed: false,
            hist: [[0.0; BANDS + 1]; HISTORY],
            hist_len: 0,
            hist_at: 0,
            back: [(0, [0.0; BANDS + 1], [0.0; BANDS + 1]); 2],
            min_gap: (sr * MIN_GAP_SECONDS) as i64,
            rise: ((sr * 0.001) as usize).max(1),
            onsets: [Onset::default(); MAX_ONSETS],
            onsets_len: 0,
            onsets_head: 0,
            last_onset: i64::MIN / 2,
        }
    }

    /// RT. Forget everything; the next sample pushed is input sample 0.
    pub(crate) fn reset(&mut self) {
        self.mono.fill(0.0);
        self.env.fill(0.0);
        self.count = 0;
        self.env_state = 0.0;
        self.last_x = 0.0;
        self.next_center = 0;
        self.primed = false;
        self.hist_len = 0;
        self.hist_at = 0;
        self.back = [(0, [0.0; BANDS + 1], [0.0; BANDS + 1]); 2];
        self.onsets_len = 0;
        self.onsets_head = 0;
        self.last_onset = i64::MIN / 2;
    }

    /// RT. Append one mono input sample.
    #[inline]
    pub(crate) fn push(&mut self, x: f32) {
        let i = self.count as usize & self.mask;
        self.mono[i] = x;
        // Pre-emphasised energy, one-pole smoothed (≈0.3 ms).
        let hp = x - 0.95 * self.last_x;
        self.last_x = x;
        self.env_state += 0.12 * (hp * hp - self.env_state);
        self.env[i] = self.env_state;
        self.count += 1;
    }

    crate::simd::avx2_dispatch! {
        /// RT. Analyse every frame the pushed input completes.
        pub(crate) fn run => run_avx2, run_generic(
            &mut self,
        )
    }

    #[inline(always)]
    fn run_generic(&mut self) {
        let half = (self.n / 2) as i64;
        while self.next_center + half <= self.count {
            let c = self.next_center;
            self.frame_at(c);
            self.next_center += self.hop as i64;
        }
    }

    fn sample(&self, i: i64) -> f32 {
        if i < 0 || i >= self.count || self.count - i > self.mask as i64 {
            0.0
        } else {
            self.mono[i as usize & self.mask]
        }
    }

    fn envelope(&self, i: i64) -> f32 {
        if i < 0 || i >= self.count || self.count - i > self.mask as i64 {
            0.0
        } else {
            self.env[i as usize & self.mask]
        }
    }

    fn frame_at(&mut self, c: i64) {
        let n = self.n;
        let start = c - (n / 2) as i64;
        for i in 0..n {
            self.frame[i] = self.sample(start + i as i64) * self.window[i];
        }
        let _ = self
            .fft
            .process_with_scratch(&mut self.frame, &mut self.spec, &mut self.scratch);
        let norm = 2.0 / (n as f32 * 0.5);
        let mut band = [0.0f32; BANDS + 1];
        for b in 0..BANDS {
            let (lo, hi) = (self.edges[b], self.edges[b + 1]);
            let mut sum = 0.0;
            for k in lo..hi {
                let l = fast_ln(1.0 + GAMMA * self.spec[k].norm_sqr().sqrt() * norm);
                sum += (l - self.prev_log[k]).max(0.0);
                self.prev_log[k] = l;
            }
            band[b + 1] = sum / (hi - lo).max(1) as f32;
        }
        // Total: per-bin mean over all bins.
        let total_bins = (n / 2 + 1) as f32;
        band[0] = (0..BANDS)
            .map(|b| band[b + 1] * (self.edges[b + 1] - self.edges[b]) as f32)
            .sum::<f32>()
            / total_bins;
        if !self.primed {
            // The first frame has no predecessor: its flux is meaningless.
            self.primed = true;
            band = [0.0; BANDS + 1];
        }
        // Means before this frame.
        let mut mean = [0.0f32; BANDS + 1];
        if self.hist_len > 0 {
            for h in &self.hist[..self.hist_len] {
                for (m, v) in mean.iter_mut().zip(h) {
                    *m += v;
                }
            }
            for m in &mut mean {
                *m /= self.hist_len as f32;
            }
        }
        // Peak-pick the previous frame: a local maximum above its threshold.
        let (pc, pflux, pmean) = self.back[1];
        let (_, ppflux, _) = self.back[0];
        if pflux[0] > ppflux[0]
            && pflux[0] >= band[0]
            && pflux[0] > RATIO * pmean[0] + FLOOR
            && pc - self.last_onset > self.min_gap
        {
            let mut strength = [1.0f32; BANDS];
            for b in 0..BANDS {
                strength[b] = (pflux[b + 1] / (RATIO * pmean[b + 1] + FLOOR)).max(0.0);
            }
            let pos = self.locate(pc);
            if pos - self.last_onset > self.min_gap {
                self.add(Onset { pos, strength });
                self.last_onset = pos;
            }
        }
        self.back[0] = self.back[1];
        self.back[1] = (c, band, mean);
        self.hist[self.hist_at] = band;
        self.hist_at = (self.hist_at + 1) % HISTORY;
        self.hist_len = (self.hist_len + 1).min(HISTORY);
    }

    /// Start of the attack near frame centre `c`: the steepest envelope rise in the frame's
    /// window, walked back while the envelope keeps falling, to below a fifth of it.
    fn locate(&self, c: i64) -> i64 {
        let half = (self.n / 2) as i64;
        let lo = (c - half).max(self.last_onset + self.min_gap);
        let hi = (c + half).min(self.count - 1);
        let r = self.rise as i64;
        let mut best = (c, 0.0f32);
        let mut i = lo + r;
        while i <= hi {
            let d = self.envelope(i) - self.envelope(i - r);
            if d > best.1 {
                best = (i, d);
            }
            i += 1;
        }
        let peak = self.envelope(best.0);
        let mut p = best.0;
        let limit = (best.0 - 8 * r).max(lo);
        while p > limit && self.envelope(p - 1) > 0.2 * peak {
            p -= 1;
        }
        p
    }

    fn add(&mut self, o: Onset) {
        let i = (self.onsets_head + self.onsets_len) % MAX_ONSETS;
        self.onsets[i] = o;
        if self.onsets_len < MAX_ONSETS {
            self.onsets_len += 1;
        } else {
            self.onsets_head = (self.onsets_head + 1) % MAX_ONSETS;
        }
    }

    /// Onsets, oldest first.
    pub(crate) fn onsets(&self) -> impl Iterator<Item = &Onset> + '_ {
        (0..self.onsets_len).map(move |i| &self.onsets[(self.onsets_head + i) % MAX_ONSETS])
    }

    /// The first onset at or after `pos` (input samples), if detected yet.
    pub(crate) fn next_after(&self, pos: i64) -> Option<Onset> {
        self.onsets().find(|o| o.pos >= pos).copied()
    }
}

/// `ln(x)` for `x >= 1`, within 2e-4 (a flux needs no more): exponent from the float's
/// bits, mantissa by a short polynomial.
#[inline]
fn fast_ln(x: f32) -> f32 {
    let bits = x.to_bits();
    let e = ((bits >> 23) & 0xff) as i32 - 127;
    let m = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000); // in [1, 2)
    // ln(m) on [1, 2): least-squares quartic in (m - 1), within 1.4e-4.
    let t = m - 1.0;
    let ln_m = t * (0.997_123_7 + t * (-0.470_003_4 + t * (0.224_309_7 - t * 0.058_413_7)));
    e as f32 * std::f32::consts::LN_2 + ln_m
}

#[cfg(test)]
mod tests {
    #[test]
    fn fast_ln_is_close() {
        for x in [1.0f32, 1.3, 2.0, 7.7, 100.0, 12345.0] {
            assert!((super::fast_ln(x) - x.ln()).abs() < 5e-4, "{x}");
        }
    }

    use super::*;

    const SR: f32 = 48_000.0;

    fn detector() -> OnsetDetector {
        OnsetDetector::new(SR, 512, 128, [550.0, 4500.0], 8192)
    }

    fn feed(d: &mut OnsetDetector, x: &[f32]) -> Vec<Onset> {
        let mut found = Vec::new();
        for chunk in x.chunks(100) {
            for &v in chunk {
                d.push(v);
            }
            d.run();
            for o in d.onsets() {
                if !found.contains(o) {
                    found.push(*o);
                }
            }
        }
        found
    }

    /// Decaying noise bursts over a quiet tone: every attack is found within 1.5 ms.
    #[test]
    fn finds_drum_like_attacks_precisely() {
        let len = SR as usize * 2;
        let hits = [4_000usize, 20_123, 33_000, 50_500, 71_234];
        let mut seed = 1u32;
        let mut rnd = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
        };
        let mut x: Vec<f32> = (0..len)
            .map(|i| 0.05 * (std::f32::consts::TAU * 220.0 * i as f32 / SR).sin())
            .collect();
        for &h in &hits {
            for i in 0..4800 {
                x[h + i] += 0.6 * rnd() * (-(i as f32) / 600.0).exp();
            }
        }
        let mut d = detector();
        let found = feed(&mut d, &x);
        assert_eq!(found.len(), hits.len(), "{found:?}");
        for (o, h) in found.iter().zip(hits) {
            assert!((o.pos - h as i64).abs() <= 72, "onset at {} vs {h}", o.pos);
            assert!(o.strength[2] > 1.0, "high band strength {:?}", o.strength);
        }
    }

    /// Steady tones (a chord with slow vibrato) have no onsets after the first.
    #[test]
    fn steady_chord_has_no_onsets() {
        let len = SR as usize * 2;
        let x: Vec<f32> = (0..len)
            .map(|i| {
                let t = i as f32 / SR;
                let vib = 1.0 + 0.004 * (std::f32::consts::TAU * 5.0 * t).sin();
                [261.6f32, 329.6, 392.0, 523.3]
                    .iter()
                    .map(|f| 0.15 * (std::f32::consts::TAU * f * vib * t).sin())
                    .sum::<f32>()
            })
            .collect();
        let mut d = detector();
        let found = feed(&mut d, &x);
        assert!(found.len() <= 1, "{found:?}");
    }

    /// A low thump (kick) is strongest in the low band.
    #[test]
    fn kick_is_a_low_band_onset() {
        let len = SR as usize;
        let mut x = vec![0.0f32; len];
        let at = 12_000;
        let mut ph = 0.0f32;
        for i in 0..30_000 {
            let f = 50.0 + 120.0 * (-(i as f32) / 900.0).exp();
            ph += std::f32::consts::TAU * f / SR;
            x[at + i] = 0.8 * ph.sin() * (-(i as f32) / 4000.0).exp();
        }
        let mut d = detector();
        let found = feed(&mut d, &x);
        assert_eq!(found.len(), 1, "{found:?}");
        let o = found[0];
        assert!((o.pos - at as i64).abs() <= 96, "{o:?}");
        assert!(o.strength[0] > 1.0, "{o:?}");
    }
}

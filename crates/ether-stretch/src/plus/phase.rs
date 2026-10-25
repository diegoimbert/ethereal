//! Phase-gradient heap integration (Průša & Holighaus, "Phase vocoder done right", EUSIPCO
//! 2017; Průša, Balazs & Søndergaard, "A noniterative method for reconstruction of phase from
//! STFT magnitude", IEEE TASLP 2017).
//!
//! The synthesis phase of a frame is integrated from the phase gradient: along time with the
//! instantaneous frequency (trapezoidal rule between the previous and the current frame),
//! along frequency with the local group delay. Bins are visited in decreasing order of
//! magnitude, so every bin takes its phase from its strongest already-known neighbour: the
//! peaks of the previous frame carry their phase forward in time, then each peak spreads its
//! phase across its lobe in frequency. This keeps partials coherent vertically (as identity
//! phase locking does) and reproduces impulses where they are in the analysis frame (the
//! group delay places them), without peak picking.
//!
//! Phases are kept as unit phasors: a frequency step with known analysis phases is then two
//! complex products and no trigonometry; only time steps take a `sin_cos`.

use realfft::num_complex::Complex32 as C;

/// Inputs of one frame, all indexed by output bin (`0..=N/2`).
pub(crate) struct Gradient<'a> {
    /// Output magnitudes (decide the visiting order).
    pub mag: &'a [f32],
    /// Instantaneous frequency (rad/sample).
    pub omega: &'a [f32],
    /// Group delay relative to the frame centre (samples; positive = later).
    pub tau: &'a [f32],
    /// Analysis phases (unit phasors), when known: the frequency step then copies the analysed phase
    /// difference between neighbours (identity phase locking, Laroche & Dolson, "Improved
    /// phase vocoder time-scale modification of audio", IEEE TSAP 1999), which is exact for
    /// a stationary partial's whole lobe, side lobes' sign flips included, and for an impulse;
    /// the group delay's integral only approximates it, and not across the zeros between
    /// side lobes.
    pub ana: Option<&'a [C]>,
}

/// The previous frame of the same resolution.
pub(crate) struct Previous<'a> {
    pub mag: &'a [f32],
    pub omega: &'a [f32],
    pub phase: &'a [C],
}

/// Reusable scratch for [`PhaseIntegrator::integrate`] (allocated once).
pub(crate) struct PhaseIntegrator {
    heap: Buckets,
    done: Vec<bool>,
}

const PREV_FLAG: u32 = 1 << 31;

/// Max-priority queue of bins by magnitude, quantised to [`BUCKET_DB`] (O(1) push and pop; the
/// order within a bucket doesn't matter at that resolution).
struct Buckets {
    head: Vec<u32>,
    next: Vec<u32>,
    val: Vec<u32>,
    len: usize,
    top: usize,
    log2_max: f32,
}

/// Bucket width and range (dB below the frame's peak; weaker bins share the last bucket).
const BUCKET_DB: f32 = 1.0;
const BUCKETS: usize = 100;
const NONE: u32 = u32::MAX;

impl Buckets {
    fn new(cap: usize) -> Self {
        Self {
            head: vec![NONE; BUCKETS],
            next: vec![NONE; cap],
            val: vec![0; cap],
            len: 0,
            top: 0,
            log2_max: 0.0,
        }
    }

    fn clear(&mut self, max: f32) {
        self.head.fill(NONE);
        self.len = 0;
        self.top = 0;
        self.log2_max = max.max(1e-30).log2();
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    fn push(&mut self, mag: f32, v: u32) {
        // log2 by the float's bits (Mitchell's approximation, within 0.09 octave-units:
        // finer than a bucket).
        let l2 = mag.max(0.0).to_bits() as f32 * (1.0 / (1u32 << 23) as f32) - 127.0;
        let db = (l2 - self.log2_max) * 6.0206;
        let b = ((BUCKETS - 1) as f32 + (db / BUCKET_DB).floor()).clamp(0.0, (BUCKETS - 1) as f32)
            as usize;
        let i = self.len;
        if i >= self.val.len() {
            return; // Never: capacity covers every bin twice.
        }
        self.val[i] = v;
        self.next[i] = self.head[b];
        self.head[b] = i as u32;
        self.len += 1;
        self.top = self.top.max(b);
    }

    #[inline]
    fn pop(&mut self) -> Option<u32> {
        loop {
            let i = self.head[self.top];
            if i != NONE {
                self.head[self.top] = self.next[i as usize];
                return Some(self.val[i as usize]);
            }
            if self.top == 0 {
                return None;
            }
            self.top -= 1;
        }
    }
}

impl PhaseIntegrator {
    /// Non-RT. Scratch for frames of up to `bins` bins.
    pub(crate) fn new(bins: usize) -> Self {
        Self {
            heap: Buckets::new(2 * bins + 8),
            done: vec![false; bins],
        }
    }

    crate::simd::avx2_dispatch! {
        /// RT. Integrate the phase (unit phasors) of bins `lo..hi` into `phase` (other bins
        /// untouched). `hop` is
        /// the synthesis hop (samples) since `prev`, `bin_step` the output bin spacing
        /// (rad/sample, `2π/N`). Bins of the previous frame below `tol` times the frame's largest
        /// magnitude don't seed the integration; if none does (a start after silence), the
        /// strongest current bin is seeded by time integration. Bins `fixed.0..fixed.1` already
        /// hold their phase (taken from another resolution): they seed the integration too.
        #[allow(clippy::too_many_arguments)]
        pub(crate) fn integrate => integrate_avx2, integrate_generic(
            &mut self,
            lo: usize,
            hi: usize,
            fixed: (usize, usize),
            hop: f32,
            bin_step: f32,
            tol: f32,
            cur: &Gradient<'_>,
            prev: &Previous<'_>,
            phase: &mut [C],
        )
    }

    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    fn integrate_generic(
        &mut self,
        lo: usize,
        hi: usize,
        fixed: (usize, usize),
        hop: f32,
        bin_step: f32,
        tol: f32,
        cur: &Gradient<'_>,
        prev: &Previous<'_>,
        phase: &mut [C],
    ) {
        if lo >= hi {
            return;
        }
        let time_step = |k: usize| prev.phase[k] * cis(hop * 0.5 * (prev.omega[k] + cur.omega[k]));
        let mut max = 0.0f32;
        for k in lo..hi {
            max = max.max(cur.mag[k]).max(prev.mag[k]);
        }
        let floor = max * tol;
        self.heap.clear(max);
        self.done[lo..hi].fill(false);
        for k in fixed.0.max(lo)..fixed.1.min(hi) {
            self.done[k] = true;
            self.heap.push(cur.mag[k], k as u32);
        }
        // The previous frame's peaks carry their phase forward; the bins around them are
        // reached faster through frequency (a time step for a non-peak bin would only lose
        // the race to its peak's spreading).
        for k in lo..hi {
            let m = prev.mag[k];
            let peak = (k == lo || m >= prev.mag[k - 1]) && (k + 1 == hi || m >= prev.mag[k + 1]);
            if peak && m > floor && !self.done[k] {
                self.heap.push(m, k as u32 | PREV_FLAG);
            }
        }
        if self.heap.is_empty() {
            let seed = (lo..hi)
                .max_by(|a, b| cur.mag[*a].total_cmp(&cur.mag[*b]))
                .unwrap_or(lo);
            phase[seed] = time_step(seed);
            self.done[seed] = true;
            self.heap.push(cur.mag[seed], seed as u32);
        }
        while let Some(e) = self.heap.pop() {
            let k = (e & !PREV_FLAG) as usize;
            if e & PREV_FLAG != 0 {
                if !self.done[k] {
                    phase[k] = time_step(k);
                    self.done[k] = true;
                    self.heap.push(cur.mag[k], k as u32);
                }
                continue;
            }
            // Spread across frequency: the analysed phase difference, or
            // φ(k±1) = φ(k) ∓ Δω·(τ(k) + τ(k±1))/2.
            let step = |j: usize, sign: f32| match cur.ana {
                Some(a) => a[j] * a[k].conj(),
                None => cis(-sign * bin_step * 0.5 * (cur.tau[k] + cur.tau[j])),
            };
            if k + 1 < hi && !self.done[k + 1] {
                let j = k + 1;
                phase[j] = phase[k] * step(j, 1.0);
                self.done[j] = true;
                self.heap.push(cur.mag[j], j as u32);
            }
            if k > lo && !self.done[k - 1] {
                let j = k - 1;
                phase[j] = phase[k] * step(j, -1.0);
                self.done[j] = true;
                self.heap.push(cur.mag[j], j as u32);
            }
        }
        // Everything is reached through frequency spreading; this only guards odd inputs
        // (NaN magnitudes).
        for (k, done) in self.done.iter().enumerate().take(hi).skip(lo) {
            if !done {
                phase[k] = time_step(k);
            }
        }
        // Keep the phasors unit (rounding drifts over long runs): one Newton step.
        for p in &mut phase[lo..hi] {
            *p *= 1.5 - 0.5 * p.norm_sqr();
        }
    }
}

/// `e^{ix}`.
#[inline]
pub(crate) fn cis(x: f32) -> C {
    let (s, c) = x.sin_cos();
    C::new(c, s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    /// `x` wrapped to `(-π, π]`.
    #[inline]
    fn wrap(x: f32) -> f32 {
        use std::f32::consts::{PI, TAU};
        if (-PI..=PI).contains(&x) {
            x
        } else {
            x - TAU * ((x + PI) / TAU).floor()
        }
    }

    #[test]
    fn wrap_bounds() {
        for x in [-20.0f32, -PI - 0.1, -1.0, 0.0, 3.0, PI + 0.1, 100.0] {
            let w = wrap(x);
            assert!(w > -PI - 1e-5 && w <= PI + 1e-5, "{x} -> {w}");
            let d = (x - w) / std::f32::consts::TAU;
            assert!((d - d.round()).abs() < 1e-4);
        }
    }

    /// A stationary partial: every bin of its lobe advances by ω·hop and the lobe stays
    /// phase-locked (equal phases across the main lobe, as the analysis of a centred frame).
    #[test]
    fn stationary_partial_advances_by_its_frequency_and_stays_locked() {
        let bins = 64;
        let omega0 = 2.0 * PI * 20.3 / 128.0;
        let mag: Vec<f32> = (0..bins)
            .map(|k| (1.0 - ((k as f32 - 20.3) / 2.0).powi(2)).max(1e-6))
            .collect();
        let omega = vec![omega0; bins];
        let tau = vec![0.0; bins];
        let prev_phase: Vec<C> = vec![cis(0.7); bins];
        let mut phase = vec![C::default(); bins];
        let mut pi = PhaseIntegrator::new(bins);
        let hop = 32.0;
        pi.integrate(
            0,
            bins,
            (0, 0),
            hop,
            2.0 * PI / 128.0,
            1e-4,
            &Gradient {
                mag: &mag,
                omega: &omega,
                tau: &tau,
                ana: None,
            },
            &Previous {
                mag: &mag,
                omega: &omega,
                phase: &prev_phase,
            },
            &mut phase,
        );
        let want = wrap(0.7 + omega0 * hop);
        for (k, ph) in phase.iter().enumerate().take(24).skip(18) {
            assert!((wrap(ph.arg() - want)).abs() < 1e-4, "bin {k}: {ph}");
        }
    }

    /// An impulse `t0` samples after the frame centre: the onset frame (silent previous
    /// frame) gets the linear phase `-2π k t0 / N` (up to a constant).
    #[test]
    fn impulse_gets_linear_phase_from_group_delay() {
        let n = 128;
        let bins = n / 2 + 1;
        let t0 = 9.0f32;
        let mag = vec![1.0; bins];
        let omega: Vec<f32> = (0..bins).map(|k| 2.0 * PI * k as f32 / n as f32).collect();
        let tau = vec![t0; bins];
        let zeros = vec![0.0; bins];
        let zero_phase = vec![C::new(1.0, 0.0); bins];
        let mut phase = vec![C::default(); bins];
        let mut pi = PhaseIntegrator::new(bins);
        pi.integrate(
            0,
            bins,
            (0, 0),
            32.0,
            2.0 * PI / n as f32,
            1e-4,
            &Gradient {
                mag: &mag,
                omega: &omega,
                tau: &tau,
                ana: None,
            },
            &Previous {
                mag: &zeros,
                omega: &omega,
                phase: &zero_phase,
            },
            &mut phase,
        );
        let slope = -2.0 * PI * t0 / n as f32;
        for k in 1..bins {
            let d = wrap(phase[k].arg() - phase[k - 1].arg() - slope);
            assert!(d.abs() < 1e-4, "bin {k}: step off by {d}");
        }
    }
}

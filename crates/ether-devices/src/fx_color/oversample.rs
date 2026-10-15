//! Linear-phase polyphase half-band oversampling (2x per stage) for the saturator.
//!
//! A half-band FIR (Kaiser-windowed sinc, `N = 4k + 3` taps, centre tap exactly 1/2) has
//! every other tap zero, so each polyphase branch is either a short FIR or a pure delay:
//! the upsampler computes one FIR output and one delayed input per input sample, the
//! downsampler one FIR over the even samples plus one delayed odd sample. Group delay is
//! `(N - 1) / 2` samples at the high rate for each filter, so an up + down pair delays by
//! `(N - 1) / 2` samples of the *low* rate.
//!
//! Buffers are fixed-size arrays (no allocation anywhere).

/// Taps of the first (base rate <-> 2x) stage.
pub(crate) const TAPS_1: usize = 47;
/// Taps of the second (2x <-> 4x) stage: it has twice the transition room.
pub(crate) const TAPS_2: usize = 23;
/// Non-zero even-phase taps of a `TAPS_1` / `TAPS_2` filter.
const EVEN_1: usize = TAPS_1.div_ceil(2);
const EVEN_2: usize = TAPS_2.div_ceil(2);

/// Kaiser beta of both designs (~ 80 dB stop band).
const KAISER_BETA: f64 = 8.0;

/// Base-rate latency of a 2x round trip (stage 1 up + down).
pub(crate) const LATENCY_2X: usize = (TAPS_1 - 1) / 2;
/// Base-rate latency of a 4x round trip: stage 1 plus stage 2 (`(TAPS_2 - 1) / 4` = 5.5
/// base samples), padded by one 2x-rate sample to a whole base sample.
pub(crate) const LATENCY_4X: usize = LATENCY_2X + (TAPS_2 - 1) / 4 + 1;

fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term, mut k) = (1.0, 1.0, 1.0);
    let q = x * x / 4.0;
    while term > 1e-12 * sum {
        term *= q / (k * k);
        sum += term;
        k += 1.0;
    }
    sum
}

/// Half-band taps (`n` odd, `(n - 3) % 4 == 0`): DC gain 1, centre tap 1/2, zero at even
/// offsets from the centre.
fn halfband(n: usize) -> Vec<f64> {
    let c = (n - 1) / 2;
    let i0b = bessel_i0(KAISER_BETA);
    let mut h: Vec<f64> = (0..n)
        .map(|i| {
            let d = i as f64 - c as f64;
            if i == c {
                return 0.0;
            }
            if (i as isize - c as isize) % 2 == 0 {
                return 0.0;
            }
            let sinc = (std::f64::consts::PI * d / 2.0).sin() / (std::f64::consts::PI * d);
            let r = d / c as f64;
            let w = bessel_i0(KAISER_BETA * (1.0 - r * r).max(0.0).sqrt()) / i0b;
            sinc * w
        })
        .collect();
    // Side taps sum to exactly 1/2 so the DC gain is 1 with the exact 1/2 centre.
    let side: f64 = h.iter().sum();
    for t in &mut h {
        *t *= 0.5 / side;
    }
    h[c] = 0.5;
    h
}

/// Even-index taps (the FIR branch) of a half-band design, as `f32`.
fn even_taps<const E: usize>(n: usize) -> [f32; E] {
    let h = halfband(n);
    let mut out = [0.0f32; E];
    for (k, o) in out.iter_mut().enumerate() {
        *o = h[2 * k] as f32;
    }
    out
}

/// A fixed-length history ring (doubled so a window is always contiguous).
#[derive(Clone, Copy)]
struct Ring<const L: usize, const D: usize> {
    buf: [f32; D],
    pos: usize,
}

impl<const L: usize, const D: usize> Ring<L, D> {
    const fn new() -> Self {
        Self {
            buf: [0.0; D],
            pos: 0,
        }
    }

    /// Push `x`; the window `[oldest .. newest]` of `L` samples is then `window()`.
    #[inline]
    fn push(&mut self, x: f32) {
        self.pos = if self.pos == 0 { L - 1 } else { self.pos - 1 };
        self.buf[self.pos] = x;
        self.buf[self.pos + L] = x;
    }

    /// Newest first: `window()[j]` is the sample pushed `j` pushes ago.
    #[inline]
    fn window(&self) -> &[f32] {
        &self.buf[self.pos..self.pos + L]
    }
}

/// One 2x stage (upsampler + downsampler state) with `E` even taps.
#[derive(Clone, Copy)]
pub(crate) struct Stage<const E: usize, const D: usize> {
    taps: [f32; E],
    /// Upsampler input history (base rate of this stage).
    up: Ring<E, D>,
    /// Downsampler even-sample history.
    down_even: Ring<E, D>,
    /// Downsampler odd-sample history.
    down_odd: Ring<E, D>,
}

impl<const E: usize, const D: usize> Stage<E, D> {
    fn new(taps: [f32; E]) -> Self {
        Self {
            taps,
            up: Ring::new(),
            down_even: Ring::new(),
            down_odd: Ring::new(),
        }
    }

    pub(crate) fn reset(&mut self) {
        self.up = Ring::new();
        self.down_even = Ring::new();
        self.down_odd = Ring::new();
    }

    /// Centre offset in even-phase samples: centre tap `c = 2·C + 1`.
    const C: usize = E - 1;

    #[inline]
    fn dot(taps: &[f32; E], w: &[f32]) -> f32 {
        let mut acc = 0.0f32;
        for (t, x) in taps.iter().zip(w) {
            acc += t * x;
        }
        acc
    }

    /// One input sample -> two output samples (gain 2 applied: unity passband).
    #[inline]
    pub(crate) fn up(&mut self, x: f32) -> [f32; 2] {
        self.up.push(x);
        let w = self.up.window();
        // y[2n] = Σ_k 2·h[2k]·x[n-k]; y[2n+1] = x[n - (c-1)/2] (centre tap 1/2 × 2).
        let even = 2.0 * Self::dot(&self.taps, w);
        let odd = w[Self::C / 2];
        [even, odd]
    }

    /// Two input samples (in time order) -> one output sample.
    #[inline]
    pub(crate) fn down(&mut self, pair: [f32; 2]) -> f32 {
        self.down_even.push(pair[0]);
        self.down_odd.push(pair[1]);
        // y[n] = Σ_k h[2k]·v[2n-2k] + 1/2·v[2n-c]; 2n-c is odd: the odd sample of the pair
        // pushed (c+1)/2 pairs ago.
        let fir = Self::dot(&self.taps, self.down_even.window());
        fir + 0.5 * self.down_odd.window()[Self::C.div_ceil(2)]
    }
}

pub(crate) type Stage1 = Stage<EVEN_1, { 2 * EVEN_1 }>;
pub(crate) type Stage2 = Stage<EVEN_2, { 2 * EVEN_2 }>;

/// Non-RT. A fresh stage-1 (base <-> 2x) filter pair.
pub(crate) fn stage1() -> Stage1 {
    Stage::new(even_taps::<EVEN_1>(TAPS_1))
}

/// Non-RT. A fresh stage-2 (2x <-> 4x) filter pair.
pub(crate) fn stage2() -> Stage2 {
    Stage::new(even_taps::<EVEN_2>(TAPS_2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halfband_shape() {
        for n in [TAPS_1, TAPS_2] {
            assert_eq!((n - 3) % 4, 0);
            let h = halfband(n);
            let c = (n - 1) / 2;
            assert_eq!(h[c], 0.5);
            assert!((h.iter().sum::<f64>() - 1.0).abs() < 1e-12);
            for (i, t) in h.iter().enumerate() {
                if i != c && (i as isize - c as isize) % 2 == 0 {
                    assert_eq!(*t, 0.0);
                }
            }
        }
    }

    /// Round trip of a low sine is the input delayed by the documented latency.
    fn round_trip_delay(four: bool) -> usize {
        let mut s1 = stage1();
        let mut s2 = stage2();
        let mut pad = 0.0f32;
        let n = 4000;
        let f = 1000.0 / 48_000.0;
        let x: Vec<f32> = (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * f * i as f32).sin())
            .collect();
        let mut y = vec![0.0f32; n];
        for i in 0..n {
            let [a, b] = s1.up(x[i]);
            let (a, b) = if four {
                let [a0, a1] = s2.up(a);
                let [b0, b1] = s2.up(b);
                let a = s2.down([a0, a1]);
                let b = s2.down([b0, b1]);
                // One 2x-sample pad.
                let out = (pad, a);
                pad = b;
                out
            } else {
                (a, b)
            };
            y[i] = s1.down([a, b]);
        }
        let lat = if four { LATENCY_4X } else { LATENCY_2X };
        let err = (lat + 100..n)
            .map(|i| (y[i] - x[i - lat]).abs())
            .fold(0.0f32, f32::max);
        assert!(err < 1e-3, "four={four}: max err {err}");
        lat
    }

    #[test]
    fn round_trips_are_pure_delays() {
        round_trip_delay(false);
        round_trip_delay(true);
    }
}

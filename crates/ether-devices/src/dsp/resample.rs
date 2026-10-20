//! Band-limited fractional reads for sample playback (sampler, multisampler).
//!
//! A sample played `rate` times faster (pitched up) must lose everything above
//! `Nyquist / rate` before it is read, or that content folds back as aliasing: plain linear
//! or Hermite interpolation of a bright sample two octaves up aliased at -19 dB. [`Kernel`]
//! is a Kaiser-windowed sinc whose cutoff follows the rate: `min(1, 1 / rate)` of the
//! source Nyquist, [`ZEROS`] zero crossings each side, so it spans `2 · ZEROS · max(rate,
//! 1)` source frames (up to `2 · MAX_HALF` at rate 16, where both samplers cap the rate).
//! At rate 1 it is exactly the identity on
//! whole frames (the kernel is 1 at 0 and 0 at every other integer), so unpitched notes
//! play bit-exact.
//!
//! The kernel is tabulated once per process ([`Kernel::get`], non-RT on first call):
//! reads are table lookups with linear interpolation, no trig.

use std::f64::consts::PI;
use std::sync::OnceLock;

/// Zero crossings of the kernel on each side.
const ZEROS: usize = 8;
/// Table points per zero crossing.
const RES: usize = 512;
/// Kaiser window shape (about -80 dB sidelobes).
const BETA: f64 = 8.0;
/// Widest kernel half-span in source frames: the full kernel up to rate 16 (4 octaves up,
/// both samplers' cap). A faster read would squeeze the kernel into this span (cutoff
/// stuck at 1/16 of Nyquist).
pub(crate) const MAX_HALF: usize = 128;

/// The tabulated kernel `sinc(x) · kaiser(x / ZEROS)` for `x` in `0..=ZEROS`.
pub(crate) struct Kernel {
    table: Vec<f32>,
}

/// One playback rate's read setup: the cutoff (`scale`, in source-frame units) and the span.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Taps {
    /// Table index per source frame (`scale · RES`, widened when the span is capped).
    step: f64,
    /// Half-span in source frames.
    half: usize,
}

impl Taps {
    /// Setup for reading at `rate` source frames per output frame.
    pub(crate) fn new(rate: f64) -> Self {
        let rate = rate.abs().max(1.0);
        let scale = 1.0 / rate;
        let ideal = (ZEROS as f64 * rate).ceil() as usize;
        let half = ideal.min(MAX_HALF);
        // A capped span: squeeze the window (not the sinc) into it.
        let squeeze = ideal as f64 / half as f64;
        Self {
            step: scale * RES as f64 * squeeze,
            half,
        }
    }

    /// Source frames needed on each side of a read position (`floor(pos) - margin ..=
    /// floor(pos) + margin`).
    pub(crate) fn margin(&self) -> usize {
        self.half + 1
    }
}

impl Kernel {
    /// The shared table (built on first use: call it from a constructor, never first on the
    /// audio thread).
    pub(crate) fn get() -> &'static Kernel {
        static K: OnceLock<Kernel> = OnceLock::new();
        K.get_or_init(Kernel::build)
    }

    fn build() -> Self {
        let n = ZEROS * RES;
        let i0 = bessel_i0(BETA);
        let mut table = vec![0.0f32; n + 2];
        for (i, t) in table.iter_mut().enumerate().take(n + 1) {
            let x = i as f64 / RES as f64;
            let sinc = if i == 0 {
                1.0
            } else if i % RES == 0 {
                // Exactly 0 at the integers: rate 1 is the identity on whole frames.
                0.0
            } else {
                (PI * x).sin() / (PI * x)
            };
            let r = x / ZEROS as f64;
            let w = bessel_i0(BETA * (1.0 - r * r).max(0.0).sqrt()) / i0;
            *t = (sinc * w) as f32;
        }
        Self { table }
    }

    /// The kernel weights for a read at fractional index `idx` (frame `k` of the buffer
    /// read is at index `k`), normalized to a sum of 1 (DC and ramps pass exactly, whatever
    /// the fraction).
    #[inline]
    pub(crate) fn weights(&self, idx: f64, taps: &Taps, out: &mut Weights) {
        let base = idx.floor();
        let frac = (idx - base) as f32;
        if frac == 0.0 && taps.step == RES as f64 {
            // A whole frame at rate <= 1: the kernel is 1 there and 0 at the other taps.
            out.first = base as i64;
            out.n = 1;
            out.w[0] = 1.0;
            out.gain = 1.0;
            return;
        }
        let half = taps.half;
        out.first = base as i64 - half as i64 + 1;
        out.n = 2 * half;
        let step = taps.step as f32;
        let last = (self.table.len() - 2) as f32;
        let table = &self.table[..];
        // Tap distances in table steps grow by `step` away from the read position.
        let lerp = |d: f32| {
            if d < last {
                let i = d as usize;
                let a = table[i];
                a + (table[i + 1] - a) * (d - i as f32)
            } else {
                0.0
            }
        };
        let mut sum = 0.0f32;
        // Left of the read position: taps half-1, half-2, ... 0.
        let mut d = frac * step;
        for w in out.w[..half].iter_mut().rev() {
            *w = lerp(d);
            sum += *w;
            d += step;
        }
        // Right: taps half, half+1, ...
        let mut d = (1.0 - frac) * step;
        for w in out.w[half..2 * half].iter_mut() {
            *w = lerp(d);
            sum += *w;
            d += step;
        }
        out.gain = if sum != 0.0 { 1.0 / sum } else { 0.0 };
    }

    /// The value of `buf` at fractional index `idx`, read with `taps`.
    #[cfg(test)]
    pub(crate) fn read(&self, buf: &[f32], idx: f64, taps: &Taps) -> f32 {
        let mut w = Weights::new();
        self.weights(idx, taps, &mut w);
        w.apply(buf)
    }
}

/// The weights of one read (see [`Kernel::weights`]): computed once per output frame,
/// applied to each channel.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Weights {
    w: [f32; 2 * MAX_HALF],
    /// Buffer index of the first tap.
    first: i64,
    n: usize,
    /// Normalization (1 / the weights' sum).
    gain: f32,
}

impl Weights {
    pub(crate) const fn new() -> Self {
        Self {
            w: [0.0; 2 * MAX_HALF],
            first: 0,
            n: 0,
            gain: 0.0,
        }
    }

    /// The weighted sum of `buf` (frames outside it count as 0).
    #[inline]
    pub(crate) fn apply(&self, buf: &[f32]) -> f32 {
        let lo = self.first.max(0);
        let hi = (self.first + self.n as i64).min(buf.len() as i64);
        if lo >= hi {
            return 0.0;
        }
        let (lo, hi) = (lo as usize, hi as usize);
        let off = (lo as i64 - self.first) as usize;
        let acc: f32 = buf[lo..hi]
            .iter()
            .zip(&self.w[off..off + (hi - lo)])
            .map(|(x, w)| x * w)
            .sum();
        // Exact (no rounding) when the weights already sum to 1: rate 1 on whole frames.
        if self.gain == 1.0 {
            acc
        } else {
            acc * self.gain
        }
    }
}

/// Modified Bessel function of the first kind, order 0 (series; non-RT).
fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let q = x * x / 4.0;
    for k in 1..50 {
        term *= q / (k * k) as f64;
        sum += term;
        if term < 1e-12 * sum {
            break;
        }
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_one_is_the_identity_on_whole_frames() {
        let k = Kernel::get();
        let buf: Vec<f32> = (0..64).map(|i| ((i * 7919) % 13) as f32 - 6.0).collect();
        let taps = Taps::new(1.0);
        for i in 8..56 {
            assert_eq!(k.read(&buf, i as f64, &taps), buf[i]);
        }
    }

    #[test]
    fn fractional_reads_of_a_low_sine_are_accurate() {
        let k = Kernel::get();
        let w = 2.0 * PI * 1000.0 / 48_000.0;
        let buf: Vec<f32> = (0..256).map(|i| (w * i as f64).sin() as f32).collect();
        for rate in [0.5, 1.0, 1.5, 3.0] {
            let taps = Taps::new(rate);
            for j in 0..40 {
                let x = 100.0 + j as f64 * 0.37;
                let want = (w * x).sin() as f32;
                let got = k.read(&buf, x, &taps);
                assert!((got - want).abs() < 2e-3, "rate {rate}: {got} vs {want}");
            }
        }
    }

    #[test]
    fn the_cutoff_follows_the_rate() {
        // A 20 kHz sine read 4x faster would land at 80 kHz: it must vanish, not alias.
        let k = Kernel::get();
        let w = 2.0 * PI * 20_000.0 / 48_000.0;
        let buf: Vec<f32> = (0..1024).map(|i| (w * i as f64).sin() as f32).collect();
        let taps = Taps::new(4.0);
        let peak = (0..100)
            .map(|j| k.read(&buf, 300.0 + j as f64 * 4.0 + 0.3, &taps).abs())
            .fold(0.0f32, f32::max);
        assert!(peak < 3e-3, "{peak}");
    }
}

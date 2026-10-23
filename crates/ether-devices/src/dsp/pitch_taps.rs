//! The two-tap delay-line pitch shifter (Shifter's `Pitch` mode, the Reverb's shimmer).
//!
//! A short delay line read by two taps half a window apart; their delay sweeps across the
//! window at the speed that transposes by the ratio (a tap read faster than the line fills
//! rises in pitch) and jumps back when it reaches the end. Each tap is weighted by `sin²` of
//! its position in the sweep, so the jump happens at zero weight and the two weights always
//! sum to one (the output is a convex mix of two delayed samples: it never exceeds the
//! input's peak). The line is allocated in `allocate`; the per-sample calls allocate nothing.

use std::f64::consts::PI;

/// Crossfade of a window change (ms).
const WINDOW_FADE_MS: f64 = 20.0;
/// Samples kept between a read and the write head (the Hermite read's look-ahead).
const GUARD: f64 = 3.0;

/// A sweep of the two taps: tap 1's position in `[0, 1)` (tap 2 is half a sweep ahead)
/// across `window` samples.
#[derive(Clone, Copy, Debug)]
struct Sweep {
    phase: f64,
    window: f64,
}

/// The two-tap delay-line pitch shifter of one channel. A `Window` change crossfades from
/// the old sweep to a new one (moving the taps' delay instead would bend the pitch).
#[derive(Debug)]
pub(crate) struct PitchTaps {
    line: Vec<f32>,
    write: usize,
    sweep: Sweep,
    /// The sweep being faded out, `fade` of `fade_len` samples left.
    old: Sweep,
    fade: u32,
    fade_len: u32,
}

impl PitchTaps {
    pub(crate) fn new() -> Self {
        let sweep = Sweep {
            phase: 0.0,
            window: 0.0,
        };
        Self {
            line: Vec::new(),
            write: 0,
            sweep,
            old: sweep,
            fade: 0,
            fade_len: 1,
        }
    }

    /// Non-RT. Size the line for windows up to `max_window_ms` at `sr` and start at
    /// `window` samples.
    pub(crate) fn allocate(&mut self, sr: f64, window: f64, max_window_ms: f64) {
        let n = (max_window_ms * 0.001 * sr).ceil() as usize + 16;
        if self.line.len() != n {
            self.line = vec![0.0; n];
        }
        self.fade_len = ((WINDOW_FADE_MS * 0.001 * sr).round() as u32).max(1);
        self.sweep.window = window;
        self.reset();
    }

    pub(crate) fn reset(&mut self) {
        self.line.fill(0.0);
        self.write = 0;
        self.sweep.phase = 0.0;
        self.old = self.sweep;
        self.fade = 0;
    }

    /// Advance `s` for `ratio` and read its two taps.
    #[inline]
    fn taps(&self, s: &mut Sweep, ratio: f64) -> f32 {
        // The delay changes by `1 - ratio` per sample: a sweep of `window` samples.
        s.phase -= (ratio - 1.0) / s.window;
        s.phase -= s.phase.floor();
        let max = (self.line.len() as f64) - GUARD - 3.0;
        let p2 = (s.phase + 0.5).fract();
        // Tap 1 weighs sin²(πp), tap 2 sin²(π(p + ½)) = 1 - sin²(πp).
        let w = (PI * s.phase).sin();
        let w1 = (w * w) as f32;
        let a = self.read((GUARD + s.phase * s.window).min(max));
        let b = self.read((GUARD + p2 * s.window).min(max));
        b + (a - b) * w1
    }

    /// 4-point Hermite read `d` (≥ 1) samples behind the newest sample.
    #[inline]
    fn read(&self, d: f64) -> f32 {
        let line = &self.line;
        let n = line.len();
        let i = d as usize;
        let t = (d - i as f64) as f32;
        // Index of the sample `i - 1` behind the newest, then walk back (`d` < n - 3).
        let mut k = self.write + n - (i - 1);
        if k >= n {
            k -= n;
        }
        let back = |k: usize| if k == 0 { n - 1 } else { k - 1 };
        let xm1 = line[k];
        let k = back(k);
        let x0 = line[k];
        let k = back(k);
        let x1 = line[k];
        let x2 = line[back(k)];
        let c1 = 0.5 * (x1 - xm1);
        let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
        let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
        ((c3 * t + c2) * t + c1) * t + x0
    }

    /// Write the next input sample (every sample, whatever the mode).
    #[inline]
    pub(crate) fn push(&mut self, x: f32) {
        self.write += 1;
        if self.write == self.line.len() {
            self.write = 0;
        }
        self.line[self.write] = if x.abs() < 1e-20 { 0.0 } else { x };
    }

    /// Read at `ratio` (after [`Self::push`]); a new `window` (samples) starts a
    /// crossfade once the previous one is over.
    #[inline]
    pub(crate) fn pitched(&mut self, ratio: f64, window: f64) -> f32 {
        if self.fade == 0 && window != self.sweep.window {
            self.old = self.sweep;
            self.sweep.window = window;
            self.fade = self.fade_len;
        }
        let mut sweep = self.sweep;
        let mut out = self.taps(&mut sweep, ratio);
        self.sweep = sweep;
        if self.fade > 0 {
            self.fade -= 1;
            let mut old = self.old;
            let o = self.taps(&mut old, ratio);
            self.old = old;
            // Equal-gain: the two sweeps read the same signal at nearby delays.
            let t = self.fade as f32 / self.fade_len as f32;
            out += (o - out) * t;
        }
        out
    }

    pub(crate) fn settled(&self) -> bool {
        self.fade == 0
    }

    /// The line's length (samples a signal can stay in it).
    pub(crate) fn len(&self) -> usize {
        self.line.len()
    }

    /// Start the sweep at `phase` (`0..1`; two channels a quarter apart decorrelate).
    pub(crate) fn set_phase(&mut self, phase: f64) {
        self.sweep.phase = phase - phase.floor();
        self.old = self.sweep;
    }
}

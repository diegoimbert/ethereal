//! Mono circular delay line with fractional (linear-interpolated) reads.

/// Fixed-capacity delay line (allocated once, never resized in `process`).
#[derive(Clone, Debug, Default)]
pub(crate) struct DelayLine {
    buf: Vec<f32>,
    write: usize,
}

impl DelayLine {
    /// Non-RT. A line that can delay by up to `max_delay` samples.
    pub(crate) fn new(max_delay: usize) -> Self {
        Self {
            buf: vec![0.0; max_delay + 2],
            write: 0,
        }
    }

    pub(crate) fn clear(&mut self) {
        self.buf.fill(0.0);
        self.write = 0;
    }

    /// Largest usable delay in samples.
    pub(crate) fn max_delay(&self) -> usize {
        self.buf.len().saturating_sub(2)
    }

    /// Push one sample (call once per frame, after the reads of that frame).
    #[inline]
    pub(crate) fn push(&mut self, x: f32) {
        self.buf[self.write] = super::flush32(x);
        self.write += 1;
        if self.write == self.buf.len() {
            self.write = 0;
        }
    }

    /// Sample written `delay` pushes ago (`delay >= 1`; 1 = the previous push).
    #[inline]
    pub(crate) fn tap(&self, delay: usize) -> f32 {
        let len = self.buf.len();
        let d = delay.clamp(1, len - 1);
        let idx = if self.write >= d {
            self.write - d
        } else {
            self.write + len - d
        };
        self.buf[idx]
    }

    /// Fractional read `delay` samples back (`1.0 ..= max_delay`).
    #[inline]
    pub(crate) fn read(&self, delay: f32) -> f32 {
        self.read_split(self.split(delay))
    }

    /// `delay` as [`Self::read`] uses it: whole samples and fraction, clamped to the line.
    /// Lets a caller whose delay doesn't change read with [`Self::read_split`] for many
    /// samples (the same reads).
    #[inline]
    pub(crate) fn split(&self, delay: f32) -> (usize, f32) {
        let d = delay.clamp(1.0, self.max_delay() as f32);
        let i = d as usize;
        (i, d - i as f32)
    }

    /// [`Self::read`] at a delay already [`Self::split`].
    #[inline]
    pub(crate) fn read_split(&self, (i, frac): (usize, f32)) -> f32 {
        let a = self.tap(i);
        let b = self.tap(i + 1);
        a + (b - a) * frac
    }
}

/// Crossfade length of a [`LookaheadTap`] change (samples; 10 ms at 48 kHz).
const LOOKAHEAD_FADE: u32 = 480;

/// The audio tap of a lookahead (`Lookahead` of the compressor and the gate): a change of
/// its length crossfades from the old tap to the new one instead of jumping the read
/// position (a jump is a click).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct LookaheadTap {
    current: usize,
    previous: usize,
    fade: u32,
}

impl LookaheadTap {
    /// Set the lookahead to `n` samples: crossfaded (`smooth`) or at once.
    pub(crate) fn set(&mut self, n: usize, smooth: bool) {
        if !smooth {
            self.fade = 0;
        } else if n != self.current {
            // Fade out whichever tap is louder right now (a change during a fade).
            if self.fade <= LOOKAHEAD_FADE / 2 {
                self.previous = self.current;
            }
            self.fade = LOOKAHEAD_FADE;
        }
        self.current = n;
    }

    /// The lookahead in samples (the latency).
    pub(crate) fn len(&self) -> usize {
        self.current
    }

    /// Whether audio goes through the lines (a lookahead, or a fade away from one).
    pub(crate) fn active(&self) -> bool {
        self.current > 0 || self.fade > 0
    }

    /// The delayed sample of `line` (after this frame's push).
    #[inline]
    pub(crate) fn read(&self, line: &DelayLine) -> f32 {
        let x = line.tap(self.current + 1);
        if self.fade == 0 {
            return x;
        }
        let t = self.fade as f32 / LOOKAHEAD_FADE as f32;
        x + (line.tap(self.previous + 1) - x) * t
    }

    /// Advance the fade by one frame (after every channel's [`Self::read`]).
    #[inline]
    pub(crate) fn advance(&mut self) {
        self.fade = self.fade.saturating_sub(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taps_and_fractional_reads() {
        let mut d = DelayLine::new(8);
        for i in 1..=5 {
            d.push(i as f32);
        }
        assert_eq!(d.tap(1), 5.0);
        assert_eq!(d.tap(3), 3.0);
        assert!((d.read(1.5) - 4.5).abs() < 1e-6);
        for i in 6..=30 {
            d.push(i as f32);
        }
        assert_eq!(d.tap(8), 23.0);
    }

    #[test]
    fn lookahead_changes_crossfade() {
        let mut line = DelayLine::new(64);
        let mut tap = LookaheadTap::default();
        tap.set(10, false);
        let mut out = Vec::new();
        for i in 0..2000 {
            line.push(i as f32);
            if i == 1000 {
                tap.set(40, true);
            }
            out.push(tap.read(&line));
            tap.advance();
        }
        // Before: 10 behind; long after: 40 behind; in between no step larger than the
        // ramp of the two taps (never the 30-sample jump).
        assert_eq!(out[900], 890.0);
        assert_eq!(out[1999], 1959.0);
        assert!(out.windows(2).all(|w| (w[1] - w[0]).abs() < 1.1), "jump");
    }
}

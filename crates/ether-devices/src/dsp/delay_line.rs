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
        let d = delay.clamp(1.0, self.max_delay() as f32);
        let i = d as usize;
        let frac = d - i as f32;
        let a = self.tap(i);
        let b = self.tap(i + 1);
        a + (b - a) * frac
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
}

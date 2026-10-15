//! Fixed delay lines used for plugin delay compensation (PDC).

/// Stereo integer-sample delay. Allocated off-thread with its final length; processing is
/// in-place and allocation-free.
#[derive(Debug, Default)]
pub(crate) struct DelayLine {
    buf: [Vec<f32>; 2],
    pos: usize,
}

impl DelayLine {
    /// Non-RT.
    pub(crate) fn new(delay: usize) -> Self {
        Self {
            buf: [vec![0.0; delay], vec![0.0; delay]],
            pos: 0,
        }
    }

    pub(crate) fn delay(&self) -> usize {
        self.buf[0].len()
    }

    /// RT. Delay `left`/`right` in place (both the same length).
    pub(crate) fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let len = self.delay();
        if len == 0 {
            return;
        }
        let mut pos = self.pos;
        let [bl, br] = &mut self.buf;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            std::mem::swap(l, &mut bl[pos]);
            std::mem::swap(r, &mut br[pos]);
            pos += 1;
            if pos == len {
                pos = 0;
            }
        }
        self.pos = pos;
    }

    /// Keep the audio in flight when a republished graph has the same delay.
    pub(crate) fn inherit(&mut self, old: &mut DelayLine) {
        if old.delay() == self.delay() {
            std::mem::swap(&mut self.buf, &mut old.buf);
            self.pos = old.pos;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delays_by_n() {
        let mut d = DelayLine::new(3);
        let mut l = [1.0, 2.0, 3.0, 4.0, 5.0];
        let mut r = [1.0, 0.0, 0.0, 0.0, 0.0];
        d.process(&mut l, &mut r);
        assert_eq!(l, [0.0, 0.0, 0.0, 1.0, 2.0]);
        assert_eq!(r, [0.0, 0.0, 0.0, 1.0, 0.0]);
        let mut l = [0.0; 2];
        let mut r = [0.0; 2];
        d.process(&mut l, &mut r);
        assert_eq!(l, [3.0, 4.0]);
    }

    #[test]
    fn zero_delay_is_identity() {
        let mut d = DelayLine::new(0);
        let mut l = [1.0, 2.0];
        let mut r = [3.0, 4.0];
        d.process(&mut l, &mut r);
        assert_eq!((l, r), ([1.0, 2.0], [3.0, 4.0]));
    }
}

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

/// A device's bypass path (`crate::mixer::ChainRt`): its input, delayed by its latency, kept
/// readable after the input buffer is overwritten (by the device or a rack's chains). A
/// ring of `delay + max_block` frames: [`Self::push`] writes a sub-block, [`Self::get`]
/// reads its delayed samples. Allocated off-thread; RT use never allocates.
#[derive(Debug, Default)]
pub(crate) struct DryLine {
    buf: [Vec<f32>; 2],
    delay: usize,
    /// Ring position just after the last pushed frame.
    pos: usize,
    /// Frames of the last push.
    last: usize,
}

impl DryLine {
    /// Non-RT.
    pub(crate) fn new(delay: usize, max_block: usize) -> Self {
        let len = delay + max_block.max(1);
        Self {
            buf: [vec![0.0; len], vec![0.0; len]],
            delay,
            pos: 0,
            last: 0,
        }
    }

    pub(crate) fn delay(&self) -> usize {
        self.delay
    }

    /// Whether this line exists (entries without audio outputs have none).
    #[inline]
    pub(crate) fn active(&self) -> bool {
        !self.buf[0].is_empty()
    }

    /// RT. Store the next `n` frames of `input` (`n <= max_block`).
    pub(crate) fn push(&mut self, input: &[Vec<f32>; 2], n: usize) {
        let len = self.buf[0].len();
        if len == 0 {
            return;
        }
        let n = n.min(len - self.delay);
        for (b, x) in self.buf.iter_mut().zip(input) {
            let head = n.min(len - self.pos);
            b[self.pos..self.pos + head].copy_from_slice(&x[..head]);
            b[..n - head].copy_from_slice(&x[head..n]);
        }
        self.pos = (self.pos + n) % len;
        self.last = n;
    }

    /// RT. Channel `ch`, frame `i` of the last pushed sub-block, delayed.
    #[inline]
    pub(crate) fn get(&self, ch: usize, i: usize) -> f32 {
        let len = self.buf[0].len();
        // pos - last + i - delay (mod len); `len > delay + last` keeps it unwritten since.
        let at = (self.pos + 2 * len + i - self.last - self.delay) % len;
        self.buf[ch][at]
    }

    /// Keep the audio in flight when a republished graph has the same delay.
    pub(crate) fn inherit(&mut self, old: &mut DryLine) {
        if old.delay == self.delay && old.buf[0].len() == self.buf[0].len() {
            std::mem::swap(&mut self.buf, &mut old.buf);
            self.pos = old.pos;
            self.last = old.last;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dry_line_delays_across_sub_blocks() {
        let mut d = DryLine::new(3, 4);
        let mut out = Vec::new();
        for block in [
            [1.0, 2.0, 3.0, 4.0],
            [5.0, 6.0, 7.0, 8.0],
            [9.0, 10.0, 11.0, 12.0],
        ] {
            d.push(&[block.to_vec(), block.to_vec()], 4);
            out.extend((0..4).map(|i| d.get(0, i)));
        }
        assert_eq!(
            out,
            [0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]
        );
        // Short sub-blocks too.
        d.push(&[vec![13.0], vec![13.0]], 1);
        assert_eq!(d.get(1, 0), 10.0);
    }

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

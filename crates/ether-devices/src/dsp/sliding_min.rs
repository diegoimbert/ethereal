//! Running minimum over the last `window` values (monotonic wedge, amortized O(1)).

/// Minimum of the last `window` pushed values. Capacity is fixed at construction.
#[derive(Clone, Debug, Default)]
pub(crate) struct SlidingMin {
    /// Ring of `(index, value)` with strictly increasing values from front to back.
    ring: Vec<(u64, f32)>,
    head: usize,
    len: usize,
    window: u64,
    count: u64,
}

impl SlidingMin {
    /// Non-RT.
    pub(crate) fn new(window: usize) -> Self {
        let window = window.max(1);
        Self {
            ring: vec![(0, 0.0); window + 1],
            head: 0,
            len: 0,
            window: window as u64,
            count: 0,
        }
    }

    pub(crate) fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
        self.count = 0;
    }

    #[inline]
    fn slot(&self, i: usize) -> usize {
        (self.head + i) % self.ring.len()
    }

    /// Push `x` and return the minimum of the last `window` values (fewer at the start).
    #[inline]
    pub(crate) fn push(&mut self, x: f32) -> f32 {
        // Drop larger-or-equal values from the back: they can never be the minimum again.
        while self.len > 0 && self.ring[self.slot(self.len - 1)].1 >= x {
            self.len -= 1;
        }
        let s = self.slot(self.len);
        self.ring[s] = (self.count, x);
        self.len += 1;
        // Drop expired values from the front.
        let oldest = (self.count + 1).saturating_sub(self.window);
        while self.ring[self.head].0 < oldest {
            self.head = (self.head + 1) % self.ring.len();
            self.len -= 1;
        }
        self.count += 1;
        self.ring[self.head].1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_naive_minimum() {
        let w = 5;
        let mut m = SlidingMin::new(w);
        let xs: Vec<f32> = (0..200)
            .map(|i| ((i * 37 % 23) as f32 - 11.0).abs())
            .collect();
        for (i, &x) in xs.iter().enumerate() {
            let got = m.push(x);
            let lo = (i + 1).saturating_sub(w);
            let want = xs[lo..=i].iter().copied().fold(f32::INFINITY, f32::min);
            assert_eq!(got, want, "at {i}");
        }
    }
}

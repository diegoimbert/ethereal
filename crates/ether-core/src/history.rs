//! Timeline history (issue #270): which part of the timeline the engine rendered at each
//! recent sample of its clock, so automation can be read where the audio it affects was.
//!
//! With plugin delay compensation, the signal reaching a device after a latent one (or a
//! fader, a bus) is `d` samples late relative to the timeline. Its automation must be
//! delayed by the same `d`: at sample `s` of the engine clock the device gets the value
//! the timeline had at `s - d`, through loops, locates and tempo changes exactly as the
//! audio went. [`TimelineHistory`] keeps the recent sub-block plans (one per
//! [`crate::engine::SubPlan`], pushed when planned) and cuts any window of the clock into
//! pieces of those plans ([`TimelineHistory::pieces`]).
//!
//! RT-safe: a fixed ring allocated with the engine. A delay longer than the ring's reach
//! (thousands of sub-blocks) reads the oldest plan's start, held.

use crate::engine::SubPlan;

/// Plans kept (at 64-frame sub-blocks and 48 kHz, ~5.5 s of history; ~44 s at 512).
pub(crate) const HISTORY_LEN: usize = 4096;

pub(crate) struct TimelineHistory {
    plans: Box<[SubPlan]>,
    /// Index of the oldest plan and number of plans (contiguous on the sample clock).
    start: usize,
    len: usize,
}

impl TimelineHistory {
    /// Non-RT.
    pub(crate) fn new() -> Self {
        Self::with_capacity(HISTORY_LEN)
    }

    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            plans: vec![SubPlan::default(); capacity.max(1)].into_boxed_slice(),
            start: 0,
            len: 0,
        }
    }

    fn at(&self, i: usize) -> &SubPlan {
        &self.plans[(self.start + i) % self.plans.len()]
    }

    /// RT. Record a plan. Plans at or after its sample time are dropped first (anticipative
    /// processing re-plans them after a resync).
    pub(crate) fn push(&mut self, plan: &SubPlan) {
        while self.len > 0 && self.at(self.len - 1).sample_time >= plan.sample_time {
            self.len -= 1;
        }
        let cap = self.plans.len();
        if self.len == cap {
            self.start = (self.start + 1) % cap;
            self.len -= 1;
        }
        self.plans[(self.start + self.len) % cap] = *plan;
        self.len += 1;
    }

    /// RT. Cut the clock window `[from, from + n)` into pieces of the recorded plans:
    /// `f(at, piece)` with `at` the piece's offset in the window, in order, covering it
    /// whole. Before the oldest plan (or the engine's start) the timeline is read as
    /// stopped at the oldest plan's start; past the newest, at the newest's end.
    pub(crate) fn pieces(
        &self,
        from: i64,
        n: usize,
        tempo: &crate::tempo::TempoMapRt,
        sr: f64,
        mut f: impl FnMut(usize, &SubPlan),
    ) {
        if n == 0 {
            return;
        }
        if self.len == 0 {
            f(0, &SubPlan::stopped(from.max(0) as u64, n, 0.0, 0.0));
            return;
        }
        let end = from + n as i64;
        let first = self.at(0);
        let mut at = 0usize;
        // Before the history.
        let oldest = first.sample_time as i64;
        if from < oldest {
            let len = ((oldest.min(end) - from) as usize).min(n);
            f(
                0,
                &SubPlan::stopped(from.max(0) as u64, len, first.b0, first.s0),
            );
            at = len;
        }
        // The first plan ending after the window's (remaining) start.
        let pos = from + at as i64;
        let mut i = {
            let (mut lo, mut hi) = (0, self.len);
            while lo < hi {
                let mid = (lo + hi) / 2;
                let p = self.at(mid);
                if p.sample_time as i64 + p.n as i64 <= pos {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            lo
        };
        while at < n && i < self.len {
            let p = self.at(i);
            let p0 = p.sample_time as i64;
            let w0 = from + at as i64;
            let skip = (w0 - p0).max(0) as usize;
            let take = (p.n - skip).min(n - at);
            let mut piece = *p;
            if skip > 0 {
                piece = crate::anticipation::split(&piece, skip, tempo, sr).1;
            }
            if take < piece.n {
                piece = crate::anticipation::split(&piece, take, tempo, sr).0;
            }
            f(at, &piece);
            at += take;
            i += 1;
        }
        if at < n {
            // Past the newest plan (never while rendering: the current plan is recorded).
            let last = self.at(self.len - 1);
            let t = last.sample_time + last.n as u64;
            f(at, &SubPlan::stopped(t, n - at, last.b1, last.s0));
        }
    }
}

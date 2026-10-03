//! [`FillPolicy`]: which chunk the writer decodes next, and into which slot.
//!
//! Shared by both hosts: the native disk thread (which reads the cursors straight from the
//! [`super::StreamCache`]) and the web Worker (which mirrors the worklet's cache: it is the
//! only writer, so it knows every slot, and gets the cursors in reports).
//!
//! Priority, highest first (the list is cut at the slot count, so it is also what is kept):
//! 1. urgent chunks: a read that missed, and positions primed by the host before a locate
//!    or play (each with the chunk after it), for [`URGENT_MS`];
//! 2. the next [`NEAR_CHUNKS`] chunks of every active cursor (a cursor whose hints keep
//!    changing; interleaved, so voices share the bandwidth);
//! 3. the head of the media ([`HEAD_CHUNKS`]: clip and sampler starts at 0);
//! 4. the host's anchors (clip starts, clip loop starts, the transport loop start: where
//!    playback jumps to without a hint beforehand);
//! 5. the rest of each active cursor's read-ahead, up to `read_ahead` chunks;
//! 6. the last position of idle cursors (resuming where playback stopped).
//!
//! Chunks outside the list are evicted oldest-written first, so recently played audio stays
//! cached while slots allow (a short loop ends up fully resident).

use std::collections::VecDeque;

use super::CHUNK_FRAMES;
use super::cache::{CursorState, MAX_CURSORS};

/// Chunks at the start of every streamed media kept resident.
pub const HEAD_CHUNKS: u64 = 2;
/// Anchors kept per media (more are dropped, furthest from the start first... in order).
pub const MAX_ANCHORS: usize = 8;
/// Near read-ahead per cursor ranked above head and anchors.
pub const NEAR_CHUNKS: u64 = 2;
/// How long an urgent chunk stays top priority.
pub const URGENT_MS: f64 = 2000.0;
/// A cursor whose hints stopped changing this long ago is idle.
pub const ACTIVE_MS: f64 = 500.0;

/// Slots to allocate for a stream with `read_ahead` chunks of read-ahead.
pub fn slot_count(read_ahead: usize) -> usize {
    // Two voices' read-ahead + head + anchors + the chunk behind each voice.
    2 * read_ahead + HEAD_CHUNKS as usize + MAX_ANCHORS + 2
}

/// Writer-side plan for one stream.
pub struct FillPolicy {
    chunks: u64,
    read_ahead: u64,
    /// Mirror of the cache slots: `(chunk, write order)`.
    slots: Vec<(Option<u64>, u64)>,
    writes: u64,
    urgent: VecDeque<(u64, f64)>,
    anchors: Vec<u64>,
    /// Per cursor index: last stamp seen and when it changed.
    seen: [(u64, f64); MAX_CURSORS],
    desired: Vec<u64>,
}

impl FillPolicy {
    /// `chunks` = chunks covering the media; `slots` = cache slots.
    pub fn new(chunks: u64, slots: usize, read_ahead: usize) -> Self {
        Self {
            chunks,
            read_ahead: read_ahead.max(1) as u64,
            slots: vec![(None, 0); slots.max(1)],
            writes: 0,
            urgent: VecDeque::new(),
            anchors: Vec::new(),
            seen: [(0, f64::NEG_INFINITY); MAX_CURSORS],
            desired: Vec::new(),
        }
    }

    /// Replace the anchors (engine frames; deduplicated by chunk, at most
    /// [`MAX_ANCHORS`]).
    pub fn set_anchors(&mut self, frames: impl IntoIterator<Item = u64>) {
        self.anchors.clear();
        for f in frames {
            let c = f / CHUNK_FRAMES as u64;
            if c < self.chunks && !self.anchors.contains(&c) && self.anchors.len() < MAX_ANCHORS {
                self.anchors.push(c);
            }
        }
    }

    pub fn anchors(&self) -> &[u64] {
        &self.anchors
    }

    /// Make the chunk at engine frame `frame` (and the next one) top priority for a while.
    pub fn urge_frame(&mut self, frame: u64, now_ms: f64) {
        self.urge(frame / CHUNK_FRAMES as u64, now_ms);
    }

    /// Make `chunk` (and the next one) top priority for a while.
    pub fn urge(&mut self, chunk: u64, now_ms: f64) {
        if chunk >= self.chunks {
            return;
        }
        self.urgent.retain(|&(c, _)| c != chunk);
        self.urgent.push_back((chunk, now_ms + URGENT_MS));
        while self.urgent.len() > MAX_ANCHORS {
            self.urgent.pop_front();
        }
    }

    /// The writer stored `chunk` in `slot` (or emptied it with `None`).
    pub fn wrote(&mut self, slot: usize, chunk: Option<u64>) {
        if let Some(s) = self.slots.get_mut(slot) {
            self.writes += 1;
            *s = (chunk, self.writes);
        }
    }

    /// Slot holding `chunk` in the mirror.
    pub fn slot_of(&self, chunk: u64) -> Option<usize> {
        self.slots.iter().position(|s| s.0 == Some(chunk))
    }

    /// Forget every slot (the cache was cleared).
    pub fn clear(&mut self) {
        self.slots.fill((None, 0));
    }

    /// Are all of `chunks` resident (per the mirror)?
    pub fn resident(&self, chunks: &[u64]) -> bool {
        chunks
            .iter()
            .all(|&c| c >= self.chunks || self.slot_of(c).is_some())
    }

    /// The next `(slot, chunk)` to decode, if any. `cursors[i]` = cursor `i` of the cache.
    pub fn next(&mut self, cursors: &[Option<CursorState>], now_ms: f64) -> Option<(usize, u64)> {
        self.plan(cursors, now_ms);
        let missing = self
            .desired
            .iter()
            .copied()
            .find(|&c| self.slot_of(c).is_none())?;
        let desired = &self.desired;
        let slot = self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, (c, _))| c.is_none_or(|c| !desired.contains(&c)))
            .min_by_key(|(_, (c, age))| (c.is_some(), *age))
            .map(|(i, _)| i)?;
        Some((slot, missing))
    }

    fn plan(&mut self, cursors: &[Option<CursorState>], now_ms: f64) {
        self.urgent.retain(|&(_, until)| until > now_ms);
        let cap = self.slots.len();
        let chunks = self.chunks;
        let mut active = Vec::with_capacity(MAX_CURSORS);
        let mut idle = Vec::with_capacity(MAX_CURSORS);
        for (i, c) in cursors.iter().enumerate().take(MAX_CURSORS) {
            let Some(c) = c else { continue };
            if self.seen[i].0 != c.stamp {
                self.seen[i] = (c.stamp, now_ms);
            }
            if now_ms - self.seen[i].1 <= ACTIVE_MS {
                active.push(*c);
            } else {
                idle.push(*c);
            }
        }
        let d = &mut self.desired;
        d.clear();
        let push = |d: &mut Vec<u64>, c: u64| {
            if c < chunks && d.len() < cap && !d.contains(&c) {
                d.push(c);
            }
        };
        let step = |c: &CursorState, k: u64| -> Option<u64> {
            let base = c.frame / CHUNK_FRAMES as u64;
            if c.backwards {
                base.checked_sub(k)
            } else {
                Some(base + k)
            }
        };
        for &(c, _) in self.urgent.iter().rev() {
            push(d, c);
            push(d, c + 1);
        }
        for k in 0..NEAR_CHUNKS.min(self.read_ahead) {
            for c in &active {
                if let Some(x) = step(c, k) {
                    push(d, x);
                }
            }
        }
        for c in 0..HEAD_CHUNKS {
            push(d, c);
        }
        for &a in &self.anchors {
            push(d, a);
        }
        for k in NEAR_CHUNKS..self.read_ahead {
            for c in &active {
                if let Some(x) = step(c, k) {
                    push(d, x);
                }
            }
        }
        for c in &idle {
            for k in 0..NEAR_CHUNKS {
                if let Some(x) = step(c, k) {
                    push(d, x);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cursor(frame: u64, stamp: u64) -> Option<CursorState> {
        Some(CursorState {
            frame,
            stamp,
            backwards: false,
        })
    }

    /// Run the policy to completion, "writing" chunks.
    fn fill(p: &mut FillPolicy, cursors: &[Option<CursorState>], now: f64) -> Vec<u64> {
        let mut out = Vec::new();
        while let Some((slot, chunk)) = p.next(cursors, now) {
            p.wrote(slot, Some(chunk));
            out.push(chunk);
            assert!(out.len() < 1000, "no progress");
        }
        out
    }

    #[test]
    fn idle_stream_keeps_head_and_anchors() {
        let mut p = FillPolicy::new(1000, slot_count(12), 12);
        p.set_anchors([CHUNK_FRAMES as u64 * 500 + 7, 3]);
        let got = fill(&mut p, &[None; MAX_CURSORS], 0.0);
        assert_eq!(got, vec![0, 1, 500]);
    }

    #[test]
    fn active_cursor_reads_ahead_then_moves() {
        let c = CHUNK_FRAMES as u64;
        let mut p = FillPolicy::new(1000, slot_count(12), 12);
        let got = fill(&mut p, &[cursor(100 * c, 1), None, None, None], 0.0);
        assert_eq!(&got[..2], &[100, 101]);
        assert_eq!(&got[2..4], &[0, 1]);
        assert_eq!(got.len(), 2 + 2 + 10);
        assert_eq!(*got.last().unwrap(), 111);
        // Playback moved on: the next chunk ahead is loaded, evicting nothing it needs.
        let got = fill(&mut p, &[cursor(101 * c, 2), None, None, None], 10.0);
        assert_eq!(got, vec![112]);
        // Every desired chunk is resident.
        assert!(p.resident(&[0, 1, 101, 112]));
    }

    #[test]
    fn urgent_chunks_come_first_and_expire() {
        let c = CHUNK_FRAMES as u64;
        let mut p = FillPolicy::new(1000, slot_count(12), 12);
        p.urge_frame(700 * c, 0.0);
        let got = fill(&mut p, &[None; MAX_CURSORS], 0.0);
        assert_eq!(got, vec![700, 701, 0, 1]);
        assert!(p.resident(&[700, 701]));
    }

    #[test]
    fn idle_cursors_keep_their_position() {
        let c = CHUNK_FRAMES as u64;
        let mut p = FillPolicy::new(1000, slot_count(4), 4);
        fill(&mut p, &[cursor(50 * c, 1), None, None, None], 0.0);
        // Long after: idle; read-ahead beyond the near chunks is not needed any more.
        let got = fill(&mut p, &[cursor(50 * c, 1), None, None, None], 10_000.0);
        assert!(got.is_empty());
        assert!(p.resident(&[50, 51]));
    }

    #[test]
    fn backwards_cursor_reads_down() {
        let c = CHUNK_FRAMES as u64;
        let mut p = FillPolicy::new(1000, slot_count(4), 4);
        let cur = Some(CursorState {
            frame: 300 * c + 5,
            stamp: 1,
            backwards: true,
        });
        let got = fill(&mut p, &[cur, None, None, None], 0.0);
        assert_eq!(&got[..2], &[300, 299]);
        assert!(got.contains(&297));
    }
}

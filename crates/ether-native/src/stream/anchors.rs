//! Stream clock anchors, host math (native) of docs/COLLAB.md §9.4.
//!
//! With the stream's 48 kHz sample counter `n` (after resampling), a listener's random RTP
//! offset `o` and `L48 = round(L * 48000 / sr)`:
//!
//! ```text
//! rtp(t)        = o + n(t)                   (mod 2^32)
//! anchor(block) = { rtp: rtp(T) + L48, position: P, playing, discontinuity }
//! ```
//!
//! `n(T)` of a tapped block is `round(frames_in * 48000 / sr)` where `frames_in` counts the
//! engine frames tapped before the block (since the capture started, plus the stream samples
//! of earlier captures). The sender stamps its Opus packets so that the **decoded** sample
//! with RTP timestamp `o + n` is stream sample `n` (the encoder look-ahead is compensated in
//! the packet timestamps, `sender.rs`), so the formula maps what is heard.
//!
//! Discontinuities (`discontinuity: true`): `StreamBlock::jump` (play from stopped, locate,
//! loop wrap, the first block after the tap is installed), `gap` (tap blocks skipped), a
//! change of `latency`, a capture (re)start, and **play → stop**. The doc's prose says
//! "stop is not a jump" (the engine does not flag it and the anchor needs no special rtp:
//! `rtp(T) + L48` like every block), while `StreamClock::discontinuity` lists "stop" among
//! the discontinuities; we flag it so a listener never interpolates a playing anchor past
//! the stop (it freezes when the last played sample is heard).

use ether_core::protocol::collab::STREAM_RTP_RATE;
use ether_core::stream_tap::StreamBlock;

/// Periodic anchor cadence in stream samples (`STREAM_CLOCK_INTERVAL_MS` at 48 kHz).
pub const ANCHOR_INTERVAL_SAMPLES: u64 =
    STREAM_RTP_RATE as u64 * ether_core::protocol::collab::STREAM_CLOCK_INTERVAL_MS / 1000;

/// `round(frames * 48000 / sr)` (exact integer rounding, half up).
pub fn to_stream_samples(frames: u64, sr: u32) -> u64 {
    let sr = sr.max(1) as u128;
    ((frames as u128 * STREAM_RTP_RATE as u128 + sr / 2) / sr) as u64
}

/// `o + n + l48 (mod 2^32)`.
pub fn anchor_rtp(offset: u32, n: u64, l48: u64) -> u32 {
    offset.wrapping_add(n.wrapping_add(l48) as u32)
}

/// What the sender knows about one tapped block, in stream samples.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockClock {
    /// `n(T)`: stream index of the block's first frame.
    pub n: u64,
    /// `L48`.
    pub l48: u64,
    pub position: f64,
    pub playing: bool,
    pub recording: bool,
    pub bpm: f64,
    /// Send an anchor now, with `discontinuity: true`.
    pub discontinuity: bool,
}

impl BlockClock {
    /// `rtp(T) + L48` for a listener with RTP offset `offset`.
    pub fn rtp(&self, offset: u32) -> u32 {
        anchor_rtp(offset, self.n, self.l48)
    }
}

/// Per-capture block → [`BlockClock`] (the listener-independent half of the anchors).
#[derive(Debug)]
pub struct ClockTracker {
    sr: u32,
    /// Stream samples before this capture (earlier captures).
    n_base: u64,
    /// Engine frames tapped in this capture.
    frames_in: u64,
    last_latency: Option<u32>,
    was_playing: bool,
    /// The next block is a discontinuity (set for the first block of a capture).
    pending: bool,
}

impl ClockTracker {
    pub fn new(sr: u32, n_base: u64) -> Self {
        Self {
            sr: sr.max(1),
            n_base,
            frames_in: 0,
            last_latency: None,
            was_playing: false,
            pending: true,
        }
    }

    /// The stream index of the next tapped frame.
    pub fn n_next(&self) -> u64 {
        self.n_base + to_stream_samples(self.frames_in, self.sr)
    }

    /// Mark the next block as a discontinuity.
    pub fn mark_discontinuity(&mut self) {
        self.pending = true;
    }

    /// Account one tapped block (call in tap order, before its audio is consumed).
    pub fn on_block(&mut self, b: &StreamBlock) -> BlockClock {
        let n = self.n_next();
        self.frames_in += b.frames as u64;
        let latency_changed = self.last_latency.is_some_and(|l| l != b.latency);
        let stopped = self.was_playing && !b.playing;
        let discontinuity = b.jump || b.gap || latency_changed || stopped || self.pending;
        self.pending = false;
        self.last_latency = Some(b.latency);
        self.was_playing = b.playing;
        BlockClock {
            n,
            l48: to_stream_samples(b.latency as u64, self.sr),
            position: b.position,
            playing: b.playing,
            recording: b.recording,
            bpm: b.bpm,
            discontinuity,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(frames: u32, position: f64, playing: bool, latency: u32, jump: bool) -> StreamBlock {
        StreamBlock {
            sample_time: 0,
            frames,
            position,
            playing,
            recording: false,
            bpm: 120.0,
            latency,
            jump,
            gap: false,
        }
    }

    #[test]
    fn stream_sample_rounding() {
        assert_eq!(to_stream_samples(480, 48_000), 480);
        assert_eq!(to_stream_samples(441, 44_100), 480);
        assert_eq!(to_stream_samples(1, 44_100), 1); // 1.088 → 1
        assert_eq!(to_stream_samples(7, 44_100), 8); // 7.619 → 8
        assert_eq!(to_stream_samples(1, 96_000), 1); // 0.5 → 1 (half up)
        assert_eq!(
            to_stream_samples(44_100 * 3600 * 30, 44_100),
            48_000 * 3600 * 30
        );
    }

    #[test]
    fn worked_example_of_the_doc() {
        // 48 kHz, L = 480, o such that rtp(T) = 1_000_000 at the wrap.
        let mut t = ClockTracker::new(48_000, 0);
        let o = 1_000_000u32.wrapping_sub(9_600);
        let first = t.on_block(&block(4_800, 7.6, true, 480, true));
        assert!(first.discontinuity);
        let a = t.on_block(&block(4_800, 7.8, true, 480, false));
        assert!(!a.discontinuity);
        assert_eq!(a.rtp(o), 995_680);
        let wrap = t.on_block(&block(4_800, 0.0, true, 480, true));
        assert!(wrap.discontinuity);
        assert_eq!(wrap.rtp(o), 1_000_480);
    }

    #[test]
    fn rtp_wraps_mod_2_32() {
        assert_eq!(anchor_rtp(u32::MAX - 10, 5, 10), 4);
        assert_eq!(anchor_rtp(u32::MAX, 1 << 32, 1), 0);
        let mut t = ClockTracker::new(44_100, (1 << 32) - 100);
        let b = t.on_block(&block(441, 1.0, true, 441, true));
        assert_eq!(b.n, (1 << 32) - 100);
        assert_eq!(b.l48, 480);
        assert_eq!(b.rtp(50), 430);
        let b = t.on_block(&block(441, 1.0, true, 441, false));
        assert_eq!(b.n, (1 << 32) + 380);
        assert_eq!(b.rtp(0), 380 + 480);
    }

    #[test]
    fn discontinuities() {
        let mut t = ClockTracker::new(48_000, 0);
        assert!(t.on_block(&block(128, 0.0, false, 0, false)).discontinuity); // first
        assert!(!t.on_block(&block(128, 0.0, false, 0, false)).discontinuity);
        assert!(t.on_block(&block(128, 0.0, true, 0, true)).discontinuity); // play
        assert!(!t.on_block(&block(128, 0.1, true, 0, false)).discontinuity);
        assert!(t.on_block(&block(128, 0.2, true, 64, false)).discontinuity); // latency
        let mut g = block(128, 0.3, true, 64, false);
        g.gap = true;
        assert!(t.on_block(&g).discontinuity); // gap
        t.mark_discontinuity();
        assert!(t.on_block(&block(128, 0.4, true, 64, false)).discontinuity); // dropped
        assert!(t.on_block(&block(128, 0.5, false, 64, false)).discontinuity); // stop
        assert!(!t.on_block(&block(128, 0.5, false, 64, false)).discontinuity);
        assert_eq!(t.n_next(), 9 * 128);
    }
}

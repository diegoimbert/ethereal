//! [`StreamCache`]: the bounded, lock-free chunk cache behind a streamed media's
//! `AudioSource`.
//!
//! One writer (the host's reader: the native disk thread, or the worklet applying chunks
//! shipped by the web Worker) fills fixed slots of [`CHUNK_FRAMES`] frames; any number of
//! audio threads read them. Each slot is a seqlock: the writer bumps the slot's sequence to
//! odd, stores the chunk index and the samples, then bumps it to even; a reader copies the
//! samples and keeps them only if the sequence was the same even value before and after.
//! Samples are stored as `AtomicU32` bit patterns (relaxed loads and stores compile to plain
//! moves), so a torn read is detected, never undefined behaviour.
//!
//! The audio side never blocks, allocates, frees or does I/O: a chunk that isn't resident
//! (or is being rewritten) is an **underrun**: silence, `read` returns `false`, the
//! per-media counter is bumped and the chunk is flagged as urgent for the writer.
//!
//! Where playback is heading comes from `AudioSource::prefetch_hint` (the engine calls it
//! after every clip, warp, sampler and preview read): hints update up to [`MAX_CURSORS`]
//! play cursors (one per voice reading the media; a hint near an existing cursor moves it,
//! a far one takes the least recently used cursor). The writer reads them through
//! [`StreamCache::cursors`] and decides what to decode (`super::StreamFiller`).

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};

use ether_core::AudioSource;

use super::CHUNK_FRAMES;

/// Play cursors tracked per streamed media (voices reading it at different positions).
pub const MAX_CURSORS: usize = 4;
/// A hint within this many frames of a cursor moves that cursor.
const CURSOR_SNAP: u64 = 2 * CHUNK_FRAMES as u64;
/// `Slot::chunk` of an empty slot.
const EMPTY: u64 = u64::MAX;

struct Slot {
    /// Seqlock sequence: odd while the writer rewrites the slot.
    seq: AtomicU64,
    /// Chunk index held (valid when `seq` is even), [`EMPTY`] if none.
    chunk: AtomicU64,
}

/// One play cursor: `pos` = last hinted frame + 1 (0 = unused); `stamp` = value of the
/// cache's hint counter at the last update (staleness, LRU); `back` = moving backwards.
struct Cursor {
    pos: AtomicU64,
    stamp: AtomicU64,
    back: AtomicU32,
}

/// A play cursor as seen by the writer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorState {
    /// Frame the voice will read next (engine rate).
    pub frame: u64,
    /// Hint counter at its last update (changes while the voice plays).
    pub stamp: u64,
    /// The voice plays backwards (reversed clips).
    pub backwards: bool,
}

/// Bounded lock-free chunk cache of one streamed media (also its `AudioSource`).
pub struct StreamCache {
    channels: u16,
    frames: u64,
    slots: Box<[Slot]>,
    /// `[slot][channel][CHUNK_FRAMES]` samples as `f32` bits.
    data: Box<[AtomicU32]>,
    cursors: [Cursor; MAX_CURSORS],
    hints: AtomicU64,
    underruns: AtomicU64,
    /// Last chunk an audio read missed, + 1 (0 = none); taken by the writer.
    missed: AtomicU64,
}

impl StreamCache {
    /// Allocate a cache of `slots` chunks (non-RT). `frames` = length at the engine rate.
    pub fn new(channels: u16, frames: u64, slots: usize) -> Self {
        let channels = channels.max(1);
        let slots = slots.max(1);
        let n = slots * channels as usize * CHUNK_FRAMES;
        Self {
            channels,
            frames,
            slots: (0..slots)
                .map(|_| Slot {
                    seq: AtomicU64::new(0),
                    chunk: AtomicU64::new(EMPTY),
                })
                .collect(),
            data: (0..n).map(|_| AtomicU32::new(0)).collect(),
            cursors: std::array::from_fn(|_| Cursor {
                pos: AtomicU64::new(0),
                stamp: AtomicU64::new(0),
                back: AtomicU32::new(0),
            }),
            hints: AtomicU64::new(0),
            underruns: AtomicU64::new(0),
            missed: AtomicU64::new(0),
        }
    }

    /// Number of chunk slots.
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// Chunks covering the media.
    pub fn chunk_count(&self) -> u64 {
        self.frames.div_ceil(CHUNK_FRAMES as u64)
    }

    /// Bytes of sample memory held (the bound: independent of the media length).
    pub fn memory_bytes(&self) -> usize {
        self.data.len() * 4
    }

    /// Underruns counted so far (reads that found their chunk missing).
    pub fn underruns(&self) -> u64 {
        self.underruns.load(Ordering::Relaxed)
    }

    /// Chunk held by `slot` (`None` if empty or being written). Writer side.
    pub fn slot_chunk(&self, slot: usize) -> Option<u64> {
        let s = &self.slots[slot];
        let seq = s.seq.load(Ordering::Acquire);
        let c = s.chunk.load(Ordering::Relaxed);
        (seq.is_multiple_of(2) && c != EMPTY).then_some(c)
    }

    /// The slot holding `chunk`, if resident.
    pub fn find(&self, chunk: u64) -> Option<usize> {
        self.slots.iter().position(|s| {
            s.chunk.load(Ordering::Relaxed) == chunk && s.seq.load(Ordering::Acquire) % 2 == 0
        })
    }

    /// The chunk an audio read missed most recently (cleared). Writer side.
    pub fn take_missed(&self) -> Option<u64> {
        match self.missed.swap(0, Ordering::Relaxed) {
            0 => None,
            c => Some(c - 1),
        }
    }

    /// The play cursors, by index (writer side).
    pub fn cursors(&self) -> [Option<CursorState>; MAX_CURSORS] {
        std::array::from_fn(|i| {
            let c = &self.cursors[i];
            let pos = c.pos.load(Ordering::Relaxed);
            (pos != 0).then(|| CursorState {
                frame: pos - 1,
                stamp: c.stamp.load(Ordering::Relaxed),
                backwards: c.back.load(Ordering::Relaxed) != 0,
            })
        })
    }

    /// Write `chunk` into `slot` (writer side; one writer at a time): `samples[c]` holds
    /// channel `c` (`CHUNK_FRAMES` frames; missing ones are zeros, a missing channel repeats
    /// the first).
    pub fn write_slot(&self, slot: usize, chunk: u64, samples: &[Vec<f32>]) {
        let s = &self.slots[slot];
        let seq = s.seq.load(Ordering::Relaxed);
        let seq = if seq % 2 == 1 { seq + 1 } else { seq };
        s.seq.store(seq + 1, Ordering::Relaxed);
        fence(Ordering::Release);
        s.chunk.store(chunk, Ordering::Relaxed);
        let ch = self.channels as usize;
        let base = slot * ch * CHUNK_FRAMES;
        for c in 0..ch {
            let src = samples.get(c).or_else(|| samples.first());
            let dst = &self.data[base + c * CHUNK_FRAMES..base + (c + 1) * CHUNK_FRAMES];
            match src {
                Some(src) => {
                    for (i, d) in dst.iter().enumerate() {
                        d.store(
                            src.get(i).copied().unwrap_or(0.0).to_bits(),
                            Ordering::Relaxed,
                        );
                    }
                }
                None => dst.iter().for_each(|d| d.store(0, Ordering::Relaxed)),
            }
        }
        s.seq.store(seq + 2, Ordering::Release);
    }

    /// Write raw little-endian `f32` bytes of one channel into a slot that the writer
    /// already claimed with [`Self::begin_slot`] (the web worklet applies chunks channel by
    /// channel straight from ring frames, without a temporary buffer).
    pub fn write_channel_bytes(&self, slot: usize, channel: u16, offset: usize, bytes: &[u8]) {
        let ch = self.channels as usize;
        if slot >= self.slots.len() || channel as usize >= ch {
            return;
        }
        let base = slot * ch * CHUNK_FRAMES + channel as usize * CHUNK_FRAMES;
        let n = (bytes.len() / 4).min(CHUNK_FRAMES.saturating_sub(offset));
        let dst = &self.data[base + offset..base + offset + n];
        for (d, b) in dst.iter().zip(bytes.as_chunks::<4>().0) {
            d.store(u32::from_le_bytes(*b), Ordering::Relaxed);
        }
    }

    /// Start rewriting `slot` with `chunk` (readers treat it as missing until
    /// [`Self::end_slot`]).
    pub fn begin_slot(&self, slot: usize, chunk: u64) {
        let Some(s) = self.slots.get(slot) else {
            return;
        };
        let seq = s.seq.load(Ordering::Relaxed);
        let seq = if seq % 2 == 1 { seq + 1 } else { seq };
        s.seq.store(seq + 1, Ordering::Relaxed);
        fence(Ordering::Release);
        s.chunk.store(chunk, Ordering::Relaxed);
    }

    /// Publish a slot started with [`Self::begin_slot`].
    pub fn end_slot(&self, slot: usize) {
        let Some(s) = self.slots.get(slot) else {
            return;
        };
        let seq = s.seq.load(Ordering::Relaxed);
        if seq % 2 == 1 {
            s.seq.store(seq + 1, Ordering::Release);
        }
    }

    /// Drop every chunk (e.g. the file changed). Writer side.
    pub fn clear(&self) {
        for s in self.slots.iter() {
            let seq = s.seq.load(Ordering::Relaxed);
            let seq = if seq % 2 == 1 { seq + 1 } else { seq };
            s.seq.store(seq + 1, Ordering::Relaxed);
            fence(Ordering::Release);
            s.chunk.store(EMPTY, Ordering::Relaxed);
            s.seq.store(seq + 2, Ordering::Release);
        }
    }

    /// **RT.** Copy frames `[offset, offset + out.len())` of `channel` from `chunk`'s slot.
    fn read_chunk(&self, chunk: u64, channel: usize, offset: usize, out: &mut [f32]) -> bool {
        let ch = self.channels as usize;
        for (i, s) in self.slots.iter().enumerate() {
            if s.chunk.load(Ordering::Relaxed) != chunk {
                continue;
            }
            let seq = s.seq.load(Ordering::Acquire);
            if seq % 2 == 1 || s.chunk.load(Ordering::Relaxed) != chunk {
                continue;
            }
            let base = i * ch * CHUNK_FRAMES + channel * CHUNK_FRAMES + offset;
            let len = out.len();
            for (o, d) in out.iter_mut().zip(&self.data[base..base + len]) {
                *o = f32::from_bits(d.load(Ordering::Relaxed));
            }
            fence(Ordering::Acquire);
            if s.seq.load(Ordering::Relaxed) == seq {
                return true;
            }
        }
        out.fill(0.0);
        false
    }

    /// **RT.** Move the nearest cursor to `frame` (or take the least recently used one).
    fn hint(&self, frame: u64) {
        let stamp = self.hints.fetch_add(1, Ordering::Relaxed) + 1;
        let mut best: Option<(usize, u64)> = None;
        let mut lru = (0, u64::MAX);
        for (i, c) in self.cursors.iter().enumerate() {
            let pos = c.pos.load(Ordering::Relaxed);
            let s = if pos == 0 {
                0
            } else {
                c.stamp.load(Ordering::Relaxed)
            };
            if s < lru.1 {
                lru = (i, s);
            }
            if pos == 0 {
                continue;
            }
            let d = (pos - 1).abs_diff(frame);
            if d <= CURSOR_SNAP && best.is_none_or(|(_, bd)| d < bd) {
                best = Some((i, d));
            }
        }
        let i = best.map_or(lru.0, |(i, _)| i);
        let c = &self.cursors[i];
        let old = c.pos.swap(frame + 1, Ordering::Relaxed);
        if best.is_some() && old != 0 && old - 1 != frame {
            c.back.store(u32::from(frame < old - 1), Ordering::Relaxed);
        } else if best.is_none() {
            c.back.store(0, Ordering::Relaxed);
        }
        c.stamp.store(stamp, Ordering::Relaxed);
    }
}

impl AudioSource for StreamCache {
    fn channels(&self) -> u16 {
        self.channels
    }

    fn frames(&self) -> u64 {
        self.frames
    }

    /// **RT**: no allocation, lock or I/O (see the module docs).
    fn read(&self, channel: u16, start: u64, out: &mut [f32]) -> bool {
        if channel >= self.channels {
            out.fill(0.0);
            return true;
        }
        let c = CHUNK_FRAMES as u64;
        let mut ok = true;
        let mut at = 0usize;
        let mut first_miss = None;
        while at < out.len() {
            let frame = start + at as u64;
            if frame >= self.frames {
                out[at..].fill(0.0);
                break;
            }
            let chunk = frame / c;
            let offset = (frame % c) as usize;
            let n = (CHUNK_FRAMES - offset)
                .min(out.len() - at)
                .min((self.frames - frame) as usize);
            if !self.read_chunk(chunk, channel as usize, offset, &mut out[at..at + n]) {
                ok = false;
                first_miss.get_or_insert(chunk);
            }
            at += n;
        }
        if let Some(chunk) = first_miss {
            self.underruns.fetch_add(1, Ordering::Relaxed);
            self.missed.store(chunk + 1, Ordering::Relaxed);
        }
        // Reads are hints too (readers that never call `prefetch_hint`, e.g. frozen
        // tracks, still move a cursor; one channel is enough).
        if channel == 0 || first_miss.is_some() {
            self.hint(start + out.len() as u64);
        }
        ok
    }

    fn prefetch_hint(&self, start: u64) {
        self.hint(start);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk_of(value: f32, channels: usize) -> Vec<Vec<f32>> {
        (0..channels)
            .map(|c| vec![value + c as f32; CHUNK_FRAMES])
            .collect()
    }

    #[test]
    fn reads_resident_chunks_and_counts_underruns() {
        let frames = CHUNK_FRAMES as u64 * 3 + 100;
        let cache = StreamCache::new(2, frames, 2);
        assert_eq!(cache.chunk_count(), 4);
        cache.write_slot(0, 0, &chunk_of(1.0, 2));
        cache.write_slot(1, 1, &chunk_of(10.0, 2));
        let mut out = vec![9.0; 8];
        // Straddles chunks 0 and 1.
        assert!(cache.read(1, CHUNK_FRAMES as u64 - 4, &mut out));
        assert_eq!(out, [2.0, 2.0, 2.0, 2.0, 11.0, 11.0, 11.0, 11.0]);
        assert_eq!(cache.underruns(), 0);
        // Chunk 2 is not resident: silence, false, counted, flagged.
        assert!(!cache.read(0, 2 * CHUNK_FRAMES as u64 + 5, &mut out));
        assert_eq!(out, [0.0; 8]);
        assert_eq!(cache.underruns(), 1);
        assert_eq!(cache.take_missed(), Some(2));
        assert_eq!(cache.take_missed(), None);
        // Past the end: zeros, not an underrun.
        assert!(cache.read(0, frames + 10, &mut out));
        assert_eq!(cache.underruns(), 1);
        // Unknown channel: zeros.
        assert!(cache.read(5, 0, &mut out));
    }

    #[test]
    fn rewriting_a_slot_replaces_its_chunk() {
        let cache = StreamCache::new(1, CHUNK_FRAMES as u64 * 8, 1);
        cache.write_slot(0, 3, &chunk_of(3.0, 1));
        assert_eq!(cache.find(3), Some(0));
        cache.begin_slot(0, 5);
        // Mid-write: not readable.
        assert_eq!(cache.slot_chunk(0), None);
        let mut out = [1.0; 4];
        assert!(!cache.read(0, 5 * CHUNK_FRAMES as u64, &mut out));
        let bytes: Vec<u8> = (0..CHUNK_FRAMES)
            .flat_map(|_| 5.0f32.to_le_bytes())
            .collect();
        cache.write_channel_bytes(0, 0, 0, &bytes);
        cache.end_slot(0);
        assert_eq!(cache.slot_chunk(0), Some(5));
        assert!(cache.read(0, 5 * CHUNK_FRAMES as u64, &mut out));
        assert_eq!(out, [5.0; 4]);
        assert_eq!(cache.find(3), None);
        cache.clear();
        assert_eq!(cache.slot_chunk(0), None);
    }

    #[test]
    fn hints_track_several_cursors() {
        let cache = StreamCache::new(1, 1 << 30, 1);
        cache.prefetch_hint(1000);
        cache.prefetch_hint(1128);
        cache.prefetch_hint(10_000_000);
        let mut cs: Vec<_> = cache.cursors().into_iter().flatten().collect();
        cs.sort_by_key(|c| c.frame);
        assert_eq!(cs.len(), 2);
        assert_eq!(cs[0].frame, 1128);
        assert!(!cs[0].backwards);
        assert_eq!(cs[1].frame, 10_000_000);
        // Reversed voice: hints go down.
        cache.prefetch_hint(10_000_000 - 256);
        let back = cache
            .cursors()
            .into_iter()
            .flatten()
            .find(|c| c.frame == 10_000_000 - 256)
            .unwrap();
        assert!(back.backwards);
        // More voices than cursors: the least recently used one is reused.
        for k in 1..=MAX_CURSORS as u64 {
            cache.prefetch_hint(k * 100_000_000);
        }
        assert_eq!(cache.cursors().into_iter().flatten().count(), MAX_CURSORS);
        assert!(
            cache
                .cursors()
                .into_iter()
                .flatten()
                .all(|c| c.frame % 100_000_000 == 0)
        );
    }

    #[test]
    fn memory_is_bounded_by_slots() {
        let ten_minutes = 48_000 * 600;
        let cache = StreamCache::new(2, ten_minutes, 30);
        assert_eq!(cache.memory_bytes(), 30 * 2 * CHUNK_FRAMES * 4);
        assert!(cache.memory_bytes() < 5 << 20);
    }
}

//! Streaming decode (v0.3, owned by the `audio-streaming` node; CONTRACTS.md §13.1).
//! Wasm-safe building blocks the hosts' reader threads use; no threads or I/O here.
//!
//! - [`ChunkDecoder`]: decode + resample a media file **chunk by chunk** at the engine
//!   rate, with seeking (symphonia seek to the nearest packet, then decode forward), from
//!   any `Read + Seek` byte source (a file natively, an OPFS file read by ranges on the web).
//! - [`StreamCache`]: the fixed-size, lock-free chunk cache shared between the reader
//!   (fills) and the audio thread (`AudioSource::read` copies; a missing chunk is an
//!   underrun: silence + `false`). Allocated once per streamed media; the audio thread
//!   never allocates, blocks or frees.
//! - [`FillPolicy`]: which chunk to decode next and which slot to reuse (play cursors from
//!   the engine's `prefetch_hint`s, urgent misses, primed locate targets, anchors).
//! - [`anchors`]: clip/loop starts and locate targets from the published graph.
//! - [`PeakBuilder`]: waveform peaks from the one decode pass, samples dropped.
//! - [`StreamFiller`]: decoder + cache + policy, the native reader thread's unit of work.
//!
//! Chunk size: [`CHUNK_FRAMES`] frames at the engine rate (all channels).

pub mod anchors;
mod cache;
mod decoder;
mod peaks;
mod policy;

use std::sync::Arc;

pub use cache::{CursorState, MAX_CURSORS, StreamCache};
pub use decoder::{BytesSource, ChunkDecoder, RESAMPLE_MARGIN, SEEK_PREROLL};
pub use peaks::PeakBuilder;
pub use policy::{
    ACTIVE_MS, FillPolicy, HEAD_CHUNKS, MAX_ANCHORS, NEAR_CHUNKS, URGENT_MS, slot_count,
};
pub use symphonia::core::io::MediaSource;

use crate::MediaError;

/// Frames per cache chunk at the engine rate.
pub const CHUNK_FRAMES: usize = 16_384;

/// One streamed media on the writer side: its decoder, its cache (also the engine's
/// `AudioSource`) and the fill policy.
pub struct StreamFiller {
    decoder: ChunkDecoder,
    cache: Arc<StreamCache>,
    policy: FillPolicy,
    buf: Vec<Vec<f32>>,
    /// Chunks that failed to decode (filled with silence instead).
    pub errors: u64,
    pub last_error: Option<MediaError>,
}

impl StreamFiller {
    /// A cache of [`slot_count`]`(read_ahead)` slots for `decoder`'s media.
    pub fn new(decoder: ChunkDecoder, read_ahead: usize) -> Self {
        let slots = slot_count(read_ahead);
        let cache = Arc::new(StreamCache::new(decoder.channels(), decoder.frames(), slots));
        let policy = FillPolicy::new(decoder.chunk_count(), slots, read_ahead);
        Self {
            decoder,
            cache,
            policy,
            buf: Vec::new(),
            errors: 0,
            last_error: None,
        }
    }

    pub fn cache(&self) -> &Arc<StreamCache> {
        &self.cache
    }

    pub fn policy_mut(&mut self) -> &mut FillPolicy {
        &mut self.policy
    }

    /// Prime the chunks at these engine frames (locate/play targets).
    pub fn urge_frames(&mut self, frames: &[u64], now_ms: f64) {
        for &f in frames {
            self.policy.urge_frame(f, now_ms);
        }
    }

    /// Are the chunks at these engine frames (and the next ones) resident?
    pub fn primed(&self, frames: &[u64]) -> bool {
        let c = CHUNK_FRAMES as u64;
        frames.iter().all(|&f| self.policy.resident(&[f / c, f / c + 1]))
    }

    /// Decode at most one chunk. Returns `true` if there was work.
    pub fn fill_one(&mut self, now_ms: f64) -> bool {
        if let Some(missed) = self.cache.take_missed() {
            self.policy.urge(missed, now_ms);
        }
        let cursors = self.cache.cursors();
        let Some((slot, chunk)) = self.policy.next(&cursors, now_ms) else {
            return false;
        };
        if let Err(e) = self.decoder.decode_chunk(chunk, &mut self.buf) {
            // Keep going: the chunk plays as silence rather than stalling the stream.
            self.errors += 1;
            self.last_error = Some(e);
            for b in &mut self.buf {
                b.clear();
                b.resize(CHUNK_FRAMES, 0.0);
            }
        }
        self.cache.write_slot(slot, chunk, &self.buf);
        self.policy.wrote(slot, Some(chunk));
        true
    }

    /// Fill until nothing is wanted (tests, priming).
    pub fn fill_all(&mut self, now_ms: f64) -> usize {
        let mut n = 0;
        while self.fill_one(now_ms) {
            n += 1;
        }
        n
    }
}

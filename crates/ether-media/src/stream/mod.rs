//! Streaming decode (v0.3, owned by the `audio-streaming` node; CONTRACTS.md §13.1).
//! Wasm-safe building blocks the hosts' reader threads use; no threads or I/O here.
//!
//! - [`ChunkDecoder`]: decode + resample a media file **chunk by chunk** at the engine
//!   rate, with seeking (symphonia seek to the nearest packet, then decode forward), from
//!   any `Read + Seek` byte source (a file natively, an OPFS sync access handle on the web).
//! - [`StreamCache`]: the fixed-size, lock-free chunk cache shared between the reader
//!   thread (fills) and the audio thread (`AudioSource::read` copies; a missing chunk is an
//!   underrun: silence + `false`). Allocated once per streamed media; the audio thread
//!   never allocates, blocks or frees.
//!
//! Chunk size: [`CHUNK_FRAMES`] frames at the engine rate (all channels).

/// Frames per cache chunk at the engine rate.
pub const CHUNK_FRAMES: usize = 16_384;

/// Chunked decoder of one media (placeholder until `audio-streaming` lands).
pub struct ChunkDecoder {
    _private: (),
}

/// Lock-free chunk cache of one streamed media (placeholder until `audio-streaming` lands).
pub struct StreamCache {
    _private: (),
}

//! Disk streaming of long media (v0.3, owned by the `audio-streaming` node; CONTRACTS.md
//! §13.1). (`crate::stream` is the collab "listen on peer" sender, unrelated.)
//!
//! `NativeBridge::stream_media` (shared touch in `bridge.rs`) opens the media file (the
//! project copy under the store root, or the external reference), creates an
//! `ether_media::stream::{ChunkDecoder, StreamCache}` pair, registers a streaming
//! `AudioSource` over the cache with `EngineHandle::add_source`, and hands the decoder to
//! one shared reader thread here. The thread keeps every streamed media's cache filled
//! around the positions the engine hints (`AudioSource::prefetch_hint`) and the controller
//! announces (loop, locate), reading [`READ_AHEAD_CHUNKS`] chunks ahead; it never touches
//! the audio thread except through the lock-free cache. Underruns are counted per media and
//! reported (`MediaEvent`, via the controller tick).

/// Chunks kept decoded ahead of the play position per streamed media.
pub const READ_AHEAD_CHUNKS: usize = 12;

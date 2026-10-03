//! Web streaming of long media (v0.3, owned by the `audio-streaming` node; CONTRACTS.md
//! §13.1).
//!
//! The engine Worker reads the media file through an OPFS sync access handle
//! (`ether_media::stream::ChunkDecoder`), and ships decoded chunks to the worklet over the
//! SAB ring; the worklet keeps them in an `ether_media::stream::StreamCache` behind a
//! streaming `AudioSource` (no allocation on the audio side: chunk slots are pre-allocated
//! when the media is registered). The worklet reports the positions it reads (prefetch
//! hints) back to the Worker, which keeps [`READ_AHEAD_CHUNKS`] chunks ahead.
//! `WasmBridge::stream_media` (shared touch in `bridge.rs`/`proto.rs`/`worklet.rs`) wires it.

/// Chunks kept ahead of the play position per streamed media (web: smaller than native,
/// the SAB ring carries them).
pub const READ_AHEAD_CHUNKS: usize = 8;

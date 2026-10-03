//! Streaming long audio from disk (v0.3, owned by the `audio-streaming` node; CONTRACTS.md
//! §13.1).
//!
//! Today every media is decoded whole and resampled by the controller (`media/`), then
//! handed to the engine (`EngineBridge::load_media`): about 115 MB for 5 minutes of stereo.
//! With streaming, media longer than [`STREAM_MIN_SECONDS`] on a host that can stream
//! (`EngineBridge::stream_media` returns `Ok(true)`) are **not** decoded into memory:
//! - the controller still runs one decode pass for the waveform peaks (kept, the samples
//!   dropped) and the content hash, then calls `stream_media` with a [`StreamSource`];
//! - the host registers an `AudioSource` whose `read` serves from a bounded read-ahead
//!   cache (fixed-size chunks at the engine rate, decoded and resampled on a host reader
//!   thread: native = a disk thread, web = the engine Worker over OPFS sync access handles,
//!   chunks shipped to the worklet over the SAB ring). The audio thread never blocks,
//!   allocates or does I/O: a miss is an underrun (silence, `read` returns `false`, counted
//!   and reported);
//! - `AudioSource::prefetch_hint` (called by the engine for upcoming clip reads) and the
//!   controller's loop/locate knowledge drive the read-ahead, so loops and jumps don't
//!   underrun; warp/stretch keep working because the stretcher reads through the same
//!   `AudioSource` (it reads ahead by its own latency, inside the cache window);
//! - offline renders (export, freeze, bounce, audio-to-MIDI) read the file synchronously
//!   (they may block).
//!
//! [`should_stream`] is the policy hook the `media/` pipeline calls (shared touch of this
//! node); until it lands it always says no (v0.2 behaviour).

use ether_core::protocol::model::{MediaRef, ProjectId};

/// Media at least this long (at their own rate) stream instead of being decoded whole.
pub const STREAM_MIN_SECONDS: f64 = 30.0;
/// Read-ahead window per streamed media voice, in seconds at the engine rate.
pub const STREAM_READ_AHEAD_SECONDS: f64 = 4.0;

/// What the host needs to stream one media (plain data, so the web bridge can ship it to
/// the Worker).
#[derive(Clone, Debug, PartialEq)]
pub struct StreamSource {
    pub project: ProjectId,
    /// The media (id, project-relative `file`, `location` for references, rate, frames).
    pub media: MediaRef,
    /// Resolved external path when the media is referenced in place and its hash matched.
    pub external_path: Option<String>,
    /// Rate the engine runs at (the source resamples to it).
    pub engine_sample_rate: u32,
}

/// Policy: stream `media`? Placeholder: never.
pub fn should_stream(media: &MediaRef, engine_sample_rate: u32) -> bool {
    let _ = (media, engine_sample_rate);
    false
}

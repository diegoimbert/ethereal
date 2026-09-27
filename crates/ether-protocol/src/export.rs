//! Offline export (roadmap v2, `export` node): render the arrangement to audio files.
//!
//! Flow: `Export::Render` replies `ExportStarted { job }` immediately; the render runs in the
//! background (controller tick) and reports `Event::Export` progress, then exactly one of
//! `Done`, `Failed` or `Cancelled`. At most one job runs at a time (`InvalidState`
//! otherwise). The render uses an independent offline engine (`ether_core::offline`), so
//! playback keeps working and the metronome is never rendered.
//!
//! Where the files go (**never a UI path**, CONTRACTS.md §2b):
//! - Native hosts write into the project folder: `<project>/exports/<file>`
//!   (`ether_model::file::EXPORTS_DIR`); `ExportResult::Files` lists project-relative paths.
//! - The web host (and remote engines, where the UI may be on another machine) keep the
//!   result engine-side and return `ExportResult::Download` tokens; the UI pulls the bytes
//!   with `Export::ReadChunk` (base64 over any transport; the WebSocket transport may carry
//!   them as binary frames, see [`crate::remote`]) and saves them as a browser download.
//!   Downloads are dropped by `Export::Release`, a new render, or closing the project.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{Base64Bytes, Beats, TrackId};

/// Identifies an export job (client-chosen ULID string, like upload ids).
pub type ExportJobId = String;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ExportCommand {
    /// Start rendering. Replies `ExportStarted`.
    Render {
        job: ExportJobId,
        request: ExportRequest,
    },
    /// Cancel a running job (`Event::Export { Cancelled }` follows). No-op if finished.
    Cancel { job: ExportJobId },
    /// Read bytes of a finished download. Replies `Bytes` (`eof` when `offset + data.len()`
    /// reaches the file size). `length` is capped by the host (at least 256 KiB served).
    ReadChunk {
        token: String,
        offset: f64,
        length: u32,
    },
    /// Drop a finished download's engine-side bytes.
    Release { token: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ExportRequest {
    pub range: ExportRange,
    pub format: ExportFormat,
    pub mode: ExportMode,
    /// Peak-normalize each file to 0 dBFS (-0.1 dB headroom) after rendering.
    pub normalize: bool,
    /// Extra render time after the range end (reverb/delay tails), `>= 0`, capped at 60.
    pub tail_seconds: f64,
    /// Base file name without extension (sanitized by the host; stems append
    /// ` - <track name>`). `None` = the project name.
    pub name: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ExportRange {
    /// The loop region (whether or not looping is enabled).
    Loop,
    /// From beat 0 to the end of the last clip (and last automation point).
    Project,
    Custom {
        start: Beats,
        end: Beats,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ExportFormat {
    pub container: AudioContainer,
    pub bit_depth: BitDepth,
    /// Output rate in Hz. `None` = the engine rate. Other rates are rendered at the engine
    /// rate and resampled.
    pub sample_rate: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum AudioContainer {
    Wav,
    /// FLAC supports `Int16`/`Int24` only (`Float32` is `InvalidArgument`).
    Flac,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum BitDepth {
    /// 16-bit PCM with TPDF dither.
    Int16,
    Int24,
    Float32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ExportMode {
    /// The master output (one stereo file).
    Mix,
    /// One file per listed track: that track's post-fader output, as if it were the only
    /// audible source feeding master (its sends/returns included, master chain excluded).
    /// Rendered as one offline pass per track.
    Stems { tracks: Vec<TrackId> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ExportEvent {
    /// `progress` 0..=1 over the whole job (all stems).
    Progress {
        job: ExportJobId,
        progress: f32,
    },
    Done {
        job: ExportJobId,
        result: ExportResult,
    },
    Failed {
        job: ExportJobId,
        message: String,
    },
    Cancelled {
        job: ExportJobId,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ExportResult {
    /// Native: files written under the project folder (project-relative paths such as
    /// `exports/My Song.wav`), in request order.
    Files { files: Vec<String> },
    /// Web/remote: engine-side results to pull with `Export::ReadChunk`.
    Download { downloads: Vec<ExportDownload> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ExportDownload {
    pub token: String,
    /// Suggested file name (with extension).
    pub name: String,
    /// MIME type (`audio/wav`, `audio/flac`).
    pub mime: String,
    pub size: f64,
}

/// Reply payload of `Export::ReadChunk` (also usable by other byte transfers).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ByteChunk {
    pub offset: f64,
    pub data: Base64Bytes,
    pub eof: bool,
}

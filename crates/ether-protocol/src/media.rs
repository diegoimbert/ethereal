//! Media import, peaks (waveform overviews) and the sample browser.
//!
//! All file access is engine-side. The browser lists *engine-visible* locations only: the
//! configured library folder(s) and the current project's own media. Paths inside a
//! location are relative (`"Drums/Kicks/kick1.wav"`) and resolved/sandboxed by the engine;
//! the UI never sees absolute paths. Importing copies the file into the project's
//! `media/` folder, so projects are self-contained.
//!
//! Uploading files from the UI machine (roadmap v2, `remote-engine`): `BeginUpload` →
//! `UploadChunk`s (in order, any size up to 1 MiB each) → `Import { source: Upload }`, which
//! fails with `InvalidState` unless exactly `size` bytes were received. `CancelUpload` (or a
//! disconnect) drops the partial upload. Hosts without upload support reply `Unsupported`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::MediaId;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum MediaCommand {
    /// Copy a file into the current project's `media/`, decode it and register it.
    /// Replies `Media` with the `MediaRef` (also delivered as a patch); peaks are computed
    /// in the background (`MediaEvent::PeaksReady`).
    Import {
        id: MediaId,
        source: MediaSource,
    },
    /// Replies `Peaks`. Hosts pick the nearest mipmap level `>= samples_per_peak`.
    GetPeaks {
        request: PeakRequest,
    },
    /// Engine-visible browse roots. Replies `Locations`.
    ListLocations,
    /// List a folder inside a location (`path` relative, `""` = root). Replies `Directory`.
    ListDirectory {
        location: BrowseLocation,
        path: String,
    },
    /// Audition a file (`media-preview` node): decoded engine-side and played by the
    /// engine's preview voice straight to the hardware output (not through the tracks,
    /// independent of the transport). Replaces any playing preview. Replies `Unit` once the
    /// preview is queued; `MediaEvent::PreviewStarted` follows, then exactly one
    /// `MediaEvent::PreviewEnded` (end of file, `StopPreview`, replaced, or failed).
    Preview {
        source: MediaSource,
    },
    /// Stop the playing preview (no-op if none).
    StopPreview,
    /// Start uploading a file from the UI machine. `upload` is a client-chosen id (ULID).
    BeginUpload {
        upload: String,
        name: String,
        size: f64,
    },
    /// Bytes `[offset, offset + data.len())` of an upload; `offset` must equal the bytes
    /// received so far. Progress is reported as `MediaEvent::UploadProgress`.
    UploadChunk {
        upload: String,
        offset: f64,
        data: crate::model::Base64Bytes,
    },
    CancelUpload {
        upload: String,
    },
}

/// Where to read a file from. Never a raw file-system path.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum MediaSource {
    /// A file inside an engine-visible browse location.
    Location {
        location: BrowseLocation,
        path: String,
    },
    /// Media already in the current project (e.g. preview).
    Project { media: MediaId },
    /// A completed upload from the UI machine (`BeginUpload`).
    Upload { upload: String },
    /// v0.2 (`file-import`): a file of the **engine machine** chosen by the user in an OS
    /// file dialog or dropped from the OS (desktop shell only: the Tauri dialog and dropped
    /// file paths). The one explicit OS-file handoff: the UI passes the path, never reads the
    /// file. The engine validates it (absolute, a regular readable file with an audio
    /// extension) and imports it as an external reference in place (`media-references`;
    /// copied into the project until that lands). Hosts where the UI is not on the engine
    /// machine (web, remote) reply `Unsupported`: they upload the bytes instead
    /// (`BeginUpload`/`UploadChunk`, then `Upload`, copied into the project).
    Path { path: String },
}

/// A browse root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum BrowseLocation {
    /// A configured library folder, by its id from `ListLocations`.
    Library { id: String },
    /// The current project's `media/` folder.
    ProjectMedia,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct BrowseRoot {
    pub location: BrowseLocation,
    /// Display name (e.g. "Library", "Project media").
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PeakRequest {
    pub media: MediaId,
    /// Source frames per peak (zoom level).
    pub samples_per_peak: u32,
    /// Range in source frames.
    pub start_frame: f64,
    pub frame_count: f64,
}

/// Min/max pairs per channel. `min[ch][i]`/`max[ch][i]` cover source frames
/// `[start_frame + i*samples_per_peak, +samples_per_peak)`, values in -1..=1.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PeakData {
    pub media: MediaId,
    /// Actual level used (>= requested).
    pub samples_per_peak: u32,
    pub start_frame: f64,
    pub min: Vec<Vec<f32>>,
    pub max: Vec<Vec<f32>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DirectoryListing {
    pub location: BrowseLocation,
    /// Relative path of the listed folder (`""` = root).
    pub path: String,
    pub entries: Vec<DirectoryEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DirectoryEntry {
    pub name: String,
    /// Relative path within the location (use with `MediaSource::Location`).
    pub path: String,
    pub kind: FileKind,
    /// File size in bytes (0 for directories).
    pub size: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum FileKind {
    Directory,
    Audio,
    Midi,
    Other,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum MediaEvent {
    ImportProgress {
        media: MediaId,
        progress: f32,
    },
    /// Peaks for `media` can now be requested.
    PeaksReady {
        media: MediaId,
    },
    /// A media file of the current project is missing/unreadable in the store.
    Missing {
        media: MediaId,
    },
    /// The set of browse locations changed (library folder configured, ...).
    LocationsChanged {
        locations: Vec<BrowseRoot>,
    },
    /// Bytes received for an upload (throttled).
    UploadProgress {
        upload: String,
        received: f64,
    },
    /// A preview started playing (`media-preview`).
    PreviewStarted {
        source: MediaSource,
    },
    /// The preview of `source` ended; the UI resets its "previewing" state.
    PreviewEnded {
        source: MediaSource,
        reason: PreviewEndReason,
    },
}

/// Why a preview ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum PreviewEndReason {
    /// Played to the end of the file.
    Finished,
    /// `Media::StopPreview`.
    Stopped,
    /// Another `Media::Preview` replaced it.
    Replaced,
    /// Decoding or playback failed (a notification carries the message).
    Failed,
}

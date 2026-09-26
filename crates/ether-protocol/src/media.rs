//! Media import, peaks (waveform overviews) and the sample browser.
//!
//! All file access is engine-side. The browser lists *engine-visible* locations only: the
//! configured library folder(s) and the current project's own media. Paths inside a
//! location are relative (`"Drums/Kicks/kick1.wav"`) and resolved/sandboxed by the engine;
//! the UI never sees absolute paths. Importing copies the file into the project's
//! `media/` folder, so projects are self-contained.
//!
//! Uploading files from the UI machine is not in v0.1; `MediaSource::Upload` and
//! `MediaCommand::BeginUpload` reserve the protocol slot (hosts reply `Unsupported`).

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
    /// Audition a file (plays on the preview bus).
    Preview {
        source: MediaSource,
    },
    StopPreview,
    /// RESERVED (v0.2+): start streaming a file from the UI machine. Hosts reply
    /// `Unsupported` in v0.1. The intended flow: `BeginUpload` → chunked binary transfer on
    /// a side channel → `Import { source: Upload { upload } }`.
    BeginUpload {
        upload: String,
        name: String,
        size: f64,
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
    /// RESERVED (v0.2+): a completed upload from the UI machine (`BeginUpload`).
    Upload { upload: String },
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
}

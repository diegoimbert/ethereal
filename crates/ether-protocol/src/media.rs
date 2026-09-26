//! Media import, peaks (waveform overviews) and the file browser.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::MediaId;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum MediaCommand {
    /// Decode + register a file (copied into the project on save). Replies `Media` with the
    /// `MediaRef`; peaks are computed in the background (`MediaEvent::PeaksReady`).
    Import {
        id: MediaId,
        source: MediaSource,
    },
    /// Replies `Peaks`. Hosts pick the nearest mipmap level `>= samples_per_peak`.
    GetPeaks {
        request: PeakRequest,
    },
    /// Replies `Directory`. Native only (web: `Unsupported`; the web browser uses OPFS/UI).
    ListDirectory {
        path: String,
    },
    /// Audition a file in the browser (plays on the preview bus).
    Preview {
        source: MediaSource,
    },
    StopPreview,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum MediaSource {
    /// Native file system path.
    Path { path: String },
    /// Browser OPFS path (the UI writes dropped files there first; no large JSON payloads).
    Opfs { path: String },
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
    pub path: String,
    pub entries: Vec<DirectoryEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DirectoryEntry {
    pub name: String,
    pub path: String,
    pub kind: FileKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum FileKind {
    Directory,
    Audio,
    Midi,
    Project,
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
    /// A referenced file is missing on load (relink needed).
    Missing {
        media: MediaId,
    },
}

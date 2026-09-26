//! References to imported media files.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::MediaId;

/// An imported audio file. Decoded data and peak mipmaps live in host caches keyed by
/// `MediaId` (+ `hash`), never in the document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MediaRef {
    pub id: MediaId,
    /// Display name (original file name).
    pub name: String,
    pub location: MediaLocation,
    pub sample_rate: u32,
    pub channels: u16,
    /// Length in sample frames at `sample_rate`.
    #[ts(type = "number")]
    pub frames: u64,
    /// Content hash (hex) of the original file, used to relink and to key caches.
    pub hash: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum MediaLocation {
    /// Relative to the project directory (preferred: `Samples/Imported/<file>`).
    ProjectRelative { path: String },
    /// Absolute path on the local file system (native only).
    Absolute { path: String },
    /// Path inside the browser's Origin Private File System (web only).
    Opfs { path: String },
}

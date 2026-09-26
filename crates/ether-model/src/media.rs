//! References to media owned by the project.
//!
//! Projects are self-contained: importing audio copies it into the project's `media/`
//! folder in the engine-side project store, so a `MediaRef` only ever points inside the
//! project. The UI never sees or supplies file-system paths.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::MediaId;

/// An imported audio file. Decoded data and peak mipmaps live in the store's per-project
/// cache (keyed by `MediaId` + `hash`), never in the document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MediaRef {
    pub id: MediaId,
    /// Display name (original file name).
    pub name: String,
    /// Path relative to the project folder, always under `media/` (e.g.
    /// `media/01J...-kick.wav`). Resolved only by the engine-side `ProjectStore`.
    pub file: String,
    pub sample_rate: u32,
    pub channels: u16,
    /// Length in sample frames at `sample_rate`.
    #[ts(type = "number")]
    pub frames: u64,
    /// Content hash (hex) of the file, used to key caches and dedupe imports.
    pub hash: Option<String>,
}

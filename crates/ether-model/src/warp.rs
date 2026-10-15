//! Warping of audio clips.
//!
//! When `WarpSettings::enabled`, the clip's content timeline (beats) is mapped to source
//! time (seconds) piecewise-linearly through its warp markers, and the audio is
//! time-stretched to follow the project tempo. With fewer than two markers the mapping is
//! derived from `source_bpm`. When disabled, the clip plays at the source's native speed:
//! content beats are converted to seconds at the tempo in effect at the clip start.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{ClipId, WarpMarkerId};
use crate::value::{Beats, Seconds};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct WarpSettings {
    pub enabled: bool,
    pub mode: WarpMode,
    /// Detected or user-set tempo of the source material.
    pub source_bpm: Option<f64>,
}

impl Default for WarpSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: WarpMode::Complex,
            source_bpm: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum WarpMode {
    /// Resample (tempo changes pitch).
    Repitch,
    /// Signalsmith Stretch (the only stretch algorithm in v0.1).
    #[default]
    Complex,
}

/// Pins content beat `beat` of clip `clip` to source time `source`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct WarpMarker {
    pub id: WarpMarkerId,
    pub clip: ClipId,
    pub beat: Beats,
    pub source: Seconds,
}

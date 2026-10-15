//! Warping of audio clips.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{Beats, ClipId, Seconds, WarpMarkerId, WarpSettings};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum WarpCommand {
    SetWarp {
        clip: ClipId,
        warp: WarpSettings,
    },
    AddMarker {
        id: WarpMarkerId,
        clip: ClipId,
        beat: Beats,
        source: Seconds,
    },
    MoveMarker {
        id: WarpMarkerId,
        beat: Beats,
        source: Seconds,
    },
    RemoveMarker {
        id: WarpMarkerId,
    },
    /// Estimate the source tempo. Replies `Tempo`.
    DetectTempo {
        clip: ClipId,
    },
}

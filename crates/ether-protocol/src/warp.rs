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
    /// Guess the source tempo from the clip's file length, with no audio analysis: the tempo
    /// in [80, 160) BPM at which the file spans a power-of-two number of 4/4 bars (right for
    /// loops, a guess for anything else). Replies `Tempo` (`None` when no tempo fits).
    DetectTempo {
        clip: ClipId,
    },
}

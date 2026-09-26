//! Session view: scenes, clip launching.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{Beats, ClipId, Color, SceneId, TimeSignature, TrackId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum SessionCommand {
    CreateScene {
        id: SceneId,
        name: Option<String>,
        before: Option<SceneId>,
    },
    /// Deletes the scene and the session clips in its row.
    DeleteScene {
        id: SceneId,
    },
    DuplicateScene {
        id: SceneId,
        new_id: SceneId,
    },
    RenameScene {
        id: SceneId,
        name: String,
    },
    SetSceneColor {
        id: SceneId,
        color: Option<Color>,
    },
    MoveScene {
        id: SceneId,
        before: Option<SceneId>,
    },
    SetSceneTempo {
        id: SceneId,
        bpm: Option<f64>,
    },
    SetSceneTimeSignature {
        id: SceneId,
        signature: Option<TimeSignature>,
    },
    /// Launch a session clip (quantized per its launch settings). Not undoable.
    LaunchClip {
        clip: ClipId,
    },
    /// Gate mode: release of a launched clip.
    ReleaseClip {
        clip: ClipId,
    },
    /// Stop the playing session clip of a track (quantized). Not undoable.
    StopTrack {
        track: TrackId,
    },
    /// Launch every clip in the row; empty slots stop their track. Not undoable.
    LaunchScene {
        scene: SceneId,
    },
    StopAll,
    /// Return a track from session playback to its arrangement clips.
    BackToArrangement {
        track: Option<TrackId>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum ClipPlayState {
    Stopped,
    /// Launched, waiting for the quantization boundary.
    Queued,
    Playing,
    /// Stop requested, waiting for the quantization boundary.
    Stopping,
    Recording,
}

/// Emitted as `Event::Session` with the slots whose state changed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ClipStateChange {
    pub track: TrackId,
    pub clip: ClipId,
    pub state: ClipPlayState,
}

/// Per-track session playback position, included in the playhead stream while any session
/// clip plays (for the clip progress indicators).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct SessionPlayback {
    pub track: TrackId,
    pub clip: ClipId,
    /// Content-relative position.
    pub position: Beats,
}

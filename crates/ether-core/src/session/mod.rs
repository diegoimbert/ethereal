//! Session-view playback: clip slots, quantized launch/stop, scenes.
//!
//! **Owned by the `core-session` node** (`crates/ether-core/src/session/**`); the `core`
//! node only calls into it from the engine. This stub fixes the public surface.
//!
//! Model: each track has at most one playing session clip. A launch is queued until the
//! next quantization boundary (clip's own quantization or the global one), then the
//! track's arrangement playback is overridden by the session clip until
//! `BackToArrangement`. Session clip data comes from `TrackDesc::session_clips` of the
//! current snapshot; state changes are reported via `EngineOutputs::session`.

use ether_protocol::model::{ClipId, Quantization, SceneId, TrackId};
use serde::{Deserialize, Serialize};

/// Controller → engine session actions (`EngineHandle::session`). RT-applied at the next
/// block; quantization is evaluated on the audio thread against the transport.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum SessionControl {
    LaunchClip {
        track: TrackId,
        clip: ClipId,
    },
    /// Gate launch mode: note/key released.
    ReleaseClip {
        track: TrackId,
        clip: ClipId,
    },
    StopTrack {
        track: TrackId,
    },
    /// Launch all clips of a scene; tracks without a clip in the scene stop.
    LaunchScene {
        scene: SceneId,
    },
    StopAll,
    BackToArrangement {
        track: Option<TrackId>,
    },
    SetGlobalQuantization {
        quantization: Quantization,
    },
}

/// Per-engine session state living on the audio thread (pre-allocated for `max_tracks`).
pub struct SessionState {
    _private: (),
}

impl SessionState {
    /// Non-RT.
    pub fn new(max_tracks: usize) -> Self {
        let _ = max_tracks;
        todo!("core-session node")
    }

    /// RT. Queue a control (from the engine's control ring).
    pub fn apply(&mut self, control: SessionControl) {
        let _ = control;
        todo!("core-session node")
    }
}

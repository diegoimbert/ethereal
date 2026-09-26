//! Clip editing (arrangement and session clips share these commands).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{
    Beats, ClipId, ClipLocation, ClipLoop, Color, Decibels, LaunchSettings, MediaId, TrackId,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ClipCommand {
    CreateMidi {
        id: ClipId,
        track: TrackId,
        location: ClipLocation,
        length: Beats,
        name: Option<String>,
    },
    /// Length/warp defaults are derived from the media (and its detected tempo).
    CreateAudio {
        id: ClipId,
        track: TrackId,
        location: ClipLocation,
        media: MediaId,
    },
    /// Deletes clips with their notes, envelopes and warp markers.
    Delete {
        ids: Vec<ClipId>,
    },
    /// Move several clips at once (multi-selection drag). Overlapping arrangement clips on
    /// the destination are trimmed/removed Ableton-style.
    Move {
        moves: Vec<ClipMove>,
    },
    /// Set timing in one go (left/right edge trims): location, length and content offset.
    SetBounds {
        id: ClipId,
        location: ClipLocation,
        length: Beats,
        offset: Beats,
    },
    /// Split an arrangement clip at timeline position `at`; the right part gets `new_id`.
    Split {
        id: ClipId,
        at: Beats,
        new_id: ClipId,
    },
    /// Copy (with notes/envelopes/markers) to `location` (None = right after the original).
    Duplicate {
        id: ClipId,
        new_id: ClipId,
        location: Option<ClipLocation>,
    },
    Rename {
        id: ClipId,
        name: String,
    },
    SetColor {
        id: ClipId,
        color: Option<Color>,
    },
    SetMuted {
        ids: Vec<ClipId>,
        muted: bool,
    },
    SetLoop {
        id: ClipId,
        looping: ClipLoop,
    },
    SetLaunch {
        id: ClipId,
        launch: LaunchSettings,
    },
    /// Audio clips only.
    SetGain {
        id: ClipId,
        gain: Decibels,
    },
    /// Audio clips only.
    SetTranspose {
        id: ClipId,
        semitones: f32,
    },
    /// Audio clips only.
    SetFades {
        id: ClipId,
        fade_in: Beats,
        fade_out: Beats,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ClipMove {
    pub id: ClipId,
    pub track: TrackId,
    pub location: ClipLocation,
}

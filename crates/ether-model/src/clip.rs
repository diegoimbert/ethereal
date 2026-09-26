//! Clips (audio + MIDI), in the arrangement or in session slots.
//!
//! Time model: a clip has its own *content timeline* in beats (notes, clip envelopes, warp
//! markers and the loop region are expressed on it, starting at 0). `offset` is the content
//! position that plays at the clip's start. With looping disabled the clip plays
//! `[offset, offset + length)` of its content; with looping enabled it plays from `offset`
//! and then repeats `[loop.start, loop.end)` until `length` is reached (arrangement) or
//! forever (session).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{ClipId, MediaId, SceneId, TrackId};
use crate::value::{Beats, Color, Decibels, Quantization};
use crate::warp::WarpSettings;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Clip {
    pub id: ClipId,
    pub track: TrackId,
    pub location: ClipLocation,
    pub name: String,
    /// `None` = inherit the track color.
    pub color: Option<Color>,
    pub muted: bool,
    /// Duration on the arrangement timeline (arrangement clips) / nominal length (session).
    pub length: Beats,
    /// Content position played at the clip start.
    pub offset: Beats,
    pub looping: ClipLoop,
    pub launch: LaunchSettings,
    pub content: ClipContent,
}

/// Where a clip lives. Arrangement and session clips are separate objects (as in Ableton).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ClipLocation {
    /// On the track's arrangement lane, starting at `start`.
    Arrangement { start: Beats },
    /// In the session slot `(clip.track, scene)`. At most one clip per slot.
    Session { scene: SceneId },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct ClipLoop {
    pub enabled: bool,
    /// Content-relative loop region.
    pub start: Beats,
    pub end: Beats,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct LaunchSettings {
    pub mode: LaunchMode,
    /// `None` = use the project's global launch quantization.
    pub quantization: Option<Quantization>,
    /// Start at the play position of the currently playing clip on the same track.
    pub legato: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum LaunchMode {
    #[default]
    Trigger,
    Gate,
    Toggle,
    Repeat,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ClipContent {
    Audio(AudioContent),
    /// Notes are separate entities (`Note::clip == this`), so note edits patch small.
    Midi,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct AudioContent {
    pub media: MediaId,
    pub gain: Decibels,
    /// Pitch shift in semitones (repitch when unwarped, pitch-preserving when warped).
    pub transpose: f32,
    /// Fade in / out lengths in beats (0 = off; the engine still applies a tiny anti-click).
    pub fade_in: Beats,
    pub fade_out: Beats,
    /// Warp settings; markers are separate `WarpMarker` entities.
    pub warp: WarpSettings,
}

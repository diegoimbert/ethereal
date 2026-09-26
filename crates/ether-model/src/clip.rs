//! Clips (audio + MIDI) on a track's arrangement lane.
//!
//! Time model: a clip has its own *content timeline* in beats (notes, clip envelopes, warp
//! markers and the loop region are expressed on it, starting at 0). `offset` is the content
//! position that plays at the clip's start. With looping disabled the clip plays
//! `[offset, offset + length)` of its content; with looping enabled it plays from `offset`
//! and then repeats `[loop.start, loop.end)` until `length` is reached.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{ClipId, MediaId, TrackId};
use crate::value::{Beats, Color, Decibels};
use crate::warp::WarpSettings;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Clip {
    pub id: ClipId,
    pub track: TrackId,
    /// Position on the track's arrangement lane.
    pub start: Beats,
    pub name: String,
    /// `None` = inherit the track color.
    pub color: Option<Color>,
    pub muted: bool,
    /// Duration on the arrangement timeline.
    pub length: Beats,
    /// Content position played at the clip start.
    pub offset: Beats,
    pub looping: ClipLoop,
    pub content: ClipContent,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct ClipLoop {
    pub enabled: bool,
    /// Content-relative loop region.
    pub start: Beats,
    pub end: Beats,
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

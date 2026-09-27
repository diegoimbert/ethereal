//! Clips (audio + MIDI) on a track's arrangement lane.
//!
//! Time model: a clip has its own *content timeline* in beats (notes, clip envelopes, warp
//! markers and the loop region are expressed on it, starting at 0). `offset` is the content
//! position that plays at the clip's start. With looping disabled the clip plays
//! `[offset, offset + length)` of its content; with looping enabled it plays from `offset`
//! and then repeats `[loop.start, loop.end)` until `length` is reached.
//!
//! # Overlaps and crossfades (roadmap v2, `clip-editing`)
//! Clip edits (`Move`, `SetBounds`, ...) keep clips on a track from overlapping by trimming
//! the covered clip, except where the overlap is a **crossfade**: the earlier clip `A`
//! ends inside the later clip `B`, and the overlap `A.end - B.start` is at most both
//! `A.fade_out` and `B.fade_in`. The engine renders every clip independently and sums them,
//! so a crossfade is simply `A`'s fade-out overlapping `B`'s fade-in (use the same curve
//! on both, e.g. `EqualPower`, for a symmetric crossfade). Overlaps are not a model
//! invariant (documents may contain them); larger overlaps just sum.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{ClipId, MediaId, TakeLaneId, TrackId};
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
    /// Take lane this clip is on (v0.2, `comping`; see [`crate::take`]). `None` = the track's
    /// main lane. Take-lane clips never play directly; comp regions select what plays. Omitted
    /// from JSON when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub lane: Option<TakeLaneId>,
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
    /// Shape of the fade-in (roadmap v2, `clip-editing`; `.ether` v3).
    pub fade_in_curve: FadeCurve,
    /// Shape of the fade-out (roadmap v2, `clip-editing`; `.ether` v3).
    pub fade_out_curve: FadeCurve,
    /// Play the source backwards (roadmap v2, `clip-editing`; `.ether` v3). The clip then
    /// behaves as if its media file were reversed: source time `t` reads the media at
    /// `media_length - t`, and `offset`, the loop region and warp markers are all expressed
    /// on that reversed timeline (toggling it does not move them; the controller may mirror
    /// warp markers in the same transaction).
    pub reversed: bool,
}

/// Gain shape of a clip fade, as a function of the normalized position `x` in the fade
/// (0 = silent end, 1 = full level). Fade-outs use the same curve mirrored in time, so a
/// fade-out of clip A overlapping a fade-in of clip B with the same curve is a symmetric
/// crossfade.
///
/// - `Linear`: `g = x` (the v0.1 behaviour and the migration default).
/// - `EqualPower`: `g = sin(x·π/2)`; constant power across a crossfade of uncorrelated
///   material.
/// - `Curve { tension }`: the automation curve law (`CurveShape::Curve`) with `tension` in
///   -1..=1 (0 = linear, > 0 = slow start, < 0 = fast start).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum FadeCurve {
    #[default]
    Linear,
    EqualPower,
    Curve {
        tension: f32,
    },
}

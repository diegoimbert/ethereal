//! Clips (audio + MIDI) on a track's arrangement lane.
//!
//! Time model: a clip has its own *content timeline* in beats (notes, clip envelopes, warp
//! markers and the loop region are expressed on it, starting at 0). `offset` is the content
//! position that plays at the clip's start. With looping disabled the clip plays
//! `[offset, offset + length)` of its content; with looping enabled it plays from `offset`
//! and then repeats `[loop.start, loop.end)` until `length` is reached.
//!
//! `offset` may be negative: content positions before 0 are empty (silence
//! before the audio's first sample; no notes until some are put there), so a negative
//! offset is a *leading gap* of `-offset` beats before the content starts. A MIDI clip's
//! notes may start in that gap (down to `min(offset, 0)`): the piano roll shows and edits
//! the whole clip, and such notes play like any other. Extending a clip's left edge past its content
//! start produces one on unlooped clips; looped clips instead wrap the offset into the loop
//! region (the loop's tail plays before the loop start). Repitch transposition scales
//! around `max(offset, 0)`, so the content still starts right after the gap.
//!
//! # Overlaps and crossfades (roadmap v2)
//! Clip edits (`Move`, `SetBounds`, ...) keep clips on a track from overlapping by trimming
//! the covered clip, except where the overlap is a **crossfade**: the earlier clip `A`
//! ends inside the later clip `B`, and the overlap `A.end - B.start` is at most both
//! `A.fade_out` and `B.fade_in`. The engine renders every clip independently and sums them,
//! so a crossfade is simply `A`'s fade-out overlapping `B`'s fade-in (use the same curve
//! on both, e.g. `EqualPower`, for a symmetric crossfade). Overlaps are not a model
//! invariant (documents may contain them); larger overlaps just sum. `ClipCommand::
//! SetCrossfade` (`crossfades`) sizes or removes one around a fixed boundary.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{ClipId, MediaId, TakeLaneId, TrackId};
use crate::note::Note;
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
    /// Content position played at the clip start. Negative = a leading gap of `-offset`
    /// beats before the content (module docs).
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

/// Shortest content loop the engine plays (beats; `ether_core::sched::MIN_LOOP`).
pub const MIN_CLIP_LOOP: f64 = 1.0 / 256.0;

impl Clip {
    /// Content position played `d` beats after the clip start: the loop unrolled exactly like
    /// the engine (`ether_core::sched::for_each_piece`), which loops only when the loop is at
    /// least [`MIN_CLIP_LOOP`] long and `offset < loop end`. Positions past the loop end wrap
    /// into `[loop start, loop end)`. Splitting or trimming a clip at `d` gives the remaining
    /// part this offset, so it keeps playing what the whole clip played there.
    pub fn content_at(&self, d: f64) -> f64 {
        let pos = self.offset.0 + d;
        let (ls, le) = (self.looping.start.0, self.looping.end.0);
        if !self.looping.enabled || le - ls < MIN_CLIP_LOOP || self.offset.0 >= le || pos < le {
            return pos;
        }
        ls + (pos - le).rem_euclid(le - ls)
    }

    /// The content loop `(start, end)` the engine plays (`ether_core::sched::content_loop`):
    /// enabled, at least [`MIN_CLIP_LOOP`] long and `offset < loop end`.
    pub fn played_loop(&self) -> Option<(f64, f64)> {
        let (ls, le) = (self.looping.start.0, self.looping.end.0);
        (self.looping.enabled && le - ls >= MIN_CLIP_LOOP && self.offset.0 < le).then_some((ls, le))
    }

    /// Loop join (CONTRACTS.md §2, Clips, "loop join"; the engine's
    /// `ether_core::sched::voice_end`): an unmuted note of `pitch` cut by the content loop
    /// wrap `d` beats after the clip start keeps sounding when the loop has a join partner,
    /// the unmuted note of that pitch starting before the loop start and still sounding
    /// there (the latest-ending one). Returns the partner and how long the note keeps
    /// sounding after the wrap: until the partner ends, or to the clip end when the partner
    /// is longer than the loop (it joins itself on every wrap). `None`: no partner, or no
    /// clip after `d`. `notes` are the clip's notes; the caller checks that the note crosses
    /// the loop end.
    pub fn loop_join<'a>(
        &self,
        notes: impl IntoIterator<Item = &'a Note>,
        pitch: u8,
        d: f64,
    ) -> Option<(&'a Note, f64)> {
        let (ls, le) = self.played_loop()?;
        let rest = self.length.0 - d;
        if rest <= LOOP_JOIN_EPS {
            return None;
        }
        let partner = notes
            .into_iter()
            .filter(|n| {
                !n.muted
                    && n.pitch == pitch
                    && n.start.0 < ls - LOOP_JOIN_EPS
                    && n.start.0 + n.duration.0 > ls + LOOP_JOIN_EPS
            })
            .max_by(|a, b| {
                (a.start.0 + a.duration.0)
                    .total_cmp(&(b.start.0 + b.duration.0))
                    .then(b.start.0.total_cmp(&a.start.0))
            })?;
        let tail = partner.start.0 + partner.duration.0 - ls;
        let after = if tail > le - ls + LOOP_JOIN_EPS {
            rest
        } else {
            tail.min(rest)
        };
        Some((partner, after))
    }
}

/// Loop-join tolerance (beats, the engine's): a note ending exactly on the loop end, or
/// starting exactly on the loop start, does not join.
pub const LOOP_JOIN_EPS: f64 = 1e-9;

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
    /// Shape of the fade-in (roadmap v2; pre-release `.ether` v3).
    pub fade_in_curve: FadeCurve,
    /// Shape of the fade-out (roadmap v2; pre-release `.ether` v3).
    pub fade_out_curve: FadeCurve,
    /// Play the source backwards (roadmap v2; pre-release `.ether` v3). The clip then
    /// behaves as if its media file were reversed (frame `i` reads the media's frame
    /// `N - 1 - i`), and `offset`, the loop region and warp markers are all expressed on that
    /// reversed timeline. The flag alone does not move them; the controller's reverse
    /// (`ClipCommand::SetReversed`) mirrors them in the same transaction
    /// ([`crate::reverse`]), so the clip plays the same material backwards.
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

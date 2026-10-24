//! Clip editing (arrangement clips).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{Beats, ClipId, ClipLoop, Color, Decibels, FadeCurve, MediaId, TrackId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ClipCommand {
    CreateMidi {
        id: ClipId,
        track: TrackId,
        start: Beats,
        length: Beats,
        name: Option<String>,
    },
    /// Length/warp defaults are derived from the media (and its detected tempo).
    CreateAudio {
        id: ClipId,
        track: TrackId,
        start: Beats,
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
    /// Set timing in one go (left/right edge trims): start, length and content offset.
    SetBounds {
        id: ClipId,
        start: Beats,
        length: Beats,
        offset: Beats,
    },
    /// Split an arrangement clip at timeline position `at`; the right part gets `new_id`.
    Split {
        id: ClipId,
        at: Beats,
        new_id: ClipId,
    },
    /// Copy (with notes/envelopes/markers) to `start` on the same track (None = right after
    /// the original).
    Duplicate {
        id: ClipId,
        new_id: ClipId,
        start: Option<Beats>,
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
    /// Audio clips only.
    SetGain {
        id: ClipId,
        gain: Decibels,
    },
    /// Audio clips only. When the transpose changes the playback speed (unwarped or
    /// `Repitch`), the clip keeps its audio and its length follows (`length / rate`), its
    /// loop region too; a longer clip is trimmed at the start of the next clip on its lane
    /// and fades are clamped (one undo step; `ether_controller::clip_editing::transpose`).
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
    /// Roadmap v2, audio clips only. `None` = unchanged.
    SetFadeCurves {
        id: ClipId,
        fade_in: Option<FadeCurve>,
        fade_out: Option<FadeCurve>,
    },
    /// Roadmap v2, audio clips only. See `AudioContent::reversed`.
    SetReversed {
        id: ClipId,
        reversed: bool,
    },
    /// Roadmap v2: crossfade two audio clips on the same track where
    /// `first` ends at or after `second` starts. Extends them into each other as needed so
    /// they overlap by `length` beats around the boundary (source material permitting) and
    /// sets `first.fade_out = second.fade_in = length` with `curve` on both (see the
    /// overlap rules in `ether_model::clip`). One undo step.
    Crossfade {
        first: ClipId,
        second: ClipId,
        length: Beats,
        curve: FadeCurve,
    },
    /// `crossfades`: set the crossfade between `first` and `second` (same track,
    /// `first` ending at or after `second` starts) to `length` beats centred on `at` (a
    /// position strictly inside both clips' combined span; the fade-handle gestures keep the
    /// boundary they started from, so dragging never drifts it). Same extension rules as
    /// `Crossfade` (source material permitting, the other side makes up the difference).
    /// `length = 0` **removes** the crossfade: both clips are trimmed to meet at `at`, and
    /// `first.fade_out` / `second.fade_in` reset to 0. `curve: None` keeps the clips'
    /// current curves (`first.fade_out_curve` / `second.fade_in_curve`). One undo step;
    /// several within one gesture merge into one.
    SetCrossfade {
        first: ClipId,
        second: ClipId,
        at: Beats,
        length: Beats,
        curve: Option<FadeCurve>,
    },
    /// Move the clip's bounds to `start .. start + length` and time-scale its content by
    /// `length / old length` so it fills them (it plays faster or slower): notes, expression
    /// lanes, the loop region, fades, clip envelopes and warp markers (an unwarped audio clip
    /// gets warped with its mode). Overlaps resolve like `SetBounds`. One undo step
    /// (`ether_controller::doc::stretch`).
    Stretch {
        id: ClipId,
        start: Beats,
        length: Beats,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ClipMove {
    pub id: ClipId,
    pub track: TrackId,
    pub start: Beats,
}

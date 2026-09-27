//! Takes and comping (v0.2, owned by the `comping` node; see `docs/ROADMAP.md` "v0.2" and
//! CONTRACTS.md §12.2).
//!
//! # Model
//! - A [`TakeLane`] belongs to one audio or MIDI track (`TakeLane::track`). Lanes are ordered
//!   by `order` (fractional index), newest take last by convention.
//! - **Take clips are ordinary clips** with `Clip::lane = Some(lane)` (`lane.track ==
//!   clip.track`), so notes, warp markers, fades, clip envelopes and every clip command work
//!   on them unchanged. Clips with `lane: None` are the track's main lane (what v0.1 calls
//!   "the arrangement"). [`crate::Project::arrangement_clips_of`] lists main-lane clips only;
//!   [`crate::Project::lane_clips_of`] lists a lane's clips.
//! - A [`CompRegion`] says "play `[start, end)` of this track from `lane`". Regions of one
//!   track never overlap (model invariant; ranges are half-open, so touching is fine).
//!
//! # Playback (engine compile, `comping` implements it in the controller)
//! - Take-lane clips are **never** played directly.
//! - Each comp region plays the clips of its lane restricted to the region (clip pieces are
//!   trimmed to `[start, end)`, keeping their content offset).
//! - **Boundary crossfades** (audio tracks): where region `A` ends exactly where region `B`
//!   starts, the two overlap by `B.crossfade` seconds centred on the boundary (each side is
//!   extended by half of it, source material permitting, with equal-power fades). At the
//!   outer edges of a run of regions (nothing adjacent) a short anti-click fade of
//!   `crossfade / 2` is applied instead. MIDI tracks ignore `crossfade`: notes are cut at the
//!   region end and notes starting before the region start don't sound.
//! - Main-lane clips keep playing as before. Where a main-lane clip and a comp region
//!   overlap, both sound (like overlapping clips); the comping UX keeps them apart
//!   ("flatten" turns the comp into main-lane clips and deletes the regions).
//! - A lane may be empty at a region's range (silence there).
//!
//! # Swipe comping (UX, frozen command shape)
//! Swiping across a take lane over `[a, b)` is **one** undoable `Take::SetComp` command: the
//! range becomes a region of that lane, existing regions are trimmed/split/removed around it
//! (a split region's right part gets the command's `split_id`), and adjacent regions of the
//! same lane are **not** merged (so undo restores exactly).
//!
//! # Recording
//! Loop recording with takes on creates one lane per loop pass (the pass's clip goes on its
//! lane) and a comp region spanning the pass range that selects the newest lane. Punch
//! recording creates one lane per punch pass the same way.
//!
//! Deleting a track cascades its regions, then its lane clips, then its lanes (the
//! controller emits the removals, children first).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{CompRegionId, TakeLaneId, TrackId};
use crate::value::{Beats, Color, OrderKey, Seconds};

/// Default boundary crossfade of a new comp region: 5 ms.
pub const DEFAULT_COMP_CROSSFADE: Seconds = Seconds(0.005);
/// Longest allowed boundary crossfade (seconds).
pub const MAX_COMP_CROSSFADE: f64 = 0.5;

/// A take lane of a track.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct TakeLane {
    pub id: TakeLaneId,
    /// An audio or MIDI track.
    pub track: TrackId,
    /// Order among the track's lanes.
    pub order: OrderKey,
    pub name: String,
    /// `None` = the track color.
    pub color: Option<Color>,
}

/// A time range of a track played from one take lane.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct CompRegion {
    pub id: CompRegionId,
    pub track: TrackId,
    /// A lane of `track`.
    pub lane: TakeLaneId,
    /// Timeline range `[start, end)`, `0 <= start < end`.
    pub start: Beats,
    pub end: Beats,
    /// Crossfade at this region's boundaries with adjacent regions, `0..=MAX_COMP_CROSSFADE`
    /// seconds (see the module docs). New regions use [`DEFAULT_COMP_CROSSFADE`].
    pub crossfade: Seconds,
}

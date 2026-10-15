//! Takes and comping (v0.2, `comping` node). Document commands, all undoable. Data model and
//! playback rules: `ether_model::take`; CONTRACTS.md §12.2.
//!
//! Take clips are ordinary clips (`Clip::lane`): edit them with `ClipCommand`/`NoteCommand` by
//! clip id. `ClipCommand::Move` keeps a clip on its lane; `MoveToLane` changes the lane.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{Beats, ClipId, Color, CompRegionId, Seconds, TakeLaneId, TrackId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum TakeCommand {
    /// New empty lane on an audio/MIDI track, before `before` (None = last). `name: None` =
    /// "Take N".
    CreateLane {
        id: TakeLaneId,
        track: TrackId,
        name: Option<String>,
        before: Option<TakeLaneId>,
    },
    /// Removes the lane with its clips and the comp regions that select it.
    RemoveLane {
        id: TakeLaneId,
    },
    RenameLane {
        id: TakeLaneId,
        name: String,
    },
    SetLaneColor {
        id: TakeLaneId,
        color: Option<Color>,
    },
    MoveLane {
        id: TakeLaneId,
        before: Option<TakeLaneId>,
    },
    /// Move clips between the main lane (`lane: None`) and a take lane of the same track.
    MoveToLane {
        clips: Vec<ClipId>,
        lane: Option<TakeLaneId>,
    },
    /// The swipe gesture: `[start, end)` of `track` now plays from `lane`. Existing regions
    /// are trimmed, split (the right part of a region split in two gets `split_id`) or
    /// removed; the new region gets `id` and the default crossfade. One undo step.
    SetComp {
        id: CompRegionId,
        split_id: CompRegionId,
        track: TrackId,
        lane: TakeLaneId,
        start: Beats,
        end: Beats,
    },
    /// Remove the comp in `[start, end)` (trim/split with `split_id` as above).
    ClearComp {
        track: TrackId,
        start: Beats,
        end: Beats,
        split_id: CompRegionId,
    },
    /// Boundary crossfade of one region (0..=0.5 s).
    SetCrossfade {
        region: CompRegionId,
        crossfade: Seconds,
    },
    /// Bake the comp into main-lane clips (one per comp piece, trimmed, with the boundary
    /// crossfades as clip fades) and delete the regions; take lanes are kept (`keep_lanes:
    /// false` removes them and their clips). New clip ids are
    /// `derive_id(seed, i)` in timeline order (`ether_model::derive_id`); MIDI pieces copy
    /// their notes with `derive_id(seed_notes, j)` in (clip, note start, pitch) order.
    Flatten {
        track: TrackId,
        seed: ClipId,
        seed_notes: ClipId,
        keep_lanes: bool,
    },
    /// Runtime, site-local (like `DrumRack::SetPadSolo`): the track plays `lane`'s clips
    /// instead of its comp until `lane: None`. No document change (no ops, not undoable, not
    /// saved or replicated); cleared when the lane or track goes and on project close.
    Audition {
        track: TrackId,
        lane: Option<TakeLaneId>,
    },
}

//! Time-selection edits across tracks (v0.2, `time-edits` node; CONTRACTS.md §12.3).
//!
//! A time selection is a beat range plus a set of tracks (empty = every audio, MIDI and group
//! track, never master/returns). "Time" commands shift everything after the range on the
//! selected tracks: clips, take-lane clips, comp regions, track automation points, and (when
//! `global` is set and the selection covers every track) markers, tempo points and time
//! signatures after the range. Clips crossing an edge are split first.
//!
//! All are document commands, one undo step each. Entities they create get ids from the
//! command's seed (`ether_model::derive_id(seed, i)`, `i` in (track order, then start, then
//! entity id) order), so replays on other sites mint the same ids.
//!
//! The time clipboard (`Copy`/`Cut`) is controller runtime state (per connection, not in the
//! document, not undoable): it holds the copied clips (with notes, envelopes, warp markers),
//! take-lane clips, comp regions and automation points, relative to the range start.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{Beats, ClipId, TrackId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum TimeEditCommand {
    /// Split every clip crossing `at` on `tracks` (main lanes and take lanes). The right
    /// parts get `derive_id(seed, i)`.
    Split {
        tracks: Vec<TrackId>,
        at: Beats,
        seed: ClipId,
    },
    /// Copy the selection to the time clipboard (not undoable, no document change).
    Copy { selection: TimeSelection },
    /// Copy, then `DeleteTime`.
    Cut {
        selection: TimeSelection,
        seed: ClipId,
    },
    /// Paste the time clipboard at `at` on `tracks` (clipboard track `i` → `tracks[i]`,
    /// matching kinds only; empty = the original tracks). `insert`: shift the material after
    /// `at` right by the clipboard length first (Ableton "paste time"); else overwrite.
    Paste {
        at: Beats,
        tracks: Vec<TrackId>,
        insert: bool,
        seed: ClipId,
    },
    /// Remove the range and shift everything after it left by its length.
    DeleteTime {
        selection: TimeSelection,
        seed: ClipId,
    },
    /// Insert `length` beats of silence at `at` (shift right).
    InsertSilence {
        tracks: Vec<TrackId>,
        at: Beats,
        length: Beats,
        global: bool,
        seed: ClipId,
    },
    /// Insert a copy of the selection right after it (shifting the rest right).
    DuplicateTime {
        selection: TimeSelection,
        seed: ClipId,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct TimeSelection {
    pub start: Beats,
    pub end: Beats,
    /// Empty = every audio, MIDI and group track.
    pub tracks: Vec<TrackId>,
    /// Also shift markers, tempo points and time signatures (only when the selection covers
    /// every track).
    pub global: bool,
}

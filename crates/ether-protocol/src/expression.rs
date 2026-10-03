//! MIDI expression editing (v0.3, `midi-expression`; MPE parts `mpe`). Document commands,
//! all undoable. Data model and value ranges: `ether_model::expression`; CONTRACTS.md §13.2.
//!
//! Curves are whole values: `SetPoints` replaces a curve, `ReplaceRange` replaces the points
//! of `[start, end)` (a pencil/eraser stroke: the UI sends one per gesture step with the
//! same gesture id, so a stroke is one undo step). Every command is idempotent.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{
    Beats, ClipId, ExpressionKind, ExpressionLaneId, ExpressionPoint, MpeSettings,
    NoteExpressionId, NoteExpressionKind, NoteId, TrackId,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ExpressionCommand {
    /// New lane on a MIDI clip. If the clip already has a lane of `kind`, succeeds without
    /// changes (that lane keeps its id).
    CreateLane {
        id: ExpressionLaneId,
        clip: ClipId,
        kind: ExpressionKind,
    },
    RemoveLane {
        id: ExpressionLaneId,
    },
    /// Replace a lane's whole curve (sorted by time; values in the kind's range).
    SetPoints {
        lane: ExpressionLaneId,
        points: Vec<ExpressionPoint>,
    },
    /// Replace the points with `start <= time < end` by `points` (all inside the range).
    ReplaceRange {
        lane: ExpressionLaneId,
        start: Beats,
        end: Beats,
        points: Vec<ExpressionPoint>,
    },
    /// Set a note's curve of `kind` (times from the note start): creates it with `id` or
    /// replaces the points of the note's existing curve of that kind (`id` then unused).
    /// Empty `points` removes the curve.
    SetNoteExpression {
        id: NoteExpressionId,
        note: NoteId,
        kind: NoteExpressionKind,
        points: Vec<ExpressionPoint>,
    },
    /// Remove the curves of `kind` (all kinds when `None`) of these notes.
    ClearNoteExpressions {
        notes: Vec<NoteId>,
        kind: Option<NoteExpressionKind>,
    },
    /// MIDI tracks only (`mpe`): MPE settings (`None` = plain MIDI).
    SetTrackMpe {
        track: TrackId,
        mpe: Option<MpeSettings>,
    },
}

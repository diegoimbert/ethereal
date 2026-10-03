//! Undo history panel (v0.3, `undo-history`; CONTRACTS.md §13.8): list the steps, jump to a
//! state, name checkpoints.
//!
//! The list is the controller's `ether_model::History` (undo stack oldest first, then the
//! redo stack): steps are labelled like the Undo menu. In a collab session it lists **this
//! site's own steps only** (per-site undo, docs/COLLAB.md): jumping undoes/redoes own steps,
//! peers' later edits stay. Runtime only: never saved (a new history starts on open), not
//! undoable; checkpoint names are kept for the session (`project-versions` can snapshot one
//! as a named version).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Stable id of a history step for the session (monotonic, never reused; merged gesture
/// commits keep the step's id).
pub type HistoryStepId = u32;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum HistoryCommand {
    /// Replies `History`.
    List,
    /// Undo or redo until `step` is the last applied step (`None` = before the oldest kept
    /// step: undo everything). The patches arrive as one batch of undo/redo patches; replies
    /// `History`. `NotFound` for unknown or dropped steps.
    JumpTo { step: Option<HistoryStepId> },
    /// Name a step (a checkpoint) or clear its name (`None`).
    SetCheckpoint {
        step: HistoryStepId,
        name: Option<String>,
    },
}

/// One undo step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct HistoryStep {
    pub id: HistoryStepId,
    /// Undo label ("Set Volume", a `Batch` label).
    pub label: String,
    /// Controller wall clock (Unix ms) of the step's first commit.
    #[ts(type = "number")]
    pub time_ms: u64,
    /// `true` = on the redo stack (undone).
    pub undone: bool,
    pub checkpoint: Option<String>,
}

/// The whole list (reply to `List`/`JumpTo`, payload of `HistoryEvent::Changed`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct HistoryList {
    /// Chronological: the applied steps oldest first, then the undone steps in the order
    /// they would be redone (so the list reads as the timeline of edits).
    pub steps: Vec<HistoryStep>,
    /// The last applied step (`None` = nothing applied).
    pub current: Option<HistoryStepId>,
    /// Steps were dropped from the front (history depth); jumping before them is impossible.
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum HistoryEvent {
    /// Sent after each change of the history while at least one client listed it (the
    /// panel is open), at most every 100 ms.
    Changed { history: HistoryList },
}

//! Project lifecycle and edit history (undo/redo, gestures, batches).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::Command;
use crate::model::GestureId;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ProjectCommand {
    /// Replace the current project with a new empty one. Replies `Project`.
    New,
    /// Load a project. Replies `Project` (also emitted as `Event::ProjectLoaded`).
    Open { source: ProjectSource },
    /// Save. `target: None` = current location (error if never saved). Replies `Saved`.
    Save { target: Option<ProjectTarget> },
    /// Full current document. Replies `Project`. Used on (re)connect and on revision gaps.
    Get,
    /// Undoable.
    SetName { name: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ProjectSource {
    /// Native: an `.ether` file path.
    Path { path: String },
    /// Web (or tests): the `.ether` JSON text itself.
    Json { json: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ProjectTarget {
    /// Native: write to this `.ether` path (media copied next to it under `Samples/`).
    Path { path: String },
    /// Web: return the `.ether` JSON in the reply (the UI downloads it / writes to OPFS).
    Json,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum EditCommand {
    Undo,
    Redo,
    /// Closes a gesture: subsequent commands start a new undo step. See
    /// `CommandEnvelope::gesture`.
    EndGesture {
        gesture: GestureId,
    },
    /// Apply several commands as ONE undo step (all-or-nothing). Only document-mutating
    /// commands are allowed inside (no transport, queries or nested batches).
    Batch {
        label: String,
        commands: Vec<Command>,
    },
}

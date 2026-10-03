//! Project lifecycle (engine-side project store) and edit history (undo/redo, gestures,
//! batches).
//!
//! **All file handling is engine-side.** The UI may run on another machine than the
//! engine, so it never reads/writes files or sends file-system paths: projects are
//! addressed by `ProjectId` (UUIDv7) inside the engine's `ProjectStore`
//! (`<projects_root>/<project-uuid>/project.ether` + `media/` + `cache/`).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::Command;
use crate::model::{GestureId, ProjectId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ProjectCommand {
    /// List stored projects. Replies `Projects`.
    List,
    /// Create a new empty project with a client-chosen id, save it to the store and make it
    /// the current project. Replies `Project` (also emitted as `Event::ProjectLoaded`).
    Create { id: ProjectId, name: String },
    /// Open a stored project (the current one is autosaved first if dirty). Replies
    /// `Project` (also emitted as `Event::ProjectLoaded`).
    Open { id: ProjectId },
    /// Save the current project to the store. Replies `Saved`.
    Save,
    /// Copy the current project (document + media) under `new_id` with `name`, and switch
    /// to the copy. Replies `Project`.
    SaveAs { new_id: ProjectId, name: String },
    /// Copy a stored project (document + media) under `new_id` without opening it.
    /// Replies `Saved` (the copy's summary).
    Duplicate {
        id: ProjectId,
        new_id: ProjectId,
        name: String,
    },
    /// Rename a project. For the current project this is an undoable document edit; for a
    /// stored one the store rewrites its file. The folder never moves.
    Rename { id: ProjectId, name: String },
    /// Delete a stored project (not the current one). Not undoable.
    Delete { id: ProjectId },
    /// Full current document. Replies `Project`. Used on (re)connect and on revision gaps.
    Get,
    /// Undoable project scale metadata; never restricts MIDI notes.
    SetScale { scale: crate::model::MusicalScale },
}

/// One entry of the project list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ProjectSummary {
    pub id: ProjectId,
    pub name: String,
    /// Last save time, Unix epoch milliseconds.
    pub modified_ms: f64,
    /// base-115: shared by this app, or an offline copy of someone's shared project
    /// (Recents badge and avatars; docs/SHARING.md §8.5). Omitted when not shared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub share: Option<crate::share::ProjectShareInfo>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ProjectEvent {
    /// The stored project list changed (create/save/rename/duplicate/delete, or external
    /// changes the store noticed).
    ListChanged { projects: Vec<ProjectSummary> },
    /// The current project was saved (explicitly or by autosave).
    Saved { project: ProjectSummary },
    /// Unsaved changes flag of the current project.
    DirtyChanged { dirty: bool },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum EditCommand {
    Undo,
    Redo,
    /// Closes a gesture: subsequent commands start a new undo step. See
    /// `ClientMessage::gesture`.
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

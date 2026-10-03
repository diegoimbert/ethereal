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
use crate::model::{DeviceId, GestureId, ProjectId};

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
    /// base-131: open a stored project in safe mode: every plugin device is kept as a
    /// bypassed placeholder (not instantiated; its state and params stay untouched in the
    /// document), and `Event::Project { SafeMode }` lists them. Nothing is written to the
    /// store. Replies `Project` (also emitted as `Event::ProjectLoaded`).
    OpenSafe { id: ProjectId },
    /// base-131: leave safe mode: instantiate the plugin devices held as placeholders.
    /// Emits `SafeMode { active: false, devices: [] }` (nothing when not in safe mode).
    /// Replies `Unit`.
    LoadPlugins,
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
    // --- base-114 (project actions) ---
    /// Pack a project (the open one: its current state) into one portable `.ether` bundle
    /// file: a ZIP archive (entries stored, not compressed) holding `project.ether` and the
    /// project's `media/` files. External media references (`MediaLocation::External`) stay
    /// references. `path`: an absolute engine-machine path chosen in the desktop's OS save
    /// dialog (the one explicit OS-file handoff, like `MediaSource::Path`): the host writes
    /// the file there and replies `Unit`; hosts without OS files (web, remote) reply
    /// `Unsupported`. `None`: replies `Bundle` with a download to pull with
    /// `Export::ReadChunk` and drop with `Export::Release`.
    ExportBundle { id: ProjectId, path: Option<String> },
    /// Unpack a `.ether` bundle (from `ExportBundle`) as a new stored project `new_id` (not
    /// opened). `name`: the new project's name (`None` = the name inside the bundle). A bare
    /// `project.ether` document (no archive) is accepted too. Replies `Saved`.
    ImportBundle {
        new_id: ProjectId,
        source: BundleSource,
        name: Option<String>,
    },
}

/// Where `ImportBundle` reads the bundle from. Never a UI-side path.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum BundleSource {
    /// A completed upload (`Media::BeginUpload` / `UploadChunk`).
    Upload { upload: String },
    /// An absolute engine-machine path from the desktop's OS open dialog (`Unsupported`
    /// elsewhere).
    Path { path: String },
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
    /// base-131: safe mode of the open project (`OpenSafe`: `active`, with the plugin
    /// devices held as bypassed placeholders; `LoadPlugins`: not active, empty). Opening a
    /// project normally leaves safe mode without this event (`ProjectLoaded` resets it).
    SafeMode {
        active: bool,
        devices: Vec<DeviceId>,
    },
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

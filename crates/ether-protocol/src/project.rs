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
    /// Open a stored project in safe mode: every plugin device is kept as a
    /// bypassed placeholder (not instantiated; its state and params stay untouched in the
    /// document), and `Event::Project { SafeMode }` lists them. Nothing is written to the
    /// store. Replies `Project` (also emitted as `Event::ProjectLoaded`).
    OpenSafe { id: ProjectId },
    /// Leave safe mode: instantiate the plugin devices held as placeholders.
    /// Emits `SafeMode { active: false, devices: [] }` (nothing when not in safe mode).
    /// Replies `Unit`.
    LoadPlugins,
    /// Save the current project to the store. Replies `Saved`.
    Save {
        /// A routine autosave (the UI's timer after edits), not the user asking to save:
        /// plugin states are re-read only from instances that reported a change, and the
        /// file may be written in the background. Absent (the default, and what older
        /// clients send) = an explicit save: every plugin's live state is read and the file
        /// is on disk when the reply comes.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        #[ts(as = "Option<bool>", optional)]
        autosave: bool,
    },
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
    /// The user discarded a project (an untitled one, on leaving it): delete it, its versions
    /// and recovery data. A project that is not open: as `Delete`. The open one: nothing of
    /// it is saved from now on (no autosave, no save before a switch or on quit), and its
    /// folder is deleted once it closes (another project opens, or the engine stops).
    /// Not undoable. Replies `Unit`.
    Discard { id: ProjectId },
    /// Full current document. Replies `Project`. Used on (re)connect and on revision gaps.
    Get,
    /// Undoable project scale metadata; never restricts MIDI notes.
    SetScale { scale: crate::model::MusicalScale },
    // --- project actions ---
    /// Pack a project (the open one: its current state) into one portable `.ether` bundle
    /// file: a ZIP archive (entries stored, not compressed) holding `project.ether` and, with
    /// `include_media` (the default), the project's `media/` files plus a copy of every
    /// external media reference (`MediaLocation::External`, packed at its `MediaRef::file`
    /// and made a project copy in the packed document; unreadable ones stay references).
    /// Without it, the document alone. `path`: an absolute engine-machine path chosen in the desktop's OS save
    /// dialog (the one explicit OS-file handoff, like `MediaSource::Path`): the host writes
    /// the file there and replies `Unit`; hosts without OS files (web, remote) reply
    /// `Unsupported`. `None`: replies `Bundle` with a download to pull with
    /// `Export::ReadChunk` and drop with `Export::Release`.
    ExportBundle {
        id: ProjectId,
        path: Option<String>,
        #[serde(default = "default_true")]
        include_media: bool,
        /// Strip every device and plugin chain from the packed document (the audio clips
        /// stay; the project's own copy is untouched).
        #[serde(default)]
        #[ts(optional)]
        hide_devices: Option<bool>,
    },
    /// Unpack a `.ether` bundle (from `ExportBundle`) as a new stored project `new_id` (not
    /// opened). `name`: the new project's name (`None` = the name inside the bundle). A bare
    /// `project.ether` document (no archive) is accepted too. Replies `Saved`.
    ImportBundle {
        new_id: ProjectId,
        source: BundleSource,
        name: Option<String>,
    },
    /// Import a project of another DAW (DAWproject, Ableton `.als`, FL Studio `.flp`,
    /// REAPER `.rpp`; detected from the content) as the new stored project `new_id`, then
    /// open it (the current project is autosaved first). Replies `Unit` at once; progress
    /// and the result arrive as `Event::Import` (`crate::import`). `name`: the new
    /// project's name (`None` = the source's name, else "Imported project"). Desktop:
    /// `source: Path` (the OS open dialog or a drop); its audio referenced by absolute or
    /// project-relative paths is referenced in place where it exists. Web and remote:
    /// `source: Upload` (audio outside the uploaded file is reported missing).
    Import {
        new_id: ProjectId,
        source: BundleSource,
        name: Option<String>,
    },
    /// Cancel the running import `id` (nothing is created). Replies `Unit`.
    CancelImport { id: ProjectId },
    /// Convert a stored project (the open one: its current state) into a
    /// project of another DAW (`crate::project_export`), its audio files included. Replies
    /// `ProjectExported`. `path`: an absolute engine-machine path from the desktop's OS save
    /// dialog (must end with the format's extension): the host writes the project there and
    /// its audio in `Samples/` next to it (DAWproject: inside the file); hosts without OS
    /// files reply `Unsupported`. `None`: the result is a download (`Export::ReadChunk`).
    ExportTo {
        id: ProjectId,
        format: crate::project_export::ProjectExportFormat,
        path: Option<String>,
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
    /// What an extension knows of the stored project (e.g. a badge on the project list),
    /// as its own JSON (`crate::ext`). Omitted when it has nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "unknown")]
    pub ext: Option<crate::ext::ExtValue>,
    /// The project it was forked from (its document's `settings.forked_from`). Omitted for
    /// an original.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub forked_from: Option<crate::model::ForkOrigin>,
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
    /// Safe mode of the open project (`OpenSafe`: `active`, with the plugin
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
    /// Takes back what the open gesture `gesture` applied and forgets its undo step: a
    /// drag cancelled with Escape leaves no step behind. Closes the gesture. No-op
    /// when `gesture` isn't the open gesture (nothing applied, or already closed).
    CancelGesture {
        gesture: GestureId,
    },
    /// Apply several commands as ONE undo step (all-or-nothing). Only document-mutating
    /// commands are allowed inside (no transport, queries or nested batches).
    Batch {
        /// The undo label in English (agents, logs, the fallback).
        label: String,
        commands: Vec<Command>,
        /// The label as a message of the UI's catalogs, shown in the user's language
        /// (docs/LOCALIZATION.md). Without it, a `label` that is one of the catalog's
        /// messages without arguments is keyed by the engine.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        key: Option<crate::model::text::TextKey>,
    },
}

fn default_true() -> bool {
    true
}

//! Project versions and crash recovery (v0.3, `project-versions`; CONTRACTS.md §13.11).
//!
//! The controller keeps rolling snapshots of the open project in its store folder
//! (`<project>/versions/<id>.ether`, through `ProjectStore::{write, read, list_dir,
//! remove}`, so it works on disk natively and in OPFS on the web):
//! - **Autosave versions**: written by the autosave tick when the document changed since the
//!   last version, at most every [`VERSION_INTERVAL_MS`]; the newest [`MAX_AUTOSAVE_VERSIONS`]
//!   are kept.
//! - **Manual versions** (`Create`), kept until deleted.
//! - **Before-restore versions**: `Restore` first snapshots the current state, so a restore
//!   can itself be undone by restoring that version.
//! - **Crash recovery**: opening a project writes a session marker (`versions/.session`),
//!   a clean close (or opening another project) removes it. At startup the UI asks
//!   `ListRecoverable`: projects whose marker survived have unsaved work in their newest
//!   autosave version (newer than `project.ether`), offered by the recovery dialog.
//!
//! Version commands are not undoable (`Restore` replaces the document like opening it).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::ProjectId;

/// `"<created ms>-<kind>"`, unique within a project.
pub type VersionId = String;

/// Minimum time between two autosave versions.
pub const VERSION_INTERVAL_MS: u64 = 5 * 60_000;
/// Autosave versions kept per project (oldest dropped first).
pub const MAX_AUTOSAVE_VERSIONS: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub enum VersionKind {
    Autosave,
    Manual,
    BeforeRestore,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct VersionInfo {
    pub id: VersionId,
    pub kind: VersionKind,
    pub name: Option<String>,
    #[ts(type = "number")]
    pub created_ms: u64,
    /// Size of the stored document in bytes.
    #[ts(type = "number")]
    pub size: u64,
}

/// What changed between two versions (summary for the compare view).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct VersionDiff {
    /// One row per entity table with changes (`"tracks"`, `"clips"`, ...).
    pub tables: Vec<TableDiff>,
    /// Project settings (tempo map is in its tables) differ.
    pub settings_changed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct TableDiff {
    pub table: String,
    pub added: u32,
    pub removed: u32,
    pub changed: u32,
    /// Display names of added/removed/changed tracks and clips (others: counts only), at most
    /// 50.
    pub names: Vec<String>,
}

/// A project with recoverable unsaved work.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct RecoveryInfo {
    pub project: ProjectId,
    pub name: String,
    /// The newest version (what `Recover` opens).
    pub version: VersionInfo,
    /// When `project.ether` was last saved.
    #[ts(type = "number")]
    pub saved_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum VersionCommand {
    /// Versions of the open project, newest first. Replies `Versions`.
    List,
    /// Snapshot the open project now. Replies `Version`.
    Create {
        name: Option<String>,
    },
    /// Replace the open document with `version` (after a `BeforeRestore` snapshot); emits
    /// `ProjectLoaded`, marks the project dirty. Replies `Unit`.
    Restore {
        version: VersionId,
    },
    Delete {
        version: VersionId,
    },
    Rename {
        version: VersionId,
        name: Option<String>,
    },
    /// Compare `version` with `against` (`None` = the open document). Replies `VersionDiff`.
    Compare {
        version: VersionId,
        against: Option<VersionId>,
    },
    /// Projects with recoverable work (startup dialog). Replies `Recoverable`.
    ListRecoverable,
    /// Open `project` at its newest version (dirty, so Save keeps it). Replies `Project`.
    Recover {
        project: ProjectId,
    },
    /// Keep the saved `project.ether`: remove the marker (versions stay).
    DiscardRecovery {
        project: ProjectId,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum VersionEvent {
    /// A version was created (autosave, manual, before restore) or deleted/renamed.
    Changed,
}

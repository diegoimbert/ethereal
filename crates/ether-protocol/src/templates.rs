//! Project and track templates (v0.3, `templates`; file format `ether_model::template`,
//! CONTRACTS.md §13.10). Templates live in the engine-side user library; the UI never
//! touches files.
//!
//! `Insert` is a document command (undoable, allowed in a `Batch`); `NewProject` behaves
//! like `Project::Create` (not undoable); the rest is library management (not undoable).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{PresetMeta, ProjectId, TrackId};

/// `"<kind>/<name>"`: `"projects/Band"`, `"tracks/Vocal chain"`, factory templates
/// `"factory/<slug>"`.
pub type TemplateId = String;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub enum TemplateKind {
    Project,
    Tracks,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct TemplateInfo {
    pub id: TemplateId,
    pub kind: TemplateKind,
    pub name: String,
    pub meta: PresetMeta,
    /// Read-only factory template.
    pub factory: bool,
    /// The project template "New project" uses.
    pub default: bool,
    #[ts(type = "number")]
    pub modified_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum TemplateCommand {
    /// Replies `Templates` (`kind: None` = both kinds).
    List {
        kind: Option<TemplateKind>,
    },
    /// Save the open project as a project template (`InvalidArgument` if the name exists
    /// and `overwrite` is false). Replies `Template`.
    SaveProject {
        name: String,
        meta: PresetMeta,
        overwrite: bool,
    },
    /// Save tracks (with their children when groups) as one track template. Replies
    /// `Template`.
    SaveTracks {
        tracks: Vec<TrackId>,
        name: String,
        meta: PresetMeta,
        overwrite: bool,
    },
    /// Insert a track template before `before` under `parent` (None = top level / end).
    /// Every new entity's id is `derive_id(seed, i)` (`ether_model::template`). Undoable.
    Insert {
        template: TemplateId,
        seed: TrackId,
        parent: Option<TrackId>,
        before: Option<TrackId>,
    },
    /// Create and open project `id` from a project template (`None` = the default template,
    /// or an empty project when there is none). Replies `Project` like `Project::Create`.
    NewProject {
        id: ProjectId,
        name: String,
        template: Option<TemplateId>,
    },
    Rename {
        template: TemplateId,
        name: String,
    },
    Delete {
        template: TemplateId,
    },
    /// The project template "New project" uses (`None` = empty project).
    SetDefault {
        template: Option<TemplateId>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum TemplateEvent {
    /// The template list changed (save, rename, delete, default).
    Changed,
}

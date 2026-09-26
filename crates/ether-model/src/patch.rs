//! Patches: how the UI mirror of the document is kept in sync.
//!
//! A patch is a list of whole-entity upserts/removals (plus settings), derived from the ops
//! that were applied. The UI applies them to its normalized store with no domain logic:
//! `Upsert` = `table[id] = entity`, `Remove` = `delete table[id]`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::entity::{Entity, EntityKey};
use crate::history::HistoryState;
use crate::op::Op;
use crate::project::{Project, ProjectSettings};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Patch {
    /// Monotonic document revision after this patch. The UI ignores patches with
    /// `revision <= current` and re-requests the project on a gap.
    #[ts(type = "number")]
    pub revision: u64,
    pub changes: Vec<PatchChange>,
    pub history: HistoryState,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum PatchChange {
    Upsert { entity: Entity },
    Remove { key: EntityKey },
    Settings { settings: ProjectSettings },
}

/// Compute the minimal patch changes for `applied` ops against the *post-apply* project
/// (coalescing several updates of one entity into one upsert).
pub fn changes_for(project: &Project, applied: &[Op]) -> Vec<PatchChange> {
    let _ = (project, applied);
    todo!("model node")
}

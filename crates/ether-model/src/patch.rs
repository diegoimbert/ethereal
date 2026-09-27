//! Patches: how the UI mirror of the document is kept in sync.
//!
//! A patch is a list of whole-entity upserts/removals (plus settings), derived from the ops
//! that were applied. The UI applies them to its normalized store with no domain logic:
//! `Upsert` = `table[id] = entity`, `Remove` = `delete table[id]`.

use std::collections::BTreeSet;

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
    /// RESERVED (roadmap v2, `collab`): who made the change when it came from another site
    /// (`None` = this site / not collaborative). Omitted from JSON when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub origin: Option<crate::collab::OpOrigin>,
}

// Entities are plain data sent once per change; boxing them would only add an allocation.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum PatchChange {
    Upsert { entity: Entity },
    Remove { key: EntityKey },
    Settings { settings: ProjectSettings },
}

/// Compute the minimal patch changes for `applied` ops against the *post-apply* project
/// (coalescing several updates of one entity into one upsert).
///
/// Each touched entity yields exactly one change, in first-touch order: `Upsert` with its
/// current value if it exists after the ops, else `Remove`. Settings yield one `Settings`
/// change (the whole settings object) at the position of the first settings op.
pub fn changes_for(project: &Project, applied: &[Op]) -> Vec<PatchChange> {
    let mut seen: BTreeSet<EntityKey> = BTreeSet::new();
    let mut settings_seen = false;
    let mut out = Vec::new();
    for op in applied {
        match op.key() {
            Some(key) => {
                if seen.insert(key) {
                    out.push(match project.get(key) {
                        Some(entity) => PatchChange::Upsert { entity },
                        None => PatchChange::Remove { key },
                    });
                }
            }
            None => {
                if !settings_seen {
                    settings_seen = true;
                    out.push(PatchChange::Settings {
                        settings: project.settings.clone(),
                    });
                }
            }
        }
    }
    out
}

/// A full-state patch: every entity upserted plus the settings (e.g. after `Project::Get`,
/// or to seed a mirror). Removals of stale entities are the receiver's job (replace the
/// whole mirror).
pub fn full_changes(project: &Project) -> Vec<PatchChange> {
    let mut out: Vec<PatchChange> = project
        .entities()
        .into_iter()
        .map(|entity| PatchChange::Upsert { entity })
        .collect();
    out.push(PatchChange::Settings {
        settings: project.settings.clone(),
    });
    out
}

impl Project {
    /// Apply patch changes the way the UI mirror does: no domain logic, no checks
    /// (`Upsert` = `table[id] = entity`, `Remove` = `delete table[id]`). For mirrors and tests.
    pub fn apply_patch_changes(&mut self, changes: &[PatchChange]) {
        for change in changes {
            match change {
                PatchChange::Upsert { entity } => self.upsert_unchecked(entity.clone()),
                PatchChange::Remove { key } => {
                    self.remove_unchecked(*key);
                }
                PatchChange::Settings { settings } => self.settings = settings.clone(),
            }
        }
    }
}

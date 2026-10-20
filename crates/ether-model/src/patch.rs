//! Patches: how the UI mirror of the document is kept in sync.
//!
//! A patch is a list of whole-entity upserts/removals (plus settings), derived from the ops
//! that were applied. The UI applies them to its normalized store with no domain logic:
//! `Upsert` = `table[id] = entity`, `Remove` = `delete table[id]`, and
//! the field-level `DeviceParam` = `devices[id].params[param] = value` for the
//! hottest edit (a knob drag), so it doesn't re-send the whole device.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::device::DeviceKind;
use crate::entity::{Entity, EntityKey};
use crate::history::HistoryState;
use crate::ids::DeviceId;
use crate::op::{DeviceChange, EntityUpdate, Op};
use crate::project::{Project, ProjectSettings};
use crate::value::ParamId;

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
    Upsert {
        entity: Entity,
    },
    Remove {
        key: EntityKey,
    },
    Settings {
        settings: ProjectSettings,
    },
    /// One parameter of an existing device: the receiver sets
    /// `devices[id].params[param] = value` (`None` = reset to default: remove the entry)
    /// and ignores it if it has no such device. Sent only to receivers that support it
    /// (`remote::FEATURE_DEVICE_PARAM_PATCH`); [`upsert_device_params`] turns it back into
    /// an `Upsert` for older ones.
    DeviceParam {
        id: DeviceId,
        param: ParamId,
        value: Option<f64>,
    },
}

/// Compute the minimal patch changes for `applied` ops against the *post-apply* project
/// (coalescing several updates of one entity into one upsert).
///
/// Each touched entity yields exactly one change, in first-touch order: `Upsert` with its
/// current value if it exists after the ops, else `Remove`. Settings yield one `Settings`
/// change (the whole settings object) at the position of the first settings op.
///
/// Exception: a device that exists after the ops and was touched *only* by
/// `DeviceChange::Param` updates yields one `DeviceParam` per distinct param instead (its
/// final value, in first-touch order), so a knob drag on a 2000-param plugin sends ~100
/// bytes rather than the whole device.
pub fn changes_for(project: &Project, applied: &[Op]) -> Vec<PatchChange> {
    // Devices touched only by param updates (decided over the whole batch first).
    let mut param_only: BTreeMap<DeviceId, bool> = BTreeMap::new();
    for op in applied {
        if let Some(EntityKey::Device(id)) = op.key() {
            let is_param = matches!(
                op,
                Op::Update {
                    update: EntityUpdate::Device {
                        change: DeviceChange::Param { .. },
                        ..
                    }
                }
            );
            *param_only.entry(id).or_insert(true) &= is_param;
        }
    }
    let mut seen: BTreeSet<EntityKey> = BTreeSet::new();
    let mut params_seen: BTreeSet<(DeviceId, ParamId)> = BTreeSet::new();
    let mut settings_seen = false;
    let mut out = Vec::new();
    for op in applied {
        match op.key() {
            Some(EntityKey::Device(id))
                if param_only.get(&id) == Some(&true) && project.devices.contains_key(&id) =>
            {
                if let Op::Update {
                    update:
                        EntityUpdate::Device {
                            change: DeviceChange::Param { param, .. },
                            ..
                        },
                } = op
                    && params_seen.insert((id, *param))
                {
                    out.push(PatchChange::DeviceParam {
                        id,
                        param: *param,
                        value: project.devices[&id].params.get(param).copied(),
                    });
                }
            }
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

/// Drop every plugin state blob (`PluginInstance.state`) from the entities in `changes`.
/// Patches are a mirror for the UI, which never reads the state; a blob is up
/// to megabytes and would otherwise be re-sent with every edit of its device. Stripped
/// states arrive as `null`: receivers must not treat that as "fresh instance".
pub fn strip_plugin_states(changes: &mut [PatchChange]) {
    for change in changes {
        if let PatchChange::Upsert {
            entity: Entity::Device(device),
        } = change
            && let DeviceKind::Plugin { plugin } = &mut device.kind
        {
            plugin.state = None;
        }
    }
}

/// [`strip_plugin_states`] for a whole project sent to a UI (`ProjectLoaded`, the
/// `Project` reply): the state blobs (megabytes in plugin-heavy projects) stay engine-side.
pub fn without_plugin_states(mut project: Project) -> Project {
    for device in project.devices.values_mut() {
        if let DeviceKind::Plugin { plugin } = &mut device.kind {
            plugin.state = None;
        }
    }
    project
}

/// Replace every `DeviceParam` in `changes` by an `Upsert` of its device's current value in
/// `project` (one per device, at its first position), for receivers that predate
/// `DeviceParam`. Plugin states are stripped like [`strip_plugin_states`].
pub fn upsert_device_params(project: &Project, changes: &mut Vec<PatchChange>) {
    if !changes
        .iter()
        .any(|c| matches!(c, PatchChange::DeviceParam { .. }))
    {
        return;
    }
    let mut seen: BTreeSet<DeviceId> = BTreeSet::new();
    let mut out = Vec::with_capacity(changes.len());
    for change in changes.drain(..) {
        match change {
            PatchChange::DeviceParam { id, .. } => {
                if seen.insert(id)
                    && let Some(device) = project.devices.get(&id)
                {
                    out.push(PatchChange::Upsert {
                        entity: Entity::Device(device.clone()),
                    });
                }
            }
            other => out.push(other),
        }
    }
    strip_plugin_states(&mut out);
    *changes = out;
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
                PatchChange::DeviceParam { id, param, value } => {
                    if let Some(device) = self.devices.get_mut(id) {
                        match value {
                            Some(v) => {
                                device.params.insert(*param, *v);
                            }
                            None => {
                                device.params.remove(param);
                            }
                        }
                    }
                }
            }
        }
    }
}

//! Controller-side plugin logic (owned by the `plugins` wave-3 node; see `docs/WAVE3.md`).
//!
//! - Inserting a plugin (`DeviceCommand::Insert { device: DeviceSpec::Plugin }`), the
//!   sandbox toggle (`PluginCommand::SetSandboxed`: the live state is captured into
//!   `PluginInstance.state`, the changed node signature re-instantiates it) and `Reload`
//!   are handled with the other commands in `handlers.rs`/`doc/devices.rs`.
//! - Saving reads every plugin's live state into `PluginInstance.state`
//!   (`EngineBridge::plugin_state`, `project.rs`); loading instantiates plugins with it
//!   (the state blob is authoritative).
//! - **Param mirroring** (this module): whenever a plugin device gets a new engine node
//!   (project load, insert, sandbox toggle, reload), its current param values are read back
//!   (`EngineBridge::plugin_param_values`) and mirrored into `Device.params`, so the UI
//!   and automation see what the restored state actually contains. The mirror is not an
//!   edit: no undo step, no dirty flag, just a patch.
//! - **Modulated params** (v0.2, `racks-modulation`): the engine sends a modulated param's
//!   effective value to the plugin while the document keeps the base, so modulated params
//!   are never mirrored back and their `ParamEdited` echoes are ignored ([`is_modulated`]).

use std::collections::BTreeMap;

use ether_core::NodeKey;
use ether_core::protocol::model::{DeviceId, DeviceKind, Entity, Patch, PatchChange, Project};
use ether_core::protocol::{Event, ServerMessage};

use crate::store::{Library, ProjectStore};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// Values closer than this are considered equal (no mirror update).
const PARAM_EPSILON: f64 = 1e-9;

/// Plugin runtime bookkeeping held by the controller.
#[derive(Debug, Default)]
pub(crate) struct PluginsState {
    /// Engine node whose params were last mirrored into the document, per plugin device.
    mirrored: BTreeMap<DeviceId, NodeKey>,
}

/// Whether a modulation mapping targets `device`'s `param` (its live value is the
/// modulated one, not the document's base).
pub(crate) fn is_modulated(
    project: &Project,
    device: DeviceId,
    param: ether_core::protocol::model::ParamId,
) -> bool {
    project
        .mod_mappings
        .values()
        .any(|m| m.device == device && m.param == param)
}

/// Mirror `values` into `project`'s device `device`. Returns the updated device when
/// anything changed. Modulated params keep their document base.
fn mirror_params(
    project: &mut Project,
    device: DeviceId,
    values: &[(ether_core::protocol::model::ParamId, f64)],
) -> Option<Entity> {
    let modulated: Vec<ether_core::protocol::model::ParamId> = values
        .iter()
        .map(|(p, _)| *p)
        .filter(|p| is_modulated(project, device, *p))
        .collect();
    let d = project.devices.get_mut(&device)?;
    let mut changed = false;
    for (param, value) in values {
        if !value.is_finite() || modulated.contains(param) {
            continue;
        }
        let same = d
            .params
            .get(param)
            .is_some_and(|v| (v - value).abs() <= PARAM_EPSILON);
        if !same {
            d.params.insert(*param, *value);
            changed = true;
        }
    }
    changed.then(|| Entity::Device(d.clone()))
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// Tick work for plugins: mirror the params of freshly instantiated plugin nodes.
    pub(crate) fn plugins_tick(&mut self, _now: u64, out: &mut dyn MessageSink) {
        let Some(doc) = self.doc.as_mut() else {
            self.plugins.mirrored.clear();
            return;
        };
        let devices = &doc.project.devices;
        self.plugins.mirrored.retain(|d, _| {
            devices
                .get(d)
                .is_some_and(|d| matches!(d.kind, DeviceKind::Plugin { .. }))
        });
        let fresh: Vec<(DeviceId, NodeKey)> = devices
            .values()
            .filter(|d| matches!(d.kind, DeviceKind::Plugin { .. }))
            .filter_map(|d| {
                let key = self.engine.node(d.id)?;
                (self.plugins.mirrored.get(&d.id) != Some(&key)).then_some((d.id, key))
            })
            .collect();
        if fresh.is_empty() {
            return;
        }
        let mut changes = Vec::new();
        for (device, key) in fresh {
            let values = self.bridge.plugin_param_values(device);
            self.plugins.mirrored.insert(device, key);
            if let Some(entity) = mirror_params(&mut doc.project, device, &values) {
                changes.push(PatchChange::Upsert { entity });
            }
        }
        if changes.is_empty() {
            return;
        }
        self.revision += 1;
        out.send(ServerMessage::Event(Event::Patch {
            patch: Patch {
                revision: self.revision,
                changes,
                history: doc.history.state(),
                origin: None,
            },
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ether_core::protocol::model::{
        Device, OrderKey, ParamId, PluginFormat, PluginInstance, TrackId, Ulid,
    };

    fn plugin_device(id: DeviceId) -> Device {
        Device {
            chain: None,
            id,
            track: TrackId(Ulid(1)),
            order: OrderKey::between(None, None),
            name: "P".into(),
            enabled: true,
            kind: DeviceKind::Plugin {
                plugin: PluginInstance {
                    format: PluginFormat::Clap,
                    plugin_id: "p".into(),
                    name: "P".into(),
                    vendor: String::new(),
                    version: String::new(),
                    sandboxed: false,
                    state: None,
                },
            },
            params: BTreeMap::from([(ParamId(1), 0.5)]),
            sidechain: None,
            pad: None,
        }
    }

    #[test]
    fn mirror_only_reports_real_changes() {
        let mut p = Project::new(&mut ether_core::protocol::model::IdGen::new(1), 0);
        let id = DeviceId(Ulid(7));
        p.devices.insert(id, plugin_device(id));
        assert!(mirror_params(&mut p, id, &[(ParamId(1), 0.5)]).is_none());
        assert!(mirror_params(&mut p, id, &[(ParamId(1), f64::NAN)]).is_none());
        let Some(Entity::Device(d)) =
            mirror_params(&mut p, id, &[(ParamId(1), 0.25), (ParamId(2), 3.0)])
        else {
            panic!("expected a device upsert");
        };
        assert_eq!(
            d.params,
            BTreeMap::from([(ParamId(1), 0.25), (ParamId(2), 3.0)])
        );
        assert_eq!(p.devices[&id].params, d.params);
        assert!(mirror_params(&mut p, DeviceId(Ulid(8)), &[(ParamId(1), 1.0)]).is_none());
    }
}

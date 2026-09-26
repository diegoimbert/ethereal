//! Keeping the engine in sync with the document: device nodes, graph publishing (coalesced)
//! and live parameter pushes.
//!
//! - Every device with a usable kind gets one engine node, created through the bridge with
//!   its document params (plugins with their state blob). A node is re-created when its
//!   "signature" changes (sampler sample, plugin id/sandbox flag) or on request (plugin
//!   reload/restart), and destroyed once a graph that no longer references it has been
//!   published (`EngineHandle::remove_node` contract).
//! - Structural document changes mark the graph dirty; the controller publishes at most
//!   every `publish_interval_ms` from `handle` and always from `tick` (bursts coalesce).
//! - Continuous controls (volume, pan, mute, send level, device params) are pushed to the
//!   engine param queue immediately and do not republish.

use std::collections::{BTreeMap, BTreeSet};

use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::*;
use ether_core::protocol::{ErrorCode, NotificationLevel};
use ether_core::{NodeKey, ParamChange, ParamTarget};

use crate::compile::{CompileContext, compile_graph_with};
use crate::doc::{DocHost, builtin_descriptor};
use crate::tx::{CmdResult, cmd_err, unsupported};
use crate::{BridgeError, EngineBridge};

/// What a device's node was built from; a change means the node must be re-created.
#[derive(Clone, Debug, PartialEq)]
enum NodeSig {
    Builtin(BuiltinDevice),
    Plugin { plugin_id: String, sandboxed: bool },
}

fn sig_of(kind: &DeviceKind) -> NodeSig {
    match kind {
        DeviceKind::Builtin { device } => NodeSig::Builtin(device.clone()),
        DeviceKind::Plugin { plugin } => NodeSig::Plugin {
            plugin_id: plugin.plugin_id.clone(),
            sandboxed: plugin.sandboxed,
        },
    }
}

#[derive(Debug)]
struct NodeEntry {
    key: NodeKey,
    sig: NodeSig,
}

#[derive(Debug, Default)]
pub(crate) struct EngineState {
    nodes: BTreeMap<DeviceId, NodeEntry>,
    /// Devices whose node creation failed for this signature (not retried until changed).
    failed: BTreeMap<DeviceId, NodeSig>,
    /// Descriptors of instantiated plugins.
    plugin_descriptors: BTreeMap<DeviceId, DeviceDescriptor>,
    /// Nodes to destroy after the next successful publish.
    pending_destroy: Vec<NodeKey>,
    /// Devices whose node must be re-created at the next sync.
    recreate: BTreeSet<DeviceId>,
    pub graph_dirty: bool,
    pub version: u64,
    pub last_publish_ms: Option<u64>,
    /// The edit being applied came from this device's own GUI: don't echo its params back.
    pub echo_from: Option<DeviceId>,
}

pub(crate) fn bridge_err(e: BridgeError) -> ether_core::protocol::CommandError {
    match e {
        BridgeError::Unsupported(m) => unsupported(m),
        other => cmd_err(ErrorCode::Plugin, other.to_string()),
    }
}

impl EngineState {
    pub fn node(&self, device: DeviceId) -> Option<NodeKey> {
        self.nodes.get(&device).map(|n| n.key)
    }

    /// Nodes of devices that are not in `project` (to destroy at the next publish).
    pub fn has_orphans(&self, project: &Project) -> bool {
        self.nodes.keys().any(|d| !project.devices.contains_key(d))
    }

    pub fn request_recreate(&mut self, device: DeviceId) {
        self.recreate.insert(device);
        self.failed.remove(&device);
        self.graph_dirty = true;
    }

    /// Forget every node (project switch): all are destroyed after the next publish.
    pub fn reset(&mut self) {
        for (_, n) in std::mem::take(&mut self.nodes) {
            self.pending_destroy.push(n.key);
        }
        self.failed.clear();
        self.plugin_descriptors.clear();
        self.recreate.clear();
        self.graph_dirty = true;
    }

    pub fn set_plugin_descriptor(&mut self, device: DeviceId, desc: Option<DeviceDescriptor>) {
        match desc {
            Some(d) => {
                self.plugin_descriptors.insert(device, d);
            }
            None => {
                self.plugin_descriptors.remove(&device);
            }
        }
    }

    pub fn descriptor(&self, device: &Device) -> Option<DeviceDescriptor> {
        match &device.kind {
            DeviceKind::Builtin { device } => Some(builtin_descriptor(device)),
            DeviceKind::Plugin { .. } => self.plugin_descriptors.get(&device.id).cloned(),
        }
    }

    fn create<B: EngineBridge>(
        bridge: &mut B,
        device: &Device,
        live_state: Option<Base64Bytes>,
    ) -> Result<NodeKey, BridgeError> {
        match &device.kind {
            DeviceKind::Builtin { device: kind } => {
                let params: Vec<(ParamId, f64)> =
                    device.params.iter().map(|(k, v)| (*k, *v)).collect();
                bridge.create_builtin(device.id, kind, &params)
            }
            DeviceKind::Plugin { plugin } => {
                let state = live_state.or_else(|| plugin.state.clone());
                bridge.create_plugin(device.id, plugin, state.as_ref())
            }
        }
    }

    /// Create/re-create/forget nodes so they match the document's devices. Returns error
    /// messages for devices that could not be instantiated.
    pub fn sync_nodes<B: EngineBridge>(
        &mut self,
        bridge: &mut B,
        project: Option<&Project>,
    ) -> Vec<String> {
        let mut errors = Vec::new();
        let empty = BTreeMap::new();
        let devices = project.map_or(&empty, |p| &p.devices);
        // Forget nodes of devices that are gone.
        let gone: Vec<DeviceId> = self
            .nodes
            .keys()
            .filter(|d| !devices.contains_key(d))
            .copied()
            .collect();
        for d in gone {
            if let Some(n) = self.nodes.remove(&d) {
                self.pending_destroy.push(n.key);
            }
            self.plugin_descriptors.remove(&d);
        }
        self.failed.retain(|d, _| devices.contains_key(d));
        self.recreate.retain(|d| devices.contains_key(d));
        for device in devices.values() {
            let sig = sig_of(&device.kind);
            let recreate = self.recreate.remove(&device.id);
            let mut live_state = None;
            match self.nodes.get(&device.id) {
                Some(n) if n.sig == sig && !recreate => continue,
                Some(_) => {
                    if matches!(device.kind, DeviceKind::Plugin { .. }) {
                        live_state = bridge.plugin_state(device.id).ok().flatten();
                    }
                    let old = self.nodes.remove(&device.id).expect("checked");
                    self.pending_destroy.push(old.key);
                }
                None => {
                    if !recreate && self.failed.get(&device.id) == Some(&sig) {
                        continue;
                    }
                }
            }
            match Self::create(bridge, device, live_state) {
                Ok(key) => {
                    self.failed.remove(&device.id);
                    if matches!(device.kind, DeviceKind::Plugin { .. }) {
                        let desc = bridge.descriptor(device.id);
                        self.set_plugin_descriptor(device.id, desc);
                    }
                    self.nodes.insert(device.id, NodeEntry { key, sig });
                }
                Err(e) => {
                    errors.push(format!("could not create device \"{}\": {e}", device.name));
                    self.failed.insert(device.id, sig);
                }
            }
        }
        errors
    }

    /// Sync nodes, compile and publish; destroy retired nodes afterwards. Returns
    /// user-facing problems.
    pub fn publish<B: EngineBridge>(
        &mut self,
        bridge: &mut B,
        project: Option<&Project>,
        armed: &BTreeSet<TrackId>,
        now_ms: u64,
    ) -> Vec<(NotificationLevel, String)> {
        let mut problems: Vec<(NotificationLevel, String)> = self
            .sync_nodes(bridge, project)
            .into_iter()
            .map(|m| (NotificationLevel::Error, m))
            .collect();
        self.version += 1;
        let desc = match project {
            Some(p) => {
                let nodes = |d: DeviceId| self.node(d);
                let descriptors = |d: &Device| self.descriptor(d);
                let is_armed = |t: TrackId| armed.contains(&t);
                compile_graph_with(
                    p,
                    &CompileContext {
                        nodes: &nodes,
                        descriptors: &descriptors,
                        armed: &is_armed,
                        version: self.version,
                    },
                )
            }
            None => ether_core::RenderGraphDesc {
                version: self.version,
                ..Default::default()
            },
        };
        self.last_publish_ms = Some(now_ms);
        match bridge.publish(desc) {
            Ok(()) => {
                self.graph_dirty = false;
                for key in std::mem::take(&mut self.pending_destroy) {
                    let _ = bridge.destroy_node(key);
                }
            }
            Err(e) => {
                // Keep the graph dirty only for transient failures (retried next tick).
                self.graph_dirty = matches!(e, BridgeError::QueueFull);
                problems.push((
                    NotificationLevel::Error,
                    format!("engine rejected the graph: {e}"),
                ));
            }
        }
        problems
    }

    /// Engine effects of applied ops: continuous controls go to the param queue, anything
    /// else marks the graph dirty.
    pub fn apply_effects<B: EngineBridge>(
        &mut self,
        bridge: &mut B,
        project: &Project,
        applied: &[Op],
    ) {
        for op in applied {
            let change = match op {
                Op::Update {
                    update: EntityUpdate::Track { id, change },
                } => match change {
                    TrackChange::Volume(v) => Some(ParamChange {
                        target: ParamTarget::TrackVolume { track: *id },
                        value: v.to_linear() as f64,
                    }),
                    TrackChange::Pan(p) => Some(ParamChange {
                        target: ParamTarget::TrackPan { track: *id },
                        value: p.0 as f64,
                    }),
                    TrackChange::Mute(m) => Some(ParamChange {
                        target: ParamTarget::TrackMute { track: *id },
                        value: if *m { 1.0 } else { 0.0 },
                    }),
                    _ => None,
                },
                Op::Update {
                    update:
                        EntityUpdate::Send {
                            id,
                            change: SendChange::Level(l),
                        },
                } => Some(ParamChange {
                    target: ParamTarget::SendLevel { send: *id },
                    value: l.to_linear() as f64,
                }),
                Op::Update {
                    update:
                        EntityUpdate::Device {
                            id,
                            change: DeviceChange::Param { param, .. },
                        },
                } => {
                    // Post-apply value (a reset removes the entry: use the default).
                    let Some(d) = project.devices.get(id) else {
                        continue;
                    };
                    if self.echo_from == Some(*id) {
                        continue;
                    }
                    let value = d.params.get(param).copied().or_else(|| {
                        self.descriptor(d).and_then(|desc| {
                            desc.params
                                .iter()
                                .find(|p| p.id == *param)
                                .map(|p| p.default)
                        })
                    });
                    match (self.node(*id), value) {
                        (Some(node), Some(value)) => Some(ParamChange {
                            target: ParamTarget::Node {
                                node,
                                param: *param,
                            },
                            value,
                        }),
                        _ => continue,
                    }
                }
                _ => None,
            };
            match change {
                Some(c) => {
                    if bridge.set_param(c).is_err() {
                        // Queue full: the next publish carries the document value.
                        self.graph_dirty = true;
                    }
                }
                None => self.graph_dirty = true,
            }
        }
    }
}

/// [`DocHost`] over the bridge + node table.
pub(crate) struct EngineCtx<'a, B> {
    pub bridge: &'a mut B,
    pub eng: &'a mut EngineState,
}

impl<B: EngineBridge> DocHost for EngineCtx<'_, B> {
    fn descriptor(&mut self, device: DeviceId, kind: &DeviceKind) -> Option<DeviceDescriptor> {
        match kind {
            DeviceKind::Builtin { device } => Some(builtin_descriptor(device)),
            DeviceKind::Plugin { .. } => {
                if let Some(d) = self.eng.plugin_descriptors.get(&device) {
                    return Some(d.clone());
                }
                let d = self.bridge.descriptor(device);
                if d.is_some() {
                    self.eng.set_plugin_descriptor(device, d.clone());
                }
                d
            }
        }
    }

    fn plugin_state(&mut self, device: DeviceId) -> Option<Base64Bytes> {
        self.eng.node(device)?;
        self.bridge.plugin_state(device).ok().flatten()
    }

    fn instantiate_plugin(
        &mut self,
        device: DeviceId,
        plugin: &PluginInstance,
    ) -> CmdResult<Option<DeviceDescriptor>> {
        let key = self
            .bridge
            .create_plugin(device, plugin, None)
            .map_err(bridge_err)?;
        if let Some(old) = self.eng.nodes.insert(
            device,
            NodeEntry {
                key,
                sig: sig_of(&DeviceKind::Plugin {
                    plugin: plugin.clone(),
                }),
            },
        ) {
            self.eng.pending_destroy.push(old.key);
        }
        let desc = self.bridge.descriptor(device);
        self.eng.set_plugin_descriptor(device, desc.clone());
        self.eng.graph_dirty = true;
        Ok(desc)
    }
}

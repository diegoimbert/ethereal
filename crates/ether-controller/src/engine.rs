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
//! - PDC is computed at publish from the node latencies the audio thread last observed.
//!   Each publish records them; the tick compares them with the live values
//!   ([`EngineState::check_latencies`]) and republishes, at most every
//!   [`LATENCY_REPUBLISH_MS`], when a node's latency changed (a built-in's lookahead param).

use std::collections::{BTreeMap, BTreeSet};

use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor, DeviceTypeRef};
use ether_core::protocol::model::*;
use ether_core::protocol::{ErrorCode, NotificationLevel};
use ether_core::{NodeKey, ParamChange, ParamTarget};

use crate::compile::{CompileContext, compile_graph_with};
use crate::doc::{DocHost, builtin_descriptor, missing_descriptor};
use crate::tx::{CmdResult, cmd_err, unsupported};
use crate::{BridgeError, EngineBridge};

/// What a device's node was built from; a change means the node must be re-created.
#[derive(Clone, Debug, PartialEq)]
enum NodeSig {
    Builtin(BuiltinDevice),
    Plugin {
        plugin_id: String,
        sandboxed: bool,
    },
    /// A missing-device placeholder: never a node.
    Missing,
}

fn sig_of(kind: &DeviceKind) -> NodeSig {
    match kind {
        // A rack's macro names and visible count are document data only: same node.
        DeviceKind::Builtin { device } => NodeSig::Builtin(
            device
                .clone()
                .with_rack_macros(ether_model::RackMacros::default()),
        ),
        DeviceKind::Plugin { plugin } => NodeSig::Plugin {
            plugin_id: plugin.plugin_id.clone(),
            sandboxed: plugin.sandboxed,
        },
        DeviceKind::Missing { .. } => NodeSig::Missing,
    }
}

#[derive(Debug)]
struct NodeEntry {
    key: NodeKey,
    sig: NodeSig,
    /// The instance's "always process" switch as last sent (`EngineBridge::create_plugin`,
    /// then `EngineBridge::set_always_process`): it changes on the live node.
    always: bool,
}

/// `PluginInstance::always_process` of a plugin device (false for anything else).
fn always_of(kind: &DeviceKind) -> bool {
    matches!(kind, DeviceKind::Plugin { plugin } if plugin.always_process)
}

/// How long the engine's queue may stay full (every publish retried) before it is said.
const QUEUE_FULL_NOTICE_MS: u64 = 5_000;

#[derive(Debug, Default)]
pub struct EngineState {
    nodes: BTreeMap<DeviceId, NodeEntry>,
    /// Devices whose node creation failed for this signature (not retried until changed).
    failed: BTreeMap<DeviceId, NodeSig>,
    /// Descriptors of instantiated plugins.
    plugin_descriptors: BTreeMap<DeviceId, DeviceDescriptor>,
    /// Nodes to destroy after the next successful publish.
    pending_destroy: Vec<NodeKey>,
    /// Devices whose node must be re-created at the next sync.
    recreate: BTreeSet<DeviceId>,
    /// Plugins to re-create from the document's `plugin.state` rather than their live state
    /// (collab: a peer replicated a new state).
    reload_from_doc: BTreeSet<DeviceId>,
    /// Safe mode: plugin devices held as bypassed placeholders (no node, so they
    /// are skipped from chains and their document state is what gets saved).
    deferred: BTreeSet<DeviceId>,
    /// Safe mode is on (the project was opened with `OpenSafe`, plugins not loaded yet).
    safe_mode: bool,
    /// The plugin devices the last sync left to instantiate (its time budget ran out): the
    /// graph stays dirty so the next tick continues.
    plugins_pending: Vec<DeviceId>,
    /// `plugins_pending` as last reported to the UI (`take_loading_change`).
    plugins_reported: Vec<DeviceId>,
    pub graph_dirty: bool,
    /// The engine refused the last published graph for lack of capacity (the message for
    /// the user); cleared by the next accepted publish.
    pub rejected: Option<crate::text::Msg>,
    pub version: u64,
    pub last_publish_ms: Option<u64>,
    /// The edit being applied came from this device's own GUI: don't echo its params back.
    pub echo_from: Option<DeviceId>,
    /// Record count-in (`tempo-metronome`): the record start while a record session's
    /// pre-roll may run, set on every published `MetronomeDesc::count_in_end`.
    pub count_in_end: Option<f64>,
    /// Soloed drum pads (runtime, `DrumRack::SetPadSolo`): compiled as mute of the other
    /// pads of their rack (`crate::drum_rack::apply_solo`).
    pub pad_solo: BTreeSet<DrumPadId>,
    /// Auditioned take lanes (runtime, `Take::Audition`): the track plays the lane instead
    /// of its comp (`crate::comping::apply_audition`).
    pub audition: BTreeMap<TrackId, TakeLaneId>,
    /// v0.2 (`midi-fx`): scale last pushed to each Scale Quantize / Random node.
    midi_fx_scales: BTreeMap<DeviceId, MusicalScale>,
    /// Node latencies as read just before the last publish. Reused
    /// across publishes; compared with the live values by [`Self::check_latencies`].
    published_latency: Vec<(NodeKey, Option<u32>)>,
    /// Plugin devices instantiated since the last [`Self::take_created_plugins`] (the save
    /// path's state cache must read them again: `crate::saving`).
    created_plugins: Vec<DeviceId>,
    /// "No engine": the document is open but no node is created and no
    /// graph is compiled or published.
    pub off: bool,
    /// The project the engine was turned off for (another project opens with the engine on).
    pub off_project: Option<ProjectId>,
    /// `off` as last told to the UI (`EngineEvent::Running`).
    pub off_reported: bool,
    /// A joined project that will load with the engine off (`AcceptInvite { engine }`).
    pub off_next: Option<ProjectId>,
    /// The message of the engine's last refusal of a graph (a project over its capacity),
    /// taken by the engine-choice switch.
    pub refused: Option<String>,
    /// Why the last switch to "my engine" failed (kept until the next switch).
    pub switch_error: Option<String>,
    /// Since when the engine's queue has been full for every publish (`None`: it took the
    /// last one), and whether that was said already.
    queue_full_since: Option<u64>,
    queue_full_told: bool,
    /// Plugins of an engine running elsewhere (an extension's, e.g. another computer's
    /// engine playing this project): empty in the app.
    pub remote: RemotePlugins,
}

/// The plugins of an engine that plays this project elsewhere, and the descriptors of plugin
/// devices as that engine instantiated them (filled by the extension that runs it). Used for
/// plugin devices this engine has no instance of (off, or the plugin is not installed here):
/// they keep their descriptor, can be inserted, and failing to load them here is no news.
/// Not cleared by [`EngineState::reset`] (the engine goes off when the remote one starts).
#[derive(Debug, Default)]
pub struct RemotePlugins {
    /// The remote engine's plugins by (format, id): their category and name.
    pub plugins: BTreeMap<(PluginFormat, String), (DeviceCategory, String)>,
    /// Descriptors of plugin devices as the remote engine instantiated them.
    pub descriptors: BTreeMap<DeviceId, DeviceDescriptor>,
}

impl RemotePlugins {
    /// A short descriptor for inserting `plugin` from the remote engine's list (its name and
    /// category; no params: that engine reports them once it instantiated it).
    pub fn insert_descriptor(&self, plugin: &PluginInstance) -> Option<DeviceDescriptor> {
        let (category, name) = self
            .plugins
            .get(&(plugin.format, plugin.plugin_id.clone()))?;
        let instrument = *category == DeviceCategory::Instrument;
        let audio = if *category == DeviceCategory::NoteEffect {
            0
        } else {
            2
        };
        Some(DeviceDescriptor {
            device_type: DeviceTypeRef::Plugin {
                plugin_id: plugin.plugin_id.clone(),
            },
            name: name.clone(),
            category: *category,
            params: Vec::new(),
            audio_inputs: if instrument { 0 } else { audio },
            audio_outputs: audio,
            midi_input: *category != DeviceCategory::AudioEffect,
            sidechain_inputs: 0,
            layout: None,
            implemented: true,
        })
    }
}

/// Devices built with media that just became available to the engine (samplers, multisamplers,
/// FX-space impulse responses): they are re-created to pick it up.
pub fn samplers_using(project: &Project, loaded: &[MediaId]) -> Vec<DeviceId> {
    project
        .devices
        .values()
        .filter(|dev| {
            matches!(&dev.kind, DeviceKind::Builtin {
                device: BuiltinDevice::Sampler { sample: Some(m), .. }
            } if loaded.contains(m))
                || crate::multisampler::uses_media(&dev.kind, loaded)
                || crate::fx_space::uses_media(&dev.kind, loaded)
        })
        .map(|dev| dev.id)
        .collect()
}

/// Minimum time between a publish and a republish caused by a node latency change (a
/// lookahead knob being dragged republishes at most this often).
pub const LATENCY_REPUBLISH_MS: u64 = 50;

/// One sync instantiates plugins for at most about this long, then publishes what is ready
/// and continues at the next tick: a project with a hundred plugins loads progressively
/// (the engine keeps ticking, the UI keeps working) instead of freezing until all are up.
/// One plugin always loads, however long it takes.
pub const PLUGIN_LOAD_BUDGET_MS: u64 = 100;

pub fn bridge_err(e: BridgeError) -> ether_core::protocol::CommandError {
    match e {
        BridgeError::Unsupported(m) => unsupported(m),
        other => cmd_err(ErrorCode::Plugin, other.to_string()),
    }
}

impl EngineState {
    pub fn node(&self, device: DeviceId) -> Option<NodeKey> {
        self.nodes.get(&device).map(|n| n.key)
    }

    /// The device of a live node (v0.2 analysis channel).
    pub fn device_of(&self, node: NodeKey) -> Option<DeviceId> {
        self.nodes
            .iter()
            .find(|(_, n)| n.key == node)
            .map(|(d, _)| *d)
    }

    /// Nodes of devices that are not in `project` (to destroy at the next publish).
    pub fn has_orphans(&self, project: &Project) -> bool {
        self.nodes.keys().any(|d| !project.devices.contains_key(d))
    }

    pub fn request_recreate(&mut self, device: DeviceId) {
        // Reloading a placeholder (safe mode) loads that plugin.
        self.deferred.remove(&device);
        self.recreate.insert(device);
        self.failed.remove(&device);
        self.graph_dirty = true;
    }

    /// Re-create `device` at the next sync from the document's plugin state (not the
    /// instance's live state).
    pub fn request_reload_from_doc(&mut self, device: DeviceId) {
        self.reload_from_doc.insert(device);
        self.request_recreate(device);
    }

    /// A new state for plugin `device` (an extension's, e.g. a collaborator's preset): loaded
    /// into the running instance in place when the host can (its editor stays open, no audio gap), else the instance is
    /// re-created from the document at the next sync. `Ok(true)`: loaded live; `Ok(false)`:
    /// re-created (or nothing runs); `Err`: the live load failed, re-created.
    pub fn apply_plugin_state<B: EngineBridge + ?Sized>(
        &mut self,
        bridge: &mut B,
        device: DeviceId,
        state: &Base64Bytes,
    ) -> Result<bool, BridgeError> {
        if self.node(device).is_none() {
            // No instance (missing plugin, frozen, failed or pending load): its next creation
            // reads the document anyway.
            return Ok(false);
        }
        match bridge.set_plugin_state(device, state) {
            Ok(true) => {
                // The save path must read it again.
                self.created_plugins.push(device);
                Ok(true)
            }
            Ok(false) => {
                self.request_reload_from_doc(device);
                Ok(false)
            }
            Err(e) => {
                self.request_reload_from_doc(device);
                Err(e)
            }
        }
    }

    /// Forget every node (project switch): all are destroyed after the next publish.
    pub fn reset(&mut self) {
        for (_, n) in std::mem::take(&mut self.nodes) {
            self.pending_destroy.push(n.key);
        }
        self.failed.clear();
        self.plugin_descriptors.clear();
        self.recreate.clear();
        self.reload_from_doc.clear();
        self.deferred.clear();
        self.safe_mode = false;
        self.pad_solo.clear();
        self.audition.clear();
        self.midi_fx_scales.clear();
        self.created_plugins.clear();
        self.graph_dirty = true;
    }

    /// The plugin devices still to load, when that changed since the last call (what
    /// `PluginEvent::Loading` reports; empty once all are loaded).
    pub fn take_loading_change(&mut self) -> Option<Vec<DeviceId>> {
        if self.plugins_pending == self.plugins_reported {
            return None;
        }
        self.plugins_reported = self.plugins_pending.clone();
        Some(self.plugins_reported.clone())
    }

    /// Plugin devices instantiated since the last call.
    pub fn take_created_plugins(&mut self) -> Vec<DeviceId> {
        std::mem::take(&mut self.created_plugins)
    }

    /// Safe mode: hold `project`'s plugin devices as placeholders. Call right
    /// after [`Self::reset`], before the first sync. Returns them.
    pub fn defer_plugins(&mut self, project: &Project) -> Vec<DeviceId> {
        self.safe_mode = true;
        self.deferred = project
            .devices
            .values()
            .filter(|d| matches!(d.kind, DeviceKind::Plugin { .. }))
            .map(|d| d.id)
            .collect();
        self.deferred.iter().copied().collect()
    }

    /// `(loaded, total)` plugin devices of `project` this engine instantiates: loaded when
    /// it has a node or failed to make one. Safe mode's placeholders and frozen devices
    /// are not counted (they get no node).
    pub fn plugin_progress(&self, project: &Project) -> (u32, u32) {
        let frozen = project.frozen_devices();
        let (mut loaded, mut total) = (0, 0);
        for d in project.devices.values() {
            if !matches!(d.kind, DeviceKind::Plugin { .. })
                || self.deferred.contains(&d.id)
                || frozen.contains(&d.id)
            {
                continue;
            }
            total += 1;
            if self.nodes.contains_key(&d.id) || self.failed.contains_key(&d.id) {
                loaded += 1;
            }
        }
        (loaded, total)
    }

    /// The last sync left plugins to instantiate (the next tick continues).
    pub fn plugins_pending(&self) -> bool {
        !self.plugins_pending.is_empty()
    }

    /// Plugin devices held as placeholders (safe mode).
    pub fn deferred(&self) -> Vec<DeviceId> {
        self.deferred.iter().copied().collect()
    }

    /// Safe mode is on.
    pub fn safe_mode(&self) -> bool {
        self.safe_mode
    }

    /// Leave safe mode: the placeholders are instantiated at the next sync. `false` when
    /// not in safe mode.
    pub fn load_deferred(&mut self) -> bool {
        if !self.safe_mode {
            return false;
        }
        self.safe_mode = false;
        self.deferred.clear();
        self.graph_dirty = true;
        true
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
            DeviceKind::Plugin { .. } => self
                .plugin_descriptors
                .get(&device.id)
                .or_else(|| self.remote.descriptors.get(&device.id))
                .cloned(),
            DeviceKind::Missing { missing } => Some(missing_descriptor(missing)),
        }
    }

    /// The descriptor of this engine's own instance of a plugin device (never a remote
    /// one).
    pub fn instance_descriptor(&self, device: DeviceId) -> Option<&DeviceDescriptor> {
        self.plugin_descriptors.get(&device)
    }

    fn create<B: EngineBridge + ?Sized>(
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
            DeviceKind::Missing { missing } => Err(BridgeError::Other(format!(
                "{} is missing",
                missing.description
            ))),
        }
    }

    /// Create/re-create/forget nodes so they match the document's devices. Returns error
    /// messages for devices that could not be instantiated. Plugin instantiation stops
    /// after [`PLUGIN_LOAD_BUDGET_MS`] (by `clock`, in ms); the rest waits for the next sync.
    pub fn sync_nodes<B: EngineBridge + ?Sized>(
        &mut self,
        bridge: &mut B,
        project: Option<&Project>,
        clock: &dyn Fn() -> u64,
    ) -> Vec<crate::text::Msg> {
        let mut errors = Vec::new();
        let started = clock();
        let mut out_of_budget = false;
        self.plugins_pending.clear();
        let empty = BTreeMap::new();
        let all = project.map_or(&empty, |p| &p.devices);
        // v0.2: frozen devices get no node (their CPU is saved; the track
        // plays its render, then a partial freeze's live tail). Their plugin state is in the
        // document (kept at freeze), so unfreezing re-creates them as they were.
        let live: BTreeMap<DeviceId, Device>;
        let devices = match project {
            Some(p) if p.tracks.values().any(|t| t.freeze.is_some()) => {
                let frozen = p.frozen_devices();
                live = all
                    .iter()
                    .filter(|(id, _)| !frozen.contains(id))
                    .map(|(k, d)| (*k, d.clone()))
                    .collect();
                &live
            }
            _ => all,
        };
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
        self.reload_from_doc.retain(|d| devices.contains_key(d));
        self.deferred.retain(|d| all.contains_key(d));
        for device in devices.values() {
            // A missing device (placeholder) has no node: nothing to create, no error.
            if matches!(device.kind, DeviceKind::Missing { .. }) {
                if let Some(n) = self.nodes.remove(&device.id) {
                    self.pending_destroy.push(n.key);
                }
                self.failed.remove(&device.id);
                continue;
            }
            if self.deferred.contains(&device.id)
                && matches!(device.kind, DeviceKind::Plugin { .. })
            {
                continue;
            }
            let sig = sig_of(&device.kind);
            let is_plugin = matches!(device.kind, DeviceKind::Plugin { .. });
            if out_of_budget && is_plugin {
                let needs_create = self.recreate.contains(&device.id)
                    || match self.nodes.get(&device.id) {
                        Some(n) => n.sig != sig,
                        None => self.failed.get(&device.id) != Some(&sig),
                    };
                if needs_create {
                    self.plugins_pending.push(device.id);
                    continue;
                }
            }
            let recreate = self.recreate.remove(&device.id);
            let from_doc = self.reload_from_doc.remove(&device.id);
            let mut live_state = None;
            // Data-only change of a built-in (sampler slices): update the live node in place
            // so sounding notes aren't cut, when the host supports it.
            if !recreate
                && let Some(n) = self.nodes.get_mut(&device.id)
                && n.sig != sig
                && let (NodeSig::Builtin(old), NodeSig::Builtin(new)) = (&n.sig, &sig)
                && (crate::drum_rack::updatable_in_place(old, new)
                    || crate::multisampler::updatable_in_place(old, new)
                    || crate::external::updatable_in_place(old, new)
                    || crate::fx_space::updatable_in_place(old, new)
                    || ether_devices::fx_modulation::updatable_in_place(old, new))
                && matches!(bridge.update_builtin(device.id, new), Ok(true))
            {
                n.sig = sig;
                continue;
            }
            let always = always_of(&device.kind);
            if let Some(n) = self.nodes.get_mut(&device.id)
                && n.sig == sig
                && !recreate
                && n.always != always
                && bridge.set_always_process(device.id, always).is_ok()
            {
                n.always = always;
            }
            match self.nodes.get(&device.id) {
                Some(n) if n.sig == sig && !recreate => continue,
                Some(n) => {
                    // The live state goes to the new node only when it is the same plugin
                    // (sandbox toggle, reload): another plugin (`Resolve::Replace`, its
                    // undo, a peer's swap) loads the document's state.
                    let same_plugin = matches!(
                        (&n.sig, &sig),
                        (NodeSig::Plugin { plugin_id: a, .. }, NodeSig::Plugin { plugin_id: b, .. })
                            if a == b
                    );
                    if same_plugin && !from_doc {
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
            let created = Self::create(bridge, device, live_state);
            if is_plugin && clock().saturating_sub(started) >= PLUGIN_LOAD_BUDGET_MS {
                out_of_budget = true;
            }
            match created {
                Ok(key) => {
                    self.failed.remove(&device.id);
                    if matches!(device.kind, DeviceKind::Plugin { .. }) {
                        let desc = bridge.descriptor(device.id);
                        self.set_plugin_descriptor(device.id, desc);
                        self.created_plugins.push(device.id);
                    }
                    self.nodes.insert(device.id, NodeEntry { key, sig, always });
                    self.midi_fx_scales.remove(&device.id);
                }
                Err(e) => {
                    // A plugin the remote engine has is heard from there: not having it
                    // here is no news.
                    let remote = matches!(&device.kind, DeviceKind::Plugin { plugin }
                        if self.remote.plugins.contains_key(&(plugin.format, plugin.plugin_id.clone())));
                    if !remote {
                        errors.push(
                            crate::text::msg("notice.engine.device_failed")
                                .arg("device", device.name.as_str())
                                .arg("error", e.to_string())
                                .into(),
                        );
                    }
                    self.failed.insert(device.id, sig);
                }
            }
        }
        errors
    }

    /// Sync nodes, compile and publish; destroy retired nodes afterwards. Returns
    /// user-facing problems.
    pub fn publish<B: EngineBridge + ?Sized>(
        &mut self,
        bridge: &mut B,
        project: Option<&Project>,
        armed: &BTreeSet<TrackId>,
        now_ms: u64,
        clock: &dyn Fn() -> u64,
    ) -> Vec<(NotificationLevel, crate::text::Msg)> {
        let mut problems: Vec<(NotificationLevel, crate::text::Msg)> = self
            .sync_nodes(bridge, project, clock)
            .into_iter()
            .map(|m| (NotificationLevel::Error, m))
            .collect();
        // v0.2 (`midi-fx`): new nodes and scale edits (which republish) get their scale.
        let nodes = &self.nodes;
        crate::midi_fx::push_scales(
            bridge,
            project,
            |d| nodes.contains_key(&d),
            &mut self.midi_fx_scales,
        );
        self.version += 1;
        let mut desc = match project {
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
        desc.click.count_in_end = self.count_in_end;
        if let Some(p) = project {
            self.pad_solo.retain(|pad| p.drum_pads.contains_key(pad));
        }
        crate::drum_rack::apply_solo(&mut desc, &self.pad_solo);
        crate::comping::apply_audition(&mut desc, project, &mut self.audition);
        self.last_publish_ms = Some(now_ms);
        // Read before publishing: a change racing the compile then costs one extra
        // republish instead of going unnoticed.
        self.published_latency.clear();
        for n in self.nodes.values() {
            self.published_latency
                .push((n.key, bridge.node_latency(n.key)));
        }
        match bridge.publish(desc) {
            Ok(()) => {
                self.rejected = None;
                self.queue_full_since = None;
                self.queue_full_told = false;
                // Plugins left to load: publish again at the next tick.
                self.graph_dirty = !self.plugins_pending.is_empty();
                for key in std::mem::take(&mut self.pending_destroy) {
                    let _ = bridge.destroy_node(key);
                }
            }
            // A full queue is transient: retried next tick, silently (a toast per tick
            // flooded the stack while a big graph was being sent). Said once if it lasts.
            Err(BridgeError::QueueFull) => {
                self.graph_dirty = true;
                let since = *self.queue_full_since.get_or_insert(now_ms);
                if !self.queue_full_told && now_ms.saturating_sub(since) >= QUEUE_FULL_NOTICE_MS {
                    self.queue_full_told = true;
                    problems.push((
                        NotificationLevel::Warning,
                        crate::text::msg("notice.engine.busy").into(),
                    ));
                }
            }
            Err(e) => {
                self.graph_dirty = false;
                self.refused = Some(e.to_string());
                let text = e.to_string();
                if text.contains("exceeds engine capacity") {
                    let message = capacity_message(&text);
                    self.rejected = Some(message.clone());
                    problems.push((NotificationLevel::Error, message));
                } else {
                    problems.push((
                        NotificationLevel::Error,
                        crate::text::msg("notice.engine.rejected")
                            .arg("error", e.to_string())
                            .into(),
                    ));
                }
            }
        }
        problems
    }

    /// Controller tick: mark the graph dirty when a node's latency
    /// differs from the value the last publish used for PDC, at most every
    /// [`LATENCY_REPUBLISH_MS`] after a publish. Never allocates; no-op while the graph is
    /// already dirty or before the first publish.
    pub fn check_latencies<B: EngineBridge + ?Sized>(&mut self, bridge: &B, now_ms: u64) {
        let Some(last) = self.last_publish_ms else {
            return;
        };
        if self.graph_dirty || now_ms.saturating_sub(last) < LATENCY_REPUBLISH_MS {
            return;
        }
        if self
            .published_latency
            .iter()
            .any(|&(key, lat)| bridge.node_latency(key) != lat)
        {
            self.graph_dirty = true;
        }
    }

    /// Engine effects of applied ops: continuous controls go to the param queue, anything
    /// else marks the graph dirty.
    pub fn apply_effects<B: EngineBridge + ?Sized>(
        &mut self,
        bridge: &mut B,
        project: &Project,
        applied: &[Op],
    ) {
        // v0.2 (`midi-fx`): a Scale Quantize's `Scale` source picks the pushed scale.
        if applied.iter().any(|op| {
            matches!(
                op,
                Op::Update {
                    update: EntityUpdate::Device { .. }
                }
            )
        }) {
            let nodes = &self.nodes;
            crate::midi_fx::push_scales(
                bridge,
                Some(project),
                |d| nodes.contains_key(&d),
                &mut self.midi_fx_scales,
            );
        }
        for op in applied {
            if crate::compile::graph_neutral(op) {
                continue;
            }
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
                    // Tracks inside a frozen group are inert (compiled without mute).
                    TrackChange::Mute(_) if project.frozen_ancestor(*id).is_some() => None,
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
                // v0.2: live modulator params.
                Op::Update {
                    update:
                        EntityUpdate::Modulator {
                            id,
                            change: ModulatorChange::Param { param, .. },
                        },
                } => project.modulators.get(id).map(|m| ParamChange {
                    target: ParamTarget::Modulator {
                        modulator: *id,
                        param: *param,
                    },
                    value: m.params.get(param).copied().unwrap_or_else(|| {
                        ether_devices::modulators::descriptor(m.kind)
                            .params
                            .iter()
                            .find(|p| p.id == *param)
                            .map_or(0.0, |p| p.default)
                    }),
                }),
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
pub struct EngineCtx<'a, B> {
    pub bridge: &'a mut B,
    pub eng: &'a mut EngineState,
    /// Loaded media (peaks, for auto-slicing).
    pub media: &'a crate::media::MediaState,
}

impl<B: EngineBridge> DocHost for EngineCtx<'_, B> {
    fn sandbox_by_default(&self, format: PluginFormat, plugin_id: &str) -> bool {
        self.bridge.sandbox_by_default(format, plugin_id)
    }

    fn peaks(&self, media: MediaId) -> Option<&ether_media::PeakMipmap> {
        self.media.peaks(media)
    }

    fn set_pad_solo(&mut self, pad: DrumPadId, solo: bool) {
        let changed = if solo {
            self.eng.pad_solo.insert(pad)
        } else {
            self.eng.pad_solo.remove(&pad)
        };
        if changed {
            self.eng.graph_dirty = true;
        }
    }

    fn set_audition(&mut self, track: TrackId, lane: Option<TakeLaneId>) {
        let changed = match lane {
            Some(l) => self.eng.audition.insert(track, l) != Some(l),
            None => self.eng.audition.remove(&track).is_some(),
        };
        if changed {
            self.eng.graph_dirty = true;
        }
    }

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
                // The remote engine's: no instance here.
                d.or_else(|| self.eng.remote.descriptors.get(&device).cloned())
            }
            DeviceKind::Missing { missing } => Some(missing_descriptor(missing)),
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
        // A remote engine plays the project: with this engine off nothing is instantiated
        // here (the remote engine instantiates it from the document); a plugin from its list
        // goes in even when it is not installed here.
        if self.eng.off {
            return Ok(self.eng.remote.insert_descriptor(plugin));
        }
        let key = match self.bridge.create_plugin(device, plugin, None) {
            Ok(key) => key,
            Err(e) => match self.eng.remote.insert_descriptor(plugin) {
                Some(d) => return Ok(Some(d)),
                None => return Err(bridge_err(e)),
            },
        };
        if let Some(old) = self.eng.nodes.insert(
            device,
            NodeEntry {
                key,
                sig: sig_of(&DeviceKind::Plugin {
                    plugin: plugin.clone(),
                }),
                always: plugin.always_process,
            },
        ) {
            self.eng.pending_destroy.push(old.key);
        }
        let desc = self.bridge.descriptor(device);
        self.eng.set_plugin_descriptor(device, desc.clone());
        self.eng.created_plugins.push(device);
        self.eng.graph_dirty = true;
        Ok(desc)
    }
}

/// What the user reads when the engine can't take the project's graph.
pub fn capacity_message(engine_error: &str) -> crate::text::Msg {
    crate::text::msg("notice.engine.too_large")
        .arg("error", engine_error)
        .into()
}

#[cfg(test)]
mod remote_plugins_tests {
    use super::*;

    fn plugin(id: &str) -> PluginInstance {
        PluginInstance {
            format: PluginFormat::Clap,
            plugin_id: id.into(),
            name: String::new(),
            vendor: String::new(),
            version: String::new(),
            sandboxed: false,
            always_process: false,
            state: None,
        }
    }

    #[test]
    fn a_remote_plugin_inserts_with_its_listed_name_and_category() {
        let mut remote = RemotePlugins::default();
        assert!(remote.insert_descriptor(&plugin("synth")).is_none());
        for (id, category, name) in [
            ("synth", DeviceCategory::Instrument, "Synth"),
            ("arp", DeviceCategory::NoteEffect, "Arp"),
            ("verb", DeviceCategory::AudioEffect, "Verb"),
        ] {
            remote
                .plugins
                .insert((PluginFormat::Clap, id.into()), (category, name.into()));
        }
        let synth = remote.insert_descriptor(&plugin("synth")).unwrap();
        assert_eq!(synth.name, "Synth");
        assert_eq!(
            (synth.audio_inputs, synth.audio_outputs, synth.midi_input),
            (0, 2, true)
        );
        let arp = remote.insert_descriptor(&plugin("arp")).unwrap();
        assert_eq!(
            (arp.audio_inputs, arp.audio_outputs, arp.midi_input),
            (0, 0, true)
        );
        let verb = remote.insert_descriptor(&plugin("verb")).unwrap();
        assert_eq!(
            (verb.audio_inputs, verb.audio_outputs, verb.midi_input),
            (2, 2, false)
        );
        assert!(verb.params.is_empty(), "the remote engine reports them");
    }
}

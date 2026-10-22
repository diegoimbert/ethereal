//! The controller Worker's side of WebCLAP (used by `crate::bridge::WebBridge`): the
//! catalog, the live devices, and the requests to the Worklet's instances.
//!
//! The controller's plugin calls are synchronous (`EngineBridge::plugin_state` before a
//! save); the instances live in the Worklet. A request goes over the control ring and the
//! Worker blocks (`Atomics.wait`, [`WebclapBridge::set_waiter`]) until the reply comes back
//! on the report ring, the next block or so. When the Worklet isn't rendering (the audio
//! context is not running yet), nothing can have changed the instance: the last known
//! state is returned without waiting long.

use std::collections::BTreeMap;

use ether_controller::{BridgeError, PluginCatalogView};
use ether_core::NodeKey;
use ether_core::plugin::PluginNotification;
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{Base64Bytes, DeviceId, ParamId, PluginFormat, PluginInstance};

use super::catalog::{Catalog, ScannedPlugin, device_descriptor};
use super::node::{NodeSpec, WebclapNode};
use super::proto::{PluginReport, WebclapMsg, WebclapRequest};

/// How long a request waits for the Worklet at most.
pub const REQUEST_TIMEOUT_MS: u32 = 2_000;
/// ...and how long while the Worklet renders no block at all.
pub const IDLE_TIMEOUT_MS: u32 = 250;
/// One wait step.
const WAIT_STEP_MS: u32 = 2;

/// Lives in `BridgeShared` (reachable from `WasmController` after the bridge moved into
/// the controller).
#[derive(Default)]
pub struct WebclapShared {
    pub catalog: Catalog,
    /// Decoded [`super::proto::REPORT_PLUGIN`] frames not handled yet.
    pub reports: Vec<PluginReport>,
}

struct Live {
    key: NodeKey,
    plugin_id: String,
    /// The last state known: the one created with, then every one read.
    state: Option<Base64Bytes>,
}

enum Reply {
    State(Option<Vec<u8>>),
    Values(Vec<(u32, f64)>),
    Failed(String),
}

#[derive(Default)]
pub struct WebclapBridge {
    live: BTreeMap<DeviceId, Live>,
    notes: Vec<(DeviceId, PluginNotification)>,
    next_seq: u32,
    replies: BTreeMap<u32, Reply>,
    /// State chunks being assembled, by request.
    partial: BTreeMap<u32, Vec<u8>>,
    waiter: Option<Box<dyn FnMut(u32)>>,
}

/// The node spec and state of a device of `plugin`.
pub fn node_spec(
    device: DeviceId,
    module: &str,
    scanned: &ScannedPlugin,
    always_process: bool,
) -> NodeSpec {
    NodeSpec {
        module: module.to_string(),
        plugin_id: scanned.id.clone(),
        name: scanned.name.clone(),
        device: device.to_string(),
        layout: scanned.layout(),
        category: scanned.category(),
        note_expressions: scanned.note_expressions,
        params: scanned.params.iter().map(|p| (p.id, p.default)).collect(),
        always_process,
    }
}

pub fn not_installed(plugin: &PluginInstance) -> BridgeError {
    let name = if plugin.name.is_empty() || plugin.name == plugin.plugin_id {
        String::new()
    } else {
        format!("{} ", plugin.name)
    };
    BridgeError::Other(format!(
        "webclap plugin {name}({}) is not installed (install it from the plugin browser)",
        plugin.plugin_id
    ))
}

impl WebclapBridge {
    /// How the Worker sleeps while it waits for the Worklet (`ms` at most). Without one,
    /// requests only look at what already arrived.
    pub fn set_waiter(&mut self, waiter: Box<dyn FnMut(u32)>) {
        self.waiter = Some(waiter);
    }

    /// A WebCLAP device's `Create` message (the caller allocated `key` and sends it).
    pub fn create(
        &mut self,
        shared: &WebclapShared,
        key: NodeKey,
        device: DeviceId,
        plugin: &PluginInstance,
        state: Option<&Base64Bytes>,
    ) -> Result<WebclapMsg, BridgeError> {
        let (module, scanned) = shared
            .catalog
            .find(&plugin.plugin_id)
            .ok_or_else(|| not_installed(plugin))?;
        let spec = node_spec(device, module, scanned, plugin.always_process);
        self.live.insert(
            device,
            Live {
                key,
                plugin_id: plugin.plugin_id.clone(),
                state: state.cloned(),
            },
        );
        Ok(WebclapMsg::Create {
            key,
            device,
            module: spec.module,
            plugin_id: spec.plugin_id,
            name: spec.name,
            layout: spec.layout,
            category: spec.category,
            note_expressions: spec.note_expressions,
            params: spec.params,
            always_process: spec.always_process,
            state: state.cloned(),
        })
    }

    /// An offline node in this realm (the controller Worker's own host).
    pub fn create_offline(
        &self,
        shared: &WebclapShared,
        device: DeviceId,
        plugin: &PluginInstance,
        state: Option<&Base64Bytes>,
        sample_rate: u32,
    ) -> Result<Box<dyn ether_core::Node>, BridgeError> {
        let (module, scanned) = shared
            .catalog
            .find(&plugin.plugin_id)
            .ok_or_else(|| not_installed(plugin))?;
        if !super::guest::has_guest() {
            return Err(BridgeError::Unsupported(
                "webclap plugins can't be rendered offline here (no host in this worker)".into(),
            ));
        }
        let node = WebclapNode::new(
            node_spec(device, module, scanned, plugin.always_process),
            state.map(|s| &s.0[..]),
            f64::from(sample_rate),
            ether_core::offline::OFFLINE_MAX_BLOCK,
        );
        let handle = node.handle();
        match super::guest::with_guest(|g| g.ready(handle)) {
            Some(Ok(true)) => Ok(Box::new(node)),
            Some(Ok(false)) => Err(BridgeError::Other(format!(
                "webclap plugin {} is not loaded in the render worker yet",
                plugin.plugin_id
            ))),
            Some(Err(e)) => Err(BridgeError::Other(e)),
            None => Err(BridgeError::Unsupported("no webclap host".into())),
        }
    }

    pub fn key(&self, device: DeviceId) -> Option<NodeKey> {
        self.live.get(&device).map(|l| l.key)
    }

    /// The node `key` was destroyed.
    pub fn forget_key(&mut self, key: NodeKey) {
        self.live.retain(|_, l| l.key != key);
    }

    pub fn descriptor(&self, shared: &WebclapShared, device: DeviceId) -> Option<DeviceDescriptor> {
        let live = self.live.get(&device)?;
        let (_, scanned) = shared.catalog.find(&live.plugin_id)?;
        Some(device_descriptor(scanned))
    }

    pub fn probe(shared: &WebclapShared, plugin: &PluginInstance) -> Option<DeviceDescriptor> {
        if plugin.format != PluginFormat::Webclap {
            return None;
        }
        shared
            .catalog
            .find(&plugin.plugin_id)
            .map(|(_, p)| device_descriptor(p))
    }

    pub fn catalog_view(shared: &WebclapShared) -> PluginCatalogView {
        PluginCatalogView {
            plugins: shared.catalog.descriptors(),
            scanning: false,
        }
    }

    /// Handle the reports the bridge drained.
    pub fn take_reports(&mut self, shared: &mut WebclapShared) {
        for r in shared.reports.drain(..) {
            let key_of = |k: NodeKey| {
                self.live
                    .iter()
                    .find(|(_, l)| l.key == k)
                    .map(|(d, _)| *d)
            };
            match r {
                PluginReport::ParamEdited { key, id, value } => {
                    if let Some(d) = key_of(key) {
                        self.notes.push((
                            d,
                            PluginNotification::ParamEdited {
                                param: ParamId(id),
                                value,
                            },
                        ));
                    }
                }
                PluginReport::GestureBegin { key, id } => {
                    if let Some(d) = key_of(key) {
                        self.notes.push((
                            d,
                            PluginNotification::GestureBegin { param: ParamId(id) },
                        ));
                    }
                }
                PluginReport::GestureEnd { key, id } => {
                    if let Some(d) = key_of(key) {
                        self.notes
                            .push((d, PluginNotification::GestureEnd { param: ParamId(id) }));
                    }
                }
                PluginReport::StateDirty { key } => {
                    if let Some(d) = key_of(key) {
                        self.notes.push((d, PluginNotification::StateDirty));
                    }
                }
                PluginReport::ParamsChanged { key } => {
                    if let Some(d) = key_of(key) {
                        self.notes.push((d, PluginNotification::ParamsChanged));
                    }
                }
                PluginReport::Crashed { key, message } => {
                    if let Some(d) = key_of(key) {
                        self.notes.push((d, PluginNotification::Crashed { message }));
                    }
                }
                PluginReport::State {
                    seq,
                    has_state,
                    total,
                    offset,
                    bytes,
                    ..
                } => {
                    if !has_state {
                        self.replies.insert(seq, Reply::State(None));
                        continue;
                    }
                    let buf = self
                        .partial
                        .entry(seq)
                        .or_insert_with(|| Vec::with_capacity(total as usize));
                    let end = offset as usize + bytes.len();
                    if buf.len() < end {
                        buf.resize(end.min(total as usize), 0);
                    }
                    let n = bytes.len().min(buf.len().saturating_sub(offset as usize));
                    buf[offset as usize..offset as usize + n].copy_from_slice(&bytes[..n]);
                    if end >= total as usize {
                        let buf = self.partial.remove(&seq).unwrap_or_default();
                        self.replies.insert(seq, Reply::State(Some(buf)));
                    }
                }
                PluginReport::Values { seq, values, .. } => {
                    self.replies.insert(seq, Reply::Values(values));
                }
                PluginReport::Failed { seq, message, .. } => {
                    self.partial.remove(&seq);
                    self.replies.insert(seq, Reply::Failed(message));
                }
            }
        }
    }

    pub fn poll_plugins(&mut self, out: &mut Vec<(DeviceId, PluginNotification)>) {
        out.append(&mut self.notes);
    }

    fn next_seq(&mut self) -> u32 {
        self.next_seq = self.next_seq.wrapping_add(1);
        self.next_seq
    }

    /// Ask the Worklet; `send` queues the message (and flushes), `pump` drains the report
    /// ring into the bridge (returns the Worklet's rendered block count).
    fn request(
        &mut self,
        device: DeviceId,
        request: WebclapRequest,
        send: &mut dyn FnMut(WebclapMsg),
        pump: &mut dyn FnMut(&mut Self) -> u64,
    ) -> Option<Reply> {
        let key = self.live.get(&device)?.key;
        let seq = self.next_seq();
        send(WebclapMsg::Request { key, seq, request });
        let start = pump(self);
        let mut waited = 0;
        loop {
            if let Some(r) = self.replies.remove(&seq) {
                return Some(r);
            }
            if self.waiter.is_none() {
                return None;
            }
            let blocks = pump(self);
            if let Some(r) = self.replies.remove(&seq) {
                return Some(r);
            }
            if waited >= REQUEST_TIMEOUT_MS || (waited >= IDLE_TIMEOUT_MS && blocks == start) {
                return None;
            }
            if let Some(wait) = self.waiter.as_mut() {
                wait(WAIT_STEP_MS);
            }
            waited += WAIT_STEP_MS;
        }
    }

    /// `EngineBridge::plugin_state` of a WebCLAP device (`None`: not one).
    pub fn plugin_state(
        &mut self,
        device: DeviceId,
        send: &mut dyn FnMut(WebclapMsg),
        pump: &mut dyn FnMut(&mut Self) -> u64,
    ) -> Option<Result<Option<Base64Bytes>, BridgeError>> {
        self.live.get(&device)?;
        let reply = self.request(device, WebclapRequest::State, send, pump);
        let live = self.live.get_mut(&device)?;
        Some(match reply {
            Some(Reply::State(state)) => {
                live.state = state.map(Base64Bytes);
                Ok(live.state.clone())
            }
            Some(Reply::Failed(m)) if live.state.is_some() => {
                let _ = m;
                Ok(live.state.clone())
            }
            Some(Reply::Failed(m)) => Err(BridgeError::Other(m)),
            // Not rendering (or slow): the last state known.
            None | Some(Reply::Values(_)) => Ok(live.state.clone()),
        })
    }

    pub fn param_values(
        &mut self,
        device: DeviceId,
        send: &mut dyn FnMut(WebclapMsg),
        pump: &mut dyn FnMut(&mut Self) -> u64,
    ) -> Vec<(ParamId, f64)> {
        match self.request(device, WebclapRequest::Values, send, pump) {
            Some(Reply::Values(v)) => v.into_iter().map(|(id, x)| (ParamId(id), x)).collect(),
            _ => Vec::new(),
        }
    }
}

//! The controller: the single writer of the document.
//!
//! `ClientMessage` → (validate, translate to ops) → `History::commit` → `Patch` events →
//! recompile the `RenderGraphDesc` and publish it to the engine (or push a live param
//! change for continuous controls) → `Reply`. Host-agnostic and wasm-safe: everything
//! host-specific (engine transport, file/OPFS I/O, media decoding, plugins, clock, entropy)
//! is behind the [`EngineBridge`], [`HostServices`], [`store::ProjectStore`] and
//! [`store::Library`] traits. The controller is the only place that decides *what* is
//! read/written; the UI never handles files.
//!
//! Runs on a non-RT thread: the Tauri backend thread natively, a Web Worker on the web.
//! Owned by the `controller` node.

pub mod store;

use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{
    Base64Bytes, BuiltinDevice, DeviceId, MediaId, MediaRef, ParamId, PluginInstance, Project,
};
use ether_core::protocol::{ClientMessage, ServerMessage};
use ether_core::{EngineOutputs, NodeKey, ParamChange, RenderGraphDesc, TransportControl};

/// Receives messages for the UI (Tauri emitter, `postMessage`, test vector, ...).
pub trait MessageSink {
    fn send(&mut self, message: ServerMessage);
}

impl MessageSink for Vec<ServerMessage> {
    fn send(&mut self, message: ServerMessage) {
        self.push(message);
    }
}

/// The controller as seen by hosts.
pub trait Controller {
    /// Handle one UI message: emits zero or more `Event`s (patches first) and exactly one
    /// `Reply` into `out`.
    fn handle(&mut self, message: ClientMessage, out: &mut dyn MessageSink);

    /// Periodic work at UI rate (~30-60 Hz): poll engine outputs (→ `Playhead`/`Meters`/
    /// `Session` messages), plugin notifications, autosave, GC of media caches.
    fn tick(&mut self, now_ms: u64, out: &mut dyn MessageSink);

    /// Read-only view of the open document (`None` before a project is created/opened).
    fn project(&self) -> Option<&Project>;
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum BridgeError {
    #[error("engine unavailable: {0}")]
    Unavailable(String),
    #[error("engine queue full")]
    QueueFull,
    #[error("unsupported on this host: {0}")]
    Unsupported(String),
    #[error("{0}")]
    Other(String),
}

/// How the controller talks to the engine.
///
/// Native: implemented directly over `ether_core::EngineHandle` (same process; nodes are
/// constructed here with `ether-devices` and sent boxed). Web: implemented by `ether-wasm`
/// by serializing each call over a SharedArrayBuffer ring to the AudioWorklet, which owns
/// the `EngineHandle` and constructs nodes itself (so every argument is plain data).
pub trait EngineBridge {
    /// Instantiate a built-in device node with initial plain param values.
    fn create_builtin(
        &mut self,
        device: DeviceId,
        kind: &BuiltinDevice,
        params: &[(ParamId, f64)],
    ) -> Result<NodeKey, BridgeError>;

    /// Instantiate a plugin node (native only; web returns `Unsupported`). `state` is the
    /// saved plugin state blob.
    fn create_plugin(
        &mut self,
        device: DeviceId,
        plugin: &PluginInstance,
        state: Option<&Base64Bytes>,
    ) -> Result<NodeKey, BridgeError>;

    fn destroy_node(&mut self, key: NodeKey) -> Result<(), BridgeError>;

    /// Make decoded media (already at the engine sample rate; decoded by the controller
    /// with `ether-media` from bytes read via the `ProjectStore`) available to the engine.
    fn load_media(
        &mut self,
        media: &MediaRef,
        audio: std::sync::Arc<ether_media::DecodedAudio>,
    ) -> Result<(), BridgeError>;
    fn unload_media(&mut self, media: MediaId) -> Result<(), BridgeError>;

    fn publish(&mut self, graph: RenderGraphDesc) -> Result<(), BridgeError>;
    fn set_param(&mut self, change: ParamChange) -> Result<(), BridgeError>;
    fn transport(&mut self, control: TransportControl) -> Result<(), BridgeError>;
    fn poll(&mut self, out: &mut EngineOutputs);

    /// Descriptors of built-in devices and of instantiated plugins.
    fn descriptor(&mut self, device: DeviceId) -> Option<DeviceDescriptor>;
}

/// Host services the controller needs besides the engine.
pub trait HostServices {
    /// Wall clock, Unix ms (for IDs, autosave, `modified_ms`).
    fn now_ms(&self) -> u64;
    /// Entropy for the `IdGen` seed.
    fn random_seed(&mut self) -> u64;
}

/// The standard controller implementation.
pub struct EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: store::ProjectStore,
    L: store::Library,
{
    pub bridge: B,
    pub host: H,
    pub store: S,
    pub library: L,
    _private: (),
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: store::ProjectStore,
    L: store::Library,
{
    /// Start with no project open (the UI lists/creates/opens via `ProjectCommand`), or
    /// reopen the last project if the host asks to.
    pub fn new(bridge: B, host: H, store: S, library: L) -> Self {
        let _ = (bridge, host, store, library);
        todo!("controller node")
    }
}

impl<B, H, S, L> Controller for EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: store::ProjectStore,
    L: store::Library,
{
    fn handle(&mut self, message: ClientMessage, out: &mut dyn MessageSink) {
        let _ = (message, out);
        todo!("controller node")
    }

    fn tick(&mut self, now_ms: u64, out: &mut dyn MessageSink) {
        let _ = (now_ms, out);
        todo!("controller node")
    }

    fn project(&self) -> Option<&Project> {
        todo!("controller node")
    }
}

/// Compile the document into the engine's render description. Pure function (unit-testable
/// without an engine); `nodes` maps devices to their engine node keys.
pub fn compile_graph(
    project: &Project,
    nodes: &dyn Fn(DeviceId) -> Option<NodeKey>,
    version: u64,
) -> RenderGraphDesc {
    let _ = (project, nodes, version);
    todo!("controller node")
}

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
//!
//! # Behaviour summary (see `docs/CONTRACTS.md` §5)
//! - **Ordering.** For every message: document patches (`Event::Patch`) and other events
//!   first, then exactly one `Reply`. Graph publishing is coalesced: at most once every
//!   [`ControllerConfig::publish_interval_ms`] from `handle`, and always from `tick` if
//!   something is pending.
//! - **Undo.** One undo step per command; commands with the same `gesture` merge until
//!   `Edit::EndGesture`; `Edit::Batch` is one all-or-nothing step.
//! - **Idempotent creates.** Creating an entity whose (client-chosen) id already exists is
//!   a successful no-op.
//! - **Unsupported.** Host-handled commands (`Engine::*`, `Plugin::{Rescan, List, OpenEditor,
//!   CloseEditor}`), media preview/upload and `Recording::ListInputs` reply `Unsupported`.
//! - **Async media.** Import copies the file and probes its header in `handle` (the reply
//!   carries the `MediaRef`); decoding, peaks and resampling are stepped from `tick`.
//! - **Engine sample rate.** Media is resampled to [`ControllerConfig::engine_sample_rate`];
//!   hosts call [`EtherController::set_engine_sample_rate`] when the device changes.

pub mod compile;
mod doc;
mod engine;
mod handlers;
mod media;
pub mod memory;
mod plugins;
mod project;
mod recording;
pub mod store;
mod tx;
mod warp;

use std::collections::BTreeMap;

use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{
    Base64Bytes, BuiltinDevice, DeviceId, GestureId, History, IdGen, MediaId, MediaRef, ParamId,
    PluginInstance, Project,
};
use ether_core::protocol::transport::TransportState;
use ether_core::protocol::{ClientMessage, Reply, ReplyResult, ServerMessage};
use ether_core::{EngineOutputs, NodeKey, ParamChange, RenderGraphDesc, TransportControl};

pub use compile::{CompileContext, compile_graph_with};
pub use media::hash::content_hash;

/// Lowest volume/send level: `Decibels::SILENCE` (treated as -inf).
pub const SILENCE_DB: f32 = ether_core::protocol::model::Decibels::SILENCE.0;
/// Highest track volume accepted (commands are clamped).
pub const MAX_VOLUME_DB: f32 = 6.0;
/// Highest send level accepted (commands are clamped).
pub const MAX_SEND_DB: f32 = 6.0;

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

    /// Periodic work at UI rate (~30-60 Hz): poll engine outputs (→ `Playhead`/`Meters`
    /// messages), plugin notifications, autosave, GC of media caches.
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

    /// Drain main-thread notifications from plugin controllers (GUI param edits and gestures,
    /// latency changes, crashes). The controller calls this from its tick: `ParamEdited` becomes
    /// an undoable SetParam, `LatencyChanged` a republish, `Crashed` a `PluginEvent::Crashed`.
    /// Hosts without plugins (web) keep the default.
    fn poll_plugins(&mut self, out: &mut Vec<(DeviceId, ether_core::plugin::PluginNotification)>) {
        let _ = out;
    }

    /// Current state blob of a plugin device, read before serializing the project (Save,
    /// SaveAs, autosave) into `PluginInstance.state`. `Ok(None)` = not a plugin / no state.
    fn plugin_state(&mut self, device: DeviceId) -> Result<Option<Base64Bytes>, BridgeError> {
        let _ = device;
        Ok(None)
    }

    /// Current plain values of a plugin device's params, read after instantiation (state
    /// load) to mirror them into the document. Hosts without plugins keep the default.
    fn plugin_param_values(&mut self, device: DeviceId) -> Vec<(ParamId, f64)> {
        let _ = device;
        Vec::new()
    }
}

/// Host services the controller needs besides the engine.
pub trait HostServices {
    /// Wall clock, Unix ms (for IDs, autosave, `modified_ms`).
    fn now_ms(&self) -> u64;
    /// Entropy for the `IdGen` seed.
    fn random_seed(&mut self) -> u64;
}

/// Tunables (defaults suit a 30-60 Hz tick).
#[derive(Clone, Debug, PartialEq)]
pub struct ControllerConfig {
    /// Rate media is resampled to before `EngineBridge::load_media`.
    pub engine_sample_rate: u32,
    /// Autosave a dirty project after this long without edits (`None` = never).
    pub autosave_after_ms: Option<u64>,
    /// Minimum time between two graph publishes from `handle` (bursts coalesce; `tick`
    /// flushes).
    pub publish_interval_ms: u64,
    /// Media work per tick, in frames (decode + resample). ~1 s of audio keeps a tick
    /// around 10-20 ms in release builds.
    pub media_frames_per_tick: usize,
    /// Undo depth (0 = unlimited).
    pub history_depth: usize,
    /// Written into saved files (`app_version`).
    pub app_version: String,
}

impl Default for ControllerConfig {
    fn default() -> Self {
        Self {
            engine_sample_rate: 48_000,
            autosave_after_ms: Some(60_000),
            publish_interval_ms: 30,
            media_frames_per_tick: 48_000,
            history_depth: 500,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

/// The open project and its edit state.
struct OpenDoc {
    project: Project,
    history: History,
    dirty: bool,
    /// Last document edit (for autosave).
    last_edit_ms: u64,
}

/// Runtime transport state (not in the document).
#[derive(Clone, Debug, Default)]
struct TransportRt {
    playing: bool,
    recording: bool,
    position: ether_core::protocol::model::Beats,
    seconds: f64,
    start_position: ether_core::protocol::model::Beats,
    last_frame: Option<ether_core::protocol::transport::PlayheadUpdate>,
    /// Engine play flag at the last poll (its transitions are adopted).
    last_engine_playing: Option<bool>,
    /// Recent `TapTempo` times (ms).
    taps: Vec<u64>,
    tap_gesture: Option<GestureId>,
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
    config: ControllerConfig,
    ids: IdGen,
    doc: Option<OpenDoc>,
    /// Monotonic patch revision (across projects).
    revision: u64,
    engine: engine::EngineState,
    media: media::MediaState,
    transport: TransportRt,
    /// Record-armed tracks (runtime state, not undoable).
    armed: std::collections::BTreeSet<ether_core::protocol::model::TrackId>,
    /// Open plugin-GUI gestures → internal gesture ids.
    plugin_gestures: BTreeMap<(DeviceId, ParamId), GestureId>,
    /// Plugin runtime bookkeeping (param mirroring after load; `plugins` module).
    plugins: plugins::PluginsState,
    next_gesture: u32,
    last_transport: Option<TransportState>,
    outputs: EngineOutputs,
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
        Self::with_config(bridge, host, store, library, ControllerConfig::default())
    }

    pub fn with_config(
        bridge: B,
        mut host: H,
        store: S,
        library: L,
        config: ControllerConfig,
    ) -> Self {
        let seed = host.random_seed();
        Self {
            bridge,
            host,
            store,
            library,
            ids: IdGen::new(seed),
            doc: None,
            revision: 0,
            engine: engine::EngineState::default(),
            media: media::MediaState::default(),
            transport: TransportRt::default(),
            armed: Default::default(),
            plugin_gestures: BTreeMap::new(),
            plugins: Default::default(),
            // Internal gestures (plugin GUI, tap tempo) live in the upper half of the id
            // space, away from UI-allocated ones.
            next_gesture: 0x8000_0000,
            last_transport: None,
            outputs: EngineOutputs::default(),
            config,
            _private: (),
        }
    }

    pub fn config(&self) -> &ControllerConfig {
        &self.config
    }

    /// The engine runs at a new sample rate: media is re-resampled and reloaded.
    pub fn set_engine_sample_rate(&mut self, sample_rate: u32) {
        if sample_rate == 0 || sample_rate == self.config.engine_sample_rate {
            return;
        }
        self.config.engine_sample_rate = sample_rate;
        self.media.reload_all();
        let project = self.doc.as_ref().map(|d| &d.project);
        self.media.sync(&mut self.bridge, project);
    }

    /// Open a stored project without a UI message (e.g. "reopen last project" at startup).
    /// Events go to `out`.
    pub fn open_project(
        &mut self,
        id: ether_core::protocol::model::ProjectId,
        out: &mut dyn MessageSink,
    ) -> Result<(), ether_core::protocol::CommandError> {
        let now = self.host.now_ms();
        self.open(id, now, out).map(drop)
    }

    /// Save the open project now (e.g. before the host quits). Events go to `out`.
    pub fn save_now(
        &mut self,
        out: &mut dyn MessageSink,
    ) -> Result<ether_core::protocol::project::ProjectSummary, ether_core::protocol::CommandError>
    {
        self.save_current(out)
    }

    /// Unsaved changes in the open project.
    pub fn is_dirty(&self) -> bool {
        self.doc.as_ref().is_some_and(|d| d.dirty)
    }

    /// Media still being decoded/resampled.
    pub fn media_pending(&self) -> bool {
        self.media.has_jobs()
    }

    /// Record-armed tracks.
    pub fn armed(&self) -> Vec<ether_core::protocol::model::TrackId> {
        self.armed.iter().copied().collect()
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
        let now = self.host.now_ms();
        let result = self.dispatch(&message, now, out);
        self.emit_transport_if_changed(out);
        out.send(ServerMessage::Reply(Reply {
            id: message.id,
            result: match result {
                Ok(value) => ReplyResult::Ok { value },
                Err(error) => ReplyResult::Err { error },
            },
        }));
        self.publish_if_due(now, false, out);
    }

    fn tick(&mut self, now_ms: u64, out: &mut dyn MessageSink) {
        self.tick_impl(now_ms, out);
    }

    fn project(&self) -> Option<&Project> {
        self.doc.as_ref().map(|d| &d.project)
    }
}

/// Compile the document into the engine's render description. Pure function (unit-testable
/// without an engine); `nodes` maps devices to their engine node keys. Plugin device
/// automation needs instance descriptors: use [`compile_graph_with`] for that.
pub fn compile_graph(
    project: &Project,
    nodes: &dyn Fn(DeviceId) -> Option<NodeKey>,
    version: u64,
) -> RenderGraphDesc {
    compile_graph_with(
        project,
        &CompileContext {
            nodes,
            descriptors: &compile::builtin_descriptors,
            armed: &|_| false,
            version,
        },
    )
}

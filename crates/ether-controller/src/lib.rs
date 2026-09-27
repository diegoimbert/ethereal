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
//!   CloseEditor}`) and media preview reply `Unsupported` (uploads: `upload` module, over the
//!   store's staging methods); `Recording::ListInputs` and
//!   record sessions go to the bridge (`EngineBridge::{list_inputs, start_recording, ...}`).
//! - **Async media.** Import copies the file and probes its header in `handle` (the reply
//!   carries the `MediaRef`); decoding, peaks and resampling are stepped from `tick`.
//! - **Engine sample rate.** Media is resampled to [`ControllerConfig::engine_sample_rate`];
//!   hosts call [`EtherController::set_engine_sample_rate`] when the device changes.

mod analysis;
mod browser;
mod clip_editing;
mod collab;
pub mod compile;
mod comping;
mod doc;
mod drum_rack;
mod engine;
mod export;
mod freeze;
mod groove;
mod groups;
mod handlers;
mod media;
mod media_preview;
mod media_refs;
pub mod memory;
mod midi_fx;
mod midi_learn;
mod multisampler;
mod plugins;
mod presets;
mod project;
mod racks;
mod recording;
mod sidechain;
pub mod store;
pub mod streaming;
mod tempo;
mod time_edit;
mod tx;
mod upload;
mod warp;

use std::collections::BTreeMap;

use ether_core::protocol::collab::{IceServer, StreamSignal};
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{
    Base64Bytes, BuiltinDevice, DeviceId, GestureId, History, IdGen, MediaId, MediaRef, ParamId,
    PluginInstance, Project, SiteId,
};
use ether_core::protocol::transport::TransportState;
use ether_core::protocol::{ClientMessage, Reply, ReplyResult, ServerMessage};
use ether_core::{EngineOutputs, NodeKey, ParamChange, RenderGraphDesc, TransportControl};

pub use compile::{CompileContext, compile_graph_with};
pub use media::hash::content_hash;
pub use recording::{AudioTake, AudioTarget, RecordSession, RecordedMidi, RecordedTakes};

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

    /// Roadmap v2 (`midi-learn`): drain incoming MIDI messages received since the last
    /// call (all ports), for MIDI mappings/learn. Called from every controller tick.
    /// Hosts without MIDI input keep the default.
    fn poll_midi_input(&mut self, out: &mut Vec<ether_core::protocol::midi_map::MidiInputEvent>) {
        let _ = out;
    }

    /// Roadmap v2 (`drum-rack`): update a live built-in node's non-parameter data in place
    /// (sampler slices) instead of re-creating it, so an edit doesn't cut sounding notes.
    /// Native: `EngineHandle::set_node_data`; web: serialized to the worklet. `Ok(false)`
    /// (the default) = not supported for this change: the controller re-creates the node.
    fn update_builtin(
        &mut self,
        device: DeviceId,
        kind: &BuiltinDevice,
    ) -> Result<bool, BridgeError> {
        let _ = (device, kind);
        Ok(false)
    }

    /// `media-preview`: play decoded `audio` (at the engine rate) on the engine's preview
    /// voice at linear `gain` as preview `id` (controller-chosen, monotonic), replacing any
    /// playing preview; `audio: None` stops it (`id` ignored). Native: `EngineHandle::preview`
    /// with an in-memory source; web: the audio is shipped to the worklet like `load_media`.
    /// Natural ends come back as `EngineOutputs::preview_ended = Some(id)` (from `poll`);
    /// stop/replace are never reported. Default: unsupported.
    fn preview(
        &mut self,
        id: u64,
        audio: Option<std::sync::Arc<ether_media::DecodedAudio>>,
        gain: f32,
    ) -> Result<(), BridgeError> {
        let _ = (id, audio, gain);
        Err(BridgeError::Unsupported(
            "preview is not available on this host".into(),
        ))
    }

    /// Roadmap v2 (`export`): a fresh, independent plugin node for offline rendering
    /// (prepared at `sample_rate`, state loaded from `state`). Never the live instance.
    /// Default: unsupported. An export never skips a plugin: if this fails (unsupported
    /// host, plugin not installed, instantiation error) the whole export fails with a
    /// message naming the plugin. Disabled plugin devices are not instantiated.
    fn create_offline_plugin(
        &mut self,
        device: DeviceId,
        plugin: &PluginInstance,
        state: Option<&Base64Bytes>,
        sample_rate: u32,
    ) -> Result<Box<dyn ether_core::Node>, BridgeError> {
        let _ = (device, plugin, state, sample_rate);
        Err(BridgeError::Unsupported(
            "offline plugin rendering is not available on this host".into(),
        ))
    }

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

    /// Hardware inputs for `RecordingCommand::ListInputs` (native). Default: unsupported (web).
    fn list_inputs(&mut self) -> Result<ether_core::protocol::recording::InputList, BridgeError> {
        Err(BridgeError::Unsupported(
            "input listing is not available on this host".into(),
        ))
    }

    /// Start capturing armed tracks' input into take files under the project's `media/` (the
    /// controller then enables engine recording). Default: unsupported (nothing is captured).
    fn start_recording(&mut self, session: &RecordSession) -> Result<(), BridgeError> {
        let _ = session;
        Err(BridgeError::Unsupported(
            "recording is not available on this host".into(),
        ))
    }

    /// Live recording view (`live-record`): append the waveform peaks and MIDI notes captured
    /// since the previous call (never blocks; called from the controller tick while
    /// recording). Default: nothing (hosts without capture).
    fn poll_recording(
        &mut self,
        audio: &mut Vec<ether_core::protocol::recording::LiveAudioChunk>,
        midi: &mut Vec<ether_core::protocol::recording::LiveMidiNote>,
    ) {
        let _ = (audio, midi);
    }

    /// Finish the capture (after engine recording was disabled): close the files and return
    /// the latency-compensated takes and MIDI. Default: unsupported.
    fn stop_recording(&mut self) -> Result<RecordedTakes, BridgeError> {
        Err(BridgeError::Unsupported(
            "recording is not available on this host".into(),
        ))
    }

    /// Current plain values of a plugin device's params, read after instantiation (state
    /// load) to mirror them into the document. Hosts without plugins keep the default.
    fn plugin_param_values(&mut self, device: DeviceId) -> Vec<(ParamId, f64)> {
        let _ = device;
        Vec::new()
    }

    // ─── v0.2 (contracts-3) ───

    /// Analysis frames pushed by device nodes since the last call (`EngineHandle::
    /// poll_analysis`; CONTRACTS.md §12.4.3). Called from every tick. Native: drain the
    /// handle; web: frames forwarded from the worklet. Default: none.
    fn poll_analysis(&mut self, out: &mut Vec<ether_core::AnalysisFrame>) {
        let _ = out;
    }

    // ─── base-53: "listen on <peer>" native sender (`stream-host`; docs/COLLAB.md §9) ───

    /// What this host can do for streaming. Default: nothing (the web build streams from
    /// the UI instead, `CollabCommand::SetHosting { ui_sender: true }`).
    fn stream_capabilities(&self) -> streaming::StreamCapabilities {
        streaming::StreamCapabilities::default()
    }

    /// Start copying the engine's stream tap (master + metronome/count-in, never the
    /// preview voice: `EngineHandle::set_stream_tap`) into the sender's ring. Called
    /// when the first listener arrives; idempotent.
    fn start_stream_capture(&mut self) -> Result<(), BridgeError> {
        Err(BridgeError::Unsupported(
            "streaming is not available on this host".into(),
        ))
    }

    /// Stop the capture (after the last listener left). Idempotent.
    fn stop_stream_capture(&mut self) -> Result<(), BridgeError> {
        Ok(())
    }

    /// Open a peer connection to `listener` for `stream` using `ice` servers: the sender
    /// creates the offer and reports it (and its ICE candidates) through `poll_stream`.
    fn stream_open(
        &mut self,
        listener: SiteId,
        stream: u32,
        ice: &[IceServer],
    ) -> Result<(), BridgeError> {
        let _ = (listener, stream, ice);
        Err(BridgeError::Unsupported(
            "streaming is not available on this host".into(),
        ))
    }

    /// A signal from `listener` (its answer, trickle ICE, or `Bye`) for `stream`.
    fn stream_signal(
        &mut self,
        listener: SiteId,
        stream: u32,
        signal: &StreamSignal,
    ) -> Result<(), BridgeError> {
        let _ = (listener, stream, signal);
        Err(BridgeError::Unsupported(
            "streaming is not available on this host".into(),
        ))
    }

    /// Close the peer connection for `stream` (no `Bye` is sent by the bridge: the
    /// controller does that). Unknown streams are ignored.
    fn stream_close(&mut self, listener: SiteId, stream: u32) -> Result<(), BridgeError> {
        let _ = (listener, stream);
        Ok(())
    }

    /// Drain what the sender produced since the last call (signals, clock anchors, link
    /// states). Called from every controller tick.
    fn poll_stream(&mut self, out: &mut Vec<streaming::StreamOutput>) {
        let _ = out;
    }

    // ─── base-53: plugin GUI mirrors (`plugin-mirror`; docs/COLLAB.md §9.6) ───

    /// Instantiate a GUI-only instance of a plugin device: never routed, never processes
    /// audio, not in any graph. Its GUI edits come back through `poll_plugins` as
    /// `ParamEdited` (→ ordinary undoable, replicated `SetParam`); `OpenEditor` for a
    /// device with a mirror and no live instance opens the mirror. Default: unsupported.
    fn create_plugin_mirror(
        &mut self,
        device: DeviceId,
        plugin: &PluginInstance,
        state: Option<&Base64Bytes>,
    ) -> Result<(), BridgeError> {
        let _ = (device, plugin, state);
        Err(BridgeError::Unsupported(
            "plugin mirrors are not available on this host".into(),
        ))
    }

    /// Drop a mirror (unknown devices are ignored).
    fn destroy_plugin_mirror(&mut self, device: DeviceId) -> Result<(), BridgeError> {
        let _ = device;
        Ok(())
    }

    /// Show a document param change in a mirror's GUI (a remote `SetParam`, undo, ...).
    fn set_plugin_mirror_param(
        &mut self,
        device: DeviceId,
        param: ParamId,
        value: f64,
    ) -> Result<(), BridgeError> {
        let _ = (device, param, value);
        Err(BridgeError::Unsupported(
            "plugin mirrors are not available on this host".into(),
        ))
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
    /// Punch flag and the active record session.
    recording: recording::RecordingState,
    /// Open plugin-GUI gestures → internal gesture ids.
    plugin_gestures: BTreeMap<(DeviceId, ParamId), GestureId>,
    /// Plugin runtime bookkeeping (param mirroring after load; `plugins` module).
    plugins: plugins::PluginsState,
    /// Offline export job and finished downloads (`export` module).
    export: export::ExportState,
    /// MIDI learn runtime state (learn mode, mapping gestures; `midi_learn` module).
    midi_learn: midi_learn::MidiLearnState,
    /// Browser preview runtime state (current preview id, decode, cache; `media_preview`).
    preview: media_preview::PreviewState,
    /// Uploads from the UI machine in progress (`upload` module, remote-engine).
    uploads: upload::UploadState,
    /// Collaboration session (`collab` module).
    collab: collab::CollabState,
    /// v0.2: watched devices for the analysis channel (`analysis` module).
    analysis: analysis::AnalysisState,
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
            recording: Default::default(),
            plugin_gestures: BTreeMap::new(),
            plugins: Default::default(),
            export: Default::default(),
            midi_learn: Default::default(),
            preview: Default::default(),
            uploads: Default::default(),
            collab: Default::default(),
            analysis: Default::default(),
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

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

pub mod agent;
#[doc(hidden)]
pub mod analysis;
#[doc(hidden)]
pub mod audio_to_midi;
#[doc(hidden)]
pub mod browser;
pub mod bundle;
#[doc(hidden)]
pub mod capture;
#[doc(hidden)]
pub mod clip_editing;
pub mod compile;
#[doc(hidden)]
pub mod comping;
#[doc(hidden)]
pub mod doc;
#[doc(hidden)]
pub mod drum_rack;
#[doc(hidden)]
pub mod engine;
#[doc(hidden)]
pub mod export;
#[doc(hidden)]
pub mod expression;
pub mod ext;
#[doc(hidden)]
pub mod external;
#[doc(hidden)]
pub mod file_import;
#[doc(hidden)]
pub mod freeze;
#[doc(hidden)]
pub mod fx_space;
#[doc(hidden)]
pub mod groove;
#[doc(hidden)]
pub mod groups;
#[doc(hidden)]
pub mod handlers;
// `Project::Import` / `CancelImport` without an extension that imports other DAWs' projects:
// `Unsupported` (`import/disabled.rs`).
#[path = "import/disabled.rs"]
#[doc(hidden)]
pub mod import;
#[doc(hidden)]
pub mod keymap;
pub mod loading;
#[doc(hidden)]
pub mod media;
#[doc(hidden)]
pub mod media_preview;
#[doc(hidden)]
pub mod media_refs;
pub mod media_stream;
pub mod memory;
#[doc(hidden)]
pub mod midi_file;
#[doc(hidden)]
pub mod midi_fx;
#[doc(hidden)]
pub mod midi_learn;
#[doc(hidden)]
pub mod mpe;
#[doc(hidden)]
pub mod multisampler;
#[doc(hidden)]
pub mod patch_compat;
#[doc(hidden)]
pub mod plugins;
#[doc(hidden)]
pub mod presets;
#[doc(hidden)]
pub mod project;
// `Project::ExportTo` without an extension that exports to other DAWs: `Unsupported`
// (`project_export/disabled.rs`).
#[path = "project_export/disabled.rs"]
#[doc(hidden)]
pub mod project_export;
#[doc(hidden)]
pub mod racks;
#[doc(hidden)]
pub mod recording;
#[doc(hidden)]
pub mod resolver;
// The default sample pack (docs/SAMPLES.md).
pub mod sample_pack;
#[doc(hidden)]
pub mod saving;
#[doc(hidden)]
pub mod sidechain;
pub mod store;
#[doc(hidden)]
pub mod templates;
#[doc(hidden)]
pub mod tempo;
pub mod text;
#[doc(hidden)]
pub mod time_edit;
#[doc(hidden)]
pub mod tx;
#[doc(hidden)]
pub mod undo_history;
#[doc(hidden)]
pub mod upload;
pub mod versions;
#[doc(hidden)]
pub mod warp;

use std::collections::BTreeMap;

use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{
    Base64Bytes, BuiltinDevice, DeviceId, GestureId, History, IdGen, MediaId, MediaRef, ParamId,
    PluginInstance, Project, TrackId,
};
use ether_core::protocol::transport::TransportState;
use ether_core::protocol::{ClientMessage, Reply, ReplyResult, ServerMessage};
use ether_core::{EngineOutputs, NodeKey, ParamChange, RenderGraphDesc, TransportControl};

pub use compile::{CompileContext, compile_graph_with};
pub use doc::{ExtLabels, set_ext_labels};
pub use engine::LATENCY_REPUBLISH_MS;
pub use media::hash::content_hash;
pub use patch_compat::{LegacyClient, PatchCompat};
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

/// The installed plugins as [`EngineBridge::plugin_catalog`] reports them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PluginCatalogView {
    /// The scanned catalog (what `PluginCommand::List` replies).
    pub plugins: Vec<ether_core::protocol::plugins::PluginDescriptor>,
    /// A plugin scan is running (the list may still grow).
    pub scanning: bool,
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
    /// Suspend (`true`) or resume the engine's audio processing
    /// (`ether_core::EngineHandle::set_suspended`): while a project loads the audio thread
    /// outputs silence and runs no graph work, and what is sent meanwhile (nodes, graphs,
    /// media, transport) is applied in order before the resume. Hosts with an engine of
    /// their own implement it; the default (test bridges, engines elsewhere) does nothing.
    fn set_suspended(&mut self, suspended: bool) -> Result<(), BridgeError> {
        let _ = suspended;
        Ok(())
    }
    fn poll(&mut self, out: &mut EngineOutputs);

    /// Descriptors of built-in devices and of instantiated plugins.
    fn descriptor(&mut self, device: DeviceId) -> Option<DeviceDescriptor>;

    /// Roadmap v2 (`midi-learn`): drain incoming MIDI messages received since the last
    /// call (all ports), for MIDI mappings/learn. Called from every controller tick.
    /// Hosts without MIDI input keep the default.
    fn poll_midi_input(&mut self, out: &mut Vec<ether_core::protocol::midi_map::MidiInputEvent>) {
        let _ = out;
    }

    /// `keyboard-midi`: play one short MIDI message from the app (the computer MIDI keyboard)
    /// as live input, as if it came from a hardware port: queued to the engine's live MIDI
    /// ring (`ether_core::recording::LiveMidi`, stamped like hardware input) so monitored MIDI
    /// tracks play it and armed ones record it. Never comes back through
    /// [`Self::poll_midi_input`] (the controller feeds MIDI capture itself). Native: the MIDI
    /// input ring; web: the Worklet's ring. Default: unsupported.
    fn live_midi(&mut self, data: [u8; 3]) -> Result<(), BridgeError> {
        let _ = data;
        Err(BridgeError::Unsupported("live MIDI input".into()))
    }

    /// (`Recording::PreviewNote`): one short MIDI message for `track`'s instrument
    /// only (`EngineHandle::preview_note`), monitored or not, never recorded. Native: the
    /// engine's control ring; web: the Worklet's. Default: unsupported.
    fn preview_note(&mut self, track: TrackId, data: [u8; 3]) -> Result<(), BridgeError> {
        let _ = (track, data);
        Err(BridgeError::Unsupported("note preview".into()))
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

    /// v0.2 (`midi-fx`, CONTRACTS.md §12.4.4): push the resolved `MusicalScale` to a live
    /// built-in node (`Node::set_data` with a `Box<MusicalScale>`: Scale Quantize, Random).
    /// Native: `EngineHandle::set_node_data`; web: `EngineMsg::NodeScale` to the worklet.
    /// `Ok(false)` (the default) = unsupported or unknown device.
    fn set_node_scale(
        &mut self,
        device: DeviceId,
        scale: ether_core::protocol::model::MusicalScale,
    ) -> Result<bool, BridgeError> {
        let _ = (device, scale);
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
    /// May be slow (a call on the plugin's main thread): routine saves only read plugins
    /// that reported `StateDirty` (`crate::saving`).
    fn plugin_state(&mut self, device: DeviceId) -> Result<Option<Base64Bytes>, BridgeError> {
        let _ = device;
        Ok(None)
    }

    /// Load `state` into the running instance of plugin `device` in place (an extension's,
    /// e.g. a collaborator's preset), so its editor stays open and its audio keeps playing.
    /// `Ok(true)`: loaded; `Ok(false)`: no running instance, or the host cannot load states
    /// live (the default). The caller then re-creates the instance from the document.
    fn set_plugin_state(
        &mut self,
        device: DeviceId,
        state: &Base64Bytes,
    ) -> Result<bool, BridgeError> {
        let _ = (device, state);
        Ok(false)
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

    /// `Resolve::Replace`: the descriptor (parameter names and ranges) of an installed
    /// plugin that no device uses yet, from a short-lived private instance (never the live
    /// graph). `None` = unknown (not installed, failed to load, or no plugin host).
    fn probe_plugin(&mut self, plugin: &PluginInstance) -> Option<DeviceDescriptor> {
        let _ = plugin;
        None
    }

    /// The installed plugins (the host's scanned catalog, what
    /// `PluginCommand::List` replies) and whether a scan is running, for the agent's
    /// `list_plugins` / `add_device`. `None` (the default) = this host has no plugin host
    /// (web).
    fn plugin_catalog(&self) -> Option<PluginCatalogView> {
        None
    }

    // ─── v0.2 ───

    /// Analysis frames pushed by device nodes since the last call (`EngineHandle::
    /// poll_analysis`; CONTRACTS.md §12.4.3). Called from every tick. Native: drain the
    /// handle; web: frames forwarded from the worklet. Default: none.
    fn poll_analysis(&mut self, out: &mut Vec<ether_core::AnalysisFrame>) {
        let _ = out;
    }

    /// Start/stop collecting a node's analysis frames (`EngineHandle::watch_analysis`;
    /// driven by the watched devices). Default: nothing (hosts without the channel).
    fn watch_analysis(&mut self, node: NodeKey, on: bool) -> Result<(), BridgeError> {
        let _ = (node, on);
        Ok(())
    }

    /// The latency (samples) a live node last reported, as the next
    /// publish would use it for PDC (`EngineHandle::node_latency`). The controller tick
    /// compares it with the values of the last publish and republishes (debounced) when a
    /// node's latency changed (e.g. a Gate's Lookahead). Cheap and non-blocking. Default:
    /// `None` (latency changes then only republish through `LatencyChanged`).
    fn node_latency(&self, key: NodeKey) -> Option<u32> {
        let _ = key;
        None
    }

    // ─── plugin GUI mirrors ───

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

    // ─── v0.3 ───

    /// Audio streaming (CONTRACTS.md §13.1): stream `source.media` from disk instead of
    /// loading decoded audio (`load_media`). `Ok(true)` = the host registered a streaming
    /// `AudioSource` for it; `Ok(false)` (the default) = not supported here, the controller
    /// decodes the whole file as before.
    fn stream_media(&mut self, source: &media_stream::StreamSource) -> Result<bool, BridgeError> {
        let _ = source;
        Ok(false)
    }

    /// Media whose audio the bridge needs (again) to build a convolution reverb's IR
    /// itself, off the engine's thread, asked for since the last call. The controller
    /// decodes each again (at most the IR's length) and hands it over with
    /// [`EngineBridge::ir_media`]. Default: none (the engine reads its own media).
    fn take_ir_media_requests(&mut self) -> Vec<MediaId> {
        Vec::new()
    }

    /// Audio of a media asked for by [`EngineBridge::take_ir_media_requests`], at
    /// the engine rate, cut to the longest IR kept. The bridge keeps it only until the IRs
    /// waiting for it are delivered.
    fn ir_media(
        &mut self,
        media: MediaId,
        audio: std::sync::Arc<ether_media::DecodedAudio>,
    ) -> Result<(), BridgeError> {
        let _ = (media, audio);
        Ok(())
    }

    /// `external-instrument` (CONTRACTS.md §13.7): hardware MIDI outputs and audio channels
    /// for `External::ListPorts`. Default: unsupported (web).
    fn list_hardware_ports(
        &mut self,
    ) -> Result<ether_core::protocol::external::HardwarePorts, BridgeError> {
        Err(BridgeError::Unsupported(
            "hardware ports are not available on this host".into(),
        ))
    }

    /// `external-instrument`: start measuring the hardware round trip of an external
    /// device's node (`EngineHandle::measure_hw_latency`); the result comes back through
    /// [`EngineBridge::poll_hw_latency`]. Default: unsupported (web).
    fn measure_hw_latency(&mut self, node: NodeKey) -> Result<(), BridgeError> {
        let _ = node;
        Err(BridgeError::Unsupported(
            "latency measurement is not available on this host".into(),
        ))
    }

    /// `external-instrument`: the next finished latency measurement, if any. Called from
    /// every controller tick.
    fn poll_hw_latency(&mut self) -> Option<ether_core::hw_io::HwLatencyResult> {
        None
    }
}

/// Host services the controller needs besides the engine.
pub trait HostServices {
    /// Wall clock, Unix ms (for IDs, autosave, `modified_ms`).
    fn now_ms(&self) -> u64;
    /// Entropy for the `IdGen` seed.
    fn random_seed(&mut self) -> u64;

    /// The sample pack (docs/SAMPLES.md): start an HTTP GET of `url` as request `id`
    /// (controller-chosen), reading at most `max_bytes` of body. The answer comes back from
    /// [`Self::poll_http`]. Never blocks. Default: unsupported (no network: the built-in kit
    /// only).
    fn http_get(&mut self, id: u64, url: &str, max_bytes: u64) -> Result<(), String> {
        let _ = (id, url, max_bytes);
        Err("downloads are not available in this build".into())
    }

    /// The host's sample pack manifest URL, overriding `DEFAULT_SAMPLES_URL` when the
    /// UI doesn't name one (natively `ETHER_SAMPLES_URL`; `""` = no remote pack).
    fn samples_url(&self) -> Option<String> {
        None
    }

    /// Drain the HTTP answers that arrived since the last call (called from every tick while
    /// requests are in flight), plus their progress.
    fn poll_http(&mut self, out: &mut Vec<HttpEvent>) {
        let _ = out;
    }
}

/// An answer (or progress) of a [`HostServices::http_get`] request.
#[derive(Clone, Debug, PartialEq)]
pub enum HttpEvent {
    /// Body bytes received so far (and the total, when the server said).
    Progress {
        id: u64,
        received: u64,
        total: Option<u64>,
    },
    /// The request finished: the HTTP status and the body (any status), or a transport error
    /// (offline, DNS, TLS, the body was over `max_bytes`).
    Done {
        id: u64,
        result: Result<(u16, Vec<u8>), String>,
    },
}

/// Tunables (defaults suit a 30-60 Hz tick).
#[derive(Clone, Debug, PartialEq)]
pub struct ControllerConfig {
    /// Rate media is resampled to before `EngineBridge::load_media`.
    pub engine_sample_rate: u32,
    /// Autosave a dirty project after this long without edits (`None` = never; plugin
    /// state changes alone are then never saved by a timer either). A backstop for the
    /// UI's own quicker autosave; plugin state changes alone are saved on a slower timer
    /// (`saving::STATE_SAVE_INTERVAL_MS`).
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
    /// Whether every receiver applies `PatchChange::DeviceParam` (see
    /// [`PatchCompat`]). The host keeps a clone to register older remote clients.
    pub patch_compat: PatchCompat,
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
            patch_compat: PatchCompat::default(),
        }
    }
}

/// The open project and its edit state.
pub struct OpenDoc {
    pub project: Project,
    pub history: History,
    pub dirty: bool,
    /// Last document edit (for autosave).
    pub last_edit_ms: u64,
}

/// Runtime transport state (not in the document).
#[derive(Clone, Debug, Default)]
pub struct TransportRt {
    pub playing: bool,
    pub recording: bool,
    pub position: ether_core::protocol::model::Beats,
    pub seconds: f64,
    pub start_position: ether_core::protocol::model::Beats,
    pub last_frame: Option<ether_core::protocol::transport::PlayheadUpdate>,
    /// Engine play flag at the last poll (its transitions are adopted).
    pub last_engine_playing: Option<bool>,
    /// Recent `TapTempo` times (ms).
    pub taps: Vec<u64>,
    pub tap_gesture: Option<GestureId>,
}

/// The standard controller implementation. `X`: an extension's own state (`ext` module),
/// `()` without one.
pub struct EtherController<B, H, S, L, X = ()>
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
    pub config: ControllerConfig,
    pub ids: IdGen,
    pub doc: Option<OpenDoc>,
    /// Monotonic patch revision (across projects).
    pub revision: u64,
    pub engine: engine::EngineState,
    pub media: media::MediaState,
    pub transport: TransportRt,
    /// Record-armed tracks (runtime state, not undoable).
    pub armed: std::collections::BTreeSet<ether_core::protocol::model::TrackId>,
    /// Punch flag and the active record session.
    pub recording: recording::RecordingState,
    /// Open plugin-GUI gestures → internal gesture ids.
    pub plugin_gestures: BTreeMap<(DeviceId, ParamId), GestureId>,
    /// Plugin runtime bookkeeping (param mirroring after load; `plugins` module).
    pub plugins: plugins::PluginsState,
    /// Offline export job and finished downloads (`export` module).
    pub export: export::ExportState,
    /// MIDI learn runtime state (learn mode, mapping gestures; `midi_learn` module).
    pub midi_learn: midi_learn::MidiLearnState,
    /// Browser preview runtime state (current preview id, decode, cache; `media_preview`).
    pub preview: media_preview::PreviewState,
    /// v0.2: the library index (`browser` module).
    pub browser: browser::BrowserState,
    /// Uploads from the UI machine in progress (`upload` module, remote-engine).
    pub uploads: upload::UploadState,
    /// v0.2: watched devices for the analysis channel (`analysis` module).
    pub analysis: analysis::AnalysisState,
    /// v0.2: the time clipboard (`time_edit` module; runtime state, not undoable).
    pub time_edit: time_edit::TimeEditState,
    /// v0.3: the MIDI capture buffer (`capture` module; runtime, site-local).
    pub capture: capture::CaptureState,
    /// v0.3: the running audio-to-MIDI job (`audio_to_midi` module).
    pub audio_to_midi: audio_to_midi::AudioToMidiState,
    /// v0.3: external devices' measurements and ports (`external` module; runtime).
    pub external: external::ExternalState,
    /// v0.3: the session keymap for hosts without a writable user library (`keymap` module).
    pub keymap: keymap::KeymapState,
    /// Agent tool runtime state (shared selection, export jobs).
    pub agent: agent::AgentState,
    /// v0.3: rolling versions and the crash-recovery session marker (`versions` module).
    pub versions: versions::VersionsState,
    /// Plugin states for saving: last read per device, `StateDirty` not saved yet
    /// (`saving` module).
    pub plugin_states: saving::PluginStates,
    /// v0.3: the history panel (`undo_history` module; runtime).
    pub undo_history: undo_history::UndoHistoryState,
    /// The extensions (`ext` module): features built outside this repository.
    pub extensions: ext::Extensions<B, H, S, L, X>,
    /// Media search job and the last import's hints (`resolver`).
    pub resolver: resolver::ResolverState,
    /// The default sample pack (`sample_pack` module; runtime).
    pub samples: sample_pack::SamplesState,
    pub next_gesture: u32,
    pub last_transport: Option<TransportState>,
    /// The open project the user discarded (`ProjectCommand::Discard`): never saved again,
    /// deleted once it closes.
    pub discarded: Option<ether_core::protocol::model::ProjectId>,
    /// The project load in progress: the engine is suspended until it ends (`loading`).
    pub loading: loading::LoadState,
    pub outputs: EngineOutputs,
    /// An extension's own state (`ext` module; `()` without one).
    pub ext: X,
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

    pub fn with_config(bridge: B, host: H, store: S, library: L, config: ControllerConfig) -> Self {
        Self::with_state(bridge, host, store, library, config, ())
    }
}

impl<B, H, S, L, X> EtherController<B, H, S, L, X>
where
    B: EngineBridge,
    H: HostServices,
    S: store::ProjectStore,
    L: store::Library,
{
    /// A controller with an extension's state `ext` (`ext` module; [`Self::with_config`]
    /// otherwise).
    pub fn with_state(
        bridge: B,
        mut host: H,
        store: S,
        library: L,
        config: ControllerConfig,
        ext: X,
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
            browser: Default::default(),
            uploads: Default::default(),
            analysis: Default::default(),
            time_edit: Default::default(),
            capture: Default::default(),
            audio_to_midi: Default::default(),
            external: Default::default(),
            keymap: Default::default(),
            agent: Default::default(),
            versions: Default::default(),
            plugin_states: Default::default(),
            undo_history: Default::default(),
            extensions: Default::default(),
            resolver: Default::default(),
            samples: Default::default(),
            // Internal gestures (plugin GUI, tap tempo) live in the upper half of the id
            // space, away from UI-allocated ones.
            next_gesture: 0x8000_0000,
            last_transport: None,
            discarded: None,
            loading: Default::default(),
            outputs: EngineOutputs::default(),
            config,
            ext,
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

    /// Save the open project now, every plugin's live state included (e.g. before the host
    /// quits). Events go to `out`.
    pub fn save_now(
        &mut self,
        out: &mut dyn MessageSink,
    ) -> Result<ether_core::protocol::project::ProjectSummary, ether_core::protocol::CommandError>
    {
        self.save_current(out)
    }

    /// Wait until background autosaves are on disk (before the host quits).
    pub fn flush_saves(&mut self) {
        self.store.flush_saves();
    }

    /// Before the host quits (or anything else that drops the open project): wait for the
    /// background autosaves, then save every unsaved change, a failed background write
    /// included, with every plugin's live state. `Ok` lists the plugins whose
    /// state could not be read (their last known state was saved instead). `Err`:
    /// the project is not safely on disk, see [`Self::recovery_document`].
    pub fn save_before_close(
        &mut self,
        out: &mut dyn MessageSink,
    ) -> Result<Vec<String>, ether_core::protocol::CommandError> {
        self.plugin_states.read_failures.clear();
        self.autosave_before_switch(out)?;
        let names = self.doc.as_ref().map(|d| &d.project.devices);
        Ok(self
            .plugin_states
            .read_failures
            .iter()
            .map(|(id, e)| {
                let name = names
                    .and_then(|devices| devices.get(id))
                    .map_or_else(|| id.to_string(), |d| d.name.clone());
                format!("plugin {name}: {e}")
            })
            .collect())
    }

    /// The open project as a project document with the last known plugin states (no
    /// plugin is read), to keep elsewhere when it could not be saved to its folder.
    pub fn recovery_document(&self) -> Option<(ether_core::protocol::model::ProjectId, String)> {
        let project = &self.doc.as_ref()?.project;
        let json = ether_core::protocol::model::file::save(
            &self.with_plugin_states(project),
            &self.config.app_version,
        )
        .ok()?;
        Some((project.id, json))
    }

    /// Unsaved work in the open project: document edits, or plugin state changes no save
    /// has written yet (those don't make the document dirty: `saving` module).
    pub fn is_dirty(&self) -> bool {
        self.has_unsaved_work()
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

impl<B, H, S, L, X> Controller for EtherController<B, H, S, L, X>
where
    B: EngineBridge,
    H: HostServices,
    S: store::ProjectStore,
    L: store::Library,
{
    fn handle(&mut self, message: ClientMessage, out: &mut dyn MessageSink) {
        let now = self.host.now_ms();
        // Export events are remembered for `get_export_status`.
        let mut tap = agent::ExportTap::new(out);
        let result = self.dispatch(&message, now, &mut tap);
        self.emit_transport_if_changed(&mut tap);
        tap.send(ServerMessage::Reply(Reply {
            id: message.id,
            result: match result {
                Ok(value) => ReplyResult::Ok { value },
                Err(error) => ReplyResult::Err { error },
            },
        }));
        self.publish_if_due(now, false, &mut tap);
        self.agent_absorb(tap);
    }

    fn tick(&mut self, now_ms: u64, out: &mut dyn MessageSink) {
        // A changed node latency marks the graph dirty; the tick's
        // publish below then recomputes PDC.
        self.engine.check_latencies(&self.bridge, now_ms);
        // v0.3 `external-instrument`: latency measurements and hardware port changes.
        self.external_tick(now_ms, out);
        // Extensions' background work (e.g. a project import from another DAW).
        self.ext_tick(now_ms, out);
        let mut tap = agent::ExportTap::new(out);
        self.tick_impl(now_ms, &mut tap);
        self.agent_absorb(tap);
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

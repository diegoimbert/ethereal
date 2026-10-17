//! [`NativeHost`]: owns the threads of the desktop runtime and routes messages.
//!
//! ```text
//!  Tauri command ──ClientMessage──▶ controller thread ──ServerMessage──▶ subscriber
//!                                    │  Controller::handle / tick (60 Hz)   (Tauri channels)
//!                                    │  host-handled: Engine::*, Plugin::{Rescan,List,
//!                                    │                OpenEditor,CloseEditor,*Folder*}
//!                                    ▼
//!                                 NativeBridge ── EngineHandle ─rings─▶ audio thread (cpal
//!                                    │                                  callback / null thread)
//!                                    └─ PluginHost ─▶ main thread (plugin controllers)
//!                                 GC thread: GarbageCollector::collect every 20 ms
//! ```
//!
//! - One controller thread owns the `Controller`, the audio backend and the engine
//!   handle; commands are processed strictly in order, so "patches before the reply"
//!   holds end to end.
//! - Meters are coalesced to ~30 Hz (max-held); playhead frames go out as produced (~60 Hz).
//! - A panicking controller never takes the host down: the message gets an `Internal`
//!   error reply and the host keeps running.

use ether_controller::text::{Msg, msg};
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, bounded, unbounded};
use ether_controller::{Controller, ControllerConfig, EtherController, MessageSink, PatchCompat};
use ether_core::parallel::ParallelExecutor as _;
use ether_core::protocol::engine::{EngineCommand, EngineEvent, EngineStatus};
use ether_core::protocol::message::{
    Command, CommandError, ErrorCode, Event, NotificationLevel, Reply, ReplyResult, ReplyValue,
};
use ether_core::protocol::meters::MeterFrame;
use ether_core::protocol::model::Project;
use ether_core::protocol::plugins::{PluginCommand, PluginEvent};
use ether_core::protocol::{ClientMessage, ServerMessage};
use ether_core::{Engine, EngineConfig, PrepareConfig};

use crate::audio::{self, AudioBackendKind, AudioOutput, AudioSettings, StreamInfo};
use crate::bridge::{NativeBridge, NativeServices};
use crate::health::{Health, Watchdog, Work};
use crate::plugin_folders::{FolderError, PluginFolderSettings};
use crate::plugins::{DedicatedThread, Instantiate, MainThread, PluginCatalog, PluginHost};
use crate::rt::AudioShared;
use crate::store::{DiskStore, LibraryRoot};

mod device;

/// Controller tick period (~60 Hz: playhead rate).
pub const TICK: Duration = Duration::from_millis(16);
/// Minimum interval between meter frames (~30 Hz).
pub const METER_INTERVAL: Duration = Duration::from_millis(33);
/// GC thread period.
pub const GC_INTERVAL: Duration = Duration::from_millis(20);

/// How long quitting waits for the open project to be saved while the plugins still answer
/// (#566). Past it, plugin calls fail and the save goes on with their last known states.
pub const QUIT_SAVE_TIMEOUT: Duration = Duration::from_secs(30);

/// Receives every `ServerMessage` for the UI (called on the controller thread).
pub type Subscriber = Arc<dyn Fn(ServerMessage) + Send + Sync>;

/// A controller as run by the native host: the [`Controller`] contract plus host
/// lifecycle hooks (defaults do nothing, so test controllers only implement `Controller`).
///
/// The engine's sample rate is fixed for the host's lifetime (a rate change is applied at
/// the next start, see `AudioState::reconfigure`), so the controller is configured with it
/// once, at construction.
pub trait HostedController: Controller {
    /// The host is quitting: persist unsaved work (the plugins still answer). Events go to
    /// `out`. A project that could not be saved is returned: the host keeps it elsewhere
    /// ([`recovered_dir`]) and says so at the next start.
    fn before_shutdown(&mut self, out: &mut dyn MessageSink) -> Option<UnsavedProject> {
        let _ = out;
        None
    }

    /// The UI went away (disconnected, or replaced by a webview reload): drop the
    /// per-connection state it left behind (its analysis watches).
    fn ui_disconnected(&mut self) {}

    /// The local engine is off ("Play with: no engine", or a participant's engine): the
    /// host closes the audio device until it is on again (checked after every step).
    fn local_engine_off(&self) -> bool {
        false
    }
}

/// The standard native controller.
pub type NativeController = EtherController<NativeBridge, NativeServices, DiskStore, DiskStore>;

/// A project [`HostedController::before_shutdown`] could not save to its folder.
#[derive(Debug, Clone, PartialEq)]
pub struct UnsavedProject {
    /// The project id (the kept file is named after it).
    pub id: String,
    /// The project document (`project.ether`).
    pub document: String,
    /// Why it could not be saved.
    pub error: String,
}

impl HostedController for NativeController {
    fn before_shutdown(&mut self, out: &mut dyn MessageSink) -> Option<UnsavedProject> {
        // Waits for background autosaves first and saves again one that failed (#561).
        match self.save_before_close(out) {
            Ok(unread) => {
                for w in unread {
                    tracing::warn!("saved on quit with the last known plugin state: {w}");
                }
                None
            }
            Err(e) => {
                tracing::error!(error = %e.message, "failed to save the project on quit");
                let (id, document) = self.recovery_document()?;
                Some(UnsavedProject {
                    id: id.to_string(),
                    document,
                    error: e.message,
                })
            }
        }
    }

    fn ui_disconnected(&mut self) {
        self.reset_analysis_watches();
    }

    fn local_engine_off(&self) -> bool {
        EtherController::local_engine_off(self)
    }
}

/// Builds the controller on the controller thread.
pub type ControllerFactory = Box<
    dyn FnOnce(NativeBridge, NativeServices, DiskStore, DiskStore) -> Box<dyn HostedController>
        + Send,
>;

/// Controller tunables for a native engine running at `sample_rate`.
pub fn native_controller_config(sample_rate: u32) -> ControllerConfig {
    ControllerConfig {
        engine_sample_rate: sample_rate,
        ..ControllerConfig::default()
    }
}

/// The standard controller (`ether_controller::EtherController`), configured for the
/// engine's sample rate.
pub fn ether_controller() -> ControllerFactory {
    ether_controller_with(PatchCompat::default())
}

/// [`ether_controller`] whose patch shapes follow `patch_compat` (audit #4: a remote
/// server registers clients that predate `PatchChange::DeviceParam` there).
pub fn ether_controller_with(patch_compat: PatchCompat) -> ControllerFactory {
    Box::new(move |bridge, services, store, library| {
        let config = ControllerConfig {
            patch_compat,
            ..native_controller_config(bridge.sample_rate())
        };
        Box::new(EtherController::with_config(
            bridge, services, store, library, config,
        ))
    })
}

#[derive(Clone, Debug)]
pub struct HostConfig {
    /// Audio settings. `None`: load `<data_dir>/config/audio.json` (defaults if missing);
    /// `ETHER_AUDIO` overrides the backend either way.
    pub audio: Option<AudioSettings>,
    /// Per-instance app data dir (`config/`, `plugin-db/`, ...).
    pub data_dir: PathBuf,
    pub instance: String,
    /// Root of the engine-side project store (see [`crate::default_projects_root`]).
    pub projects_root: PathBuf,
    /// Sample library folders exposed to the browser.
    pub library_roots: Vec<LibraryRoot>,
}

/// Pluggable parts of the host (the Tauri app passes its main-thread executor).
pub struct HostOptions {
    pub main_thread: Arc<dyn MainThread>,
    pub instantiate: Instantiate,
    pub controller: ControllerFactory,
    /// The switch `controller` reads to pick patch shapes ([`NativeHost::patch_compat`]).
    /// A custom `controller` should be built with a clone of it.
    pub patch_compat: PatchCompat,
}

impl Default for HostOptions {
    fn default() -> Self {
        let patch_compat = PatchCompat::default();
        Self {
            main_thread: Arc::new(DedicatedThread::new()),
            instantiate: crate::plugins::instantiate_any(),
            controller: ether_controller_with(patch_compat.clone()),
            patch_compat,
        }
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum HostError {
    #[error("host is shut down")]
    Stopped,
    #[error("invalid message: {0}")]
    InvalidMessage(String),
    #[error("{0}")]
    Start(String),
}

enum HostMsg {
    Client(ClientMessage),
    /// A message produced off the controller thread (plugin scan progress).
    Emit(ServerMessage),
    /// [`NativeHost::ui_disconnected`].
    UiDisconnected,
    /// Quitting: save now, while the plugins still answer, then reply (#566).
    SaveBeforeQuit(Sender<()>),
    Shutdown,
}

/// Where projects that could not be saved on quit are kept: `<data_dir>/recovered`.
pub fn recovered_dir(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join("recovered")
}

/// Lists the copies kept by [`keep_unsaved`] that the user wasn't told about yet.
const RECOVERED_NOTICE: &str = "unreported.txt";

/// Keep a project that could not be saved on quit in [`recovered_dir`]; the next start
/// reports it ([`recovered_warning`]).
fn keep_unsaved(data_dir: &std::path::Path, p: &UnsavedProject) {
    let dir = recovered_dir(data_dir);
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let file = dir.join(format!("{}-{secs}.ether", p.id));
    if let Err(e) = crate::store::atomic_write(&file, p.document.as_bytes()) {
        tracing::error!(%e, error = %p.error, "the unsaved project could not be kept either");
        return;
    }
    tracing::warn!(file = %file.display(), error = %p.error, "kept the unsaved project");
    let notice = dir.join(RECOVERED_NOTICE);
    let mut listed = std::fs::read_to_string(&notice).unwrap_or_default();
    listed.push_str(&format!("{}\n", file.display()));
    if let Err(e) = crate::store::atomic_write(&notice, listed.as_bytes()) {
        tracing::warn!(%e, "failed to note the kept project");
    }
}

/// The warning about projects [`keep_unsaved`] kept since the last start (reported once).
fn recovered_warning(data_dir: &std::path::Path) -> Option<Msg> {
    let notice = recovered_dir(data_dir).join(RECOVERED_NOTICE);
    let listed = std::fs::read_to_string(&notice).ok()?;
    let _ = std::fs::remove_file(&notice);
    let files: Vec<&str> = listed.lines().filter(|l| !l.trim().is_empty()).collect();
    (!files.is_empty()).then(|| {
        msg("notice.audio.recovered_copy")
            .arg("files", files.join(", "))
            .into()
    })
}

/// The running native host. Methods are callable from any thread (Tauri commands).
pub struct NativeHost {
    tx: Sender<HostMsg>,
    subscriber: Arc<Mutex<Option<Subscriber>>>,
    controller_thread: Option<JoinHandle<()>>,
    gc_stop: Arc<AtomicBool>,
    gc_thread: Option<JoinHandle<()>>,
    /// Reports controller stalls (`crate::health`); stopped before the controller.
    watchdog: Option<Watchdog>,
    info: StreamInfo,
    plugins: PluginHost,
    patch_compat: PatchCompat,
    /// The 1 ms Windows timer for the host's lifetime (plugins wait in `Sleep(1)` loops).
    _timer: crate::timer_resolution::HighResolutionTimer,
}

impl std::fmt::Debug for NativeHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeHost")
            .field("audio", &self.info)
            .finish()
    }
}

fn settings_file(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join("config").join("audio.json")
}

/// Load persisted audio settings (the user's choice; `ETHER_AUDIO` is applied only to the
/// running backend, see [`env_backend`], never saved).
pub fn load_audio_settings(data_dir: &std::path::Path) -> AudioSettings {
    std::fs::read_to_string(settings_file(data_dir))
        .ok()
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default()
}

/// The backend forced by `ETHER_AUDIO`, if set.
fn env_backend() -> Option<AudioBackendKind> {
    AudioBackendKind::parse(std::env::var("ETHER_AUDIO").ok().as_deref())
}

/// Engine settings persisted next to the audio settings (`<data_dir>/config/engine.json`).
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct EngineSettings {
    /// Audio worker threads (`EngineConfig::worker_threads`); `None` = the default.
    worker_threads: Option<usize>,
}

/// Worker threads for the engine: `ETHER_WORKERS`, else `worker_threads` of
/// `<data_dir>/config/engine.json`, else [`crate::workers::default_workers`] with a real
/// device, and [`HEADLESS_WORKERS`] with the `null`/`offline` backends (tests, CI and
/// parallel dev instances share machines: a few workers still exercise the parallel path).
pub fn worker_threads(data_dir: &std::path::Path, backend: AudioBackendKind) -> usize {
    if let Some(n) = crate::workers::env_workers() {
        return n;
    }
    std::fs::read_to_string(data_dir.join("config").join("engine.json"))
        .ok()
        .and_then(|j| serde_json::from_str::<EngineSettings>(&j).ok())
        .and_then(|s| s.worker_threads)
        .unwrap_or_else(|| match backend {
            AudioBackendKind::Cpal => crate::workers::default_workers(),
            _ => HEADLESS_WORKERS,
        })
}

/// Default worker count without an audio device (see [`worker_threads`]).
pub const HEADLESS_WORKERS: usize = 1;

fn save_audio_settings(data_dir: &std::path::Path, s: &AudioSettings) {
    if let Ok(json) = serde_json::to_string_pretty(s)
        && let Err(e) = crate::store::atomic_write(&settings_file(data_dir), json.as_bytes())
    {
        tracing::warn!(%e, "failed to save audio settings");
    }
}

fn start_failed(e: &dyn std::fmt::Display) -> Msg {
    msg("notice.audio.start_failed")
        .arg("error", e.to_string())
        .into()
}

/// Start `settings` or fall back to the null backend (so the app runs without a device).
fn start_audio(
    engine: Box<Engine>,
    settings: &AudioSettings,
    shared: &Arc<AudioShared>,
) -> (Result<AudioOutput, Box<Engine>>, Option<Msg>) {
    match AudioOutput::start(engine, settings, shared.clone()) {
        Ok(out) => (Ok(out), None),
        Err((e, engine)) if settings.backend == AudioBackendKind::Cpal => {
            let warning: Msg = msg("notice.audio.device_unavailable")
                .arg("error", e.to_string())
                .into();
            tracing::warn!("{}", warning.text);
            let null = AudioSettings {
                backend: AudioBackendKind::Null,
                ..settings.clone()
            };
            match AudioOutput::start(engine, &null, shared.clone()) {
                Ok(out) => (Ok(out), Some(warning)),
                Err((e, engine)) => (Err(engine), Some(start_failed(&e))),
            }
        }
        Err((e, engine)) => (Err(engine), Some(start_failed(&e))),
    }
}

impl NativeHost {
    /// Create the engine, start audio, GC and controller threads.
    pub fn start(config: HostConfig, options: HostOptions) -> Result<Self, HostError> {
        // Before any plugin loads (see `timer_resolution`).
        let timer = crate::timer_resolution::HighResolutionTimer::acquire();
        // `settings` stay what the user chose (they are saved again by `SetAudioConfig`);
        // a missing device only makes this run use the null backend (`run_settings`), so a
        // temporary fallback never becomes the saved choice.
        // `ETHER_AUDIO` forces the backend of settings loaded from disk (not of explicit ones).
        let (settings, forced_backend) = match config.audio.clone() {
            Some(s) => (s, None),
            None => (load_audio_settings(&config.data_dir), env_backend()),
        };
        let mut run_settings = settings.clone();
        if let Some(b) = forced_backend {
            run_settings.backend = b;
        }
        let resolved = match audio::resolve(&run_settings) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(%e, "audio device unavailable; using the null backend");
                run_settings.backend = AudioBackendKind::Null;
                audio::resolve(&run_settings).map_err(|e| HostError::Start(e.to_string()))?
            }
        };
        let engine_config = EngineConfig {
            sample_rate: resolved.info.sample_rate,
            max_block_size: settings.max_block_size.max(16),
            worker_threads: worker_threads(&config.data_dir, settings.backend),
            // Every channel of a multi-channel interface (`crate::recording`).
            input_channels: crate::recording::MAX_INPUT_CHANNELS as u16,
            ..Default::default()
        };
        let prepare = PrepareConfig {
            sample_rate: engine_config.sample_rate as f32,
            max_block_size: engine_config.max_block_size,
            max_events_per_block: engine_config.max_events_per_block,
        };
        let mut parts = ether_core::create(engine_config.clone());
        if engine_config.worker_threads > 0 {
            // Multicore (`crate::workers`): independent tracks run on RT worker threads.
            let block = std::time::Duration::from_secs_f64(
                engine_config.max_block_size as f64 / engine_config.sample_rate.max(1) as f64,
            );
            // Real-time priority only with a real device (headless hosts share machines).
            let pool = crate::workers::WorkerPool::with_options(
                engine_config.worker_threads,
                crate::workers::PoolOptions {
                    period: block,
                    realtime: settings.backend == AudioBackendKind::Cpal,
                    flush_denormals: true,
                },
            );
            tracing::info!(workers = pool.workers(), "audio worker pool started");
            parts.engine.set_executor(Box::new(pool));
        }
        // Complex warp (feature `warp-complex`); without a stretcher Complex clips play
        // Repitch (as on the web).
        #[cfg(feature = "warp-complex")]
        parts
            .handle
            .set_stretcher_factory(Arc::new(ether_stretch::SignalsmithFactory::default()));
        let shared = Arc::new(AudioShared::default());
        shared
            .recording
            .set_projects_root(config.projects_root.clone());
        let (started, warning) = start_audio(Box::new(parts.engine), &run_settings, &shared);
        let (output, parked) = match started {
            Ok(out) => (Some(out), None),
            Err(engine) => (None, Some(engine)),
        };
        let info = output
            .as_ref()
            .map(|o| o.info.clone())
            .unwrap_or(resolved.info);

        // GC thread: drops everything the audio thread retired.
        let gc_stop = Arc::new(AtomicBool::new(false));
        let gc_thread = {
            let stop = gc_stop.clone();
            let mut gc = parts.gc;
            std::thread::Builder::new()
                .name("ether-gc".into())
                .spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        gc.collect();
                        std::thread::sleep(GC_INTERVAL);
                    }
                    gc.collect();
                })
                .map_err(|e| HostError::Start(e.to_string()))?
        };

        let (tx, rx) = unbounded();
        let subscriber: Arc<Mutex<Option<Subscriber>>> = Arc::default();
        let plugins = PluginHost::new(options.main_thread);
        let plugin_db = config.data_dir.join("plugin-db");
        let catalog = PluginCatalog::open(&plugin_db);
        let plugin_folders = PluginFolderSettings::open(&plugin_db);
        let store = DiskStore::new(config.projects_root.clone(), config.library_roots.clone())
            .with_sample_cache(Some(config.data_dir.join("samples")));
        let mut bridge = NativeBridge::new(
            parts.handle,
            prepare,
            plugins.clone(),
            catalog.clone(),
            options.instantiate,
            shared.clone(),
        );
        let scanning = Arc::new(AtomicBool::new(false));
        bridge.set_scan_flag(scanning.clone());
        let factory = options.controller;
        let patch_compat = options.patch_compat;
        let health = plugins.health().clone();
        let ctx = ControllerThread {
            health: health.clone(),
            rx,
            tx: tx.clone(),
            router: Router::new(subscriber.clone()),
            audio: {
                let mut audio = AudioState::new(
                    output,
                    parked,
                    settings,
                    forced_backend,
                    shared,
                    prepare.sample_rate as u32,
                    config.data_dir.clone(),
                    config.instance.clone(),
                );
                // No device at startup: audio comes on when it is plugged in.
                let want = audio.run_settings(&audio.settings);
                audio.watch_fallback(&want, Instant::now());
                audio
            },
            plugins: plugins.clone(),
            catalog,
            scanning,
            rescan_pending: Arc::new(AtomicU8::new(RESCAN_NONE)),
            scan_db: Arc::new(Mutex::new(())),
            plugin_folders,
            plugin_db,
            startup_warnings: warning
                .into_iter()
                .chain(recovered_warning(&config.data_dir))
                .collect(),
        };
        let controller_thread = std::thread::Builder::new()
            .name("ether-controller".into())
            .spawn(move || {
                let controller = match std::panic::catch_unwind(AssertUnwindSafe(|| {
                    factory(bridge, NativeServices::default(), store.clone(), store)
                })) {
                    Ok(c) => c,
                    Err(p) => {
                        let msg = panic_message(&p);
                        tracing::error!(%msg, "controller failed to start");
                        Box::new(Unavailable(msg))
                    }
                };
                ctx.run(controller);
            })
            .map_err(|e| HostError::Start(e.to_string()))?;
        let watchdog = {
            let router = Router::new(subscriber.clone());
            Watchdog::start(health, move |m| router.deliver(m))
                .map_err(|e| HostError::Start(e.to_string()))?
        };

        Ok(Self {
            tx,
            subscriber,
            controller_thread: Some(controller_thread),
            gc_stop,
            gc_thread: Some(gc_thread),
            watchdog: Some(watchdog),
            info,
            plugins,
            patch_compat,
            _timer: timer,
        })
    }

    /// Where remote clients that predate `PatchChange::DeviceParam` register (audit #4).
    pub fn patch_compat(&self) -> &PatchCompat {
        &self.patch_compat
    }

    /// The audio stream the host started with.
    pub fn audio_info(&self) -> &StreamInfo {
        &self.info
    }

    /// Queue a client message (never blocks).
    pub fn send(&self, message: ClientMessage) -> Result<(), HostError> {
        self.tx
            .send(HostMsg::Client(message))
            .map_err(|_| HostError::Stopped)
    }

    /// Queue a JSON-encoded `ClientMessage`.
    pub fn send_json(&self, message_json: &str) -> Result<(), HostError> {
        let msg = serde_json::from_str(message_json)
            .map_err(|e| HostError::InvalidMessage(e.to_string()))?;
        self.send(msg)
    }

    /// Set (or replace, e.g. after a UI reload) the receiver of all `ServerMessage`s.
    pub fn subscribe(&self, subscriber: Subscriber) {
        if let Ok(mut s) = self.subscriber.lock() {
            *s = Some(subscriber);
        }
    }

    /// The UI went away (disconnected, or replaced by a webview reload re-subscribing): the
    /// controller forgets its per-connection state (analysis watches), in order with the
    /// queued messages.
    pub fn ui_disconnected(&self) -> Result<(), HostError> {
        self.tx
            .send(HostMsg::UiDisconnected)
            .map_err(|_| HostError::Stopped)
    }

    pub fn unsubscribe(&self) {
        if let Ok(mut s) = self.subscriber.lock() {
            *s = None;
        }
    }

    /// Stop the controller (and audio) and GC threads.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        // Saving on quit may take a while: not a stall worth reporting.
        drop(self.watchdog.take());
        // Save first, while the plugins still answer (#566): their live states are read on
        // the main thread, and `stop` may run ON it (Tauri's exit), so it serves those calls
        // while it waits. (Already stopped: the send fails.)
        let (done_tx, done_rx) = bounded(1);
        if self.tx.send(HostMsg::SaveBeforeQuit(done_tx)).is_ok() {
            let deadline = Instant::now() + QUIT_SAVE_TIMEOUT;
            loop {
                self.plugins.run_pending_main();
                match done_rx.recv_timeout(Duration::from_millis(2)) {
                    Err(RecvTimeoutError::Timeout) if Instant::now() < deadline => {}
                    Err(RecvTimeoutError::Timeout) => {
                        tracing::warn!(
                            "saving on quit: the plugins don't answer, saving without them"
                        );
                        break;
                    }
                    _ => break,
                }
            }
        }
        // Main-thread plugin calls fail fast from now on: the main thread may be this one,
        // which then can't serve the controller thread we are about to join.
        self.plugins.close();
        let _ = self.tx.send(HostMsg::Shutdown);
        if let Some(t) = self.controller_thread.take() {
            let _ = t.join();
        }
        self.gc_stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.gc_thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for NativeHost {
    fn drop(&mut self) {
        self.stop();
    }
}

fn panic_message(p: &Box<dyn std::any::Any + Send>) -> String {
    p.downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic".into())
}

fn reply_ok(id: u32, value: ReplyValue) -> ServerMessage {
    ServerMessage::Reply(Reply {
        id,
        result: ReplyResult::Ok { value },
    })
}

fn reply_err(id: u32, code: ErrorCode, message: impl Into<String>) -> ServerMessage {
    ServerMessage::Reply(Reply {
        id,
        result: ReplyResult::Err {
            error: CommandError {
                code,
                message: message.into(),
                key: None,
            },
        },
    })
}

fn status_event(status: EngineStatus) -> ServerMessage {
    ServerMessage::Event(Event::Engine {
        event: EngineEvent::Status { status },
    })
}

fn notification(
    level: NotificationLevel,
    message: impl Into<ether_controller::text::Msg>,
) -> ServerMessage {
    let m = message.into();
    ServerMessage::Event(Event::Notification {
        level,
        message: m.text,
        key: m.key,
    })
}

/// Stand-in when the controller could not be constructed: every command fails cleanly.
struct Unavailable(String);

impl HostedController for Unavailable {}

impl Controller for Unavailable {
    fn handle(&mut self, message: ClientMessage, out: &mut dyn MessageSink) {
        out.send(reply_err(
            message.id,
            ErrorCode::Internal,
            format!("controller unavailable: {}", self.0),
        ));
    }
    fn tick(&mut self, _now_ms: u64, _out: &mut dyn MessageSink) {}
    fn project(&self) -> Option<&Project> {
        None
    }
}

/// Forwards messages to the subscriber; coalesces meters to [`METER_INTERVAL`].
struct Router {
    subscriber: Arc<Mutex<Option<Subscriber>>>,
    pending_meters: Option<MeterFrame>,
    last_meters: Instant,
    /// Id of the last reply delivered (to detect a missing reply after a panic).
    last_reply: Option<u32>,
}

impl Router {
    fn new(subscriber: Arc<Mutex<Option<Subscriber>>>) -> Self {
        Self {
            subscriber,
            pending_meters: None,
            last_meters: Instant::now(),
            last_reply: None,
        }
    }

    fn deliver(&self, m: ServerMessage) {
        let sub = self.subscriber.lock().ok().and_then(|s| s.clone());
        if let Some(sub) = sub {
            sub(m);
        }
    }

    fn flush_meters(&mut self, now: Instant) {
        if now.duration_since(self.last_meters) >= METER_INTERVAL
            && let Some(frame) = self.pending_meters.take()
        {
            self.last_meters = now;
            self.deliver(ServerMessage::Meters(frame));
        }
    }
}

impl MessageSink for Router {
    fn send(&mut self, message: ServerMessage) {
        match message {
            ServerMessage::Meters(frame) => match &mut self.pending_meters {
                Some(p) => {
                    for t in frame.tracks {
                        match p.tracks.iter_mut().find(|e| e.track == t.track) {
                            Some(e) => {
                                for ch in 0..2 {
                                    e.peak[ch] = e.peak[ch].max(t.peak[ch]);
                                    e.rms[ch] = e.rms[ch].max(t.rms[ch]);
                                }
                                e.clipped |= t.clipped;
                            }
                            None => p.tracks.push(t),
                        }
                    }
                    p.cpu_load = p.cpu_load.max(frame.cpu_load);
                }
                None => self.pending_meters = Some(frame),
            },
            m => {
                if let ServerMessage::Reply(r) = &m {
                    self.last_reply = Some(r.id);
                }
                self.deliver(m);
            }
        }
    }
}

struct AudioState {
    output: Option<AudioOutput>,
    /// The engine while no backend runs it (every start attempt failed); kept so a later
    /// `SetAudioConfig` can start it again.
    parked: Option<Box<Engine>>,
    /// The user's choice (saved). What runs may differ: `forced_backend`, or the null
    /// backend when the device was unavailable at startup.
    settings: AudioSettings,
    /// `ETHER_AUDIO`: the backend every start uses, whatever the settings say.
    forced_backend: Option<AudioBackendKind>,
    shared: Arc<AudioShared>,
    engine_rate: u32,
    data_dir: PathBuf,
    instance: String,
    last_xruns: u32,
    /// The device is closed while the local engine is off (docs/COLLAB.md §15): the engine
    /// runs on the null backend, which holds no device and keeps draining its queues (the
    /// controller's teardown graph, transport commands), until the engine is on again.
    released: bool,
    /// The chosen output device is gone and a fallback runs ([`device`]).
    lost: Option<device::Lost>,
    /// Callback progress, to notice a stream that died without saying so ([`device`]).
    stall: device::StallWatch,
    /// The last render crash ([`device`]).
    last_crash: Option<Instant>,
}

impl AudioState {
    #[allow(clippy::too_many_arguments)]
    fn new(
        output: Option<AudioOutput>,
        parked: Option<Box<Engine>>,
        settings: AudioSettings,
        forced_backend: Option<AudioBackendKind>,
        shared: Arc<AudioShared>,
        engine_rate: u32,
        data_dir: PathBuf,
        instance: String,
    ) -> Self {
        Self {
            output,
            parked,
            settings,
            forced_backend,
            shared,
            engine_rate,
            data_dir,
            instance,
            last_xruns: 0,
            released: false,
            lost: None,
            stall: device::StallWatch::new(Instant::now()),
            last_crash: None,
        }
    }

    fn status(&self) -> EngineStatus {
        let info = self.output.as_ref().map(|o| &o.info);
        let buffer = self.shared.buffer_size.load(Ordering::Relaxed);
        let buffer = if buffer > 0 {
            buffer
        } else {
            info.map_or(0, |i| i.buffer_size)
        };
        EngineStatus {
            running: self.output.is_some() && self.shared.running.load(Ordering::Relaxed),
            backend: info.map_or("none", |i| i.backend.name()).to_string(),
            sample_rate: self.engine_rate,
            buffer_size: buffer,
            // cpal doesn't expose device latency portably: report one buffer.
            output_latency: self.shared.recording.output_latency_or(buffer),
            input_latency: self.shared.recording.input_latency(),
            xruns: self.shared.xruns.load(Ordering::Relaxed),
            instance: self.instance.clone(),
            device_released: self.released,
        }
    }

    /// What a start runs for the user's `settings`: the engine's rate, `ETHER_AUDIO`'s
    /// backend if set, and no device while it is released.
    fn run_settings(&self, settings: &AudioSettings) -> AudioSettings {
        let backend = if self.released {
            AudioBackendKind::Null
        } else {
            self.forced_backend.unwrap_or(settings.backend)
        };
        AudioSettings {
            sample_rate: Some(self.engine_rate),
            backend,
            ..settings.clone()
        }
    }

    /// Close the device (`true`: the engine moves to the null backend) or open the saved
    /// one again. A device that can't be opened any more falls back to the null backend,
    /// as at startup, and the warning to show is returned.
    fn set_released(&mut self, released: bool) -> Option<Msg> {
        if released == self.released {
            return None;
        }
        self.released = released;
        let run = self.run_settings(&self.settings);
        // Already running what is asked (the null backend, chosen or forced). A startup
        // that fell back to null is not: opening again retries the device.
        if self
            .output
            .as_ref()
            .is_some_and(|o| o.info.backend == run.backend)
        {
            return None;
        }
        let engine = match self.output.take() {
            Some(out) => out.stop(),
            None => self.parked.take(),
        };
        let Some(engine) = engine else {
            return Some(msg("notice.audio.engine_lost").into());
        };
        let (started, warning) = start_audio(engine, &run, &self.shared);
        match started {
            Ok(out) => self.output = Some(out),
            Err(engine) => self.parked = Some(engine),
        }
        self.watch_fallback(&run, Instant::now());
        warning
    }

    /// Switch device/backend/buffer size. The engine moves over; a sample-rate change needs
    /// a new engine, so it is saved and applied at the next start.
    fn reconfigure(&mut self, new: AudioSettings) -> Result<Option<Msg>, String> {
        save_audio_settings(&self.data_dir, &new);
        let mut note = None;
        if new.sample_rate.is_some_and(|r| r != self.engine_rate) {
            note = Some(
                msg("notice.audio.rate_after_restart")
                    .arg("rate", i64::from(new.sample_rate.unwrap_or_default()))
                    .into(),
            );
        }
        // While the device is released this only saves the choice (the null backend
        // restarts with it); it opens when the engine is on again.
        let run = self.run_settings(&new);
        let engine = match self.output.take() {
            Some(out) => out.stop(),
            None => self.parked.take(),
        };
        let Some(engine) = engine else {
            return Err("audio engine is lost; restart Ethereal".into());
        };
        match AudioOutput::start(engine, &run, self.shared.clone()) {
            Ok(out) => {
                self.output = Some(out);
                self.settings = new;
                self.lost = None;
                Ok(note)
            }
            Err((e, engine)) => {
                // Restore the previous configuration.
                let old = self.run_settings(&self.settings);
                match start_audio(engine, &old, &self.shared).0 {
                    Ok(out) => self.output = Some(out),
                    Err(engine) => self.parked = Some(engine),
                }
                self.watch_fallback(&old, Instant::now());
                save_audio_settings(&self.data_dir, &self.settings);
                Err(e.to_string())
            }
        }
    }
}

/// A controller step longer than this is logged (the playhead, meters and every queued
/// command wait for it).
const SLOW_STEP: Duration = Duration::from_millis(100);

fn log_slow_step(step: &str, what: Option<&str>, took: Duration) {
    if took >= SLOW_STEP {
        tracing::warn!(
            step,
            what = what.unwrap_or("-"),
            ms = took.as_millis() as u64,
            "slow controller step"
        );
    }
}

/// The start of a command's `Debug` form (e.g. `Device(SetParam { device: ..`), for logs.
/// Formatting stops at the limit, so a big payload costs nothing.
fn command_label(msg: &ClientMessage) -> String {
    struct Capped(String);
    impl std::fmt::Write for Capped {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            let room = 80usize.saturating_sub(self.0.len());
            if s.len() <= room {
                self.0.push_str(s);
                Ok(())
            } else {
                let cut = (0..=room)
                    .rev()
                    .find(|&i| s.is_char_boundary(i))
                    .unwrap_or(0);
                self.0.push_str(&s[..cut]);
                Err(std::fmt::Error)
            }
        }
    }
    let mut out = Capped(String::new());
    let _ = std::fmt::write(&mut out, format_args!("{:?}", msg.command));
    out.0
}

struct ControllerThread {
    /// Marks each step (command, tick) for the watchdog.
    health: Arc<Health>,
    rx: Receiver<HostMsg>,
    tx: Sender<HostMsg>,
    router: Router,
    audio: AudioState,
    plugins: PluginHost,
    catalog: PluginCatalog,
    scanning: Arc<AtomicBool>,
    /// A rescan asked for while one runs: [`RESCAN_NONE`], [`RESCAN_INCREMENTAL`] or
    /// [`RESCAN_FULL`]; the running scan starts another when it ends.
    rescan_pending: Arc<AtomicU8>,
    /// Held while the scan cache and the plugin list are read and rewritten (a whole scan,
    /// or applying one retried target), so a retry ending mid-scan isn't lost.
    scan_db: Arc<Mutex<()>>,
    plugin_folders: PluginFolderSettings,
    /// Plugin list, folder settings and scan cache live here.
    plugin_db: PathBuf,
    /// Shown once, at the first message from the UI.
    startup_warnings: Vec<Msg>,
}

impl ControllerThread {
    fn run(mut self, mut controller: Box<dyn HostedController>) {
        crate::health::set_controller_thread();
        let start = Instant::now();
        let mut next_tick = start;
        let mut tick_panicked = false;
        let mut saved_on_quit = false;
        'run: loop {
            let now = Instant::now();
            match self
                .rx
                .recv_timeout(next_tick.saturating_duration_since(now))
            {
                Ok(msg) => {
                    // Commands go first: handle everything queued before the next tick, so
                    // a busy tick (plugin loads, big projects) delays play/stop by at most
                    // one tick instead of one tick per queued message.
                    let mut next = Some(msg);
                    while let Some(msg) = next.take() {
                        match msg {
                            HostMsg::Client(msg) => {
                                let health = self.health.clone();
                                let _step = health.controller_step(Work::Command);
                                let started = Instant::now();
                                let what = command_label(&msg);
                                self.handle(controller.as_mut(), msg);
                                log_slow_step("command", Some(&what), started.elapsed());
                            }
                            HostMsg::Emit(m) => self.router.send(m),
                            HostMsg::UiDisconnected => controller.ui_disconnected(),
                            HostMsg::SaveBeforeQuit(done) => {
                                if !saved_on_quit {
                                    saved_on_quit = true;
                                    self.save_on_quit(controller.as_mut());
                                }
                                let _ = done.send(());
                            }
                            HostMsg::Shutdown => break 'run,
                        }
                        next = self.rx.try_recv().ok();
                    }
                    self.sync_device(controller.as_ref());
                }
                Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {}
            }
            let now = Instant::now();
            if now >= next_tick {
                let now_ms = NativeServices::now_ms_static();
                let health = self.health.clone();
                let step = health.controller_step(Work::Tick);
                let started = Instant::now();
                let r = std::panic::catch_unwind(AssertUnwindSafe(|| {
                    controller.tick(now_ms, &mut self.router)
                }));
                drop(step);
                log_slow_step("tick", None, started.elapsed());
                if let Err(p) = r
                    && !tick_panicked
                {
                    tick_panicked = true;
                    tracing::error!(msg = %panic_message(&p), "controller tick panicked");
                }
                self.sync_device(controller.as_ref());
                self.report_xruns();
                self.check_device();
                next_tick += TICK;
                if next_tick < now {
                    next_tick = now + TICK;
                }
            }
            self.router.flush_meters(now);
        }
        if !saved_on_quit {
            self.save_on_quit(controller.as_mut());
        }
        drop(controller);
        if let Some(out) = self.audio.output.take() {
            drop(out.stop());
        }
    }

    /// [`HostedController::before_shutdown`]; a project it could not save is kept in
    /// [`recovered_dir`].
    fn save_on_quit(&mut self, controller: &mut dyn HostedController) {
        let router = &mut self.router;
        match std::panic::catch_unwind(AssertUnwindSafe(|| controller.before_shutdown(router))) {
            Ok(Some(unsaved)) => keep_unsaved(&self.audio.data_dir, &unsaved),
            Ok(None) => {}
            Err(p) => tracing::error!(msg = %panic_message(&p), "saving on quit panicked"),
        }
    }

    /// Close the audio device while the local engine is off, open it when it is on again;
    /// the UI gets the new status (and a warning when the device is gone).
    fn sync_device(&mut self, controller: &dyn HostedController) {
        let off = controller.local_engine_off();
        if off == self.audio.released {
            return;
        }
        let started = Instant::now();
        if let Some(w) = self.audio.set_released(off) {
            self.router
                .send(notification(NotificationLevel::Warning, w));
        }
        log_slow_step("audio device", None, started.elapsed());
        tracing::info!(
            released = off,
            "audio device {}",
            if off { "closed" } else { "opened" }
        );
        self.router.send(ServerMessage::Event(Event::Engine {
            event: EngineEvent::Status {
                status: self.audio.status(),
            },
        }));
    }

    /// A lost output device or crashed render: notify, restart elsewhere, come back when
    /// the device does ([`device`]).
    fn check_device(&mut self) {
        let started = Instant::now();
        let msgs = self.audio.check_device(started);
        if !msgs.is_empty() {
            log_slow_step("audio device", None, started.elapsed());
        }
        for m in msgs {
            self.router.send(m);
        }
    }

    fn report_xruns(&mut self) {
        let x = self.audio.shared.xruns.load(Ordering::Relaxed);
        if x != self.audio.last_xruns {
            self.audio.last_xruns = x;
            self.router.send(ServerMessage::Event(Event::Engine {
                event: EngineEvent::Xrun { count: x },
            }));
        }
    }

    fn handle(&mut self, controller: &mut dyn HostedController, msg: ClientMessage) {
        for w in std::mem::take(&mut self.startup_warnings) {
            self.router
                .send(notification(NotificationLevel::Warning, w));
        }
        let id = msg.id;
        match &msg.command {
            Command::Engine(cmd) => {
                let reply = self.engine_command(id, cmd.clone());
                self.router.send(reply);
                return;
            }
            Command::Plugin(
                cmd @ (PluginCommand::List
                | PluginCommand::Rescan { .. }
                | PluginCommand::ListFolders
                | PluginCommand::AddFolder { .. }
                | PluginCommand::RemoveFolder { .. }
                | PluginCommand::SetIncludeDefaults { .. }
                | PluginCommand::OpenEditor { .. }
                | PluginCommand::CloseEditor { .. }
                | PluginCommand::SetEditorKeyCapture { .. }
                | PluginCommand::ListScanFailures
                | PluginCommand::ParamTexts { .. }),
            ) => {
                let reply = self.plugin_command(id, cmd.clone());
                self.router.send(reply);
                return;
            }
            // Replies when the scan ends, from the retry thread.
            Command::Plugin(PluginCommand::RetryScan {
                path,
                format,
                timeout_ms,
            }) => {
                let target = ether_plugin_host::ScanTarget {
                    format: *format,
                    path: PathBuf::from(path),
                };
                self.spawn_retry(id, target, *timeout_ms);
                return;
            }
            _ => {}
        }
        self.router.last_reply = None;
        let r = std::panic::catch_unwind(AssertUnwindSafe(|| {
            controller.handle(msg, &mut self.router)
        }));
        if let Err(p) = r {
            let text = panic_message(&p);
            tracing::error!(%text, "controller panicked handling a message");
            if self.router.last_reply != Some(id) {
                self.router.send(reply_err(
                    id,
                    ErrorCode::Internal,
                    format!("internal error: {text}"),
                ));
            }
        }
    }

    fn engine_command(&mut self, id: u32, cmd: EngineCommand) -> ServerMessage {
        match cmd {
            EngineCommand::GetStatus => reply_ok(
                id,
                ReplyValue::Status {
                    status: self.audio.status(),
                },
            ),
            EngineCommand::ListAudioDevices => reply_ok(
                id,
                ReplyValue::AudioDevices {
                    devices: audio::list_devices(&self.audio.settings),
                },
            ),
            EngineCommand::SetAudioConfig { config } => {
                let new = match self.audio.settings.merged(&config) {
                    Ok(s) => s,
                    Err(e) => return reply_err(id, ErrorCode::InvalidArgument, e.to_string()),
                };
                match self.audio.reconfigure(new) {
                    Ok(note) => {
                        if let Some(n) = note {
                            self.router.send(notification(NotificationLevel::Info, n));
                        }
                        if let Some(e) = self.audio.shared.recording.input_error() {
                            self.router
                                .send(notification(NotificationLevel::Warning, e));
                        }
                        let status = self.audio.status();
                        self.router.send(ServerMessage::Event(Event::Engine {
                            event: EngineEvent::Status {
                                status: status.clone(),
                            },
                        }));
                        reply_ok(id, ReplyValue::Status { status })
                    }
                    Err(e) => reply_err(id, ErrorCode::Io, e),
                }
            }
        }
    }

    fn plugin_command(&mut self, id: u32, cmd: PluginCommand) -> ServerMessage {
        let plugin_err =
            |e: ether_core::plugin::PluginError| reply_err(id, ErrorCode::Plugin, e.to_string());
        match cmd {
            PluginCommand::List => reply_ok(
                id,
                ReplyValue::Plugins {
                    plugins: self.catalog.list(),
                },
            ),
            PluginCommand::OpenEditor { device } => match self.plugins.open_editor(device) {
                Ok(()) => reply_ok(id, ReplyValue::Unit),
                Err(e) => plugin_err(e),
            },
            PluginCommand::CloseEditor { device } => match self.plugins.close_editor(device) {
                Ok(()) => reply_ok(id, ReplyValue::Unit),
                Err(e) => plugin_err(e),
            },
            PluginCommand::ParamTexts { device, values } => {
                let values = values.iter().map(|v| (v.param, v.value)).collect();
                let texts = self.plugins.param_texts(device, values);
                reply_ok(id, ReplyValue::ParamTexts { texts })
            }
            PluginCommand::SetEditorKeyCapture { codes } => {
                // Process-wide (the editor windows' key hook); sandbox helpers get it on
                // their next poll.
                ether_plugin_host::editor_keys::set_captured_codes(codes);
                reply_ok(id, ReplyValue::Unit)
            }
            PluginCommand::Rescan { full } => {
                self.request_scan(full);
                reply_ok(id, ReplyValue::Unit)
            }
            PluginCommand::ListScanFailures => reply_ok(
                id,
                ReplyValue::ScanFailures {
                    failed: self.catalog.failures(),
                },
            ),
            PluginCommand::ListFolders => self.folders_reply(id),
            PluginCommand::AddFolder { path, format } => {
                match self.plugin_folders.add(&path, format) {
                    Ok(()) => {
                        self.request_scan(false);
                        self.folders_reply(id)
                    }
                    Err(e) => folder_err(id, e),
                }
            }
            PluginCommand::RemoveFolder { path } => match self.plugin_folders.remove(&path) {
                Ok(()) => {
                    self.request_scan(false);
                    self.folders_reply(id)
                }
                Err(e) => folder_err(id, e),
            },
            PluginCommand::SetIncludeDefaults { include } => {
                self.plugin_folders.set_include_defaults(include);
                self.request_scan(false);
                self.folders_reply(id)
            }
            // Routed to the controller by `handle`.
            _ => reply_err(id, ErrorCode::Internal, "unexpected plugin command"),
        }
    }

    fn folders_reply(&self, id: u32) -> ServerMessage {
        reply_ok(
            id,
            ReplyValue::PluginFolders {
                folders: self.plugin_folders.list(&crate::plugins::formats()),
            },
        )
    }

    /// Start a scan, or (one is running) queue another for when it ends. A queued full
    /// scan wins over a queued incremental one.
    fn request_scan(&self, full: bool) {
        if self.scanning.swap(true, Ordering::SeqCst) {
            self.rescan_pending.fetch_max(
                if full {
                    RESCAN_FULL
                } else {
                    RESCAN_INCREMENTAL
                },
                Ordering::SeqCst,
            );
            return;
        }
        self.spawn_scan(full);
    }

    /// Scan the plugin folders out-of-process on a background thread (parallel children;
    /// unchanged plugins come from the scan cache unless `full`); progress arrives as
    /// `Event::Plugin` messages. Runs again while rescans were queued meanwhile.
    fn spawn_scan(&self, full: bool) {
        let tx = self.tx.clone();
        let catalog = self.catalog.clone();
        let scanning = self.scanning.clone();
        let pending = self.rescan_pending.clone();
        let folders = self.plugin_folders.clone();
        let scan_db = self.scan_db.clone();
        let cache_file = self.plugin_db.join(ether_plugin_host::SCAN_CACHE_FILE);
        let emit = move |e: PluginEvent| {
            let _ = tx.send(HostMsg::Emit(ServerMessage::Event(Event::Plugin {
                event: e,
            })));
        };
        let spawned = std::thread::Builder::new()
            .name("ether-plugin-scan".into())
            .spawn(move || {
                let mut full = full;
                loop {
                    let db = scan_db.lock().unwrap_or_else(|e| e.into_inner());
                    let formats = crate::plugins::formats();
                    let targets = folders.discover(&formats);
                    let report = match ether_plugin_host::ScanRunner::locate() {
                        Some(runner) => {
                            let mut cache = ether_plugin_host::ScanCache::open(&cache_file);
                            let scan = runner.scan_targets_cached_with(
                                &targets,
                                &mut cache,
                                full,
                                |t| formats.registry_version(t),
                                |done, total, current| {
                                    emit(PluginEvent::ScanProgress {
                                        done,
                                        total,
                                        current: current.map(|p| p.display().to_string()),
                                    });
                                },
                            );
                            if let Err(e) = cache.save() {
                                tracing::warn!(%e, "failed to write the plugin scan cache");
                            }
                            tracing::info!(
                                scanned = scan.scanned,
                                reused = scan.reused,
                                jobs = runner.jobs,
                                full,
                                "plugin scan done"
                            );
                            scan.report
                        }
                        None => ether_plugin_host::ScanReport {
                            plugins: Vec::new(),
                            failed: vec![ether_core::protocol::plugins::ScanFailure {
                                path: String::new(),
                                message: "plugin scanner binary not found".into(),
                                format: None,
                            }],
                        },
                    };
                    let count = report.plugins.len() as u32;
                    catalog.replace(report.plugins);
                    catalog.set_failures(report.failed.clone());
                    drop(db);
                    emit(PluginEvent::ScanFinished {
                        plugins: count,
                        failed: report.failed,
                    });
                    let next = pending.swap(RESCAN_NONE, Ordering::SeqCst);
                    if next != RESCAN_NONE {
                        full = next == RESCAN_FULL;
                        continue;
                    }
                    scanning.store(false, Ordering::SeqCst);
                    // A request that saw `scanning` just before it was cleared queued
                    // itself: take it over unless a new scan already started.
                    let next = pending.load(Ordering::SeqCst);
                    if next == RESCAN_NONE || scanning.swap(true, Ordering::SeqCst) {
                        break;
                    }
                    full = pending.swap(RESCAN_NONE, Ordering::SeqCst) == RESCAN_FULL;
                }
            });
        if spawned.is_err() {
            self.scanning.store(false, Ordering::SeqCst);
        }
    }

    /// `Plugin::RetryScan`: scan one target on a background thread (its own scanner child,
    /// with a longer timeout), then record it in the scan cache and the plugin list and
    /// reply `ScanRetried` (followed by `ScanFinished`, so lists refresh). Several retries
    /// may run at once. A retry that can't start is an error reply right away.
    fn spawn_retry(&self, id: u32, target: ether_plugin_host::ScanTarget, timeout_ms: Option<u32>) {
        let reply_now = |reply| {
            let _ = self.tx.send(HostMsg::Emit(reply));
        };
        let Some(runner) = ether_plugin_host::ScanRunner::locate() else {
            return reply_now(reply_err(
                id,
                ErrorCode::Plugin,
                "plugin scanner binary not found",
            ));
        };
        let runner = runner.with_timeout(retry_timeout(timeout_ms));
        let tx = self.tx.clone();
        let catalog = self.catalog.clone();
        let scan_db = self.scan_db.clone();
        let scanning = self.scanning.clone();
        let cache_file = self.plugin_db.join(ether_plugin_host::SCAN_CACHE_FILE);
        let spawned = std::thread::Builder::new()
            .name("ether-plugin-retry".into())
            .spawn(move || {
                let scan = runner.scan_one(&target);
                {
                    let _db = scan_db.lock().unwrap_or_else(|e| e.into_inner());
                    let mut cache = ether_plugin_host::ScanCache::open(&cache_file);
                    cache.record_with(
                        &scan,
                        crate::plugins::formats().registry_version(&scan.target),
                    );
                    if let Err(e) = cache.save() {
                        tracing::warn!(%e, "failed to write the plugin scan cache");
                    }
                    catalog.apply_retry(&scan);
                }
                let (plugins, failure) = match scan.outcome() {
                    Ok(p) => (p.to_vec(), None),
                    Err(f) => (Vec::new(), Some(f)),
                };
                tracing::info!(
                    target = %target.path.display(),
                    timeout = ?runner.timeout,
                    plugins = plugins.len(),
                    failure = failure.as_ref().map(|f| f.message.as_str()),
                    "plugin scan retry done"
                );
                let _ = tx.send(HostMsg::Emit(reply_ok(
                    id,
                    ReplyValue::ScanRetried { plugins, failure },
                )));
                // A scan running now reads the recorded result and reports it itself (and
                // its progress must not look finished).
                if !scanning.load(Ordering::SeqCst) {
                    let _ = tx.send(HostMsg::Emit(ServerMessage::Event(Event::Plugin {
                        event: PluginEvent::ScanFinished {
                            plugins: catalog.len() as u32,
                            failed: catalog.failures(),
                        },
                    })));
                }
            });
        if let Err(e) = spawned {
            reply_now(reply_err(
                id,
                ErrorCode::Internal,
                format!("retry thread: {e}"),
            ));
        }
    }
}

/// Default timeout of `Plugin::RetryScan`, and its bounds.
const RETRY_TIMEOUT: Duration = Duration::from_secs(90);
const RETRY_TIMEOUT_MIN: Duration = Duration::from_secs(1);
/// A shell bundle with hundreds of plugins (Waves) can take many minutes to describe.
const RETRY_TIMEOUT_MAX: Duration = Duration::from_secs(3600);

fn retry_timeout(timeout_ms: Option<u32>) -> Duration {
    timeout_ms.map_or(RETRY_TIMEOUT, |ms| {
        Duration::from_millis(ms.into()).clamp(RETRY_TIMEOUT_MIN, RETRY_TIMEOUT_MAX)
    })
}

#[cfg(test)]
#[test]
fn retry_timeout_defaults_to_90s_and_is_clamped() {
    assert_eq!(retry_timeout(None), Duration::from_secs(90));
    assert_eq!(retry_timeout(Some(90_000)), Duration::from_secs(90));
    assert_eq!(retry_timeout(Some(0)), RETRY_TIMEOUT_MIN);
    assert_eq!(retry_timeout(Some(u32::MAX)), RETRY_TIMEOUT_MAX);
}

const RESCAN_NONE: u8 = 0;
const RESCAN_INCREMENTAL: u8 = 1;
const RESCAN_FULL: u8 = 2;

fn folder_err(id: u32, e: FolderError) -> ServerMessage {
    match e {
        FolderError::Invalid(m) => reply_err(id, ErrorCode::InvalidArgument, m),
        FolderError::NotFound(m) => reply_err(id, ErrorCode::NotFound, m),
    }
}

impl NativeServices {
    fn now_ms_static() -> u64 {
        use ether_controller::HostServices;
        NativeServices::default().now_ms()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_not_saved_on_quit_is_kept_and_reported_once() {
        let dir = crate::test_util::TempDir::new("recovered");
        assert_eq!(recovered_warning(dir.path()), None);
        keep_unsaved(
            dir.path(),
            &UnsavedProject {
                id: "p1".into(),
                document: "{\"doc\":1}".into(),
                error: "disk full".into(),
            },
        );
        let kept: Vec<PathBuf> = std::fs::read_dir(recovered_dir(dir.path()))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "ether"))
            .collect();
        assert_eq!(kept.len(), 1);
        assert_eq!(std::fs::read_to_string(&kept[0]).unwrap(), "{\"doc\":1}");
        let warning = recovered_warning(dir.path()).expect("reported");
        assert!(
            warning.text.contains(&kept[0].display().to_string()),
            "{warning}"
        );
        assert_eq!(recovered_warning(dir.path()), None, "once");
        assert!(kept[0].exists(), "the copy stays");
    }
}

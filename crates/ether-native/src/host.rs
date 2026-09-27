//! [`NativeHost`]: owns the threads of the desktop runtime and routes messages.
//!
//! ```text
//!  Tauri command ──ClientMessage──▶ controller thread ──ServerMessage──▶ subscriber
//!                                    │  Controller::handle / tick (60 Hz)   (Tauri channels)
//!                                    │  host-handled: Engine::*, Plugin::{Rescan,List,
//!                                    │                OpenEditor,CloseEditor}
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

use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, unbounded};
use ether_controller::{Controller, ControllerConfig, EtherController, MessageSink};
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
use crate::plugins::{DedicatedThread, Instantiate, MainThread, PluginCatalog, PluginHost};
use crate::rt::AudioShared;
use crate::store::{DiskStore, LibraryRoot};

/// Controller tick period (~60 Hz: playhead rate).
pub const TICK: Duration = Duration::from_millis(16);
/// Minimum interval between meter frames (~30 Hz).
pub const METER_INTERVAL: Duration = Duration::from_millis(33);
/// GC thread period.
pub const GC_INTERVAL: Duration = Duration::from_millis(20);

/// Receives every `ServerMessage` for the UI (called on the controller thread).
pub type Subscriber = Arc<dyn Fn(ServerMessage) + Send + Sync>;

/// A controller as run by the native host: the [`Controller`] contract plus host
/// lifecycle hooks (defaults do nothing, so test controllers only implement `Controller`).
///
/// The engine's sample rate is fixed for the host's lifetime (a rate change is applied at
/// the next start, see `AudioState::reconfigure`), so the controller is configured with it
/// once, at construction.
pub trait HostedController: Controller {
    /// The host is quitting: persist unsaved work. Events go to `out`.
    fn before_shutdown(&mut self, out: &mut dyn MessageSink) {
        let _ = out;
    }
}

/// The standard native controller.
pub type NativeController = EtherController<NativeBridge, NativeServices, DiskStore, DiskStore>;

impl HostedController for NativeController {
    fn before_shutdown(&mut self, out: &mut dyn MessageSink) {
        if self.is_dirty()
            && let Err(e) = self.save_now(out)
        {
            tracing::warn!(error = %e.message, "failed to save the project on quit");
        }
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
    Box::new(|bridge, services, store, library| {
        let config = native_controller_config(bridge.sample_rate());
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
}

impl Default for HostOptions {
    fn default() -> Self {
        Self {
            main_thread: Arc::new(DedicatedThread::new()),
            instantiate: Arc::new(ether_clap::instantiate),
            controller: ether_controller(),
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
    Shutdown,
}

/// The running native host. Methods are callable from any thread (Tauri commands).
pub struct NativeHost {
    tx: Sender<HostMsg>,
    subscriber: Arc<Mutex<Option<Subscriber>>>,
    controller_thread: Option<JoinHandle<()>>,
    gc_stop: Arc<AtomicBool>,
    gc_thread: Option<JoinHandle<()>>,
    info: StreamInfo,
    plugins: PluginHost,
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

/// Load persisted audio settings, then apply `ETHER_AUDIO`.
pub fn load_audio_settings(data_dir: &std::path::Path) -> AudioSettings {
    let mut s: AudioSettings = std::fs::read_to_string(settings_file(data_dir))
        .ok()
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default();
    if let Some(b) = AudioBackendKind::parse(std::env::var("ETHER_AUDIO").ok().as_deref()) {
        s.backend = b;
    }
    s
}

fn save_audio_settings(data_dir: &std::path::Path, s: &AudioSettings) {
    if let Ok(json) = serde_json::to_string_pretty(s)
        && let Err(e) = crate::store::atomic_write(&settings_file(data_dir), json.as_bytes())
    {
        tracing::warn!(%e, "failed to save audio settings");
    }
}

/// Start `settings` or fall back to the null backend (so the app runs without a device).
fn start_audio(
    engine: Box<Engine>,
    settings: &AudioSettings,
    shared: &Arc<AudioShared>,
) -> (Result<AudioOutput, Box<Engine>>, Option<String>) {
    match AudioOutput::start(engine, settings, shared.clone()) {
        Ok(out) => (Ok(out), None),
        Err((e, engine)) if settings.backend == AudioBackendKind::Cpal => {
            let warning = format!("Audio device unavailable ({e}); running without sound.");
            tracing::warn!("{warning}");
            let null = AudioSettings {
                backend: AudioBackendKind::Null,
                ..settings.clone()
            };
            match AudioOutput::start(engine, &null, shared.clone()) {
                Ok(out) => (Ok(out), Some(warning)),
                Err((e, engine)) => (Err(engine), Some(format!("Audio failed to start: {e}"))),
            }
        }
        Err((e, engine)) => (Err(engine), Some(format!("Audio failed to start: {e}"))),
    }
}

impl NativeHost {
    /// Create the engine, start audio, GC and controller threads.
    pub fn start(config: HostConfig, options: HostOptions) -> Result<Self, HostError> {
        let mut settings = config
            .audio
            .clone()
            .unwrap_or_else(|| load_audio_settings(&config.data_dir));
        let resolved = match audio::resolve(&settings) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(%e, "audio device unavailable; using the null backend");
                settings.backend = AudioBackendKind::Null;
                audio::resolve(&settings).map_err(|e| HostError::Start(e.to_string()))?
            }
        };
        let engine_config = EngineConfig {
            sample_rate: resolved.info.sample_rate,
            max_block_size: settings.max_block_size.max(16),
            ..Default::default()
        };
        let prepare = PrepareConfig {
            sample_rate: engine_config.sample_rate as f32,
            max_block_size: engine_config.max_block_size,
            max_events_per_block: engine_config.max_events_per_block,
        };
        let mut parts = ether_core::create(engine_config);
        parts
            .handle
            .set_stretcher_factory(Arc::new(ether_stretch::SignalsmithFactory::default()));
        let shared = Arc::new(AudioShared::default());
        shared
            .recording
            .set_projects_root(config.projects_root.clone());
        let (started, warning) = start_audio(Box::new(parts.engine), &settings, &shared);
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
        let catalog = PluginCatalog::open(&config.data_dir.join("plugin-db"));
        let store = DiskStore::new(config.projects_root.clone(), config.library_roots.clone());
        let bridge = NativeBridge::new(
            parts.handle,
            prepare,
            plugins.clone(),
            catalog.clone(),
            options.instantiate,
            shared.clone(),
        );
        let factory = options.controller;
        let ctx = ControllerThread {
            rx,
            tx: tx.clone(),
            router: Router::new(subscriber.clone()),
            audio: AudioState {
                output,
                parked,
                settings,
                shared,
                engine_rate: prepare.sample_rate as u32,
                data_dir: config.data_dir.clone(),
                instance: config.instance.clone(),
                last_xruns: 0,
            },
            plugins: plugins.clone(),
            catalog,
            scanning: Arc::new(AtomicBool::new(false)),
            startup_warning: warning,
        };
        let controller_thread = std::thread::Builder::new()
            .name("ether-controller".into())
            .spawn(move || {
                let controller = match std::panic::catch_unwind(AssertUnwindSafe(|| {
                    factory(bridge, NativeServices, store.clone(), store)
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

        Ok(Self {
            tx,
            subscriber,
            controller_thread: Some(controller_thread),
            gc_stop,
            gc_thread: Some(gc_thread),
            info,
            plugins,
        })
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
        // Main-thread plugin calls fail fast from now on: `stop` may run ON the main thread
        // (Tauri exit), which then can't serve the controller thread we are about to join.
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
            },
        },
    })
}

fn notification(level: NotificationLevel, message: impl Into<String>) -> ServerMessage {
    ServerMessage::Event(Event::Notification {
        level,
        message: message.into(),
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
    settings: AudioSettings,
    shared: Arc<AudioShared>,
    engine_rate: u32,
    data_dir: PathBuf,
    instance: String,
    last_xruns: u32,
}

impl AudioState {
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
        }
    }

    /// Switch device/backend/buffer size. The engine moves over; a sample-rate change needs
    /// a new engine, so it is saved and applied at the next start.
    fn reconfigure(&mut self, new: AudioSettings) -> Result<Option<String>, String> {
        save_audio_settings(&self.data_dir, &new);
        let mut note = None;
        if new.sample_rate.is_some_and(|r| r != self.engine_rate) {
            note = Some(format!(
                "Sample rate {} Hz will be used after restarting Ethereal.",
                new.sample_rate.unwrap_or_default()
            ));
        }
        let run = AudioSettings {
            sample_rate: Some(self.engine_rate),
            ..new.clone()
        };
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
                Ok(note)
            }
            Err((e, engine)) => {
                // Restore the previous configuration.
                let old = AudioSettings {
                    sample_rate: Some(self.engine_rate),
                    ..self.settings.clone()
                };
                match start_audio(engine, &old, &self.shared).0 {
                    Ok(out) => self.output = Some(out),
                    Err(engine) => self.parked = Some(engine),
                }
                save_audio_settings(&self.data_dir, &self.settings);
                Err(e.to_string())
            }
        }
    }
}

struct ControllerThread {
    rx: Receiver<HostMsg>,
    tx: Sender<HostMsg>,
    router: Router,
    audio: AudioState,
    plugins: PluginHost,
    catalog: PluginCatalog,
    scanning: Arc<AtomicBool>,
    startup_warning: Option<String>,
}

impl ControllerThread {
    fn run(mut self, mut controller: Box<dyn HostedController>) {
        let start = Instant::now();
        let mut next_tick = start;
        let mut tick_panicked = false;
        loop {
            let now = Instant::now();
            match self
                .rx
                .recv_timeout(next_tick.saturating_duration_since(now))
            {
                Ok(HostMsg::Client(msg)) => self.handle(controller.as_mut(), msg),
                Ok(HostMsg::Emit(m)) => self.router.send(m),
                Ok(HostMsg::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {}
            }
            let now = Instant::now();
            if now >= next_tick {
                let now_ms = NativeServices::now_ms_static();
                let r = std::panic::catch_unwind(AssertUnwindSafe(|| {
                    controller.tick(now_ms, &mut self.router)
                }));
                if let Err(p) = r
                    && !tick_panicked
                {
                    tick_panicked = true;
                    tracing::error!(msg = %panic_message(&p), "controller tick panicked");
                }
                self.report_xruns();
                next_tick += TICK;
                if next_tick < now {
                    next_tick = now + TICK;
                }
            }
            self.router.flush_meters(now);
        }
        let saved = std::panic::catch_unwind(AssertUnwindSafe(|| {
            controller.before_shutdown(&mut self.router)
        }));
        if let Err(p) = saved {
            tracing::error!(msg = %panic_message(&p), "saving on quit panicked");
        }
        drop(controller);
        if let Some(out) = self.audio.output.take() {
            drop(out.stop());
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
        if let Some(w) = self.startup_warning.take() {
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
                | PluginCommand::Rescan
                | PluginCommand::OpenEditor { .. }
                | PluginCommand::CloseEditor { .. }),
            ) => {
                let reply = self.plugin_command(id, cmd.clone());
                self.router.send(reply);
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
            PluginCommand::Rescan => {
                if self.scanning.swap(true, Ordering::SeqCst) {
                    return reply_ok(id, ReplyValue::Unit);
                }
                self.spawn_scan();
                reply_ok(id, ReplyValue::Unit)
            }
            // Routed to the controller by `handle`.
            _ => reply_err(id, ErrorCode::Internal, "unexpected plugin command"),
        }
    }

    /// Scan plugin bundles out-of-process on a background thread; progress arrives as
    /// `Event::Plugin` messages.
    fn spawn_scan(&self) {
        let tx = self.tx.clone();
        let catalog = self.catalog.clone();
        let scanning = self.scanning.clone();
        let emit = move |e: PluginEvent| {
            let _ = tx.send(HostMsg::Emit(ServerMessage::Event(Event::Plugin {
                event: e,
            })));
        };
        let spawned = std::thread::Builder::new()
            .name("ether-plugin-scan".into())
            .spawn(move || {
                let bundles = ether_clap::find_bundles(&ether_clap::default_search_paths());
                let report = match ether_clap::ScanRunner::locate() {
                    Some(runner) => runner.scan_all(&bundles, |done, total, current| {
                        emit(PluginEvent::ScanProgress {
                            done,
                            total,
                            current: current.map(|p| p.display().to_string()),
                        });
                    }),
                    None => ether_clap::ScanReport {
                        plugins: Vec::new(),
                        failed: vec![ether_core::protocol::plugins::ScanFailure {
                            path: String::new(),
                            message: "plugin scanner binary not found".into(),
                        }],
                    },
                };
                let count = report.plugins.len() as u32;
                catalog.replace(report.plugins);
                emit(PluginEvent::ScanFinished {
                    plugins: count,
                    failed: report.failed,
                });
                scanning.store(false, Ordering::SeqCst);
            });
        if spawned.is_err() {
            self.scanning.store(false, Ordering::SeqCst);
        }
    }
}

impl NativeServices {
    fn now_ms_static() -> u64 {
        use ether_controller::HostServices;
        NativeServices.now_ms()
    }
}

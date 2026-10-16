//! Ethereal desktop shell (Tauri v2): thin glue over `ether-native`.
//!
//! # IPC (backs `ui/src/transport/tauri/TauriTransport.ts`)
//! - `ether_connect(messages, playhead, meters)`: registers three Tauri `Channel`s and
//!   returns [`EngineInfo`]. Replies and events (`ServerMessage::Reply | Event`) go to
//!   `messages`, in order (so a command's patches arrive before its reply); `Playhead`
//!   frames (~60 Hz) and `Meters` frames (~30 Hz) go to their own channels. Calling it again
//!   (UI reload) replaces the previous channels and drops the old UI's analysis watches
//!   (`NativeHost::ui_disconnected`).
//! - `ether_send(message)` / `ether_send_batch(messages)`: queue `ClientMessage`s in order
//!   (never blocks; replies arrive on `messages`). Both run off the main thread.
//! - Playhead and meter frames are delivered latest-wins (at most one queued per stream,
//!   see [`Latest`]): a busy main thread never accumulates a backlog of stale frames.
//! - `ether_disconnect()`: drop the channels (and the UI's analysis watches).
//! - `engine_info()`: instance/data dir/audio diagnostics.
//! - `agent_bridge_status()` / `agent_bridge_set_enabled(enabled)`: the opt-in loopback
//!   bridge for AI agents (`ether-mcp`), see [`agent_bridge`].
//! - `ai_http_allow(base_url)`: lets the AI chat fetch a configured provider through
//!   `tauri-plugin-http` (no CORS), see [`ai_http`].
//! - `app_quit_guard(ready)` / `app_quit()` / `app_quit_cancel()`: the quit confirmation
//!   (the shell emits `ether://quit-requested`; ⌘W in the main window emits
//!   `ether://close-requested`), see [`quit`].
//!
//! # Threads
//! The host runs its own controller, GC and audio threads. Plugin controllers and editors
//! run on the **process main thread** through [`TauriMainThread`]
//! (`AppHandle::run_on_main_thread`), as AppKit requires for plugin editor windows on macOS;
//! on Windows on their own UI thread instead ([`plugin_main_thread`]), so a slow plugin
//! never freezes the app's window and webview (both on the main thread).

pub mod agent_bridge;
pub mod ai_http;
pub mod app_menu;
pub mod instance;
pub mod log_file;
mod main_queue;
pub mod path_drop;
pub mod quit;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ether_core::protocol::message::PlayheadFrame;
use ether_core::protocol::meters::MeterFrame;
use ether_core::protocol::{ClientMessage, ServerMessage};
use ether_native::host::{HostConfig, HostOptions};
use ether_native::{LibraryRoot, MainThread, NativeHost, editor_owner};

use agent_bridge::{AgentBridgeState, AgentBridgeStatus};
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, RunEvent, WindowEvent};

/// Resolved per-process paths, managed by Tauri.
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub instance: String,
    pub data_dir: PathBuf,
    pub projects_root: PathBuf,
}

/// The running host (taken on exit for an orderly shutdown). Shared with the agent
/// bridge's listener while it is enabled.
#[derive(Default)]
pub struct HostSlot(Mutex<Option<Arc<NativeHost>>>);

impl HostSlot {
    fn get(&self) -> Option<Arc<NativeHost>> {
        self.0.lock().ok().and_then(|g| g.clone())
    }

    fn with<R>(&self, f: impl FnOnce(&NativeHost) -> R) -> Result<R, String> {
        let guard = self
            .0
            .lock()
            .map_err(|_| "host lock poisoned".to_string())?;
        guard
            .as_ref()
            .map(|h| f(h))
            .ok_or_else(|| "engine is not running".to_string())
    }
}

/// The process main thread (set first thing in [`run`]).
static MAIN_THREAD: std::sync::OnceLock<std::thread::ThreadId> = std::sync::OnceLock::new();

/// Runs plugin main-thread work on the process main thread (Tauri's event loop), through a
/// [`main_queue::MainQueue`] the main thread also serves while it stops the host (#566).
pub struct TauriMainThread {
    app: AppHandle,
    queue: main_queue::MainQueue,
}

impl TauriMainThread {
    fn new(app: AppHandle) -> Self {
        let main = *MAIN_THREAD.get_or_init(|| std::thread::current().id());
        Self {
            app,
            queue: main_queue::MainQueue::new(main),
        }
    }
}

impl MainThread for TauriMainThread {
    fn spawn(&self, f: Box<dyn FnOnce() + Send>) {
        if let Err(e) = self.app.run_on_main_thread(self.queue.push(f)) {
            tracing::error!(%e, "run_on_main_thread failed");
        }
    }

    fn run_pending(&self) {
        self.queue.run_pending();
    }
}

/// Where plugin main-thread work and editors run. Windows: their own UI thread (a slow
/// plugin never blocks the app's window and webview), unless `ETHEREAL_PLUGIN_THREAD=main`.
/// Elsewhere: the process main thread (AppKit requires it on macOS).
fn plugin_main_thread(app: &AppHandle) -> Arc<dyn MainThread> {
    #[cfg(windows)]
    if std::env::var("ETHEREAL_PLUGIN_THREAD").as_deref() != Ok("main") {
        match ether_native::PluginUiThread::new() {
            Ok(t) => {
                tracing::info!("plugins run on their own UI thread");
                return Arc::new(t);
            }
            Err(e) => tracing::warn!(%e, "no plugin UI thread: plugins run on the main thread"),
        }
    }
    Arc::new(TauriMainThread::new(app.clone()))
}

#[derive(Debug, Serialize)]
pub struct EngineInfo {
    pub version: &'static str,
    pub instance: String,
    pub data_dir: String,
    pub projects_root: String,
    pub backend: String,
    pub sample_rate: u32,
    pub buffer_size: u32,
    pub device: Option<String>,
}

fn info(paths: &AppPaths, host: &NativeHost) -> EngineInfo {
    let a = host.audio_info();
    EngineInfo {
        version: env!("CARGO_PKG_VERSION"),
        instance: paths.instance.clone(),
        data_dir: paths.data_dir.display().to_string(),
        projects_root: paths.projects_root.display().to_string(),
        backend: a.backend.name().to_string(),
        sample_rate: a.sample_rate,
        buffer_size: a.buffer_size,
        device: a.device.clone(),
    }
}

#[tauri::command]
fn engine_info(
    paths: tauri::State<'_, AppPaths>,
    host: tauri::State<'_, HostSlot>,
) -> Result<EngineInfo, String> {
    host.with(|h| info(&paths, h))
}

/// Latest-wins delivery of a frame stream (playhead, meters) to its channel.
///
/// Every `Channel::send` becomes a script evaluation queued on the main thread (the
/// webview's thread). When that thread is busy (plugin work, a long UI task), frames sent
/// at 30–60 Hz used to pile up there and then replay in a burst of stale positions. Here
/// at most one delivery per stream is queued: it sends whatever frame is the latest when
/// it runs, and frames superseded meanwhile are dropped.
struct Latest<T> {
    app: AppHandle,
    channel: Channel<T>,
    frame: Mutex<Option<T>>,
    queued: std::sync::atomic::AtomicBool,
}

impl<T: tauri::ipc::IpcResponse + Send + Sync + 'static> Latest<T> {
    fn new(app: AppHandle, channel: Channel<T>) -> Arc<Self> {
        Arc::new(Self {
            app,
            channel,
            frame: Mutex::new(None),
            queued: std::sync::atomic::AtomicBool::new(false),
        })
    }

    fn push(self: &Arc<Self>, frame: T) {
        use std::sync::atomic::Ordering;
        if let Ok(mut slot) = self.frame.lock() {
            *slot = Some(frame);
        }
        if self.queued.swap(true, Ordering::AcqRel) {
            return; // the queued delivery will send this frame
        }
        let this = self.clone();
        let r = self.app.run_on_main_thread(move || {
            this.queued.store(false, Ordering::Release);
            let frame = this.frame.lock().ok().and_then(|mut f| f.take());
            if let Some(frame) = frame
                && let Err(e) = this.channel.send(frame)
            {
                tracing::debug!(%e, "channel send failed");
            }
        });
        if let Err(e) = r {
            self.queued.store(false, Ordering::Release);
            tracing::debug!(%e, "run_on_main_thread failed");
        }
    }
}

#[tauri::command]
fn ether_connect(
    app: AppHandle,
    paths: tauri::State<'_, AppPaths>,
    host: tauri::State<'_, HostSlot>,
    bridge: tauri::State<'_, AgentBridgeState>,
    messages: Channel<ServerMessage>,
    playhead: Channel<PlayheadFrame>,
    meters: Channel<MeterFrame>,
) -> Result<EngineInfo, String> {
    host.with(|h| {
        let playhead = Latest::new(app.clone(), playhead);
        let meters = Latest::new(app.clone(), meters);
        // The host's subscriber is the fan-out (UI + agent bridge); this swaps the UI part.
        let replaced = bridge.fanout.set_ui(Some(Arc::new(move |m| match m {
            ServerMessage::Playhead(p) => playhead.push(p),
            ServerMessage::Meters(f) => meters.push(f),
            other => {
                if let Err(e) = messages.send(other) {
                    tracing::debug!(%e, "channel send failed");
                }
            }
        })));
        // A webview reload: the old UI's analysis watches would keep the engine analysing.
        if replaced && let Err(e) = h.ui_disconnected() {
            tracing::debug!(%e, "ui_disconnected failed");
        }
        info(&paths, h)
    })
}

/// Queue `messages` in order (the UI sends what accumulated during the previous invoke in
/// one go). `async` like [`ether_send`].
#[tauri::command(async)]
fn ether_send_batch(
    host: tauri::State<'_, HostSlot>,
    messages: Vec<ClientMessage>,
) -> Result<(), String> {
    host.with(|h| {
        messages
            .into_iter()
            .try_for_each(|m| h.send(m).map_err(|e| e.to_string()))
    })?
}

/// `async`: runs on the async runtime, not on the main thread (Tauri runs synchronous
/// commands there), so queueing a command never waits behind plugin or window work.
#[tauri::command(async)]
fn ether_send(host: tauri::State<'_, HostSlot>, message: ClientMessage) -> Result<(), String> {
    host.with(|h| h.send(message).map_err(|e| e.to_string()))?
}

#[tauri::command]
fn ether_disconnect(
    host: tauri::State<'_, HostSlot>,
    bridge: tauri::State<'_, AgentBridgeState>,
) -> Result<(), String> {
    // The UI's analysis watches go with it (best effort: the host may be stopping).
    if bridge.fanout.set_ui(None) {
        let _ = host.with(|h| h.ui_disconnected());
    }
    Ok(())
}

#[tauri::command]
fn agent_bridge_status(bridge: tauri::State<'_, AgentBridgeState>) -> AgentBridgeStatus {
    bridge.status()
}

#[tauri::command]
fn agent_bridge_set_enabled(
    host: tauri::State<'_, HostSlot>,
    bridge: tauri::State<'_, AgentBridgeState>,
    enabled: bool,
) -> Result<AgentBridgeStatus, String> {
    bridge.set_enabled(host.get(), enabled)?;
    Ok(bridge.status())
}

/// base-114: file holding the remembered collaboration relay token (in the app data dir).
const COLLAB_TOKEN_FILE: &str = "collab-token";

/// The remembered collaboration token, if any. The token is never logged.
#[tauri::command]
fn collab_token_load(paths: tauri::State<'_, AppPaths>) -> Option<String> {
    std::fs::read_to_string(paths.data_dir.join(COLLAB_TOKEN_FILE))
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// Remember (or, with `None`/empty, forget) the collaboration token. Owner-only file on Unix.
#[tauri::command]
fn collab_token_save(
    paths: tauri::State<'_, AppPaths>,
    token: Option<String>,
) -> Result<(), String> {
    let path = paths.data_dir.join(COLLAB_TOKEN_FILE);
    let token = token.unwrap_or_default();
    if token.trim().is_empty() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err("could not forget the token".into())
            }
            _ => Ok(()),
        };
    }
    std::fs::create_dir_all(&paths.data_dir)
        .map_err(|_| "could not create the app data folder".to_string())?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut f = options
        .open(&path)
        .map_err(|_| "could not remember the token".to_string())?;
    std::io::Write::write_all(&mut f, token.trim().as_bytes())
        .map_err(|_| "could not remember the token".to_string())
}

fn init_tracing() {
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    // stdout (the dev terminal) and the log file (`log_file`, once the data dir is known).
    // `try_init` so tests / repeated init don't panic.
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer())
        .with(fmt::layer().with_ansi(false).with_writer(log_file::LogFile))
        .try_init();
}

/// Library folders for the sample browser: the generated demo samples
/// (`<data_dir>/demo-samples`, shown as "Factory"), then `ETHER_LIBRARY` (OS path-list).
/// Other folders are the user's own, added from the browser ("Add folder…").
fn library_roots(data_dir: &std::path::Path) -> Vec<LibraryRoot> {
    let demo = data_dir.join("demo-samples");
    let mut roots = match ether_native::demo_samples::ensure(&demo) {
        Ok(()) => vec![LibraryRoot {
            id: "demo".into(),
            name: "factory".into(),
            path: demo,
        }],
        Err(e) => {
            tracing::warn!(%e, "could not write the demo samples");
            Vec::new()
        }
    };
    roots.extend(env_library_roots());
    roots
}

fn env_library_roots() -> Vec<LibraryRoot> {
    let Some(list) = std::env::var_os("ETHER_LIBRARY") else {
        return Vec::new();
    };
    std::env::split_paths(&list)
        .enumerate()
        .filter(|(_, p)| p.is_dir())
        .map(|(i, path)| LibraryRoot {
            id: format!("lib{i}"),
            name: path
                .file_name()
                .map_or_else(|| "library".into(), |n| n.to_string_lossy().into_owned()),
            path,
        })
        .collect()
}

/// The main window, from its `tauri.conf.json` entry (`"create": false`, so it can take a
/// new-window handler). A page asking for a new window (a `target="_blank"` link,
/// `window.open`, also from the news iframe: docs/NEWS.md) gets the system browser instead;
/// the webview never opens a second window of its own.
fn create_main_window(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == "main")
        .cloned()
        .ok_or("no \"main\" window in tauri.conf.json")?;
    tauri::WebviewWindowBuilder::from_config(app.handle(), &config)?
        .on_new_window(|url, _features| {
            if external_link(&url)
                && let Err(e) = open_in_browser(url.as_str())
            {
                tracing::warn!(%e, "could not open a link in the browser");
            }
            tauri::webview::NewWindowResponse::Deny
        })
        .build()?;
    Ok(())
}

/// Links that may leave for the system browser (never `file:`, custom schemes or `javascript:`).
fn external_link(url: &tauri::Url) -> bool {
    matches!(url.scheme(), "https" | "http" | "mailto")
}

/// Open `url` in the system browser (`target="_blank"` links never open a second webview;
/// [`create_main_window`]).
fn open_in_browser(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut c = std::process::Command::new("open");
    #[cfg(windows)]
    let mut c = {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut c = std::process::Command::new("xdg-open");
    c.arg(url).spawn().map(drop)
}

/// Entry point shared by the binary (and mobile targets, should they ever exist).
#[cfg_attr(mobile, tauri::mobile_entry_point)]
/// Runs the desktop app.
///
/// Excluded from the test build: its `generate_context!` and the test context's would both emit
/// the macOS `_EMBED_INFO_PLIST` symbol, which fails to link ("already defined").
#[cfg(not(test))]
pub fn run() {
    let _ = MAIN_THREAD.set(std::thread::current().id());
    init_tracing();

    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default();
    // `quit`: ⌘Q asks first; ⌘W closes the key plugin editor, never the main window.
    #[cfg(target_os = "macos")]
    {
        builder = builder
            .menu(quit::menu::build)
            .on_menu_event(quit::menu::on_event);
    }
    let app = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        // `ai-chat`: provider requests from Rust (no CORS), scoped (see `ai_http`).
        .plugin(tauri_plugin_http::init())
        .manage(ai_http::AiHttpScope::default())
        .manage(HostSlot::default())
        .manage(quit::QuitGate::default())
        .setup(|app| {
            create_main_window(app)?;
            let instance = instance::instance_id();
            let data_dir = instance::app_data_dir(app.handle())?;
            if let Err(e) = log_file::open(&data_dir.join("logs")) {
                tracing::warn!(%e, "could not create the log file");
            }
            let projects_root =
                ether_native::default_projects_root(cfg!(debug_assertions), &data_dir);
            std::fs::create_dir_all(&projects_root)?;
            tracing::info!(
                %instance,
                data_dir = %data_dir.display(),
                projects_root = %projects_root.display(),
                "ethereal desktop starting"
            );
            let host = NativeHost::start(
                HostConfig {
                    audio: None,
                    data_dir: data_dir.clone(),
                    instance: instance.clone(),
                    projects_root: projects_root.clone(),
                    library_roots: library_roots(&data_dir),
                },
                HostOptions {
                    main_thread: plugin_main_thread(app.handle()),
                    ..HostOptions::default()
                },
            )
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
            tracing::info!(audio = ?host.audio_info(), "engine started");
            let host = Arc::new(host);
            // `agent-api`: everything the engine emits goes through the fan-out (UI, and
            // the agent bridge when enabled).
            let bridge = AgentBridgeState::new(data_dir.clone(), instance.clone());
            {
                let fanout = bridge.fanout.clone();
                host.subscribe(Arc::new(move |m| fanout.deliver(m)));
            }
            bridge.restore(host.clone());
            app.manage(bridge);
            if let Ok(mut slot) = app.state::<HostSlot>().0.lock() {
                *slot = Some(host);
            }
            app.manage(AppPaths {
                instance,
                data_dir,
                projects_root,
            });
            // `file-import`: OS file drops reach the UI as paths (macOS).
            if let Some(window) = app.get_webview_window("main") {
                // `plugin-editor-owner`: plugin editor windows stay above the main window.
                let owner = editor_owner::OwnerWindow::from_window(&window);
                if owner.is_none() {
                    tracing::info!("plugin editors not tied to the main window (no usable handle)");
                }
                editor_owner::set_owner(owner);
                path_drop::install(app.handle(), &window);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            match event {
                // `plugin-editor-owner`: editors not owned by the main window (made on
                // another thread) are put back above it when it is activated.
                WindowEvent::Focused(true) => editor_owner::owner_activated(),
                // `quit`: closing the main window quits; ask first.
                WindowEvent::CloseRequested { api, .. } => {
                    let app = window.app_handle();
                    let gate = app.state::<quit::QuitGate>();
                    if gate.request(std::time::Instant::now()) == quit::QuitAction::Ask {
                        api.prevent_close();
                        quit::ask(app);
                    }
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            engine_info,
            ether_connect,
            ether_send,
            ether_send_batch,
            ether_disconnect,
            collab_token_load,
            collab_token_save,
            agent_bridge_status,
            agent_bridge_set_enabled,
            ai_http::ai_http_allow,
            log_file::open_engine_log,
            quit::app_quit_guard,
            quit::app_quit,
            quit::app_quit_cancel,
            app_menu::menu_set_shortcuts,
            app_menu::menu_native_edit,
            app_menu::menu_set_labels
        ])
        .build(tauri::generate_context!())
        .expect("error while building the Ethereal desktop app");

    app.run(|app, event| match event {
        // `quit`: the last window closing is asked about; exits with a code (`app_quit`,
        // the updater's restart) are not.
        RunEvent::ExitRequested { code, api, .. } => {
            if app
                .state::<quit::QuitGate>()
                .holds_exit(code, std::time::Instant::now())
            {
                api.prevent_exit();
                quit::ask(app);
            }
        }
        RunEvent::Exit => shutdown(app),
        _ => {}
    });
}

/// Orderly shutdown (on exit, on restart, and before the Windows update installer takes
/// over): stop the agent bridge, then the engine. Idempotent.
pub(crate) fn shutdown(app: &AppHandle) {
    // Stop the agent bridge first (deletes its runtime file, releases the host), without
    // changing the persisted setting.
    if let Some(bridge) = app.try_state::<AgentBridgeState>()
        && let Err(e) = bridge.set_running(None, false)
    {
        tracing::warn!(%e, "agent bridge");
    }
    let host = app
        .try_state::<HostSlot>()
        .and_then(|slot| slot.0.lock().ok().and_then(|mut s| s.take()));
    match host.map(Arc::try_unwrap) {
        Some(Ok(host)) => host.shutdown(),
        // Still referenced elsewhere: the last reference stops it on drop.
        Some(Err(host)) => drop(host),
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::external_link;

    #[test]
    fn only_web_and_mail_links_leave_for_the_browser() {
        let ok = |s: &str| external_link(&tauri::Url::parse(s).unwrap());
        assert!(ok("https://ethereal.ws/news"));
        assert!(ok("http://example.com"));
        assert!(ok("mailto:hi@example.com"));
        assert!(!ok("file:///etc/passwd"));
        assert!(!ok("javascript:alert(1)"));
        assert!(!ok("tauri://localhost/index.html"));
        assert!(!ok("ethereal://join/x"));
    }
}

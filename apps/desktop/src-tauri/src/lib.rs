//! Ethereal desktop shell (Tauri v2): thin glue over `ether-native`.
//!
//! # IPC (backs `ui/src/transport/tauri/TauriTransport.ts`)
//! - `ether_connect(messages, playhead, meters)`: registers three Tauri `Channel`s and
//!   returns [`EngineInfo`]. Replies and events (`ServerMessage::Reply | Event`) go to
//!   `messages`, in order (so a command's patches arrive before its reply); `Playhead`
//!   frames (~60 Hz) and `Meters` frames (~30 Hz) go to their own channels. Calling it again
//!   (UI reload) replaces the previous channels.
//! - `ether_send(message)`: queue one `ClientMessage` (never blocks; the reply arrives on
//!   `messages`).
//! - `ether_disconnect()`: drop the channels.
//! - `engine_info()`: instance/data dir/audio diagnostics.
//!
//! # Threads
//! The host runs its own controller, GC and audio threads. CLAP plugin controllers run on
//! the **process main thread** through [`TauriMainThread`] (`AppHandle::run_on_main_thread`),
//! as AppKit requires for plugin editor windows on macOS.

pub mod instance;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ether_core::protocol::message::PlayheadFrame;
use ether_core::protocol::meters::MeterFrame;
use ether_core::protocol::{ClientMessage, ServerMessage};
use ether_native::host::{HostConfig, HostOptions};
use ether_native::{LibraryRoot, MainThread, NativeHost};
use serde::Serialize;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, RunEvent};

/// Resolved per-process paths, managed by Tauri.
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub instance: String,
    pub data_dir: PathBuf,
    pub projects_root: PathBuf,
}

/// The running host (taken on exit for an orderly shutdown).
#[derive(Default)]
pub struct HostSlot(Mutex<Option<NativeHost>>);

impl HostSlot {
    fn with<R>(&self, f: impl FnOnce(&NativeHost) -> R) -> Result<R, String> {
        let guard = self.0.lock().map_err(|_| "host lock poisoned".to_string())?;
        guard
            .as_ref()
            .map(f)
            .ok_or_else(|| "engine is not running".to_string())
    }
}

/// Runs plugin main-thread work on the process main thread (Tauri's event loop).
pub struct TauriMainThread(AppHandle);

impl MainThread for TauriMainThread {
    fn spawn(&self, f: Box<dyn FnOnce() + Send>) {
        if let Err(e) = self.0.run_on_main_thread(f) {
            tracing::error!(%e, "run_on_main_thread failed");
        }
    }
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

#[tauri::command]
fn ether_connect(
    paths: tauri::State<'_, AppPaths>,
    host: tauri::State<'_, HostSlot>,
    messages: Channel<ServerMessage>,
    playhead: Channel<PlayheadFrame>,
    meters: Channel<MeterFrame>,
) -> Result<EngineInfo, String> {
    host.with(|h| {
        h.subscribe(Arc::new(move |m| {
            let r = match m {
                ServerMessage::Playhead(p) => playhead.send(p),
                ServerMessage::Meters(f) => meters.send(f),
                other => messages.send(other),
            };
            if let Err(e) = r {
                tracing::debug!(%e, "channel send failed");
            }
        }));
        info(&paths, h)
    })
}

#[tauri::command]
fn ether_send(host: tauri::State<'_, HostSlot>, message: ClientMessage) -> Result<(), String> {
    host.with(|h| h.send(message).map_err(|e| e.to_string()))?
}

#[tauri::command]
fn ether_disconnect(host: tauri::State<'_, HostSlot>) -> Result<(), String> {
    host.with(|h| h.unsubscribe())
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    // `try_init` so tests / repeated init don't panic.
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

/// Library folders for the sample browser: `ETHER_LIBRARY` (OS path-list), else the
/// user's Music folder if it exists.
fn library_roots(app: &AppHandle) -> Vec<LibraryRoot> {
    if let Some(list) = std::env::var_os("ETHER_LIBRARY") {
        return std::env::split_paths(&list)
            .enumerate()
            .filter(|(_, p)| p.is_dir())
            .map(|(i, path)| LibraryRoot {
                id: format!("lib{i}"),
                name: path
                    .file_name()
                    .map_or_else(|| "Library".into(), |n| n.to_string_lossy().into_owned()),
                path,
            })
            .collect();
    }
    app.path()
        .audio_dir()
        .ok()
        .filter(|p| p.is_dir())
        .map(|path| LibraryRoot {
            id: "music".into(),
            name: "Music".into(),
            path,
        })
        .into_iter()
        .collect()
}

/// Entry point shared by the binary (and mobile targets, should they ever exist).
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .manage(HostSlot::default())
        .setup(|app| {
            let instance = instance::instance_id();
            let data_dir = instance::app_data_dir(app.handle())?;
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
                    library_roots: library_roots(app.handle()),
                },
                HostOptions {
                    main_thread: Arc::new(TauriMainThread(app.handle().clone())),
                    ..HostOptions::default()
                },
            )
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
            tracing::info!(audio = ?host.audio_info(), "engine started");
            if let Ok(mut slot) = app.state::<HostSlot>().0.lock() {
                *slot = Some(host);
            }
            app.manage(AppPaths {
                instance,
                data_dir,
                projects_root,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            engine_info,
            ether_connect,
            ether_send,
            ether_disconnect
        ])
        .build(tauri::generate_context!())
        .expect("error while building the Ethereal desktop app");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            let host = app
                .state::<HostSlot>()
                .0
                .lock()
                .ok()
                .and_then(|mut s| s.take());
            if let Some(host) = host {
                host.shutdown();
            }
        }
    });
}

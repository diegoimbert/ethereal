//! Ethereal desktop shell (Tauri v2).
//!
//! OWNERSHIP: the `native-host` node owns `apps/desktop/**`. It will wire `ether-native`
//! (cpal RT thread, snapshot swap, GC thread, disk streaming) and expose the Tauri
//! commands/channels backing `TauriTransport` (the desktop `EngineTransport`).
//! Until then this is a thin shell with a placeholder `engine_info` command.

pub mod instance;

use std::path::PathBuf;

use serde::Serialize;
use tauri::Manager;

/// Resolved per-process state, managed by Tauri.
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub instance: String,
    pub data_dir: PathBuf,
}

#[derive(Debug, Serialize)]
pub struct EngineInfo {
    pub version: &'static str,
    pub instance: String,
    pub data_dir: String,
}

/// Placeholder until `native-host` wires `ether-native`.
#[tauri::command]
fn engine_info(paths: tauri::State<'_, AppPaths>) -> EngineInfo {
    EngineInfo {
        version: env!("CARGO_PKG_VERSION"),
        instance: paths.instance.clone(),
        data_dir: paths.data_dir.display().to_string(),
    }
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    // `try_init` so tests / repeated init don't panic.
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

/// Entry point shared by the binary (and mobile targets, should they ever exist).
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            let instance = instance::instance_id();
            let data_dir = instance::app_data_dir(app.handle())?;
            tracing::info!(%instance, data_dir = %data_dir.display(), "ethereal desktop starting");
            app.manage(AppPaths { instance, data_dir });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![engine_info])
        .run(tauri::generate_context!())
        .expect("error while running the Ethereal desktop app");
}

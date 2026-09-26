//! CLAP plugin hosting via `clack` (native only).
//!
//! Provides the in-process implementation of `ether_core::PluginController` /
//! `ether_core::PluginNode`, and the bundle scanning used by `ether-plugin-scanner` (the
//! app itself never loads a plugin for scanning; that always happens out-of-process).
//! Owned by the `clap` node (adds `clack-host`/`clack-extensions` from the workspace).
#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};

use ether_core::plugin::{PluginController, PluginError};
use ether_core::protocol::plugins::PluginDescriptor;

/// Platform default CLAP search paths (+ `CLAP_PATH`).
pub fn default_search_paths() -> Vec<PathBuf> {
    todo!("clap node")
}

/// Enumerate `.clap` bundles under `paths` (no loading).
pub fn find_bundles(paths: &[PathBuf]) -> Vec<PathBuf> {
    let _ = paths;
    todo!("clap node")
}

/// Load one bundle and list its plugins. Called ONLY inside the scanner process.
pub fn scan_bundle(bundle: &Path) -> Result<Vec<PluginDescriptor>, PluginError> {
    let _ = bundle;
    todo!("clap node")
}

/// Instantiate a plugin in-process (main thread).
pub fn instantiate(
    bundle: &Path,
    plugin_id: &str,
) -> Result<Box<dyn PluginController>, PluginError> {
    let _ = (bundle, plugin_id);
    todo!("clap node")
}

//! CLAP plugin hosting via `clack` (native only).
//!
//! Provides the in-process implementation of `ether_core::PluginController` ([`ClapPlugin`])
//! / `ether_core::PluginNode` ([`ClapNode`]), and the bundle scanning used by
//! `ether-plugin-scanner` (the app itself never loads a plugin for scanning; that always
//! happens out-of-process, driven by [`ScanRunner`]).
//!
//! # Threading
//! A [`ClapPlugin`] is `!Send` and must live on the thread that runs the plugin's CLAP
//! main-thread callbacks; call [`PluginController::poll`] from it regularly (~30-60 Hz).
//! Floating editor windows additionally need that thread to be the OS main thread on macOS.
//! The [`ClapNode`] returned by `activate` is `Send` and goes to the engine.
//!
//! # Parameters
//! CLAP parameter values are plain values, as in the document, so `ParamId` = CLAP param id
//! and values pass through unchanged (`ParamScale::Linear`). While active, parameter changes
//! reach the plugin as sample-accurate `EventKind::Param` events (or `Device::set_param`);
//! changes made in the plugin's GUI come back from [`PluginController::poll`] as
//! `ParamEdited` + gesture notifications.
#![cfg(not(target_arch = "wasm32"))]

mod host;
mod node;
mod params;
mod plugin;
pub mod scan;
#[doc(hidden)]
pub mod testing;

use std::path::Path;

use ether_core::plugin::{PluginController, PluginError};

pub use node::ClapNode;
pub use plugin::ClapPlugin;
pub use scan::{
    ScanReport, ScanRunner, category_from_features, default_search_paths, find_bundles, scan_bundle,
};

/// Instantiate a plugin in-process (main thread).
pub fn instantiate(
    bundle: &Path,
    plugin_id: &str,
) -> Result<Box<dyn PluginController>, PluginError> {
    Ok(Box::new(ClapPlugin::load(bundle, plugin_id)?))
}

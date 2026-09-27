//! Out-of-process plugin sandbox (native only).
//!
//! [`spawn`] launches `ether-sandbox-helper`, which loads the plugin (CLAP, VST3 or AU, per
//! `SandboxOptions.format` / `--format`) through its `PluginFormatHost`; the
//! returned [`SandboxedPlugin`] / [`SandboxedNode`] implement the same `PluginController` /
//! `PluginNode` contracts as the in-process host and proxy everything to the helper:
//! - control (params, state, editor, notifications): length-prefixed JSON frames over the
//!   helper's stdin/stdout (`wire`), each request with a timeout (a hung helper is killed);
//! - audio + events: a shared-memory region (`shm`), one block in flight;
//! - block sync: a POSIX named semaphore (host → helper "process"); the helper publishes
//!   completion with an atomic in shared memory which the host checks with a bounded spin,
//!   so the audio thread never blocks. On a miss it outputs silence and counts an underrun;
//! - exactly `max_block_size` samples of extra latency, reported via `Node::latency` (on top
//!   of the plugin's own) so PDC compensates;
//! - helper crash/hang → the node outputs silence and is faulted, `poll` reports
//!   `PluginNotification::Crashed`; the host process is unaffected.
//!
//! All OS object names come from `ether_core::plugin::ipc_name(instance, pid, ...)` (hashed to
//! fit macOS' 31-byte limit), a stale object with the same name is removed before creation,
//! and the names are unlinked as soon as the helper has opened them, so nothing named
//! outlives a crash of either process.
//!
//! Platforms: macOS and Linux. Elsewhere (Windows) [`spawn`] returns an error for now.
//! Owned by the `sandbox` node.
#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};

use ether_core::plugin::{PluginController, PluginError};

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[doc(hidden)]
pub mod helper;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod host;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod node;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod shm;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod sys;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod wire;

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use host::{SandboxOptions, SandboxedPlugin};
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use node::SandboxedNode;

/// Name of the helper executable.
pub const HELPER_NAME: &str = "ether-sandbox-helper";

/// Where the helper binary lives: `$ETHER_SANDBOX_HELPER` if set, else next to the current
/// executable (or one directory up, for test binaries in `target/<profile>/deps`).
pub fn helper_path() -> PathBuf {
    if let Some(p) = std::env::var_os("ETHER_SANDBOX_HELPER") {
        return PathBuf::from(p);
    }
    let file = format!("{HELPER_NAME}{}", std::env::consts::EXE_SUFFIX);
    let exe = std::env::current_exe().unwrap_or_default();
    let mut candidates = exe.ancestors().skip(1).take(2).map(|d| d.join(&file));
    let first = candidates.next().unwrap_or_else(|| PathBuf::from(&file));
    std::iter::once(first.clone())
        .chain(candidates)
        .find(|p| p.is_file())
        .unwrap_or(first)
}

/// Start a sandboxed instance of `plugin_id` from `bundle` with default options. `instance`
/// is the dev instance id used in IPC names.
pub fn spawn(
    bundle: &Path,
    plugin_id: &str,
    instance: &str,
) -> Result<Box<dyn PluginController>, PluginError> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        Ok(Box::new(SandboxedPlugin::spawn(
            bundle,
            plugin_id,
            instance,
            SandboxOptions::default(),
        )?))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (bundle, plugin_id, instance);
        Err(PluginError::Ipc(
            "out-of-process plugins are not supported on this platform yet".into(),
        ))
    }
}

//! Out-of-process plugin sandbox (native only).
//!
//! [`spawn`] launches `ether-sandbox-helper`, which loads the plugin via `ether-clap`; the
//! returned controller/node proxy everything over IPC:
//! - audio + events: a shared-memory region (double-buffered block I/O),
//! - block sync: two cross-process semaphores (host → helper "process", helper → host
//!   "done"); the host never waits past the block deadline: on a miss it outputs silence,
//! - control (state, params, editor): a message pipe,
//! - +1 block latency reported via `Node::latency` so PDC compensates,
//! - crash/timeout → `PluginNode::is_faulted()` + `PluginNotification::Crashed`.
//!
//! All OS object names come from `ether_core::plugin::ipc_name(instance, pid, ...)` so
//! parallel app instances never collide. Owned by the `sandbox` node.
#![cfg(not(target_arch = "wasm32"))]

use std::path::Path;

use ether_core::plugin::{PluginController, PluginError};

/// Where the helper binary lives (next to the app executable by default).
pub fn helper_path() -> std::path::PathBuf {
    todo!("sandbox node")
}

/// Start a sandboxed instance of `plugin_id` from `bundle`. `instance` is the dev instance
/// id used in IPC names.
pub fn spawn(
    bundle: &Path,
    plugin_id: &str,
    instance: &str,
) -> Result<Box<dyn PluginController>, PluginError> {
    let _ = (bundle, plugin_id, instance);
    todo!("sandbox node")
}

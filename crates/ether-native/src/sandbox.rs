//! Out-of-process plugin hosting (owned by the `plugins` wave-3 node; see `docs/WAVE3.md`).
//!
//! `NativeBridge::create_plugin` (`bridge.rs`) picks [`instantiate`] instead of the
//! in-process instantiator when `PluginInstance.sandboxed` is set. The sandboxed controller
//! (`ether_sandbox::SandboxedPlugin`) then goes through the same main-thread registry as an
//! in-process one ([`crate::plugins::PluginHost`]): state restore, activation, polling,
//! editor, retirement. Its node reports +1 block of latency (PDC) and, if the helper dies,
//! it is faulted: [`crate::plugins::HostedPluginNode`] then bypasses it (dry signal, delayed
//! by the reported latency so PDC stays aligned) and the controller reports
//! `PluginEvent::Crashed` until `PluginCommand::Reload`.
//!
//! The helper binary is found by `ether_sandbox::helper_path()`: `$ETHER_SANDBOX_HELPER`,
//! else next to the running executable (the desktop bundle ships it there). Tests (and
//! embedders) can override it with [`set_helper_path`].

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use ether_core::plugin::{PluginController, PluginError};

use crate::plugins::Instantiate;

static HELPER_OVERRIDE: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Use `path` as the sandbox helper executable for every later sandboxed instance
/// (`None` restores the default lookup).
pub fn set_helper_path(path: Option<PathBuf>) {
    if let Ok(mut p) = HELPER_OVERRIDE.write() {
        *p = path;
    }
}

/// The helper executable sandboxed instances will use.
pub fn helper_path() -> PathBuf {
    HELPER_OVERRIDE
        .read()
        .ok()
        .and_then(|p| p.clone())
        .unwrap_or_else(ether_sandbox::helper_path)
}

/// Start `plugin_id` from `bundle` in a sandbox helper process (main thread).
pub fn spawn(bundle: &Path, plugin_id: &str) -> Result<Box<dyn PluginController>, PluginError> {
    let instance = crate::instance::instance_id();
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let options = ether_sandbox::SandboxOptions {
            helper: helper_path(),
            ..ether_sandbox::SandboxOptions::default()
        };
        Ok(Box::new(ether_sandbox::SandboxedPlugin::spawn(
            bundle, plugin_id, &instance, options,
        )?))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        ether_sandbox::spawn(bundle, plugin_id, &instance)
    }
}

/// Instantiator for sandboxed plugins (see `NativeBridge::create_plugin`).
pub fn instantiate() -> Instantiate {
    Arc::new(spawn)
}

/// Pick the instantiator for a plugin instance: the sandbox, or the in-process one.
pub fn instantiator(sandboxed: bool, in_process: &Instantiate) -> Instantiate {
    if sandboxed {
        instantiate()
    } else {
        in_process.clone()
    }
}

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
//! Every format can be sandboxed: the helper loads the plugin through the same
//! `PluginFormatHost` as the in-process host (`--format clap|vst3|au`). AUv3 extensions
//! already run out of process (Apple's XPC bridge); sandboxing one is allowed anyway (the
//! helper then hosts the `AUAudioUnit` proxy) because it is harmless, keeps the toggle
//! uniform across formats, and still isolates in-process v2 units and the AU host code.
//! It adds the usual +1 block of latency.
//!
//! The helper binary is found by `ether_sandbox::helper_path()`: `$ETHER_SANDBOX_HELPER`,
//! else next to the running executable (the desktop bundle ships it there). Tests (and
//! embedders) can override it with [`set_helper_path`].

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, RwLock};

use ether_core::plugin::{PluginController, PluginError};
use ether_core::protocol::model::PluginFormat;

use crate::plugins::Instantiate;

static HELPER_OVERRIDE: RwLock<Option<PathBuf>> = RwLock::new(None);
/// Pid of the most recently spawned helper (diagnostics and crash tests).
static LAST_HELPER_PID: AtomicU32 = AtomicU32::new(0);

/// Pid of the most recently spawned sandbox helper process, if any.
pub fn last_helper_pid() -> Option<u32> {
    Some(LAST_HELPER_PID.load(Ordering::SeqCst)).filter(|p| *p != 0)
}

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

/// Start the `format` plugin `plugin_id` from `bundle` (the component id for AUs) in a
/// sandbox helper process (main thread). The helper loads it with `--format`.
pub fn spawn(
    format: PluginFormat,
    bundle: &Path,
    plugin_id: &str,
) -> Result<Box<dyn PluginController>, PluginError> {
    let instance = crate::instance::instance_id();
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let options = ether_sandbox::SandboxOptions {
            helper: helper_path(),
            format,
            ..ether_sandbox::SandboxOptions::default()
        };
        let plugin = ether_sandbox::SandboxedPlugin::spawn(bundle, plugin_id, &instance, options)?;
        LAST_HELPER_PID.store(plugin.helper_pid(), Ordering::SeqCst);
        Ok(Box::new(plugin))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = format;
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

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
//!
//! The audio thread waits at most `ether_sandbox::default_wait_budget` (a quarter of a
//! max-size block) for the helper's result before it plays silence and counts an underrun.
//! Offline tests can lengthen that with [`set_wait_budget`] so a CPU-starved helper on a
//! loaded machine doesn't fail their audio assertions; the product never calls it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use ether_core::plugin::{PluginController, PluginError};
use ether_core::protocol::model::PluginFormat;

use crate::plugins::Instantiate;

static HELPER_OVERRIDE: RwLock<Option<PathBuf>> = RwLock::new(None);
/// Test-only override of the audio thread's wait budget (`None` = the production default).
static WAIT_BUDGET_OVERRIDE: RwLock<Option<Duration>> = RwLock::new(None);
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

/// Tests only: let the audio thread of every later sandboxed instance wait up to `budget`
/// for the helper's block (`None` restores the production default, a quarter of a
/// max-size block). Offline renders use a long budget so they don't depend on the helper
/// being scheduled within ~1 ms. The product never calls this.
pub fn set_wait_budget(budget: Option<Duration>) {
    if let Ok(mut b) = WAIT_BUDGET_OVERRIDE.write() {
        *b = budget;
    }
}

/// Options for a sandboxed `format` instance: the default ones, plus the helper and wait
/// budget overrides.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn options(format: PluginFormat) -> ether_sandbox::SandboxOptions {
    ether_sandbox::SandboxOptions {
        helper: helper_path(),
        format,
        wait_budget: WAIT_BUDGET_OVERRIDE.read().ok().and_then(|b| *b),
        ..ether_sandbox::SandboxOptions::default()
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
        let plugin =
            ether_sandbox::SandboxedPlugin::spawn(bundle, plugin_id, &instance, options(format))?;
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

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests {
    use super::*;

    /// Nothing in the product sets the override: sandboxed instances get the production
    /// wait budget (a quarter of a max-size block).
    #[test]
    fn production_wait_budget_is_the_default() {
        assert_eq!(options(PluginFormat::Clap).wait_budget, None);
        let config = ether_core::config::PrepareConfig {
            sample_rate: 48_000.0,
            max_block_size: 256,
            max_events_per_block: 256,
        };
        let budget = ether_sandbox::default_wait_budget(&config);
        assert!((budget.as_secs_f64() - 256.0 / 48_000.0 / 4.0).abs() < 1e-9);

        set_wait_budget(Some(Duration::from_secs(10)));
        assert_eq!(
            options(PluginFormat::Vst3).wait_budget,
            Some(Duration::from_secs(10))
        );
        set_wait_budget(None);
        assert_eq!(options(PluginFormat::Clap).wait_budget, None);
    }
}

//! CLAP plugins: scanning, editors, sandboxing. Also the scanner-process wire format.
//!
//! Plugins are native-only; the web host replies `Unsupported`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::devices::DeviceCategory;
use crate::model::{DeviceId, PluginFormat};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum PluginCommand {
    /// Re-scan the plugin search paths out-of-process. Progress via `Event::Plugin`.
    Rescan,
    /// Replies `Plugins` from the cached plugin DB.
    List,
    /// Open the plugin's floating editor window.
    OpenEditor {
        device: DeviceId,
    },
    CloseEditor {
        device: DeviceId,
    },
    /// Move an instance in/out of the sandbox (re-instantiates with its current state).
    SetSandboxed {
        device: DeviceId,
        sandboxed: bool,
    },
    /// Re-instantiate a crashed plugin from its last saved state.
    Reload {
        device: DeviceId,
    },
}

/// One plugin found by the scanner.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PluginDescriptor {
    pub format: PluginFormat,
    /// CLAP id.
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub description: String,
    /// CLAP feature strings (`instrument`, `audio-effect`, `reverb`, ...).
    pub features: Vec<String>,
    pub category: DeviceCategory,
    /// Bundle path on disk.
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum PluginEvent {
    ScanProgress {
        done: u32,
        total: u32,
        current: Option<String>,
    },
    ScanFinished {
        plugins: u32,
        failed: Vec<ScanFailure>,
    },
    EditorClosed {
        device: DeviceId,
    },
    /// The plugin crashed (sandboxed) or faulted; its node is bypassed until `Reload`.
    Crashed {
        device: DeviceId,
        message: String,
    },
    /// Reported latency changed (PDC re-computed).
    LatencyChanged {
        device: DeviceId,
        samples: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ScanFailure {
    pub path: String,
    pub message: String,
}

/// Scanner process protocol: the host runs `ether-plugin-scanner` with a JSON `ScanRequest`
/// on stdin and reads one JSON `ScanResponse` from stdout. One bundle per process (a crash
/// only loses that bundle).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ScanRequest {
    pub bundle_path: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ScanResponse {
    Ok { plugins: Vec<PluginDescriptor> },
    Err { message: String },
}

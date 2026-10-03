//! Plugins (CLAP, VST3, AU): scanning, editors, sandboxing. Also the scanner-process wire
//! format. Per-format id rules: `ether_model::PluginFormat` and `docs/PLUGIN-FORMATS.md`.
//!
//! Plugins are native-only; the web host replies `Unsupported`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::devices::DeviceCategory;
use crate::model::{DeviceId, PluginFormat};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum PluginCommand {
    /// Re-scan the plugin folders out-of-process. Progress via `Event::Plugin`. Only new
    /// or changed plugins are loaded again (the rest come from the scan cache); `full`
    /// ignores the cache and loads every plugin (also retries failed ones).
    Rescan {
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        #[ts(as = "Option<bool>", optional)]
        full: bool,
    },
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
    /// Replies `PluginFolders`: the folders scanned for plugins.
    ListFolders,
    /// Add a user plugin folder (`format`: only that format's plugins; `null` = any) and
    /// rescan (incrementally). Adding a folder already listed updates its format filter.
    /// Replies `PluginFolders`.
    AddFolder {
        path: String,
        format: Option<PluginFormat>,
    },
    /// Remove a user plugin folder and rescan. Replies `PluginFolders`.
    RemoveFolder {
        path: String,
    },
    /// Whether the OS default plugin folders (and, for AU, the system component registry)
    /// are scanned; on by default. Rescans. Replies `PluginFolders`.
    SetIncludeDefaults {
        include: bool,
    },
}

/// The folders scanned for plugins (`Plugin::ListFolders`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct PluginFolders {
    /// Scan the OS default folders (`defaults`).
    pub include_defaults: bool,
    /// The OS default folders of each format (read-only; may not exist), including the
    /// `CLAP_PATH`/`VST3_PATH`-style environment overrides.
    pub defaults: Vec<DefaultPluginFolder>,
    /// Folders the user added, in the order added.
    pub folders: Vec<PluginFolder>,
}

/// A user plugin folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct PluginFolder {
    pub path: String,
    /// Only plugins of this format are looked for here; `null` = any format.
    pub format: Option<PluginFormat>,
}

/// An OS default plugin folder of one format.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct DefaultPluginFolder {
    pub path: String,
    pub format: PluginFormat,
    /// The folder exists on this machine.
    pub exists: bool,
}

/// One plugin found by the scanner.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PluginDescriptor {
    pub format: PluginFormat,
    /// Format-specific plugin id (`PluginInstance.plugin_id`; convention per format in
    /// `ether_model::PluginFormat`): CLAP id, VST3 class id (32 hex), AU `type:subtype:manu`.
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub description: String,
    /// Feature/category strings, lowercase. CLAP feature strings as-is (`instrument`,
    /// `audio-effect`, `reverb`, ...); VST3 sub-categories split on `|` (`fx`, `instrument`,
    /// `delay`, ...); AU the component type (`aufx`, `aumu`, `aumf`, `aumi`).
    pub features: Vec<String>,
    pub category: DeviceCategory,
    /// Where the host loads the plugin from: the `.clap`/`.vst3` bundle path on disk. AU:
    /// the scan target (the component id, same as `id`), since AUs are instantiated from
    /// the system component registry, not from a path.
    pub path: String,
    /// v0.2 (`plugin-sidechain`): channels of the plugin's aux/sidechain input bus found at
    /// scan (CLAP second input audio port, VST3 `kAux` input bus, AU input bus 1); 0 = none.
    /// The instance's `DeviceDescriptor::sidechain_inputs` (from the bus layout at
    /// instantiation) is authoritative. Older catalogs omit it (= 0).
    #[serde(default)]
    pub sidechain_inputs: u16,
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
/// on stdin and reads one JSON `ScanResponse` from stdout. One scan target per process (a
/// crash only loses that target).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ScanRequest {
    /// The scan target: a `.clap`/`.vst3` bundle path, or for AU a component id
    /// (`type:subtype:manufacturer`, from the in-process registry listing).
    pub bundle_path: String,
    /// Format of the target. Omitted/`null` = inferred from the path's extension (`.clap`,
    /// `.vst3`), which is what hosts older than VST3/AU support sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub format: Option<PluginFormat>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ScanResponse {
    Ok { plugins: Vec<PluginDescriptor> },
    Err { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rescan_full_is_optional_on_the_wire() {
        // Clients that predate `full` keep sending (and receiving) the bare variant.
        let bare: PluginCommand = serde_json::from_str(r#"{"type":"Rescan"}"#).unwrap();
        assert_eq!(bare, PluginCommand::Rescan { full: false });
        assert_eq!(
            serde_json::to_string(&bare).unwrap(),
            r#"{"type":"Rescan"}"#
        );
        let full = PluginCommand::Rescan { full: true };
        assert_eq!(
            serde_json::to_string(&full).unwrap(),
            r#"{"type":"Rescan","full":true}"#
        );
    }
}

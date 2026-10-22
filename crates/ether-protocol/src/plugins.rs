//! Plugins (CLAP, VST3, AU): scanning, editors, sandboxing. Also the scanner-process wire
//! format. Per-format id rules: `ether_model::PluginFormat` and `docs/PLUGIN-FORMATS.md`.
//!
//! Plugins are native-only; the web host replies `Unsupported`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::devices::DeviceCategory;
use crate::model::{DeviceId, ParamId, PluginFormat};

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
    /// Replies `SandboxDefaults`: the plugins this machine inserts sandboxed by default
    /// (settings, kept by the engine host next to its plugin list). A new instance of one
    /// (`Device::Insert` and the like with `sandboxed: null`, a replacement by another plugin,
    /// an imported project's plugins) starts sandboxed; each instance keeps its own flag after
    /// that (`SetSandboxed`, saved in the project), and editing the list never changes
    /// existing instances. Native host only (the web replies `Unsupported`).
    ListSandboxDefaults,
    /// Add (`sandboxed: true`) or remove a plugin from `ListSandboxDefaults`, by format and
    /// plugin id (ids compare case-insensitively). `name` is shown in settings (kept for a
    /// plugin that is no longer installed). Replies `SandboxDefaults`.
    SetSandboxDefault {
        format: PluginFormat,
        plugin_id: String,
        #[serde(default)]
        #[ts(optional)]
        name: Option<String>,
        sandboxed: bool,
    },
    /// Keys the host's plugin editor windows forward to the app (as `PluginEvent::EditorKey`)
    /// instead of giving them to the plugin, besides `Space` (always forwarded, the
    /// transport). `codes` are `KeyboardEvent.code` values (`"KeyA"`, `"Digit1"`, ...);
    /// replaces the previous set. A key is forwarded only without Ctrl / Alt / Cmd (plugin
    /// shortcuts keep working); Shift is allowed. Not undoable, not saved. Native host only
    /// (the web replies `Unsupported`).
    SetEditorKeyCapture {
        codes: Vec<String>,
    },
    /// Replies `ScanFailures`: the scan targets that failed to load in the last scan (also
    /// known after a restart, from the scan cache), updated by `RetryScan`.
    ListScanFailures,
    /// Scan one target again (a failed one from `ListScanFailures`), out-of-process with a
    /// longer timeout: `timeout_ms` (default 90 s; the host clamps it to 1 s ..= 10 min).
    /// Runs off the controller thread and replies `ScanRetried` when that scan ends. On
    /// success its plugins join the list (and the scan cache); either way a `ScanFinished`
    /// event with the updated failure list follows.
    RetryScan {
        path: String,
        format: PluginFormat,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        timeout_ms: Option<u32>,
    },
    /// Replies `ParamTexts`: the plugin's own display text for each plain value (`"-6.0 dB"`,
    /// `"Saw"`), for the generic device UI. Not an edit. `null` entries = no text (the UI
    /// formats the number): unknown param, a plugin without text, or (VST2, which only
    /// describes its current value) a value the plugin doesn't hold yet. Native host only
    /// (the web replies `Unsupported`).
    ParamTexts {
        device: DeviceId,
        values: Vec<PluginParamValue>,
    },
}

/// A plain value of one plugin param (`PluginCommand::ParamTexts`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PluginParamValue {
    pub param: ParamId,
    pub value: f64,
}

/// A key pressed or released while a plugin editor window (a native window the host made)
/// had keyboard focus, forwarded to the app because its code is captured
/// (`PluginCommand::SetEditorKeyCapture`; `Space` always). Mirrors the DOM `KeyboardEvent`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct PluginWindowKey {
    /// `KeyboardEvent.key` (`" "`, `"a"`, `"A"`), best effort from the code and Shift.
    pub key: String,
    /// `KeyboardEvent.code`: the physical key (`"Space"`, `"KeyA"`).
    pub code: String,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
    /// `true` for a press (`keydown`), `false` for a release (`keyup`).
    pub down: bool,
    /// An auto-repeat press.
    pub repeat: bool,
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

/// A plugin inserted sandboxed by default (`PluginCommand::ListSandboxDefaults`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SandboxDefault {
    pub format: PluginFormat,
    /// `PluginInstance.plugin_id` (stable per format; never a path).
    pub plugin_id: String,
    /// Its name when it was added (display only).
    pub name: String,
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
    /// v0.2: channels of the plugin's aux/sidechain input bus found at
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
    /// A scan ended (also sent after a `RetryScan`): `plugins` known, the targets that
    /// failed. The plugin list may have changed.
    ScanFinished {
        plugins: u32,
        failed: Vec<ScanFailure>,
    },
    EditorClosed {
        device: DeviceId,
    },
    /// A captured key in `device`'s editor window (`PluginCommand::SetEditorKeyCapture`):
    /// the app handles it as if pressed in its own window (Space = play/stop).
    EditorKey {
        device: DeviceId,
        key: PluginWindowKey,
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
    /// The plugin devices still to load: instances are created a few at a time (a time
    /// budget per engine tick) when a project opens or many plugins arrive at once. Sent when
    /// the set changes; empty once all are loaded.
    Loading {
        pending: Vec<DeviceId>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ScanFailure {
    pub path: String,
    /// Why it failed (`scan timed out after 20s`, the plugin's load error, crash info).
    pub message: String,
    /// The format the target was scanned as (what `RetryScan` takes); omitted by hosts
    /// older than `RetryScan` and for failures that aren't about one target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub format: Option<PluginFormat>,
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

    #[test]
    fn retry_and_failure_fields_are_additive() {
        // A failure from an older host (no format) still parses; new fields are omitted
        // when unset.
        let old: ScanFailure =
            serde_json::from_str(r#"{"path":"/p/a.clap","message":"x"}"#).unwrap();
        assert_eq!(old.format, None);
        assert_eq!(
            serde_json::to_string(&old).unwrap(),
            r#"{"path":"/p/a.clap","message":"x"}"#
        );
        let retry: PluginCommand =
            serde_json::from_str(r#"{"type":"RetryScan","path":"/p/a.clap","format":"Clap"}"#)
                .unwrap();
        assert_eq!(
            retry,
            PluginCommand::RetryScan {
                path: "/p/a.clap".into(),
                format: PluginFormat::Clap,
                timeout_ms: None
            }
        );
    }
}

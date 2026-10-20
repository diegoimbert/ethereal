//! Plugin contracts shared by in-process (`ether-clap`) and sandboxed (`ether-sandbox`)
//! hosting. Core never loads plugins itself (wasm-safe): it only defines the traits.
//!
//! A plugin instance has two halves, mirroring CLAP's thread model:
//! - [`PluginController`] lives on the main/controller thread: state, params metadata,
//!   editor window, notifications. Not required to be `Send` (CLAP main-thread objects
//!   usually aren't).
//! - [`PluginNode`] is the audio-thread processor inserted into the engine like any
//!   [`Device`].
//!
//! # Sandboxed instances
//! `ether-sandbox` implements both traits by proxying to a helper process: shared-memory
//! audio/event buffers, a pair of cross-process semaphores for block sync, +1 block of
//! latency reported via [`crate::Node::latency`] (so PDC accounts for it). If the helper
//! dies or misses a deadline, the node outputs silence, [`PluginNode::is_faulted`] becomes
//! true and the controller reports `PluginEvent::Crashed` and bypasses it.
//!
//! # IPC naming rule (parallel dev instances)
//! Every globally named OS object (shared memory segment, semaphore, socket/pipe, scanner
//! temp file) MUST be named with [`ipc_name`], which embeds the dev instance id
//! (`ETHER_INSTANCE`) and the host pid, so several app instances/agents never collide.

use ether_protocol::devices::{DeviceDescriptor, ParamInfo};
use ether_protocol::model::ParamId;
use serde::{Deserialize, Serialize};

use crate::config::PrepareConfig;
use crate::node::Device;

/// Audio-thread half of a plugin instance.
pub trait PluginNode: Device {
    /// True after a crash/timeout (sandbox) or a fatal plugin error. The node then outputs
    /// silence until replaced.
    fn is_faulted(&self) -> bool;
}

/// Main-thread half of a plugin instance.
pub trait PluginController {
    fn descriptor(&self) -> DeviceDescriptor;

    /// Current parameter metadata (plugins may rescan params).
    fn params(&mut self) -> Vec<ParamInfo>;

    /// Activate for processing and hand out the audio-thread half. Only one active node
    /// at a time; call [`PluginController::deactivate`] with it before re-activating.
    fn activate(&mut self, config: &PrepareConfig) -> Result<Box<dyn PluginNode>, PluginError>;

    /// Take back the audio half (after the engine returned it through the GC ring).
    fn deactivate(&mut self, node: Box<dyn PluginNode>);

    fn save_state(&mut self) -> Result<Vec<u8>, PluginError>;
    fn load_state(&mut self, state: &[u8]) -> Result<(), PluginError>;

    fn has_editor(&self) -> bool;
    /// Open a floating editor window (no embedding in v0.1).
    fn open_editor(&mut self) -> Result<(), PluginError>;
    fn close_editor(&mut self);

    /// The open editor's top-level native window (for capturing it and injecting input into
    /// it). `None` while no editor is open, or when the window isn't known (yet): a
    /// plugin's own floating window (CLAP) is discovered best-effort after it opens, so this
    /// may turn `Some` a few polls later. Main thread.
    fn editor_window(&self) -> Option<NativeWindow> {
        None
    }

    /// Main-thread housekeeping (CLAP `on_main_thread`, timers, GUI callbacks). Appends
    /// notifications for the controller. Call regularly (~30-60 Hz).
    fn poll(&mut self, out: &mut Vec<PluginNotification>);

    /// Current plain value of `param` (main thread; valid active or inactive). Used after
    /// `load_state` to mirror plugin params into the document. `None` = unknown/unsupported.
    fn param_value(&mut self, param: ParamId) -> Option<f64> {
        let _ = param;
        None
    }

    /// The plugin's own display text for `value` (plain) of `param`, units included
    /// (`"-6.0 dB"`, `"Saw"`), for the generic device UI (`PluginCommand::ParamTexts`).
    /// Main thread. CLAP `params.value_to_text`; VST3 `getParamStringByValue` + the param's
    /// `units`; VST2 `effGetParamDisplay` + `effGetParamLabel`, which only describe the
    /// current value (`None` when `value` isn't it). AU keeps the default: its params carry
    /// real units and ranges, which the generic formatting already shows.
    /// `None` = no text (unknown param, or the plugin has none): callers format the number.
    fn param_text(&mut self, param: ParamId, value: f64) -> Option<String> {
        let _ = (param, value);
        None
    }

    /// Set a parameter's plain value while the plugin is NOT active (main thread; e.g. CLAP
    /// `params.flush`, VST3 `IEditController::setParamNormalized` + processor sync). While
    /// active, send `ProcessEvent::Param` to the node instead. Default: `Unsupported`.
    fn set_param_value(&mut self, param: ParamId, value: f64) -> Result<(), PluginError> {
        let _ = (param, value);
        Err(PluginError::Unsupported(
            "setting params while inactive".into(),
        ))
    }
}

/// A plugin editor's top-level native window, owned by process `pid`: the host process for
/// in-process plugins, the helper's for sandboxed ones (X11 window ids and CGWindowIDs are
/// valid across processes; an `HWND` is system-wide too).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeWindow {
    pub pid: u32,
    pub handle: WindowHandle,
}

/// A platform window handle as plain integers (serializable, `Send`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum WindowHandle {
    /// The X11 window id (the client window, not a window manager frame).
    X11 {
        window: u64,
    },
    Win32 {
        hwnd: isize,
    },
    AppKit {
        /// `-[NSWindow windowNumber]`, i.e. the `CGWindowID` (ScreenCaptureKit, CGWindowList).
        #[serde(rename = "windowNumber")]
        window_number: i64,
        /// The `NSWindow*`, only meaningful inside `pid`; 0 when reported across processes.
        #[serde(rename = "nsWindow")]
        ns_window: usize,
    },
}

/// Things a plugin tells its host outside of audio processing.
#[derive(Clone, Debug, PartialEq)]
pub enum PluginNotification {
    /// The user moved a control in the plugin GUI (becomes an undoable `SetParam`).
    ParamEdited {
        param: ParamId,
        value: f64,
    },
    GestureBegin {
        param: ParamId,
    },
    GestureEnd {
        param: ParamId,
    },
    /// Latency changed; the controller re-publishes the graph (PDC).
    LatencyChanged {
        samples: u32,
    },
    /// Plugin asked to be restarted (deactivate + activate).
    RestartRequested,
    /// Param list/metadata changed (rescan).
    ParamsChanged,
    /// Internal state changed: the state is re-read by the next save (not a document edit;
    /// saved on its own by a slow timer, see `ether_controller`'s `saving` module).
    StateDirty,
    EditorClosed,
    /// A captured key in the editor window (`ether_plugin_host::editor_keys`), forwarded to
    /// the app as `PluginEvent::EditorKey`.
    EditorKey(ether_protocol::plugins::PluginWindowKey),
    Crashed {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PluginError {
    #[error("plugin not found: {0}")]
    NotFound(String),
    #[error("plugin failed to load: {0}")]
    Load(String),
    #[error("plugin activation failed: {0}")]
    Activation(String),
    #[error("plugin state error: {0}")]
    State(String),
    #[error("plugin has no editor")]
    NoEditor,
    #[error("plugin crashed: {0}")]
    Crashed(String),
    #[error("ipc error: {0}")]
    Ipc(String),
    /// The format, platform or build doesn't support this (e.g. a VST3/AU host that is not
    /// implemented yet, AU off macOS, sandboxing on Windows).
    #[error("unsupported: {0}")]
    Unsupported(String),
}

/// Name for a globally visible IPC object: `ether-<instance>-<pid>-<purpose>`.
///
/// `instance` is the sanitized `ETHER_INSTANCE` (see README "Running multiple dev
/// instances"), `pid` the host process id, `purpose` e.g. `sbx-<device>-shm`. Some OSes
/// limit names (macOS POSIX shm: 31 chars): implementations may hash the result but must
/// keep instance + pid as inputs.
pub fn ipc_name(instance: &str, pid: u32, purpose: &str) -> String {
    format!("ether-{instance}-{pid}-{purpose}")
}

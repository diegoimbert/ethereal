//! Device chains, parameters and device descriptors.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{BuiltinDevice, BuiltinDeviceType, DeviceId, MediaId, ParamId, TrackId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum DeviceCommand {
    /// Insert on `track` before `before` (None = end of chain).
    Insert {
        id: DeviceId,
        track: TrackId,
        device: DeviceSpec,
        before: Option<DeviceId>,
    },
    Remove {
        id: DeviceId,
    },
    /// Move within / across chains.
    Move {
        id: DeviceId,
        track: TrackId,
        before: Option<DeviceId>,
    },
    Duplicate {
        id: DeviceId,
        new_id: DeviceId,
    },
    Rename {
        id: DeviceId,
        name: String,
    },
    SetEnabled {
        id: DeviceId,
        enabled: bool,
    },
    /// Set a parameter to a *plain* value (clamped to the param range). Undoable; send with
    /// a gesture id while dragging. The engine receives it through the lock-free param queue
    /// immediately (no snapshot rebuild).
    SetParam {
        device: DeviceId,
        param: ParamId,
        value: f64,
    },
    ResetParam {
        device: DeviceId,
        param: ParamId,
    },
    /// Sampler only.
    SetSample {
        device: DeviceId,
        media: Option<MediaId>,
    },
    /// Replies `DeviceTypes` with every built-in device descriptor.
    ListBuiltin,
    /// Replies `Descriptor` for an instantiated device (plugins: params discovered at load).
    GetDescriptor {
        device: DeviceId,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum DeviceSpec {
    Builtin {
        device: BuiltinDevice,
    },
    /// CLAP plugin id from the plugin list. `sandboxed: None` = user default.
    Plugin {
        plugin_id: String,
        sandboxed: Option<bool>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum DeviceCategory {
    Instrument,
    AudioEffect,
    NoteEffect,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum DeviceTypeRef {
    Builtin { device: BuiltinDeviceType },
    Plugin { plugin_id: String },
}

/// Static description of a device type (or of a loaded plugin instance).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DeviceDescriptor {
    pub device_type: DeviceTypeRef,
    pub name: String,
    pub category: DeviceCategory,
    pub params: Vec<ParamInfo>,
    /// Audio I/O channel counts of the main ports.
    pub audio_inputs: u16,
    pub audio_outputs: u16,
    pub midi_input: bool,
}

/// Parameter metadata. Plain values are what the document stores; normalized 0..=1 values
/// are what automation and generic UI controls use. `scale` defines the mapping.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ParamInfo {
    pub id: ParamId,
    pub name: String,
    /// Group/section name for the generic device UI (e.g. "Filter", "Envelope").
    pub group: Option<String>,
    pub unit: ParamUnit,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub scale: ParamScale,
    /// `Some(labels)` = discrete/enum param with `labels.len()` steps from `min` to `max`.
    pub labels: Option<Vec<String>>,
    pub automatable: bool,
    pub hidden: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum ParamUnit {
    None,
    Decibels,
    Hertz,
    Milliseconds,
    Seconds,
    Percent,
    Semitones,
    Ratio,
    Pan,
    Toggle,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ParamScale {
    Linear,
    /// Logarithmic (min must be > 0), e.g. frequencies.
    Log,
    /// `normalized^exponent` (e.g. 2.0 for time/attack knobs).
    Power {
        exponent: f64,
    },
    /// Decibel fader law (volume faders).
    Fader,
}

impl ParamInfo {
    /// Map a normalized value 0..=1 to a plain value.
    pub fn to_plain(&self, normalized: f64) -> f64 {
        let _ = normalized;
        todo!("core node: param scale mapping (shared with the UI; keep test vectors)")
    }

    /// Map a plain value to normalized 0..=1.
    pub fn to_normalized(&self, plain: f64) -> f64 {
        let _ = plain;
        todo!("core node: param scale mapping")
    }
}

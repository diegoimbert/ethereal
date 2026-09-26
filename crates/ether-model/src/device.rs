//! Device instances on a track's device chain (built-in devices and plugins).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{DeviceId, MediaId, TrackId};
use crate::value::{Base64Bytes, OrderKey, ParamId};

/// A device on a track. Chain order = `order` among devices with the same `track`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Device {
    pub id: DeviceId,
    pub track: TrackId,
    pub order: OrderKey,
    pub name: String,
    /// Bypass switch (Ableton's device on/off).
    pub enabled: bool,
    pub kind: DeviceKind,
    /// Parameter values in *plain* units (Hz, dB, ms, ...). Missing = the param's default.
    /// For plugins this is a mirror for UI/automation; the plugin state blob is authoritative
    /// on load.
    pub params: BTreeMap<ParamId, f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum DeviceKind {
    Builtin { device: BuiltinDevice },
    Plugin { plugin: PluginInstance },
}

/// Built-in devices and their non-parameter data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum BuiltinDevice {
    Synth,
    Sampler { sample: Option<MediaId> },
    Compressor,
    Delay,
}

/// Data-less discriminant of [`BuiltinDevice`] (used in descriptors and factories).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub enum BuiltinDeviceType {
    Synth,
    Sampler,
    Compressor,
    Delay,
}

impl BuiltinDevice {
    pub fn device_type(&self) -> BuiltinDeviceType {
        match self {
            Self::Synth => BuiltinDeviceType::Synth,
            Self::Sampler { .. } => BuiltinDeviceType::Sampler,
            Self::Compressor => BuiltinDeviceType::Compressor,
            Self::Delay => BuiltinDeviceType::Delay,
        }
    }
}

/// A plugin instance. `state` is the opaque blob from the plugin's state extension.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PluginInstance {
    pub format: PluginFormat,
    /// CLAP plugin id (reverse-DNS, e.g. `com.u-he.diva`).
    pub plugin_id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    /// Run out-of-process (crash isolation, +1 block latency).
    pub sandboxed: bool,
    /// Last saved plugin state. `None` = fresh instance.
    pub state: Option<Base64Bytes>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub enum PluginFormat {
    Clap,
}

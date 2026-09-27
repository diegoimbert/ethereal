//! Device instances on a track's device chain (built-in devices and plugins).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::drum_rack::SliceSettings;
use crate::ids::{DeviceId, DrumPadId, MediaId, TrackId};
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
    /// Sidechain source (roadmap v2, `sidechain`; `.ether` v3): the post-fader output of this
    /// track feeds the device's sidechain input. Ignored by devices without one
    /// (`DeviceDescriptor::sidechain_inputs == 0`). Sidechain edges count as routing edges
    /// (no cycles); see CONTRACTS.md §11.10 for processing order and PDC.
    pub sidechain: Option<TrackId>,
    /// Drum pad whose chain this device is on (roadmap v2, `drum-rack`; `.ether` v3). `None` =
    /// the track's own chain. Pad devices keep `track` = the rack's track; chain order is
    /// `order` among devices with the same `(track, pad)`.
    pub pad: Option<DrumPadId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum DeviceKind {
    Builtin { device: BuiltinDevice },
    Plugin { plugin: PluginInstance },
}

/// Built-in devices and their non-parameter data.
///
/// Parameters are *not* here (they are `Device::params`, described by the device type's
/// `DeviceDescriptor` from `ether-devices`). Variants after `Delay` are roadmap v2: their
/// parameter lists are defined by the owning node (`devices-2`, `drum-rack`); ids are
/// append-only, never renumbered once released.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum BuiltinDevice {
    Synth,
    /// `slices` (roadmap v2, `drum-rack`; `.ether` v3): slice markers + slice mode, see
    /// [`crate::drum_rack`].
    Sampler {
        sample: Option<MediaId>,
        slices: SliceSettings,
    },
    Compressor,
    Delay,
    /// Parametric EQ with a fixed number of bands (`devices-2`).
    Eq,
    /// Algorithmic reverb (`devices-2`).
    Reverb,
    /// Brickwall/lookahead limiter (`devices-2`).
    Limiter,
    /// Gain, pan, stereo width, phase invert, mono (`devices-2`).
    Utility,
    /// Instrument hosting [`crate::drum_rack::DrumPad`]s, each with its own chain
    /// (`drum-rack`).
    DrumRack,
}

/// Data-less discriminant of [`BuiltinDevice`] (used in descriptors and factories).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub enum BuiltinDeviceType {
    Synth,
    Sampler,
    Compressor,
    Delay,
    Eq,
    Reverb,
    Limiter,
    Utility,
    DrumRack,
}

impl BuiltinDeviceType {
    /// Every built-in type, in `DeviceCommand::ListBuiltin` order.
    pub const ALL: [BuiltinDeviceType; 9] = [
        Self::Synth,
        Self::Sampler,
        Self::Compressor,
        Self::Delay,
        Self::Eq,
        Self::Reverb,
        Self::Limiter,
        Self::Utility,
        Self::DrumRack,
    ];
}

impl BuiltinDevice {
    pub fn device_type(&self) -> BuiltinDeviceType {
        match self {
            Self::Synth => BuiltinDeviceType::Synth,
            Self::Sampler { .. } => BuiltinDeviceType::Sampler,
            Self::Compressor => BuiltinDeviceType::Compressor,
            Self::Delay => BuiltinDeviceType::Delay,
            Self::Eq => BuiltinDeviceType::Eq,
            Self::Reverb => BuiltinDeviceType::Reverb,
            Self::Limiter => BuiltinDeviceType::Limiter,
            Self::Utility => BuiltinDeviceType::Utility,
            Self::DrumRack => BuiltinDeviceType::DrumRack,
        }
    }

    /// A fresh instance of `ty` with default data (empty sampler, no slices).
    pub fn new(ty: BuiltinDeviceType) -> Self {
        match ty {
            BuiltinDeviceType::Synth => Self::Synth,
            BuiltinDeviceType::Sampler => Self::Sampler {
                sample: None,
                slices: SliceSettings::default(),
            },
            BuiltinDeviceType::Compressor => Self::Compressor,
            BuiltinDeviceType::Delay => Self::Delay,
            BuiltinDeviceType::Eq => Self::Eq,
            BuiltinDeviceType::Reverb => Self::Reverb,
            BuiltinDeviceType::Limiter => Self::Limiter,
            BuiltinDeviceType::Utility => Self::Utility,
            BuiltinDeviceType::DrumRack => Self::DrumRack,
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

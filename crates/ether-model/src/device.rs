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
///
/// The document never stores where a plugin lives on disk: the host resolves
/// `(format, plugin_id)` against its scanned plugin catalog, so a project opens on any machine
/// that has the plugin installed (see `docs/PLUGIN-FORMATS.md`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PluginInstance {
    pub format: PluginFormat,
    /// Format-specific plugin id (see [`PluginFormat`] for the per-format convention), e.g.
    /// `com.u-he.diva` (CLAP), `565354416D627261736F6E6963000000` (VST3),
    /// `aufx:dely:appl` (AU).
    pub plugin_id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    /// Run out-of-process (crash isolation, +1 block latency).
    pub sandboxed: bool,
    /// Last saved plugin state. `None` = fresh instance.
    pub state: Option<Base64Bytes>,
}

/// Plugin format. Serialized as the plain variant name (`"Clap"`, `"Vst3"`, `"Au"`); these
/// tags are stable (`.ether` files and the wire depend on them) and new formats are additive.
///
/// `plugin_id` convention per format (what [`PluginInstance::plugin_id`] and the scanner's
/// `PluginDescriptor::id` hold):
/// - [`PluginFormat::Clap`]: the CLAP plugin id, reverse-DNS (`com.u-he.diva`).
/// - [`PluginFormat::Vst3`]: the audio-processor class id (`PClassInfo::cid`) in the SDK's
///   canonical `FUID::toString` form (the `CID` in `moduleinfo.json`): 32 **uppercase** hex
///   characters, the words `l1 l2 l3 l4` of `INLINE_UID`, no separators
///   (`565354416D627261736F6E6963000000`). Identical on every OS (on Windows the in-memory
///   TUID uses the COM GUID layout; `ether_vst3::class_id_to_string` converts). The `.vst3`
///   bundle path is not part of the id: the host finds it in its plugin catalog.
/// - [`PluginFormat::Au`]: the `AudioComponentDescription` four-char codes
///   `type:subtype:manufacturer` (`aufx:dely:appl` = Apple AUDelay). Each code is exactly
///   four characters (Mac OS Roman, printable), kept verbatim (case-sensitive, spaces kept).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
pub enum PluginFormat {
    Clap,
    Vst3,
    Au,
}

impl PluginFormat {
    /// Every format, in scan/display order.
    pub const ALL: [PluginFormat; 3] = [PluginFormat::Clap, PluginFormat::Vst3, PluginFormat::Au];

    /// Lowercase CLI/log name: `clap`, `vst3`, `au` (e.g. `ether-sandbox-helper --format`).
    pub fn as_str(self) -> &'static str {
        match self {
            PluginFormat::Clap => "clap",
            PluginFormat::Vst3 => "vst3",
            PluginFormat::Au => "au",
        }
    }

    /// Inverse of [`PluginFormat::as_str`] (case-insensitive).
    pub fn parse(s: &str) -> Option<PluginFormat> {
        Self::ALL
            .into_iter()
            .find(|f| f.as_str().eq_ignore_ascii_case(s))
    }
}

impl std::fmt::Display for PluginFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

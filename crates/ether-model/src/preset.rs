//! Device preset files (v0.2, owned by the `presets` node; CONTRACTS.md §12.5).
//!
//! ```json
//! { "format": "ethereal-preset", "version": 1, "app_version": "0.2.0",
//!   "preset": { "name": "Warm Pad", "device": { "type": "Builtin", "device": "PolySynth" },
//!               "meta": { "tags": ["pad"], "author": "Ethereal", "description": null },
//!               "params": { "0": 1.0, "12": 800.0 }, "kind": null, "state": null } }
//! ```
//!
//! - Built-in presets store `params` (plain values; ids are the device's frozen param ids;
//!   unknown ids are ignored on load, missing ids reset to their default) and, for devices
//!   with kind data (sampler, multisampler), `kind`: the whole `BuiltinDevice` value. Media ids
//!   inside `kind` are resolved through `samples` (a preset carries the sample references by
//!   library location, not project `MediaId`s: loading imports/references them first).
//! - Plugin presets store the opaque plugin `state` blob (authoritative) plus the `params`
//!   mirror (UI only), and the plugin identity in `device`.
//! - Loading a preset onto a device is one undoable document edit (params reset + set, kind
//!   replaced, plugin state replaced). It never changes the device type: a preset for another
//!   type is rejected (`InvalidArgument`).
//! - Factory presets ship embedded in `ether-devices` (read-only); user presets are files in
//!   the engine-side user library (`<library>/Presets/<device-key>/<name>.etherpreset`). The
//!   UI never touches files.
//!
//! - **Rack presets** (v0.3, `rack-presets`; version 2): a preset of a rack device
//!   (`InstrumentRack`, `AudioEffectRack`, `MidiEffectRack`) also stores its structure in
//!   `rack` ([`PresetRack`]): the chains with their devices, the rack's modulators and the
//!   modulation mappings inside it. Loading one replaces the rack's chains (one undo step,
//!   new ids derived from the load command's seed in chain/device order). Version-1 files
//!   (no `rack`) load unchanged: a v1 rack preset sets macros and params only.
//!
//! Loading: parse, check `format`, run migrations up to [`PRESET_VERSION`] (1 → 2 is
//! additive: `rack` absent = `None`), then deserialize. Unknown top-level fields are ignored.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::device::{BuiltinDevice, BuiltinDeviceType, PluginFormat};
use crate::error::FileError;
use crate::ids::MediaId;
use crate::modulation::ModulatorKind;
use crate::rack::Zone;
use crate::value::{Base64Bytes, ParamId};
use crate::value::{Color, Decibels, Pan};

/// Magic string in the `format` field.
pub const PRESET_FORMAT_TAG: &str = "ethereal-preset";
/// Current preset file version (2 = v0.3 rack presets, `Preset::rack`).
pub const PRESET_VERSION: u32 = 2;
/// File extension (without dot).
pub const PRESET_EXTENSION: &str = "etherpreset";
/// User preset folder inside the user library root.
pub const PRESETS_DIR: &str = "Presets";

/// A preset file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PresetFile {
    pub format: String,
    pub version: u32,
    /// Version of the app that wrote the file (informational).
    pub app_version: String,
    pub preset: Preset,
}

/// The device type a preset is for.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum PresetDevice {
    Builtin {
        device: BuiltinDeviceType,
    },
    Plugin {
        format: PluginFormat,
        plugin_id: String,
        /// Display only.
        name: String,
        vendor: String,
    },
}

/// Descriptive metadata (tags, author).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct PresetMeta {
    /// Free-form lowercase tags (`"pad"`, `"bass"`, `"warm"`), deduplicated, sorted.
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// A sample referenced by a sample-based preset: `media` is the id used inside
/// `Preset::kind`; `location`/`path` say where the file is (a browse location id and a path
/// relative to it, as in `MediaSource::Location`), `hash` the content hash for relinking.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PresetSample {
    pub media: MediaId,
    pub location: String,
    pub path: String,
    pub hash: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Preset {
    pub name: String,
    pub device: PresetDevice,
    #[serde(default)]
    pub meta: PresetMeta,
    /// Plain param values.
    #[serde(default)]
    pub params: BTreeMap<ParamId, f64>,
    /// Built-in kind data (sampler, multisampler), same type as `device`.
    #[serde(default)]
    pub kind: Option<BuiltinDevice>,
    /// Samples referenced by `kind`.
    #[serde(default)]
    pub samples: Vec<PresetSample>,
    /// Plugin state blob (plugins only).
    #[serde(default)]
    pub state: Option<Base64Bytes>,
    /// Rack structure (v0.3, `rack-presets`; rack devices only). Omitted when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub rack: Option<PresetRack>,
}

/// The structure of a rack preset (v0.3, `rack-presets`). Indices refer to positions in
/// these lists (chains in order, devices in chain order, modulators in order).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct PresetRack {
    pub chains: Vec<PresetChain>,
    /// Modulators hosted by the rack device itself.
    #[serde(default)]
    pub modulators: Vec<PresetModulator>,
    /// Mappings whose source and target are inside the preset.
    #[serde(default)]
    pub mappings: Vec<PresetModMapping>,
}

/// One rack chain (fields as `ether_model::rack::RackChain`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PresetChain {
    pub name: String,
    pub color: Option<Color>,
    pub volume: Decibels,
    pub pan: Pan,
    pub mute: bool,
    pub solo: bool,
    pub keys: Zone,
    pub velocities: Zone,
    pub select: Zone,
    pub devices: Vec<PresetChainDevice>,
}

/// One device of a rack chain (built-in or plugin; no nested racks in v0.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PresetChainDevice {
    pub name: String,
    pub enabled: bool,
    pub device: PresetDevice,
    #[serde(default)]
    pub params: BTreeMap<ParamId, f64>,
    #[serde(default)]
    pub kind: Option<BuiltinDevice>,
    #[serde(default)]
    pub state: Option<Base64Bytes>,
}

/// A modulator hosted by the rack device.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PresetModulator {
    pub name: String,
    pub kind: ModulatorKind,
    #[serde(default)]
    pub params: BTreeMap<ParamId, f64>,
}

/// A modulation mapping inside a rack preset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PresetModMapping {
    pub source: PresetModSource,
    pub target: PresetModTarget,
    pub param: ParamId,
    pub depth: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum PresetModSource {
    /// Rack macro `index`.
    Macro { index: u8 },
    /// `PresetRack::modulators[index]`.
    Modulator { index: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum PresetModTarget {
    /// The rack device itself (non-macro params).
    Rack,
    /// `PresetRack::chains[chain].devices[device]`.
    ChainDevice { chain: u32, device: u32 },
}

/// Parse and validate a preset file (format tag, version, device/kind consistency).
pub fn load_preset(json: &str) -> Result<Preset, FileError> {
    let doc: serde_json::Value = serde_json::from_str(json)?;
    if doc.get("format").and_then(|f| f.as_str()) != Some(PRESET_FORMAT_TAG) {
        return Err(FileError::NotAnEtherFile(format!(
            "missing or wrong \"format\" (expected {PRESET_FORMAT_TAG:?})"
        )));
    }
    let version = doc
        .get("version")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| FileError::NotAnEtherFile("missing or invalid \"version\"".into()))?;
    if version > u64::from(PRESET_VERSION) {
        return Err(FileError::TooNew {
            found: version as u32,
            supported: PRESET_VERSION,
        });
    }
    let file: PresetFile = serde_json::from_value(doc)?;
    let p = file.preset;
    if let Some(rack) = &p.rack {
        let is_rack = matches!(&p.device, PresetDevice::Builtin { device } if device.is_rack());
        let refs_ok = rack.mappings.iter().all(|m| {
            let source = match m.source {
                PresetModSource::Macro { index } => {
                    usize::from(index) < crate::rack::RACK_MACROS as usize
                }
                PresetModSource::Modulator { index } => (index as usize) < rack.modulators.len(),
            };
            let target = match m.target {
                PresetModTarget::Rack => true,
                PresetModTarget::ChainDevice { chain, device } => rack
                    .chains
                    .get(chain as usize)
                    .is_some_and(|c| (device as usize) < c.devices.len()),
            };
            source && target && m.depth.is_finite()
        });
        if !is_rack || !refs_ok {
            return Err(FileError::Migration {
                from: version as u32,
                message: "invalid rack structure in preset".into(),
            });
        }
    }
    if let (PresetDevice::Builtin { device }, Some(kind)) = (&p.device, &p.kind)
        && kind.device_type() != *device
    {
        return Err(FileError::Migration {
            from: version as u32,
            message: "preset kind data does not match its device type".into(),
        });
    }
    if p.params.values().any(|v| !v.is_finite()) {
        return Err(FileError::Migration {
            from: version as u32,
            message: "preset param values must be finite".into(),
        });
    }
    Ok(p)
}

/// Serialize a preset at [`PRESET_VERSION`] (pretty JSON, stable key order).
pub fn save_preset(preset: &Preset, app_version: &str) -> Result<String, FileError> {
    let mut s = serde_json::to_string_pretty(&PresetFile {
        format: PRESET_FORMAT_TAG.into(),
        version: PRESET_VERSION,
        app_version: app_version.into(),
        preset: preset.clone(),
    })?;
    s.push('\n');
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset() -> Preset {
        Preset {
            name: "Init".into(),
            device: PresetDevice::Builtin {
                device: BuiltinDeviceType::PolySynth,
            },
            meta: PresetMeta {
                tags: vec!["pad".into()],
                author: Some("Ethereal".into()),
                description: None,
            },
            params: BTreeMap::from([(ParamId(0), 1.0), (ParamId(3), 440.0)]),
            kind: None,
            samples: vec![],
            state: None,
            rack: None,
        }
    }

    #[test]
    fn roundtrip_and_checks() {
        let p = preset();
        let json = save_preset(&p, "0.2.0").unwrap();
        assert!(json.contains("\"format\": \"ethereal-preset\""));
        assert_eq!(load_preset(&json).unwrap(), p);
        // Minimal files load with defaults.
        let min = r#"{"format":"ethereal-preset","version":1,"app_version":"x",
            "preset":{"name":"A","device":{"type":"Builtin","device":"Chorus"}}}"#;
        let m = load_preset(min).unwrap();
        assert!(m.params.is_empty() && m.meta.tags.is_empty());
        assert!(load_preset(r#"{"format":"ethereal-project","version":1}"#).is_err());
        assert!(matches!(
            load_preset(r#"{"format":"ethereal-preset","version":9}"#),
            Err(FileError::TooNew { .. })
        ));
        // Kind data must match the device type.
        let mut bad = preset();
        bad.kind = Some(BuiltinDevice::Chorus);
        assert!(load_preset(&save_preset(&bad, "x").unwrap()).is_err());
    }
}

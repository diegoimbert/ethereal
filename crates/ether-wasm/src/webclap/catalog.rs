//! The installed WebCLAP plugins, as the browser's scan describes them.
//!
//! The page scans a module once at install (`apps/web/src/engine/webclap/scan.ts`, in a
//! Worker of its own): it instantiates each plugin of the module and reads its CLAP
//! metadata, unconverted ([`ScannedPlugin`]). The page keeps that with the module and hands
//! the whole list to the controller Worker (`WasmController::set_webclap_catalog`), which
//! turns it into the same descriptors native CLAP plugins have: [`plugin_descriptor`] for
//! the browser, [`device_descriptor`] (params, ports) for the device.

use ether_core::protocol::devices::{
    DeviceCategory, DeviceDescriptor, DeviceTypeRef, ParamInfo, ParamScale, ParamUnit,
};
use ether_core::protocol::model::{ParamId, PluginFormat};
use ether_core::protocol::plugins::PluginDescriptor;
use serde::{Deserialize, Serialize};

/// CLAP `clap_param_info_flags` the host reads.
pub mod param_flags {
    pub const IS_STEPPED: u32 = 1 << 0;
    pub const IS_HIDDEN: u32 = 1 << 2;
    pub const IS_READONLY: u32 = 1 << 3;
    pub const IS_AUTOMATABLE: u32 = 1 << 5;
    pub const IS_ENUM: u32 = 1 << 16;
}

/// Largest stepped range the scan reads labels for (as `ether-clap`).
pub const MAX_LABELS: f64 = 128.0;

/// One installed module: its id (a content hash, the page's key for it) and its plugins.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScannedModule {
    pub module: String,
    pub plugins: Vec<ScannedPlugin>,
}

/// A plugin's CLAP metadata, as read at install.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannedPlugin {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub vendor: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub params: Vec<ScannedParam>,
    /// `None`: no `clap.audio-ports` extension (a plain stereo plugin is assumed).
    #[serde(default)]
    pub audio_inputs: Option<Vec<ScannedPort>>,
    #[serde(default)]
    pub audio_outputs: Option<Vec<ScannedPort>>,
    /// Note input ports.
    #[serde(default)]
    pub note_inputs: u32,
    /// The first note input port takes the CLAP dialect (note expressions).
    #[serde(default)]
    pub note_expressions: bool,
    /// Has the `clap.webview/3` extension (a web UI).
    #[serde(default)]
    pub webview: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScannedParam {
    pub id: u32,
    pub flags: u32,
    pub name: String,
    #[serde(default)]
    pub module: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    /// `value_to_text` of each step of a stepped param (at most [`MAX_LABELS`] + 1).
    #[serde(default)]
    pub labels: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScannedPort {
    pub channels: u32,
    /// `CLAP_AUDIO_PORT_IS_MAIN`.
    #[serde(default)]
    pub main: bool,
}

/// The engine-side port layout of a plugin: main input/output channels (capped at the
/// engine's stereo), sidechain channels, note input. Same rules as `ether-clap`: the main
/// port is the first flagged main (else the first port), the sidechain the first other
/// input port with channels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layout {
    pub inputs: u16,
    pub outputs: u16,
    pub sidechain: u16,
    pub midi: bool,
}

fn main_port(ports: &[ScannedPort]) -> Option<usize> {
    ports
        .iter()
        .position(|p| p.main)
        .or((!ports.is_empty()).then_some(0))
}

impl ScannedPlugin {
    pub fn layout(&self) -> Layout {
        let stereo = [ScannedPort {
            channels: 2,
            main: true,
        }];
        let (ins, outs) = match (&self.audio_inputs, &self.audio_outputs) {
            (None, None) => (&stereo[..], &stereo[..]),
            (i, o) => (
                i.as_deref().unwrap_or(&[]),
                o.as_deref().unwrap_or(&[]),
            ),
        };
        let main_in = main_port(ins);
        let main_out = main_port(outs);
        let ch = |ports: &[ScannedPort], i: Option<usize>| {
            i.and_then(|i| ports.get(i))
                .map_or(0, |p| p.channels.min(2) as u16)
        };
        let aux = (0..ins.len()).find(|&i| Some(i) != main_in && ins[i].channels > 0);
        Layout {
            inputs: ch(ins, main_in),
            outputs: ch(outs, main_out),
            sidechain: ch(ins, aux),
            midi: self.note_inputs > 0,
        }
    }

    pub fn category(&self) -> DeviceCategory {
        let has = |f: &str| self.features.iter().any(|x| x == f);
        if has("instrument") {
            DeviceCategory::Instrument
        } else if has("note-effect") && !has("audio-effect") {
            DeviceCategory::NoteEffect
        } else {
            DeviceCategory::AudioEffect
        }
    }

    /// CLAP params → `ParamInfo`, as `ether-clap` maps them: plain values pass through
    /// (linear), stepped ranges get their labels, `module` is the group.
    pub fn param_infos(&self) -> Vec<ParamInfo> {
        use param_flags::*;
        self.params
            .iter()
            .map(|p| {
                let stepped = p.flags & (IS_STEPPED | IS_ENUM) != 0;
                let labels = p.labels.clone().filter(|l| {
                    stepped
                        && p.max > p.min
                        && p.max - p.min <= MAX_LABELS
                        && p.min.fract() == 0.0
                        && p.max.fract() == 0.0
                        && l.len() == (p.max - p.min) as usize + 1
                });
                let unit = if stepped && p.min == 0.0 && p.max == 1.0 {
                    ParamUnit::Toggle
                } else {
                    ParamUnit::None
                };
                ParamInfo {
                    id: ParamId(p.id),
                    name: p.name.clone(),
                    group: (!p.module.is_empty()).then(|| p.module.clone()),
                    unit,
                    min: p.min,
                    max: p.max,
                    default: p.default,
                    scale: ParamScale::Linear,
                    labels,
                    automatable: p.flags & IS_AUTOMATABLE != 0 && p.flags & IS_READONLY == 0,
                    hidden: p.flags & IS_HIDDEN != 0,
                    step: None,
                    remote: None,
                }
            })
            .collect()
    }
}

/// The browser's entry for `plugin` of module `module` (`path` = `webclap:<module>`).
pub fn plugin_descriptor(module: &str, plugin: &ScannedPlugin) -> PluginDescriptor {
    PluginDescriptor {
        format: PluginFormat::Webclap,
        id: plugin.id.clone(),
        name: plugin.name.clone(),
        vendor: plugin.vendor.clone(),
        version: plugin.version.clone(),
        description: plugin.description.clone(),
        features: plugin.features.iter().map(|f| f.to_lowercase()).collect(),
        category: plugin.category(),
        path: format!("webclap:{module}"),
        sidechain_inputs: plugin.layout().sidechain,
    }
}

/// A device of `plugin`: its params and ports.
pub fn device_descriptor(plugin: &ScannedPlugin) -> DeviceDescriptor {
    let layout = plugin.layout();
    DeviceDescriptor {
        device_type: DeviceTypeRef::Plugin {
            plugin_id: plugin.id.clone(),
        },
        name: plugin.name.clone(),
        category: plugin.category(),
        params: plugin.param_infos(),
        audio_inputs: layout.inputs,
        audio_outputs: layout.outputs,
        midi_input: layout.midi,
        sidechain_inputs: layout.sidechain,
        layout: None,
        implemented: true,
    }
}

/// The installed modules (the page's list; later installs first win nothing: the last
/// module listing a plugin id provides it, so reinstalling a newer build replaces it).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Catalog {
    pub modules: Vec<ScannedModule>,
}

impl Catalog {
    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        Ok(Self {
            modules: serde_json::from_str(json)?,
        })
    }

    /// The module providing `plugin_id`, and the plugin.
    pub fn find(&self, plugin_id: &str) -> Option<(&str, &ScannedPlugin)> {
        self.modules.iter().rev().find_map(|m| {
            m.plugins
                .iter()
                .find(|p| p.id == plugin_id)
                .map(|p| (m.module.as_str(), p))
        })
    }

    /// Every plugin once (by id), as the browser lists them.
    pub fn descriptors(&self) -> Vec<PluginDescriptor> {
        let mut out: Vec<PluginDescriptor> = Vec::new();
        for m in self.modules.iter().rev() {
            for p in &m.plugins {
                if !out.iter().any(|d| d.id == p.id) {
                    out.push(plugin_descriptor(&m.module, p));
                }
            }
        }
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin(json: serde_json::Value) -> ScannedPlugin {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn layout_follows_the_clap_rules() {
        // No audio-ports extension: stereo in and out.
        let p = plugin(serde_json::json!({ "id": "a", "name": "A" }));
        assert_eq!(
            p.layout(),
            Layout {
                inputs: 2,
                outputs: 2,
                sidechain: 0,
                midi: false
            }
        );
        // Instrument: no input, a mono output, a note port; a main port flagged second,
        // the other input is the sidechain (capped at stereo).
        let p = plugin(serde_json::json!({
            "id": "b", "name": "B", "noteInputs": 1,
            "audioInputs": [{ "channels": 4 }, { "channels": 1, "main": true }],
            "audioOutputs": [{ "channels": 1, "main": true }],
        }));
        assert_eq!(
            p.layout(),
            Layout {
                inputs: 1,
                outputs: 1,
                sidechain: 2,
                midi: true
            }
        );
        let p = plugin(serde_json::json!({
            "id": "c", "name": "C", "audioInputs": [], "audioOutputs": [{ "channels": 2 }],
        }));
        assert_eq!((p.layout().inputs, p.layout().outputs), (0, 2));
    }

    #[test]
    fn params_map_like_native_clap() {
        use param_flags::*;
        let p = plugin(serde_json::json!({
            "id": "a", "name": "A", "features": ["instrument"],
            "params": [
                { "id": 7, "flags": IS_AUTOMATABLE, "name": "gain", "module": "out",
                  "min": 0.0, "max": 2.0, "default": 1.0 },
                { "id": 9, "flags": IS_STEPPED | IS_AUTOMATABLE, "name": "on",
                  "min": 0.0, "max": 1.0, "default": 0.0, "labels": ["off", "on"] },
                { "id": 11, "flags": IS_HIDDEN | IS_READONLY | IS_AUTOMATABLE, "name": "meter",
                  "min": 0.0, "max": 1.0, "default": 0.0 },
            ],
        }));
        let infos = p.param_infos();
        assert_eq!(infos[0].id, ParamId(7));
        assert_eq!(infos[0].group.as_deref(), Some("out"));
        assert!(infos[0].automatable && !infos[0].hidden);
        assert_eq!(infos[0].labels, None);
        assert_eq!(infos[1].unit, ParamUnit::Toggle);
        assert_eq!(
            infos[1].labels.as_deref(),
            Some(&["off".to_string(), "on".to_string()][..])
        );
        assert!(!infos[2].automatable && infos[2].hidden);
        let d = device_descriptor(&p);
        assert_eq!(d.category, DeviceCategory::Instrument);
        assert_eq!(
            d.device_type,
            DeviceTypeRef::Plugin {
                plugin_id: "a".into()
            }
        );
    }

    #[test]
    fn the_last_module_listing_an_id_provides_it() {
        let cat = Catalog::parse(
            &serde_json::json!([
                { "module": "m1", "plugins": [{ "id": "x", "name": "Old" }, { "id": "y", "name": "Y" }] },
                { "module": "m2", "plugins": [{ "id": "x", "name": "New" }] },
            ])
            .to_string(),
        )
        .unwrap();
        assert_eq!(cat.find("x").map(|(m, p)| (m, p.name.as_str())), Some(("m2", "New")));
        assert_eq!(cat.find("y").map(|(m, _)| m), Some("m1"));
        assert!(cat.find("z").is_none());
        let list = cat.descriptors();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "New");
        assert_eq!(list[0].path, "webclap:m2");
        assert_eq!(list[0].format, PluginFormat::Webclap);
    }
}

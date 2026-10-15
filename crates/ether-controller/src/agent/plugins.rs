//! Devices and installed plugins (`agent-plugins`): `list_plugins`, `add_device` (built-in
//! types and plugins, with the UI's placement rules), device presets, and the device-window
//! UI actions.

use ether_core::protocol::Command;
use ether_core::protocol::ReplyValue;
use ether_core::protocol::devices::{DeviceCategory, DeviceCommand, DeviceSpec};
use ether_core::protocol::model::*;
use ether_core::protocol::plugins::PluginDescriptor;
use ether_core::protocol::presets::{PresetCommand, PresetInfo, PresetSource};
use serde_json::{Map, Value, json};

use super::read::category_name;
use super::resolve;
use super::schema::Args;
use super::{ToolError, ToolResult, label};
use crate::store::{Library, ProjectStore};
use crate::tx::{internal, not_found};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// `clap`, `vst3`, `au`, `vst2`.
pub(super) fn format_key(f: PluginFormat) -> &'static str {
    f.as_str()
}

fn plugin_json(p: &PluginDescriptor) -> Value {
    let mut m = Map::new();
    m.insert("plugin_id".into(), json!(p.id));
    m.insert("name".into(), json!(p.name));
    m.insert("vendor".into(), json!(p.vendor));
    m.insert("format".into(), json!(format_key(p.format)));
    m.insert("category".into(), json!(category_name(p.category)));
    if !p.version.is_empty() {
        m.insert("version".into(), json!(p.version));
    }
    // A few descriptive features ("reverb", "synthesizer"), not the format plumbing.
    let tags: Vec<&String> = p
        .features
        .iter()
        .filter(|f| {
            !matches!(
                f.as_str(),
                "instrument"
                    | "audio-effect"
                    | "audio_effect"
                    | "fx"
                    | "stereo"
                    | "mono"
                    | "aufx"
                    | "aumu"
                    | "aumf"
                    | "aumi"
                    | "note-effect"
            )
        })
        .take(4)
        .collect();
    if !tags.is_empty() {
        m.insert("tags".into(), json!(tags));
    }
    Value::Object(m)
}

fn category_arg(s: &str) -> Option<DeviceCategory> {
    match s {
        "instrument" => Some(DeviceCategory::Instrument),
        "audio_effect" => Some(DeviceCategory::AudioEffect),
        "midi_effect" => Some(DeviceCategory::NoteEffect),
        _ => None,
    }
}

/// What `add_device` inserts.
enum NewDevice {
    Builtin(BuiltinDeviceType),
    Plugin(PluginDescriptor),
}

impl NewDevice {
    fn category(&self) -> DeviceCategory {
        match self {
            NewDevice::Builtin(t) => ether_devices::descriptor(*t).category,
            NewDevice::Plugin(p) => p.category,
        }
    }

    fn spec(&self) -> DeviceSpec {
        match self {
            NewDevice::Builtin(t) => DeviceSpec::Builtin {
                device: BuiltinDevice::new(*t),
            },
            NewDevice::Plugin(p) => DeviceSpec::Plugin {
                plugin_id: p.id.clone(),
                sandboxed: None,
                format: Some(p.format),
                name: Some(p.name.clone()),
            },
        }
    }

    fn describe(&self) -> Value {
        match self {
            NewDevice::Builtin(t) => json!({ "type": t.key() }),
            NewDevice::Plugin(p) => json!({
                "type": format!("plugin:{}:{}", format_key(p.format), p.id),
                "plugin": p.name,
                "vendor": p.vendor,
                "format": format_key(p.format),
            }),
        }
    }
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    // ─── list_plugins ───────────────────────────────────────────────────────────────────

    pub(super) fn tool_list_plugins(&mut self, a: Args) -> ToolResult {
        let Some(cat) = self.bridge.plugin_catalog() else {
            return Ok(json!({
                "available": false,
                "total": 0,
                "plugins": [],
                "note": "this host runs no plugins (web app): only the built-in devices of list_device_types",
            }));
        };
        let query = a.str("query").map(str::trim).filter(|q| !q.is_empty());
        let category = a.str("category").and_then(category_arg);
        let format = a.str("format").and_then(PluginFormat::parse);
        let offset = a.u64("offset").unwrap_or(0) as usize;
        let limit = a.u64("limit").unwrap_or(50).max(1) as usize;
        let q = query.map(resolve::tokens);
        let mut list: Vec<(&PluginDescriptor, f64)> = cat
            .plugins
            .iter()
            .filter(|p| category.is_none_or(|c| p.category == c))
            .filter(|p| format.is_none_or(|f| p.format == f))
            .filter_map(|p| match (query, &q) {
                (Some(text), Some(q)) => {
                    let hay = format!("{} {} {}", p.name, p.vendor, p.features.join(" "));
                    let s = resolve::score_name(text, q, &p.name)
                        .max(0.9 * resolve::score_name(text, q, &hay));
                    (s >= resolve::MIN_SCORE).then_some((p, s))
                }
                _ => Some((p, 0.0)),
            })
            .collect();
        list.sort_by(|(a, sa), (b, sb)| {
            sb.total_cmp(sa)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                .then(format_rank(a.format).cmp(&format_rank(b.format)))
        });
        let total = list.len();
        let plugins: Vec<Value> = list
            .iter()
            .skip(offset)
            .take(limit)
            .map(|(p, _)| plugin_json(p))
            .collect();
        let mut v = json!({
            "available": true,
            "scanning": cat.scanning,
            "installed": cat.plugins.len(),
            "total": total,
            "offset": offset,
            "plugins": plugins,
        });
        if offset + limit < total {
            v["note"] = json!(format!(
                "more plugins: call again with offset {}",
                offset + limit
            ));
        } else if cat.plugins.is_empty() {
            v["note"] = json!(if cat.scanning {
                "the plugin scan is still running: try again in a moment"
            } else {
                "no plugins found: the user can add plugin folders and rescan in the Plugins settings"
            });
        }
        Ok(v)
    }

    /// The plugin a call names: `plugin_id` (+ `format`), or a name `query` that matches
    /// one plugin (several formats of one plugin count as one: CLAP, then VST3, AU, VST2).
    fn agent_find_plugin(
        &self,
        plugin_id: Option<&str>,
        format: Option<PluginFormat>,
        query: Option<&str>,
    ) -> Result<Option<PluginDescriptor>, ToolError> {
        let cat = self.bridge.plugin_catalog();
        if let Some(id) = plugin_id {
            let Some(cat) = cat else {
                return Err(ToolError::Command(crate::tx::keyed_err(
                    ether_core::protocol::ErrorCode::Unsupported,
                    crate::text::msg("error.agent.plugins_unavailable"),
                )));
            };
            let mut found: Vec<&PluginDescriptor> = cat
                .plugins
                .iter()
                .filter(|p| p.id == id && format.is_none_or(|f| p.format == f))
                .collect();
            found.sort_by_key(|p| format_rank(p.format));
            return match found.first() {
                Some(p) => Ok(Some((*p).clone())),
                None => Err(ToolError::Command(not_found(format!(
                    "installed plugin `{id}`{} (see list_plugins)",
                    format
                        .map(|f| format!(" ({})", f.as_str()))
                        .unwrap_or_default()
                )))),
            };
        }
        let (Some(text), Some(cat)) = (query, cat) else {
            return Ok(None);
        };
        let q = resolve::tokens(text);
        let mut scored: Vec<(&PluginDescriptor, f64)> = cat
            .plugins
            .iter()
            .filter(|p| format.is_none_or(|f| p.format == f))
            .map(|p| {
                let s = resolve::score_name(text, &q, &p.name)
                    .max(0.95 * resolve::score_name(text, &q, &format!("{} {}", p.vendor, p.name)));
                (p, s)
            })
            .filter(|(_, s)| *s >= 0.7)
            .collect();
        if scored.is_empty() {
            return Ok(None);
        }
        scored.sort_by(|(a, sa), (b, sb)| {
            sb.total_cmp(sa)
                .then(format_rank(a.format).cmp(&format_rank(b.format)))
        });
        let best = scored[0].1;
        let mut tied: Vec<&PluginDescriptor> = scored
            .iter()
            .filter(|(_, s)| *s >= best - 0.05)
            .map(|(p, _)| *p)
            .collect();
        // One plugin in several formats: the preferred format.
        let key = |p: &PluginDescriptor| (p.name.to_lowercase(), p.vendor.to_lowercase());
        let first = key(tied[0]);
        if tied.iter().all(|p| key(p) == first) {
            return Ok(Some(tied[0].clone()));
        }
        tied.truncate(8);
        Err(ToolError::Ambiguous {
            message: format!(
                "\"{text}\" matches several installed plugins: pass `plugin_id` (and `format`), or ask the user"
            ),
            candidates: json!(tied.iter().map(|p| plugin_json(p)).collect::<Vec<_>>()),
        })
    }

    // ─── add_device ─────────────────────────────────────────────────────────────────────

    /// Where a new device of `category` goes on `track` (the UI's rules,
    /// `ui/src/app/shell/deviceInsert.ts`): an instrument replaces the track's instrument
    /// in place (returned as `replace`), or with none goes after the MIDI effects; a MIDI
    /// effect goes before the instrument; audio effects go last. `index` overrides the
    /// position (an instrument still replaces the old one).
    fn agent_placement(
        &mut self,
        track: TrackId,
        category: DeviceCategory,
        index: Option<usize>,
    ) -> Result<(Option<DeviceId>, Option<DeviceId>), ToolError> {
        let chain: Vec<DeviceId> = self
            .agent_project()?
            .devices_of(track)
            .iter()
            .map(|d| d.id)
            .collect();
        let mut cats = Vec::with_capacity(chain.len());
        for d in &chain {
            cats.push(self.agent_descriptor(*d).map(|x| x.category));
        }
        let replace = (category == DeviceCategory::Instrument)
            .then(|| {
                chain
                    .iter()
                    .zip(&cats)
                    .find(|(_, c)| **c == Some(DeviceCategory::Instrument))
                    .map(|(d, _)| *d)
            })
            .flatten();
        let before = match index {
            Some(i) => chain.get(i).copied(),
            None => match category {
                DeviceCategory::AudioEffect => None,
                DeviceCategory::Instrument if replace.is_some() => replace,
                _ => chain
                    .iter()
                    .zip(&cats)
                    .find(|(_, c)| **c != Some(DeviceCategory::NoteEffect))
                    .map(|(d, _)| *d),
            },
        };
        Ok((before, replace))
    }

    pub(super) fn tool_add_device(
        &mut self,
        a: Args,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> ToolResult {
        let track: TrackId = a.req_id("track_id", "track")?;
        let t = self
            .agent_project()?
            .tracks
            .get(&track)
            .ok_or_else(|| ToolError::Command(not_found(format!("track {track}"))))?
            .clone();
        let format = match a.str("format") {
            Some(f) => Some(PluginFormat::parse(f).ok_or_else(|| {
                ToolError::input(format!(
                    "unknown plugin format `{f}` (clap, vst3, au, vst2)"
                ))
            })?),
            None => None,
        };
        let new = match (a.str("type"), a.str("plugin_id"), a.str("query")) {
            (Some(ty), _, _) => NewDevice::Builtin(super::edit::builtin_type(ty)?),
            (None, Some(id), _) => NewDevice::Plugin(
                self.agent_find_plugin(Some(id), format, None)?
                    .expect("plugin_id resolves or errors"),
            ),
            (None, None, Some(q)) => {
                // A built-in type first ("reverb", "Poly Synth"), then installed plugins.
                if format.is_none()
                    && let Ok(t) = super::edit::builtin_type(q)
                {
                    NewDevice::Builtin(t)
                } else if let Some(p) = self.agent_find_plugin(None, format, Some(q))? {
                    NewDevice::Plugin(p)
                } else if format.is_none()
                    && let Some(t) = builtin_by_name(q)
                {
                    NewDevice::Builtin(t)
                } else {
                    let plugins = match self.bridge.plugin_catalog() {
                        Some(_) => "no installed plugin",
                        None => {
                            "plugins are not available on this host (web app), and no built-in device"
                        }
                    };
                    return Err(ToolError::input(format!(
                        "{plugins} matches \"{q}\": see list_plugins / list_device_types"
                    )));
                }
            }
            (None, None, None) => {
                return Err(ToolError::input(
                    "pass `type` (built-in device), `plugin_id` (installed plugin) or `query` (a name)",
                ));
            }
        };
        let category = new.category();
        if category == DeviceCategory::Instrument && t.kind != TrackKind::Midi {
            return Err(ToolError::input(format!(
                "instruments go on MIDI tracks; track \"{}\" is {}. Create a MIDI track (create_track kind midi) or pick another track",
                t.name,
                super::read::kind_name(t.kind)
            )));
        }
        if category == DeviceCategory::NoteEffect && t.kind != TrackKind::Midi {
            return Err(ToolError::input("MIDI effects go on MIDI tracks"));
        }
        let index = a.u64("index").map(|i| i as usize);
        let (before, replace) = self.agent_placement(track, category, index)?;
        let replace = replace.filter(|_| a.bool("replace").unwrap_or(true));
        let replaced = replace.and_then(|r| {
            self.agent_project()
                .ok()?
                .devices
                .get(&r)
                .map(|d| json!({ "device_id": r.to_string(), "name": d.name }))
        });
        let id: DeviceId = self.agent_id(now);
        let mut commands = vec![Command::Device(DeviceCommand::Insert {
            id,
            track,
            device: new.spec(),
            before,
        })];
        if let Some(r) = replace {
            commands.push(Command::Device(DeviceCommand::Remove { id: r }));
        }
        let action = if replace.is_some() {
            "replace instrument".into()
        } else {
            crate::doc::label_of(&commands[0], None)
        };
        self.agent_batch(label(&action), &commands, now, out)?;
        let p = self.agent_project()?;
        let position = p.devices_of(track).iter().position(|d| d.id == id);
        let mut v = new.describe();
        v["device_id"] = json!(id.to_string());
        v["name"] = json!(p.devices.get(&id).map(|d| d.name.clone()));
        v["category"] = json!(category_name(category));
        v["track_id"] = json!(track.to_string());
        v["index"] = json!(position);
        if let Some(r) = replaced {
            v["replaced"] = r;
        }
        Ok(v)
    }

    // ─── presets ────────────────────────────────────────────────────────────────────────

    fn agent_presets(
        &mut self,
        device: DeviceId,
        text: Option<String>,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> Result<Vec<PresetInfo>, ToolError> {
        let d = self
            .agent_project()?
            .devices
            .get(&device)
            .cloned()
            .ok_or_else(|| ToolError::Command(not_found(format!("device {device}"))))?;
        let pd = match &d.kind {
            DeviceKind::Builtin { device } => PresetDevice::Builtin {
                device: device.device_type(),
            },
            DeviceKind::Plugin { plugin } => PresetDevice::Plugin {
                format: plugin.format,
                plugin_id: plugin.plugin_id.clone(),
                name: plugin.name.clone(),
                vendor: plugin.vendor.clone(),
            },
            DeviceKind::Missing { .. } => {
                return Err(ToolError::Command(crate::tx::invalid(format!(
                    "\"{}\" is a missing device: replace it first",
                    d.name
                ))));
            }
        };
        match self.agent_dispatch(
            Command::Preset(PresetCommand::List {
                device: Some(pd),
                text,
            }),
            None,
            now,
            out,
        )? {
            ReplyValue::Presets { presets } => Ok(presets),
            _ => Err(ToolError::Command(internal("unexpected preset reply"))),
        }
    }

    pub(super) fn tool_list_device_presets(
        &mut self,
        a: Args,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> ToolResult {
        let device: DeviceId = a.req_id("device_id", "device")?;
        let query = a.str("query").map(str::trim).filter(|q| !q.is_empty());
        let offset = a.u64("offset").unwrap_or(0) as usize;
        let limit = a.u64("limit").unwrap_or(40).max(1) as usize;
        let all = self.agent_presets(device, None, now, out)?;
        let list: Vec<&PresetInfo> = match query {
            Some(text) => {
                let q = resolve::tokens(text);
                let mut scored: Vec<(&PresetInfo, f64)> = all
                    .iter()
                    .map(|p| {
                        let hay = format!("{} {}", p.name, p.meta.tags.join(" "));
                        (
                            p,
                            resolve::score_name(text, &q, &p.name)
                                .max(0.9 * resolve::score_name(text, &q, &hay)),
                        )
                    })
                    .filter(|(_, s)| *s >= resolve::MIN_SCORE)
                    .collect();
                scored.sort_by(|a, b| b.1.total_cmp(&a.1));
                scored.into_iter().map(|(p, _)| p).collect()
            }
            None => all.iter().collect(),
        };
        let items: Vec<Value> = list
            .iter()
            .skip(offset)
            .take(limit)
            .map(|p| {
                let mut m = json!({
                    "preset_id": p.preset.id,
                    "name": p.name,
                    "source": match p.preset.source { PresetSource::Factory => "factory", PresetSource::User => "user" },
                });
                if !p.meta.tags.is_empty() {
                    m["tags"] = json!(p.meta.tags);
                }
                m
            })
            .collect();
        let mut v = json!({ "device_id": device.to_string(), "total": list.len(), "offset": offset, "presets": items });
        if offset + limit < list.len() {
            v["note"] = json!(format!(
                "more presets: call again with offset {}",
                offset + limit
            ));
        } else if all.is_empty() {
            v["note"] = json!(
                "no presets for this device (plugin factory programs are only in the plugin's own window)"
            );
        }
        Ok(v)
    }

    pub(super) fn tool_load_device_preset(
        &mut self,
        a: Args,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> ToolResult {
        let device: DeviceId = a.req_id("device_id", "device")?;
        let wanted = a.req_str("preset")?.trim().to_string();
        let all = self.agent_presets(device, None, now, out)?;
        let preset = match all.iter().find(|p| p.preset.id == wanted) {
            Some(p) => p.clone(),
            None => {
                let q = resolve::tokens(&wanted);
                let mut scored: Vec<(&PresetInfo, f64)> = all
                    .iter()
                    .map(|p| (p, resolve::score_name(&wanted, &q, &p.name)))
                    .filter(|(_, s)| *s >= 0.7)
                    .collect();
                scored.sort_by(|a, b| b.1.total_cmp(&a.1));
                match scored.as_slice() {
                    [] => {
                        return Err(ToolError::input(format!(
                            "no preset of this device matches \"{wanted}\" (see list_device_presets)"
                        )));
                    }
                    [(p, s), rest @ ..] if rest.first().is_none_or(|(_, s2)| s - s2 > 0.05) => {
                        (*p).clone()
                    }
                    many => {
                        return Err(ToolError::Ambiguous {
                            message: format!("\"{wanted}\" matches several presets"),
                            candidates: json!(
                                many.iter()
                                    .take(8)
                                    .map(
                                        |(p, _)| json!({ "preset_id": p.preset.id, "name": p.name })
                                    )
                                    .collect::<Vec<_>>()
                            ),
                        });
                    }
                }
            }
        };
        let seed = self.agent_id(now);
        self.agent_steps(
            vec![Command::Preset(PresetCommand::Load {
                device,
                preset: preset.preset.clone(),
                seed: Some(seed),
            })],
            now,
            out,
        )?;
        Ok(
            json!({ "device_id": device.to_string(), "preset": preset.name, "preset_id": preset.preset.id }),
        )
    }
}

/// Preferred format when one plugin is installed in several.
fn format_rank(f: PluginFormat) -> u8 {
    match f {
        PluginFormat::Clap => 0,
        PluginFormat::Vst3 => 1,
        PluginFormat::Au => 2,
        PluginFormat::Vst2 => 3,
    }
}

/// A built-in type by display name or alias ("Poly Synth", "réverb").
pub(super) fn builtin_by_name(text: &str) -> Option<BuiltinDeviceType> {
    let q = resolve::tokens(text);
    let mut best: Option<(BuiltinDeviceType, f64)> = None;
    for d in ether_devices::offered_descriptors() {
        let ether_core::protocol::devices::DeviceTypeRef::Builtin { device } = d.device_type else {
            continue;
        };
        let s = resolve::score_name(text, &q, &d.name).max(resolve::score_name(
            text,
            &q,
            &device.key().replace('-', " "),
        ));
        if s >= 0.85 && best.is_none_or(|(_, b)| s > b) {
            best = Some((device, s));
        }
    }
    best.map(|(t, _)| t)
}

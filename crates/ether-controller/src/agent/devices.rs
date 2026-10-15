//! Device parameters, drum pads and rack chains (`agent-plugins`): `get_device_params`,
//! `get_device_param`, `set_device_param` (names or ids; plain, display, normalized or
//! relative values), `set_drum_pad`, `load_drum_pad_sample`, `set_rack_chain`.

use ether_core::protocol::Command;
use ether_core::protocol::devices::{DeviceCommand, DeviceDescriptor, ParamInfo, ParamUnit};
use ether_core::protocol::drum_rack::DrumRackCommand;
use ether_core::protocol::media::MediaCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::racks::RackCommand;
use serde_json::{Map, Value, json};

use super::read::{category_name, device_type, r2f, r3};
use super::resolve;
use super::schema::Args;
use super::{ToolError, ToolResult, label};
use crate::store::{Library, ProjectStore};
use crate::tx::not_found;
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// 60 = C3 (Ableton naming).
pub(super) fn note_name(n: u8) -> String {
    format!(
        "{}{}",
        NOTE_NAMES[usize::from(n % 12)],
        i32::from(n / 12) - 2
    )
}

/// "C1", "f#2", "Db1" → MIDI note.
pub(super) fn parse_note(s: &str) -> Option<u8> {
    let s = s.trim();
    if let Ok(n) = s.parse::<u8>() {
        return (n <= 127).then_some(n);
    }
    let mut chars = s.chars();
    let letter = chars.next()?.to_ascii_uppercase();
    let base: i32 = match letter {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let rest: String = chars.collect();
    let (acc, octave) = if let Some(r) = rest.strip_prefix('#') {
        (1, r)
    } else if let Some(r) = rest.strip_prefix('b') {
        (-1, r)
    } else {
        (0, rest.as_str())
    };
    let octave: i32 = octave.parse().ok()?;
    let n = base + acc + (octave + 2) * 12;
    (0..=127).contains(&n).then_some(n as u8)
}

fn unit_name(u: ParamUnit) -> &'static str {
    match u {
        ParamUnit::None => "",
        ParamUnit::Decibels => "dB",
        ParamUnit::Hertz => "Hz",
        ParamUnit::Milliseconds => "ms",
        ParamUnit::Seconds => "s",
        ParamUnit::Percent => "%",
        ParamUnit::Semitones => "st",
        ParamUnit::Ratio => "ratio",
        ParamUnit::Pan => "pan",
        ParamUnit::Toggle => "toggle",
        ParamUnit::Cents => "cents",
    }
}

fn label_index(info: &ParamInfo, plain: f64) -> Option<usize> {
    let labels = info.labels.as_ref()?;
    let n = labels.len().max(2) - 1;
    let span = info.max - info.min;
    if span <= 0.0 {
        return Some(0);
    }
    Some(
        (((plain - info.min) / span) * n as f64)
            .round()
            .clamp(0.0, n as f64) as usize,
    )
}

/// Human display of a plain value ("1.20 kHz", "-6.0 dB", "Saw", "On").
pub(super) fn display(info: &ParamInfo, plain: f64) -> String {
    if let (Some(labels), Some(i)) = (&info.labels, label_index(info, plain))
        && let Some(l) = labels.get(i)
    {
        return l.clone();
    }
    let num = |x: f64| {
        let a = x.abs();
        if a >= 100.0 || x.fract() == 0.0 {
            format!("{x:.0}")
        } else if a >= 10.0 {
            format!("{x:.1}")
        } else {
            format!("{x:.2}")
        }
    };
    match info.unit {
        ParamUnit::Decibels if plain <= info.min + 1e-9 && info.min <= -60.0 => "-inf dB".into(),
        ParamUnit::Decibels => format!("{plain:.1} dB"),
        ParamUnit::Hertz if plain >= 1000.0 => format!("{:.2} kHz", plain / 1000.0),
        ParamUnit::Hertz => format!("{} Hz", num(plain)),
        ParamUnit::Milliseconds if plain >= 1000.0 => format!("{:.2} s", plain / 1000.0),
        ParamUnit::Milliseconds => format!("{} ms", num(plain)),
        ParamUnit::Seconds => format!("{} s", num(plain)),
        ParamUnit::Percent => format!("{} %", num(plain)),
        ParamUnit::Semitones => format!("{plain:+.1} st"),
        ParamUnit::Cents if info.min < 0.0 => format!("{plain:+.0} ct"),
        ParamUnit::Cents => format!("{} ct", num(plain)),
        ParamUnit::Ratio => format!("{plain:.1}:1"),
        ParamUnit::Pan if plain.abs() < 0.005 => "C".into(),
        ParamUnit::Pan => format!(
            "{:.0}{}",
            plain.abs() * 100.0,
            if plain < 0.0 { "L" } else { "R" }
        ),
        ParamUnit::Toggle => if plain >= (info.min + info.max) / 2.0 {
            "On"
        } else {
            "Off"
        }
        .into(),
        ParamUnit::None => num(plain),
    }
}

/// Plugin params that are plumbing, not sound ("MIDI CC 12", "Ch 1 CC 7"): hidden unless
/// `all` is asked.
fn is_plumbing(p: &ParamInfo) -> bool {
    let n = p.name.trim().to_lowercase();
    n.is_empty()
        || n == "-"
        || n.starts_with("midi cc")
        || n.contains(" cc ")
        || (n.starts_with("cc") && n[2..].trim().chars().all(|c| c.is_ascii_digit()))
        || n.starts_with("midi ch")
        || n.starts_with("unused")
        || (n.starts_with("param") && n[5..].trim().chars().all(|c| c.is_ascii_digit()))
}

/// A value given as text with a unit ("2 kHz", "-6dB", "250 ms", "40%") → plain.
fn parse_display(info: &ParamInfo, s: &str) -> Option<f64> {
    let t = s.trim().to_lowercase().replace(',', ".");
    let split = t
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+'))
        .unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    let x: f64 = num.trim().parse().ok()?;
    let unit = unit.trim();
    Some(match (unit, info.unit) {
        ("", _) => x,
        ("khz" | "k", ParamUnit::Hertz) => x * 1000.0,
        ("hz", ParamUnit::Hertz) => x,
        ("s" | "sec" | "secs" | "seconds", ParamUnit::Milliseconds) => x * 1000.0,
        ("ms", ParamUnit::Seconds) => x / 1000.0,
        ("ms", ParamUnit::Milliseconds) | ("s" | "sec", ParamUnit::Seconds) => x,
        ("db", ParamUnit::Decibels) => x,
        ("%", ParamUnit::Percent) => x,
        ("%", _) => info.min + (info.max - info.min) * x / 100.0,
        ("st" | "semitones" | "semi", ParamUnit::Semitones) => x,
        (":1", ParamUnit::Ratio) => x,
        _ => return None,
    })
}

pub(super) fn param_json(info: &ParamInfo, plain: f64) -> Value {
    let mut m = Map::new();
    m.insert("id".into(), json!(info.id.0));
    m.insert("name".into(), json!(info.name));
    if let Some(g) = &info.group {
        m.insert("group".into(), json!(g));
    }
    m.insert("value".into(), json!(r3(plain)));
    m.insert("display".into(), json!(display(info, plain)));
    let unit = unit_name(info.unit);
    if !unit.is_empty() {
        m.insert("unit".into(), json!(unit));
    }
    m.insert("min".into(), json!(r3(info.min)));
    m.insert("max".into(), json!(r3(info.max)));
    m.insert("default".into(), json!(r3(info.default)));
    m.insert(
        "normalized".into(),
        json!(r3(info.to_normalized(plain).clamp(0.0, 1.0))),
    );
    if let Some(labels) = &info.labels {
        m.insert("labels".into(), json!(labels));
    }
    if let Some(step) = info.step {
        m.insert("step".into(), json!(step));
    }
    Value::Object(m)
}

/// The parameter a call names: id (number or numeric text), exact name, "group name", or
/// the best fuzzy name match (ambiguous ties return candidates).
pub(super) fn find_param(
    desc: &DeviceDescriptor,
    device: DeviceId,
    param: &Value,
) -> Result<ParamInfo, ToolError> {
    let visible = || desc.params.iter().filter(|p| !p.hidden);
    let by_id = |n: u64| desc.params.iter().find(|p| u64::from(p.id.0) == n);
    let found = match param {
        Value::Number(n) => n.as_u64().and_then(by_id),
        Value::String(name) => {
            let wanted = name.trim().to_lowercase();
            visible()
                .find(|p| p.name.to_lowercase() == wanted)
                .or_else(|| {
                    visible().find(|p| {
                        p.group.as_ref().is_some_and(|g| {
                            format!("{} {}", g.to_lowercase(), p.name.to_lowercase()) == wanted
                        })
                    })
                })
                .or_else(|| wanted.parse::<u64>().ok().and_then(by_id))
                .or_else(|| {
                    let q = resolve::tokens(name);
                    let mut scored: Vec<(&ParamInfo, f64)> = visible()
                        .map(|p| {
                            let full = match &p.group {
                                Some(g) => format!("{g} {}", p.name),
                                None => p.name.clone(),
                            };
                            let s = resolve::score_name(name, &q, &p.name)
                                .max(0.95 * resolve::score_name(name, &q, &full));
                            (p, s)
                        })
                        .filter(|(_, s)| *s >= 0.7)
                        .collect();
                    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
                    match scored.as_slice() {
                        [(p, _)] => Some(*p),
                        [(p, s), (_, s2), ..] if s - s2 > 0.05 => Some(*p),
                        _ => None,
                    }
                })
        }
        _ => None,
    };
    found.cloned().ok_or_else(|| {
        let text = match param {
            Value::String(s) => s.clone(),
            v => v.to_string(),
        };
        let q = resolve::tokens(&text);
        let mut close: Vec<(&ParamInfo, f64)> = visible()
            .map(|p| (p, resolve::score_name(&text, &q, &p.name)))
            .filter(|(_, s)| *s > 0.3)
            .collect();
        close.sort_by(|a, b| b.1.total_cmp(&a.1));
        if close.len() > 1 {
            ToolError::Ambiguous {
                message: format!(
                    "device {device}: \"{text}\" matches several parameters; pass one id"
                ),
                candidates: json!(
                    close
                        .iter()
                        .take(8)
                        .map(|(p, _)| json!({ "id": p.id.0, "name": p.name, "group": p.group }))
                        .collect::<Vec<_>>()
                ),
            }
        } else {
            ToolError::input(format!(
                "device {device} has no parameter {text}; see get_device_params (query filters by name)"
            ))
        }
    })
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    fn agent_device(&self, id: DeviceId) -> Result<Device, ToolError> {
        self.agent_project()?
            .devices
            .get(&id)
            .cloned()
            .ok_or_else(|| ToolError::Command(not_found(format!("device {id}"))))
    }

    fn agent_device_desc(&mut self, id: DeviceId) -> Result<DeviceDescriptor, ToolError> {
        self.agent_descriptor(id).ok_or_else(|| {
            ToolError::Command(not_found(format!(
                "parameters of device {id} (plugin not loaded: is it installed?)"
            )))
        })
    }

    /// Pads (drum racks) and chains (racks) of a device, for `get_device_params`.
    fn device_inside(&self, p: &Project, d: &Device) -> Map<String, Value> {
        let mut m = Map::new();
        let pads = p.pads_of(d.id);
        if !pads.is_empty() {
            m.insert(
                "pads".into(),
                json!(
                    pads.iter()
                        .map(|pad| {
                            let mut v = json!({
                                "pad_id": pad.id.to_string(),
                                "note": pad.note,
                                "note_name": note_name(pad.note),
                                "name": pad.name,
                                "volume_db": r2f(pad.volume.0),
                            });
                            if pad.pan.0.abs() > 1e-3 {
                                v["pan"] = json!(r2f(pad.pan.0));
                            }
                            if pad.mute {
                                v["mute"] = json!(true);
                            }
                            if let Some(g) = pad.choke_group {
                                v["choke_group"] = json!(g);
                            }
                            let devs: Vec<Value> = p
                                .pad_devices_of(pad.id)
                                .iter()
                                .map(|x| json!({ "id": x.id.to_string(), "name": x.name, "type": device_type(x) }))
                                .collect();
                            if !devs.is_empty() {
                                v["devices"] = json!(devs);
                            }
                            v
                        })
                        .collect::<Vec<_>>()
                ),
            );
        }
        let chains = p.chains_of(d.id);
        if !chains.is_empty() {
            m.insert(
                "chains".into(),
                json!(
                    chains
                        .iter()
                        .map(|c| {
                            let mut v = json!({
                                "chain_id": c.id.to_string(),
                                "name": c.name,
                                "volume_db": r2f(c.volume.0),
                                "pan": r2f(c.pan.0),
                            });
                            if c.mute {
                                v["mute"] = json!(true);
                            }
                            if c.solo {
                                v["solo"] = json!(true);
                            }
                            v["devices"] = json!(
                                p.chain_devices_of(c.id)
                                    .iter()
                                    .map(|x| json!({ "id": x.id.to_string(), "name": x.name, "type": device_type(x) }))
                                    .collect::<Vec<_>>()
                            );
                            v
                        })
                        .collect::<Vec<_>>()
                ),
            );
        }
        m
    }

    pub(super) fn tool_get_device_params(&mut self, a: Args) -> ToolResult {
        let id: DeviceId = a.req_id("device_id", "device")?;
        let d = self.agent_device(id)?;
        let desc = self.agent_device_desc(id)?;
        let offset = a.u64("offset").unwrap_or(0) as usize;
        let limit = a
            .u64("limit")
            .unwrap_or(u64::from(super::tools::DEVICE_PARAMS_PAGE))
            .max(1) as usize;
        let all = a.bool("all").unwrap_or(false);
        let query = a.str("query").map(str::trim).filter(|q| !q.is_empty());
        let q = query.map(resolve::tokens);
        let mut visible: Vec<(&ParamInfo, f64)> = desc
            .params
            .iter()
            .filter(|p| !p.hidden)
            .filter(|p| all || !matches!(d.kind, DeviceKind::Plugin { .. }) || !is_plumbing(p))
            .filter_map(|p| match (query, &q) {
                (Some(text), Some(q)) => {
                    let lower = text.to_lowercase();
                    let contains = p.name.to_lowercase().contains(&lower)
                        || p.group
                            .as_deref()
                            .is_some_and(|g| g.to_lowercase().contains(&lower));
                    let full = match &p.group {
                        Some(g) => format!("{g} {}", p.name),
                        None => p.name.clone(),
                    };
                    let s = if contains {
                        1.0
                    } else {
                        resolve::score_name(text, q, &full)
                    };
                    (s >= resolve::MIN_SCORE).then_some((p, s))
                }
                _ => Some((p, 1.0)),
            })
            .collect();
        if query.is_some() {
            visible.sort_by(|a, b| b.1.total_cmp(&a.1));
        }
        let params: Vec<Value> = visible
            .iter()
            .skip(offset)
            .take(limit)
            .map(|(p, _)| param_json(p, d.params.get(&p.id).copied().unwrap_or(p.default)))
            .collect();
        let shown = params.len();
        let mut device = json!({
            "id": d.id.to_string(),
            "name": d.name,
            "type": device_type(&d),
            "category": category_name(desc.category),
            "track_id": d.track.to_string(),
            "enabled": d.enabled,
        });
        if offset == 0 {
            let p = self.agent_project()?;
            for (k, v) in self.device_inside(p, &d) {
                device[k] = v;
            }
        }
        let mut v = json!({
            "device": device,
            "total": visible.len(),
            "offset": offset,
            "params": params,
        });
        if let Some(q) = query {
            v["query"] = json!(q);
        }
        if offset + limit < visible.len() {
            v["note"] = json!(format!(
                "showing {} of {} parameters: call again with offset {}, or pass `query` to filter by name",
                shown,
                visible.len(),
                offset + limit
            ));
        }
        Ok(v)
    }

    pub(super) fn tool_get_device_param(&mut self, a: Args) -> ToolResult {
        let id: DeviceId = a.req_id("device_id", "device")?;
        let d = self.agent_device(id)?;
        let desc = self.agent_device_desc(id)?;
        let info = find_param(&desc, id, a.get("param").expect("required"))?;
        let plain = d.params.get(&info.id).copied().unwrap_or(info.default);
        let mut v = param_json(&info, plain);
        v["device_id"] = json!(id.to_string());
        v["device"] = json!(d.name);
        // What "a bit" means for this knob: 12 % of its travel.
        if info.labels.is_none() && info.unit != ParamUnit::Toggle {
            let n = info.to_normalized(plain);
            let up = info.snap(info.to_plain((n + 0.12).min(1.0)));
            let down = info.snap(info.to_plain((n - 0.12).max(0.0)));
            v["a_bit_more"] = json!(display(&info, up));
            v["a_bit_less"] = json!(display(&info, down));
        }
        Ok(v)
    }

    pub(super) fn tool_set_device_param(
        &mut self,
        a: Args,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> ToolResult {
        let device: DeviceId = a.req_id("device_id", "device")?;
        let d = self.agent_device(device)?;
        let desc = self.agent_device_desc(device)?;
        let info = find_param(&desc, device, a.get("param").expect("required"))?;
        let current = d.params.get(&info.id).copied().unwrap_or(info.default);
        let given = [
            a.has("value"),
            a.has("normalized"),
            a.has("delta"),
            a.has("delta_percent"),
        ]
        .iter()
        .filter(|x| **x)
        .count();
        if given != 1 {
            return Err(ToolError::input(
                "pass exactly one of `value`, `normalized`, `delta` or `delta_percent`",
            ));
        }
        let plain = if let Some(n) = a.f64("normalized") {
            info.to_plain(n.clamp(0.0, 1.0))
        } else if let Some(dv) = a.f64("delta") {
            current + dv
        } else if let Some(pc) = a.f64("delta_percent") {
            let n = info.to_normalized(current) + pc / 100.0;
            info.to_plain(n.clamp(0.0, 1.0))
        } else {
            match a.get("value").expect("checked") {
                Value::Bool(b) => {
                    if *b {
                        info.max
                    } else {
                        info.min
                    }
                }
                Value::Number(n) => n.as_f64().unwrap_or(info.default),
                Value::String(s) => {
                    let wanted = s.trim().to_lowercase();
                    let by_label = info.labels.as_ref().and_then(|labels| {
                        let i = labels
                            .iter()
                            .position(|l| l.to_lowercase() == wanted)
                            .or_else(|| {
                                labels
                                    .iter()
                                    .position(|l| l.to_lowercase().starts_with(&wanted))
                            })?;
                        let n = labels.len().max(2) - 1;
                        Some(info.min + (info.max - info.min) * i as f64 / n as f64)
                    });
                    let toggle = match wanted.as_str() {
                        "on" | "true" | "yes" => Some(info.max),
                        "off" | "false" | "no" => Some(info.min),
                        _ => None,
                    };
                    match by_label.or(toggle).or_else(|| parse_display(&info, s)) {
                        Some(v) => v,
                        None => {
                            return Err(ToolError::input(match &info.labels {
                                Some(l) => format!(
                                    "`{s}` is not a choice of {}: use one of {}",
                                    info.name,
                                    l.join(", ")
                                ),
                                None => format!(
                                    "{} needs a number in {} (or text like \"2 kHz\", \"-6 dB\", \"40%\")",
                                    info.name,
                                    unit_name(info.unit)
                                ),
                            }));
                        }
                    }
                }
                _ => return Err(ToolError::input("value must be a number, label or boolean")),
            }
        };
        if !plain.is_finite() {
            return Err(ToolError::input("value must be finite"));
        }
        let value = info.snap(plain.clamp(info.min.min(info.max), info.min.max(info.max)));
        self.agent_batch(
            label(format!("set {}", info.name.to_lowercase())),
            &[Command::Device(DeviceCommand::SetParam {
                device,
                param: info.id,
                value,
            })],
            now,
            out,
        )?;
        let stored = self
            .agent_project()?
            .devices
            .get(&device)
            .and_then(|d| d.params.get(&info.id).copied())
            .unwrap_or(value);
        let mut v = json!({
            "device_id": device.to_string(),
            "param": info.id.0,
            "name": info.name,
            "previous": display(&info, current),
            "value": r3(stored),
            "display": display(&info, stored),
        });
        if info.labels.is_some() {
            v["label"] = v["display"].clone();
        }
        if (plain - value).abs() > 1e-6 * (1.0 + plain.abs()) {
            v["note"] = json!(format!(
                "requested {} was clamped/snapped to the range {}..{}",
                r3(plain),
                display(&info, info.min),
                display(&info, info.max)
            ));
        }
        Ok(v)
    }

    // ─── drum pads ──────────────────────────────────────────────────────────────────────

    /// The pad a call names on drum rack `rack`: `pad` (pad id, pad name, note name "C1" or
    /// number) or `note`.
    fn agent_pad(&self, rack: DeviceId, a: &Args) -> Result<Option<DrumPad>, ToolError> {
        let p = self.agent_project()?;
        let pads = p.pads_of(rack);
        if let Some(n) = a.u64("note") {
            return Ok(pads
                .iter()
                .find(|x| u64::from(x.note) == n)
                .map(|x| (*x).clone()));
        }
        let Some(text) = a.str("pad").map(str::trim) else {
            return Err(ToolError::input(
                "pass `pad` (id, name or note like \"C1\") or `note`",
            ));
        };
        if let Ok(id) = text.parse::<DrumPadId>() {
            return Ok(pads.iter().find(|x| x.id == id).map(|x| (*x).clone()));
        }
        if let Some(n) = parse_note(text) {
            return Ok(pads.iter().find(|x| x.note == n).map(|x| (*x).clone()));
        }
        let q = resolve::tokens(text);
        let mut scored: Vec<(&DrumPad, f64)> = pads
            .iter()
            .map(|x| (*x, resolve::score_name(text, &q, &x.name)))
            .filter(|(_, s)| *s >= 0.6)
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        match scored.as_slice() {
            [] => Err(ToolError::input(format!(
                "no pad of this drum rack matches \"{text}\" (get_device_params lists the pads)"
            ))),
            [(x, _)] => Ok(Some((*x).clone())),
            [(x, s), (_, s2), ..] if s - s2 > 0.05 => Ok(Some((*x).clone())),
            many => Err(ToolError::Ambiguous {
                message: format!("\"{text}\" matches several pads"),
                candidates: json!(
                    many.iter()
                        .take(8)
                        .map(|(x, _)| json!({ "pad_id": x.id.to_string(), "name": x.name, "note": x.note }))
                        .collect::<Vec<_>>()
                ),
            }),
        }
    }

    fn agent_drum_rack(&self, id: DeviceId) -> Result<Device, ToolError> {
        let d = self.agent_device(id)?;
        match &d.kind {
            DeviceKind::Builtin { device }
                if device.device_type() == BuiltinDeviceType::DrumRack =>
            {
                Ok(d)
            }
            _ => Err(ToolError::input(format!(
                "device \"{}\" is not a Drum Rack",
                d.name
            ))),
        }
    }

    pub(super) fn tool_set_drum_pad(
        &mut self,
        a: Args,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> ToolResult {
        let rack: DeviceId = a.req_id("device_id", "device")?;
        self.agent_drum_rack(rack)?;
        let pad = self.agent_pad(rack, &a)?.ok_or_else(|| {
            ToolError::input("that pad is empty: load a sample with load_drum_pad_sample")
        })?;
        let id = pad.id;
        let mut commands = Vec::new();
        if let Some(db) = a.f64("volume_db") {
            commands.push(DrumRackCommand::SetPadVolume {
                id,
                volume: Decibels(db as f32),
            });
        }
        if let Some(pan) = a.f64("pan") {
            commands.push(DrumRackCommand::SetPadPan {
                id,
                pan: Pan(pan as f32),
            });
        }
        if let Some(mute) = a.bool("mute") {
            commands.push(DrumRackCommand::SetPadMute { id, mute });
        }
        if let Some(g) = a.u64("choke_group") {
            commands.push(DrumRackCommand::SetChokeGroup {
                id,
                group: (g > 0).then_some(g as u8),
            });
        }
        if let Some(name) = a.str("name") {
            commands.push(DrumRackCommand::RenamePad {
                id,
                name: name.to_string(),
            });
        }
        if let Some(n) = a.u64("new_note") {
            commands.push(DrumRackCommand::SetPadNote { id, note: n as u8 });
        }
        let solo = a.bool("solo");
        if commands.is_empty() && solo.is_none() {
            return Err(ToolError::input(
                "nothing to change: pass volume_db, pan, mute, solo, choke_group, name or new_note",
            ));
        }
        if !commands.is_empty() {
            let commands: Vec<Command> = commands.into_iter().map(Command::DrumRack).collect();
            self.agent_batch(label("edit pad"), &commands, now, out)?;
        }
        if let Some(solo) = solo {
            self.agent_dispatch(
                Command::DrumRack(DrumRackCommand::SetPadSolo { id, solo }),
                None,
                now,
                out,
            )?;
        }
        let p = self.agent_project()?;
        let pad = p
            .drum_pads
            .get(&id)
            .ok_or_else(|| ToolError::Command(crate::tx::internal("pad vanished")))?;
        Ok(json!({
            "device_id": rack.to_string(),
            "pad_id": id.to_string(),
            "name": pad.name,
            "note": pad.note,
            "note_name": note_name(pad.note),
            "volume_db": r2f(pad.volume.0),
            "pan": r2f(pad.pan.0),
            "mute": pad.mute,
            "choke_group": pad.choke_group,
        }))
    }

    pub(super) fn tool_load_drum_pad_sample(
        &mut self,
        a: Args,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> ToolResult {
        let rack: DeviceId = a.req_id("device_id", "device")?;
        self.agent_drum_rack(rack)?;
        let existing = if a.has("pad") || a.has("note") {
            match self.agent_pad(rack, &a) {
                Ok(p) => p,
                // A note name with no pad yet is fine ("load a clap on D1").
                Err(_) if a.str("pad").and_then(parse_note).is_some() => None,
                Err(e) => return Err(e),
            }
        } else {
            return Err(ToolError::input("pass `note` (36 = C1) or `pad`"));
        };
        let note = match (&existing, a.u64("note"), a.str("pad").and_then(parse_note)) {
            (Some(p), _, _) => p.note,
            (None, Some(n), _) => n.min(127) as u8,
            (None, None, Some(n)) => n,
            _ => return Err(ToolError::input("pass `note` (36 = C1)")),
        };
        // The sample: a library item, or the media of an audio clip of the project.
        let (media_cmd, media, name): (Option<Command>, MediaId, String) =
            if let Some(clip) = a.id::<ClipId>("clip_id", "clip")? {
                let p = self.agent_project()?;
                let c = p
                    .clips
                    .get(&clip)
                    .ok_or_else(|| ToolError::Command(not_found(format!("clip {clip}"))))?;
                let ClipContent::Audio(au) = &c.content else {
                    return Err(ToolError::input("clip_id must be an audio clip"));
                };
                (None, au.media, c.name.clone())
            } else {
                let item = self.find_browser_item(a.req_str("item_id")?, now, out)?;
                let Some(source) = item.source.clone() else {
                    return Err(ToolError::input(format!("`{}` is not a sample", item.name)));
                };
                let media: MediaId = self.agent_id(now);
                let name = item
                    .name
                    .rsplit_once('.')
                    .map_or(item.name.clone(), |(n, _)| n.to_string());
                (
                    Some(Command::Media(MediaCommand::Import { id: media, source })),
                    media,
                    name,
                )
            };
        let pad: DrumPadId = self.agent_id(now);
        let device: DeviceId = self.agent_id(now);
        let mut steps = Vec::new();
        steps.extend(media_cmd);
        if let Some(old) = &existing {
            steps.push(Command::DrumRack(DrumRackCommand::RemovePad { id: old.id }));
        }
        steps.push(Command::DrumRack(DrumRackCommand::AddSamplePad {
            pad,
            device,
            rack,
            note,
            media,
        }));
        steps.push(Command::DrumRack(DrumRackCommand::RenamePad {
            id: pad,
            name: name.clone(),
        }));
        self.agent_steps(steps, now, out)?;
        Ok(json!({
            "device_id": rack.to_string(),
            "pad_id": pad.to_string(),
            "sampler_device_id": device.to_string(),
            "note": note,
            "note_name": note_name(note),
            "name": name,
            "replaced": existing.map(|p| p.name),
        }))
    }

    // ─── rack chains ────────────────────────────────────────────────────────────────────

    pub(super) fn tool_set_rack_chain(
        &mut self,
        a: Args,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> ToolResult {
        let text = a.req_str("chain")?.trim().to_string();
        let p = self.agent_project()?;
        let chain = if let Ok(id) = text.parse::<RackChainId>() {
            p.rack_chains.get(&id).cloned()
        } else {
            let rack: Option<DeviceId> = a.id("device_id", "device")?;
            let q = resolve::tokens(&text);
            let mut scored: Vec<(&RackChain, f64)> = p
                .rack_chains
                .values()
                .filter(|c| rack.is_none_or(|r| c.rack == r))
                .map(|c| (c, resolve::score_name(&text, &q, &c.name)))
                .filter(|(_, s)| *s >= 0.6)
                .collect();
            scored.sort_by(|a, b| b.1.total_cmp(&a.1));
            match scored.as_slice() {
                [] => None,
                [(c, _)] => Some((*c).clone()),
                [(c, s), (_, s2), ..] if s - s2 > 0.05 => Some((*c).clone()),
                many => {
                    return Err(ToolError::Ambiguous {
                        message: format!("\"{text}\" matches several rack chains"),
                        candidates: json!(
                            many.iter()
                                .take(8)
                                .map(|(c, _)| json!({ "chain_id": c.id.to_string(), "name": c.name, "rack_device_id": c.rack.to_string() }))
                                .collect::<Vec<_>>()
                        ),
                    });
                }
            }
        }
        .ok_or_else(|| ToolError::input(format!("no rack chain matches \"{text}\"")))?;
        let id = chain.id;
        let mut commands = Vec::new();
        let (volume, pan, mute, solo) = (
            a.f64("volume_db").map(|v| Decibels(v as f32)),
            a.f64("pan").map(|v| Pan(v as f32)),
            a.bool("mute"),
            a.bool("solo"),
        );
        if volume.is_some() || pan.is_some() || mute.is_some() || solo.is_some() {
            commands.push(Command::Rack(RackCommand::SetChainMix {
                id,
                volume,
                pan,
                mute,
                solo,
            }));
        }
        if let Some(name) = a.str("name") {
            commands.push(Command::Rack(RackCommand::RenameChain {
                id,
                name: name.to_string(),
            }));
        }
        if commands.is_empty() {
            return Err(ToolError::input(
                "nothing to change: pass volume_db, pan, mute, solo or name",
            ));
        }
        self.agent_batch(label("edit chain"), &commands, now, out)?;
        let c = self
            .agent_project()?
            .rack_chains
            .get(&id)
            .cloned()
            .ok_or_else(|| ToolError::Command(crate::tx::internal("chain vanished")))?;
        Ok(json!({
            "chain_id": id.to_string(),
            "device_id": c.rack.to_string(),
            "name": c.name,
            "volume_db": r2f(c.volume.0),
            "pan": r2f(c.pan.0),
            "mute": c.mute,
            "solo": c.solo,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes() {
        assert_eq!(note_name(60), "C3");
        assert_eq!(note_name(36), "C1");
        assert_eq!(parse_note("C1"), Some(36));
        assert_eq!(parse_note("f#2"), Some(54));
        assert_eq!(parse_note("Db1"), Some(37));
        assert_eq!(parse_note("38"), Some(38));
        assert_eq!(parse_note("kick"), None);
    }
}

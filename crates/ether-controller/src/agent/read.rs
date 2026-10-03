//! Read tools: compact JSON views of the document for the model.

use ether_core::protocol::Command;
use ether_core::protocol::ReplyValue;
use ether_core::protocol::browser::{BrowserCommand, BrowserQuery, BrowserSort, LibraryItemKind};
use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor, DeviceTypeRef};
use ether_core::protocol::model::*;
use serde_json::{Map, Value, json};

use super::schema::Args;
use super::{AgentState, ExportStatus, ToolError, ToolResult};
use crate::doc::DocHost;
use crate::engine::EngineCtx;
use crate::store::{Library, ProjectStore};
use crate::tx::not_found;
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// Clips listed per track in the overview before truncating.
const OVERVIEW_CLIPS_PER_TRACK: usize = 32;
/// Tracks listed in the overview before truncating.
const OVERVIEW_TRACKS: usize = 128;

/// Round to 3 decimals (beats, dB) so the model doesn't see float noise.
pub(super) fn r3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

pub(super) fn r2f(x: f32) -> f64 {
    ((x as f64) * 100.0).round() / 100.0
}

pub(super) fn kind_name(kind: TrackKind) -> &'static str {
    match kind {
        TrackKind::Audio => "audio",
        TrackKind::Midi => "midi",
        TrackKind::Group => "group",
        TrackKind::Return => "return",
        TrackKind::Master => "master",
        TrackKind::Vca => "vca",
    }
}

pub(super) fn category_name(c: DeviceCategory) -> &'static str {
    match c {
        DeviceCategory::Instrument => "instrument",
        DeviceCategory::AudioEffect => "audio_effect",
        DeviceCategory::NoteEffect => "midi_effect",
    }
}

pub(super) fn device_type(d: &Device) -> String {
    match &d.kind {
        DeviceKind::Builtin { device } => device.device_type().key(),
        DeviceKind::Plugin { plugin } => {
            format!("plugin:{}:{}", plugin.format.as_str(), plugin.plugin_id)
        }
    }
}

pub(super) fn color_hex(c: Color) -> String {
    format!("#{:06x}", c.0 & 0xff_ffff)
}

/// MIDI velocity 1..=127 of a model velocity 0..=1.
pub(super) fn midi_velocity(v: f32) -> u8 {
    ((v as f64) * 127.0).round().clamp(1.0, 127.0) as u8
}

fn clip_summary(p: &Project, c: &Clip) -> Value {
    let mut m = Map::new();
    m.insert("id".into(), json!(c.id.to_string()));
    m.insert("name".into(), json!(c.name));
    m.insert("start_beats".into(), json!(r3(c.start.0)));
    m.insert("length_beats".into(), json!(r3(c.length.0)));
    match &c.content {
        ClipContent::Midi => {
            m.insert("kind".into(), json!("midi"));
            m.insert(
                "notes".into(),
                json!(p.notes.values().filter(|n| n.clip == c.id).count()),
            );
        }
        ClipContent::Audio(a) => {
            m.insert("kind".into(), json!("audio"));
            if let Some(media) = p.media.get(&a.media) {
                m.insert("media".into(), json!(media.name));
            }
        }
    }
    if c.muted {
        m.insert("muted".into(), json!(true));
    }
    if c.looping.enabled {
        m.insert(
            "loop".into(),
            json!([r3(c.looping.start.0), r3(c.looping.end.0)]),
        );
    }
    Value::Object(m)
}

fn device_summary(d: &Device) -> Value {
    let mut m = Map::new();
    m.insert("id".into(), json!(d.id.to_string()));
    m.insert("name".into(), json!(d.name));
    m.insert("type".into(), json!(device_type(d)));
    if !d.enabled {
        m.insert("enabled".into(), json!(false));
    }
    Value::Object(m)
}

/// Tracks in display order (groups followed by their children, depth-first).
pub(super) fn tracks_in_order(p: &Project) -> Vec<&Track> {
    fn walk<'a>(p: &'a Project, t: &'a Track, out: &mut Vec<&'a Track>) {
        out.push(t);
        for c in p.child_tracks(t.id) {
            walk(p, c, out);
        }
    }
    let mut out = Vec::with_capacity(p.tracks.len());
    for t in p.tracks_ordered() {
        walk(p, t, &mut out);
    }
    out
}

fn export_status_json(job: &str, s: &ExportStatus) -> Value {
    match s {
        ExportStatus::Running { progress } => {
            json!({ "job_id": job, "status": "running", "progress": r2f(*progress) })
        }
        ExportStatus::Done { result } => json!({
            "job_id": job,
            "status": "done",
            "result": serde_json::to_value(result).unwrap_or(Value::Null),
        }),
        ExportStatus::Failed { message } => {
            json!({ "job_id": job, "status": "failed", "message": message })
        }
        ExportStatus::Cancelled => json!({ "job_id": job, "status": "cancelled" }),
    }
}

impl AgentState {
    fn selection_json(&self) -> Value {
        let Some(s) = &self.selection else {
            return Value::Null;
        };
        fn list<I: ToString>(ids: &[I], max: usize) -> Value {
            json!(
                ids.iter()
                    .take(max)
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            )
        }
        let mut m = Map::new();
        if !s.selected_tracks.is_empty() {
            m.insert("tracks".into(), list(&s.selected_tracks, 64));
        }
        if !s.selected_clips.is_empty() {
            m.insert("clips".into(), list(&s.selected_clips, 64));
        }
        if !s.selected_notes.is_empty() {
            m.insert("notes".into(), list(&s.selected_notes, 64));
        }
        if !s.selected_devices.is_empty() {
            m.insert("devices".into(), list(&s.selected_devices, 64));
        }
        if let Some(c) = s.editing_clip {
            m.insert("open_clip_id".into(), json!(c.to_string()));
        }
        if let Some(c) = s.cursor {
            m.insert("cursor_beats".into(), json!(r3(c.0)));
        }
        Value::Object(m)
    }
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    pub(super) fn agent_descriptor(&mut self, device: DeviceId) -> Option<DeviceDescriptor> {
        let d = self.doc.as_ref()?.project.devices.get(&device)?.clone();
        let mut host = EngineCtx {
            bridge: &mut self.bridge,
            eng: &mut self.engine,
            media: &self.media,
        };
        host.descriptor(d.id, &d.kind)
    }

    pub(super) fn tool_overview(&mut self) -> ToolResult {
        let p = self.agent_project()?;
        let map = p.tempo_map();
        let at = self.transport.position;
        let sig = map.signature_at(at);
        let mut tracks = Vec::new();
        let ordered = tracks_in_order(p);
        for t in ordered.iter().take(OVERVIEW_TRACKS) {
            let mut m = Map::new();
            m.insert("id".into(), json!(t.id.to_string()));
            m.insert("name".into(), json!(t.name));
            m.insert("kind".into(), json!(kind_name(t.kind)));
            if let Some(parent) = t.parent {
                m.insert("parent_track_id".into(), json!(parent.to_string()));
            }
            m.insert("volume_db".into(), json!(r2f(t.mixer.volume.0)));
            m.insert("pan".into(), json!(r2f(t.mixer.pan.0)));
            if t.mixer.mute {
                m.insert("mute".into(), json!(true));
            }
            if t.mixer.solo {
                m.insert("solo".into(), json!(true));
            }
            if self.armed.contains(&t.id) {
                m.insert("armed".into(), json!(true));
            }
            if t.freeze.is_some() {
                m.insert("frozen".into(), json!(true));
            }
            let devices: Vec<Value> = p.devices_of(t.id).into_iter().map(device_summary).collect();
            if !devices.is_empty() {
                m.insert("devices".into(), Value::Array(devices));
            }
            let clips = p.arrangement_clips_of(t.id);
            if !clips.is_empty() {
                m.insert(
                    "clips".into(),
                    Value::Array(
                        clips
                            .iter()
                            .take(OVERVIEW_CLIPS_PER_TRACK)
                            .map(|c| clip_summary(p, c))
                            .collect(),
                    ),
                );
                if clips.len() > OVERVIEW_CLIPS_PER_TRACK {
                    m.insert(
                        "clips_note".into(),
                        json!(format!(
                            "{} more clips: use get_track",
                            clips.len() - OVERVIEW_CLIPS_PER_TRACK
                        )),
                    );
                }
            }
            tracks.push(Value::Object(m));
        }
        let mut v = json!({
            "project": { "id": p.id.to_string(), "name": p.settings.name },
            "tempo_bpm": r3(map.bpm_at(at)),
            "time_signature": format!("{}/{}", sig.numerator, sig.denominator),
            "loop": {
                "enabled": p.settings.loop_enabled,
                "start_beats": r3(p.settings.loop_region.start.0),
                "end_beats": r3(p.settings.loop_region.end.0),
            },
            "transport": {
                "playing": self.transport.playing,
                "recording": self.transport.recording,
                "playhead_beats": r3(at.0),
            },
            "history": self.agent_history(),
            "selection": self.agent.selection_json(),
            "tracks": tracks,
        });
        if p.tempo_points.len() > 1 || p.time_signatures.len() > 1 {
            v["tempo_changes"] = json!(p.tempo_points.len() + p.time_signatures.len() - 2);
        }
        if ordered.len() > OVERVIEW_TRACKS {
            v["note"] = json!(format!(
                "{} more tracks not listed",
                ordered.len() - OVERVIEW_TRACKS
            ));
        }
        Ok(v)
    }

    pub(super) fn tool_get_track(&mut self, a: Args) -> ToolResult {
        let id: TrackId = a.req_id("track_id", "track")?;
        let p = self.agent_project()?;
        let t = p
            .tracks
            .get(&id)
            .ok_or_else(|| ToolError::Command(not_found(format!("track {id}"))))?;
        let offset = a.u64("clip_offset").unwrap_or(0) as usize;
        let limit = a.u64("clip_limit").unwrap_or(200) as usize;
        let clips = p.arrangement_clips_of(id);
        let sends: Vec<Value> = p
            .sends
            .values()
            .filter(|s| s.from == id)
            .map(|s| {
                json!({
                    "id": s.id.to_string(),
                    "to_track_id": s.to.to_string(),
                    "level_db": r2f(s.level.0),
                    "pre_fader": s.pre_fader,
                })
            })
            .collect();
        let devices: Vec<Value> = p
            .devices_of(id)
            .into_iter()
            .enumerate()
            .map(|(i, d)| {
                let mut v = device_summary(d);
                v["index"] = json!(i);
                v
            })
            .collect();
        let items: Vec<Value> = clips
            .iter()
            .skip(offset)
            .take(limit)
            .map(|c| clip_summary(p, c))
            .collect();
        let mut v = json!({
            "id": t.id.to_string(),
            "name": t.name,
            "kind": kind_name(t.kind),
            "color": color_hex(t.color),
            "parent_track_id": t.parent.map(|p| p.to_string()),
            "child_track_ids": p.child_tracks(id).iter().map(|c| c.id.to_string()).collect::<Vec<_>>(),
            "mixer": {
                "volume_db": r2f(t.mixer.volume.0),
                "pan": r2f(t.mixer.pan.0),
                "mute": t.mixer.mute,
                "solo": t.mixer.solo,
                "armed": self.armed.contains(&id),
            },
            "input": serde_json::to_value(&t.input).unwrap_or(Value::Null),
            "output": serde_json::to_value(&t.output).unwrap_or(Value::Null),
            "frozen": t.freeze.is_some(),
            "sends": sends,
            "devices": devices,
            "clips": { "total": clips.len(), "offset": offset, "items": items },
        });
        if offset + limit < clips.len() {
            v["clips"]["note"] = json!(format!(
                "more clips: call again with clip_offset {}",
                offset + limit
            ));
        }
        Ok(v)
    }

    pub(super) fn tool_get_clip_notes(&mut self, a: Args) -> ToolResult {
        let id: ClipId = a.req_id("clip_id", "clip")?;
        let p = self.agent_project()?;
        let c = p
            .clips
            .get(&id)
            .ok_or_else(|| ToolError::Command(not_found(format!("clip {id}"))))?;
        if !matches!(c.content, ClipContent::Midi) {
            return Err(ToolError::input(format!(
                "clip {id} is an audio clip (no notes)"
            )));
        }
        let offset = a.u64("offset").unwrap_or(0) as usize;
        let limit = a.u64("limit").unwrap_or(500) as usize;
        let notes = p.notes_of(id);
        let items: Vec<Value> = notes
            .iter()
            .skip(offset)
            .take(limit)
            .map(|n| {
                let mut m = Map::new();
                m.insert("id".into(), json!(n.id.to_string()));
                m.insert("pitch".into(), json!(n.pitch));
                m.insert("start_beats".into(), json!(r3(n.start.0)));
                m.insert("duration_beats".into(), json!(r3(n.duration.0)));
                m.insert("velocity".into(), json!(midi_velocity(n.velocity)));
                if n.muted {
                    m.insert("muted".into(), json!(true));
                }
                Value::Object(m)
            })
            .collect();
        let mut v = json!({
            "clip": {
                "id": c.id.to_string(),
                "name": c.name,
                "track_id": c.track.to_string(),
                "start_beats": r3(c.start.0),
                "length_beats": r3(c.length.0),
                "loop": { "enabled": c.looping.enabled, "start_beats": r3(c.looping.start.0), "end_beats": r3(c.looping.end.0) },
            },
            "total": notes.len(),
            "offset": offset,
            "notes": items,
        });
        if offset + limit < notes.len() {
            v["note"] = json!(format!(
                "more notes: call again with offset {}",
                offset + limit
            ));
        }
        Ok(v)
    }

    pub(super) fn tool_list_device_types(&mut self, a: Args) -> ToolResult {
        let only = a.str("category");
        let types: Vec<Value> = ether_devices::all_descriptors()
            .into_iter()
            .filter_map(|d| {
                let DeviceTypeRef::Builtin { device } = d.device_type else {
                    return None;
                };
                let category = category_name(d.category);
                if only.is_some_and(|o| o != category) {
                    return None;
                }
                Some(json!({ "type": device.key(), "name": d.name, "category": category }))
            })
            .collect();
        Ok(json!({ "device_types": types }))
    }

    pub(super) fn tool_get_device_params(&mut self, a: Args) -> ToolResult {
        let id: DeviceId = a.req_id("device_id", "device")?;
        let d = self
            .agent_project()?
            .devices
            .get(&id)
            .cloned()
            .ok_or_else(|| ToolError::Command(not_found(format!("device {id}"))))?;
        let desc = self.agent_descriptor(id).ok_or_else(|| {
            ToolError::Command(not_found(format!(
                "parameters of device {id} (plugin not loaded)"
            )))
        })?;
        let offset = a.u64("offset").unwrap_or(0) as usize;
        let limit = a
            .u64("limit")
            .unwrap_or(u64::from(super::tools::DEVICE_PARAMS_PAGE))
            .max(1) as usize;
        let query = a
            .str("query")
            .map(str::trim)
            .filter(|q| !q.is_empty())
            .map(str::to_lowercase);
        let matches = |p: &&ether_core::protocol::devices::ParamInfo| {
            query.as_deref().is_none_or(|q| {
                p.name.to_lowercase().contains(q)
                    || p.group
                        .as_deref()
                        .is_some_and(|g| g.to_lowercase().contains(q))
            })
        };
        let visible: Vec<_> = desc
            .params
            .iter()
            .filter(|p| !p.hidden)
            .filter(matches)
            .collect();
        let params: Vec<Value> = visible
            .iter()
            .skip(offset)
            .take(limit)
            .map(|p| {
                let value = d.params.get(&p.id).copied().unwrap_or(p.default);
                let mut m = Map::new();
                m.insert("id".into(), json!(p.id.0));
                m.insert("name".into(), json!(p.name));
                if let Some(g) = &p.group {
                    m.insert("group".into(), json!(g));
                }
                m.insert("unit".into(), json!(format!("{:?}", p.unit).to_lowercase()));
                m.insert("min".into(), json!(p.min));
                m.insert("max".into(), json!(p.max));
                m.insert("default".into(), json!(p.default));
                m.insert("value".into(), json!(r3(value)));
                if let Some(labels) = &p.labels {
                    m.insert("labels".into(), json!(labels));
                }
                if let Some(step) = p.step {
                    m.insert("step".into(), json!(step));
                }
                Value::Object(m)
            })
            .collect();
        let shown = params.len();
        let mut v = json!({
            "device": {
                "id": d.id.to_string(),
                "name": d.name,
                "type": device_type(&d),
                "category": category_name(desc.category),
                "track_id": d.track.to_string(),
                "enabled": d.enabled,
            },
            "total": visible.len(),
            "offset": offset,
            "params": params,
        });
        if let Some(q) = &query {
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

    pub(super) fn tool_export_status(&mut self, a: Args) -> ToolResult {
        let found = match a.str("job_id") {
            Some(job) => self.agent.exports.iter().find(|(j, _)| j == job),
            None => self.agent.exports.last(),
        };
        match found {
            Some((job, s)) => Ok(export_status_json(job, s)),
            None => Err(ToolError::input(match a.str("job_id") {
                Some(job) => format!("no export job `{job}` (only recent jobs are kept)"),
                None => "no export was started yet".to_string(),
            })),
        }
    }

    pub(super) fn tool_search_browser(
        &mut self,
        a: Args,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> ToolResult {
        let kinds = match a.str("kind") {
            Some("audio") => vec![LibraryItemKind::Audio],
            Some("midi") => vec![LibraryItemKind::Midi],
            Some("preset") => vec![LibraryItemKind::Preset],
            _ => Vec::new(),
        };
        let text = a.str("text").unwrap_or_default().to_string();
        let query = BrowserQuery {
            sort: if text.trim().is_empty() {
                BrowserSort::Name
            } else {
                BrowserSort::Relevance
            },
            text,
            kinds,
            tags: Vec::new(),
            favourites_only: false,
            roots: Vec::new(),
            folder: None,
            device: None,
            offset: a.u64("offset").unwrap_or(0) as u32,
            limit: a.u64("limit").unwrap_or(20) as u32,
        };
        let reply = self.agent_dispatch(
            Command::Browser(BrowserCommand::Query { query }),
            None,
            now,
            out,
        )?;
        let ReplyValue::BrowserPage { page } = reply else {
            return Err(ToolError::Command(crate::tx::internal(
                "unexpected browser reply",
            )));
        };
        let items: Vec<Value> = page
            .items
            .iter()
            .map(|i| {
                let mut m = Map::new();
                m.insert("id".into(), json!(i.id));
                m.insert("kind".into(), json!(format!("{:?}", i.kind).to_lowercase()));
                m.insert("name".into(), json!(i.name));
                m.insert("path".into(), json!(i.path));
                if let Some(d) = i.meta.duration_seconds {
                    m.insert("seconds".into(), json!(r3(d)));
                }
                if let Some(b) = i.meta.bpm {
                    m.insert("bpm".into(), json!(r3(b)));
                }
                if let Some(k) = &i.meta.key {
                    m.insert("key".into(), json!(k));
                }
                if !i.tags.is_empty() {
                    m.insert("tags".into(), json!(i.tags));
                }
                Value::Object(m)
            })
            .collect();
        let shown = page.offset as usize + items.len();
        let mut v = json!({ "total": page.total, "offset": page.offset, "items": items });
        if shown < page.total as usize {
            v["note"] = json!(format!("more results: call again with offset {shown}"));
        }
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(r3(1.000_000_1), 1.0);
        assert_eq!(midi_velocity(1.0), 127);
        assert_eq!(midi_velocity(0.0), 1);
        assert_eq!(midi_velocity(100.0 / 127.0), 100);
        assert_eq!(color_hex(Color(0x00ff_8800)), "#ff8800");
    }
}

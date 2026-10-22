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
        DeviceKind::Missing { missing } => format!("missing:{}", missing.source_id),
    }
}

pub(super) fn color_hex(c: Color) -> String {
    format!("#{:06x}", c.0 & 0xff_ffff)
}

/// MIDI velocity 1..=127 of a model velocity 0..=1.
pub(super) fn midi_velocity(v: f32) -> u8 {
    ((v as f64) * 127.0).round().clamp(1.0, 127.0) as u8
}

/// Compact clip: id, name, kind, start/end/length in beats, bars ("5-8", 1-based,
/// inclusive), loop, notes (MIDI) or media (audio).
pub(super) fn clip_summary(p: &Project, map: &TempoMap, c: &Clip) -> Value {
    let end = c.start.0 + c.length.0;
    let mut m = Map::new();
    m.insert("id".into(), json!(c.id.to_string()));
    m.insert("name".into(), json!(c.name));
    m.insert("start_beats".into(), json!(r3(c.start.0)));
    m.insert("end_beats".into(), json!(r3(end)));
    m.insert("length_beats".into(), json!(r3(c.length.0)));
    m.insert("bars".into(), json!(bars(map, c.start.0, end)));
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

/// "5-8": the bars (1-based, inclusive) that `[start, end)` touches.
pub(super) fn bars(map: &TempoMap, start: f64, end: f64) -> String {
    let a = map.bar_beat(Beats(start)).bar;
    // Just before the end (bar_beat snaps values within its epsilon of a bar line).
    let b = map.bar_beat(Beats((end - 0.01).max(start))).bar;
    if a == b {
        format!("{a}")
    } else {
        format!("{a}-{b}")
    }
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

/// A device with what is inside it (rack chains and their devices, drum pads), compact.
pub(super) fn device_tree(p: &Project, d: &Device, depth: usize) -> Value {
    let mut v = device_summary(d);
    let chains = p.chains_of(d.id);
    if !chains.is_empty() && depth < 3 {
        v["chains"] = json!(
            chains
                .iter()
                .map(|c| {
                    let mut cm = json!({ "id": c.id.to_string(), "name": c.name });
                    if c.mute {
                        cm["mute"] = json!(true);
                    }
                    let devs: Vec<Value> = p
                        .chain_devices_of(c.id)
                        .into_iter()
                        .map(|x| device_tree(p, x, depth + 1))
                        .collect();
                    if !devs.is_empty() {
                        cm["devices"] = json!(devs);
                    }
                    cm
                })
                .collect::<Vec<_>>()
        );
    }
    let pads = p.pads_of(d.id);
    if !pads.is_empty() {
        v["pads"] = json!(pads.len());
    }
    v
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
        if !s.tracks.is_empty() {
            m.insert("tracks".into(), list(&s.tracks, 64));
        }
        if !s.clips.is_empty() {
            m.insert("clips".into(), list(&s.clips, 64));
        }
        if !s.notes.is_empty() {
            m.insert("notes".into(), list(&s.notes, 64));
        }
        if !s.devices.is_empty() {
            m.insert("devices".into(), list(&s.devices, 64));
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

impl<B, H, S, L, X> EtherController<B, H, S, L, X>
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
                if let Some(g) = p.tracks.get(&parent) {
                    m.insert("group".into(), json!(g.name));
                }
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
            if let Some(f) = &t.freeze {
                m.insert("frozen".into(), json!(true));
                // Partial freeze: the devices after this one stay live and editable.
                if let Some(d) = f.up_to {
                    m.insert("frozenUpTo".into(), json!(d.to_string()));
                }
            }
            if let Some(g) = p.frozen_ancestor(t.id) {
                m.insert("frozenByGroup".into(), json!(g.to_string()));
            }
            let devices: Vec<Value> = p
                .devices_of(t.id)
                .into_iter()
                .map(|d| device_tree(p, d, 0))
                .collect();
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
                            .map(|c| clip_summary(p, &map, c))
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
        let end = super::resolve::song_end(p);
        let sections: Vec<Value> = super::resolve::sections(p)
            .into_iter()
            .map(|(mk, e)| {
                json!({
                    "id": mk.id.to_string(),
                    "name": mk.name,
                    "start_beats": r3(mk.position.0),
                    "end_beats": r3(e),
                    "bars": bars(&map, mk.position.0, e.max(mk.position.0 + 1e-3)),
                })
            })
            .collect();
        let mut v = json!({
            "project": { "id": p.id.to_string(), "name": p.settings.name },
            "tempo_bpm": r3(map.bpm_at(at)),
            "time_signature": format!("{}/{}", sig.numerator, sig.denominator),
            "beats_per_bar": r3(f64::from(sig.numerator) * 4.0 / f64::from(sig.denominator)),
            "song_end_beats": r3(end),
            "song_seconds": r3(map.beats_to_seconds(Beats(end)).0),
            "sections": sections,
            "loop": {
                "enabled": p.settings.loop_enabled,
                "start_beats": r3(p.settings.loop_region.start.0),
                "end_beats": r3(p.settings.loop_region.end.0),
            },
            "transport": {
                "playing": self.transport.playing,
                "recording": self.transport.recording,
                "playhead_beats": r3(at.0),
                "playhead_bar": map.bar_beat(at).bar,
            },
            "history": self.agent_history(),
            "selection": self.agent.selection_json(),
            "tracks": tracks,
        });
        // Still loading: play/stop/record are refused until it is ready.
        if let Some(l) = self.loading.progress() {
            v["transport"]["loading"] = json!({
                "plugins_loaded": l.plugins_loaded,
                "plugins_total": l.plugins_total,
                "media_loaded": l.media_loaded,
                "media_total": l.media_total,
            });
        }
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
        let map = p.tempo_map();
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
                let mut v = device_tree(p, d, 0);
                v["index"] = json!(i);
                v
            })
            .collect();
        let items: Vec<Value> = clips
            .iter()
            .skip(offset)
            .take(limit)
            .map(|c| clip_summary(p, &map, c))
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
        let types: Vec<Value> = ether_devices::offered_descriptors()
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

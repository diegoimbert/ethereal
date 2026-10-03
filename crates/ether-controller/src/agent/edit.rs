//! Edit tools: each call is ONE undo step (`AI: <action>`), built from ordinary document
//! commands so validation, engine effects, patches and collab replication are the same as
//! for UI edits.

use ether_core::protocol::Command;
use ether_core::protocol::ReplyValue;
use ether_core::protocol::browser::{BrowserCommand, BrowserQuery, BrowserSort, LibraryItem};
use ether_core::protocol::clips::{ClipCommand, ClipMove};
use ether_core::protocol::devices::{DeviceCategory, DeviceCommand, DeviceSpec};
use ether_core::protocol::export::{
    AudioContainer, BitDepth, ExportCommand, ExportFormat, ExportMode, ExportRange, ExportRequest,
};
use ether_core::protocol::media::MediaCommand;
use ether_core::protocol::mixer::MixerCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteSpec};
use ether_core::protocol::presets::PresetCommand;
use ether_core::protocol::project::{EditCommand, ProjectCommand};
use ether_core::protocol::recording::RecordingCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::transport::TransportCommand;
use serde_json::{Value, json};

use super::read::{category_name, r2f, r3};
use super::schema::Args;
use super::{ToolError, ToolResult, label};
use crate::store::{Library, ProjectStore};
use crate::tx::{internal, not_found};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// A built-in device type from a key such as `poly-synth` (also accepts `Poly Synth`,
/// `poly_synth`, `PolySynth`).
pub(super) fn builtin_type(key: &str) -> Result<BuiltinDeviceType, ToolError> {
    let norm = |s: &str| {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase()
    };
    let wanted = norm(key);
    BuiltinDeviceType::ALL
        .into_iter()
        .find(|t| norm(&t.key()) == wanted)
        .ok_or_else(|| {
            let keys: Vec<String> = BuiltinDeviceType::ALL.iter().map(|t| t.key()).collect();
            ToolError::input(format!(
                "unknown device type `{key}`. Built-in types: {}",
                keys.join(", ")
            ))
        })
}

fn parse_color(s: &str) -> Result<Color, ToolError> {
    let hex = s.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return Err(ToolError::input(format!("color `{s}` must be \"#RRGGBB\"")));
    }
    u32::from_str_radix(hex, 16)
        .map(Color)
        .map_err(|_| ToolError::input(format!("color `{s}` must be \"#RRGGBB\"")))
}

/// Velocity 1..=127 → model 0..=1.
fn velocity(a: &Args) -> f32 {
    (a.f64("velocity").unwrap_or(100.0).clamp(1.0, 127.0) / 127.0) as f32
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    fn track_of(&self, id: TrackId) -> Result<&Track, ToolError> {
        self.agent_project()?
            .tracks
            .get(&id)
            .ok_or_else(|| ToolError::Command(not_found(format!("track {id}"))))
    }

    fn clip_of(&self, id: ClipId) -> Result<&Clip, ToolError> {
        self.agent_project()?
            .clips
            .get(&id)
            .ok_or_else(|| ToolError::Command(not_found(format!("clip {id}"))))
    }

    /// Note specs (with fresh ids) from a `notes` array of the input.
    fn note_specs(&mut self, notes: &[Value], now: u64) -> Result<Vec<NoteSpec>, ToolError> {
        notes
            .iter()
            .map(|n| {
                let n = Args(
                    n.as_object()
                        .ok_or_else(|| ToolError::input("notes must be objects"))?,
                );
                Ok(NoteSpec {
                    id: self.agent_id(now),
                    pitch: n.req_f64("pitch")?.clamp(0.0, 127.0) as u8,
                    velocity: velocity(&n),
                    start: Beats(n.req_f64("start_beats")?),
                    duration: Beats(n.req_f64("duration_beats")?),
                })
            })
            .collect()
    }

    // ─── Tracks ─────────────────────────────────────────────────────────────────────────

    pub(super) fn tool_create_track(
        &mut self,
        a: Args,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> ToolResult {
        self.agent_project()?;
        let kind = match a.req_str("kind")? {
            "midi" => TrackKind::Midi,
            "audio" => TrackKind::Audio,
            "return" => TrackKind::Return,
            _ => TrackKind::Group,
        };
        let instrument = match (kind, a.str("instrument")) {
            (TrackKind::Midi, None) => Some(BuiltinDeviceType::Synth),
            (TrackKind::Midi, Some(k)) if k.eq_ignore_ascii_case("none") => None,
            (TrackKind::Midi, Some(k)) => {
                let t = builtin_type(k)?;
                if ether_devices::descriptor(t).category != DeviceCategory::Instrument {
                    return Err(ToolError::input(format!(
                        "`{k}` is not an instrument (see list_device_types)"
                    )));
                }
                Some(t)
            }
            (_, Some(_)) => {
                return Err(ToolError::input("`instrument` is only for MIDI tracks"));
            }
            (_, None) => None,
        };
        let color = a.str("color").map(parse_color).transpose()?;
        let id: TrackId = self.agent_id(now);
        let mut commands = vec![Command::Track(TrackCommand::Create {
            id,
            kind,
            name: a.str("name").map(str::to_string),
            color,
            parent: a.id("parent_track_id", "track")?,
            before: a.id("before_track_id", "track")?,
        })];
        let device: Option<DeviceId> = instrument.map(|_| self.agent_id(now));
        if let (Some(t), Some(device)) = (instrument, device) {
            commands.push(Command::Device(DeviceCommand::Insert {
                id: device,
                track: id,
                device: DeviceSpec::Builtin {
                    device: BuiltinDevice::new(t),
                },
                before: None,
            }));
        }
        self.agent_batch(&label("Create Track"), &commands, now, out)?;
        let t = self.track_of(id)?;
        Ok(json!({
            "track_id": id.to_string(),
            "name": t.name,
            "kind": super::read::kind_name(t.kind),
            "instrument_device_id": device.map(|d| d.to_string()),
        }))
    }

    pub(super) fn tool_delete_track(
        &mut self,
        a: Args,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> ToolResult {
        let id: TrackId = a.req_id("track_id", "track")?;
        let t = self.track_of(id)?;
        if t.kind == TrackKind::Master {
            return Err(ToolError::input("the master track cannot be deleted"));
        }
        let name = t.name.clone();
        self.agent_batch(
            &label("Delete Track"),
            &[Command::Track(TrackCommand::Delete { id })],
            now,
            out,
        )?;
        Ok(json!({ "track_id": id.to_string(), "deleted": true, "name": name }))
    }

    pub(super) fn tool_rename_track(
        &mut self,
        a: Args,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> ToolResult {
        let id: TrackId = a.req_id("track_id", "track")?;
        let name = a.req_str("name")?.to_string();
        self.track_of(id)?;
        self.agent_batch(
            &label("Rename Track"),
            &[Command::Track(TrackCommand::Rename { id, name })],
            now,
            out,
        )?;
        Ok(json!({ "track_id": id.to_string(), "name": self.track_of(id)?.name }))
    }

    pub(super) fn tool_set_track_mix(
        &mut self,
        a: Args,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> ToolResult {
        let track: TrackId = a.req_id("track_id", "track")?;
        self.track_of(track)?;
        let mut commands = Vec::new();
        if let Some(db) = a.f64("volume_db") {
            commands.push(Command::Mixer(MixerCommand::SetVolume {
                track,
                volume: Decibels(db as f32),
            }));
        }
        if let Some(pan) = a.f64("pan") {
            commands.push(Command::Mixer(MixerCommand::SetPan {
                track,
                pan: Pan(pan as f32),
            }));
        }
        if let Some(mute) = a.bool("mute") {
            commands.push(Command::Mixer(MixerCommand::SetMute { track, mute }));
        }
        if let Some(solo) = a.bool("solo") {
            commands.push(Command::Mixer(MixerCommand::SetSolo {
                track,
                solo,
                exclusive: false,
            }));
        }
        let arm = a.bool("arm");
        if commands.is_empty() && arm.is_none() {
            return Err(ToolError::input(
                "nothing to change: pass volume_db, pan, mute, solo or arm",
            ));
        }
        if !commands.is_empty() {
            self.agent_batch(&label("Set Mix"), &commands, now, out)?;
        }
        if let Some(armed) = arm {
            self.agent_dispatch(
                Command::Recording(RecordingCommand::Arm {
                    track,
                    armed,
                    exclusive: false,
                }),
                None,
                now,
                out,
            )?;
        }
        let t = self.track_of(track)?;
        Ok(json!({
            "track_id": track.to_string(),
            "volume_db": r2f(t.mixer.volume.0),
            "pan": r2f(t.mixer.pan.0),
            "mute": t.mixer.mute,
            "solo": t.mixer.solo,
            "armed": self.armed.contains(&track),
        }))
    }

    // ─── Devices ────────────────────────────────────────────────────────────────────────

    pub(super) fn tool_add_device(
        &mut self,
        a: Args,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> ToolResult {
        let track: TrackId = a.req_id("track_id", "track")?;
        let ty = builtin_type(a.req_str("type")?)?;
        let p = self.agent_project()?;
        if !p.tracks.contains_key(&track) {
            return Err(ToolError::Command(not_found(format!("track {track}"))));
        }
        let before = a
            .u64("index")
            .and_then(|i| p.devices_of(track).get(i as usize).map(|d| d.id));
        let id: DeviceId = self.agent_id(now);
        self.agent_batch(
            &label("Add Device"),
            &[Command::Device(DeviceCommand::Insert {
                id,
                track,
                device: DeviceSpec::Builtin {
                    device: BuiltinDevice::new(ty),
                },
                before,
            })],
            now,
            out,
        )?;
        let p = self.agent_project()?;
        let position = p.devices_of(track).iter().position(|d| d.id == id);
        Ok(json!({
            "device_id": id.to_string(),
            "name": p.devices.get(&id).map(|d| d.name.clone()),
            "type": ty.key(),
            "category": category_name(ether_devices::descriptor(ty).category),
            "index": position,
        }))
    }

    pub(super) fn tool_remove_device(
        &mut self,
        a: Args,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> ToolResult {
        let id: DeviceId = a.req_id("device_id", "device")?;
        let name = self
            .agent_project()?
            .devices
            .get(&id)
            .map(|d| d.name.clone())
            .ok_or_else(|| ToolError::Command(not_found(format!("device {id}"))))?;
        self.agent_batch(
            &label("Remove Device"),
            &[Command::Device(DeviceCommand::Remove { id })],
            now,
            out,
        )?;
        Ok(json!({ "device_id": id.to_string(), "removed": true, "name": name }))
    }

    pub(super) fn tool_set_device_param(
        &mut self,
        a: Args,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> ToolResult {
        let device: DeviceId = a.req_id("device_id", "device")?;
        if !self.agent_project()?.devices.contains_key(&device) {
            return Err(ToolError::Command(not_found(format!("device {device}"))));
        }
        let desc = self.agent_descriptor(device).ok_or_else(|| {
            ToolError::Command(not_found(format!(
                "parameters of device {device} (plugin not loaded)"
            )))
        })?;
        let param = a.get("param").expect("required");
        let info = match param {
            Value::String(name) => {
                let wanted = name.trim().to_lowercase();
                desc.params
                    .iter()
                    .find(|p| p.name.to_lowercase() == wanted)
                    .or_else(|| {
                        // "Filter Cutoff" = group + name; or a numeric id passed as text.
                        desc.params.iter().find(|p| {
                            p.group.as_ref().is_some_and(|g| {
                                format!("{} {}", g.to_lowercase(), p.name.to_lowercase()) == wanted
                            }) || wanted.parse::<u32>().is_ok_and(|n| p.id.0 == n)
                        })
                    })
            }
            v => v
                .as_u64()
                .and_then(|n| desc.params.iter().find(|p| u64::from(p.id.0) == n)),
        }
        .cloned()
        .ok_or_else(|| {
            ToolError::input(format!(
                "device {device} has no parameter {param}; see get_device_params"
            ))
        })?;
        let raw = a.get("value").expect("required");
        let plain = match raw {
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
                    let i = labels.iter().position(|l| l.to_lowercase() == wanted)?;
                    let n = labels.len().max(2) - 1;
                    Some(info.min + (info.max - info.min) * i as f64 / n as f64)
                });
                match (by_label, wanted.parse::<f64>()) {
                    (Some(v), _) => v,
                    (None, Ok(v)) => v,
                    (None, Err(_)) => {
                        return Err(ToolError::input(match &info.labels {
                            Some(l) => format!(
                                "`{s}` is not a choice of {}: use one of {}",
                                info.name,
                                l.join(", ")
                            ),
                            None => format!("{} needs a number", info.name),
                        }));
                    }
                }
            }
            _ => return Err(ToolError::input("value must be a number, label or boolean")),
        };
        if !plain.is_finite() {
            return Err(ToolError::input("value must be finite"));
        }
        let value = info.snap(plain);
        self.agent_batch(
            &label(&format!("Set {}", info.name)),
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
            "value": r3(stored),
            "unit": format!("{:?}", info.unit).to_lowercase(),
        });
        if let Some(labels) = &info.labels {
            let n = labels.len().max(2) - 1;
            let span = info.max - info.min;
            if span > 0.0 {
                let i = (((stored - info.min) / span) * n as f64).round() as usize;
                if let Some(l) = labels.get(i) {
                    v["label"] = json!(l);
                }
            }
        }
        if (plain - value).abs() > 1e-9 {
            v["note"] = json!(format!(
                "requested {plain} was clamped/snapped to the range {}..{}",
                info.min, info.max
            ));
        }
        Ok(v)
    }

    // ─── Clips & notes ──────────────────────────────────────────────────────────────────

    pub(super) fn tool_create_midi_clip(
        &mut self,
        a: Args,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> ToolResult {
        let track: TrackId = a.req_id("track_id", "track")?;
        if self.track_of(track)?.kind != TrackKind::Midi {
            return Err(ToolError::input(format!(
                "track {track} is not a MIDI track"
            )));
        }
        let id: ClipId = self.agent_id(now);
        let mut commands = vec![Command::Clip(ClipCommand::CreateMidi {
            id,
            track,
            start: Beats(a.req_f64("start_beats")?),
            length: Beats(a.req_f64("length_beats")?),
            name: a.str("name").map(str::to_string),
        })];
        let notes = match a.array("notes") {
            Some(n) if !n.is_empty() => self.note_specs(n, now)?,
            _ => Vec::new(),
        };
        let note_ids: Vec<String> = notes.iter().map(|n| n.id.to_string()).collect();
        if !notes.is_empty() {
            commands.push(Command::Note(NoteCommand::Add { clip: id, notes }));
        }
        self.agent_batch(&label("Create MIDI Clip"), &commands, now, out)?;
        let c = self.clip_of(id)?;
        Ok(json!({
            "clip_id": id.to_string(),
            "name": c.name,
            "start_beats": r3(c.start.0),
            "length_beats": r3(c.length.0),
            "note_ids": note_ids,
        }))
    }

    pub(super) fn tool_add_notes(
        &mut self,
        a: Args,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> ToolResult {
        let clip: ClipId = a.req_id("clip_id", "clip")?;
        if !matches!(self.clip_of(clip)?.content, ClipContent::Midi) {
            return Err(ToolError::input(format!(
                "clip {clip} is an audio clip; notes go into MIDI clips"
            )));
        }
        let notes = self.note_specs(a.array("notes").map_or(&[][..], Vec::as_slice), now)?;
        let ids: Vec<String> = notes.iter().map(|n| n.id.to_string()).collect();
        let clip_len = self.clip_of(clip)?.length.0;
        let past_end = notes
            .iter()
            .filter(|n| n.start.0 >= clip_len - Beats::EPSILON)
            .count();
        self.agent_batch(
            &label("Add Notes"),
            &[Command::Note(NoteCommand::Add { clip, notes })],
            now,
            out,
        )?;
        let mut v = json!({ "clip_id": clip.to_string(), "added": ids.len(), "note_ids": ids });
        if past_end > 0 {
            v["note"] = json!(format!(
                "{past_end} note(s) start at or after the clip end ({}) and are not heard; lengthen the clip with set_clip",
                r3(clip_len)
            ));
        }
        Ok(v)
    }

    pub(super) fn tool_remove_notes(
        &mut self,
        a: Args,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> ToolResult {
        let ids: Vec<NoteId> = if a.has("note_ids") {
            let ids: Vec<NoteId> = a.ids("note_ids", "note")?;
            let p = self.agent_project()?;
            if let Some(missing) = ids.iter().find(|n| !p.notes.contains_key(n)) {
                return Err(ToolError::Command(not_found(format!("note {missing}"))));
            }
            ids
        } else if let Some(clip) = a.id::<ClipId>("clip_id", "clip")? {
            self.clip_of(clip)?;
            let pitch = a.u64("pitch");
            let from = a.f64("from_beats").unwrap_or(f64::NEG_INFINITY);
            let to = a.f64("to_beats").unwrap_or(f64::INFINITY);
            self.agent_project()?
                .notes_of(clip)
                .into_iter()
                .filter(|n| pitch.is_none_or(|p| u64::from(n.pitch) == p))
                .filter(|n| n.start.0 >= from - Beats::EPSILON && n.start.0 < to - Beats::EPSILON)
                .map(|n| n.id)
                .collect()
        } else {
            return Err(ToolError::input(
                "pass `note_ids`, or `clip_id` (with optional filters)",
            ));
        };
        if ids.is_empty() {
            return Ok(json!({ "removed": 0 }));
        }
        let n = ids.len();
        self.agent_batch(
            &label("Remove Notes"),
            &[Command::Note(NoteCommand::Remove { ids })],
            now,
            out,
        )?;
        Ok(json!({ "removed": n }))
    }

    pub(super) fn tool_set_clip(
        &mut self,
        a: Args,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> ToolResult {
        let id: ClipId = a.req_id("clip_id", "clip")?;
        let c = self.clip_of(id)?.clone();
        let mut commands = Vec::new();
        let start = a.f64("start_beats").map(Beats).unwrap_or(c.start);
        let track: TrackId = a.id("track_id", "track")?.unwrap_or(c.track);
        if a.has("start_beats") || a.has("track_id") {
            commands.push(Command::Clip(ClipCommand::Move {
                moves: vec![ClipMove { id, track, start }],
            }));
        }
        if let Some(length) = a.f64("length_beats") {
            commands.push(Command::Clip(ClipCommand::SetBounds {
                id,
                start,
                length: Beats(length),
                offset: c.offset,
            }));
        }
        if let Some(name) = a.str("name") {
            commands.push(Command::Clip(ClipCommand::Rename {
                id,
                name: name.to_string(),
            }));
        }
        if let Some(muted) = a.bool("muted") {
            commands.push(Command::Clip(ClipCommand::SetMuted {
                ids: vec![id],
                muted,
            }));
        }
        if a.has("loop_enabled") || a.has("loop_start_beats") || a.has("loop_end_beats") {
            let enabled = a.bool("loop_enabled").unwrap_or(c.looping.enabled);
            let mut ls = a.f64("loop_start_beats").unwrap_or(c.looping.start.0);
            let mut le = a.f64("loop_end_beats").unwrap_or(c.looping.end.0);
            if enabled && le <= ls + Beats::EPSILON && !a.has("loop_end_beats") {
                // No region yet: loop what the clip shows.
                ls = c.offset.0;
                le = c.offset.0 + a.f64("length_beats").unwrap_or(c.length.0);
            }
            if le <= ls + Beats::EPSILON {
                return Err(ToolError::input(
                    "loop_end_beats must be after loop_start_beats",
                ));
            }
            commands.push(Command::Clip(ClipCommand::SetLoop {
                id,
                looping: ClipLoop {
                    enabled,
                    start: Beats(ls),
                    end: Beats(le),
                },
            }));
        }
        if commands.is_empty() {
            return Err(ToolError::input(
                "nothing to change: pass start_beats, track_id, length_beats, name, muted or loop_*",
            ));
        }
        self.agent_batch(&label("Edit Clip"), &commands, now, out)?;
        let p = self.agent_project()?;
        match p.clips.get(&id) {
            Some(c) => Ok(json!({
                "clip_id": id.to_string(),
                "track_id": c.track.to_string(),
                "name": c.name,
                "start_beats": r3(c.start.0),
                "length_beats": r3(c.length.0),
                "muted": c.muted,
                "loop": { "enabled": c.looping.enabled, "start_beats": r3(c.looping.start.0), "end_beats": r3(c.looping.end.0) },
            })),
            None => Ok(
                json!({ "clip_id": id.to_string(), "note": "the clip was removed by the move (fully covered by another clip)" }),
            ),
        }
    }

    pub(super) fn tool_delete_clip(
        &mut self,
        a: Args,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> ToolResult {
        let mut ids: Vec<ClipId> = a.ids("clip_ids", "clip")?;
        if let Some(c) = a.id::<ClipId>("clip_id", "clip")? {
            ids.push(c);
        }
        ids.sort();
        ids.dedup();
        if ids.is_empty() {
            return Err(ToolError::input("pass `clip_id` or `clip_ids`"));
        }
        for c in &ids {
            self.clip_of(*c)?;
        }
        let n = ids.len();
        self.agent_batch(
            &label("Delete Clip"),
            &[Command::Clip(ClipCommand::Delete { ids })],
            now,
            out,
        )?;
        Ok(json!({ "deleted": n }))
    }

    pub(super) fn tool_duplicate_clip(
        &mut self,
        a: Args,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> ToolResult {
        let id: ClipId = a.req_id("clip_id", "clip")?;
        self.clip_of(id)?;
        let new_id: ClipId = self.agent_id(now);
        self.agent_batch(
            &label("Duplicate Clip"),
            &[Command::Clip(ClipCommand::Duplicate {
                id,
                new_id,
                start: a.f64("start_beats").map(Beats),
            })],
            now,
            out,
        )?;
        let c = self.clip_of(new_id)?;
        Ok(json!({
            "clip_id": new_id.to_string(),
            "track_id": c.track.to_string(),
            "start_beats": r3(c.start.0),
            "length_beats": r3(c.length.0),
        }))
    }

    // ─── Song ───────────────────────────────────────────────────────────────────────────

    pub(super) fn tool_set_tempo(
        &mut self,
        a: Args,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> ToolResult {
        let bpm = a.req_f64("bpm")?;
        self.agent_batch(
            &label("Set Tempo"),
            &[Command::Transport(TransportCommand::SetTempo { bpm })],
            now,
            out,
        )?;
        let p = self.agent_project()?;
        Ok(json!({ "tempo_bpm": r3(p.tempo_map().bpm_at(self.transport.position)) }))
    }

    pub(super) fn tool_set_time_signature(
        &mut self,
        a: Args,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> ToolResult {
        let signature = TimeSignature {
            numerator: a.req_f64("numerator")? as u8,
            denominator: a.req_f64("denominator")? as u8,
        };
        self.agent_batch(
            &label("Set Time Signature"),
            &[Command::Transport(TransportCommand::SetTimeSignature {
                signature,
            })],
            now,
            out,
        )?;
        let s = self
            .agent_project()?
            .tempo_map()
            .signature_at(self.transport.position);
        Ok(json!({ "time_signature": format!("{}/{}", s.numerator, s.denominator) }))
    }

    fn transport_json(&self) -> Value {
        let mut v = json!({
            "playing": self.transport.playing,
            "playhead_beats": r3(self.transport.position.0),
        });
        if let Some(p) = self.doc.as_ref().map(|d| &d.project) {
            v["loop"] = json!({
                "enabled": p.settings.loop_enabled,
                "start_beats": r3(p.settings.loop_region.start.0),
                "end_beats": r3(p.settings.loop_region.end.0),
            });
        }
        v
    }

    pub(super) fn tool_transport(
        &mut self,
        a: Args,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> ToolResult {
        self.agent_project()?;
        let t = |c| Command::Transport(c);
        match a.req_str("action")? {
            "play" => {
                self.agent_dispatch(t(TransportCommand::Play), None, now, out)?;
            }
            "stop" => {
                if self.transport.playing {
                    self.agent_dispatch(t(TransportCommand::Stop), None, now, out)?;
                }
            }
            "seek" => {
                let position = a.f64("position_beats").ok_or_else(|| {
                    ToolError::input("seek needs `position_beats` (beats from the song start)")
                })?;
                self.agent_dispatch(
                    t(TransportCommand::Locate {
                        position: Beats(position),
                    }),
                    None,
                    now,
                    out,
                )?;
            }
            _ => {
                let mut commands = Vec::new();
                if a.has("loop_start_beats") || a.has("loop_end_beats") {
                    let current = self.agent_project()?.settings.loop_region;
                    let region = BeatRange {
                        start: a
                            .f64("loop_start_beats")
                            .map(Beats)
                            .unwrap_or(current.start),
                        end: a.f64("loop_end_beats").map(Beats).unwrap_or(current.end),
                    };
                    if region.end.0 <= region.start.0 + Beats::EPSILON {
                        return Err(ToolError::input(
                            "loop_end_beats must be after loop_start_beats",
                        ));
                    }
                    commands.push(t(TransportCommand::SetLoopRegion { region }));
                }
                if let Some(enabled) = a.bool("loop_enabled") {
                    commands.push(t(TransportCommand::SetLoopEnabled { enabled }));
                }
                if commands.is_empty() {
                    return Err(ToolError::input(
                        "loop needs loop_enabled and/or loop_start_beats/loop_end_beats",
                    ));
                }
                self.agent_batch(&label("Set Loop"), &commands, now, out)?;
            }
        }
        Ok(self.transport_json())
    }

    pub(super) fn tool_undo_redo(
        &mut self,
        undo: bool,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> ToolResult {
        self.agent_project()?;
        let label = self.doc.as_ref().and_then(|d| {
            let h = d.history.state();
            if undo { h.undo_label } else { h.redo_label }
        });
        let command = if undo {
            EditCommand::Undo
        } else {
            EditCommand::Redo
        };
        self.agent_dispatch(Command::Edit(command), None, now, out)?;
        let mut v = json!({ "history": self.agent_history() });
        v[if undo { "undone" } else { "redone" }] = json!(label);
        Ok(v)
    }

    pub(super) fn tool_save(&mut self, now: u64, out: &mut dyn MessageSink) -> ToolResult {
        self.agent_project()?;
        match self.agent_dispatch(Command::Project(ProjectCommand::Save), None, now, out)? {
            ReplyValue::Saved { project } => Ok(json!({
                "saved": project.name,
                "project_id": project.id.to_string(),
            })),
            _ => Ok(json!({ "saved": true })),
        }
    }

    // ─── Library ────────────────────────────────────────────────────────────────────────

    fn find_browser_item(
        &mut self,
        item: &str,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> Result<LibraryItem, ToolError> {
        let (root, path) = item.split_once('/').ok_or_else(|| {
            ToolError::input(format!(
                "`{item}` is not a library item id (use search_browser)"
            ))
        })?;
        let query = BrowserQuery {
            text: String::new(),
            kinds: Vec::new(),
            tags: Vec::new(),
            favourites_only: false,
            roots: vec![root.to_string()],
            folder: Some(path.to_string()),
            device: None,
            sort: BrowserSort::Name,
            offset: 0,
            limit: 200,
        };
        match self.agent_dispatch(
            Command::Browser(BrowserCommand::Query { query }),
            None,
            now,
            out,
        )? {
            ReplyValue::BrowserPage { page } => page
                .items
                .into_iter()
                .find(|i| i.id == item)
                .ok_or_else(|| ToolError::Command(not_found(format!("library item `{item}`")))),
            _ => Err(ToolError::Command(internal("unexpected browser reply"))),
        }
    }

    pub(super) fn tool_load_browser_item(
        &mut self,
        a: Args,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> ToolResult {
        self.agent_project()?;
        let item = self.find_browser_item(a.req_str("item_id")?, now, out)?;
        if let Some(preset) = item.preset.clone() {
            if let Some(device) = a.id::<DeviceId>("device_id", "device")? {
                // Rack presets that store chains mint their ids from the seed (collab replays).
                let seed = self.agent_id(now);
                self.agent_steps(
                    vec![Command::Preset(PresetCommand::Load {
                        device,
                        preset,
                        seed: Some(seed),
                    })],
                    now,
                    out,
                )?;
                return Ok(json!({ "device_id": device.to_string(), "preset": item.name }));
            }
            let track: TrackId = a.id("track_id", "track")?.ok_or_else(|| {
                ToolError::input(
                    "presets need `device_id` (apply to it) or `track_id` (add a new device)",
                )
            })?;
            let list = self.agent_dispatch(
                Command::Preset(PresetCommand::List {
                    device: None,
                    text: None,
                }),
                None,
                now,
                out,
            )?;
            let ReplyValue::Presets { presets } = list else {
                return Err(ToolError::Command(internal("unexpected preset reply")));
            };
            let info = presets
                .into_iter()
                .find(|p| p.preset == preset)
                .ok_or_else(|| ToolError::Command(not_found(format!("preset `{}`", item.name))))?;
            let PresetDevice::Builtin { device: ty } = info.device else {
                return Err(ToolError::input(
                    "plugin presets can only be applied to an existing plugin device (`device_id`)",
                ));
            };
            let device: DeviceId = self.agent_id(now);
            let seed = self.agent_id(now);
            self.agent_steps(
                vec![
                    Command::Device(DeviceCommand::Insert {
                        id: device,
                        track,
                        device: DeviceSpec::Builtin {
                            device: BuiltinDevice::new(ty),
                        },
                        before: None,
                    }),
                    Command::Preset(PresetCommand::Load {
                        device,
                        preset,
                        seed: Some(seed),
                    }),
                ],
                now,
                out,
            )?;
            return Ok(
                json!({ "device_id": device.to_string(), "preset": item.name, "type": ty.key() }),
            );
        }
        let Some(source) = item.source.clone() else {
            return Err(ToolError::input(format!(
                "`{}` cannot be loaded by this tool",
                item.name
            )));
        };
        if !matches!(
            item.kind,
            ether_core::protocol::browser::LibraryItemKind::Audio
        ) {
            return Err(ToolError::input(format!(
                "`{}` is not an audio sample; only audio and presets can be loaded",
                item.name
            )));
        }
        let track: TrackId = a
            .id("track_id", "track")?
            .ok_or_else(|| ToolError::input("audio items need `track_id` (an audio track)"))?;
        if self.track_of(track)?.kind != TrackKind::Audio {
            return Err(ToolError::input(format!(
                "track {track} is not an audio track"
            )));
        }
        let media: MediaId = self.agent_id(now);
        let clip: ClipId = self.agent_id(now);
        self.agent_steps(
            vec![
                Command::Media(MediaCommand::Import { id: media, source }),
                Command::Clip(ClipCommand::CreateAudio {
                    id: clip,
                    track,
                    start: Beats(a.f64("start_beats").unwrap_or(0.0)),
                    media,
                }),
            ],
            now,
            out,
        )?;
        let c = self.clip_of(clip)?;
        Ok(json!({
            "clip_id": clip.to_string(),
            "media_id": media.to_string(),
            "start_beats": r3(c.start.0),
            "length_beats": r3(c.length.0),
        }))
    }

    // ─── Export ─────────────────────────────────────────────────────────────────────────

    pub(super) fn tool_export(
        &mut self,
        a: Args,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> ToolResult {
        self.agent_project()?;
        let range = match a.str("range").unwrap_or("project") {
            "loop" => ExportRange::Loop,
            "custom" => {
                let (Some(start), Some(end)) = (a.f64("start_beats"), a.f64("end_beats")) else {
                    return Err(ToolError::input(
                        "a custom range needs `start_beats` and `end_beats`",
                    ));
                };
                if end <= start {
                    return Err(ToolError::input("`end_beats` must be after `start_beats`"));
                }
                ExportRange::Custom {
                    start: Beats(start),
                    end: Beats(end),
                }
            }
            _ => ExportRange::Project,
        };
        let container = match a.str("format") {
            Some("flac") => AudioContainer::Flac,
            _ => AudioContainer::Wav,
        };
        let bit_depth = match a.u64("bit_depth") {
            Some(16) => BitDepth::Int16,
            Some(32) => BitDepth::Float32,
            _ => BitDepth::Int24,
        };
        let stems: Vec<TrackId> = a.ids("stem_track_ids", "track")?;
        let mode = if stems.is_empty() {
            ExportMode::Mix
        } else {
            ExportMode::Stems { tracks: stems }
        };
        let job = self.ids.next_ulid(now).to_string();
        let request = ExportRequest {
            range,
            format: ExportFormat {
                container,
                bit_depth,
                sample_rate: None,
            },
            mode,
            normalize: a.bool("normalize").unwrap_or(false),
            tail_seconds: a.f64("tail_seconds").unwrap_or(2.0),
            name: a.str("name").map(str::to_string),
        };
        self.agent_dispatch(
            Command::Export(ExportCommand::Render {
                job: job.clone(),
                request,
            }),
            None,
            now,
            out,
        )?;
        // Registered as running (the tap may update it when this call's events arrive).
        self.agent.export_mut(&job);
        Ok(json!({
            "job_id": job,
            "status": "started",
            "note": "the render runs in the background: poll get_export_status",
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_type_keys() {
        assert_eq!(
            builtin_type("poly-synth").unwrap(),
            BuiltinDeviceType::PolySynth
        );
        assert_eq!(
            builtin_type("Poly Synth").unwrap(),
            BuiltinDeviceType::PolySynth
        );
        assert_eq!(
            builtin_type("drum_rack").unwrap(),
            BuiltinDeviceType::DrumRack
        );
        assert!(matches!(builtin_type("kazoo"), Err(ToolError::Input(_))));
        assert_eq!(parse_color("#FF8800").unwrap(), Color(0xff8800));
        assert!(parse_color("red").is_err());
    }
}

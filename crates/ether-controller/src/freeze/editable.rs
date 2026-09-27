//! [`check_editable`]: a frozen track's clips, devices and device automation can't be edited
//! (CONTRACTS.md §12.3); its mixer (fader, pan, mute/solo, sends, routing), name, color and
//! position stay editable, and so does the track itself (delete, duplicate, move).
//!
//! The command's entity ids are collected from its serialized form (every ULID string, with
//! its field name) and resolved against the project, so every present and future content
//! command of the listed domains is covered without a per-variant table:
//! - clips, notes, warp markers, take lanes and comp regions of a frozen track;
//! - devices on a frozen track (and what hangs off them: drum pads, rack chains, modulators,
//!   modulation mappings), and automation lanes/points of a device on a frozen track or of a
//!   frozen track's clip (clip envelopes). Mixer lanes (volume, pan, sends) stay editable;
//! - a frozen track named by a `track` field (creating or moving clips/devices onto it,
//!   arming it, comping it) or listed in a time edit's `tracks` (an empty list means every
//!   track, so it includes the frozen ones).

use ether_core::protocol::Command;
use ether_core::protocol::model::*;
use serde_json::Value;

use crate::tx::{CmdResult, invalid_state};

/// How a command's `track` fields count.
#[derive(Clone, Copy, PartialEq)]
enum Tracks {
    /// Track ids never block (mixer-side references, e.g. a sidechain source).
    Ignore,
    /// A `track` field naming a frozen track blocks.
    Field,
    /// `tracks` lists (empty = all) and `track` fields block.
    Selection,
}

/// Domains whose commands edit track content, and how their track ids count.
fn domain(command: &Command) -> Option<Tracks> {
    use ether_core::protocol::devices::DeviceCommand as D;
    use ether_core::protocol::groove::GrooveCommand as G;
    use ether_core::protocol::presets::PresetCommand as P;
    use ether_core::protocol::recording::RecordingCommand as R;
    use ether_core::protocol::warp::WarpCommand as W;
    Some(match command {
        Command::Clip(_) | Command::Take(_) => Tracks::Field,
        Command::Device(c) => match c {
            D::ListBuiltin | D::GetDescriptor { .. } => return None,
            D::Insert { .. } | D::Move { .. } => Tracks::Field,
            _ => Tracks::Ignore,
        },
        Command::Note(_)
        | Command::Automation(_)
        | Command::DrumRack(_)
        | Command::Slice(_)
        | Command::Rack(_)
        | Command::Plugin(_) => Tracks::Ignore,
        Command::Modulation(_) => Tracks::Ignore,
        Command::Warp(c) => match c {
            W::DetectTempo { .. } => return None,
            _ => Tracks::Ignore,
        },
        Command::Groove(c) => match c {
            G::Humanize { .. } => Tracks::Ignore,
            _ => return None,
        },
        Command::Preset(c) => match c {
            P::Load { .. } => Tracks::Ignore,
            _ => return None,
        },
        Command::Recording(c) => match c {
            R::Arm { .. } => Tracks::Field,
            _ => return None,
        },
        Command::TimeEdit(_) => Tracks::Selection,
        _ => return None,
    })
}

/// Every `(field name, ULID)` in `v` (array items inherit their array's field name).
fn collect_ids(v: &Value, key: Option<&str>, out: &mut Vec<(String, Ulid)>) {
    match v {
        Value::String(s) if s.len() == 26 => {
            if let Ok(id) = s.parse::<ClipId>() {
                out.push((key.unwrap_or_default().to_string(), id.ulid()));
            }
        }
        Value::Array(items) => {
            for i in items {
                collect_ids(i, key, out);
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                collect_ids(v, Some(k), out);
            }
        }
        _ => {}
    }
}

fn frozen(p: &Project, track: TrackId) -> Option<&Track> {
    p.tracks.get(&track).filter(|t| t.freeze.is_some())
}

/// The frozen track that entity `id` belongs to (content of a frozen track), if any.
fn frozen_owner(p: &Project, id: Ulid) -> Option<&Track> {
    let clip_track = |c: ClipId| p.clips.get(&c).map(|c| c.track);
    let device_track = |d: DeviceId| p.devices.get(&d).map(|d| d.track);
    let lane_track = |l: AutomationLaneId| {
        let lane = p.automation_lanes.get(&l)?;
        match (&lane.owner, &lane.target) {
            (AutomationOwner::Clip { clip }, _) => clip_track(*clip),
            (_, AutomationTarget::DeviceParam { device, .. }) => device_track(*device),
            _ => None,
        }
    };
    let track = clip_track(ClipId(id))
        .or_else(|| device_track(DeviceId(id)))
        .or_else(|| p.notes.get(&NoteId(id)).and_then(|n| clip_track(n.clip)))
        .or_else(|| lane_track(AutomationLaneId(id)))
        .or_else(|| {
            p.automation_points
                .get(&AutomationPointId(id))
                .and_then(|pt| lane_track(pt.lane))
        })
        .or_else(|| {
            p.warp_markers
                .get(&WarpMarkerId(id))
                .and_then(|m| clip_track(m.clip))
        })
        .or_else(|| {
            p.drum_pads
                .get(&DrumPadId(id))
                .and_then(|d| device_track(d.rack))
        })
        .or_else(|| {
            p.rack_chains
                .get(&RackChainId(id))
                .and_then(|c| device_track(c.rack))
        })
        .or_else(|| {
            p.modulators
                .get(&ModulatorId(id))
                .and_then(|m| device_track(m.device))
        })
        .or_else(|| {
            p.mod_mappings
                .get(&ModMappingId(id))
                .and_then(|m| device_track(m.device))
        })
        .or_else(|| p.take_lanes.get(&TakeLaneId(id)).map(|l| l.track))
        .or_else(|| p.comp_regions.get(&CompRegionId(id)).map(|r| r.track))?;
    frozen(p, track)
}

fn rejected(t: &Track) -> ether_core::protocol::CommandError {
    invalid_state(format!(
        "track \"{}\" is frozen: unfreeze it to edit its clips and devices",
        t.name
    ))
}

/// Reject document commands that edit a frozen track's content (see the module docs).
pub(crate) fn check_editable(p: &Project, command: &Command) -> CmdResult<()> {
    if !p.tracks.values().any(|t| t.freeze.is_some()) {
        return Ok(());
    }
    let Some(tracks) = domain(command) else {
        return Ok(());
    };
    let Ok(json) = serde_json::to_value(command) else {
        return Ok(());
    };
    let mut ids = Vec::new();
    collect_ids(&json, None, &mut ids);
    for (key, id) in &ids {
        if let Some(t) = p.tracks.get(&TrackId(*id)) {
            let blocks = match tracks {
                Tracks::Ignore => false,
                Tracks::Field => key == "track",
                Tracks::Selection => key == "track" || key == "tracks",
            };
            if blocks && t.freeze.is_some() {
                return Err(rejected(t));
            }
            continue;
        }
        if let Some(t) = frozen_owner(p, *id) {
            return Err(rejected(t));
        }
    }
    // A time edit over every track includes the frozen ones.
    if tracks == Tracks::Selection && selection_is_all(&json) {
        let mut all: Vec<&Track> = p.tracks.values().filter(|t| t.freeze.is_some()).collect();
        all.sort_by(|a, b| a.order.cmp(&b.order));
        if let Some(t) = all.first() {
            return Err(rejected(t));
        }
    }
    Ok(())
}

/// Some `tracks` field of the command is an empty list (= every track).
fn selection_is_all(v: &Value) -> bool {
    match v {
        Value::Object(map) => map.iter().any(|(k, v)| {
            (k == "tracks" && v.as_array().is_some_and(Vec::is_empty)) || selection_is_all(v)
        }),
        Value::Array(items) => items.iter().any(selection_is_all),
        _ => false,
    }
}

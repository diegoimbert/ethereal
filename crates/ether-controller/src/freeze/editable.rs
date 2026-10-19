//! [`check_editable`]: a frozen track's clips, devices and device automation can't be edited
//! (CONTRACTS.md §12.3); its mixer (fader, pan, mute/solo, sends, routing), name, color and
//! position stay editable, and so does the track itself (delete, duplicate, move).
//!
//! The command's entity ids are collected from its serialized form (every ULID string, with
//! its field name) and resolved against the project, so every present and future content
//! command of the listed domains is covered without a per-variant table:
//! - clips, notes, warp markers, take lanes and comp regions of a frozen track;
//! - frozen devices (`Project::device_frozen`: every device of a frozen track, or those up to
//!   the freeze point of a partial freeze) and what hangs off them (drum pads, rack chains,
//!   modulators, modulation mappings), and automation lanes/points of a frozen device or of
//!   a frozen track's clip (clip envelopes). Mixer lanes (volume, pan, sends) stay editable,
//!   and so does a partial freeze's live tail (inserting or moving devices into it too);
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
        Command::Groove(G::Humanize { .. }) => Tracks::Ignore,
        Command::Preset(P::Load { .. }) => Tracks::Ignore,
        Command::Recording(R::Arm { .. }) => Tracks::Field,
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

/// The frozen track covering `track`: itself, or the frozen group it sits in (group freeze).
fn frozen(p: &Project, track: TrackId) -> Option<&Track> {
    p.frozen_by(track).and_then(|t| p.tracks.get(&t))
}

/// What a command's entity hangs off: track content (clips and what is on them, take lanes,
/// comp regions) or a device (what is on it: pads, rack chains, modulators, mappings, its
/// automation lanes).
#[derive(Clone, Copy)]
enum Owner {
    Track(TrackId),
    Device(DeviceId),
}

/// The frozen track that entity `id` belongs to (content of a frozen track, or a frozen
/// device: a partial freeze's live tail stays editable), if any.
fn frozen_owner(p: &Project, id: Ulid) -> Option<&Track> {
    let clip_track = |c: ClipId| p.clips.get(&c).map(|c| Owner::Track(c.track));
    let device = |d: DeviceId| p.devices.get(&d).map(|d| Owner::Device(d.id));
    let lane_owner = |l: AutomationLaneId| {
        let lane = p.automation_lanes.get(&l)?;
        match (&lane.owner, &lane.target) {
            (AutomationOwner::Clip { clip }, _) => clip_track(*clip),
            (
                _,
                AutomationTarget::DeviceParam { device: d, .. }
                | AutomationTarget::DeviceEnabled { device: d },
            ) => device(*d),
            _ => None,
        }
    };
    let owner = clip_track(ClipId(id))
        .or_else(|| device(DeviceId(id)))
        .or_else(|| p.notes.get(&NoteId(id)).and_then(|n| clip_track(n.clip)))
        .or_else(|| lane_owner(AutomationLaneId(id)))
        .or_else(|| {
            p.automation_points
                .get(&AutomationPointId(id))
                .and_then(|pt| lane_owner(pt.lane))
        })
        .or_else(|| {
            p.warp_markers
                .get(&WarpMarkerId(id))
                .and_then(|m| clip_track(m.clip))
        })
        .or_else(|| p.drum_pads.get(&DrumPadId(id)).and_then(|d| device(d.rack)))
        .or_else(|| {
            p.rack_chains
                .get(&RackChainId(id))
                .and_then(|c| device(c.rack))
        })
        .or_else(|| {
            p.modulators
                .get(&ModulatorId(id))
                .and_then(|m| device(m.device))
        })
        .or_else(|| {
            p.mod_mappings
                .get(&ModMappingId(id))
                .and_then(|m| device(m.device))
        })
        .or_else(|| {
            p.take_lanes
                .get(&TakeLaneId(id))
                .map(|l| Owner::Track(l.track))
        })
        .or_else(|| {
            p.comp_regions
                .get(&CompRegionId(id))
                .map(|r| Owner::Track(r.track))
        })?;
    match owner {
        Owner::Track(track) => frozen(p, track),
        Owner::Device(d) => {
            if !p.device_frozen(d) {
                return None;
            }
            frozen(p, p.devices.get(&d)?.track)
        }
    }
}

/// A `Device::{Insert, Move}` onto a partially frozen track lands in its live tail: always,
/// since its `before` (when set) is checked as a device on its own (a frozen one blocks).
fn into_live_tail(p: &Project, command: &Command, track: TrackId) -> bool {
    use ether_core::protocol::devices::DeviceCommand as D;
    let Command::Device(D::Insert { track: t, .. } | D::Move { track: t, .. }) = command else {
        return false;
    };
    let Some(up_to) = p
        .tracks
        .get(&track)
        .and_then(|t| t.freeze.as_ref())
        .and_then(|f| f.up_to)
    else {
        return false;
    };
    *t == track
        && p.frozen_ancestor(track).is_none()
        && p.devices_of(track).iter().any(|d| d.id == up_to)
}

fn rejected(t: &Track) -> ether_core::protocol::CommandError {
    crate::tx::keyed_err(
        ether_core::protocol::ErrorCode::InvalidState,
        crate::text::msg("error.freeze.track_frozen").arg("track", t.name.as_str()),
    )
}

/// Group freeze: the tracks inside a frozen group are locked (moved, deleted, duplicated,
/// regrouped) and nothing can be created or moved into one.
fn check_structure(p: &Project, command: &Command) -> CmdResult<()> {
    use ether_core::protocol::tracks::TrackCommand as T;
    let Command::Track(c) = command else {
        return Ok(());
    };
    let locked = |id: &TrackId| -> CmdResult<()> {
        match p.frozen_ancestor(*id).and_then(|g| p.tracks.get(&g)) {
            Some(g) => Err(invalid_state(format!(
                "track is inside frozen group \"{}\": unfreeze the group first",
                g.name
            ))),
            None => Ok(()),
        }
    };
    let into = |parent: &Option<TrackId>| -> CmdResult<()> {
        match parent.and_then(|g| frozen(p, g)) {
            Some(g) => Err(invalid_state(format!(
                "group \"{}\" is frozen: unfreeze it first",
                g.name
            ))),
            None => Ok(()),
        }
    };
    match c {
        T::Create { parent, .. } => into(parent),
        T::Move { id, parent, .. } => {
            locked(id)?;
            into(parent)
        }
        T::Delete { id } | T::Duplicate { id, .. } | T::Ungroup { group: id, .. } => locked(id),
        T::GroupSelected { ids, .. } => ids.iter().try_for_each(locked),
        _ => Ok(()),
    }
}

/// Reject document commands that edit a frozen track's content (see the module docs).
pub fn check_editable(p: &Project, command: &Command) -> CmdResult<()> {
    if !p.tracks.values().any(|t| t.freeze.is_some()) {
        return Ok(());
    }
    check_structure(p, command)?;
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
            if blocks
                && let Some(f) = frozen(p, t.id)
                && !into_live_tail(p, command, t.id)
            {
                return Err(rejected(f));
            }
            continue;
        }
        if let Some(t) = frozen_owner(p, *id) {
            return Err(rejected(t));
        }
    }
    // A time edit over every track includes the frozen ones.
    if tracks == Tracks::Selection && selection_is_all(&json) {
        let mut all: Vec<&Track> = p
            .tracks
            .values()
            .filter(|t| p.frozen_by(t.id).is_some())
            .collect();
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

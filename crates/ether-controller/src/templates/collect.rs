//! `SaveTracks`: the entities of a track template, collected from the document (pure; the
//! controller then fills plugin states and sample references).
//!
//! Tracks (selected ones plus the descendants of selected groups) in arrangement order
//! (a pre-order walk, so parents come before children), then their devices, pads, rack
//! chains and nested devices (parents first), modulators, sends between template tracks,
//! track automation lanes whose target is inside the template and their points, and
//! modulation mappings inside the template. Clips (and everything on them), take lanes,
//! MIDI mappings and anything else is left out. References to tracks outside the template
//! are cut here already (output → `Default`, input/VCA/sidechains → none, parent → top
//! level), and freeze state is dropped (its audio is not carried).

use std::collections::BTreeSet;

use ether_core::protocol::model::*;

use crate::tx::{CmdResult, invalid, not_found};

/// Every track id in arrangement order (pre-order: groups before their children).
fn flat_order(p: &Project) -> Vec<TrackId> {
    fn walk(p: &Project, tracks: Vec<&Track>, out: &mut Vec<TrackId>) {
        for t in tracks {
            out.push(t.id);
            walk(p, p.child_tracks(t.id), out);
        }
    }
    let mut out = Vec::new();
    walk(p, p.tracks_ordered(), &mut out);
    out
}

/// The entities of a track template made of `tracks` (see the module docs).
pub(crate) fn collect_tracks(p: &Project, tracks: &[TrackId]) -> CmdResult<Vec<Entity>> {
    if tracks.is_empty() {
        return Err(invalid("a track template needs at least one track"));
    }
    let mut selected: BTreeSet<TrackId> = BTreeSet::new();
    for id in tracks {
        let t = p
            .tracks
            .get(id)
            .ok_or_else(|| not_found(format!("track {id}")))?;
        if t.kind == TrackKind::Master {
            return Err(invalid("the master track cannot be saved as a template"));
        }
        selected.insert(*id);
    }
    // Selected tracks and every descendant of a selected group.
    let order = flat_order(p);
    let mut set: BTreeSet<TrackId> = BTreeSet::new();
    for id in &order {
        let mut cur = Some(*id);
        while let Some(c) = cur {
            if selected.contains(&c) {
                set.insert(*id);
                break;
            }
            cur = p.tracks.get(&c).and_then(|t| t.parent);
        }
    }
    let mut out = Vec::new();
    for id in order.iter().filter(|id| set.contains(id)) {
        let mut t = p.tracks[id].clone();
        if t.parent.is_some_and(|g| !set.contains(&g)) {
            t.parent = None;
        }
        if matches!(t.output, TrackOutput::Track { track } if !set.contains(&track)) {
            t.output = TrackOutput::Default;
        }
        if matches!(t.input, TrackInput::Track { track, .. } if !set.contains(&track)) {
            t.input = TrackInput::None;
        }
        if t.vca.is_some_and(|v| !set.contains(&v)) {
            t.vca = None;
        }
        t.freeze = None;
        out.push(Entity::Track(t));
    }

    // Devices, pads and chains, parents first (racks nest).
    let mut devices: BTreeSet<DeviceId> = BTreeSet::new();
    let mut pads: BTreeSet<DrumPadId> = BTreeSet::new();
    let mut chains: BTreeSet<RackChainId> = BTreeSet::new();
    let fix_device = |d: &Device| {
        let mut d = d.clone();
        if d.sidechain.is_some_and(|s| !set.contains(&s)) {
            d.sidechain = None;
        }
        d
    };
    for d in p
        .devices
        .values()
        .filter(|d| set.contains(&d.track) && d.pad.is_none() && d.chain.is_none())
    {
        devices.insert(d.id);
        out.push(Entity::Device(fix_device(d)));
    }
    loop {
        let mut progressed = false;
        for pad in p.drum_pads.values() {
            if devices.contains(&pad.rack) && pads.insert(pad.id) {
                out.push(Entity::DrumPad(pad.clone()));
                progressed = true;
            }
        }
        for c in p.rack_chains.values() {
            if devices.contains(&c.rack) && chains.insert(c.id) {
                out.push(Entity::RackChain(c.clone()));
                progressed = true;
            }
        }
        for d in p.devices.values() {
            let parent_in = d.pad.is_some_and(|x| pads.contains(&x))
                || d.chain.is_some_and(|x| chains.contains(&x));
            if parent_in && devices.insert(d.id) {
                out.push(Entity::Device(fix_device(d)));
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }

    let mut modulators: BTreeSet<ModulatorId> = BTreeSet::new();
    for m in p
        .modulators
        .values()
        .filter(|m| devices.contains(&m.device))
    {
        let mut m = m.clone();
        if m.sidechain.is_some_and(|s| !set.contains(&s)) {
            m.sidechain = None;
        }
        modulators.insert(m.id);
        out.push(Entity::Modulator(m));
    }

    let mut sends: BTreeSet<SendId> = BTreeSet::new();
    for s in p
        .sends
        .values()
        .filter(|s| set.contains(&s.from) && set.contains(&s.to))
    {
        sends.insert(s.id);
        out.push(Entity::Send(s.clone()));
    }

    let target_in = |t: &AutomationTarget| match *t {
        AutomationTarget::TrackVolume { track } | AutomationTarget::TrackPan { track } => {
            set.contains(&track)
        }
        AutomationTarget::SendLevel { send } => sends.contains(&send),
        AutomationTarget::DeviceParam { device, .. } => devices.contains(&device),
    };
    let lanes: Vec<&AutomationLane> = p
        .automation_lanes
        .values()
        .filter(|l| {
            matches!(l.owner, AutomationOwner::Track { track } if set.contains(&track))
                && target_in(&l.target)
        })
        .collect();
    for l in &lanes {
        out.push(Entity::AutomationLane((*l).clone()));
    }
    for l in &lanes {
        out.extend(
            p.points_of(l.id)
                .into_iter()
                .cloned()
                .map(Entity::AutomationPoint),
        );
    }

    for m in p.mod_mappings.values() {
        let source_in = match m.source {
            ModSource::Modulator { modulator } => modulators.contains(&modulator),
            ModSource::Macro { rack, .. } => devices.contains(&rack),
        };
        if source_in && devices.contains(&m.device) {
            out.push(Entity::ModMapping(m.clone()));
        }
    }
    Ok(out)
}

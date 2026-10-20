//! Rack chains reordered within their rack (`Rack::MoveChain`) and copied / moved between racks
//! (`Rack::CopyChain`, #892): same track, nested racks, other tracks; content rules, never
//! inside itself, modulation scope, automation carried on the same track, one undo step each.

use crate::common;

use common::*;
use ether_core::protocol::automation::{AutomationCommand, PointSpec};
use ether_core::protocol::devices::{DeviceCommand, DeviceSpec};
use ether_core::protocol::model::*;
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::racks::{ModulationCommand, RackCommand};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::*;

fn track(h: &mut Harness, kind: TrackKind) -> TrackId {
    let id: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id,
        kind,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    id
}

fn insert(h: &mut Harness, track: TrackId, ty: BuiltinDeviceType) -> DeviceId {
    let id: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id,
        track,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::new(ty),
        },
        before: None,
    }));
    id
}

fn add_chain(h: &mut Harness, rack: DeviceId) -> RackChainId {
    let id: RackChainId = h.id();
    h.ok(Command::Rack(RackCommand::AddChain {
        id,
        rack,
        name: None,
        before: None,
    }));
    id
}

fn chain_insert(h: &mut Harness, chain: RackChainId, ty: BuiltinDeviceType) -> DeviceId {
    let id: DeviceId = h.id();
    h.ok(Command::Rack(RackCommand::InsertDevice {
        id,
        chain,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::new(ty),
        },
        before: None,
    }));
    id
}

fn copy_chain(
    h: &mut Harness,
    id: RackChainId,
    rack: DeviceId,
    before: Option<RackChainId>,
    cut: bool,
) -> (RackChainId, Vec<ServerMessage>) {
    let new_id: RackChainId = h.id();
    let out = h.send(Command::Rack(RackCommand::CopyChain {
        id,
        new_id,
        rack,
        before,
        cut,
    }));
    (new_id, out)
}

fn names(h: &Harness, rack: DeviceId) -> Vec<String> {
    h.project()
        .chains_of(rack)
        .iter()
        .map(|c| c.name.clone())
        .collect()
}

fn rename(h: &mut Harness, id: RackChainId, name: &str) {
    h.ok(Command::Rack(RackCommand::RenameChain {
        id,
        name: name.into(),
    }));
}

fn undo(h: &mut Harness) {
    h.ok(Command::Edit(EditCommand::Undo));
}

fn redo(h: &mut Harness) {
    h.ok(Command::Edit(EditCommand::Redo));
}

/// An audio track with an audio effect rack holding chains "a", "b", "c" (its first chain
/// renamed "a").
fn three_chains(h: &mut Harness) -> (TrackId, DeviceId, [RackChainId; 3]) {
    let t = track(h, TrackKind::Audio);
    let rack = insert(h, t, BuiltinDeviceType::AudioEffectRack);
    let a = h.project().chains_of(rack)[0].id;
    let b = add_chain(h, rack);
    let c = add_chain(h, rack);
    for (id, n) in [(a, "a"), (b, "b"), (c, "c")] {
        rename(h, id, n);
    }
    (t, rack, [a, b, c])
}

#[test]
fn move_chain_reorders_within_its_rack_with_undo_redo() {
    let mut h = Harness::with_project();
    let (_, rack, [a, _, c]) = three_chains(&mut h);
    h.ok(Command::Rack(RackCommand::MoveChain {
        id: c,
        before: Some(a),
    }));
    assert_eq!(names(&h, rack), ["c", "a", "b"]);
    undo(&mut h);
    assert_eq!(names(&h, rack), ["a", "b", "c"]);
    redo(&mut h);
    assert_eq!(names(&h, rack), ["c", "a", "b"]);
    // To the end.
    h.ok(Command::Rack(RackCommand::MoveChain {
        id: c,
        before: None,
    }));
    assert_eq!(names(&h, rack), ["a", "b", "c"]);
}

#[test]
fn copy_chain_duplicates_in_place_keeping_settings_devices_and_macro_mappings() {
    let mut h = Harness::with_project();
    let (_, rack, [a, b, _]) = three_chains(&mut h);
    let delay = chain_insert(&mut h, a, BuiltinDeviceType::Delay);
    h.ok(Command::Device(DeviceCommand::SetParam {
        device: delay,
        param: ParamId(0),
        value: 0.25,
    }));
    h.ok(Command::Rack(RackCommand::SetChainMix {
        id: a,
        volume: Some(Decibels(-6.0)),
        pan: Some(Pan(0.5)),
        mute: Some(true),
        solo: None,
    }));
    let map: ModMappingId = h.id();
    h.ok(Command::Modulation(ModulationCommand::Map {
        id: map,
        source: ModSource::Macro { rack, index: 0 },
        device: delay,
        param: ParamId(0),
        depth: 0.5,
    }));
    let (copy, out) = copy_chain(&mut h, a, rack, Some(b), false);
    ok(&out);
    assert_eq!(names(&h, rack), ["a", "a", "b", "c"]);
    let p = h.project();
    let (src, dst) = (&p.rack_chains[&a], &p.rack_chains[&copy]);
    assert_eq!(
        (dst.volume, dst.pan, dst.mute, dst.rack),
        (src.volume, src.pan, src.mute, rack)
    );
    let devs = p.chain_devices_of(copy);
    assert_eq!(devs.len(), 1);
    let delay2 = devs[0].id;
    assert_ne!(delay2, delay);
    assert_eq!(devs[0].params, p.devices[&delay].params);
    // The macro still surrounds the copy: its mapping is copied.
    let maps = p.mappings_to(delay2);
    assert_eq!(maps.len(), 1);
    assert_eq!(maps[0].source, ModSource::Macro { rack, index: 0 });
    assert_eq!(p.mappings_to(delay).len(), 1);
    // One undo step; idempotent on new_id.
    let again = h.send(Command::Rack(RackCommand::CopyChain {
        id: a,
        new_id: copy,
        rack,
        before: None,
        cut: false,
    }));
    ok(&again);
    assert_eq!(names(&h, rack).len(), 4);
    undo(&mut h);
    assert!(!h.project().rack_chains.contains_key(&copy));
    assert!(!h.project().devices.contains_key(&delay2));
    assert_eq!(names(&h, rack), ["a", "b", "c"]);
}

#[test]
fn cut_moves_a_chain_to_another_rack_on_the_same_track_with_its_automation() {
    let mut h = Harness::with_project();
    let (t, rack, [a, b, _]) = three_chains(&mut h);
    let delay = chain_insert(&mut h, a, BuiltinDeviceType::Delay);
    let other = insert(&mut h, t, BuiltinDeviceType::AudioEffectRack);
    let first = h.project().chains_of(other)[0].id;
    // A track lane on the delay's first param, and a mapping from the source rack's macro.
    let lane: AutomationLaneId = h.id();
    h.ok(Command::Automation(AutomationCommand::CreateLane {
        id: lane,
        owner: AutomationOwner::Track { track: t },
        target: AutomationTarget::DeviceParam {
            device: delay,
            param: ParamId(0),
        },
    }));
    let points = [(0.0, 0.1), (4.0, 0.9)]
        .iter()
        .map(|&(t, v)| PointSpec {
            id: h.id(),
            time: Beats(t),
            value: v,
            curve: CurveShape::Linear,
        })
        .collect();
    h.ok(Command::Automation(AutomationCommand::AddPoints {
        lane,
        points,
    }));
    let map: ModMappingId = h.id();
    h.ok(Command::Modulation(ModulationCommand::Map {
        id: map,
        source: ModSource::Macro { rack, index: 1 },
        device: delay,
        param: ParamId(1),
        depth: 0.5,
    }));
    let before = h.project().clone();

    let (moved, out) = copy_chain(&mut h, a, other, Some(first), true);
    ok(&out);
    let p = h.project();
    assert!(!p.rack_chains.contains_key(&a));
    assert!(!p.devices.contains_key(&delay));
    assert_eq!(names(&h, rack), ["b", "c"]);
    assert_eq!(names(&h, other), ["a", "chain 1"]);
    let delay2 = p.chain_devices_of(moved)[0].id;
    // The lane follows (retargeted, same points); the old rack's macro mapping is dropped.
    let lanes: Vec<&AutomationLane> = p
        .automation_lanes
        .values()
        .filter(|l| l.target.device() == Some(delay2))
        .collect();
    assert_eq!(lanes.len(), 1);
    assert_eq!(lanes[0].owner, AutomationOwner::Track { track: t });
    let pts = p
        .automation_points
        .values()
        .filter(|pt| pt.lane == lanes[0].id)
        .count();
    assert_eq!(pts, 2);
    assert!(p.mappings_to(delay2).is_empty());
    // A notice says the macro mapping went.
    assert!(
        format!("{out:?}").contains("modulation"),
        "expected a notice: {out:?}"
    );
    // One undo step brings everything back as it was; redo moves it again.
    undo(&mut h);
    let p = h.project();
    assert_eq!(p.rack_chains, before.rack_chains);
    assert_eq!(p.devices, before.devices);
    assert_eq!(p.automation_lanes, before.automation_lanes);
    assert_eq!(p.mod_mappings, before.mod_mappings);
    redo(&mut h);
    assert_eq!(names(&h, other), ["a", "chain 1"]);
    let _ = b;
}

#[test]
fn chains_move_across_tracks_and_into_nested_racks() {
    let mut h = Harness::with_project();
    let (_, rack, [a, _, _]) = three_chains(&mut h);
    chain_insert(&mut h, a, BuiltinDeviceType::Delay);
    // A nested rack holding a reverb, on the chain to move.
    let nested = chain_insert(&mut h, a, BuiltinDeviceType::AudioEffectRack);
    let inner = h.project().chains_of(nested)[0].id;
    chain_insert(&mut h, inner, BuiltinDeviceType::Reverb);

    let other_track = track(&mut h, TrackKind::Audio);
    let target = insert(&mut h, other_track, BuiltinDeviceType::AudioEffectRack);
    let (moved, out) = copy_chain(&mut h, a, target, None, true);
    ok(&out);
    let p = h.project();
    let devs = p.chain_devices_of(moved);
    assert_eq!(devs.len(), 2);
    assert!(devs.iter().all(|d| d.track == other_track));
    let nested2 = devs[1].id;
    let inner2 = p.chains_of(nested2);
    assert_eq!(inner2.len(), 1);
    let reverb = p.chain_devices_of(inner2[0].id);
    assert_eq!(reverb.len(), 1);
    assert_eq!(reverb[0].track, other_track);
    assert_eq!(names(&h, rack), ["b", "c"]);
    undo(&mut h);
    assert_eq!(names(&h, rack), ["a", "b", "c"]);

    // Into a rack nested on a sibling chain of the same rack (copy).
    let b = h.project().chains_of(rack)[1].id;
    let sub = chain_insert(&mut h, b, BuiltinDeviceType::AudioEffectRack);
    let (copy, out) = copy_chain(&mut h, a, sub, None, false);
    ok(&out);
    let p = h.project();
    assert_eq!(p.rack_chains[&copy].rack, sub);
    assert_eq!(p.chain_devices_of(copy).len(), 2);
}

#[test]
fn invalid_targets_are_refused_and_change_nothing() {
    let mut h = Harness::with_project();
    let midi = track(&mut h, TrackKind::Midi);
    let inst = insert(&mut h, midi, BuiltinDeviceType::InstrumentRack);
    let synth_chain = add_chain(&mut h, inst);
    chain_insert(&mut h, synth_chain, BuiltinDeviceType::Synth);
    let (_, fx, [a, _, _]) = three_chains(&mut h);
    let nested = chain_insert(&mut h, a, BuiltinDeviceType::AudioEffectRack);
    let snapshot = h.project().clone();

    // An instrument chain into an audio effect rack.
    let (_, out) = copy_chain(&mut h, synth_chain, fx, None, true);
    assert!(
        err(&out).message.contains("audio effects"),
        "{:?}",
        err(&out)
    );
    // A chain into a rack nested on itself.
    let (_, out) = copy_chain(&mut h, a, nested, None, true);
    assert!(
        err(&out).message.contains("inside itself"),
        "{:?}",
        err(&out)
    );
    let (_, out) = copy_chain(&mut h, a, nested, None, false);
    assert!(
        err(&out).message.contains("inside itself"),
        "{:?}",
        err(&out)
    );
    // `before` must be a chain of the target rack; the target must be a rack.
    let (_, out) = copy_chain(&mut h, a, inst, Some(a), false);
    assert!(err(&out).message.contains("before"), "{:?}", err(&out));
    let delay = chain_insert(&mut h, a, BuiltinDeviceType::Delay);
    let snapshot2 = h.project().clone();
    let (_, out) = copy_chain(&mut h, a, delay, None, false);
    assert!(err(&out).message.contains("not a rack"), "{:?}", err(&out));
    assert_eq!(h.project().rack_chains, snapshot2.rack_chains);
    assert_eq!(h.project().devices.len(), snapshot2.devices.len());
    assert!(snapshot.rack_chains.len() <= h.project().rack_chains.len());

    // An effect chain into an instrument rack is fine (instrument rack chains take anything).
    let (copy, out) = copy_chain(&mut h, a, inst, None, false);
    ok(&out);
    assert!(
        h.project()
            .chain_devices_of(copy)
            .iter()
            .all(|d| d.track == midi)
    );
}

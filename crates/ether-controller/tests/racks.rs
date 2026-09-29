//! Racks (`racks-modulation`): chains, chain devices, mix/zones/solo compiled into
//! `TrackDesc::chain_racks`, chain content rules, Group (with modulators re-hosted on the
//! rack), duplicate, cascades, one undo step each, save/reopen.

mod common;

use common::*;
use ether_core::graph::TrackDesc;
use ether_core::protocol::devices::{DeviceCommand, DeviceSpec};
use ether_core::protocol::model::*;
use ether_core::protocol::project::{EditCommand, ProjectCommand};
use ether_core::protocol::racks::{ModulationCommand, RackCommand};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::*;
use ether_core::rack_chains::ChainRackKind;

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

fn rack_cmd(h: &mut Harness, c: RackCommand) -> ReplyValue {
    h.ok(Command::Rack(c))
}

fn add_chain(h: &mut Harness, rack: DeviceId) -> RackChainId {
    let id: RackChainId = h.id();
    rack_cmd(
        h,
        RackCommand::AddChain {
            id,
            rack,
            name: None,
            before: None,
        },
    );
    id
}

fn chain_insert(h: &mut Harness, chain: RackChainId, ty: BuiltinDeviceType) -> DeviceId {
    let id: DeviceId = h.id();
    rack_cmd(
        h,
        RackCommand::InsertDevice {
            id,
            chain,
            device: DeviceSpec::Builtin {
                device: BuiltinDevice::new(ty),
            },
            before: None,
        },
    );
    id
}

fn undo(h: &mut Harness) {
    h.ok(Command::Edit(EditCommand::Undo));
}

fn graph_track(h: &mut Harness, track: TrackId) -> TrackDesc {
    h.tick();
    h.ctl
        .bridge
        .last_graph()
        .tracks
        .iter()
        .find(|t| t.id == track)
        .cloned()
        .unwrap()
}

fn node_of(h: &Harness, device: DeviceId) -> ether_core::NodeKey {
    *h.ctl
        .bridge
        .live
        .iter()
        .find(|(_, d)| **d == device)
        .expect("device has a node")
        .0
}

#[test]
fn chains_compile_with_devices_mix_zones_and_solo() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
    let rack = insert(&mut h, t, BuiltinDeviceType::InstrumentRack);
    let a = add_chain(&mut h, rack);
    let b = add_chain(&mut h, rack);
    assert_eq!(h.project().rack_chains[&a].name, "Chain 1");
    assert_eq!(h.project().rack_chains[&b].name, "Chain 2");
    let synth = chain_insert(&mut h, a, BuiltinDeviceType::Synth);
    let delay = chain_insert(&mut h, a, BuiltinDeviceType::Delay);
    let synth_b = chain_insert(&mut h, b, BuiltinDeviceType::PolySynth);
    // Chain devices are not on the track chain.
    assert_eq!(
        h.project()
            .devices_of(t)
            .iter()
            .map(|d| d.id)
            .collect::<Vec<_>>(),
        vec![rack]
    );
    rack_cmd(
        &mut h,
        RackCommand::SetChainMix {
            id: b,
            volume: Some(Decibels(-6.0)),
            pan: Some(Pan(0.5)),
            mute: None,
            solo: None,
        },
    );
    rack_cmd(
        &mut h,
        RackCommand::SetChainZones {
            id: b,
            keys: Some(Zone { lo: 60, hi: 127 }),
            velocities: None,
            select: Some(Zone { lo: 10, hi: 20 }),
        },
    );
    h.ok(Command::Device(DeviceCommand::SetParam {
        device: rack,
        param: RACK_SELECTOR_PARAM,
        value: 12.0,
    }));
    let td = graph_track(&mut h, t);
    assert_eq!(td.chain.len(), 1);
    let r = &td.chain_racks[0];
    assert_eq!(r.rack, node_of(&h, rack));
    assert_eq!(r.kind, ChainRackKind::Instrument);
    assert_eq!(r.selector, 12);
    assert_eq!(
        r.chains[0].chain.iter().map(|e| e.node).collect::<Vec<_>>(),
        vec![node_of(&h, synth), node_of(&h, delay)]
    );
    assert_eq!(r.chains[1].chain[0].node, node_of(&h, synth_b));
    assert!((r.chains[1].volume - Decibels(-6.0).to_linear()).abs() < 1e-6);
    assert_eq!(r.chains[1].pan, 0.5);
    assert_eq!(r.chains[1].keys, (60, 127));
    assert_eq!(r.chains[1].select, (10, 20));
    assert!(!r.chains[0].mute && !r.chains[1].mute);
    // Solo mutes the other chains.
    rack_cmd(
        &mut h,
        RackCommand::SetChainMix {
            id: a,
            volume: None,
            pan: None,
            mute: None,
            solo: Some(true),
        },
    );
    let td = graph_track(&mut h, t);
    assert!(!td.chain_racks[0].chains[0].mute);
    assert!(td.chain_racks[0].chains[1].mute);
}

#[test]
fn chain_content_rules_and_no_nesting() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
    let fx = insert(&mut h, t, BuiltinDeviceType::AudioEffectRack);
    let c = add_chain(&mut h, fx);
    let id: DeviceId = h.id();
    let out = h.send(Command::Rack(RackCommand::InsertDevice {
        id,
        chain: c,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::Synth,
        },
        before: None,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    let out = h.send(Command::Rack(RackCommand::InsertDevice {
        id,
        chain: c,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::new(BuiltinDeviceType::AudioEffectRack),
        },
        before: None,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    let midi = insert(&mut h, t, BuiltinDeviceType::MidiEffectRack);
    let mc = add_chain(&mut h, midi);
    let out = h.send(Command::Rack(RackCommand::InsertDevice {
        id,
        chain: mc,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::Delay,
        },
        before: None,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    chain_insert(&mut h, mc, BuiltinDeviceType::Arpeggiator);
    chain_insert(&mut h, c, BuiltinDeviceType::Delay);
}

#[test]
fn move_device_between_track_chain_and_rack_chain() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Audio);
    let rack = insert(&mut h, t, BuiltinDeviceType::AudioEffectRack);
    let delay = insert(&mut h, t, BuiltinDeviceType::Delay);
    let c = add_chain(&mut h, rack);
    rack_cmd(
        &mut h,
        RackCommand::MoveDevice {
            id: delay,
            chain: Some(c),
            before: None,
        },
    );
    assert_eq!(h.project().devices[&delay].chain, Some(c));
    assert_eq!(h.project().devices_of(t).len(), 1);
    rack_cmd(
        &mut h,
        RackCommand::MoveDevice {
            id: delay,
            chain: None,
            before: Some(rack),
        },
    );
    assert_eq!(h.project().devices[&delay].chain, None);
    assert_eq!(
        h.project()
            .devices_of(t)
            .iter()
            .map(|d| d.id)
            .collect::<Vec<_>>(),
        vec![delay, rack]
    );
    undo(&mut h);
    assert_eq!(h.project().devices[&delay].chain, Some(c));
}

#[test]
fn group_wraps_devices_and_rehosts_their_modulators() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Audio);
    let eq = insert(&mut h, t, BuiltinDeviceType::Eq);
    let delay = insert(&mut h, t, BuiltinDeviceType::Delay);
    let comp = insert(&mut h, t, BuiltinDeviceType::Compressor);
    let lfo: ModulatorId = h.id();
    h.ok(Command::Modulation(ModulationCommand::AddModulator {
        id: lfo,
        device: delay,
        kind: ModulatorKind::Lfo,
        name: None,
    }));
    let map: ModMappingId = h.id();
    h.ok(Command::Modulation(ModulationCommand::Map {
        id: map,
        source: ModSource::Modulator { modulator: lfo },
        device: delay,
        param: ParamId(0),
        depth: 0.4,
    }));
    let before = h.project().clone();
    let rack: DeviceId = h.id();
    let chain: RackChainId = h.id();
    // Not consecutive.
    let out = h.send(Command::Rack(RackCommand::Group {
        rack,
        rack_type: BuiltinDeviceType::AudioEffectRack,
        chain,
        devices: vec![eq, comp],
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    rack_cmd(
        &mut h,
        RackCommand::Group {
            rack,
            rack_type: BuiltinDeviceType::AudioEffectRack,
            chain,
            devices: vec![delay, eq],
        },
    );
    let p = h.project();
    assert_eq!(
        p.devices_of(t).iter().map(|d| d.id).collect::<Vec<_>>(),
        vec![rack, comp]
    );
    assert_eq!(
        p.chain_devices_of(chain)
            .iter()
            .map(|d| d.id)
            .collect::<Vec<_>>(),
        vec![eq, delay]
    );
    assert!(!p.modulators.contains_key(&lfo));
    let rehosted = p.modulators_of(rack);
    assert_eq!(rehosted.len(), 1);
    let new_lfo = rehosted[0].id;
    assert_eq!(new_lfo, derive_id(rack, 0));
    let maps = p.mappings_to(delay);
    assert_eq!(maps.len(), 1);
    assert_eq!(maps[0].source, ModSource::Modulator { modulator: new_lfo });
    assert_eq!(maps[0].depth, 0.4);
    // Retried: idempotent. One undo step restores everything.
    rack_cmd(
        &mut h,
        RackCommand::Group {
            rack,
            rack_type: BuiltinDeviceType::AudioEffectRack,
            chain,
            devices: vec![delay, eq],
        },
    );
    undo(&mut h);
    assert_eq!(h.project(), &before);
}

#[test]
fn removing_a_rack_or_chain_cascades_and_undoes() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
    let rack = insert(&mut h, t, BuiltinDeviceType::InstrumentRack);
    let c = add_chain(&mut h, rack);
    let synth = chain_insert(&mut h, c, BuiltinDeviceType::Synth);
    let map: ModMappingId = h.id();
    h.ok(Command::Modulation(ModulationCommand::Map {
        id: map,
        source: ModSource::Macro { rack, index: 2 },
        device: synth,
        param: ParamId(0),
        depth: 1.0,
    }));
    let before = h.project().clone();
    rack_cmd(&mut h, RackCommand::RemoveChain { id: c });
    assert!(h.project().rack_chains.is_empty());
    assert!(!h.project().devices.contains_key(&synth));
    assert!(h.project().mod_mappings.is_empty());
    undo(&mut h);
    assert_eq!(h.project(), &before);
    h.ok(Command::Device(DeviceCommand::Remove { id: rack }));
    assert!(h.project().devices.is_empty());
    assert!(h.project().rack_chains.is_empty());
    undo(&mut h);
    assert_eq!(h.project(), &before);
}

#[test]
fn duplicating_a_rack_copies_chains_modulators_and_mappings() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
    let rack = insert(&mut h, t, BuiltinDeviceType::InstrumentRack);
    let c = add_chain(&mut h, rack);
    let synth = chain_insert(&mut h, c, BuiltinDeviceType::Synth);
    let lfo: ModulatorId = h.id();
    h.ok(Command::Modulation(ModulationCommand::AddModulator {
        id: lfo,
        device: rack,
        kind: ModulatorKind::Lfo,
        name: None,
    }));
    for (source, param) in [
        (ModSource::Modulator { modulator: lfo }, ParamId(1)),
        (ModSource::Macro { rack, index: 0 }, ParamId(2)),
    ] {
        let id: ModMappingId = h.id();
        h.ok(Command::Modulation(ModulationCommand::Map {
            id,
            source,
            device: synth,
            param,
            depth: 0.5,
        }));
    }
    let copy: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Duplicate {
        id: rack,
        new_id: copy,
    }));
    let p = h.project();
    let chains = p.chains_of(copy);
    assert_eq!(chains.len(), 1);
    let devs = p.chain_devices_of(chains[0].id);
    assert_eq!(devs.len(), 1);
    let synth2 = devs[0].id;
    assert_ne!(synth2, synth);
    let mods = p.modulators_of(copy);
    assert_eq!(mods.len(), 1);
    let maps = p.mappings_to(synth2);
    assert_eq!(maps.len(), 2);
    assert!(maps.iter().any(|m| m.source
        == ModSource::Modulator {
            modulator: mods[0].id
        }));
    assert!(maps.iter().any(|m| m.source
        == ModSource::Macro {
            rack: copy,
            index: 0
        }));
    // The original is untouched.
    assert_eq!(p.mappings_to(synth).len(), 2);
}

#[test]
fn midi_racks_always_compile_and_racks_survive_save_and_reopen() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
    let midi = insert(&mut h, t, BuiltinDeviceType::MidiEffectRack);
    let td = graph_track(&mut h, t);
    assert_eq!(td.chain_racks.len(), 1);
    assert_eq!(td.chain_racks[0].kind, ChainRackKind::MidiEffect);
    assert!(td.chain_racks[0].chains.is_empty());
    let c = add_chain(&mut h, midi);
    chain_insert(&mut h, c, BuiltinDeviceType::Chord);
    rack_cmd(
        &mut h,
        RackCommand::RenameChain {
            id: c,
            name: "Chords".into(),
        },
    );
    let doc = h.project().clone();
    h.ok(Command::Project(ProjectCommand::Save));
    h.create_project("Other");
    h.ok(Command::Project(ProjectCommand::Open { id: doc.id }));
    assert_eq!(h.project(), &doc);
    let td = graph_track(&mut h, t);
    assert_eq!(td.chain_racks[0].chains[0].chain.len(), 1);
}

#[test]
fn duplicating_a_track_copies_its_racks_and_modulation() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
    let rack = insert(&mut h, t, BuiltinDeviceType::InstrumentRack);
    let c = add_chain(&mut h, rack);
    let synth = chain_insert(&mut h, c, BuiltinDeviceType::Synth);
    let map: ModMappingId = h.id();
    h.ok(Command::Modulation(ModulationCommand::Map {
        id: map,
        source: ModSource::Macro { rack, index: 0 },
        device: synth,
        param: ParamId(1),
        depth: 0.5,
    }));
    let copy: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Duplicate {
        id: t,
        new_id: copy,
    }));
    let p = h.project();
    let racks = p.devices_of(copy);
    assert_eq!(racks.len(), 1);
    let chains = p.chains_of(racks[0].id);
    assert_eq!(chains.len(), 1);
    let devs = p.chain_devices_of(chains[0].id);
    assert_eq!(devs.len(), 1);
    assert_eq!(devs[0].track, copy);
    let maps = p.mappings_to(devs[0].id);
    assert_eq!(maps.len(), 1);
    assert_eq!(
        maps[0].source,
        ModSource::Macro {
            rack: racks[0].id,
            index: 0
        }
    );
}

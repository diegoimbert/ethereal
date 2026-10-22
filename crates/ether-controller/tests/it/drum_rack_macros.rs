//! Drum rack macros: the 16 rack macros (params 2..=17, after the rack's volume and pan) map
//! to params of the devices on the rack's pads at any depth (pad racks' chains included),
//! compile into `TrackDesc::modulation` with `MacroDesc::first`, undo in one step, follow
//! duplication, survive save/reopen; an outer instrument rack's macros reach into a nested
//! drum rack's pads.

use crate::common;

use common::*;
use ether_core::graph::TrackDesc;
use ether_core::modulation::ModSourceDesc;
use ether_core::protocol::devices::{DeviceCommand, DeviceSpec};
use ether_core::protocol::drum_rack::DrumRackCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::project::{EditCommand, ProjectCommand};
use ether_core::protocol::racks::{ModulationCommand, RackCommand};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::{Command, ErrorCode, Reply, ReplyResult, ServerMessage};

fn track(h: &mut Harness) -> TrackId {
    let id = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id,
        kind: TrackKind::Midi,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    id
}

fn insert(h: &mut Harness, track: TrackId, ty: BuiltinDeviceType) -> DeviceId {
    let id = h.id();
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

fn add_pad(h: &mut Harness, rack: DeviceId, note: u8) -> DrumPadId {
    let id: DrumPadId = h.id();
    h.ok(Command::DrumRack(DrumRackCommand::AddPad {
        id,
        rack,
        note,
        name: None,
    }));
    id
}

fn pad_insert(h: &mut Harness, pad: DrumPadId, ty: BuiltinDeviceType) -> DeviceId {
    let id: DeviceId = h.id();
    h.ok(Command::DrumRack(DrumRackCommand::InsertDevice {
        id,
        pad,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::new(ty),
        },
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

fn map(
    h: &mut Harness,
    rack: DeviceId,
    index: u8,
    device: DeviceId,
    param: ParamId,
    depth: f64,
) -> Result<ModMappingId, ErrorCode> {
    let id: ModMappingId = h.id();
    let out = h.send(Command::Modulation(ModulationCommand::Map {
        id,
        source: ModSource::Macro { rack, index },
        device,
        param,
        depth,
    }));
    match out.last() {
        Some(ServerMessage::Reply(Reply {
            result: ReplyResult::Err { error },
            ..
        })) => Err(error.code),
        _ => Ok(id),
    }
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

/// The first automatable param of a built-in type.
fn automatable(ty: BuiltinDeviceType) -> ParamId {
    ether_devices::descriptor(ty)
        .params
        .iter()
        .find(|p| p.automatable)
        .expect("an automatable param")
        .id
}

const MACRO_1: ParamId = ParamId(DRUM_RACK_FIRST_MACRO);

#[test]
fn drum_rack_macros_map_to_pad_devices_and_compile() {
    let mut h = Harness::with_project();
    let t = track(&mut h);
    let drums = insert(&mut h, t, BuiltinDeviceType::DrumRack);
    let other = insert(&mut h, t, BuiltinDeviceType::Reverb);
    let kick = add_pad(&mut h, drums, 36);
    let synth = pad_insert(&mut h, kick, BuiltinDeviceType::Synth);
    // A pad rack with a chorus in its (first) chain.
    let pad_fx = pad_insert(&mut h, kick, BuiltinDeviceType::AudioEffectRack);
    let chain = h.project().chains_of(pad_fx)[0].id;
    let chorus = chain_insert(&mut h, chain, BuiltinDeviceType::Chorus);
    let synth_p = automatable(BuiltinDeviceType::Synth);
    let chorus_p = automatable(BuiltinDeviceType::Chorus);

    // The macro knobs are the rack's params 2..=9.
    h.ok(Command::Device(DeviceCommand::SetParam {
        device: drums,
        param: ParamId(MACRO_1.0 + 3),
        value: 0.6,
    }));
    // Mapping is one undo step.
    let before = h.project().clone();
    let m1 = map(&mut h, drums, 0, synth, synth_p, 0.5).unwrap();
    undo(&mut h);
    assert_eq!(h.project(), &before);
    h.ok(Command::Modulation(ModulationCommand::Map {
        id: m1,
        source: ModSource::Macro {
            rack: drums,
            index: 0,
        },
        device: synth,
        param: synth_p,
        depth: 0.5,
    }));
    map(&mut h, drums, 3, chorus, chorus_p, -0.25).unwrap();
    // The rack's own volume is a target; its macros, another device and macro 17 are not.
    map(&mut h, drums, 1, drums, ParamId(0), 0.5).unwrap();
    assert!(map(&mut h, drums, 2, drums, MACRO_1, 0.5).is_err());
    assert!(map(&mut h, drums, 0, other, ParamId(0), 0.5).is_err());
    assert!(map(&mut h, drums, 16, synth, synth_p, 0.5).is_err());

    // Compiled: the drum rack's macro values from its params 2..=9, mappings onto the pad
    // devices' nodes.
    let tr = graph_track(&mut h, t);
    let rack_node = node_of(&h, drums);
    let m = tr
        .modulation
        .macros
        .iter()
        .find(|m| m.rack == rack_node)
        .expect("the drum rack's macros");
    // Its 16 macros are contiguous: macro 9 follows macro 8.
    assert_eq!(
        (m.first, m.upper),
        (DRUM_RACK_FIRST_MACRO, DRUM_RACK_FIRST_MACRO + 8)
    );
    assert_eq!(m.values[3], 0.6);
    let targets: Vec<_> = tr
        .modulation
        .mappings
        .iter()
        .map(|x| (x.source, x.node, x.param))
        .collect();
    for (index, device, param) in [(0, synth, synth_p), (3, chorus, chorus_p)] {
        assert!(
            targets.contains(&(
                ModSourceDesc::Macro {
                    rack: rack_node,
                    index
                },
                node_of(&h, device),
                param
            )),
            "{targets:?}"
        );
    }

    // Moving the synth to another pad of the same rack keeps its mapping.
    let snare = add_pad(&mut h, drums, 38);
    h.ok(Command::DrumRack(DrumRackCommand::MoveDevice {
        id: synth,
        pad: Some(snare),
        before: None,
    }));
    assert!(h.project().mod_mappings.contains_key(&m1));

    // Duplicating the drum rack copies its macro mappings onto the copied pad devices.
    let copy: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Duplicate {
        id: drums,
        new_id: copy,
        track: None,
    }));
    let p = h.project();
    let copied: Vec<&ModMapping> = p
        .mod_mappings
        .values()
        .filter(|m| matches!(m.source, ModSource::Macro { rack, .. } if rack == copy))
        .collect();
    assert_eq!(copied.len(), 3);
    for m in &copied {
        assert!(m.device == copy || p.is_inside(m.device, copy), "{m:?}");
        assert!(m.device != synth && m.device != chorus);
    }
    assert_eq!(
        p.devices[&copy].params.get(&ParamId(MACRO_1.0 + 3)),
        Some(&0.6)
    );
    undo(&mut h);

    // Saved and reopened as is.
    let doc = h.project().clone();
    h.ok(Command::Project(ProjectCommand::Save { autosave: false }));
    h.create_project("Other");
    h.ok(Command::Project(ProjectCommand::Open { id: doc.id }));
    assert_eq!(h.project(), &doc);

    // Removing a pad removes the mappings onto its devices.
    h.ok(Command::DrumRack(DrumRackCommand::RemovePad { id: kick }));
    assert!(
        h.project()
            .mod_mappings
            .values()
            .all(|m| m.device != chorus)
    );
    undo(&mut h);
    assert_eq!(h.project(), &doc);
}

#[test]
fn an_outer_racks_macros_reach_a_nested_drum_racks_pads() {
    let mut h = Harness::with_project();
    let t = track(&mut h);
    let inst = insert(&mut h, t, BuiltinDeviceType::InstrumentRack);
    let ic: RackChainId = h.id();
    h.ok(Command::Rack(RackCommand::AddChain {
        id: ic,
        rack: inst,
        name: None,
        before: None,
    }));
    let drums = chain_insert(&mut h, ic, BuiltinDeviceType::DrumRack);
    let pad = add_pad(&mut h, drums, 36);
    let synth = pad_insert(&mut h, pad, BuiltinDeviceType::Synth);
    let p = automatable(BuiltinDeviceType::Synth);
    map(&mut h, inst, 2, synth, p, 0.5).unwrap();
    // And the nested drum rack's own macros too.
    map(&mut h, drums, 0, synth, p, 0.25).unwrap();
    let tr = graph_track(&mut h, t);
    let racks: Vec<_> = tr
        .modulation
        .macros
        .iter()
        .map(|m| (m.rack, m.first))
        .collect();
    assert!(racks.contains(&(node_of(&h, inst), 0)), "{racks:?}");
    assert!(
        racks.contains(&(node_of(&h, drums), DRUM_RACK_FIRST_MACRO)),
        "{racks:?}"
    );
    assert_eq!(
        tr.modulation
            .mappings
            .iter()
            .filter(|m| m.node == node_of(&h, synth))
            .count(),
        2
    );
    // Moving the drum rack out of the instrument rack drops the outer macro's mapping (out
    // of scope) and keeps its own.
    h.ok(Command::Rack(RackCommand::MoveDevice {
        id: drums,
        chain: None,
        before: None,
    }));
    let sources: Vec<ModSource> = h
        .project()
        .mod_mappings
        .values()
        .map(|m| m.source)
        .collect();
    assert_eq!(
        sources,
        vec![ModSource::Macro {
            rack: drums,
            index: 0
        }]
    );
}

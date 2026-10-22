//! Rack macro banks (CONTRACTS.md §12.6, `ether_model::RackMacros`): every rack type,
//! drum racks included, has 16 macros (9..16 appended after the existing params), shows
//! `visible` of them and may rename them. `Device::RenameMacro` / `Device::SetMacroCount` are
//! one undo step each, keep the rack's node, compile all 16 macros into the engine
//! (`MacroDesc::{first, upper}`), save with the project and round-trip through rack presets.

use crate::common;

use common::*;
use ether_controller::memory::MemoryLibrary;
use ether_core::graph::TrackDesc;
use ether_core::modulation::ModSourceDesc;
use ether_core::protocol::devices::{DeviceCommand, DeviceSpec};
use ether_core::protocol::model::*;
use ether_core::protocol::presets::{PresetCommand, PresetInfo};
use ether_core::protocol::project::{EditCommand, ProjectCommand};
use ether_core::protocol::racks::{ModulationCommand, RackCommand};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::{Command, ErrorCode, ReplyValue};

fn harness() -> Harness {
    let mut h = Harness::with(
        FakeBridge::default(),
        MemoryLibrary::new().with_user_root("user"),
        Default::default(),
    );
    h.create_project("Macro banks");
    h
}

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

fn bank(h: &Harness, rack: DeviceId) -> RackMacros {
    let DeviceKind::Builtin { device } = &h.project().devices[&rack].kind else {
        panic!("a built-in")
    };
    device.rack_macros().expect("a rack")
}

fn rename(h: &mut Harness, device: DeviceId, index: u8, name: &str) {
    h.ok(Command::Device(DeviceCommand::RenameMacro {
        device,
        index,
        name: name.into(),
    }));
}

fn count(h: &mut Harness, device: DeviceId, count: u8) {
    h.ok(Command::Device(DeviceCommand::SetMacroCount {
        device,
        count,
    }));
}

fn undo(h: &mut Harness) {
    h.ok(Command::Edit(EditCommand::Undo));
}

fn redo(h: &mut Harness) {
    h.ok(Command::Edit(EditCommand::Redo));
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

const RACKS: [BuiltinDeviceType; 4] = [
    BuiltinDeviceType::InstrumentRack,
    BuiltinDeviceType::AudioEffectRack,
    BuiltinDeviceType::MidiEffectRack,
    BuiltinDeviceType::DrumRack,
];

#[test]
fn every_rack_has_16_macros_appended_after_its_params() {
    for ty in RACKS {
        let desc = ether_devices::descriptor(ty);
        for i in 0..RACK_MACROS {
            let id = macro_param(ty, i).expect("a rack");
            let p = desc
                .params
                .iter()
                .find(|p| p.id == id)
                .unwrap_or_else(|| panic!("{ty:?} macro {i}: no param {id:?}"));
            assert_eq!(p.name, format!("Macro {}", i + 1), "{ty:?}");
            assert_eq!(macro_index(ty, id), Some(i));
        }
        assert_eq!(macro_param(ty, RACK_MACROS), None);
    }
    // The original ids stay where they were (append-only).
    let inst = BuiltinDeviceType::InstrumentRack;
    assert_eq!(macro_param(inst, 7), Some(ParamId(7)));
    assert_eq!(macro_index(inst, RACK_SELECTOR_PARAM), None);
    assert_eq!(macro_param(inst, 8), Some(ParamId(9)));
    assert_eq!(macro_param(inst, 15), Some(ParamId(16)));
    let drum = BuiltinDeviceType::DrumRack;
    assert_eq!(macro_param(drum, 0), Some(ParamId(2)));
    assert_eq!(macro_param(drum, 15), Some(ParamId(17)));
}

#[test]
fn renaming_and_showing_macros_is_undoable_and_keeps_the_node() {
    for ty in RACKS {
        let mut h = harness();
        let t = track(&mut h);
        let rack = insert(&mut h, t, ty);
        h.tick();
        let node = node_of(&h, rack);
        assert_eq!(bank(&h, rack), RackMacros::default());

        let start = h.project().clone();
        rename(&mut h, rack, 0, "  Brightness  ");
        rename(&mut h, rack, 9, "Space");
        count(&mut h, rack, 12);
        let b = bank(&h, rack);
        assert_eq!(b.visible, 12, "{ty:?}");
        assert_eq!(b.name(0), Some("Brightness"));
        assert_eq!(b.name(9), Some("Space"));
        assert_eq!(b.name(1), None);
        assert_eq!(b.display_name(1), "Macro 2");
        // Document data only: the rack's node is kept.
        h.tick();
        assert_eq!(node_of(&h, rack), node, "{ty:?}");

        // One undo step each.
        undo(&mut h);
        assert_eq!(bank(&h, rack).visible, 8);
        undo(&mut h);
        assert_eq!(bank(&h, rack).name(9), None);
        redo(&mut h);
        assert_eq!(bank(&h, rack).name(9), Some("Space"));
        undo(&mut h);
        undo(&mut h);
        assert_eq!(h.project(), &start, "{ty:?}");

        // An empty name is the default again; the defaults are stored as none.
        rename(&mut h, rack, 3, "Drive");
        rename(&mut h, rack, 3, "   ");
        assert_eq!(h.project().devices[&rack], start.devices[&rack]);
        // Clamped to 1..=16; out-of-range macros and non-racks are refused.
        count(&mut h, rack, 0);
        assert_eq!(bank(&h, rack).visible, 1);
        count(&mut h, rack, 40);
        assert_eq!(bank(&h, rack).visible, 16);
        let out = h.send(Command::Device(DeviceCommand::RenameMacro {
            device: rack,
            index: 16,
            name: "x".into(),
        }));
        assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
        let synth = insert(&mut h, t, BuiltinDeviceType::Synth);
        let out = h.send(Command::Device(DeviceCommand::SetMacroCount {
            device: synth,
            count: 4,
        }));
        assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
        // Long names are cut to 64 chars.
        rename(&mut h, rack, 2, &"x".repeat(100));
        assert_eq!(bank(&h, rack).name(2).map(str::len), Some(64));
    }
}

#[test]
fn macros_9_to_16_compile_and_modulate() {
    let mut h = harness();
    let t = track(&mut h);
    let rack = insert(&mut h, t, BuiltinDeviceType::InstrumentRack);
    let chain: RackChainId = h.id();
    h.ok(Command::Rack(RackCommand::AddChain {
        id: chain,
        rack,
        name: None,
        before: None,
    }));
    let synth: DeviceId = h.id();
    h.ok(Command::Rack(RackCommand::InsertDevice {
        id: synth,
        chain,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::new(BuiltinDeviceType::Synth),
        },
        before: None,
    }));
    // Macro 12 (index 11): param 12, after the chain selector.
    h.ok(Command::Device(DeviceCommand::SetParam {
        device: rack,
        param: ParamId(12),
        value: 0.75,
    }));
    let id: ModMappingId = h.id();
    let param = ether_devices::descriptor(BuiltinDeviceType::Synth)
        .params
        .iter()
        .find(|p| p.automatable)
        .unwrap()
        .id;
    h.ok(Command::Modulation(ModulationCommand::Map {
        id,
        source: ModSource::Macro { rack, index: 11 },
        device: synth,
        param,
        depth: 0.5,
    }));
    // Hidden macros still modulate (8 shown).
    assert_eq!(bank(&h, rack).visible, 8);
    let tr = graph_track(&mut h, t);
    let node = node_of(&h, rack);
    let m = tr
        .modulation
        .macros
        .iter()
        .find(|m| m.rack == node)
        .expect("the rack's macros");
    assert_eq!((m.first, m.upper), (0, 9));
    assert_eq!(m.values[11], 0.75);
    assert!(tr.modulation.mappings.iter().any(|x| x.source
        == ModSourceDesc::Macro {
            rack: node,
            index: 11
        }));
}

fn save(h: &mut Harness, device: DeviceId, name: &str) -> PresetInfo {
    match h.ok(Command::Preset(PresetCommand::Save {
        device,
        name: name.into(),
        meta: PresetMeta::default(),
        overwrite: false,
    })) {
        ReplyValue::Preset { preset } => preset,
        other => panic!("{other:?}"),
    }
}

#[test]
fn macro_banks_save_with_the_project_and_round_trip_through_presets() {
    for ty in RACKS {
        let mut h = harness();
        let t = track(&mut h);
        let rack = insert(&mut h, t, ty);
        rename(&mut h, rack, 0, "Tone");
        rename(&mut h, rack, 13, "Width");
        count(&mut h, rack, 14);
        h.ok(Command::Device(DeviceCommand::SetParam {
            device: rack,
            param: macro_param(ty, 13).unwrap(),
            value: 0.5,
        }));
        let saved = bank(&h, rack);

        // Saved and reopened as is.
        let doc = h.project().clone();
        h.ok(Command::Project(ProjectCommand::Save { autosave: false }));
        h.create_project("Other");
        h.ok(Command::Project(ProjectCommand::Open { id: doc.id }));
        assert_eq!(h.project(), &doc);

        // A preset keeps the bank and macro 14's value; loading it onto a fresh rack
        // restores them as one undo step.
        let info = save(&mut h, rack, "Bank");
        let t2 = track(&mut h);
        let fresh = insert(&mut h, t2, ty);
        let seed: RackChainId = h.id();
        let before = h.project().clone();
        h.ok(Command::Preset(PresetCommand::Load {
            device: fresh,
            preset: info.preset.clone(),
            seed: Some(seed),
        }));
        assert_eq!(bank(&h, fresh), saved, "{ty:?}");
        assert_eq!(
            h.project().devices[&fresh]
                .params
                .get(&macro_param(ty, 13).unwrap()),
            Some(&0.5)
        );
        undo(&mut h);
        assert_eq!(h.project(), &before);

        // A preset of a rack with the default bank resets a renamed rack's.
        let plain = insert(&mut h, t2, ty);
        let info = save(&mut h, plain, "Plain");
        let seed: RackChainId = h.id();
        h.ok(Command::Preset(PresetCommand::Load {
            device: rack,
            preset: info.preset,
            seed: Some(seed),
        }));
        assert_eq!(bank(&h, rack), RackMacros::default(), "{ty:?}");
    }
}

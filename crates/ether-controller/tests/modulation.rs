//! Modulation (`racks-modulation`, Bitwig-style): modulators inside devices, mappings with
//! depth, scope rules, compile into `TrackDesc::modulation` (bases, macros, sidechains),
//! one undo step each.

mod common;

use common::*;
use ether_core::graph::TrackDesc;
use ether_core::modulation::ModSourceDesc;
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

fn modulation(h: &mut Harness, c: ModulationCommand) -> ReplyValue {
    h.ok(Command::Modulation(c))
}

fn add_modulator(h: &mut Harness, device: DeviceId, kind: ModulatorKind) -> ModulatorId {
    let id: ModulatorId = h.id();
    modulation(
        h,
        ModulationCommand::AddModulator {
            id,
            device,
            kind,
            name: None,
        },
    );
    id
}

fn map(
    h: &mut Harness,
    source: ModSource,
    device: DeviceId,
    param: ParamId,
    depth: f64,
) -> Vec<ServerMessage> {
    let id: ModMappingId = h.id();
    h.send(Command::Modulation(ModulationCommand::Map {
        id,
        source,
        device,
        param,
        depth,
    }))
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
fn modulators_get_default_params_and_numbered_names() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Audio);
    let d = insert(&mut h, t, BuiltinDeviceType::Delay);
    let a = add_modulator(&mut h, d, ModulatorKind::Lfo);
    let b = add_modulator(&mut h, d, ModulatorKind::Lfo);
    let p = h.project();
    assert_eq!(p.modulators[&a].name, "LFO");
    assert_eq!(p.modulators[&b].name, "LFO 2");
    let desc = ether_devices::modulators::descriptor(ModulatorKind::Lfo);
    assert_eq!(p.modulators[&a].params.len(), desc.params.len());
    assert_eq!(
        p.modulators_of(d).iter().map(|m| m.id).collect::<Vec<_>>(),
        vec![a, b]
    );
    // Params clamp and snap.
    modulation(
        &mut h,
        ModulationCommand::SetModulatorParam {
            modulator: a,
            param: ParamId(1),
            value: 1000.0,
        },
    );
    assert_eq!(h.project().modulators[&a].params[&ParamId(1)], 40.0);
    modulation(
        &mut h,
        ModulationCommand::SetModulatorParam {
            modulator: a,
            param: ParamId(0),
            value: 2.4,
        },
    );
    assert_eq!(h.project().modulators[&a].params[&ParamId(0)], 2.0);
    modulation(
        &mut h,
        ModulationCommand::ResetModulatorParam {
            modulator: a,
            param: ParamId(1),
        },
    );
    assert_eq!(h.project().modulators[&a].params[&ParamId(1)], 1.0);
    modulation(
        &mut h,
        ModulationCommand::RenameModulator {
            id: a,
            name: "Wobble".into(),
        },
    );
    assert_eq!(h.project().modulators[&a].name, "Wobble");
}

#[test]
fn mappings_respect_scope_and_uniqueness() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
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
            device: BuiltinDevice::Synth,
        },
        before: None,
    }));
    let other = insert(&mut h, t, BuiltinDeviceType::Delay);
    let lfo = add_modulator(&mut h, rack, ModulatorKind::Lfo);
    let src = ModSource::Modulator { modulator: lfo };
    // A rack's modulator reaches its chain devices and its own selector...
    ok(&map(&mut h, src, synth, ParamId(0), 0.5));
    ok(&map(&mut h, src, rack, RACK_SELECTOR_PARAM, 0.5));
    // ...but not devices outside the rack, nor its macros.
    let before = h.project().clone();
    let out = map(&mut h, src, other, ParamId(0), 0.5);
    assert_ne!(err(&out).code, ErrorCode::Unsupported);
    let out = map(&mut h, src, rack, ParamId(0), 0.5);
    assert_ne!(err(&out).code, ErrorCode::Unsupported);
    // Duplicate (source, target).
    let out = map(&mut h, src, synth, ParamId(0), -0.5);
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    // Unknown param.
    let out = map(&mut h, src, synth, ParamId(999), 0.5);
    assert_eq!(err(&out).code, ErrorCode::NotFound);
    assert_eq!(h.project(), &before);
    // Macros reach chain devices, not other devices.
    ok(&map(
        &mut h,
        ModSource::Macro { rack, index: 7 },
        synth,
        ParamId(1),
        -1.0,
    ));
    let out = map(
        &mut h,
        ModSource::Macro { rack, index: 7 },
        other,
        ParamId(1),
        1.0,
    );
    assert_ne!(err(&out).code, ErrorCode::Unsupported);
    // Modulators can't live on chain devices.
    let id: ModulatorId = h.id();
    let out = h.send(Command::Modulation(ModulationCommand::AddModulator {
        id,
        device: synth,
        kind: ModulatorKind::Lfo,
        name: None,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
}

#[test]
fn depth_edits_clamp_and_undo() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Audio);
    let d = insert(&mut h, t, BuiltinDeviceType::Delay);
    let lfo = add_modulator(&mut h, d, ModulatorKind::Lfo);
    let id: ModMappingId = h.id();
    modulation(
        &mut h,
        ModulationCommand::Map {
            id,
            source: ModSource::Modulator { modulator: lfo },
            device: d,
            param: ParamId(0),
            depth: 3.0,
        },
    );
    assert_eq!(h.project().mod_mappings[&id].depth, 1.0);
    modulation(&mut h, ModulationCommand::SetDepth { id, depth: -0.25 });
    assert_eq!(h.project().mod_mappings[&id].depth, -0.25);
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(h.project().mod_mappings[&id].depth, 1.0);
    modulation(&mut h, ModulationCommand::Unmap { id });
    assert!(h.project().mod_mappings.is_empty());
    // Removing the modulator removes its mappings.
    let id2: ModMappingId = h.id();
    modulation(
        &mut h,
        ModulationCommand::Map {
            id: id2,
            source: ModSource::Modulator { modulator: lfo },
            device: d,
            param: ParamId(1),
            depth: 0.5,
        },
    );
    modulation(&mut h, ModulationCommand::RemoveModulator { id: lfo });
    assert!(h.project().mod_mappings.is_empty());
    assert!(h.project().modulators.is_empty());
}

#[test]
fn modulation_compiles_with_bases_macros_and_indices() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
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
            device: BuiltinDevice::Synth,
        },
        before: None,
    }));
    let env = add_modulator(&mut h, rack, ModulatorKind::Envelope);
    let lfo = add_modulator(&mut h, rack, ModulatorKind::Lfo);
    let info = ether_devices::descriptor(BuiltinDeviceType::Synth)
        .params
        .into_iter()
        .find(|p| p.labels.is_none() && p.step.is_none())
        .unwrap();
    h.ok(Command::Device(DeviceCommand::SetParam {
        device: synth,
        param: info.id,
        value: info.to_plain(0.25),
    }));
    ok(&map(
        &mut h,
        ModSource::Modulator { modulator: lfo },
        synth,
        info.id,
        0.5,
    ));
    ok(&map(
        &mut h,
        ModSource::Macro { rack, index: 3 },
        synth,
        info.id,
        -0.2,
    ));
    h.ok(Command::Device(DeviceCommand::SetParam {
        device: rack,
        param: ParamId(3),
        value: 0.75,
    }));
    let td = graph_track(&mut h, t);
    let m = &td.modulation;
    assert_eq!(m.modulators.len(), 2);
    assert_eq!(m.modulators[0].id, env);
    assert_eq!(m.modulators[1].id, lfo);
    assert_eq!(m.modulators[1].host, node_of(&h, rack));
    assert_eq!(
        m.modulators[1].params.len(),
        ether_devices::modulators::descriptor(ModulatorKind::Lfo)
            .params
            .len()
    );
    assert_eq!(m.mappings.len(), 2);
    for mm in &m.mappings {
        assert_eq!(mm.node, node_of(&h, synth));
        assert!((mm.base - 0.25).abs() < 1e-9, "{}", mm.base);
    }
    assert!(
        m.mappings
            .iter()
            .any(|mm| mm.source == ModSourceDesc::Modulator(1) && mm.depth == 0.5)
    );
    assert!(m.mappings.iter().any(|mm| mm.source
        == ModSourceDesc::Macro {
            rack: node_of(&h, rack),
            index: 3
        }));
    assert_eq!(m.macros.len(), 1);
    assert_eq!(m.macros[0].values[3], 0.75);
}

#[test]
fn follower_sidechain_validation() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Audio);
    let kick = track(&mut h, TrackKind::Audio);
    let d = insert(&mut h, t, BuiltinDeviceType::Delay);
    let lfo = add_modulator(&mut h, d, ModulatorKind::Lfo);
    let follower = add_modulator(&mut h, d, ModulatorKind::EnvelopeFollower);
    let out = h.send(Command::Modulation(ModulationCommand::SetSidechain {
        modulator: lfo,
        source: Some(kick),
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    let out = h.send(Command::Modulation(ModulationCommand::SetSidechain {
        modulator: follower,
        source: Some(t),
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    modulation(
        &mut h,
        ModulationCommand::SetSidechain {
            modulator: follower,
            source: Some(kick),
        },
    );
    let td = graph_track(&mut h, t);
    assert_eq!(td.modulation.modulators[1].sidechain, Some(kick));
    // Deleting the source track cuts it.
    h.ok(Command::Track(TrackCommand::Delete { id: kick }));
    assert_eq!(h.project().modulators[&follower].sidechain, None);
}

#[test]
fn keytrack_and_velocity_are_listed() {
    let mut h = Harness::with_project();
    match h.ok(Command::Modulation(ModulationCommand::ListModulatorKinds)) {
        ReplyValue::ModulatorKinds { kinds } => {
            assert_eq!(kinds.len(), 7);
            assert!(kinds.iter().any(|k| k.kind == ModulatorKind::Keytrack));
            assert!(kinds.iter().any(|k| k.kind == ModulatorKind::Velocity));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn modulator_param_drags_take_the_fast_path() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Audio);
    let d = insert(&mut h, t, BuiltinDeviceType::Delay);
    let lfo = add_modulator(&mut h, d, ModulatorKind::Lfo);
    h.tick();
    let publishes = h.ctl.bridge.publishes();
    modulation(
        &mut h,
        ModulationCommand::SetModulatorParam {
            modulator: lfo,
            param: ParamId(1),
            value: 7.5,
        },
    );
    h.tick();
    assert_eq!(h.ctl.bridge.publishes(), publishes, "no republish");
    assert!(h.ctl.bridge.param_changes().iter().any(|c| c.target
        == ether_core::ParamTarget::Modulator {
            modulator: lfo,
            param: ParamId(1)
        }
        && c.value == 7.5));
}

#[test]
fn plugin_echoes_of_modulated_params_are_ignored() {
    use ether_core::plugin::PluginNotification;
    use ether_core::protocol::devices::DeviceCategory;
    use std::collections::BTreeMap;
    let bridge = FakeBridge {
        plugins: Some(BTreeMap::from([(
            "com.test.Verb".to_string(),
            plugin_descriptor("Verb", DeviceCategory::AudioEffect),
        )])),
        ..Default::default()
    };
    let mut h = Harness::with(bridge, Default::default(), Default::default());
    h.create_project("Plugins");
    let t = track(&mut h, TrackKind::Audio);
    let d: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id: d,
        track: t,
        device: DeviceSpec::Plugin {
            plugin_id: "com.test.Verb".into(),
            sandboxed: None,
            format: None,
        },
        before: None,
    }));
    let lfo = add_modulator(&mut h, d, ModulatorKind::Lfo);
    ok(&map(
        &mut h,
        ModSource::Modulator { modulator: lfo },
        d,
        ParamId(7),
        0.5,
    ));
    h.tick();
    let base = h.project().devices[&d].params.get(&ParamId(7)).copied();
    h.ctl.bridge.plugin_notes.push((
        d,
        PluginNotification::ParamEdited {
            param: ParamId(7),
            value: 91.0,
        },
    ));
    h.tick();
    assert_eq!(
        h.project().devices[&d].params.get(&ParamId(7)).copied(),
        base
    );
}

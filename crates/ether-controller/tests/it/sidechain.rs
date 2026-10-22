//! `DeviceCommand::SetSidechain` (sidechain node): validation (cycles, source kinds, devices
//! without a sidechain input, same-track taps), one undo step, compilation into the render
//! desc (`ChainEntry::sidechain` at any depth, `TrackDesc::sidechain_taps`), the source-track
//! and source-chain delete cascades, and save/reopen.

use crate::common;

use common::*;
use ether_controller::store::ProjectStore;
use ether_core::protocol::devices::{DeviceCommand, DeviceSpec};
use ether_core::protocol::mixer::MixerCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::project::{EditCommand, ProjectCommand};
use ether_core::protocol::racks::RackCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::{Command, ErrorCode};
use ether_core::{ScPoint, SidechainTapDesc};

fn track(h: &mut Harness, kind: TrackKind) -> TrackId {
    let id = h.id();
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

fn device(h: &mut Harness, track: TrackId, ty: BuiltinDeviceType) -> DeviceId {
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

fn set(device: DeviceId, source: Option<TrackId>) -> Command {
    Command::Device(DeviceCommand::SetSidechain {
        tap: None,
        device,
        source,
    })
}

fn edit(h: &mut Harness, c: EditCommand) {
    h.ok(Command::Edit(c));
}

/// The compiled sidechain of `device`'s chain entry in the last published graph.
fn compiled(h: &mut Harness, track: TrackId, index: usize) -> Option<TrackId> {
    h.tick();
    let g = h.ctl.bridge.last_graph();
    g.tracks.iter().find(|t| t.id == track).unwrap().chain[index].sidechain
}

fn master(h: &Harness) -> TrackId {
    h.project()
        .tracks
        .values()
        .find(|t| t.kind == TrackKind::Master)
        .unwrap()
        .id
}

#[test]
fn set_compiles_and_undoes_as_one_step() {
    let mut h = Harness::with_project();
    let kick = track(&mut h, TrackKind::Audio);
    let pad = track(&mut h, TrackKind::Midi);
    let comp = device(&mut h, pad, BuiltinDeviceType::Compressor);
    let lim = device(&mut h, pad, BuiltinDeviceType::Limiter);
    assert_eq!(compiled(&mut h, pad, 0), None);

    let out = h.send(set(comp, Some(kick)));
    ok(&out);
    assert_eq!(patches(&out).len(), 1, "one patch");
    assert_eq!(h.project().devices[&comp].sidechain, Some(kick));
    assert_eq!(compiled(&mut h, pad, 0), Some(kick));
    assert_eq!(compiled(&mut h, pad, 1), None);

    // The limiter has a sidechain input too.
    h.ok(set(lim, Some(kick)));
    assert_eq!(compiled(&mut h, pad, 1), Some(kick));

    // Setting the current value again changes nothing (no undo step).
    let out = h.send(set(comp, Some(kick)));
    ok(&out);
    assert!(patches(&out).is_empty());

    edit(&mut h, EditCommand::Undo);
    assert_eq!(h.project().devices[&lim].sidechain, None);
    assert_eq!(h.project().devices[&comp].sidechain, Some(kick));
    edit(&mut h, EditCommand::Undo);
    assert_eq!(h.project().devices[&comp].sidechain, None);
    assert_eq!(compiled(&mut h, pad, 0), None);
    edit(&mut h, EditCommand::Redo);
    assert_eq!(compiled(&mut h, pad, 0), Some(kick));

    // Clearing.
    h.ok(set(comp, None));
    assert_eq!(compiled(&mut h, pad, 0), None);
    edit(&mut h, EditCommand::Undo);
    assert_eq!(h.project().devices[&comp].sidechain, Some(kick));
}

#[test]
fn returns_and_groups_can_be_sources() {
    let mut h = Harness::with_project();
    let ret = track(&mut h, TrackKind::Return);
    let group = track(&mut h, TrackKind::Group);
    let bass = track(&mut h, TrackKind::Audio);
    let comp = device(&mut h, bass, BuiltinDeviceType::Compressor);
    h.ok(set(comp, Some(ret)));
    h.ok(set(comp, Some(group)));
    assert_eq!(compiled(&mut h, bass, 0), Some(group));
    // A sidechain on the master track listening to a regular track is fine too.
    let m = master(&h);
    let mcomp = device(&mut h, m, BuiltinDeviceType::Compressor);
    h.ok(set(mcomp, Some(bass)));
}

#[test]
fn rejects_invalid_sources_and_devices_without_changes() {
    let mut h = Harness::with_project();
    let kick = track(&mut h, TrackKind::Audio);
    let bass = track(&mut h, TrackKind::Audio);
    let comp = device(&mut h, bass, BuiltinDeviceType::Compressor);
    let delay = device(&mut h, bass, BuiltinDeviceType::Delay);
    let m = master(&h);
    let missing_track: TrackId = h.id();
    let missing_device: DeviceId = h.id();
    let before = h.project().clone();

    let cases = [
        (set(comp, Some(bass)), ErrorCode::InvalidArgument), // own track
        (set(comp, Some(m)), ErrorCode::InvalidArgument),    // master
        (set(delay, Some(kick)), ErrorCode::InvalidArgument), // no sidechain input
        (set(comp, Some(missing_track)), ErrorCode::NotFound),
        (set(missing_device, Some(kick)), ErrorCode::NotFound),
    ];
    for (c, code) in cases {
        let out = h.send(c.clone());
        assert_eq!(err(&out).code, code, "{c:?}");
        assert!(patches(&out).is_empty(), "{c:?}");
    }
    assert_eq!(h.project(), &before);
    // Clearing a device without an input is harmless.
    h.ok(set(delay, None));
}

#[test]
fn rejects_routing_cycles() {
    let mut h = Harness::with_project();
    let a = track(&mut h, TrackKind::Audio);
    let b = track(&mut h, TrackKind::Audio);
    let ret = track(&mut h, TrackKind::Return);
    let comp_b = device(&mut h, b, BuiltinDeviceType::Compressor);
    let comp_a = device(&mut h, a, BuiltinDeviceType::Compressor);
    let comp_ret = device(&mut h, ret, BuiltinDeviceType::Compressor);

    // a → b (sidechain); b → a would close a cycle.
    h.ok(set(comp_b, Some(a)));
    let out = h.send(set(comp_a, Some(b)));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);

    // a sends to the return: the return listening to a is fine (same direction)...
    let send: SendId = h.id();
    h.ok(Command::Mixer(MixerCommand::CreateSend {
        id: send,
        from: a,
        to: ret,
        level: Decibels(0.0),
        pre_fader: false,
    }));
    h.ok(set(comp_ret, Some(a)));
    // ...but a listening to the return (which a feeds) is a cycle.
    let out = h.send(set(comp_a, Some(ret)));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);

    // A group may listen to its own child (the child is rendered first anyway), but the
    // child can't listen to its group (the group sums the child).
    let group = track(&mut h, TrackKind::Group);
    let child: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id: child,
        kind: TrackKind::Audio,
        name: None,
        color: None,
        parent: Some(group),
        before: None,
    }));
    let comp_g = device(&mut h, group, BuiltinDeviceType::Compressor);
    let comp_child = device(&mut h, child, BuiltinDeviceType::Compressor);
    h.ok(set(comp_g, Some(child)));
    let out = h.send(set(comp_child, Some(group)));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);

    // And an existing sidechain edge blocks a later send that would close a cycle.
    let ret2 = track(&mut h, TrackKind::Return);
    let c = track(&mut h, TrackKind::Audio);
    let comp_c = device(&mut h, c, BuiltinDeviceType::Compressor);
    h.ok(set(comp_c, Some(ret2))); // ret2 → c
    let send2: SendId = h.id();
    let out = h.send(Command::Mixer(MixerCommand::CreateSend {
        id: send2,
        from: c,
        to: ret2,
        level: Decibels(0.0),
        pre_fader: false,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
}

#[test]
fn deleting_the_source_cuts_the_sidechain_in_one_undo_step() {
    let mut h = Harness::with_project();
    let kick = track(&mut h, TrackKind::Audio);
    let bass = track(&mut h, TrackKind::Audio);
    let comp = device(&mut h, bass, BuiltinDeviceType::Compressor);
    h.ok(set(comp, Some(kick)));
    h.ok(Command::Track(TrackCommand::Delete { id: kick }));
    assert_eq!(h.project().devices[&comp].sidechain, None);
    assert_eq!(compiled(&mut h, bass, 0), None);
    edit(&mut h, EditCommand::Undo);
    assert_eq!(h.project().devices[&comp].sidechain, Some(kick));
    assert_eq!(compiled(&mut h, bass, 0), Some(kick));
}

#[test]
fn survives_save_and_reopen() {
    let mut h = Harness::with_project();
    let kick = track(&mut h, TrackKind::Audio);
    let bass = track(&mut h, TrackKind::Audio);
    let comp = device(&mut h, bass, BuiltinDeviceType::Compressor);
    h.ok(set(comp, Some(kick)));
    h.ok(Command::Project(ProjectCommand::Save { autosave: false }));
    let doc = h.project().clone();
    h.create_project("Other");
    h.ok(Command::Project(ProjectCommand::Open { id: doc.id }));
    assert_eq!(h.project(), &doc);
    assert_eq!(h.project().devices[&comp].sidechain, Some(kick));
    assert_eq!(compiled(&mut h, bass, 0), Some(kick));
}

#[test]
fn pad_devices_take_a_sidechain() {
    // Build a rack with a pad chain through model ops (drum-rack commands are another
    // node's), save it to the store and open it.
    let mut h = Harness::new();
    let mut ids = IdGen::new(5);
    let mut p = Project::new(&mut ids, T0);
    let ins = |p: &mut Project, e: Entity| p.apply(&Op::Insert { entity: e }).unwrap();
    let mk_track = |ids: &mut IdGen, order: &str| Track {
        freeze: None,
        vca: Default::default(),
        id: ids.next(T0),
        kind: TrackKind::Midi,
        name: "t".into(),
        color: Color(0),
        order: OrderKey(order.into()),
        parent: None,
        mixer: TrackMixer::default(),
        input: TrackInput::None,
        output: TrackOutput::Default,
        monitor: MonitorMode::Auto,
        scale: Default::default(),
        mpe: None,
    };
    let kick = mk_track(&mut ids, "a1");
    let drums = mk_track(&mut ids, "a2");
    let (kick_id, drums_id) = (kick.id, drums.id);
    ins(&mut p, Entity::Track(kick));
    ins(&mut p, Entity::Track(drums));
    let mk_dev =
        |ids: &mut IdGen, device: BuiltinDevice, pad: Option<DrumPadId>, order: &str| Device {
            sidechain_tap: None,
            chain: None,
            id: ids.next(T0),
            track: drums_id,
            order: OrderKey(order.into()),
            name: "d".into(),
            enabled: true,
            kind: DeviceKind::Builtin { device },
            params: Default::default(),
            sidechain: None,
            pad,
        };
    let rack = mk_dev(
        &mut ids,
        BuiltinDevice::DrumRack { macros: None },
        None,
        "a0",
    );
    let rack_id = rack.id;
    ins(&mut p, Entity::Device(rack));
    let pad_id: DrumPadId = ids.next(T0);
    ins(
        &mut p,
        Entity::DrumPad(DrumPad {
            id: pad_id,
            rack: rack_id,
            note: 36,
            name: "pad".into(),
            color: None,
            choke_group: None,
            volume: Decibels::UNITY,
            pan: Pan(0.0),
            mute: false,
        }),
    );
    let pad_comp = mk_dev(&mut ids, BuiltinDevice::Compressor, Some(pad_id), "a0");
    let pad_comp_id = pad_comp.id;
    ins(&mut p, Entity::Device(pad_comp));
    h.ctl.store.create(p.id).unwrap();
    h.ctl
        .store
        .save(p.id, &file::save(&p, "test").unwrap())
        .unwrap();
    h.ok(Command::Project(ProjectCommand::Open { id: p.id }));

    h.ok(set(pad_comp_id, Some(kick_id)));
    assert_eq!(h.project().devices[&pad_comp_id].sidechain, Some(kick_id));
    // Compiled into the pad chain, post-fader (no tap listed).
    h.tick();
    let g = h.ctl.bridge.last_graph();
    let t = g.tracks.iter().find(|t| t.id == drums_id).unwrap();
    assert_eq!(t.racks[0].pads[0].chain[0].sidechain, Some(kick_id));
    assert!(t.sidechain_taps.is_empty());
    // Its own track's post-fader output would feed itself; its pre-FX signal is fine.
    let out = h.send(set(pad_comp_id, Some(drums_id)));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    h.ok(Command::Device(DeviceCommand::SetSidechain {
        device: pad_comp_id,
        source: Some(drums_id),
        tap: Some(SidechainTap {
            point: InputTap::PreFx,
            chain: None,
        }),
    }));
    // Undo: back to the kick, post-fader (one step).
    edit(&mut h, EditCommand::Undo);
    let d = &h.project().devices[&pad_comp_id];
    assert_eq!((d.sidechain, d.sidechain_tap), (Some(kick_id), None));
}

/// The node of the first device of the first chain of the first rack of `track`.
fn chain_node(h: &mut Harness, track: TrackId) -> ether_core::NodeKey {
    h.tick();
    let g = h.ctl.bridge.last_graph();
    let t = g.tracks.iter().find(|t| t.id == track).unwrap();
    t.chain_racks[0].chains[0].chain[0].node
}

#[test]
fn chain_taps_compile_to_engine_points_and_go_with_the_chain() {
    let mut h = Harness::with_project();
    let kick = track(&mut h, TrackKind::Midi);
    let bass = track(&mut h, TrackKind::Midi);
    // Kick: Instrument Rack → "Kick SC" chain → Synth.
    let rack = device(&mut h, kick, BuiltinDeviceType::InstrumentRack);
    let chain: RackChainId = h.id();
    h.ok(Command::Rack(RackCommand::AddChain {
        id: chain,
        rack,
        name: Some("Kick SC".into()),
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
    // Bass: Audio Effect Rack → chain → Compressor (nested consumer).
    let fx = device(&mut h, bass, BuiltinDeviceType::AudioEffectRack);
    // (An audio effect rack comes with its first chain.)
    let fx_chain = h.project().chains_of(fx)[0].id;
    let comp: DeviceId = h.id();
    h.ok(Command::Rack(RackCommand::InsertDevice {
        id: comp,
        chain: fx_chain,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::new(BuiltinDeviceType::Compressor),
        },
        before: None,
    }));
    let at = |point| SidechainTap {
        point,
        chain: Some(SidechainChain::Rack { chain }),
    };
    h.ok(Command::Device(DeviceCommand::SetSidechain {
        device: comp,
        source: Some(kick),
        tap: Some(at(InputTap::PreFx)),
    }));
    // Pre FX of a chain with an instrument = that instrument's output.
    let synth_node = chain_node(&mut h, kick);
    let comp_node = chain_node(&mut h, bass);
    let taps = |h: &mut Harness| {
        h.tick();
        let g = h.ctl.bridge.last_graph();
        let t = g.tracks.iter().find(|t| t.id == bass).unwrap();
        assert_eq!(t.chain_racks[0].chains[0].chain[0].sidechain, Some(kick));
        t.sidechain_taps.clone()
    };
    assert_eq!(
        taps(&mut h),
        vec![SidechainTapDesc {
            node: comp_node,
            point: ScPoint::After(synth_node),
        }]
    );
    h.ok(Command::Device(DeviceCommand::SetSidechain {
        device: comp,
        source: Some(kick),
        tap: Some(at(InputTap::PostFader)),
    }));
    assert_eq!(taps(&mut h)[0].point, ScPoint::ChainMix(chain.0.0));
    // A chain of the device's own track it sits in: rejected.
    let out = h.send(Command::Device(DeviceCommand::SetSidechain {
        device: comp,
        source: Some(bass),
        tap: Some(SidechainTap {
            point: InputTap::PostFx,
            chain: Some(SidechainChain::Rack { chain: fx_chain }),
        }),
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    // Removing the tapped chain cuts the sidechain; undo restores both.
    h.ok(Command::Rack(RackCommand::RemoveChain { id: chain }));
    let d = &h.project().devices[&comp];
    assert_eq!((d.sidechain, d.sidechain_tap), (None, None));
    edit(&mut h, EditCommand::Undo);
    let d = &h.project().devices[&comp];
    assert_eq!(
        (d.sidechain, d.sidechain_tap),
        (Some(kick), Some(at(InputTap::PostFader)))
    );
    // Deleting the source track cuts it too (its chains go with it).
    h.ok(Command::Track(TrackCommand::Delete { id: kick }));
    let d = &h.project().devices[&comp];
    assert_eq!((d.sidechain, d.sidechain_tap), (None, None));
}

#[test]
fn a_chain_keys_another_chain_of_its_rack_which_runs_first() {
    let mut h = Harness::with_project();
    let bass = track(&mut h, TrackKind::Audio);
    // Audio Effect Rack: its first chain holds the compressor, a second chain "Key" a gate.
    let fx = device(&mut h, bass, BuiltinDeviceType::AudioEffectRack);
    let first = h.project().chains_of(fx)[0].id;
    let key: RackChainId = h.id();
    h.ok(Command::Rack(RackCommand::AddChain {
        id: key,
        rack: fx,
        name: Some("Key".into()),
        before: None,
    }));
    let insert = |h: &mut Harness, chain, kind| {
        let id: DeviceId = h.id();
        h.ok(Command::Rack(RackCommand::InsertDevice {
            id,
            chain,
            device: DeviceSpec::Builtin {
                device: BuiltinDevice::new(kind),
            },
            before: None,
        }));
        id
    };
    let comp = insert(&mut h, first, BuiltinDeviceType::Compressor);
    let gate = insert(&mut h, key, BuiltinDeviceType::Gate);
    let on = |chain, point| SidechainTap {
        point,
        chain: Some(SidechainChain::Rack { chain }),
    };
    let run = |h: &mut Harness| {
        h.tick();
        let g = h.ctl.bridge.last_graph();
        let t = g.tracks.iter().find(|t| t.id == bass).unwrap();
        let order: Vec<RackChainId> = t.chain_racks[0].chains.iter().map(|c| c.id).collect();
        (order, t.sidechain_taps.clone())
    };
    // Before: the chains in their own order.
    assert_eq!(run(&mut h).0, vec![first, key]);
    h.ok(Command::Device(DeviceCommand::SetSidechain {
        device: comp,
        source: Some(bass),
        tap: Some(on(key, InputTap::PostFx)),
    }));
    // The keyed chain runs first; post-FX is its output before the rack's alignment.
    let (order, taps) = run(&mut h);
    assert_eq!(order, vec![key, first]);
    assert_eq!(taps.len(), 1);
    assert_eq!(taps[0].point, ScPoint::ChainRaw(key.0.0));
    // Pre-FX: the chain's input.
    h.ok(Command::Device(DeviceCommand::SetSidechain {
        device: comp,
        source: Some(bass),
        tap: Some(on(key, InputTap::PreFx)),
    }));
    assert_eq!(run(&mut h).1[0].point, ScPoint::ChainIn(key.0.0));
    // Post-fader (after the chain's volume, aligned with every chain): not for a sibling.
    let out = h.send(Command::Device(DeviceCommand::SetSidechain {
        device: comp,
        source: Some(bass),
        tap: Some(on(key, InputTap::PostFader)),
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    // The gate keying the compressor's chain back would loop: rejected.
    let out = h.send(Command::Device(DeviceCommand::SetSidechain {
        device: gate,
        source: Some(bass),
        tap: Some(on(first, InputTap::PostFx)),
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    // Unkeyed: back to the chains' own order.
    h.ok(Command::Device(DeviceCommand::SetSidechain {
        device: comp,
        source: None,
        tap: None,
    }));
    assert_eq!(run(&mut h).0, vec![first, key]);
}

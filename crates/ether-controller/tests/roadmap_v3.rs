//! v0.2 routing (contracts-3): every new command reaches its node's module and, until the
//! node implements it, replies `Unsupported` without changing the document; new built-ins
//! insert and compile as placeholders.
//!
//! **One test per node**: each node deletes (or rewrites into real behaviour tests) only
//! its own function, so parallel nodes never conflict here. Keep `assert_unsupported`.

mod common;

use common::*;
use ether_core::protocol::analysis::AnalysisCommand;
use ether_core::protocol::devices::{DeviceCommand, DeviceSpec};
use ether_core::protocol::media::{MediaCommand, MediaSource};
use ether_core::protocol::model::*;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::{Command, ErrorCode, ReplyValue};

/// Sends `c` and asserts it replies `Unsupported` without changing the document.
fn assert_unsupported(h: &mut Harness, c: Command) {
    let before = h.project().clone();
    let out = h.send(c.clone());
    assert_eq!(err(&out).code, ErrorCode::Unsupported, "{c:?}");
    assert!(patches(&out).is_empty(), "{c:?}");
    assert_eq!(h.project(), &before, "{c:?}");
}

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

/// Inserts every type of a device group on a fitting track; they compile into the chain.
fn group_inserts_and_compiles(types: &[BuiltinDeviceType]) {
    let mut h = Harness::with_project();
    let midi = track(&mut h, TrackKind::Midi);
    for &ty in types {
        let d = insert(&mut h, midi, ty);
        let dev = &h.project().devices[&d];
        assert_eq!(
            dev.kind,
            DeviceKind::Builtin {
                device: BuiltinDevice::new(ty)
            }
        );
        assert_eq!(dev.chain, None);
        // Every param starts at its descriptor default.
        let desc = ether_devices::descriptor(ty);
        assert_eq!(dev.params.len(), desc.params.len(), "{ty:?}");
    }
    h.tick();
    let graph = h.ctl.bridge.last_graph();
    let t = graph.tracks.iter().find(|t| t.id == midi).unwrap();
    assert_eq!(t.chain.len(), types.len());
}

// ─── device groups (placeholders until each node lands) ─────────────────────────────────

#[test]
fn synth_2_poly_synth_inserts_compiles_and_keeps_its_params() {
    group_inserts_and_compiles(&[BuiltinDeviceType::PolySynth]);
    // The real device (not a placeholder) reports the document's param values.
    let mut dev = ether_devices::create(
        &BuiltinDevice::new(BuiltinDeviceType::PolySynth),
        &ether_devices::NoSamples,
    );
    let cutoff = ether_devices::poly_synth::poly_synth::CUTOFF;
    dev.set_param(cutoff, 1234.0);
    assert_eq!(dev.param(cutoff), Some(1234.0));
    assert_eq!(dev.channels(), (0, 2));
}

#[test]
fn multisampler_is_a_placeholder() {
    group_inserts_and_compiles(&[BuiltinDeviceType::MultiSampler]);
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
    let d = insert(&mut h, t, BuiltinDeviceType::MultiSampler);
    assert_unsupported(
        &mut h,
        Command::Device(DeviceCommand::SetZones {
            device: d,
            zones: vec![SampleZone::default()],
        }),
    );
}

#[test]
fn fx_color_devices_insert_with_layouts_and_an_auto_filter_sidechain() {
    group_inserts_and_compiles(&[
        BuiltinDeviceType::Saturator,
        BuiltinDeviceType::Bitcrusher,
        BuiltinDeviceType::AutoFilter,
    ]);
    let mut h = Harness::with_project();
    let kick = track(&mut h, TrackKind::Audio);
    let pad = track(&mut h, TrackKind::Audio);
    let sat = insert(&mut h, pad, BuiltinDeviceType::Saturator);
    let crush = insert(&mut h, pad, BuiltinDeviceType::Bitcrusher);
    let filter = insert(&mut h, pad, BuiltinDeviceType::AutoFilter);
    for d in [sat, crush, filter] {
        let ReplyValue::Descriptor { descriptor } =
            h.ok(Command::Device(DeviceCommand::GetDescriptor { device: d }))
        else {
            panic!("descriptor reply");
        };
        assert!(descriptor.layout.is_some(), "{:?}", descriptor.name);
    }
    // The auto filter's envelope follower keys from a sidechain; the others have none.
    h.ok(Command::Device(DeviceCommand::SetSidechain {
        device: filter,
        source: Some(kick),
    }));
    assert_eq!(h.project().devices[&filter].sidechain, Some(kick));
    for d in [sat, crush] {
        let out = h.send(Command::Device(DeviceCommand::SetSidechain {
            device: d,
            source: Some(kick),
        }));
        assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    }
    h.tick();
    let graph = h.ctl.bridge.last_graph();
    let t = graph.tracks.iter().find(|t| t.id == pad).unwrap();
    assert_eq!(t.chain.len(), 3);
    assert!(t.chain[0].sidechain.is_none() && t.chain[1].sidechain.is_none());
    assert!(t.chain[2].sidechain.is_some());
}

#[test]
fn fx_modulation_devices_insert_with_layouts() {
    let types = [
        BuiltinDeviceType::Chorus,
        BuiltinDeviceType::Phaser,
        BuiltinDeviceType::Flanger,
        BuiltinDeviceType::Tremolo,
    ];
    group_inserts_and_compiles(&types);
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Audio);
    let ids: Vec<DeviceId> = types.iter().map(|&ty| insert(&mut h, t, ty)).collect();
    for &d in &ids {
        let ReplyValue::Descriptor { descriptor } =
            h.ok(Command::Device(DeviceCommand::GetDescriptor { device: d }))
        else {
            panic!("descriptor reply");
        };
        assert!(descriptor.layout.is_some(), "{:?}", descriptor.name);
        assert!(!ether_devices::factory_presets(descriptor_type(&h, d)).is_empty());
    }
    // The appended Through Zero toggle (param 9) is a regular, undoable param.
    let flanger = ids[2];
    let tz = ether_devices::fx_modulation::flanger::THROUGH_ZERO;
    h.ok(Command::Device(DeviceCommand::SetParam {
        device: flanger,
        param: tz,
        value: 1.0,
    }));
    assert_eq!(h.project().devices[&flanger].params.get(&tz), Some(&1.0));
    h.tick();
    let graph = h.ctl.bridge.last_graph();
    let track = graph.tracks.iter().find(|x| x.id == t).unwrap();
    assert_eq!(track.chain.len(), 4);
}

fn descriptor_type(h: &Harness, d: DeviceId) -> BuiltinDeviceType {
    match &h.project().devices[&d].kind {
        DeviceKind::Builtin { device } => device.device_type(),
        other => panic!("{other:?}"),
    }
}

/// fx-dynamics: the three devices insert and compile with their layouts; Gate and
/// Multiband Compressor take a sidechain source (compiled into the chain entry), the
/// Transient Shaper refuses one; their gain-reduction meters can be watched.
#[test]
fn fx_dynamics_devices_insert_with_layouts_and_sidechains() {
    group_inserts_and_compiles(&[
        BuiltinDeviceType::Gate,
        BuiltinDeviceType::MultibandCompressor,
        BuiltinDeviceType::TransientShaper,
    ]);
    let mut h = Harness::with_project();
    let kick = track(&mut h, TrackKind::Audio);
    let pad = track(&mut h, TrackKind::Audio);
    let gate = insert(&mut h, pad, BuiltinDeviceType::Gate);
    let mbc = insert(&mut h, pad, BuiltinDeviceType::MultibandCompressor);
    let shaper = insert(&mut h, pad, BuiltinDeviceType::TransientShaper);
    for d in [gate, mbc, shaper] {
        let ReplyValue::Descriptor { descriptor } =
            h.ok(Command::Device(DeviceCommand::GetDescriptor { device: d }))
        else {
            panic!("descriptor reply");
        };
        assert!(descriptor.layout.is_some(), "{:?}", descriptor.name);
    }
    for d in [gate, mbc] {
        h.ok(Command::Device(DeviceCommand::SetSidechain {
            device: d,
            source: Some(kick),
        }));
        assert_eq!(h.project().devices[&d].sidechain, Some(kick));
    }
    let out = h.send(Command::Device(DeviceCommand::SetSidechain {
        device: shaper,
        source: Some(kick),
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    h.tick();
    let graph = h.ctl.bridge.last_graph();
    let t = graph.tracks.iter().find(|t| t.id == pad).unwrap();
    assert_eq!(t.chain.len(), 3);
    assert!(t.chain[0].sidechain.is_some() && t.chain[1].sidechain.is_some());
    assert!(t.chain[2].sidechain.is_none());
    // The meters are watchable (the renderer's Meter widgets watch while mounted).
    assert_eq!(
        h.ok(Command::Analysis(AnalysisCommand::Watch { device: gate })),
        ReplyValue::Unit
    );
    h.tick();
    assert!(h.ctl.bridge.analysis_watches.iter().any(|(_, on)| *on));
}

#[test]
fn fx_analysis_devices_insert_and_watches_reach_the_engine() {
    group_inserts_and_compiles(&[
        BuiltinDeviceType::SpectrumAnalyzer,
        BuiltinDeviceType::Tuner,
    ]);
    // Watch/Unwatch are refcounted and reach the engine (frames: tests/analysis_devices.rs).
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Audio);
    let d = insert(&mut h, t, BuiltinDeviceType::SpectrumAnalyzer);
    assert_eq!(
        h.ok(Command::Analysis(AnalysisCommand::Watch { device: d })),
        ReplyValue::Unit
    );
    assert_eq!(
        h.ok(Command::Analysis(AnalysisCommand::Unwatch { device: d })),
        ReplyValue::Unit
    );
    // Watches reach the engine (refcounted; a failed push is retried next tick).
    h.tick();
    let key = h
        .ctl
        .bridge
        .live
        .iter()
        .find(|(_, dev)| **dev == d)
        .map(|(k, _)| *k)
        .unwrap();
    h.ctl.bridge.fail_watches = true;
    h.ok(Command::Analysis(AnalysisCommand::Watch { device: d }));
    h.ok(Command::Analysis(AnalysisCommand::Watch { device: d }));
    h.tick();
    assert!(h.ctl.bridge.analysis_watches.is_empty());
    h.ctl.bridge.fail_watches = false;
    h.tick();
    assert_eq!(h.ctl.bridge.analysis_watches, vec![(key, true)]);
    h.ok(Command::Analysis(AnalysisCommand::Unwatch { device: d }));
    h.tick();
    assert_eq!(h.ctl.bridge.analysis_watches.len(), 1, "still watched once");
    h.ok(Command::Analysis(AnalysisCommand::Unwatch { device: d }));
    h.tick();
    assert_eq!(h.ctl.bridge.analysis_watches.last(), Some(&(key, false)));
}

#[test]
fn midi_fx_devices_insert_with_layouts_before_the_instrument() {
    let types = [
        BuiltinDeviceType::Arpeggiator,
        BuiltinDeviceType::Chord,
        BuiltinDeviceType::ScaleQuantize,
        BuiltinDeviceType::NoteLength,
        BuiltinDeviceType::Velocity,
        BuiltinDeviceType::Randomizer,
    ];
    group_inserts_and_compiles(&types);
    // Real devices with panels; ordering and scale pushes: tests/midi_fx.rs.
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
    for ty in types {
        let d = insert(&mut h, t, ty);
        let ReplyValue::Descriptor { descriptor } =
            h.ok(Command::Device(DeviceCommand::GetDescriptor { device: d }))
        else {
            panic!("descriptor reply");
        };
        assert!(descriptor.layout.is_some(), "{ty:?}");
    }
    insert(&mut h, t, BuiltinDeviceType::Synth);
    let id: DeviceId = h.id();
    let out = h.send(Command::Device(DeviceCommand::Insert {
        id,
        track: t,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::new(BuiltinDeviceType::Arpeggiator),
        },
        before: None,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
}

// ─── feature nodes ──────────────────────────────────────────────────────────────────────

// presets: see tests/presets.rs. racks-modulation: see tests/racks.rs and tests/modulation.rs.

// browser-v2: see tests/browser.rs.

// media-references: see tests/media_refs.rs.

// groups-buses: see tests/groups.rs.

/// `file-import` landed: `Path` is validated first, then read through
/// `Library::read_external`; a host without OS files (web, this memory library) replies
/// `Unsupported` (native coverage: `ether-native/tests/file_import_e2e.rs`).
#[test]
fn file_import_path_needs_an_os_file_host() {
    let mut h = Harness::with_project();
    let path_import = |h: &mut Harness, path: &str| {
        let id: MediaId = h.id();
        Command::Media(MediaCommand::Import {
            id,
            source: MediaSource::Path { path: path.into() },
        })
    };
    let c = path_import(&mut h, "/Users/me/kick.wav");
    assert_unsupported(&mut h, c);
    for bad in ["kick.wav", "/Users/me/notes.txt"] {
        let c = path_import(&mut h, bad);
        assert_eq!(err(&h.send(c)).code, ErrorCode::InvalidArgument, "{bad}");
    }
}

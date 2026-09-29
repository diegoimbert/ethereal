//! v0.2 routing (contracts-3): every new command reaches its node's module and, until the
//! node implements it, replies `Unsupported` without changing the document; new built-ins
//! insert and compile as placeholders.
//!
//! **One test per node**: each node deletes (or rewrites into real behaviour tests) only
//! its own function, so parallel nodes never conflict here. Keep `assert_unsupported`.

mod common;

use common::*;
use ether_core::protocol::analysis::AnalysisCommand;
use ether_core::protocol::browser::{BrowserCommand, BrowserQuery, BrowserSort};
use ether_core::protocol::devices::{DeviceCommand, DeviceSpec};
use ether_core::protocol::media::{MediaCommand, MediaSource};
use ether_core::protocol::model::*;
use ether_core::protocol::racks::{ModulationCommand, RackCommand};
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
fn fx_color_devices_are_placeholders() {
    group_inserts_and_compiles(&[
        BuiltinDeviceType::Saturator,
        BuiltinDeviceType::Bitcrusher,
        BuiltinDeviceType::AutoFilter,
    ]);
}

#[test]
fn fx_modulation_devices_are_placeholders() {
    group_inserts_and_compiles(&[
        BuiltinDeviceType::Chorus,
        BuiltinDeviceType::Phaser,
        BuiltinDeviceType::Flanger,
        BuiltinDeviceType::Tremolo,
    ]);
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
fn midi_fx_devices_are_placeholders() {
    group_inserts_and_compiles(&[
        BuiltinDeviceType::Arpeggiator,
        BuiltinDeviceType::Chord,
        BuiltinDeviceType::ScaleQuantize,
        BuiltinDeviceType::NoteLength,
        BuiltinDeviceType::Velocity,
        BuiltinDeviceType::Randomizer,
    ]);
}

// ─── feature nodes ──────────────────────────────────────────────────────────────────────

#[test]
fn racks_modulation_reply_unsupported() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
    let rack = insert(&mut h, t, BuiltinDeviceType::InstrumentRack);
    insert(&mut h, t, BuiltinDeviceType::AudioEffectRack);
    let chain: RackChainId = h.id();
    assert_unsupported(
        &mut h,
        Command::Rack(RackCommand::AddChain {
            id: chain,
            rack,
            name: None,
            before: None,
        }),
    );
    let modulator: ModulatorId = h.id();
    assert_unsupported(
        &mut h,
        Command::Modulation(ModulationCommand::AddModulator {
            id: modulator,
            device: rack,
            kind: ModulatorKind::Lfo,
            name: None,
        }),
    );
    let mapping: ModMappingId = h.id();
    assert_unsupported(
        &mut h,
        Command::Modulation(ModulationCommand::Map {
            id: mapping,
            source: ModSource::Macro { rack, index: 0 },
            device: rack,
            param: RACK_SELECTOR_PARAM,
            depth: 0.5,
        }),
    );
    // Modulator kinds are listed from the frozen tables already.
    match h.ok(Command::Modulation(ModulationCommand::ListModulatorKinds)) {
        ReplyValue::ModulatorKinds { kinds } => assert_eq!(kinds.len(), ModulatorKind::ALL.len()),
        other => panic!("{other:?}"),
    }
    // Rack params: 8 macros + chain selector.
    assert_eq!(h.project().devices[&rack].params.len(), 9);
}

#[test]
fn browser_v2_replies_unsupported() {
    let mut h = Harness::with_project();
    assert_unsupported(
        &mut h,
        Command::Browser(BrowserCommand::Query {
            query: BrowserQuery {
                text: "kick".into(),
                kinds: vec![],
                tags: vec![],
                favourites_only: false,
                roots: vec![],
                folder: None,
                device: None,
                sort: BrowserSort::Name,
                offset: 0,
                limit: 50,
            },
        }),
    );
    assert_unsupported(&mut h, Command::Browser(BrowserCommand::ListRoots));
}

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

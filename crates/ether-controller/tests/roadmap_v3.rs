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
use ether_core::protocol::presets::{PresetCommand, PresetRef, PresetSource};
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
fn synth_2_poly_synth_is_a_placeholder() {
    group_inserts_and_compiles(&[BuiltinDeviceType::PolySynth]);
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

#[test]
fn fx_dynamics_devices_are_placeholders() {
    group_inserts_and_compiles(&[
        BuiltinDeviceType::Gate,
        BuiltinDeviceType::MultibandCompressor,
        BuiltinDeviceType::TransientShaper,
    ]);
}

#[test]
fn fx_analysis_devices_are_placeholders_and_watch_works() {
    group_inserts_and_compiles(&[
        BuiltinDeviceType::SpectrumAnalyzer,
        BuiltinDeviceType::Tuner,
    ]);
    // The analysis channel's watch commands are implemented (contracts-3).
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
fn presets_reply_unsupported() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
    let d = insert(&mut h, t, BuiltinDeviceType::PolySynth);
    let preset = PresetRef {
        source: PresetSource::User,
        id: "poly-synth/x.etherpreset".into(),
    };
    for c in [
        PresetCommand::List {
            device: None,
            text: None,
        },
        PresetCommand::Load {
            device: d,
            preset: preset.clone(),
        },
        PresetCommand::Save {
            device: d,
            name: "X".into(),
            meta: PresetMeta::default(),
            overwrite: false,
        },
        PresetCommand::Rename {
            preset: preset.clone(),
            name: "Y".into(),
        },
        PresetCommand::Delete {
            preset: preset.clone(),
        },
        PresetCommand::SetMeta {
            preset,
            meta: PresetMeta::default(),
        },
    ] {
        assert_unsupported(&mut h, Command::Preset(c));
    }
}

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
fn sample_accurate_automation_has_no_commands() {
    // Engine-only node: the API is `EventKind::Param` at an offset (already delivered) and
    // `ether_core::automation_rt` (v0.1 behaviour moved verbatim). The node replaces this
    // test with block-size-independence tests (ether-core/tests/sample_accurate*.rs).
    assert_eq!(ether_core::automation_rt::PARAM_GRID, 32);
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

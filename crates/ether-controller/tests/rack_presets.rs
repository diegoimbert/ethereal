//! Rack presets (v0.3, `rack-presets`; CONTRACTS.md §13.9): saving a rack stores its chains,
//! chain devices, modulators and the mappings inside it; `Preset::Load { seed }` replaces
//! them as one undo step with `derive_id(seed, i)` ids; v1 rack presets load unchanged.

mod common;

use std::collections::BTreeMap;

use common::*;
use ether_controller::memory::MemoryLibrary;
use ether_controller::store::Library;
use ether_core::protocol::devices::{DeviceCategory, DeviceCommand, DeviceSpec};
use ether_core::protocol::media::{BrowseLocation, MediaCommand, MediaSource};
use ether_core::protocol::model::*;
use ether_core::protocol::presets::{PresetCommand, PresetInfo, PresetRef, PresetSource};
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::racks::{ModulationCommand, RackCommand};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::{Command, ErrorCode, Event, ReplyValue, ServerMessage};

const USER: &str = "user";

fn harness_with(bridge: FakeBridge, library: MemoryLibrary) -> Harness {
    let mut h = Harness::with(bridge, library.with_user_root(USER), Default::default());
    h.create_project("Rack presets");
    h
}

fn harness() -> Harness {
    harness_with(FakeBridge::default(), MemoryLibrary::new())
}

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

fn add_chain(h: &mut Harness, rack: DeviceId, name: &str) -> RackChainId {
    let id: RackChainId = h.id();
    h.ok(Command::Rack(RackCommand::AddChain {
        id,
        rack,
        name: Some(name.into()),
        before: None,
    }));
    id
}

fn chain_insert(h: &mut Harness, chain: RackChainId, spec: DeviceSpec) -> DeviceId {
    let id: DeviceId = h.id();
    h.ok(Command::Rack(RackCommand::InsertDevice {
        id,
        chain,
        device: spec,
        before: None,
    }));
    id
}

fn builtin(ty: BuiltinDeviceType) -> DeviceSpec {
    DeviceSpec::Builtin {
        device: BuiltinDevice::new(ty),
    }
}

fn set_param(h: &mut Harness, device: DeviceId, id: u32, value: f64) {
    h.ok(Command::Device(DeviceCommand::SetParam {
        device,
        param: ParamId(id),
        value,
    }));
}

fn map(h: &mut Harness, source: ModSource, device: DeviceId, param: u32, depth: f64) {
    let id: ModMappingId = h.id();
    h.ok(Command::Modulation(ModulationCommand::Map {
        id,
        source,
        device,
        param: ParamId(param),
        depth,
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

fn load(h: &mut Harness, device: DeviceId, preset: PresetRef, seed: Option<RackChainId>) -> Vec<ServerMessage> {
    h.send(Command::Preset(PresetCommand::Load {
        device,
        preset,
        seed,
    }))
}

fn factory(id: &str) -> PresetRef {
    PresetRef {
        source: PresetSource::Factory,
        id: id.into(),
    }
}

fn undo(h: &mut Harness) {
    h.ok(Command::Edit(EditCommand::Undo));
}

fn redo(h: &mut Harness) {
    h.ok(Command::Edit(EditCommand::Redo));
}

fn warnings(out: &[ServerMessage]) -> Vec<String> {
    events(out)
        .into_iter()
        .filter_map(|e| match e {
            Event::Notification { message, .. } => Some(message),
            _ => None,
        })
        .collect()
}

type ChainShape = (String, Option<Color>, Decibels, Pan, bool, bool, Zone, Zone, Zone);

/// Everything a rack preset covers, with ids replaced by positions (comparable across racks).
#[derive(Debug, PartialEq)]
struct Shape {
    params: BTreeMap<ParamId, f64>,
    chains: Vec<(ChainShape, Vec<(String, bool, DeviceKind, BTreeMap<ParamId, f64>)>)>,
    modulators: Vec<(String, ModulatorKind, BTreeMap<ParamId, f64>)>,
    mappings: Vec<(String, String, ParamId, f64)>,
}

fn shape(p: &Project, rack: DeviceId) -> Shape {
    let mut names: BTreeMap<DeviceId, String> = BTreeMap::from([(rack, "rack".to_string())]);
    let chains = p
        .chains_of(rack)
        .into_iter()
        .enumerate()
        .map(|(ci, c)| {
            let devices = p
                .chain_devices_of(c.id)
                .into_iter()
                .enumerate()
                .map(|(di, d)| {
                    names.insert(d.id, format!("{ci}.{di}"));
                    (d.name.clone(), d.enabled, d.kind.clone(), d.params.clone())
                })
                .collect();
            let c = (
                c.name.clone(),
                c.color,
                c.volume,
                c.pan,
                c.mute,
                c.solo,
                c.keys,
                c.velocities,
                c.select,
            );
            (c, devices)
        })
        .collect();
    let mods = p.modulators_of(rack);
    let mod_name = |id: ModulatorId| {
        format!("mod{}", mods.iter().position(|m| m.id == id).expect("rack modulator"))
    };
    let mut mappings: Vec<_> = p
        .mod_mappings
        .values()
        .filter_map(|m| {
            let target = names.get(&m.device)?.clone();
            let source = match m.source {
                ModSource::Macro { rack: r, index } if r == rack => format!("macro{index}"),
                ModSource::Modulator { modulator } => mod_name(modulator),
                ModSource::Macro { .. } => return None,
            };
            Some((source, target, m.param, m.depth))
        })
        .collect();
    mappings.sort_by(|a, b| a.partial_cmp(b).unwrap());
    Shape {
        params: p.devices[&rack].params.clone(),
        chains,
        modulators: mods
            .iter()
            .map(|m| (m.name.clone(), m.kind, m.params.clone()))
            .collect(),
        mappings,
    }
}

/// An instrument rack with two chains (zones, mix, a disabled device, edited params), an LFO
/// on the rack, and macro + modulator mappings into the chains and onto the rack itself.
fn build_rack(h: &mut Harness, t: TrackId) -> DeviceId {
    let rack = insert(h, t, BuiltinDeviceType::InstrumentRack);
    set_param(h, rack, 0, 0.25);
    set_param(h, rack, 8, 12.0);
    let low = add_chain(h, rack, "Low");
    h.ok(Command::Rack(RackCommand::SetChainZones {
        id: low,
        keys: Some(Zone { lo: 0, hi: 59 }),
        velocities: None,
        select: Some(Zone { lo: 0, hi: 63 }),
    }));
    h.ok(Command::Rack(RackCommand::SetChainMix {
        id: low,
        volume: Some(Decibels(-6.0)),
        pan: Some(Pan(-0.5)),
        mute: None,
        solo: None,
    }));
    h.ok(Command::Rack(RackCommand::SetChainColor {
        id: low,
        color: Some(Color(0x3366ff)),
    }));
    let bass = chain_insert(h, low, builtin(BuiltinDeviceType::PolySynth));
    set_param(h, bass, 26, 500.0);
    let high = add_chain(h, rack, "High");
    h.ok(Command::Rack(RackCommand::SetChainZones {
        id: high,
        keys: Some(Zone { lo: 60, hi: 127 }),
        velocities: Some(Zone { lo: 20, hi: 127 }),
        select: None,
    }));
    let arp_free = chain_insert(h, high, builtin(BuiltinDeviceType::Synth));
    set_param(h, arp_free, 6, 2_000.0);
    let fx = chain_insert(h, high, builtin(BuiltinDeviceType::Chorus));
    h.ok(Command::Device(DeviceCommand::SetEnabled {
        id: fx,
        enabled: false,
    }));
    let lfo: ModulatorId = h.id();
    h.ok(Command::Modulation(ModulationCommand::AddModulator {
        id: lfo,
        device: rack,
        kind: ModulatorKind::Lfo,
        name: Some("Wobble".into()),
    }));
    h.ok(Command::Modulation(ModulationCommand::SetModulatorParam {
        modulator: lfo,
        param: ParamId(1),
        value: 3.5,
    }));
    map(h, ModSource::Macro { rack, index: 0 }, bass, 26, 0.4);
    map(h, ModSource::Macro { rack, index: 1 }, fx, 8, -0.3);
    map(h, ModSource::Modulator { modulator: lfo }, arp_free, 6, 0.2);
    map(h, ModSource::Modulator { modulator: lfo }, rack, 8, 0.1);
    rack
}

#[test]
fn saving_a_rack_stores_its_structure_and_loading_rebuilds_it() {
    let mut h = harness();
    let t = track(&mut h, TrackKind::Midi);
    let rack = build_rack(&mut h, t);
    let info = save(&mut h, rack, "Split Stack");
    let json = String::from_utf8(
        h.ctl
            .library
            .read(USER, &format!("Presets/{}", info.preset.id))
            .expect("written"),
    )
    .unwrap();
    let file = load_preset(&json).unwrap();
    assert!(json.contains("\"version\": 2"));
    let structure = file.rack.expect("rack structure");
    assert_eq!(structure.chains.len(), 2);
    assert_eq!(structure.chains[1].devices.len(), 2);
    assert_eq!(structure.modulators.len(), 1);
    assert_eq!(structure.mappings.len(), 4, "{:?}", structure.mappings);

    // Load onto an empty rack on another track: same structure, new ids from the seed.
    let t2 = track(&mut h, TrackKind::Midi);
    let rack2 = insert(&mut h, t2, BuiltinDeviceType::InstrumentRack);
    let before = h.project().clone();
    let seed: RackChainId = h.id();
    let out = load(&mut h, rack2, info.preset.clone(), Some(seed));
    ok(&out);
    assert!(warnings(&out).is_empty(), "{:?}", warnings(&out));
    assert_eq!(shape(h.project(), rack2), shape(&before, rack));

    // Ids: chains, then each chain's devices, then modulators, then mappings.
    let p = h.project();
    let chains = p.chains_of(rack2);
    let ids: Vec<u32> = (0..chains.len() as u32).collect();
    for (c, i) in chains.iter().zip(&ids) {
        assert_eq!(c.id, derive_id::<_, RackChainId>(seed, *i));
    }
    let mut i = 2;
    for c in &chains {
        for d in p.chain_devices_of(c.id) {
            assert_eq!(d.id, derive_id::<_, DeviceId>(seed, i));
            i += 1;
        }
    }
    assert_eq!(p.modulators_of(rack2)[0].id, derive_id::<_, ModulatorId>(seed, i));
    let mapping_ids: Vec<ModMappingId> = (i + 1..i + 5).map(|n| derive_id(seed, n)).collect();
    for id in &mapping_ids {
        assert!(p.mod_mappings.contains_key(id), "mapping {id}");
    }

    // One undo step back to the empty rack, and redo.
    let after = h.project().clone();
    undo(&mut h);
    assert_eq!(h.project().rack_chains, before.rack_chains);
    assert_eq!(h.project().devices, before.devices);
    assert_eq!(h.project().modulators, before.modulators);
    assert_eq!(h.project().mod_mappings, before.mod_mappings);
    redo(&mut h);
    assert_eq!(h.project().devices, after.devices);
    assert_eq!(h.project().mod_mappings, after.mod_mappings);
}

#[test]
fn loading_replaces_the_existing_structure_in_one_undo_step() {
    let mut h = harness();
    let t = track(&mut h, TrackKind::Midi);
    let rack = build_rack(&mut h, t);
    // Modulation that is not the rack's own is kept.
    let outside = insert(&mut h, t, BuiltinDeviceType::Utility);
    let lfo: ModulatorId = h.id();
    h.ok(Command::Modulation(ModulationCommand::AddModulator {
        id: lfo,
        device: outside,
        kind: ModulatorKind::Lfo,
        name: None,
    }));
    map(&mut h, ModSource::Modulator { modulator: lfo }, outside, 0, 0.5);
    let before = h.project().clone();
    let seed: RackChainId = h.id();
    let out = load(&mut h, rack, factory("instrument-rack/layered-pad"), Some(seed));
    ok(&out);
    let p = h.project();
    let chains = p.chains_of(rack);
    assert_eq!(
        chains.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        ["Body", "Air"]
    );
    for c in &before.rack_chains {
        assert!(!p.rack_chains.contains_key(c.0), "old chain removed");
    }
    assert_eq!(p.modulators_of(rack).len(), 1);
    assert_eq!(p.modulators_of(rack)[0].name, "Breath");
    // Every mapping sourced from the rack was replaced.
    assert_eq!(
        p.mod_mappings
            .values()
            .filter(|m| matches!(m.source, ModSource::Macro { rack: r, .. } if r == rack)
                || p.modulators.get(&match m.source {
                    ModSource::Modulator { modulator } => modulator,
                    _ => return false,
                }).is_some_and(|x| x.device == rack))
            .count(),
        4
    );
    assert_eq!(p.mappings_to(outside).len(), 1);
    assert_eq!(p.devices[&rack].params[&ParamId(0)], 0.25);
    assert_eq!(p.devices[&rack].params[&ParamId(1)], 0.4);
    undo(&mut h);
    assert_eq!(h.project().rack_chains, before.rack_chains);
    assert_eq!(h.project().devices, before.devices);
    assert_eq!(h.project().modulators, before.modulators);
    assert_eq!(h.project().mod_mappings, before.mod_mappings);
}

#[test]
fn seeds_are_required_for_structure_and_v1_presets_load_unchanged() {
    let mut h = harness();
    let t = track(&mut h, TrackKind::Midi);
    let rack = build_rack(&mut h, t);
    let before = h.project().clone();
    let out = load(&mut h, rack, factory("instrument-rack/key-split"), None);
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    assert_eq!(h.project().devices, before.devices);

    // A v1 preset (macros and the selector only) keeps the chains; a seed is ignored.
    let seed: RackChainId = h.id();
    for s in [None, Some(seed)] {
        ok(&load(&mut h, rack, factory("instrument-rack/init"), s));
        let p = h.project();
        assert_eq!(p.rack_chains, before.rack_chains);
        assert_eq!(p.modulators, before.modulators);
        assert_eq!(p.mod_mappings, before.mod_mappings);
        assert_eq!(p.devices[&rack].params[&ParamId(0)], 0.0);
        assert_eq!(p.devices[&rack].params[&ParamId(8)], 0.0);
    }
}

#[test]
fn same_seed_same_ids_on_every_site() {
    // Two sites holding the same document replay the same command: same ids.
    let site = || {
        let mut h = harness();
        let t = track(&mut h, TrackKind::Audio);
        let rack = insert(&mut h, t, BuiltinDeviceType::AudioEffectRack);
        (h, rack)
    };
    let (mut a, rack) = site();
    let (mut b, rack_b) = site();
    assert_eq!(rack, rack_b);
    let seed: RackChainId = a.id();
    for h in [&mut a, &mut b] {
        ok(&load(h, rack, factory("audio-effect-rack/dub-space"), Some(seed)));
    }
    assert_eq!(a.project().chains_of(rack).len(), 2);
    assert_eq!(a.project().rack_chains, b.project().rack_chains);
    assert_eq!(a.project().devices, b.project().devices);
    assert_eq!(a.project().modulators, b.project().modulators);
    assert_eq!(a.project().mod_mappings, b.project().mod_mappings);
}

#[test]
fn every_factory_rack_preset_loads_cleanly_onto_its_rack() {
    let mut h = harness();
    for ty in [
        BuiltinDeviceType::InstrumentRack,
        BuiltinDeviceType::AudioEffectRack,
        BuiltinDeviceType::MidiEffectRack,
    ] {
        let t = track(&mut h, TrackKind::Midi);
        let rack = insert(&mut h, t, ty);
        for f in ether_devices::factory_presets(ty) {
            let preset = load_preset(f.json).unwrap();
            let seed: RackChainId = h.id();
            let out = load(&mut h, rack, factory(f.id), Some(seed));
            ok(&out);
            assert!(warnings(&out).is_empty(), "{}: {:?}", f.id, warnings(&out));
            let Some(structure) = preset.rack else {
                continue;
            };
            let s = shape(h.project(), rack);
            assert_eq!(s.chains.len(), structure.chains.len(), "{}", f.id);
            for (got, want) in s.chains.iter().zip(&structure.chains) {
                assert_eq!(got.0.0, want.name);
                assert_eq!(got.1.len(), want.devices.len(), "{}", f.id);
            }
            assert_eq!(s.modulators.len(), structure.modulators.len(), "{}", f.id);
            assert_eq!(s.mappings.len(), structure.mappings.len(), "{}", f.id);
        }
        // The graph compiles with the loaded chains.
        h.tick();
    }
}

#[test]
fn content_the_rack_refuses_fails_the_whole_load() {
    let mut library = MemoryLibrary::new();
    let bad = r#"{"format":"ethereal-preset","version":2,"app_version":"x","preset":{
        "name":"Bad","device":{"type":"Builtin","device":"AudioEffectRack"},
        "rack":{"chains":[{"name":"A","color":null,"volume":0,"pan":0,"mute":false,"solo":false,
          "keys":{"lo":0,"hi":127},"velocities":{"lo":0,"hi":127},"select":{"lo":0,"hi":127},
          "devices":[{"name":"S","enabled":true,"device":{"type":"Builtin","device":"Synth"}}]}]}}}"#;
    library.add_file(
        USER,
        "Presets/audio-effect-rack/Bad.etherpreset",
        bad.as_bytes().to_vec(),
    );
    let mut h = harness_with(FakeBridge::default(), library);
    let t = track(&mut h, TrackKind::Audio);
    let rack = insert(&mut h, t, BuiltinDeviceType::AudioEffectRack);
    let c = add_chain(&mut h, rack, "Keep");
    chain_insert(&mut h, c, builtin(BuiltinDeviceType::Delay));
    let before = h.project().clone();
    let seed: RackChainId = h.id();
    let out = load(
        &mut h,
        rack,
        PresetRef {
            source: PresetSource::User,
            id: "audio-effect-rack/Bad.etherpreset".into(),
        },
        Some(seed),
    );
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    assert_eq!(h.project().devices, before.devices);
    assert_eq!(h.project().rack_chains, before.rack_chains);
    // Another rack type's preset is refused too.
    let out = load(&mut h, rack, factory("midi-effect-rack/chord-arp"), Some(seed));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
}

#[test]
fn chain_plugins_keep_their_state_and_chain_samplers_their_sample() {
    const FX: &str = "com.test.Fx";
    let bridge = FakeBridge {
        plugins: Some(BTreeMap::from([(
            FX.to_string(),
            plugin_descriptor("Fx", DeviceCategory::AudioEffect),
        )])),
        ..Default::default()
    };
    let mut library = MemoryLibrary::new();
    library.add_file(
        "lib",
        "hit.wav",
        wav(48_000, &[sine(48_000, 220.0, 4_800, 0.5)]),
    );
    let mut h = harness_with(bridge, library);
    let t = track(&mut h, TrackKind::Midi);
    let rack = insert(&mut h, t, BuiltinDeviceType::InstrumentRack);
    let c = add_chain(&mut h, rack, "Layer");
    let sampler = chain_insert(&mut h, c, builtin(BuiltinDeviceType::Sampler));
    let media: MediaId = h.id();
    h.ok(Command::Media(MediaCommand::Import {
        id: media,
        source: MediaSource::Location {
            location: BrowseLocation::Library { id: "lib".into() },
            path: "hit.wav".into(),
        },
    }));
    h.ok(Command::Device(DeviceCommand::SetSample {
        device: sampler,
        media: Some(media),
    }));
    let fx = chain_insert(
        &mut h,
        c,
        DeviceSpec::Plugin {
            plugin_id: FX.into(),
            sandboxed: Some(false),
            format: Some(PluginFormat::Clap),
        },
    );
    h.tick();
    h.ctl
        .bridge
        .plugin_states
        .insert(fx, Base64Bytes(b"fx-state".to_vec()));
    let info = save(&mut h, rack, "Sampled");

    // Another project: the sample is imported, the plugin re-created from its state.
    h.create_project("Other");
    let t = track(&mut h, TrackKind::Midi);
    let rack2 = insert(&mut h, t, BuiltinDeviceType::InstrumentRack);
    let seed: RackChainId = h.id();
    ok(&load(&mut h, rack2, info.preset, Some(seed)));
    h.tick();
    let p = h.project();
    let chain = p.chains_of(rack2)[0].id;
    let devices = p.chain_devices_of(chain);
    assert_eq!(devices.len(), 2);
    let DeviceKind::Builtin {
        device: BuiltinDevice::Sampler { sample, .. },
    } = &devices[0].kind
    else {
        panic!("{:?}", devices[0].kind)
    };
    let sample = sample.expect("sample imported");
    assert!(p.media.contains_key(&sample));
    let DeviceKind::Plugin { plugin } = &devices[1].kind else {
        panic!()
    };
    assert_eq!(plugin.state, Some(Base64Bytes(b"fx-state".to_vec())));
    let fx2 = devices[1].id;
    assert!(h.ctl.bridge.calls.iter().any(|c| matches!(
        c,
        Call::CreatePlugin(id, _, Some(state)) if *id == fx2 && state.0 == b"fx-state"
    )));
}

//! Drum rack presets: saving a drum rack stores its pads (note, name, color, choke
//! group, mix) and their chains, with the pads' samples copied into the user library;
//! `Preset::Load { seed }` replaces the rack's pads in one undo step; drum rack presets
//! without a structure still load (params only, the pads stay).

use crate::common;

use std::collections::BTreeMap;

use common::*;
use ether_controller::memory::MemoryLibrary;
use ether_controller::store::Library;
use ether_core::protocol::devices::{DeviceCommand, DeviceSpec};
use ether_core::protocol::drum_rack::DrumRackCommand;
use ether_core::protocol::media::{BrowseLocation, MediaCommand, MediaSource};
use ether_core::protocol::model::*;
use ether_core::protocol::presets::{PresetCommand, PresetInfo, PresetRef, PresetSource};
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::racks::{ModulationCommand, RackCommand};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::{Command, ErrorCode, ReplyValue, ServerMessage};

const USER: &str = "user";

fn harness() -> Harness {
    let mut library = MemoryLibrary::new().with_user_root(USER);
    library.add_file(
        "lib",
        "kick.wav",
        wav(48_000, &[sine(48_000, 60.0, 4_800, 0.5)]),
    );
    library.add_file(
        "lib",
        "snare.wav",
        wav(48_000, &[sine(48_000, 200.0, 2_400, 0.5)]),
    );
    let mut h = Harness::with(FakeBridge::default(), library, Default::default());
    h.create_project("Drum rack presets");
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

fn import(h: &mut Harness, path: &str) -> MediaId {
    let id: MediaId = h.id();
    h.ok(Command::Media(MediaCommand::Import {
        id,
        source: MediaSource::Location {
            location: BrowseLocation::Library { id: "lib".into() },
            path: path.into(),
        },
    }));
    id
}

fn drum(h: &mut Harness, c: DrumRackCommand) {
    h.ok(Command::DrumRack(c));
}

fn add_pad(h: &mut Harness, rack: DeviceId, note: u8) -> DrumPadId {
    let id: DrumPadId = h.id();
    drum(
        h,
        DrumRackCommand::AddPad {
            id,
            rack,
            note,
            name: None,
        },
    );
    id
}

fn pad_insert(h: &mut Harness, pad: DrumPadId, ty: BuiltinDeviceType) -> DeviceId {
    let id: DeviceId = h.id();
    drum(
        h,
        DrumRackCommand::InsertDevice {
            id,
            pad,
            device: DeviceSpec::Builtin {
                device: BuiltinDevice::new(ty),
            },
            before: None,
        },
    );
    id
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

fn load(
    h: &mut Harness,
    device: DeviceId,
    preset: PresetRef,
    seed: Option<RackChainId>,
) -> Vec<ServerMessage> {
    h.send(Command::Preset(PresetCommand::Load {
        device,
        preset,
        seed,
    }))
}

fn preset_file(h: &mut Harness, info: &PresetInfo) -> Preset {
    let json = String::from_utf8(
        h.ctl
            .library
            .read(USER, &format!("Presets/{}", info.preset.id))
            .expect("written"),
    )
    .unwrap();
    load_preset(&json).unwrap()
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

type PadShape = (u8, String, Option<Color>, Option<u8>, Decibels, Pan, bool);
type DeviceShape = (
    String,
    bool,
    BuiltinDeviceType,
    BTreeMap<ParamId, f64>,
    bool,
);

/// The pads of a drum rack with their chains, ids replaced by positions, samples by
/// "has one" (media ids differ across projects).
fn shape(p: &Project, rack: DeviceId) -> Vec<(PadShape, Vec<DeviceShape>)> {
    p.pads_of(rack)
        .into_iter()
        .map(|pad| {
            let devices = p
                .pad_devices_of(pad.id)
                .into_iter()
                .map(|d| {
                    let DeviceKind::Builtin { device } = &d.kind else {
                        panic!("{:?}", d.kind)
                    };
                    let sampled = matches!(device, BuiltinDevice::Sampler { sample: Some(m), .. } if p.media.contains_key(m));
                    (
                        d.name.clone(),
                        d.enabled,
                        device.device_type(),
                        d.params.clone(),
                        sampled,
                    )
                })
                .collect();
            let shape = (
                pad.note,
                pad.name.clone(),
                pad.color,
                pad.choke_group,
                pad.volume,
                pad.pan,
                pad.mute,
            );
            (shape, devices)
        })
        .collect()
}

/// A drum rack with a kick pad (sample-pad shortcut), a snare pad (colored, choke group,
/// mix, a sampler and a disabled utility with edited params) and an LFO on the rack mapped
/// to its pan.
fn build_kit(h: &mut Harness, t: TrackId) -> DeviceId {
    let rack = insert(h, t, BuiltinDeviceType::DrumRack);
    let kick = import(h, "kick.wav");
    let (kick_pad, kick_dev): (DrumPadId, DeviceId) = (h.id(), h.id());
    drum(
        h,
        DrumRackCommand::AddSamplePad {
            pad: kick_pad,
            device: kick_dev,
            rack,
            note: 36,
            media: kick,
        },
    );
    let snare = add_pad(h, rack, 38);
    drum(
        h,
        DrumRackCommand::RenamePad {
            id: snare,
            name: "Snare".into(),
        },
    );
    drum(
        h,
        DrumRackCommand::SetPadColor {
            id: snare,
            color: Some(Color(0xff3366)),
        },
    );
    drum(
        h,
        DrumRackCommand::SetChokeGroup {
            id: snare,
            group: Some(2),
        },
    );
    drum(
        h,
        DrumRackCommand::SetPadVolume {
            id: snare,
            volume: Decibels(-6.0),
        },
    );
    drum(
        h,
        DrumRackCommand::SetPadPan {
            id: snare,
            pan: Pan(0.25),
        },
    );
    drum(
        h,
        DrumRackCommand::SetPadMute {
            id: snare,
            mute: true,
        },
    );
    let sampler = pad_insert(h, snare, BuiltinDeviceType::Sampler);
    let media = import(h, "snare.wav");
    h.ok(Command::Device(DeviceCommand::SetSample {
        device: sampler,
        media: Some(media),
    }));
    let utility = pad_insert(h, snare, BuiltinDeviceType::Utility);
    h.ok(Command::Device(DeviceCommand::SetEnabled {
        id: utility,
        enabled: false,
    }));
    h.ok(Command::Device(DeviceCommand::SetParam {
        device: utility,
        param: automatable(BuiltinDeviceType::Utility),
        value: -3.0,
    }));
    let lfo: ModulatorId = h.id();
    h.ok(Command::Modulation(ModulationCommand::AddModulator {
        id: lfo,
        device: rack,
        kind: ModulatorKind::Lfo,
        name: Some("Shake".into()),
    }));
    let mapping: ModMappingId = h.id();
    h.ok(Command::Modulation(ModulationCommand::Map {
        id: mapping,
        source: ModSource::Modulator { modulator: lfo },
        device: rack,
        param: ParamId(1),
        depth: 0.3,
    }));
    rack
}

#[test]
fn saving_a_drum_rack_stores_its_pads_and_loading_rebuilds_them_in_one_undo_step() {
    let mut h = harness();
    let t = track(&mut h);
    let rack = build_kit(&mut h, t);
    let kit = shape(h.project(), rack);
    assert_eq!(kit.len(), 2);
    let info = save(&mut h, rack, "Kit");

    // The file stores the pads, their chains and the rack's modulation; the samples are
    // copied into the user library.
    let file = preset_file(&mut h, &info);
    let structure = file.rack.expect("drum rack structure");
    assert!(structure.chains.is_empty());
    assert_eq!(
        structure.pads.iter().map(|p| p.note).collect::<Vec<_>>(),
        [36, 38]
    );
    assert_eq!(structure.pads[1].devices.len(), 2);
    assert_eq!(structure.modulators.len(), 1);
    assert_eq!(
        structure.mappings,
        [PresetModMapping {
            source: PresetModSource::Modulator { index: 0 },
            target: PresetModTarget::Rack,
            param: ParamId(1),
            depth: 0.3,
        }]
    );
    assert_eq!(file.samples.len(), 2);
    let files = h.ctl.library.files(USER);
    for name in ["-kick.wav", "-snare.wav"] {
        assert!(
            files
                .iter()
                .any(|f| f.starts_with("Samples/") && f.ends_with(name)),
            "{files:?}"
        );
    }

    // Another project, a drum rack with a pad of its own: the preset replaces it.
    h.create_project("Other");
    let t = track(&mut h);
    let rack2 = insert(&mut h, t, BuiltinDeviceType::DrumRack);
    let old = add_pad(&mut h, rack2, 40);
    pad_insert(&mut h, old, BuiltinDeviceType::Sampler);
    let before = h.project().clone();
    let seed: RackChainId = h.id();
    ok(&load(&mut h, rack2, info.preset.clone(), Some(seed)));
    assert_eq!(shape(h.project(), rack2), kit);
    let p = h.project();
    assert!(!p.drum_pads.contains_key(&old), "old pad removed");
    // Ids: pads, then each pad's devices, then modulators, then mappings.
    let pads = p.pads_of(rack2);
    for (i, pad) in pads.iter().enumerate() {
        assert_eq!(pad.id, derive_id::<_, DrumPadId>(seed, i as u32));
    }
    let mut i = 2;
    for pad in &pads {
        for d in p.pad_devices_of(pad.id) {
            assert_eq!(d.id, derive_id::<_, DeviceId>(seed, i));
            i += 1;
        }
    }
    let lfo = p.modulators_of(rack2)[0];
    assert_eq!((lfo.id, lfo.name.as_str()), (derive_id(seed, i), "Shake"));
    let mapping = &p.mod_mappings[&derive_id::<_, ModMappingId>(seed, i + 1)];
    assert_eq!((mapping.device, mapping.param), (rack2, ParamId(1)));
    assert_eq!(mapping.source, ModSource::Modulator { modulator: lfo.id });

    // One undo step back to the old pad, and redo.
    let after = h.project().clone();
    h.ok(Command::Edit(EditCommand::Undo));
    let p = h.project();
    assert_eq!(p.drum_pads, before.drum_pads);
    assert_eq!(p.devices, before.devices);
    assert_eq!(p.modulators, before.modulators);
    assert_eq!(p.mod_mappings, before.mod_mappings);
    h.ok(Command::Edit(EditCommand::Redo));
    assert_eq!(h.project().drum_pads, after.drum_pads);
    assert_eq!(h.project().devices, after.devices);
    assert_eq!(h.project().mod_mappings, after.mod_mappings);
}

#[test]
fn older_drum_rack_presets_keep_the_pads_and_structure_needs_a_seed() {
    let mut h = harness();
    let t = track(&mut h);
    let rack = build_kit(&mut h, t);
    let info = save(&mut h, rack, "Kit");
    let before = h.project().clone();
    // A structure needs a seed (nothing changes without one).
    let out = load(&mut h, rack, info.preset.clone(), None);
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    assert_eq!(h.project().drum_pads, before.drum_pads);

    // A preset written by older versions (params only) loads without a seed and keeps the pads.
    let old = r#"{"format":"ethereal-preset","version":2,"app_version":"0.6.0",
        "preset":{"name":"Old Kit","device":{"type":"Builtin","device":"DrumRack"},"params":{}}}"#;
    h.ctl
        .library
        .write_file(
            USER,
            "Presets/drum-rack/Old Kit.etherpreset",
            old.as_bytes(),
        )
        .unwrap();
    let old = PresetRef {
        source: PresetSource::User,
        id: "drum-rack/Old Kit.etherpreset".into(),
    };
    let seed: RackChainId = h.id();
    for s in [None, Some(seed)] {
        ok(&load(&mut h, rack, old.clone(), s));
        let p = h.project();
        assert_eq!(p.drum_pads, before.drum_pads);
        assert_eq!(p.devices.len(), before.devices.len());
        assert_eq!(p.modulators, before.modulators);
    }
}

#[test]
fn a_drum_rack_in_a_rack_chain_loads_the_pads_without_the_modulators() {
    let mut h = harness();
    let t = track(&mut h);
    let rack = build_kit(&mut h, t);
    let kit = shape(h.project(), rack);
    let info = save(&mut h, rack, "Kit");

    let t2 = track(&mut h);
    let outer = insert(&mut h, t2, BuiltinDeviceType::InstrumentRack);
    let chain: RackChainId = h.id();
    h.ok(Command::Rack(RackCommand::AddChain {
        id: chain,
        rack: outer,
        name: None,
        before: None,
    }));
    let nested: DeviceId = h.id();
    h.ok(Command::Rack(RackCommand::InsertDevice {
        id: nested,
        chain,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::DrumRack,
        },
        before: None,
    }));
    let seed: RackChainId = h.id();
    ok(&load(&mut h, nested, info.preset, Some(seed)));
    assert_eq!(shape(h.project(), nested), kit);
    assert!(h.project().modulators_of(nested).is_empty());
}

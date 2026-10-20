//! Convolution reverb IRs (v0.3, `fx-space`, CONTRACTS.md §13.6): `Device::SetIr` is one
//! undo step and validates its source and target, `Device::ListFactoryIrs` lists the
//! factory set, IR edits reach the engine, and a reverb whose IR media loads late is
//! rebuilt with it (media referenced like samples).

use crate::common;

use common::*;
use ether_controller::memory::MemoryLibrary;
use ether_core::protocol::devices::{DeviceCommand, DeviceSpec};
use ether_core::protocol::media::*;
use ether_core::protocol::model::*;
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::*;

const SR: u32 = 48_000;

struct Setup {
    h: Harness,
    device: DeviceId,
    media: MediaId,
}

/// A decaying noise burst (an IR-like file).
fn ir_samples(frames: usize, seed: u32) -> Vec<f32> {
    let mut s = seed;
    (0..frames)
        .map(|i| {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let n = (s >> 8) as f32 / (1 << 24) as f32 * 2.0 - 1.0;
            n * (-(i as f32) / (frames as f32 / 6.0)).exp()
        })
        .collect()
}

fn harness(file: &str) -> Harness {
    let mut lib = MemoryLibrary::new();
    lib.add_root("lib", "Library");
    let ir = ir_samples(SR as usize / 2, 3);
    lib.add_file("lib", file, wav(SR, &[ir.clone(), ir]));
    let mut h = Harness::with(FakeBridge::default(), lib, Default::default());
    h.create_project("Space");
    h
}

fn insert_reverb(h: &mut Harness) -> DeviceId {
    let track: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id: track,
        kind: TrackKind::Audio,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    let device: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id: device,
        track,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::new(BuiltinDeviceType::ConvolutionReverb),
        },
        before: None,
    }));
    device
}

fn import(h: &mut Harness, file: &str) -> MediaId {
    let media: MediaId = h.id();
    h.ok(Command::Media(MediaCommand::Import {
        id: media,
        source: MediaSource::Location {
            location: BrowseLocation::Library { id: "lib".into() },
            path: file.into(),
        },
    }));
    media
}

fn setup() -> Setup {
    let mut h = harness("hall.wav");
    let device = insert_reverb(&mut h);
    let media = import(&mut h, "hall.wav");
    h.drain_media();
    Setup { h, device, media }
}

fn ir_of(h: &Harness, d: DeviceId) -> Option<IrSource> {
    match &h.project().devices[&d].kind {
        DeviceKind::Builtin {
            device: BuiltinDevice::ConvolutionReverb { ir },
        } => ir.clone(),
        other => panic!("not a convolution reverb: {other:?}"),
    }
}

fn set_ir(device: DeviceId, ir: Option<IrSource>) -> Command {
    Command::Device(DeviceCommand::SetIr { device, ir })
}

fn factory(id: &str) -> Option<IrSource> {
    Some(IrSource::Factory { id: id.into() })
}

#[test]
fn set_ir_is_one_undo_step() {
    let Setup {
        mut h,
        device,
        media,
    } = setup();
    // A new convolution reverb starts on the factory hall.
    assert_eq!(ir_of(&h, device), factory("hall"));
    h.ok(set_ir(device, factory("plate")));
    assert_eq!(ir_of(&h, device), factory("plate"));
    let m = Some(IrSource::Media { media });
    h.ok(set_ir(device, m.clone()));
    assert_eq!(ir_of(&h, device), m);
    assert_eq!(
        h.project().devices[&device].kind,
        DeviceKind::Builtin {
            device: BuiltinDevice::ConvolutionReverb { ir: m.clone() }
        }
    );
    h.ok(set_ir(device, None));
    assert_eq!(ir_of(&h, device), None);
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(ir_of(&h, device), m);
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(ir_of(&h, device), factory("plate"));
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(ir_of(&h, device), factory("hall"));
    h.ok(Command::Edit(EditCommand::Redo));
    assert_eq!(ir_of(&h, device), factory("plate"));
}

#[test]
fn set_ir_validates() {
    let Setup { mut h, device, .. } = setup();
    let before = h.project().clone();
    let out = h.send(set_ir(device, factory("no-such-ir")));
    assert_eq!(err(&out).code, ErrorCode::NotFound);
    let unknown: MediaId = h.id();
    let out = h.send(set_ir(device, Some(IrSource::Media { media: unknown })));
    assert_eq!(err(&out).code, ErrorCode::NotFound);
    let track = h.project().devices[&device].track;
    let comp: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id: comp,
        track,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::Compressor,
        },
        before: None,
    }));
    let out = h.send(set_ir(comp, factory("hall")));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    let missing: DeviceId = h.id();
    let out = h.send(set_ir(missing, factory("hall")));
    assert_eq!(err(&out).code, ErrorCode::NotFound);
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(h.project(), &before);
}

#[test]
fn set_ir_works_in_a_batch() {
    let Setup { mut h, device, .. } = setup();
    h.ok(Command::Edit(EditCommand::Batch {
        key: None,
        label: "IR".into(),
        commands: vec![
            set_ir(device, factory("room")),
            set_ir(device, factory("plate")),
        ],
    }));
    assert_eq!(ir_of(&h, device), factory("plate"));
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(ir_of(&h, device), factory("hall"));
}

#[test]
fn lists_the_factory_irs() {
    let mut h = Harness::with_project();
    let ReplyValue::FactoryIrs { irs } = h.ok(Command::Device(DeviceCommand::ListFactoryIrs))
    else {
        panic!("expected FactoryIrs");
    };
    let ids: Vec<&str> = irs.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(
        ids,
        ["room", "chamber", "plate", "hall", "cathedral", "ambience"]
    );
    for ir in &irs {
        assert!(ir.length.0 > 0.0 && ir.channels == 2, "{ir:?}");
        assert!(!ir.name.is_empty() && !ir.category.is_empty());
    }
    let hall = irs.iter().find(|i| i.id == "hall").unwrap();
    assert_eq!(
        (hall.name.as_str(), hall.category.as_str()),
        ("Concert Hall", "Hall")
    );
}

#[test]
fn ir_edits_reach_the_engine_and_media_is_listed() {
    let Setup {
        mut h,
        device,
        media,
    } = setup();
    h.tick();
    let created = |h: &Harness| {
        h.ctl
            .bridge
            .calls
            .iter()
            .filter(|c| matches!(c, Call::CreateBuiltin(d, _) if *d == device))
            .count()
    };
    let n0 = created(&h);
    h.ok(set_ir(device, Some(IrSource::Media { media })));
    h.tick();
    // The fake bridge has no in-place update: the node is rebuilt with the IR.
    assert_eq!(created(&h), n0 + 1);
    // Same IR again: no-op.
    h.ok(set_ir(device, Some(IrSource::Media { media })));
    h.tick();
    assert_eq!(created(&h), n0 + 1);
    // The IR media is the device's media (loaded, collected, relinked like samples).
    let DeviceKind::Builtin { device: kind } = &h.project().devices[&device].kind else {
        unreachable!()
    };
    assert_eq!(kind.media(), vec![media]);
}

#[test]
fn ir_media_load_rebuilds_the_reverb() {
    let mut h = harness("late.wav");
    let device = insert_reverb(&mut h);
    let media = import(&mut h, "late.wav");
    // IR set before the media finished loading.
    h.ok(set_ir(device, Some(IrSource::Media { media })));
    h.tick();
    h.drain_media();
    h.tick();
    let calls = &h.ctl.bridge.calls;
    let loaded = calls
        .iter()
        .position(|c| matches!(c, Call::LoadMedia(m) if *m == media))
        .expect("IR media loaded");
    assert!(
        calls[loaded..]
            .iter()
            .any(|c| matches!(c, Call::CreateBuiltin(d, _) if *d == device)),
        "rebuilt after its IR media loaded"
    );
}

#[test]
fn factory_presets_set_the_ir_and_params_in_one_step() {
    use ether_core::protocol::presets::{PresetCommand, PresetRef, PresetSource};
    let Setup { mut h, device, .. } = setup();
    let out = h.send(Command::Preset(PresetCommand::Load {
        device,
        preset: PresetRef {
            source: PresetSource::Factory,
            id: "convolution-reverb/reverse-swell".into(),
        },
        seed: None,
    }));
    ok(&out);
    assert_eq!(ir_of(&h, device), factory("hall"));
    let params = &h.project().devices[&device].params;
    assert_eq!(params.get(&ParamId(8)), Some(&1.0), "reverse");
    assert_eq!(params.get(&ParamId(2)), Some(&70.0), "decay");
    h.ok(Command::Edit(EditCommand::Undo));
    // Back to the new device's IR (the factory hall too) and params.
    assert_eq!(ir_of(&h, device), factory("hall"));
    assert_eq!(
        h.project().devices[&device].params.get(&ParamId(8)),
        Some(&0.0)
    );
}

#[test]
fn user_presets_carry_their_ir_file() {
    use ether_core::protocol::model::PresetMeta;
    use ether_core::protocol::presets::PresetCommand;
    let mut lib = MemoryLibrary::new().with_user_root("user");
    lib.add_root("lib", "Library");
    let ir = ir_samples(SR as usize / 4, 5);
    lib.add_file("lib", "space.wav", wav(SR, &[ir]));
    let mut h = Harness::with(FakeBridge::default(), lib, Default::default());
    h.create_project("Space");
    let device = insert_reverb(&mut h);
    let media = import(&mut h, "space.wav");
    h.drain_media();
    h.ok(set_ir(device, Some(IrSource::Media { media })));
    let saved = match h.ok(Command::Preset(PresetCommand::Save {
        device,
        name: "My Space".into(),
        meta: PresetMeta::default(),
        overwrite: false,
    })) {
        ReplyValue::Preset { preset } => preset,
        other => panic!("{other:?}"),
    };
    // Another project: the IR file comes back from the user library with the preset.
    h.create_project("Other");
    let other = insert_reverb(&mut h);
    h.ok(Command::Preset(PresetCommand::Load {
        device: other,
        preset: saved.preset.clone(),
        seed: None,
    }));
    match ir_of(&h, other) {
        Some(IrSource::Media { media }) => assert!(h.project().media.contains_key(&media)),
        other => panic!("IR not restored: {other:?}"),
    }
}

#[test]
fn ir_media_is_decoded_again_for_the_bridge_cut_to_the_longest_ir() {
    // A web bridge builds media IRs itself, from audio it asks for again (it doesn't
    // keep every loaded media): the controller decodes the media again (the first
    // `MAX_IR_SECONDS`, at the engine rate), hands it over, and keeps nothing.
    let mut lib = MemoryLibrary::new();
    lib.add_root("lib", "Library");
    let ir = ir_samples(44_100 * 14, 5);
    lib.add_file("lib", "long.wav", wav(44_100, &[ir.clone(), ir]));
    let mut h = Harness::with(FakeBridge::default(), lib, Default::default());
    h.create_project("Space");
    let media = import(&mut h, "long.wav");
    h.drain_media();
    let loads = |h: &Harness| {
        h.ctl
            .bridge
            .calls
            .iter()
            .filter(|c| matches!(c, Call::LoadMedia(_)))
            .count()
    };
    assert_eq!(loads(&h), 1);
    h.ctl.bridge.ir_requests.push(media);
    let out = h.drain_media();
    assert_eq!(loads(&h), 1, "not loaded into the engine again");
    assert!(
        !out.iter()
            .any(|m| format!("{m:?}").contains("Notification")),
        "{out:?}"
    );
    let [(m, audio)] = &h.ctl.bridge.ir_audio[..] else {
        panic!("one IR media handed over: {}", h.ctl.bridge.ir_audio.len())
    };
    assert_eq!(*m, media);
    let loaded = &h.ctl.bridge.media[&media];
    assert_eq!(audio.sample_rate, loaded.sample_rate);
    assert_eq!(audio.channels.len(), 2);
    let max = (ether_devices::fx_space::ir::MAX_IR_SECONDS * audio.sample_rate as f64) as usize;
    let frames = audio.frames();
    assert!(
        frames >= max && frames < max + audio.sample_rate as usize / 5,
        "{frames} frames for an IR of at most {max}"
    );
    // The same audio as loaded (away from the cut).
    let diff = audio.channels[0][..max - 4_800]
        .iter()
        .zip(&loaded.channels[0])
        .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
    assert!(diff < 1e-4, "differs from the loaded audio by {diff}");
}

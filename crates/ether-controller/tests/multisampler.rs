//! Multisampler zones (v0.2, `multisampler`): `Device::SetZones` replaces the zones in one
//! undo step, validates media and the target device, reaches the engine (node rebuilt with
//! the zones on this fake bridge), and multisamplers rebuild when their zone media loads.

mod common;

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

fn setup() -> Setup {
    let mut lib = MemoryLibrary::new();
    lib.add_root("lib", "Library");
    let x = sine(SR, 440.0, SR as usize / 2, 0.5);
    lib.add_file("lib", "C3.wav", wav(SR, &[x.clone(), x]));
    let mut h = Harness::with(FakeBridge::default(), lib, Default::default());
    h.create_project("Keys");
    let track: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id: track,
        kind: TrackKind::Midi,
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
            device: BuiltinDevice::new(BuiltinDeviceType::MultiSampler),
        },
        before: None,
    }));
    let media: MediaId = h.id();
    h.ok(Command::Media(MediaCommand::Import {
        id: media,
        source: MediaSource::Location {
            location: BrowseLocation::Library { id: "lib".into() },
            path: "C3.wav".into(),
        },
    }));
    h.drain_media();
    Setup { h, device, media }
}

fn zones_of(h: &Harness, d: DeviceId) -> Vec<SampleZone> {
    match &h.project().devices[&d].kind {
        DeviceKind::Builtin {
            device: BuiltinDevice::MultiSampler { zones },
        } => zones.clone(),
        other => panic!("not a multisampler: {other:?}"),
    }
}

fn zone(media: MediaId, lo: u8, hi: u8) -> SampleZone {
    SampleZone {
        media: Some(media),
        keys: Zone { lo, hi },
        root_key: lo,
        ..SampleZone::default()
    }
}

fn set_zones(device: DeviceId, zones: Vec<SampleZone>) -> Command {
    Command::Device(DeviceCommand::SetZones { device, zones })
}

#[test]
fn set_zones_is_one_undo_step() {
    let Setup {
        mut h,
        device,
        media,
    } = setup();
    assert!(zones_of(&h, device).is_empty());
    let a = vec![zone(media, 48, 59), zone(media, 60, 71)];
    h.ok(set_zones(device, a.clone()));
    assert_eq!(zones_of(&h, device), a);
    let b = vec![zone(media, 0, 127)];
    h.ok(set_zones(device, b.clone()));
    assert_eq!(zones_of(&h, device), b);
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(zones_of(&h, device), a);
    h.ok(Command::Edit(EditCommand::Undo));
    assert!(zones_of(&h, device).is_empty());
    h.ok(Command::Edit(EditCommand::Redo));
    assert_eq!(zones_of(&h, device), a);
}

#[test]
fn set_zones_validates() {
    let Setup {
        mut h,
        device,
        media,
    } = setup();
    let before = h.project().clone();
    // Unknown media.
    let unknown: MediaId = h.id();
    let out = h.send(set_zones(device, vec![zone(unknown, 0, 127)]));
    assert_eq!(err(&out).code, ErrorCode::NotFound);
    // Bad ranges (rejected by the model).
    let bad = SampleZone {
        keys: Zone { lo: 80, hi: 10 },
        ..zone(media, 0, 0)
    };
    let out = h.send(set_zones(device, vec![bad]));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    // Too many zones.
    let many = vec![zone(media, 0, 127); MAX_ZONES + 1];
    let out = h.send(set_zones(device, many));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    // Not a multisampler.
    let track = h.project().devices[&device].track;
    let synth: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id: synth,
        track,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::Compressor,
        },
        before: None,
    }));
    let out = h.send(set_zones(synth, vec![]));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(h.project(), &before);
}

#[test]
fn zone_edits_reach_the_engine() {
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
    h.ok(set_zones(device, vec![zone(media, 0, 127)]));
    h.tick();
    // The fake bridge has no in-place update: the node is rebuilt with the zones.
    assert_eq!(created(&h), n0 + 1);
    // Unchanged zones: no rebuild.
    h.ok(set_zones(device, vec![zone(media, 0, 127)]));
    h.tick();
    assert_eq!(created(&h), n0 + 1);
}

#[test]
fn zone_media_load_rebuilds_the_multisampler() {
    let mut lib = MemoryLibrary::new();
    lib.add_root("lib", "Library");
    let x = sine(SR, 220.0, SR as usize, 0.5);
    lib.add_file("lib", "A2.wav", wav(SR, &[x]));
    let mut h = Harness::with(FakeBridge::default(), lib, Default::default());
    h.create_project("Late");
    let track: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id: track,
        kind: TrackKind::Midi,
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
            device: BuiltinDevice::new(BuiltinDeviceType::MultiSampler),
        },
        before: None,
    }));
    let media: MediaId = h.id();
    h.ok(Command::Media(MediaCommand::Import {
        id: media,
        source: MediaSource::Location {
            location: BrowseLocation::Library { id: "lib".into() },
            path: "A2.wav".into(),
        },
    }));
    // Zones set before the media finished loading.
    h.ok(set_zones(device, vec![zone(media, 0, 127)]));
    h.tick();
    let created = |h: &Harness| {
        h.ctl
            .bridge
            .calls
            .iter()
            .filter(|c| matches!(c, Call::CreateBuiltin(d, _) if *d == device))
            .count()
    };
    let before = created(&h);
    h.drain_media();
    h.tick();
    assert!(
        h.ctl
            .bridge
            .calls
            .iter()
            .any(|c| matches!(c, Call::LoadMedia(m) if *m == media))
    );
    assert!(created(&h) > before, "rebuilt after its zone media loaded");
}

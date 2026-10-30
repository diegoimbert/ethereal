//! The media manager's `MediaRef::Remove`: what uses a media is unlinked (audio clips
//! deleted, samplers and convolution reverbs unloaded, multisampler zones emptied) and the
//! media removed, in one undo step.

use crate::common;

use common::*;
use ether_controller::memory::MemoryLibrary;
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::devices::{DeviceCommand, DeviceSpec};
use ether_core::protocol::media::*;
use ether_core::protocol::media_refs::MediaRefCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::*;

const SR: u32 = 48_000;

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

fn insert(h: &mut Harness, track: TrackId, device: BuiltinDevice) -> DeviceId {
    let id: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id,
        track,
        device: DeviceSpec::Builtin { device },
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
    h.drain_media();
    id
}

fn builtin(h: &Harness, d: DeviceId) -> BuiltinDevice {
    match &h.project().devices[&d].kind {
        DeviceKind::Builtin { device } => device.clone(),
        other => panic!("{other:?}"),
    }
}

fn remove(media: Vec<MediaId>) -> Command {
    Command::MediaRef(MediaRefCommand::Remove { media })
}

#[test]
fn removing_a_sample_unlinks_its_clips_and_devices_in_one_undo_step() {
    let mut lib = MemoryLibrary::new();
    lib.add_root("lib", "Library");
    let x = sine(SR, 440.0, SR as usize / 2, 0.5);
    lib.add_file("lib", "Kick.wav", wav(SR, &[x.clone(), x.clone()]));
    lib.add_file("lib", "Keep.wav", wav(SR, &[x.clone(), x]));
    let mut h = Harness::with(FakeBridge::default(), lib, Default::default());
    h.create_project("Media");
    let kick = import(&mut h, "Kick.wav");
    let keep = import(&mut h, "Keep.wav");

    let audio = track(&mut h, TrackKind::Audio);
    let clips: Vec<ClipId> = [0.0, 4.0]
        .into_iter()
        .map(|start| {
            let id: ClipId = h.id();
            h.ok(Command::Clip(ClipCommand::CreateAudio {
                id,
                track: audio,
                start: Beats(start),
                media: kick,
            }));
            id
        })
        .collect();
    let kept: ClipId = h.id();
    h.ok(Command::Clip(ClipCommand::CreateAudio {
        id: kept,
        track: audio,
        start: Beats(8.0),
        media: keep,
    }));

    let midi = track(&mut h, TrackKind::Midi);
    let sampler = insert(&mut h, midi, BuiltinDevice::new(BuiltinDeviceType::Sampler));
    h.ok(Command::Device(DeviceCommand::SetSample {
        device: sampler,
        media: Some(kick),
    }));
    let keys = track(&mut h, TrackKind::Midi);
    let multi = insert(&mut h, keys, BuiltinDevice::new(BuiltinDeviceType::MultiSampler));
    let zone = |media, lo, hi| SampleZone {
        media: Some(media),
        keys: Zone { lo, hi },
        root_key: lo,
        ..SampleZone::default()
    };
    h.ok(Command::Device(DeviceCommand::SetZones {
        device: multi,
        zones: vec![zone(kick, 0, 59), zone(keep, 60, 127)],
    }));
    let reverb = insert(
        &mut h,
        audio,
        BuiltinDevice::ConvolutionReverb {
            ir: Some(IrSource::Media { media: kick }),
        },
    );

    let users = h.project().media_users(kick);
    assert_eq!(users.clips.len(), 2);
    assert_eq!(users.devices.len(), 3);
    let before = h.project().clone();

    h.ok(remove(vec![kick]));
    let p = h.project();
    assert!(!p.media.contains_key(&kick));
    assert!(p.media.contains_key(&keep), "other media stay");
    for c in &clips {
        assert!(!p.clips.contains_key(c), "its clips are deleted");
    }
    assert!(p.clips.contains_key(&kept));
    assert!(matches!(builtin(&h, sampler), BuiltinDevice::Sampler { sample: None, .. }));
    let BuiltinDevice::MultiSampler { zones } = builtin(&h, multi) else {
        panic!()
    };
    assert_eq!(zones.len(), 2, "zones kept, emptied");
    assert_eq!(zones[0].media, None);
    assert_eq!(zones[1].media, Some(keep));
    assert!(matches!(builtin(&h, reverb), BuiltinDevice::ConvolutionReverb { ir: None }));

    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(*h.project(), before, "one undo step puts everything back");
}

#[test]
fn removing_unused_media_and_refusals() {
    let mut lib = MemoryLibrary::new();
    lib.add_root("lib", "Library");
    let x = sine(SR, 440.0, SR as usize / 4, 0.5);
    lib.add_file("lib", "A.wav", wav(SR, &[x.clone()]));
    lib.add_file("lib", "B.wav", wav(SR, &[x]));
    let mut h = Harness::with(FakeBridge::default(), lib, Default::default());
    h.create_project("Media");
    let a = import(&mut h, "A.wav");
    let b = import(&mut h, "B.wav");
    assert!(h.project().media_users(a).is_empty());

    h.ok(remove(vec![a, b]));
    assert!(h.project().media.is_empty());
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(h.project().media.len(), 2);

    let out = h.send(remove(vec![]));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    let ghost: MediaId = h.id();
    let out = h.send(remove(vec![a, ghost]));
    assert_eq!(err(&out).code, ErrorCode::NotFound);
    assert!(h.project().media.contains_key(&a), "nothing removed on a refusal");
}

//! Clip envelopes of an audio clip's own gain and transposition
//! (`AutomationTarget::{ClipGain, ClipTranspose}`): validation, compilation into the clip's
//! content desc, and following the clip (duplicate, cross-track move).

mod common;

use common::*;
use ether_controller::compile::{CLIP_GAIN_MAPPING, CLIP_TRANSPOSE_MAPPING, track_param_info};
use ether_controller::memory::MemoryLibrary;
use ether_core::graph::{ClipContentDesc, ClipDesc};
use ether_core::protocol::automation::{AutomationCommand, PointSpec};
use ether_core::protocol::clips::{ClipCommand, ClipMove};
use ether_core::protocol::media::{BrowseLocation, MediaCommand, MediaSource};
use ether_core::protocol::model::*;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::*;

const SR: u32 = 48_000;

fn harness() -> Harness {
    let mut lib = MemoryLibrary::new();
    lib.add_root("lib", "Library");
    lib.add_file("lib", "a.wav", wav(SR, &[vec![0.25; SR as usize]]));
    let mut h = Harness::with(FakeBridge::default(), lib, Default::default());
    h.create_project("Clip envelopes");
    h
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

fn audio_clip(h: &mut Harness, track: TrackId) -> ClipId {
    let media: MediaId = h.id();
    h.ok(Command::Media(MediaCommand::Import {
        id: media,
        source: MediaSource::Location {
            location: BrowseLocation::Library { id: "lib".into() },
            path: "a.wav".into(),
        },
    }));
    let clip: ClipId = h.id();
    h.ok(Command::Clip(ClipCommand::CreateAudio {
        id: clip,
        track,
        start: Beats(0.0),
        media,
    }));
    h.drain_media();
    clip
}

fn create_lane(
    h: &mut Harness,
    owner: AutomationOwner,
    target: AutomationTarget,
) -> Vec<ServerMessage> {
    let id: AutomationLaneId = h.id();
    h.send(Command::Automation(AutomationCommand::CreateLane {
        id,
        owner,
        target,
    }))
}

fn lane(h: &mut Harness, clip: ClipId, target: AutomationTarget, points: &[(f64, f64)]) {
    let id: AutomationLaneId = h.id();
    h.ok(Command::Automation(AutomationCommand::CreateLane {
        id,
        owner: AutomationOwner::Clip { clip },
        target,
    }));
    let points = points
        .iter()
        .map(|&(t, v)| PointSpec {
            id: h.id(),
            time: Beats(t),
            value: v,
            curve: CurveShape::Linear,
        })
        .collect();
    h.ok(Command::Automation(AutomationCommand::AddPoints {
        lane: id,
        points,
    }));
}

fn compiled(h: &mut Harness, clip: ClipId) -> ClipDesc {
    h.tick();
    let g = h.ctl.bridge.last_graph();
    g.tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .find(|c| c.id == clip)
        .expect("clip compiled")
        .clone()
}

fn lanes_of(h: &Harness, clip: ClipId) -> Vec<AutomationLane> {
    h.project()
        .automation_lanes
        .values()
        .filter(|l| l.owner == AutomationOwner::Clip { clip })
        .cloned()
        .collect()
}

#[test]
fn only_audio_clips_have_gain_and_transposition_envelopes() {
    let mut h = harness();
    let midi = track(&mut h, TrackKind::Midi);
    let audio = track(&mut h, TrackKind::Audio);
    let m: ClipId = h.id();
    h.ok(Command::Clip(ClipCommand::CreateMidi {
        id: m,
        track: midi,
        start: Beats(0.0),
        length: Beats(4.0),
        name: None,
    }));
    for target in [AutomationTarget::ClipGain, AutomationTarget::ClipTranspose] {
        let out = create_lane(&mut h, AutomationOwner::Clip { clip: m }, target);
        assert_eq!(
            err(&out).code,
            ErrorCode::InvalidArgument,
            "{target:?} on MIDI"
        );
        let out = create_lane(&mut h, AutomationOwner::Track { track: audio }, target);
        assert_eq!(
            err(&out).code,
            ErrorCode::InvalidArgument,
            "{target:?} on a track"
        );
    }
    let a = audio_clip(&mut h, audio);
    ok(&create_lane(
        &mut h,
        AutomationOwner::Clip { clip: a },
        AutomationTarget::ClipGain,
    ));
}

#[test]
fn gain_and_transposition_envelopes_compile_into_the_clip() {
    let mut h = harness();
    let t = track(&mut h, TrackKind::Audio);
    let c = audio_clip(&mut h, t);
    // Nothing yet.
    match compiled(&mut h, c).content {
        ClipContentDesc::Audio {
            gain_envelope,
            transpose_envelope,
            ..
        } => assert!(gain_envelope.is_none() && transpose_envelope.is_none()),
        ClipContentDesc::Midi { .. } => unreachable!(),
    }
    lane(
        &mut h,
        c,
        AutomationTarget::ClipGain,
        &[(0.0, 0.2), (1.0, 0.9)],
    );
    lane(&mut h, c, AutomationTarget::ClipTranspose, &[(0.5, 0.75)]);
    let cd = compiled(&mut h, c);
    // They are the clip's own, not track targets.
    assert!(cd.envelopes.is_empty());
    let ClipContentDesc::Audio {
        gain_envelope: Some(gain),
        transpose_envelope: Some(tr),
        ..
    } = &cd.content
    else {
        panic!("{:?}", cd.content);
    };
    assert_eq!(gain.mapping, CLIP_GAIN_MAPPING);
    assert_eq!(
        gain.points.iter().map(|p| (p.0, p.1)).collect::<Vec<_>>(),
        [(0.0, 0.2), (1.0, 0.9)]
    );
    assert_eq!(tr.env.mapping, CLIP_TRANSPOSE_MAPPING);
    // 0.75 of ±48 st: +24.
    assert_eq!(tr.env.plain_at(3.0), 24.0);
    assert!(!tr.rate.cum.is_empty());

    // A disabled envelope is not compiled.
    let gain_lane = lanes_of(&h, c)
        .into_iter()
        .find(|l| l.target == AutomationTarget::ClipGain)
        .unwrap();
    h.ok(Command::Automation(AutomationCommand::SetLaneEnabled {
        id: gain_lane.id,
        enabled: false,
    }));
    match compiled(&mut h, c).content {
        ClipContentDesc::Audio { gain_envelope, .. } => assert!(gain_envelope.is_none()),
        ClipContentDesc::Midi { .. } => unreachable!(),
    }

    let info = track_param_info(&AutomationTarget::ClipTranspose).unwrap();
    assert_eq!((info.min, info.max, info.default), (-48.0, 48.0, 0.0));
    let info = track_param_info(&AutomationTarget::ClipGain).unwrap();
    assert_eq!((info.min, info.max, info.default), (-144.0, 6.0, 0.0));
}

#[test]
fn they_follow_the_clip() {
    let mut h = harness();
    let a = track(&mut h, TrackKind::Audio);
    let b = track(&mut h, TrackKind::Audio);
    let c = audio_clip(&mut h, a);
    lane(&mut h, c, AutomationTarget::ClipTranspose, &[(0.0, 0.25)]);

    // Duplicate: the copy has its own envelope.
    let copy: ClipId = h.id();
    h.ok(Command::Clip(ClipCommand::Duplicate {
        id: c,
        new_id: copy,
        start: None,
    }));
    let lanes = lanes_of(&h, copy);
    assert_eq!(lanes.len(), 1);
    assert_eq!(lanes[0].target, AutomationTarget::ClipTranspose);
    assert_eq!(h.project().points_of(lanes[0].id).len(), 1);

    // Moving to another track keeps it (no warning: it isn't a track target).
    let out = h.send(Command::Clip(ClipCommand::Move {
        moves: vec![ClipMove {
            id: c,
            track: b,
            start: Beats(0.0),
        }],
    }));
    ok(&out);
    assert!(!events(&out).iter().any(|e| matches!(
        e,
        Event::Notification {
            level: NotificationLevel::Warning,
            ..
        }
    )));
    assert_eq!(lanes_of(&h, c).len(), 1);
    let cd = compiled(&mut h, c);
    assert!(matches!(
        cd.content,
        ClipContentDesc::Audio {
            transpose_envelope: Some(_),
            ..
        }
    ));
}

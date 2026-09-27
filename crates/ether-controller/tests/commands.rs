//! End-to-end command handling: ordering, patches, undo/redo, gestures, batches,
//! idempotency, cascades.

mod common;

use common::*;
use ether_core::protocol::automation::{AutomationCommand, PointSpec};
use ether_core::protocol::clips::{ClipCommand, ClipMove};
use ether_core::protocol::devices::{DeviceCommand, DeviceSpec};
use ether_core::protocol::mixer::MixerCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteSpec};
use ether_core::protocol::project::{EditCommand, ProjectCommand, ProjectEvent};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::transport::TransportCommand;
use ether_core::protocol::*;

fn create_track(h: &mut Harness, kind: TrackKind) -> TrackId {
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

fn midi_clip(h: &mut Harness, track: TrackId, start: f64, length: f64) -> ClipId {
    let id = h.id();
    h.ok(Command::Clip(ClipCommand::CreateMidi {
        id,
        track,
        start: Beats(start),
        length: Beats(length),
        name: None,
    }));
    id
}

#[test]
fn scales_are_undoable_metadata_and_never_restrict_notes() {
    let mut h = Harness::with_project();
    let track = create_track(&mut h, TrackKind::Midi);
    let clip = midi_clip(&mut h, track, 0.0, 4.0);
    assert_eq!(h.project().settings.scale, MusicalScale::default());
    assert_eq!(h.project().tracks[&track].scale, TrackScale::FollowProject);
    let minor = MusicalScale {
        root: 0,
        kind: ScaleKind::Minor,
    };
    let out = h.send(Command::Project(ProjectCommand::SetScale { scale: minor }));
    ok(&out);
    assert!(
        matches!(&patches(&out)[0].changes[0], PatchChange::Settings { settings } if settings.scale == minor)
    );
    let custom = TrackScale::Custom {
        scale: MusicalScale {
            root: 9,
            kind: ScaleKind::Dorian,
        },
    };
    h.ok(Command::Track(TrackCommand::SetScale {
        id: track,
        scale: custom,
    }));
    assert_eq!(h.project().tracks[&track].scale, custom);
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(h.project().tracks[&track].scale, TrackScale::FollowProject);
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(h.project().settings.scale, MusicalScale::default());
    h.ok(Command::Edit(EditCommand::Redo));
    h.ok(Command::Edit(EditCommand::Redo));
    let note = h.id();
    h.ok(Command::Note(NoteCommand::Add {
        clip,
        notes: vec![NoteSpec {
            id: note,
            pitch: 61,
            velocity: 0.8,
            start: Beats::ZERO,
            duration: Beats(1.0),
        }],
    }));
    assert_eq!(h.project().notes[&note].pitch, 61);
    let copy = h.id();
    h.ok(Command::Track(TrackCommand::Duplicate {
        id: track,
        new_id: copy,
    }));
    assert_eq!(h.project().tracks[&copy].scale, custom);
    let original = h.project().clone();
    h.ok(Command::Project(ProjectCommand::Save));
    h.create_project("Other");
    h.ok(Command::Project(ProjectCommand::Open { id: original.id }));
    assert_eq!(h.project(), &original);
    for scale in [TrackScale::Chromatic, TrackScale::FollowProject] {
        h.ok(Command::Track(TrackCommand::SetScale { id: track, scale }));
        assert_eq!(h.project().tracks[&track].scale, scale);
        assert_eq!(h.project().notes[&note].pitch, 61);
    }
}

#[test]
fn invalid_scales_are_rejected_atomically() {
    let mut h = Harness::with_project();
    let track = create_track(&mut h, TrackKind::Midi);
    let audio = create_track(&mut h, TrackKind::Audio);
    let original = h.project().clone();
    let invalid = MusicalScale {
        root: 12,
        kind: ScaleKind::Major,
    };
    for command in [
        Command::Project(ProjectCommand::SetScale { scale: invalid }),
        Command::Track(TrackCommand::SetScale {
            id: track,
            scale: TrackScale::Custom { scale: invalid },
        }),
        Command::Track(TrackCommand::SetScale {
            id: audio,
            scale: TrackScale::Chromatic,
        }),
    ] {
        let out = h.send(command);
        assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
        assert!(patches(&out).is_empty());
        assert_eq!(h.project(), &original);
    }
}

#[test]
fn patch_then_reply_and_monotonic_revisions() {
    let mut h = Harness::with_project();
    let id: TrackId = h.id();
    let out = h.send(Command::Track(TrackCommand::Create {
        id,
        kind: TrackKind::Audio,
        name: Some("Drums".into()),
        color: None,
        parent: None,
        before: None,
    }));
    // Patch first, reply last; the dirty flag flips.
    assert!(matches!(out[0], ServerMessage::Event(Event::Patch { .. })));
    let evs = events(&out);
    assert!(evs.contains(&Event::Project {
        event: ProjectEvent::DirtyChanged { dirty: true }
    }));
    let p1 = patches(&out);
    assert_eq!(p1.len(), 1);
    assert!(
        matches!(&p1[0].changes[0], PatchChange::Upsert { entity: Entity::Track(t) } if t.id == id && t.name == "Drums")
    );
    assert!(p1[0].history.can_undo);
    assert_eq!(p1[0].history.undo_label.as_deref(), Some("Create"));

    let out = h.send(Command::Track(TrackCommand::Rename {
        id,
        name: "Beat".into(),
    }));
    let p2 = patches(&out);
    assert_eq!(p2[0].revision, p1[0].revision + 1);

    // The UI mirror, fed only with patches, equals the document.
    let mut mirror = h.project().clone();
    mirror.tracks.remove(&id);
    mirror.apply_patch_changes(&p1[0].changes);
    mirror.apply_patch_changes(&p2[0].changes);
    assert_eq!(&mirror, h.project());
}

#[test]
fn errors_change_nothing() {
    let mut h = Harness::with_project();
    let before = h.project().clone();
    let missing: TrackId = h.id();
    let out = h.send(Command::Mixer(MixerCommand::SetVolume {
        track: missing,
        volume: Decibels(-6.0),
    }));
    assert_eq!(err(&out).code, ErrorCode::NotFound);
    assert!(patches(&out).is_empty());
    assert_eq!(h.project(), &before);

    // A batch fails as a whole.
    let a: TrackId = h.id();
    let out = h.send(Command::Edit(EditCommand::Batch {
        label: "Two".into(),
        commands: vec![
            Command::Track(TrackCommand::Create {
                id: a,
                kind: TrackKind::Midi,
                name: None,
                color: None,
                parent: None,
                before: None,
            }),
            Command::Mixer(MixerCommand::SetPan {
                track: missing,
                pan: Pan(0.5),
            }),
        ],
    }));
    assert_eq!(err(&out).code, ErrorCode::NotFound);
    assert_eq!(h.project(), &before);
}

#[test]
fn no_project_is_invalid_state() {
    let mut h = Harness::new();
    let t: TrackId = h.id();
    let out = h.send(Command::Track(TrackCommand::Delete { id: t }));
    assert_eq!(err(&out).code, ErrorCode::InvalidState);
}

#[test]
fn idempotent_creates() {
    let mut h = Harness::with_project();
    let t = create_track(&mut h, TrackKind::Midi);
    let cmd = Command::Track(TrackCommand::Create {
        id: t,
        kind: TrackKind::Midi,
        name: None,
        color: None,
        parent: None,
        before: None,
    });
    let out = h.send(cmd);
    ok(&out);
    assert!(patches(&out).is_empty(), "a retried create changes nothing");

    let c = midi_clip(&mut h, t, 0.0, 4.0);
    let out = h.send(Command::Clip(ClipCommand::CreateMidi {
        id: c,
        track: t,
        start: Beats(8.0),
        length: Beats(4.0),
        name: None,
    }));
    ok(&out);
    assert!(patches(&out).is_empty());
    assert_eq!(h.project().clips[&c].start, Beats(0.0));
}

#[test]
fn gestures_merge_into_one_undo_step() {
    let mut h = Harness::with_project();
    let t = create_track(&mut h, TrackKind::Audio);
    let g = GestureId(5);
    for v in [-1.0, -2.0, -3.0] {
        let out = h.send_with(
            Command::Mixer(MixerCommand::SetVolume {
                track: t,
                volume: Decibels(v),
            }),
            Some(g),
        );
        ok(&out);
    }
    h.ok(Command::Edit(EditCommand::EndGesture { gesture: g }));
    // A new gesture with the same id after EndGesture is a new step.
    h.send_with(
        Command::Mixer(MixerCommand::SetVolume {
            track: t,
            volume: Decibels(-4.0),
        }),
        Some(g),
    );
    assert_eq!(h.project().tracks[&t].mixer.volume, Decibels(-4.0));
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(h.project().tracks[&t].mixer.volume, Decibels(-3.0));
    let out = h.send(Command::Edit(EditCommand::Undo));
    let p = patches(&out);
    assert_eq!(p.len(), 1);
    assert_eq!(h.project().tracks[&t].mixer.volume, Decibels(0.0));
    assert_eq!(p[0].history.redo_label.as_deref(), Some("Set Volume"));
    h.ok(Command::Edit(EditCommand::Redo));
    assert_eq!(h.project().tracks[&t].mixer.volume, Decibels(-3.0));
    // Undo the track creation, then nothing is left.
    h.ok(Command::Edit(EditCommand::Undo));
    h.ok(Command::Edit(EditCommand::Undo));
    let out = h.send(Command::Edit(EditCommand::Undo));
    assert_eq!(err(&out).code, ErrorCode::InvalidState);
}

#[test]
fn batch_is_one_undo_step_and_rejects_non_document_commands() {
    let mut h = Harness::with_project();
    let t: TrackId = h.id();
    let d: DeviceId = h.id();
    let out = h.send(Command::Edit(EditCommand::Batch {
        label: "New MIDI track".into(),
        commands: vec![
            Command::Track(TrackCommand::Create {
                id: t,
                kind: TrackKind::Midi,
                name: None,
                color: None,
                parent: None,
                before: None,
            }),
            Command::Device(DeviceCommand::Insert {
                id: d,
                track: t,
                device: DeviceSpec::Builtin {
                    device: BuiltinDevice::Synth,
                },
                before: None,
            }),
        ],
    }));
    let p = patches(&out);
    assert_eq!(p.len(), 1);
    assert_eq!(p[0].changes.len(), 2);
    assert_eq!(p[0].history.undo_label.as_deref(), Some("New MIDI track"));
    assert!(h.project().devices.contains_key(&d));
    h.ok(Command::Edit(EditCommand::Undo));
    assert!(!h.project().tracks.contains_key(&t));
    assert!(!h.project().devices.contains_key(&d));

    let out = h.send(Command::Edit(EditCommand::Batch {
        label: "x".into(),
        commands: vec![Command::Transport(TransportCommand::Play)],
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
}

#[test]
fn instruments_only_on_midi_tracks() {
    let mut h = Harness::with_project();
    let t = create_track(&mut h, TrackKind::Audio);
    let d: DeviceId = h.id();
    let out = h.send(Command::Device(DeviceCommand::Insert {
        id: d,
        track: t,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::Synth,
        },
        before: None,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
}

#[test]
fn delete_track_cascades_and_undo_restores() {
    let mut h = Harness::with_project();
    let group = create_track(&mut h, TrackKind::Group);
    let ret = create_track(&mut h, TrackKind::Return);
    let t: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id: t,
        kind: TrackKind::Midi,
        name: None,
        color: None,
        parent: Some(group),
        before: None,
    }));
    let d: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id: d,
        track: t,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::Synth,
        },
        before: None,
    }));
    let c = midi_clip(&mut h, t, 0.0, 4.0);
    let n: NoteId = h.id();
    h.ok(Command::Note(NoteCommand::Add {
        clip: c,
        notes: vec![NoteSpec {
            id: n,
            pitch: 60,
            velocity: 0.8,
            start: Beats(0.0),
            duration: Beats(1.0),
        }],
    }));
    let s: SendId = h.id();
    h.ok(Command::Mixer(MixerCommand::CreateSend {
        id: s,
        from: t,
        to: ret,
        level: Decibels(-6.0),
        pre_fader: false,
    }));
    let lane: AutomationLaneId = h.id();
    h.ok(Command::Automation(AutomationCommand::CreateLane {
        id: lane,
        owner: AutomationOwner::Track { track: t },
        target: AutomationTarget::DeviceParam {
            device: d,
            param: ParamId(0),
        },
    }));
    let pt: AutomationPointId = h.id();
    h.ok(Command::Automation(AutomationCommand::AddPoints {
        lane,
        points: vec![PointSpec {
            id: pt,
            time: Beats(0.0),
            value: 0.5,
            curve: CurveShape::Linear,
        }],
    }));
    let before = h.project().clone();

    // Deleting the group deletes its child and everything on it.
    h.ok(Command::Track(TrackCommand::Delete { id: group }));
    let p = h.project();
    assert!(!p.tracks.contains_key(&group) && !p.tracks.contains_key(&t));
    assert!(p.clips.is_empty() && p.notes.is_empty() && p.devices.is_empty());
    assert!(p.sends.is_empty() && p.automation_lanes.is_empty() && p.automation_points.is_empty());
    assert!(p.tracks.contains_key(&ret));
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(h.project(), &before);
}

#[test]
fn duplicate_send_route_is_a_noop() {
    let mut h = Harness::with_project();
    let t = create_track(&mut h, TrackKind::Audio);
    let r = create_track(&mut h, TrackKind::Return);
    let (s1, s2): (SendId, SendId) = (h.id(), h.id());
    for s in [s1, s2] {
        let out = h.send(Command::Mixer(MixerCommand::CreateSend {
            id: s,
            from: t,
            to: r,
            level: Decibels(0.0),
            pre_fader: false,
        }));
        ok(&out);
    }
    assert_eq!(h.project().sends.len(), 1);
    assert!(h.project().sends.contains_key(&s1));
}

#[test]
fn clip_move_trims_overlaps_and_split_keeps_content() {
    let mut h = Harness::with_project();
    let t = create_track(&mut h, TrackKind::Midi);
    let a = midi_clip(&mut h, t, 0.0, 8.0);
    let b = midi_clip(&mut h, t, 16.0, 4.0);
    // Move b into the middle of a: a is split around it.
    h.ok(Command::Clip(ClipCommand::Move {
        moves: vec![ClipMove {
            id: b,
            track: t,
            start: Beats(2.0),
        }],
    }));
    let p = h.project();
    assert_eq!(p.clips.len(), 3);
    assert_eq!(p.clips[&a].length, Beats(2.0));
    let right = p.clips.values().find(|c| c.id != a && c.id != b).unwrap();
    assert_eq!(right.start, Beats(6.0));
    assert_eq!(right.length, Beats(2.0));
    assert_eq!(right.offset, Beats(6.0));

    // Split.
    let r: ClipId = h.id();
    h.ok(Command::Clip(ClipCommand::Split {
        id: b,
        at: Beats(3.0),
        new_id: r,
    }));
    assert_eq!(h.project().clips[&b].length, Beats(1.0));
    assert_eq!(h.project().clips[&r].start, Beats(3.0));
    assert_eq!(h.project().clips[&r].offset, Beats(1.0));

    // Creating a clip that covers everything removes the others.
    midi_clip(&mut h, t, 0.0, 32.0);
    assert_eq!(h.project().clips.len(), 1);
}

#[test]
fn note_editing_and_quantize() {
    let mut h = Harness::with_project();
    let t = create_track(&mut h, TrackKind::Midi);
    let c = midi_clip(&mut h, t, 0.0, 4.0);
    let (n1, n2): (NoteId, NoteId) = (h.id(), h.id());
    h.ok(Command::Note(NoteCommand::Add {
        clip: c,
        notes: vec![
            NoteSpec {
                id: n1,
                pitch: 60,
                velocity: 2.0,
                start: Beats(0.1),
                duration: Beats(0.5),
            },
            NoteSpec {
                id: n2,
                pitch: 64,
                velocity: 0.5,
                start: Beats(1.9),
                duration: Beats(1.0),
            },
        ],
    }));
    assert_eq!(h.project().notes[&n1].velocity, 1.0);
    let out = h.send(Command::Note(NoteCommand::Quantize {
        clip: c,
        notes: None,
        grid: Beats(1.0),
        strength: 1.0,
        ends: false,
        swing: 0.0,
    }));
    ok(&out);
    assert!(h.project().notes[&n1].start.approx_eq(Beats(0.0)));
    assert!(h.project().notes[&n2].start.approx_eq(Beats(2.0)));
    let bad_id: NoteId = h.id();
    let bad = h.send(Command::Note(NoteCommand::Add {
        clip: c,
        notes: vec![NoteSpec {
            id: bad_id,
            pitch: 200,
            velocity: 0.5,
            start: Beats(0.0),
            duration: Beats(1.0),
        }],
    }));
    assert_eq!(err(&bad).code, ErrorCode::InvalidArgument);
}

#[test]
fn track_duplicate_deep_copies_and_retargets_lanes() {
    let mut h = Harness::with_project();
    let t = create_track(&mut h, TrackKind::Midi);
    let d: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id: d,
        track: t,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::Synth,
        },
        before: None,
    }));
    let c = midi_clip(&mut h, t, 0.0, 4.0);
    let nid: NoteId = h.id();
    h.ok(Command::Note(NoteCommand::Add {
        clip: c,
        notes: vec![NoteSpec {
            id: nid,
            pitch: 60,
            velocity: 0.5,
            start: Beats(0.0),
            duration: Beats(1.0),
        }],
    }));
    let lane: AutomationLaneId = h.id();
    h.ok(Command::Automation(AutomationCommand::CreateLane {
        id: lane,
        owner: AutomationOwner::Track { track: t },
        target: AutomationTarget::DeviceParam {
            device: d,
            param: ParamId(1),
        },
    }));
    let copy: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Duplicate {
        id: t,
        new_id: copy,
    }));
    let p = h.project();
    assert_eq!(p.tracks.len(), 3);
    let new_dev = p.devices_of(copy)[0].id;
    assert_ne!(new_dev, d);
    assert_eq!(p.clips.values().filter(|c| c.track == copy).count(), 1);
    assert_eq!(p.notes.len(), 2);
    let lanes: Vec<_> = p
        .automation_lanes
        .values()
        .filter(|l| l.owner == AutomationOwner::Track { track: copy })
        .collect();
    assert_eq!(lanes.len(), 1);
    assert_eq!(
        lanes[0].target,
        AutomationTarget::DeviceParam {
            device: new_dev,
            param: ParamId(1)
        }
    );
    // Order: right after the original.
    let top: Vec<TrackId> = p.tracks_ordered().iter().map(|t| t.id).collect();
    let i = top.iter().position(|x| *x == t).unwrap();
    assert_eq!(top[i + 1], copy);
}

#[test]
fn set_tempo_and_loop_are_undoable_and_emit_transport() {
    let mut h = Harness::with_project();
    let out = h.send(Command::Transport(TransportCommand::SetTempo { bpm: 90.0 }));
    let evs = events(&out);
    assert!(matches!(evs[0], Event::Patch { .. }));
    assert!(
        evs.iter()
            .any(|e| matches!(e, Event::Transport { state } if state.bpm == 90.0))
    );
    h.ok(Command::Transport(TransportCommand::SetLoopRegion {
        region: BeatRange {
            start: Beats(4.0),
            end: Beats(8.0),
        },
    }));
    h.ok(Command::Edit(EditCommand::Undo));
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(h.project().tempo_map().bpm_at(Beats(0.0)), 120.0);
}

#[test]
fn host_handled_and_unsupported_commands() {
    use ether_core::protocol::engine::EngineCommand;
    use ether_core::protocol::media::MediaCommand;
    use ether_core::protocol::plugins::PluginCommand;
    use ether_core::protocol::recording::RecordingCommand;
    let mut h = Harness::with_project();
    for c in [
        Command::Engine(EngineCommand::GetStatus),
        Command::Plugin(PluginCommand::Rescan),
        Command::Plugin(PluginCommand::List),
        Command::Media(MediaCommand::StopPreview),
        Command::Recording(RecordingCommand::ListInputs),
    ] {
        let out = h.send(c);
        assert_eq!(err(&out).code, ErrorCode::Unsupported);
    }
}

#[test]
fn rename_current_project_is_undoable() {
    use ether_core::protocol::project::ProjectCommand;
    let mut h = Harness::with_project();
    let id = h.project().id;
    let out = h.send(Command::Project(ProjectCommand::Rename {
        id,
        name: "Song".into(),
    }));
    ok(&out);
    assert!(events(&out).iter().any(|e| matches!(e, Event::Project { event: ProjectEvent::ListChanged { projects } } if projects[0].name == "Song")));
    assert_eq!(h.project().settings.name, "Song");
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(h.project().settings.name, "Test");
}

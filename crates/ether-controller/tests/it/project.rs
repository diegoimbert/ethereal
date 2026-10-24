//! Project lifecycle through the `ProjectStore`.

use crate::common;

use common::*;
use ether_controller::ControllerConfig;
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::devices::{DeviceCommand, DeviceSpec};
use ether_core::protocol::mixer::MixerCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteSpec};
use ether_core::protocol::project::{EditCommand, ProjectCommand, ProjectEvent};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::*;

fn populate(h: &mut Harness) {
    let t: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id: t,
        kind: TrackKind::Midi,
        name: Some("Keys".into()),
        color: None,
        parent: None,
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
    let c: ClipId = h.id();
    h.ok(Command::Clip(ClipCommand::CreateMidi {
        id: c,
        track: t,
        start: Beats(1.0 / 3.0),
        length: Beats(4.0),
        name: Some("Riff".into()),
    }));
    let n: NoteId = h.id();
    h.ok(Command::Note(NoteCommand::Add {
        clip: c,
        notes: vec![NoteSpec {
            id: n,
            pitch: 67,
            velocity: 0.7,
            start: Beats(0.1),
            duration: Beats(0.25),
        }],
    }));
    h.ok(Command::Mixer(MixerCommand::SetVolume {
        track: t,
        volume: Decibels(-3.5),
    }));
}

#[test]
fn create_announces_and_stores_the_project() {
    let mut h = Harness::new();
    let id = h.project_id();
    let out = h.send(Command::Project(ProjectCommand::Create {
        id,
        name: "  My Song ".into(),
    }));
    let v = ok(&out);
    assert!(
        matches!(&v, ReplyValue::Project { project } if project.id == id && project.settings.name == "My Song")
    );
    let evs = events(&out);
    assert!(matches!(&evs[0], Event::ProjectLoaded { project } if project.id == id));
    assert!(evs.iter().any(|e| matches!(e, Event::Transport { .. })));
    assert!(evs.iter().any(|e| matches!(e, Event::Project { event: ProjectEvent::ListChanged { projects } } if projects.len() == 1 && projects[0].name == "My Song")));
    assert!(h.ctl.store.file(id, "project.ether").is_some());
    // Retried create: same project, no error.
    let out = h.send(Command::Project(ProjectCommand::Create {
        id,
        name: "My Song".into(),
    }));
    ok(&out);
    // Empty name.
    let other = h.project_id();
    let out = h.send(Command::Project(ProjectCommand::Create {
        id: other,
        name: " ".into(),
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
}

#[test]
fn save_and_reopen_roundtrip() {
    let mut h = Harness::with_project();
    populate(&mut h);
    assert!(h.ctl.is_dirty());
    let doc = h.project().clone();
    h.advance(1_000);
    let out = h.send(Command::Project(ProjectCommand::Save { autosave: false }));
    let v = ok(&out);
    assert!(
        matches!(&v, ReplyValue::Saved { project } if project.id == doc.id && project.modified_ms == (T0 + 1_000) as f64)
    );
    let evs = events(&out);
    assert!(evs.contains(&Event::Project {
        event: ProjectEvent::DirtyChanged { dirty: false }
    }));
    assert!(evs.iter().any(|e| matches!(
        e,
        Event::Project {
            event: ProjectEvent::Saved { .. }
        }
    )));

    // Switch to another project, then back.
    let other = h.create_project("Other");
    assert_eq!(h.project().id, other);
    h.tick();
    assert!(
        h.ctl.bridge.live.is_empty(),
        "nodes of the closed project are destroyed"
    );
    let out = h.send(Command::Project(ProjectCommand::Open { id: doc.id }));
    let v = ok(&out);
    assert!(matches!(&v, ReplyValue::Project { project } if **project == doc));
    assert_eq!(h.project(), &doc);
    assert!(!h.ctl.is_dirty());
    // History does not survive a reopen.
    let out = h.send(Command::Edit(EditCommand::Undo));
    assert_eq!(err(&out).code, ErrorCode::InvalidState);
    // Engine nodes were rebuilt for the reopened document.
    h.tick();
    assert_eq!(h.ctl.bridge.live.len(), 1);
    assert_eq!(h.ctl.bridge.last_graph().tracks.len(), 2);
}

#[test]
fn opening_another_project_autosaves_a_dirty_one() {
    let mut h = Harness::with_project();
    let first = h.project().id;
    let other = h.create_project("Other");
    h.send(Command::Project(ProjectCommand::Open { id: first }));
    populate(&mut h);
    let edited = h.project().clone();
    let out = h.send(Command::Project(ProjectCommand::Open { id: other }));
    assert!(events(&out).iter().any(|e| matches!(e, Event::Project { event: ProjectEvent::Saved { project } } if project.id == first)));
    h.ok(Command::Project(ProjectCommand::Open { id: first }));
    assert_eq!(h.project(), &edited);
}

#[test]
fn autosave_after_idle() {
    let config = ControllerConfig {
        autosave_after_ms: Some(5_000),
        ..Default::default()
    };
    let mut h = Harness::with(FakeBridge::default(), Default::default(), config);
    h.create_project("A");
    populate(&mut h);
    h.advance(1_000);
    assert!(!events(&h.tick()).iter().any(|e| matches!(
        e,
        Event::Project {
            event: ProjectEvent::Saved { .. }
        }
    )));
    h.advance(5_000);
    let out = h.tick();
    assert!(events(&out).iter().any(|e| matches!(
        e,
        Event::Project {
            event: ProjectEvent::Saved { .. }
        }
    )));
    assert!(!h.ctl.is_dirty());
    let id = h.project().id;
    let json = std::str::from_utf8(h.ctl.store.file(id, "project.ether").unwrap())
        .unwrap()
        .to_string();
    assert_eq!(
        &ether_core::protocol::model::file::load(&json).unwrap(),
        h.project()
    );
}

/// A save reads every plugin's state on the main thread: autosave waits for the transport
/// to stop (and saves anyway after 10 minutes of playback).
#[test]
fn autosave_waits_while_playing() {
    use ether_core::protocol::transport::TransportCommand;
    let saved = |out: &[ServerMessage]| {
        events(out).iter().any(|e| {
            matches!(
                e,
                Event::Project {
                    event: ProjectEvent::Saved { .. }
                }
            )
        })
    };
    let config = ControllerConfig {
        autosave_after_ms: Some(5_000),
        ..Default::default()
    };
    let mut h = Harness::with(FakeBridge::default(), Default::default(), config);
    h.create_project("A");
    populate(&mut h);
    h.ok(Command::Transport(TransportCommand::Play));
    h.advance(6_000);
    assert!(!saved(&h.tick()));
    h.ok(Command::Transport(TransportCommand::Stop));
    assert!(saved(&h.tick()));

    populate(&mut h);
    h.ok(Command::Transport(TransportCommand::Play));
    h.advance(6_000);
    assert!(!saved(&h.tick()));
    h.advance(10 * 60_000);
    assert!(saved(&h.tick()));
}

#[test]
fn list_rename_duplicate_delete_save_as() {
    let mut h = Harness::with_project();
    let a = h.project().id;
    populate(&mut h);
    h.ok(Command::Project(ProjectCommand::Save { autosave: false }));
    let b = h.create_project("B");

    // Rename a stored (not open) project.
    h.ok(Command::Project(ProjectCommand::Rename {
        id: a,
        name: "Renamed".into(),
    }));
    let ReplyValue::Projects { projects } = h.ok(Command::Project(ProjectCommand::List)) else {
        panic!()
    };
    assert_eq!(projects.len(), 2);
    assert!(projects.iter().any(|p| p.id == a && p.name == "Renamed"));

    // Duplicate a stored project (not opened).
    let c = h.project_id();
    let v = h.ok(Command::Project(ProjectCommand::Duplicate {
        id: a,
        new_id: c,
        name: "Copy".into(),
    }));
    let ReplyValue::Saved { project: copy } = v else {
        panic!("{v:?}")
    };
    assert_eq!((copy.id, copy.name.as_str()), (c, "Copy"));
    // A fork: it records where it came from (the list shows it), the original doesn't.
    let origin = copy.forked_from.clone().expect("forked from");
    assert_eq!((origin.project, origin.name.as_str()), (a, "Renamed"));
    let ReplyValue::Projects { projects } = h.ok(Command::Project(ProjectCommand::List)) else {
        panic!()
    };
    let listed = |id| {
        projects
            .iter()
            .find(|p| p.id == id)
            .unwrap()
            .forked_from
            .clone()
    };
    assert_eq!(listed(c), Some(origin));
    assert_eq!(listed(a), None);
    assert_eq!(h.project().id, b, "duplicate does not open the copy");
    // Retried duplicate is a no-op.
    h.ok(Command::Project(ProjectCommand::Duplicate {
        id: a,
        new_id: c,
        name: "Copy".into(),
    }));

    // Deleting the open project is refused; others are deleted.
    let out = h.send(Command::Project(ProjectCommand::Delete { id: b }));
    assert_eq!(err(&out).code, ErrorCode::InvalidState);
    let out = h.send(Command::Project(ProjectCommand::Delete { id: c }));
    ok(&out);
    assert!(!h.ctl.store.contains(c));
    assert!(events(&out).iter().any(|e| matches!(e, Event::Project { event: ProjectEvent::ListChanged { projects } } if projects.len() == 2)));

    // Save As switches to the copy, which has the current (unsaved) state.
    h.ok(Command::Project(ProjectCommand::Open { id: a }));
    let t: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id: t,
        kind: TrackKind::Audio,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    let d = h.project_id();
    let out = h.send(Command::Project(ProjectCommand::SaveAs {
        new_id: d,
        name: "Version 2".into(),
    }));
    ok(&out);
    assert!(matches!(&events(&out)[0], Event::ProjectLoaded { project } if project.id == d));
    assert_eq!(h.project().id, d);
    assert_eq!(h.project().settings.name, "Version 2");
    let origin = h.project().settings.forked_from.clone().expect("a fork");
    assert_eq!((origin.project, origin.name.as_str()), (a, "Renamed"));
    // Autosaves of the fork keep it on its summary.
    let out = h.send(Command::Project(ProjectCommand::Save { autosave: true }));
    assert!(
        matches!(ok(&out), ReplyValue::Saved { project } if project.forked_from.as_ref() == Some(&origin))
    );
    assert!(h.project().tracks.contains_key(&t));
    assert!(!h.ctl.is_dirty());
    // The original on disk doesn't have the new track.
    let orig = std::str::from_utf8(h.ctl.store.file(a, "project.ether").unwrap())
        .unwrap()
        .to_string();
    assert!(
        !ether_core::protocol::model::file::load(&orig)
            .unwrap()
            .tracks
            .contains_key(&t)
    );
}

#[test]
fn get_resends_runtime_state() {
    let mut h = Harness::with_project();
    let out = h.send(Command::Project(ProjectCommand::Get));
    assert!(matches!(ok(&out), ReplyValue::Project { .. }));
    let evs = events(&out);
    assert!(evs.iter().any(|e| matches!(e, Event::Transport { .. })));
    assert!(evs.iter().any(|e| matches!(e, Event::Recording { .. })));
}

/// Save As and Duplicate make private copies: never an extension's private files
/// (`store::set_store_ext`), whose summary the project list carries.
#[test]
fn copies_never_carry_an_extensions_private_files() {
    use ether_controller::store::ProjectStore;
    use ether_controller::store::test_store_ext::{self, PRIVATE};
    test_store_ext::install();

    let mut h = Harness::with_project();
    let a = h.project().id;
    h.ok(Command::Project(ProjectCommand::Save { autosave: false }));
    h.ctl.store.write(a, PRIVATE, br#"{"badge":1}"#).unwrap();
    let ReplyValue::Projects { projects } = h.ok(Command::Project(ProjectCommand::List)) else {
        panic!()
    };
    assert!(projects[0].ext.is_some());

    // Duplicate (not opened).
    let c = h.project_id();
    let v = h.ok(Command::Project(ProjectCommand::Duplicate {
        id: a,
        new_id: c,
        name: "Copy".into(),
    }));
    assert!(matches!(v, ReplyValue::Saved { project } if project.ext.is_none()));
    assert!(h.ctl.store.file(c, PRIVATE).is_none());

    // Save As (switches to the copy).
    let d = h.project_id();
    let out = h.send(Command::Project(ProjectCommand::SaveAs {
        new_id: d,
        name: "Version 2".into(),
    }));
    ok(&out);
    assert!(h.ctl.store.file(d, PRIVATE).is_none());
    assert!(
        h.ctl.store.file(a, PRIVATE).is_some(),
        "the original keeps it"
    );
    let ReplyValue::Projects { projects } = h.ok(Command::Project(ProjectCommand::List)) else {
        panic!()
    };
    let with: Vec<_> = projects
        .iter()
        .filter(|p| p.ext.is_some())
        .map(|p| p.id)
        .collect();
    assert_eq!(with, [a]);
}

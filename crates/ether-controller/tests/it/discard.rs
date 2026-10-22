//! `Project::Discard`: the user discards a project (an untitled one, on leaving it).

use crate::common;

use common::*;
use ether_core::protocol::model::*;
use ether_core::protocol::project::{ProjectCommand, ProjectEvent};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::versions::VersionCommand;
use ether_core::protocol::*;

const MARKER: &str = "versions/.session";

fn add_track(h: &mut Harness) {
    let id: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id,
        kind: TrackKind::Midi,
        name: Some("Keys".into()),
        color: None,
        parent: None,
        before: None,
    }));
}

fn discard(h: &mut Harness, id: ProjectId) -> Vec<ServerMessage> {
    let out = h.send(Command::Project(ProjectCommand::Discard { id }));
    ok(&out);
    out
}

#[test]
fn a_stored_project_is_deleted_at_once() {
    let mut h = Harness::new();
    let a = h.create_project("A");
    h.create_project("B");
    let out = discard(&mut h, a);
    assert!(!h.ctl.store.contains(a));
    assert!(events(&out).iter().any(|e| matches!(e, Event::Project { event: ProjectEvent::ListChanged { projects } } if projects.len() == 1)));
}

#[test]
fn the_open_project_is_never_saved_again_and_deleted_when_another_opens() {
    let mut h = Harness::new();
    let a = h.create_project("A");
    h.ok(Command::Version(VersionCommand::Create {
        name: Some("v1".into()),
    }));
    add_track(&mut h);
    let saved = h.ctl.store.file(a, "project.ether").map(<[u8]>::to_vec);
    discard(&mut h, a);
    assert!(h.ctl.store.contains(a), "kept while it is open");
    // No save, explicit or automatic, writes it.
    let out = h.send(Command::Project(ProjectCommand::Save { autosave: false }));
    assert_eq!(err(&out).code, ErrorCode::InvalidState);
    add_track(&mut h);
    h.advance(60 * 60 * 1000);
    h.tick();
    assert_eq!(
        h.ctl.store.file(a, "project.ether").map(<[u8]>::to_vec),
        saved
    );
    // Opening another project deletes it: document, versions and recovery marker.
    let b = h.create_project("B");
    assert!(!h.ctl.store.contains(a));
    assert!(h.ctl.store.file(a, MARKER).is_none());
    assert_eq!(h.project().id, b);
    assert!(h.ctl.discarded.is_none());
    // B is an ordinary project.
    add_track(&mut h);
    h.ok(Command::Project(ProjectCommand::Save { autosave: false }));
}

#[test]
fn the_open_project_is_deleted_on_quit_instead_of_saved() {
    let mut h = Harness::new();
    let keep = h.create_project("Kept");
    let a = h.create_project("A");
    add_track(&mut h);
    discard(&mut h, a);
    let mut out = Vec::new();
    assert!(h.ctl.save_before_close(&mut out).is_ok());
    assert!(!h.ctl.store.contains(a));
    assert!(h.ctl.store.contains(keep));
}

#[test]
fn without_a_discard_quitting_saves_as_before() {
    let mut h = Harness::new();
    let a = h.create_project("A");
    add_track(&mut h);
    let mut out = Vec::new();
    assert!(h.ctl.save_before_close(&mut out).is_ok());
    assert!(h.ctl.store.contains(a));
    assert!(!h.ctl.is_dirty());
}

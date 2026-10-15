//! #561: a routine autosave is written in the background and says `Saved` before the write
//! lands. When that write fails, the project must not be closed (another project opened,
//! the host quitting) without saving it again, or without telling the caller.

mod common;

use common::*;
use ether_core::protocol::model::*;
use ether_core::protocol::project::ProjectCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::*;

/// A project with one saved track, then an edit (a second track) whose routine autosave
/// was accepted but failed in the background.
fn failed_autosave() -> (Harness, ProjectId, TrackId) {
    let mut h = Harness::new();
    let first = h.create_project("First");
    let edit = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id: edit,
        kind: TrackKind::Audio,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    h.ctl.store.fail_saves = true;
    h.ok(Command::Project(ProjectCommand::Save { autosave: true }));
    // Accepted: the controller believes it is saved.
    assert!(!h.ctl.is_dirty());
    assert!(!stored_tracks(&h, first).contains(&edit), "never written");
    (h, first, edit)
}

fn stored_tracks(h: &Harness, id: ProjectId) -> Vec<TrackId> {
    let json = std::str::from_utf8(h.ctl.store.file(id, "project.ether").unwrap()).unwrap();
    file::load(json).unwrap().tracks.into_keys().collect()
}

fn create_other(h: &mut Harness) -> (ProjectId, Vec<ServerMessage>) {
    let other = h.project_id();
    let out = h.send(Command::Project(ProjectCommand::Create {
        id: other,
        name: "Other".into(),
    }));
    (other, out)
}

#[test]
fn switching_projects_writes_an_autosave_that_failed_in_the_background() {
    let (mut h, first, edit) = failed_autosave();
    // The disk recovered (a sync client released the file).
    h.ctl.store.fail_saves = false;
    let (other, out) = create_other(&mut h);
    ok(&out);
    assert_eq!(h.project().id, other);
    assert!(
        stored_tracks(&h, first).contains(&edit),
        "the edit reached the first project's file before it was closed"
    );
}

#[test]
fn switching_projects_is_refused_while_the_project_cannot_be_saved() {
    let (mut h, first, edit) = failed_autosave();
    let (_, out) = create_other(&mut h);
    let e = err(&out);
    assert!(e.message.contains("no space left"), "{e:?}");
    // Still open, with the edit, and unsaved.
    assert_eq!(h.project().id, first);
    assert!(h.project().tracks.contains_key(&edit));
    assert!(h.ctl.is_dirty());
    // Once the disk is back, the switch saves it first.
    h.ctl.store.fail_saves = false;
    h.ok(Command::Project(ProjectCommand::Open { id: first }));
    let (other, out) = create_other(&mut h);
    ok(&out);
    assert_eq!(h.project().id, other);
    assert!(stored_tracks(&h, first).contains(&edit));
}

#[test]
fn closing_on_quit_writes_an_autosave_that_failed_in_the_background() {
    let (mut h, first, edit) = failed_autosave();
    h.ctl.store.fail_saves = false;
    let mut out = Vec::new();
    let unread = h.ctl.save_before_close(&mut out).expect("saved");
    assert!(unread.is_empty());
    assert!(stored_tracks(&h, first).contains(&edit));
    assert!(!h.ctl.is_dirty());
}

#[test]
fn closing_on_quit_reports_a_project_it_cannot_save_and_keeps_a_recovery_copy() {
    let (mut h, first, edit) = failed_autosave();
    let mut out = Vec::new();
    assert!(h.ctl.save_before_close(&mut out).is_err());
    let (id, json) = h.ctl.recovery_document().expect("a project is open");
    assert_eq!(id, first);
    assert!(file::load(&json).unwrap().tracks.contains_key(&edit));
}

/// #564: "save as" and "duplicate" copy the folder, then write the copy's document. When that
/// write fails (a full disk or quota), the copy is removed: it would list as a second project
/// with the old name and content.
#[test]
fn a_save_as_or_duplicate_that_cannot_write_leaves_no_copy() {
    let mut h = Harness::new();
    let first = h.create_project("First");
    h.ctl.store.fail_saves = true;

    let copy = h.project_id();
    let e = err(&h.send(Command::Project(ProjectCommand::SaveAs {
        new_id: copy,
        name: "Copy".into(),
    })));
    assert!(e.message.contains("no space left"), "{e:?}");
    assert!(!h.ctl.store.contains(copy), "no half-made copy");
    assert_eq!(h.project().id, first, "still on the original");

    let dup = h.project_id();
    let e = err(&h.send(Command::Project(ProjectCommand::Duplicate {
        id: first,
        new_id: dup,
        name: "Dup".into(),
    })));
    assert!(e.message.contains("no space left"), "{e:?}");
    assert!(!h.ctl.store.contains(dup), "no half-made duplicate");
    assert!(h.ctl.store.contains(first));

    // Once the disk is back, both work.
    h.ctl.store.fail_saves = false;
    h.ok(Command::Project(ProjectCommand::Duplicate {
        id: first,
        new_id: dup,
        name: "Dup".into(),
    }));
    assert!(h.ctl.store.contains(dup));
}

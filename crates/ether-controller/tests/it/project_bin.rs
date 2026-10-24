//! Archived projects and the bin (`project_status`): binned projects leave the list, can't
//! be opened, come back on restore, are deleted for good on `Delete`/`EmptyBin` or after
//! 30 days; archived ones stay listed, flagged, and still open; copies are live.

use crate::common;

use common::*;
use ether_core::protocol::project::{
    BIN_RETENTION_MS, ProjectCommand, ProjectEvent, ProjectSummary,
};
use ether_core::protocol::*;

fn list(h: &mut Harness) -> Vec<ProjectSummary> {
    match h.ok(Command::Project(ProjectCommand::List)) {
        ReplyValue::Projects { projects } => projects,
        r => panic!("{r:?}"),
    }
}

fn bin(h: &mut Harness) -> Vec<ProjectSummary> {
    match h.ok(Command::Project(ProjectCommand::ListBin)) {
        ReplyValue::Projects { projects } => projects,
        r => panic!("{r:?}"),
    }
}

fn project(c: ProjectCommand) -> Command {
    Command::Project(c)
}

#[test]
fn the_bin_holds_projects_until_restored_or_deleted() {
    let mut h = Harness::with_project();
    let a = h.project().id;
    h.ok(project(ProjectCommand::Save { autosave: false }));
    let b = h.create_project("B");

    // The open project can't go to the bin.
    let e = err(&h.send(project(ProjectCommand::MoveToBin { id: b })));
    assert_eq!(e.code, ErrorCode::InvalidState);

    let out = h.send(project(ProjectCommand::MoveToBin { id: a }));
    ok(&out);
    assert!(events(&out).iter().any(|e| matches!(e, Event::Project { event: ProjectEvent::ListChanged { projects } } if projects.iter().all(|p| p.id != a))));
    assert!(list(&mut h).iter().all(|p| p.id != a));
    let binned = bin(&mut h);
    assert_eq!(binned.len(), 1);
    assert_eq!(binned[0].id, a);
    assert_eq!(binned[0].binned_ms, Some(h.ctl.host.now as f64));
    // Not openable from the bin.
    for open in [
        ProjectCommand::Open { id: a },
        ProjectCommand::OpenSafe { id: a },
    ] {
        assert_eq!(err(&h.send(project(open))).code, ErrorCode::InvalidState);
    }
    assert_eq!(h.project().id, b);

    // Restored: listed and openable again.
    h.ok(project(ProjectCommand::RestoreFromBin { id: a }));
    assert!(bin(&mut h).is_empty());
    assert!(
        list(&mut h)
            .iter()
            .any(|p| p.id == a && p.binned_ms.is_none())
    );
    h.ok(project(ProjectCommand::Open { id: a }));
    assert_eq!(h.project().id, a);

    // Deleted for good from the bin, one by one or all at once.
    let c = h.create_project("C");
    let d = h.create_project("D");
    for id in [a, b, c] {
        h.ok(project(ProjectCommand::MoveToBin { id }));
    }
    h.ok(project(ProjectCommand::Delete { id: a }));
    assert!(!h.ctl.store.contains(a));
    assert_eq!(bin(&mut h).len(), 2);
    h.ok(project(ProjectCommand::EmptyBin));
    assert!(bin(&mut h).is_empty());
    assert!(!h.ctl.store.contains(b) && !h.ctl.store.contains(c));
    assert_eq!(list(&mut h).iter().map(|p| p.id).collect::<Vec<_>>(), [d]);

    // Unknown projects.
    let unknown = h.project_id();
    let e = err(&h.send(project(ProjectCommand::MoveToBin { id: unknown })));
    assert_eq!(e.code, ErrorCode::NotFound);
}

#[test]
fn the_bin_empties_itself_after_30_days() {
    let mut h = Harness::with_project();
    let a = h.project().id;
    h.ok(project(ProjectCommand::Save { autosave: false }));
    let b = h.create_project("B");
    h.ok(project(ProjectCommand::MoveToBin { id: a }));
    h.advance(BIN_RETENTION_MS - 1);
    assert_eq!(bin(&mut h).len(), 1, "not yet");
    assert!(h.ctl.store.contains(a));
    h.advance(1);
    assert!(list(&mut h).iter().all(|p| p.id != a));
    assert!(!h.ctl.store.contains(a), "deleted by the listing");
    assert!(bin(&mut h).is_empty());
    assert!(h.ctl.store.contains(b));
}

#[test]
fn archived_projects_stay_listed_flagged_and_open() {
    let mut h = Harness::with_project();
    let a = h.project().id;
    h.ok(project(ProjectCommand::Save { autosave: false }));
    h.create_project("B");
    h.ok(project(ProjectCommand::SetArchived {
        id: a,
        archived: true,
    }));
    let listed = |h: &mut Harness, id| list(h).into_iter().find(|p| p.id == id).unwrap();
    assert!(listed(&mut h, a).archived);
    h.ok(project(ProjectCommand::Open { id: a }));
    assert_eq!(h.project().id, a);
    assert!(listed(&mut h, a).archived, "opening keeps it archived");

    // Copies are live projects.
    let copy = h.project_id();
    h.ok(project(ProjectCommand::Duplicate {
        id: a,
        new_id: copy,
        name: "Copy".into(),
    }));
    assert!(!listed(&mut h, copy).archived);
    let saved_as = h.project_id();
    h.ok(project(ProjectCommand::SaveAs {
        new_id: saved_as,
        name: "Again".into(),
    }));
    assert!(!listed(&mut h, saved_as).archived);

    // An archived project in the bin comes back archived.
    h.ok(project(ProjectCommand::MoveToBin { id: a }));
    assert!(bin(&mut h)[0].archived);
    h.ok(project(ProjectCommand::RestoreFromBin { id: a }));
    assert!(listed(&mut h, a).archived);
    h.ok(project(ProjectCommand::SetArchived {
        id: a,
        archived: false,
    }));
    assert!(!listed(&mut h, a).archived);
}

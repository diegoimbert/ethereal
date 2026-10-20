//! Project versions and crash recovery (CONTRACTS.md §13.11).

use crate::common;

use common::*;
use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::store::{ProjectStore, StoreError};
use ether_controller::{Controller, ControllerConfig, EtherController};
use ether_core::protocol::media::DirectoryListing;
use ether_core::protocol::model::*;
use ether_core::protocol::project::{ProjectCommand, ProjectEvent, ProjectSummary};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::versions::*;
use ether_core::protocol::*;

const MARKER: &str = "versions/.session";

fn add_track(h: &mut Harness, name: &str) -> TrackId {
    let id: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id,
        kind: TrackKind::Audio,
        name: Some(name.into()),
        color: None,
        parent: None,
        before: None,
    }));
    id
}

fn versions(h: &mut Harness) -> Vec<VersionInfo> {
    match h.ok(Command::Version(VersionCommand::List)) {
        ReplyValue::Versions { versions } => versions,
        r => panic!("{r:?}"),
    }
}

fn create(h: &mut Harness, name: Option<&str>) -> VersionInfo {
    match h.ok(Command::Version(VersionCommand::Create {
        name: name.map(str::to_owned),
    })) {
        ReplyValue::Version { version } => version,
        r => panic!("{r:?}"),
    }
}

fn recoverable(h: &mut Harness) -> Vec<RecoveryInfo> {
    match h.ok(Command::Version(VersionCommand::ListRecoverable)) {
        ReplyValue::Recoverable { projects } => projects,
        r => panic!("{r:?}"),
    }
}

fn changed_events(out: &[ServerMessage]) -> usize {
    events(out)
        .iter()
        .filter(|e| {
            matches!(
                e,
                Event::Version {
                    event: VersionEvent::Changed
                }
            )
        })
        .count()
}

/// No idle autosave of `project.ether`, so versions are the only safety net.
fn no_autosave() -> Harness {
    Harness::with(
        FakeBridge::default(),
        MemoryLibrary::new(),
        ControllerConfig {
            autosave_after_ms: None,
            ..ControllerConfig::default()
        },
    )
}

/// A new controller over `store`, as after a restart.
fn restart(store: MemoryStore, now: u64) -> Harness {
    let mut h = no_autosave();
    h.ctl.store = store;
    h.ctl.host.now = now;
    h.ctl.store.now_ms = now;
    h
}

/// Kill the controller without a clean close (its `Drop` never runs) and keep the store.
fn crash(h: Harness) -> MemoryStore {
    let store = h.ctl.store.clone();
    std::mem::forget(h);
    store
}

#[test]
fn autosave_versions_roll_at_the_interval_only_after_changes() {
    let mut h = no_autosave();
    h.create_project("Song");
    // Nothing changed: no version, however long we wait.
    h.advance(VERSION_INTERVAL_MS * 3);
    h.tick();
    assert!(versions(&mut h).is_empty());

    add_track(&mut h, "Drums");
    let out = h.tick();
    assert_eq!(changed_events(&out), 1, "first change after the interval");
    let v = versions(&mut h);
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].kind, VersionKind::Autosave);
    assert_eq!(v[0].id, format!("{}-autosave", h.ctl.host.now));
    assert!(v[0].size > 0);

    // Another change, but before the interval: nothing yet.
    add_track(&mut h, "Bass");
    h.advance(VERSION_INTERVAL_MS - 1);
    h.tick();
    assert_eq!(versions(&mut h).len(), 1);
    h.advance(1);
    h.tick();
    let v = versions(&mut h);
    assert_eq!(v.len(), 2);
    assert!(v[0].created_ms > v[1].created_ms, "newest first");
}

#[test]
fn autosave_versions_are_thinned_in_tiers() {
    let mut h = no_autosave();
    let id = h.create_project("Song");
    let manual = create(&mut h, Some("Keep me"));
    // A named autosave version is never pruned either.
    for i in 0..MAX_AUTOSAVE_VERSIONS + 5 {
        add_track(&mut h, &format!("T{i}"));
        h.advance(VERSION_INTERVAL_MS);
        h.tick();
        if i == 0 {
            let first = versions(&mut h)[0].id.clone();
            h.ok(Command::Version(VersionCommand::Rename {
                version: first,
                name: Some("First take".into()),
            }));
        }
    }
    let v = versions(&mut h);
    let unnamed: Vec<_> = v
        .iter()
        .filter(|v| v.kind == VersionKind::Autosave && v.name.is_none())
        .collect();
    assert_eq!(unnamed.len(), MAX_AUTOSAVE_VERSIONS);
    assert_eq!(unnamed[0].created_ms, h.ctl.host.now, "the newest stays");
    assert!(v.iter().any(|x| x.id == manual.id), "manual versions stay");
    assert!(v.iter().any(|x| x.name.as_deref() == Some("First take")));
    // Pruning is stable: `retain` keeps everything that is left.
    let left: Vec<_> = unnamed.into_iter().cloned().collect();
    let keep = ether_controller::versions::retain(&left, h.ctl.host.now);
    assert!(left.iter().all(|x| keep.contains(&x.id)));
    assert!(h.ctl.store.file(id, "versions/names.json").is_some());
}

#[test]
fn an_existing_folder_of_fifty_old_autosaves_is_pruned_on_the_next_version() {
    let mut h = no_autosave();
    let id = h.create_project("Song");
    // What the 5-minute, newest-50 rule left behind (plus a named and a manual version).
    let start = h.ctl.host.now - 50 * 5 * 60_000;
    for i in 0..50u64 {
        let ms = start + i * 5 * 60_000;
        h.ctl
            .store
            .write(id, &format!("versions/{ms}-autosave.ether"), b"{}")
            .unwrap();
    }
    h.ctl
        .store
        .write(id, &format!("versions/{start}-manual.ether"), b"{}")
        .unwrap();
    h.ctl
        .store
        .write(
            id,
            "versions/names.json",
            format!("{{\"{}-autosave\": \"Named\"}}", start + 5 * 60_000).as_bytes(),
        )
        .unwrap();
    assert_eq!(versions(&mut h).len(), 51);

    add_track(&mut h, "Drums");
    h.advance(VERSION_INTERVAL_MS);
    h.tick();
    let v = versions(&mut h);
    let unnamed = v
        .iter()
        .filter(|v| v.kind == VersionKind::Autosave && v.name.is_none())
        .count();
    assert_eq!(unnamed, MAX_AUTOSAVE_VERSIONS);
    assert_eq!(v.len(), MAX_AUTOSAVE_VERSIONS + 2, "named and manual stay");
    assert_eq!(v[0].created_ms, h.ctl.host.now);
    assert!(v.iter().any(|x| x.created_ms == start), "the oldest stays");
}

#[test]
fn before_restore_versions_keep_the_newest_ten() {
    let mut h = no_autosave();
    h.create_project("Song");
    let target = create(&mut h, Some("Target"));
    for _ in 0..MAX_BEFORE_RESTORE_VERSIONS + 3 {
        h.advance(1);
        h.ok(Command::Version(VersionCommand::Restore {
            version: target.id.clone(),
        }));
    }
    let v = versions(&mut h);
    let before: Vec<_> = v
        .iter()
        .filter(|v| v.kind == VersionKind::BeforeRestore)
        .collect();
    assert_eq!(before.len(), MAX_BEFORE_RESTORE_VERSIONS);
    assert_eq!(before[0].created_ms, h.ctl.host.now, "the newest stay");
    assert!(v.iter().any(|x| x.id == target.id));
}

#[test]
fn a_clock_going_backwards_does_not_stop_autosave_versions() {
    let mut h = no_autosave();
    h.create_project("Song");
    add_track(&mut h, "A");
    h.advance(VERSION_INTERVAL_MS);
    h.tick();
    assert_eq!(versions(&mut h).len(), 1);
    // The clock jumps back an hour.
    h.ctl.host.now -= 3_600_000;
    h.ctl.store.now_ms = h.ctl.host.now;
    add_track(&mut h, "B");
    h.tick();
    let v = versions(&mut h);
    assert_eq!(v.len(), 2);
    assert!(v.iter().any(|x| x.created_ms == h.ctl.host.now));
}

#[test]
fn manual_versions_rename_delete_and_compare() {
    let mut h = Harness::with_project();
    add_track(&mut h, "Drums");
    let out = h.send(Command::Version(VersionCommand::Create {
        name: Some("  Before vocals ".into()),
    }));
    assert_eq!(changed_events(&out), 1);
    let ReplyValue::Version { version: v1 } = ok(&out) else {
        panic!()
    };
    assert_eq!(v1.kind, VersionKind::Manual);
    assert_eq!(v1.name.as_deref(), Some("Before vocals"));
    // Same millisecond: still a unique id.
    let v2 = create(&mut h, None);
    assert_ne!(v1.id, v2.id);

    let vocals = add_track(&mut h, "Vocals");
    h.ok(Command::Track(TrackCommand::Rename {
        id: vocals,
        name: "Lead vocals".into(),
    }));
    let pid = h.project().id;
    h.ok(Command::Project(ProjectCommand::Rename {
        id: pid,
        name: "Renamed".into(),
    }));
    let diff = match h.ok(Command::Version(VersionCommand::Compare {
        version: v1.id.clone(),
        against: None,
    })) {
        ReplyValue::VersionDiff { diff } => diff,
        r => panic!("{r:?}"),
    };
    let tracks = diff.tables.iter().find(|t| t.table == "tracks").unwrap();
    assert_eq!((tracks.added, tracks.removed), (1, 0));
    assert!(tracks.names.contains(&"Lead vocals".to_string()));
    assert!(diff.settings_changed);
    // Two stored versions of the same state: nothing differs.
    let same = h.ok(Command::Version(VersionCommand::Compare {
        version: v1.id.clone(),
        against: Some(v2.id.clone()),
    }));
    assert_eq!(
        same,
        ReplyValue::VersionDiff {
            diff: VersionDiff::default()
        }
    );

    h.ok(Command::Version(VersionCommand::Rename {
        version: v2.id.clone(),
        name: Some("Second".into()),
    }));
    h.ok(Command::Version(VersionCommand::Rename {
        version: v1.id.clone(),
        name: None,
    }));
    let v = versions(&mut h);
    let find = |id: &str| v.iter().find(|x| x.id == id).unwrap().name.clone();
    assert_eq!(find(&v2.id).as_deref(), Some("Second"));
    assert_eq!(find(&v1.id), None);

    h.ok(Command::Version(VersionCommand::Delete {
        version: v2.id.clone(),
    }));
    assert_eq!(versions(&mut h).len(), 1);
    let e = err(&h.send(Command::Version(VersionCommand::Delete { version: v2.id })));
    assert_eq!(e.code, ErrorCode::NotFound);
    let e = err(&h.send(Command::Version(VersionCommand::Restore {
        version: "../project".into(),
    })));
    assert_eq!(e.code, ErrorCode::InvalidArgument);
}

#[test]
fn restore_snapshots_first_and_replaces_the_document() {
    let mut h = Harness::with_project();
    add_track(&mut h, "Drums");
    let v1 = create(&mut h, None);
    add_track(&mut h, "Bass");
    h.ok(Command::Project(ProjectCommand::Save { autosave: false }));
    assert_eq!(h.project().tracks.len(), 3, "master + 2");
    h.advance(10);

    let out = h.send(Command::Version(VersionCommand::Restore {
        version: v1.id.clone(),
    }));
    ok(&out);
    let ev = events(&out);
    assert!(
        ev.iter()
            .any(|e| matches!(e, Event::ProjectLoaded { project } if project.tracks.len() == 2))
    );
    assert!(ev.iter().any(|e| matches!(
        e,
        Event::Project {
            event: ProjectEvent::DirtyChanged { dirty: true }
        }
    )));
    assert!(h.ctl.is_dirty());
    assert_eq!(h.project().tracks.len(), 2);

    // The state before the restore is a version: restoring it undoes the restore.
    let before = versions(&mut h)
        .into_iter()
        .find(|v| v.kind == VersionKind::BeforeRestore)
        .expect("before-restore version");
    h.ok(Command::Version(VersionCommand::Restore {
        version: before.id,
    }));
    assert_eq!(h.project().tracks.len(), 3);
}

#[test]
fn versions_never_copy_or_restore_other_project_files() {
    let mut h = Harness::with_project();
    let id = h.project().id;
    h.ctl
        .store
        .write(id, "share.json", br#"{"room":"a"}"#)
        .unwrap();
    let version = create(&mut h, None);
    h.ctl
        .store
        .write(id, "share.json", br#"{"room":"b"}"#)
        .unwrap();
    h.ok(Command::Version(VersionCommand::Restore {
        version: version.id,
    }));
    assert_eq!(
        h.ctl.store.file(id, "share.json"),
        Some(&br#"{"room":"b"}"#[..])
    );
    let listing = h.ctl.store.list_dir(id, "versions").unwrap();
    assert!(
        listing
            .entries
            .iter()
            .all(|e| e.name.ends_with(".ether") || e.name.starts_with('.')),
        "{listing:?}"
    );
}

#[test]
fn a_killed_session_offers_recovery_and_restores_the_newest_version() {
    let mut h = no_autosave();
    let id = h.create_project("Song");
    h.ok(Command::Project(ProjectCommand::Save { autosave: false }));
    assert!(h.ctl.store.file(id, MARKER).is_some(), "marker on open");
    add_track(&mut h, "Unsaved work");
    h.advance(VERSION_INTERVAL_MS);
    h.tick();
    add_track(&mut h, "Lost (after the last version)");
    let now = h.ctl.host.now;
    let store = crash(h);

    let mut h = restart(store, now + 60_000);
    let rec = recoverable(&mut h);
    assert_eq!(rec.len(), 1, "{rec:?}");
    assert_eq!(rec[0].project, id);
    assert_eq!(rec[0].name, "Song");
    assert_eq!(rec[0].version.kind, VersionKind::Autosave);
    assert!(rec[0].version.created_ms > rec[0].saved_ms);

    let out = h.send(Command::Version(VersionCommand::Recover { project: id }));
    let ReplyValue::Project { project } = ok(&out) else {
        panic!()
    };
    let names: Vec<_> = project.tracks.values().map(|t| t.name.clone()).collect();
    assert!(names.contains(&"Unsaved work".to_string()), "{names:?}");
    assert!(h.ctl.is_dirty(), "Save keeps the recovered state");
    assert!(recoverable(&mut h).is_empty());
    h.ok(Command::Project(ProjectCommand::Save { autosave: false }));
    assert!(recoverable(&mut h).is_empty());
}

#[test]
fn a_save_after_the_last_version_offers_nothing() {
    let mut h = no_autosave();
    h.create_project("Song");
    add_track(&mut h, "A");
    h.advance(VERSION_INTERVAL_MS);
    h.tick();
    h.advance(1000);
    h.ok(Command::Project(ProjectCommand::Save { autosave: false }));
    let now = h.ctl.host.now;
    let store = crash(h);
    let mut h = restart(store, now + 1);
    assert!(recoverable(&mut h).is_empty());

    // A version newer than the save but with the same content: nothing to recover either.
    let _taken = h.project_id(); // the restarted harness replays its ids
    let id = h.create_project("Same");
    add_track(&mut h, "B");
    h.ok(Command::Project(ProjectCommand::Save { autosave: false }));
    h.advance(1000);
    create(&mut h, Some("Same as saved"));
    let now = h.ctl.host.now;
    let store = crash(h);
    let mut h = restart(store, now + 1);
    assert!(recoverable(&mut h).is_empty());
    h.ok(Command::Project(ProjectCommand::Open { id }));
    assert_eq!(versions(&mut h).len(), 1);
}

type StoreResult<T> = Result<T, StoreError>;

/// A store shared with the test, to see what the controller leaves behind when dropped.
#[derive(Clone, Default)]
struct Shared(std::rc::Rc<std::cell::RefCell<MemoryStore>>);

impl ProjectStore for Shared {
    fn list(&mut self) -> StoreResult<Vec<ProjectSummary>> {
        self.0.borrow_mut().list()
    }
    fn create(&mut self, id: ProjectId) -> StoreResult<()> {
        self.0.borrow_mut().create(id)
    }
    fn load(&mut self, id: ProjectId) -> StoreResult<String> {
        self.0.borrow_mut().load(id)
    }
    fn save(&mut self, id: ProjectId, json: &str) -> StoreResult<ProjectSummary> {
        self.0.borrow_mut().save(id, json)
    }
    fn duplicate(&mut self, from: ProjectId, to: ProjectId) -> StoreResult<()> {
        self.0.borrow_mut().duplicate(from, to)
    }
    fn delete(&mut self, id: ProjectId) -> StoreResult<()> {
        self.0.borrow_mut().delete(id)
    }
    fn read(&mut self, id: ProjectId, p: &str) -> StoreResult<Vec<u8>> {
        self.0.borrow_mut().read(id, p)
    }
    fn write(&mut self, id: ProjectId, p: &str, b: &[u8]) -> StoreResult<()> {
        self.0.borrow_mut().write(id, p, b)
    }
    fn list_dir(&mut self, id: ProjectId, p: &str) -> StoreResult<DirectoryListing> {
        self.0.borrow_mut().list_dir(id, p)
    }
    fn remove(&mut self, id: ProjectId, p: &str) -> StoreResult<()> {
        self.0.borrow_mut().remove(id, p)
    }
}

#[test]
fn a_clean_close_removes_the_marker() {
    let shared = Shared::default();
    let mut ctl = EtherController::new(
        FakeBridge::default(),
        FakeHost::at(T0),
        shared.clone(),
        MemoryLibrary::new(),
    );
    let id = IdGen::new(7).next_project_id(T0);
    let mut out = Vec::new();
    ctl.handle(
        ClientMessage {
            id: 1,
            gesture: None,
            command: Command::Project(ProjectCommand::Create {
                id,
                name: "Song".into(),
            }),
        },
        &mut out,
    );
    assert!(matches!(ok(&out), ReplyValue::Project { .. }));
    assert!(shared.0.borrow().file(id, MARKER).is_some());
    drop(ctl);
    assert!(shared.0.borrow().file(id, MARKER).is_none());
}

#[test]
fn opening_another_project_closes_the_previous_session() {
    let mut h = no_autosave();
    let a = h.create_project("A");
    assert!(h.ctl.store.file(a, MARKER).is_some());
    let b = h.create_project("B");
    assert!(
        h.ctl.store.file(a, MARKER).is_none(),
        "switching is a clean close"
    );
    assert!(h.ctl.store.file(b, MARKER).is_some());
    h.ok(Command::Project(ProjectCommand::Open { id: a }));
    assert!(h.ctl.store.file(b, MARKER).is_none());
    // Duplicates are not open anywhere.
    let c = h.project_id();
    h.ok(Command::Project(ProjectCommand::Duplicate {
        id: a,
        new_id: c,
        name: "C".into(),
    }));
    assert!(h.ctl.store.file(c, MARKER).is_none());
    // Save As moves the session to the new folder.
    let d = h.project_id();
    h.ok(Command::Project(ProjectCommand::SaveAs {
        new_id: d,
        name: "D".into(),
    }));
    assert!(h.ctl.store.file(a, MARKER).is_none());
    assert!(h.ctl.store.file(d, MARKER).is_some());
}

#[test]
fn an_auto_reopened_project_still_offers_recovery_and_discard_keeps_the_save() {
    let mut h = no_autosave();
    let id = h.create_project("Song");
    add_track(&mut h, "Unsaved");
    h.advance(VERSION_INTERVAL_MS);
    h.tick();
    let now = h.ctl.host.now;
    let store = crash(h);

    // The host reopens the last project before the UI asks.
    let mut h = restart(store, now + 5);
    h.ok(Command::Project(ProjectCommand::Open { id }));
    assert!(h.project().tracks.len() == 1, "opened the saved file");
    assert_eq!(recoverable(&mut h).len(), 1);
    h.ok(Command::Version(VersionCommand::DiscardRecovery {
        project: id,
    }));
    assert!(recoverable(&mut h).is_empty());
    assert!(h.project().tracks.len() == 1);
    assert!(
        h.ctl.store.file(id, MARKER).is_some(),
        "our own marker stays"
    );

    // Discarding a project that is not open removes its marker; versions stay.
    add_track(&mut h, "Again");
    h.advance(VERSION_INTERVAL_MS);
    h.tick();
    let now = h.ctl.host.now;
    let store = crash(h);
    let mut h = restart(store, now + 5);
    assert_eq!(recoverable(&mut h).len(), 1);
    h.ok(Command::Version(VersionCommand::DiscardRecovery {
        project: id,
    }));
    assert!(recoverable(&mut h).is_empty());
    assert!(h.ctl.store.file(id, MARKER).is_none());
    h.ok(Command::Project(ProjectCommand::Open { id }));
    assert!(versions(&mut h).len() >= 2);
}

#[test]
fn commands_need_an_open_project() {
    let mut h = Harness::new();
    let e = err(&h.send(Command::Version(VersionCommand::List)));
    assert_eq!(e.code, ErrorCode::InvalidState);
    assert!(recoverable(&mut h).is_empty());
}

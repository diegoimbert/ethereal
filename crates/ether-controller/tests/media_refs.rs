//! `media-references` (CONTRACTS.md §12.9): samples are referenced in place. Imports by OS
//! path and from library roots become `MediaLocation::External` (nothing copied); missing
//! files are detected when a project opens (silence, `MediaEvent::Missing`,
//! `ListMissing`); `Relink`, `Search` (library roots and a folder, by name then content
//! hash) and `CollectAll` (one undo step, then save) fix them.

#[path = "common/mod.rs"]
mod common;

use std::collections::BTreeMap;

use common::{FakeBridge, FakeHost, T0, err, events, ok, wav};
use ether_controller::memory::MemoryStore;
use ether_controller::store::{Library, ProjectStore, StoreError, check_relative_path, file_kind};
use ether_controller::{Controller, ControllerConfig, EtherController};
use ether_core::protocol::media::{
    BrowseLocation, BrowseRoot, DirectoryEntry, DirectoryListing, FileKind, MediaCommand,
    MediaEvent, MediaSource,
};
use ether_core::protocol::media_refs::{MediaRefCommand, MediaRefEvent};
use ether_core::protocol::model::*;
use ether_core::protocol::project::{EditCommand, ProjectCommand, ProjectEvent};
use ether_core::protocol::*;

/// The engine machine's files (absolute paths), with one library root "lib" at `/Lib`.
#[derive(Default)]
struct Fs {
    files: BTreeMap<String, Vec<u8>>,
}

const LIB: &str = "/Lib";

impl Fs {
    /// Direct children of an absolute folder: (absolute path, name, is a folder).
    fn children(&self, dir: &str) -> Option<Vec<(String, String, bool)>> {
        let prefix = format!("{}/", dir.trim_end_matches('/'));
        let mut out = BTreeMap::new();
        for path in self.files.keys() {
            if let Some(rest) = path.strip_prefix(&prefix) {
                match rest.split_once('/') {
                    Some((d, _)) => out.insert(d.to_string(), true),
                    None => out.insert(rest.to_string(), false),
                };
            }
        }
        if out.is_empty() {
            return None;
        }
        Some(
            out.into_iter()
                .map(|(name, is_dir)| (format!("{prefix}{name}"), name, is_dir))
                .collect(),
        )
    }
}

impl Library for Fs {
    fn roots(&self) -> Vec<BrowseRoot> {
        vec![BrowseRoot {
            location: BrowseLocation::Library { id: "lib".into() },
            name: "Library".into(),
        }]
    }
    fn list_dir(&mut self, root: &str, rel: &str) -> Result<DirectoryListing, StoreError> {
        check_relative_path(rel)?;
        let dir = if rel.is_empty() {
            LIB.to_string()
        } else {
            format!("{LIB}/{rel}")
        };
        let entries = self
            .children(&dir)
            .unwrap_or_default()
            .into_iter()
            .map(|(abs, name, is_dir)| DirectoryEntry {
                kind: if is_dir {
                    FileKind::Directory
                } else {
                    file_kind(&name)
                },
                path: abs[LIB.len() + 1..].to_string(),
                name,
                size: 0.0,
            })
            .collect();
        Ok(DirectoryListing {
            location: BrowseLocation::Library { id: root.into() },
            path: rel.into(),
            entries,
        })
    }
    fn read(&mut self, _root: &str, rel: &str) -> Result<Vec<u8>, StoreError> {
        check_relative_path(rel)?;
        self.read_external(&format!("{LIB}/{rel}"))
    }
    fn external_path(&self, _root: &str, rel: &str) -> Option<String> {
        Some(format!("{LIB}/{rel}"))
    }
    fn read_external(&mut self, path: &str) -> Result<Vec<u8>, StoreError> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(path.into()))
    }
    fn list_external_dir(&mut self, path: &str) -> Result<Vec<(String, bool)>, StoreError> {
        let c = self
            .children(path)
            .ok_or_else(|| StoreError::NotFound(path.into()))?;
        Ok(c.into_iter().map(|(abs, _, d)| (abs, d)).collect())
    }
}

type Ctl = EtherController<FakeBridge, FakeHost, MemoryStore, Fs>;

struct H {
    ctl: Ctl,
    ids: IdGen,
    next: u32,
    pid: ProjectId,
}

impl H {
    fn new(files: &[(&str, Vec<u8>)]) -> Self {
        let mut store = MemoryStore::new();
        store.now_ms = T0;
        let fs = Fs {
            files: files
                .iter()
                .map(|(p, b)| (p.to_string(), b.clone()))
                .collect(),
        };
        let ctl = EtherController::with_config(
            FakeBridge::default(),
            FakeHost { now: T0 },
            store,
            fs,
            ControllerConfig {
                autosave_after_ms: None,
                ..ControllerConfig::default()
            },
        );
        let mut h = Self {
            ctl,
            ids: IdGen::new(7),
            next: 1,
            pid: ProjectId::v7(T0, [3; 10]),
        };
        let pid = h.pid;
        h.ok(Command::Project(ProjectCommand::Create {
            id: pid,
            name: "Refs".into(),
        }));
        h
    }

    fn fs(&mut self) -> &mut BTreeMap<String, Vec<u8>> {
        &mut self.ctl.library.files
    }

    fn send(&mut self, command: Command) -> Vec<ServerMessage> {
        let id = self.next;
        self.next += 1;
        let mut out = Vec::new();
        self.ctl.handle(
            ClientMessage {
                id,
                gesture: None,
                command,
            },
            &mut out,
        );
        out
    }

    fn ok(&mut self, command: Command) -> ReplyValue {
        ok(&self.send(command))
    }

    /// Tick until media jobs are done and nothing happens for a while (searches are
    /// stepped per tick); returns the events.
    fn settle(&mut self) -> Vec<Event> {
        let mut all = Vec::new();
        let mut quiet = 0;
        for _ in 0..20_000 {
            let mut out = Vec::new();
            self.ctl.host.now += 10;
            let now = self.ctl.host.now;
            self.ctl.tick(now, &mut out);
            let evs = events(&out);
            quiet = if evs.is_empty() && !self.ctl.media_pending() {
                quiet + 1
            } else {
                0
            };
            all.extend(evs);
            if quiet > 40 {
                return all;
            }
        }
        panic!("did not settle");
    }

    fn import(&mut self, source: MediaSource) -> MediaRef {
        let id: MediaId = self.ids.next(T0);
        match self.ok(Command::Media(MediaCommand::Import { id, source })) {
            ReplyValue::Media { media } => media,
            r => panic!("{r:?}"),
        }
    }

    fn media(&self, id: MediaId) -> MediaRef {
        self.ctl.project().unwrap().media[&id].clone()
    }

    fn missing(&mut self) -> Vec<MediaId> {
        match self.ok(Command::MediaRef(MediaRefCommand::ListMissing)) {
            ReplyValue::MissingMedia { media } => media,
            r => panic!("{r:?}"),
        }
    }

    fn undo(&mut self) {
        self.ok(Command::Edit(EditCommand::Undo));
    }

    /// Save, close and reopen the project (a new session: every media is checked again).
    fn reopen(&mut self) -> Vec<Event> {
        self.ok(Command::Project(ProjectCommand::Save));
        let pid = self.pid;
        let other: ProjectId = self.ids.next_project_id(T0);
        self.ok(Command::Project(ProjectCommand::Create {
            id: other,
            name: "Other".into(),
        }));
        self.ok(Command::Project(ProjectCommand::Open { id: pid }));
        self.settle()
    }

    fn stored(&self, file: &str) -> Option<Vec<u8>> {
        self.ctl.store.file(self.pid, file).map(<[u8]>::to_vec)
    }
}

fn is_missing_event(e: &Event, media: MediaId) -> bool {
    matches!(e, Event::Media { event: MediaEvent::Missing { media: m } } if *m == media)
}

fn resolved(evs: &[Event], media: MediaId) -> bool {
    evs.iter().any(|e| {
        matches!(e, Event::MediaRef { event: MediaRefEvent::Resolved { media: m } } if *m == media)
    })
}

fn warnings(out: &[ServerMessage]) -> Vec<String> {
    events(out)
        .into_iter()
        .filter_map(|e| match e {
            Event::Notification {
                level: NotificationLevel::Warning,
                message,
            } => Some(message),
            _ => None,
        })
        .collect()
}

fn kick() -> Vec<u8> {
    wav(48_000, &[vec![0.5; 4_800]])
}

fn snare() -> Vec<u8> {
    wav(48_000, &[vec![-0.25; 2_400]])
}

const DESKTOP: &str = "/Users/ana/Desktop/Kick.wav";

#[test]
fn path_imports_are_referenced_in_place() {
    let mut h = H::new(&[(DESKTOP, kick())]);
    let m = h.import(MediaSource::Path {
        path: DESKTOP.into(),
    });
    assert_eq!(
        m.location,
        MediaLocation::External {
            path: DESKTOP.into()
        }
    );
    assert!(
        m.hash.is_some(),
        "external references carry the content hash"
    );
    assert!(m.file.starts_with("media/"), "{}", m.file);
    assert_eq!(h.stored(&m.file), None, "nothing copied into the project");
    let evs = h.settle();
    assert!(!evs.iter().any(|e| is_missing_event(e, m.id)));
    assert!(h.missing().is_empty());
    assert!(
        h.ctl.bridge.media.contains_key(&m.id),
        "plays from the external file"
    );

    // Library files are referenced in place too; uploads (no path) are copied.
    h.fs().insert(format!("{LIB}/Drums/Snare.wav"), snare());
    let lib = h.import(MediaSource::Location {
        location: BrowseLocation::Library { id: "lib".into() },
        path: "Drums/Snare.wav".into(),
    });
    assert_eq!(
        lib.location,
        MediaLocation::External {
            path: format!("{LIB}/Drums/Snare.wav")
        }
    );
    assert_eq!(h.stored(&lib.file), None);
}

#[test]
fn missing_files_are_detected_on_open_and_relinked() {
    let mut h = H::new(&[(DESKTOP, kick())]);
    let m = h.import(MediaSource::Path {
        path: DESKTOP.into(),
    });
    h.settle();
    // The file is moved away while the project is closed.
    let bytes = h.fs().remove(DESKTOP).unwrap();
    let moved = "/Users/ana/Samples/Kick.wav";
    h.fs().insert(moved.into(), bytes);
    let evs = h.reopen();
    assert!(evs.iter().any(|e| is_missing_event(e, m.id)), "{evs:#?}");
    assert!(
        !evs.iter().any(|e| matches!(
            e,
            Event::Notification {
                level: NotificationLevel::Error,
                ..
            }
        )),
        "missing media show on the clips, not as error toasts"
    );
    assert_eq!(h.missing(), vec![m.id]);
    assert!(
        !h.ctl.bridge.media.contains_key(&m.id),
        "silence until relinked"
    );

    // Relink (one undo step): loads again and reports it resolved.
    let out = h.send(Command::MediaRef(MediaRefCommand::Relink {
        media: m.id,
        source: MediaSource::Path { path: moved.into() },
    }));
    ok(&out);
    assert!(warnings(&out).is_empty(), "same content: no warning");
    assert_eq!(
        h.media(m.id).location,
        MediaLocation::External { path: moved.into() }
    );
    let evs = h.settle();
    assert!(resolved(&evs, m.id), "{evs:#?}");
    assert!(h.missing().is_empty());
    assert!(h.ctl.bridge.media.contains_key(&m.id));

    // Undo points back at the old path. The content is the same (same hash): the loaded
    // audio keeps playing; the next open reports it missing again.
    h.undo();
    assert_eq!(
        h.media(m.id).location,
        MediaLocation::External {
            path: DESKTOP.into()
        }
    );
    h.settle();
    assert!(h.ctl.bridge.media.contains_key(&m.id));
    let evs = h.reopen();
    assert!(evs.iter().any(|e| is_missing_event(e, m.id)), "{evs:#?}");
}

#[test]
fn a_restored_file_resolves_on_recheck() {
    let mut h = H::new(&[(DESKTOP, kick())]);
    let m = h.import(MediaSource::Path {
        path: DESKTOP.into(),
    });
    h.settle();
    let bytes = h.fs().remove(DESKTOP).unwrap();
    h.reopen();
    assert_eq!(h.missing(), vec![m.id]);
    h.settle();
    h.fs().insert(DESKTOP.into(), bytes);
    // `ListMissing` checks again.
    assert_eq!(h.missing(), vec![m.id]);
    let evs = h.settle();
    assert!(resolved(&evs, m.id));
    assert!(h.missing().is_empty());
}

#[test]
fn relinking_to_other_content_updates_the_hash_with_a_warning() {
    let other = "/Users/ana/Desktop/Kick (edit).wav";
    let stereo = "/Users/ana/Desktop/Kick stereo.wav";
    let mut h = H::new(&[
        (DESKTOP, kick()),
        (other, wav(48_000, &[vec![0.9; 9_600]])),
        (stereo, wav(48_000, &[vec![0.1; 10], vec![0.1; 10]])),
        ("/Users/ana/Desktop/notes.txt", b"x".to_vec()),
    ]);
    let m = h.import(MediaSource::Path {
        path: DESKTOP.into(),
    });
    h.settle();
    let out = h.send(Command::MediaRef(MediaRefCommand::Relink {
        media: m.id,
        source: MediaSource::Path { path: other.into() },
    }));
    ok(&out);
    assert_eq!(warnings(&out).len(), 1, "{out:#?}");
    let now = h.media(m.id);
    assert_ne!(now.hash, m.hash);
    assert_eq!(
        now.frames, m.frames,
        "the document's length stays the reference"
    );
    h.settle();
    assert!(h.ctl.bridge.media.contains_key(&m.id));
    // One undo step restores both the path and the hash.
    h.undo();
    assert_eq!(h.media(m.id), m);

    // Another channel count: refused. Not an audio file: refused.
    let e = err(&h.send(Command::MediaRef(MediaRefCommand::Relink {
        media: m.id,
        source: MediaSource::Path {
            path: stereo.into(),
        },
    })));
    assert_eq!(e.code, ErrorCode::InvalidArgument, "{e:?}");
    let e = err(&h.send(Command::MediaRef(MediaRefCommand::Relink {
        media: m.id,
        source: MediaSource::Path {
            path: "/Users/ana/Desktop/notes.txt".into(),
        },
    })));
    assert_eq!(e.code, ErrorCode::InvalidArgument);
    assert_eq!(h.media(m.id), m);
}

#[test]
fn search_relinks_hash_matches_and_reports_name_candidates() {
    let hat = wav(48_000, &[vec![0.3; 1_200]]);
    let mut h = H::new(&[
        (DESKTOP, kick()),
        ("/Users/ana/Desktop/Hat.wav", hat.clone()),
        ("/Users/ana/Desktop/Snare.wav", snare()),
    ]);
    let k = h.import(MediaSource::Path {
        path: DESKTOP.into(),
    });
    let hh = h.import(MediaSource::Path {
        path: "/Users/ana/Desktop/Hat.wav".into(),
    });
    let s = h.import(MediaSource::Path {
        path: "/Users/ana/Desktop/Snare.wav".into(),
    });
    h.settle();
    // The desktop is cleaned up: the kick went into the library, the hat into a folder
    // (with an unrelated file of the same name elsewhere), the snare is gone for good.
    let kick = h.fs().remove(DESKTOP).unwrap();
    h.fs().remove("/Users/ana/Desktop/Hat.wav");
    h.fs().remove("/Users/ana/Desktop/Snare.wav");
    h.fs().insert(format!("{LIB}/One Shots/kick.WAV"), kick);
    h.fs().insert("/Volumes/Backup/2026/Hat.wav".into(), hat);
    h.fs().insert(
        "/Volumes/Backup/old/Hat.wav".into(),
        wav(48_000, &[vec![0.0; 5]]),
    );
    h.reopen();
    let mut missing = h.missing();
    missing.sort();
    let mut all = vec![k.id, hh.id, s.id];
    all.sort();
    assert_eq!(missing, all);

    // Library roots only.
    let out = h.send(Command::MediaRef(MediaRefCommand::Search {
        media: None,
        folder: None,
    }));
    assert_eq!(ok(&out), ReplyValue::Unit);
    let evs = h.settle();
    assert_eq!(
        h.media(k.id).location,
        MediaLocation::External {
            path: format!("{LIB}/One Shots/kick.WAV")
        },
        "hash match in the library: relinked automatically"
    );
    assert!(resolved(&evs, k.id));
    let candidates = |evs: &[Event], id: MediaId| -> Option<Vec<MediaSource>> {
        evs.iter().find_map(|e| match e {
            Event::MediaRef {
                event: MediaRefEvent::Candidates { media, candidates },
            } if *media == id => Some(candidates.clone()),
            _ => None,
        })
    };
    assert_eq!(candidates(&evs, hh.id), Some(vec![]));
    assert!(evs.iter().any(|e| matches!(
        e,
        Event::MediaRef {
            event: MediaRefEvent::SearchProgress { total: Some(_), .. }
        }
    )));

    // A folder too: the matching hat is relinked, the other one isn't offered (it was
    // matched); the snare has nothing.
    h.ok(Command::MediaRef(MediaRefCommand::Search {
        media: None,
        folder: Some("/Volumes/Backup".into()),
    }));
    let evs = h.settle();
    assert_eq!(
        h.media(hh.id).location,
        MediaLocation::External {
            path: "/Volumes/Backup/2026/Hat.wav".into()
        }
    );
    assert_eq!(candidates(&evs, s.id), Some(vec![]));
    assert_eq!(h.missing(), vec![s.id]);
    // Both relinks were undoable steps.
    h.undo();
    assert_eq!(h.media(hh.id).location, hh.location);

    // A name match with other content is a candidate, not relinked.
    h.fs().insert(
        "/Volumes/Backup/Snare.wav".into(),
        wav(48_000, &[vec![0.7; 7]]),
    );
    h.ok(Command::MediaRef(MediaRefCommand::Search {
        media: Some(s.id),
        folder: Some("/Volumes/Backup".into()),
    }));
    let evs = h.settle();
    assert_eq!(
        candidates(&evs, s.id),
        Some(vec![MediaSource::Path {
            path: "/Volumes/Backup/Snare.wav".into()
        }])
    );
    assert_eq!(h.media(s.id).location, s.location);

    // A folder that isn't there.
    let e = err(&h.send(Command::MediaRef(MediaRefCommand::Search {
        media: None,
        folder: Some("/nope".into()),
    })));
    assert_eq!(e.code, ErrorCode::NotFound);
}

#[test]
fn collect_all_copies_external_media_in_one_undo_step_and_saves() {
    let mut h = H::new(&[(DESKTOP, kick()), ("/Users/ana/Desktop/Snare.wav", snare())]);
    let k = h.import(MediaSource::Path {
        path: DESKTOP.into(),
    });
    let s = h.import(MediaSource::Path {
        path: "/Users/ana/Desktop/Snare.wav".into(),
    });
    h.settle();
    h.fs().remove("/Users/ana/Desktop/Snare.wav");
    let out = h.send(Command::MediaRef(MediaRefCommand::CollectAll));
    ok(&out);
    let evs = events(&out);
    assert!(evs.iter().any(|e| matches!(
        e,
        Event::MediaRef {
            event: MediaRefEvent::CollectProgress { done: 2, total: 2 }
        }
    )));
    assert!(
        evs.iter().any(|e| matches!(
            e,
            Event::Project {
                event: ProjectEvent::Saved { .. }
            }
        )),
        "saved"
    );
    assert_eq!(warnings(&out).len(), 1, "the missing snare is reported");
    assert_eq!(h.media(k.id).location, MediaLocation::Project);
    assert_eq!(h.stored(&k.file), Some(kick()));
    assert_eq!(h.media(s.id).location, s.location, "missing: left as is");
    // The saved document says so.
    let saved = h.ctl.store.load(h.pid).unwrap();
    assert!(!saved.contains(DESKTOP), "{saved}");
    // Plays from the project copy even without the original.
    h.fs().remove(DESKTOP);
    let evs = h.reopen();
    assert!(!evs.iter().any(|e| is_missing_event(e, k.id)));
    assert_eq!(h.missing(), vec![s.id]);
    // One undo step (before the reopen, history is per session): checked directly.
    let mut h = H::new(&[(DESKTOP, kick())]);
    let k = h.import(MediaSource::Path {
        path: DESKTOP.into(),
    });
    h.ok(Command::MediaRef(MediaRefCommand::CollectAll));
    h.undo();
    assert_eq!(h.media(k.id).location, k.location);
}

#[test]
fn project_media_stay_project_media() {
    // Media in the project folder (v0.1 imports, recordings) read from the project copy.
    let mut h = H::new(&[]);
    let pid = h.pid;
    h.ctl.store.write(pid, "media/old.wav", &kick()).unwrap();
    let m = h.import(MediaSource::Location {
        location: BrowseLocation::ProjectMedia,
        path: "old.wav".into(),
    });
    assert_eq!(m.location, MediaLocation::Project);
    h.settle();
    assert!(h.ctl.bridge.media.contains_key(&m.id));
    assert!(h.missing().is_empty());
}

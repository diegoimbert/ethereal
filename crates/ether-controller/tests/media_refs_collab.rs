//! `media-references` in a collab session (CONTRACTS.md §12.9): a media's location is
//! per-site. A desktop site references files in place; **its local absolute paths never
//! leave the machine** (no transaction, snapshot or media chunk carries them, including
//! after `Relink` and `CollectAll`); peers receive the bytes by hash and store them at
//! `MediaRef::file` in their own project. Stored media are never replaced, so a relink to
//! other content is refused during a session.

#[path = "common/mod.rs"]
mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use common::{FakeBridge, T0, ok, wav};
use ether_collab::memory::Hub;
use ether_collab::relay::RelayConfig;
use ether_collab::wire::SnapshotData;
use ether_collab::{BoxTransport, CollabTransport, ConnectRequest, LinkState};
use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::store::{Library, StoreError};
use ether_controller::{Controller, ControllerConfig, EtherController, HostServices};
use ether_core::protocol::collab::{CollabCommand, CollabMessage};
use ether_core::protocol::media::{BrowseRoot, DirectoryListing, MediaCommand, MediaSource};
use ether_core::protocol::media_refs::MediaRefCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::project::ProjectCommand;
use ether_core::protocol::*;

/// The desktop site's own files.
#[derive(Default)]
struct Os {
    inner: MemoryLibrary,
    files: BTreeMap<String, Vec<u8>>,
}

impl Library for Os {
    fn roots(&self) -> Vec<BrowseRoot> {
        self.inner.roots()
    }
    fn list_dir(&mut self, root: &str, rel: &str) -> Result<DirectoryListing, StoreError> {
        self.inner.list_dir(root, rel)
    }
    fn read(&mut self, root: &str, rel: &str) -> Result<Vec<u8>, StoreError> {
        self.inner.read(root, rel)
    }
    fn read_external(&mut self, path: &str) -> Result<Vec<u8>, StoreError> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(path.into()))
    }
}

/// Records every message a site sends (snapshots decoded to their `.ether` text).
struct Tap {
    inner: BoxTransport,
    log: Arc<Mutex<Vec<String>>>,
}

impl CollabTransport for Tap {
    fn send(&mut self, message: &CollabMessage) {
        let mut text = serde_json::to_string(message).expect("serializes");
        if let CollabMessage::Snapshot { data } = message {
            text.push_str(&SnapshotData::decode(data).expect("snapshot").ether);
        }
        self.log.lock().unwrap().push(text);
        self.inner.send(message);
    }
    fn poll(&mut self, out: &mut Vec<CollabMessage>) {
        self.inner.poll(out)
    }
    fn state(&self) -> LinkState {
        self.inner.state()
    }
    fn close(&mut self) {
        self.inner.close()
    }
}

struct Host {
    now: u64,
    seed: u64,
}

impl HostServices for Host {
    fn now_ms(&self) -> u64 {
        self.now
    }
    fn random_seed(&mut self) -> u64 {
        self.seed
    }
}

struct Site {
    ctl: EtherController<FakeBridge, Host, MemoryStore, Os>,
    ids: IdGen,
    next: u32,
    sent: Arc<Mutex<Vec<String>>>,
}

impl Site {
    fn new(seed: u64, hub: &Hub, files: &[(&str, Vec<u8>)]) -> Self {
        let mut store = MemoryStore::new();
        store.now_ms = T0;
        let mut ctl = EtherController::with_config(
            FakeBridge::default(),
            Host { now: T0, seed },
            store,
            Os {
                inner: MemoryLibrary::new(),
                files: files
                    .iter()
                    .map(|(p, b)| (p.to_string(), b.clone()))
                    .collect(),
            },
            ControllerConfig {
                autosave_after_ms: None,
                ..ControllerConfig::default()
            },
        );
        let sent = Arc::new(Mutex::new(Vec::new()));
        let log = sent.clone();
        let mut connect = hub.connector();
        ctl.set_collab_connector(Box::new(move |r: &ConnectRequest| -> BoxTransport {
            Box::new(Tap {
                inner: connect(r),
                log: log.clone(),
            })
        }));
        Self {
            ctl,
            ids: IdGen::new(seed.wrapping_mul(7919)),
            next: 1,
            sent,
        }
    }

    fn ok(&mut self, command: Command) -> ReplyValue {
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
        ok(&out)
    }

    fn err(&mut self, command: Command) -> CommandError {
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
        common::err(&out)
    }

    fn tick(&mut self) {
        self.ctl.host.now += 20;
        self.ctl.store.now_ms = self.ctl.host.now;
        let now = self.ctl.host.now;
        let mut out = Vec::new();
        self.ctl.tick(now, &mut out);
    }

    fn join(&mut self, name: &str) {
        self.ok(Command::Collab(CollabCommand::Join {
            server: "ws://hub".into(),
            session: "refs".into(),
            token: None,
            name: name.into(),
        }));
    }

    fn import(&mut self, path: &str) -> MediaId {
        let id: MediaId = self.ids.next(T0);
        self.ok(Command::Media(MediaCommand::Import {
            id,
            source: MediaSource::Path { path: path.into() },
        }));
        id
    }

    fn project(&self) -> &Project {
        self.ctl.project().expect("open project")
    }

    fn stored(&self, media: MediaId) -> Option<Vec<u8>> {
        let p = self.project();
        let m = p.media.get(&media)?;
        self.ctl.store.file(p.id, &m.file).map(<[u8]>::to_vec)
    }

    fn leaked(&self, needle: &str) -> Vec<String> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.contains(needle))
            .map(|m| m.chars().take(300).collect())
            .collect()
    }
}

fn settle(sites: &mut [&mut Site], hub: &Hub) {
    for _ in 0..500 {
        for s in sites.iter_mut() {
            s.tick();
        }
        let delivered = hub.deliver();
        let waiting: usize = hub.links().iter().map(|c| hub.outgoing(*c)).sum();
        let pending: usize = sites.iter().map(|s| s.ctl.collab_pending()).sum();
        if delivered == 0 && waiting == 0 && pending == 0 {
            for s in sites.iter_mut() {
                s.tick();
            }
            return;
        }
    }
    panic!("sites did not settle");
}

const HOME: &str = "/Users/ana";
const KICK: &str = "/Users/ana/Samples/Kick.wav";
const KICK_MOVED: &str = "/Users/ana/Archive/Kick.wav";
const SNARE: &str = "/Users/ana/Samples/Snare.wav";
const SNARE_EDIT: &str = "/Users/ana/Samples/Snare (edit).wav";
const HAT: &str = "/Users/ana/Desktop/Hat.wav";

#[test]
fn local_paths_never_leave_the_site() {
    // Compaction snapshots from the sites happen during the run too.
    let hub = Hub::new(RelayConfig {
        compact_after: 4,
        max_log: 40,
        ..Default::default()
    });
    let kick = wav(48_000, &[vec![0.5; 2_400]]);
    let snare = wav(48_000, &[vec![-0.5; 1_200]]);
    let snare_edit = wav(48_000, &[vec![-0.2; 1_200]]);
    let hat = wav(48_000, &[vec![0.1; 600]]);
    let mut a = Site::new(
        51,
        &hub,
        &[
            (KICK, kick.clone()),
            (KICK_MOVED, kick.clone()),
            (SNARE, snare.clone()),
            (SNARE_EDIT, snare_edit.clone()),
            (HAT, hat.clone()),
        ],
    );
    let mut b = Site::new(52, &hub, &[]);
    let pid: ProjectId = a.ids.next_project_id(T0);
    a.ok(Command::Project(ProjectCommand::Create {
        id: pid,
        name: "Refs".into(),
    }));
    // Referenced before the session: in the creation snapshot.
    let k = a.import(KICK);
    a.join("A");
    settle(&mut [&mut a], &hub);
    b.join("B");
    settle(&mut [&mut a, &mut b], &hub);
    // Referenced during the session: in a transaction.
    let s = a.import(SNARE);
    let h = a.import(HAT);
    settle(&mut [&mut a, &mut b], &hub);
    for m in [k, s, h] {
        assert!(matches!(
            a.project().media[&m].location,
            MediaLocation::External { .. }
        ));
        assert_eq!(b.project().media[&m].location, MediaLocation::Project);
    }
    assert_eq!(b.stored(k), Some(kick.clone()));
    assert_eq!(b.stored(s), Some(snare.clone()));

    // Relink to the same content elsewhere (local only); other content is refused.
    a.ok(Command::MediaRef(MediaRefCommand::Relink {
        media: k,
        source: MediaSource::Path {
            path: KICK_MOVED.into(),
        },
    }));
    let e = a.err(Command::MediaRef(MediaRefCommand::Relink {
        media: s,
        source: MediaSource::Path {
            path: SNARE_EDIT.into(),
        },
    }));
    assert_eq!(e.code, ErrorCode::InvalidState, "{e:?}");
    settle(&mut [&mut a, &mut b], &hub);
    for _ in 0..50 {
        a.tick();
        b.tick();
    }
    assert_eq!(
        a.project().media[&k].location,
        MediaLocation::External {
            path: KICK_MOVED.into()
        }
    );
    assert_eq!(b.project().media[&k].location, MediaLocation::Project);
    assert_eq!(b.project().media[&s].hash, a.project().media[&s].hash);
    assert_eq!(b.stored(s), Some(snare.clone()));
    assert!(!b.ctl.media_pending());
    assert!(b.ctl.bridge.media.contains_key(&s));

    // Collect all (local: A now reads its project copies) and save.
    a.ok(Command::MediaRef(MediaRefCommand::CollectAll));
    settle(&mut [&mut a, &mut b], &hub);
    assert!(
        a.project()
            .media
            .values()
            .all(|m| m.location == MediaLocation::Project)
    );
    assert_eq!(a.stored(h), Some(hat));

    // Undo the collect (local again), then a late joiner gets the session state.
    a.ok(Command::Edit(ether_core::protocol::project::EditCommand::Undo));
    let mut c = Site::new(53, &hub, &[]);
    c.join("C");
    settle(&mut [&mut a, &mut b, &mut c], &hub);
    assert_eq!(c.stored(k), Some(kick));
    assert_eq!(c.stored(s), Some(snare));
    for site in [&b, &c] {
        assert!(
            site.project()
                .media
                .values()
                .all(|m| m.location == MediaLocation::Project)
        );
    }

    // Nothing any site sent carries A's paths.
    for site in [&a, &b, &c] {
        assert!(!site.sent.lock().unwrap().is_empty());
        let leaks = site.leaked(HOME);
        assert!(leaks.is_empty(), "local paths sent: {leaks:#?}");
    }
    assert!(
        a.sent
            .lock()
            .unwrap()
            .iter()
            .any(|m| m.contains("\"Snapshot\"")),
        "the snapshots were checked"
    );
}

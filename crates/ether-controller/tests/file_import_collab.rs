//! `file-import` in a collab session (CONTRACTS.md §12.13): a file imported from the user's
//! computer on one site plays on every site. A web or remote site uploads the bytes
//! (`BeginUpload` → `UploadChunk`s → `Import { source: Upload }`), a desktop site imports an
//! OS path (`Import { source: Path }`, read with `Library::read_external`); either way the
//! media's bytes are pushed ahead of the transaction and peers store them at
//! `MediaRef::file`. Late joiners get them from the relay cache.

#[path = "common/mod.rs"]
mod common;

use std::collections::BTreeMap;

use common::{FakeBridge, T0, ok, wav};
use ether_collab::memory::Hub;
use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::store::{Library, ProjectStore, StoreError};
use ether_controller::{Controller, ControllerConfig, EtherController, HostServices};
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::collab::CollabCommand;
use ether_core::protocol::media::{
    BrowseRoot, DirectoryListing, MediaCommand, MediaEvent, MediaSource,
};
use ether_core::protocol::model::*;
use ether_core::protocol::project::{ProjectCommand, ProjectSummary};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::*;

/// `MemoryStore` with upload staging (as the native and OPFS stores have).
#[derive(Default)]
struct Store {
    inner: MemoryStore,
    staged: BTreeMap<String, Vec<u8>>,
}

impl ProjectStore for Store {
    fn list(&mut self) -> Result<Vec<ProjectSummary>, StoreError> {
        self.inner.list()
    }
    fn create(&mut self, id: ProjectId) -> Result<(), StoreError> {
        self.inner.create(id)
    }
    fn load(&mut self, id: ProjectId) -> Result<String, StoreError> {
        self.inner.load(id)
    }
    fn save(&mut self, id: ProjectId, json: &str) -> Result<ProjectSummary, StoreError> {
        self.inner.save(id, json)
    }
    fn duplicate(&mut self, from: ProjectId, to: ProjectId) -> Result<(), StoreError> {
        self.inner.duplicate(from, to)
    }
    fn delete(&mut self, id: ProjectId) -> Result<(), StoreError> {
        self.inner.delete(id)
    }
    fn read(&mut self, id: ProjectId, rel: &str) -> Result<Vec<u8>, StoreError> {
        self.inner.read(id, rel)
    }
    fn write(&mut self, id: ProjectId, rel: &str, bytes: &[u8]) -> Result<(), StoreError> {
        self.inner.write(id, rel, bytes)
    }
    fn list_dir(&mut self, id: ProjectId, rel: &str) -> Result<DirectoryListing, StoreError> {
        self.inner.list_dir(id, rel)
    }
    fn begin_upload(&mut self, upload: &str, _size: u64) -> Result<(), StoreError> {
        self.staged.insert(upload.into(), Vec::new());
        Ok(())
    }
    fn append_upload(&mut self, upload: &str, _offset: u64, b: &[u8]) -> Result<u64, StoreError> {
        let data = self
            .staged
            .get_mut(upload)
            .ok_or_else(|| StoreError::NotFound(upload.into()))?;
        data.extend_from_slice(b);
        Ok(data.len() as u64)
    }
    fn read_upload(&mut self, upload: &str) -> Result<Vec<u8>, StoreError> {
        self.staged
            .get(upload)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(upload.into()))
    }
    fn discard_upload(&mut self, upload: &str) -> Result<(), StoreError> {
        self.staged.remove(upload);
        Ok(())
    }
}

/// A library that can read the site's own OS files by absolute path (a desktop host).
#[derive(Default)]
struct OsLibrary {
    inner: MemoryLibrary,
    os_files: BTreeMap<String, Vec<u8>>,
}

impl Library for OsLibrary {
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
        self.os_files
            .get(path)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(path.into()))
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
    ctl: EtherController<FakeBridge, Host, Store, OsLibrary>,
    ids: IdGen,
    next: u32,
    log: Vec<ServerMessage>,
}

impl Site {
    fn new(seed: u64, hub: &Hub, os_files: BTreeMap<String, Vec<u8>>) -> Self {
        let mut ctl = EtherController::with_config(
            FakeBridge::default(),
            Host { now: T0, seed },
            Store::default(),
            OsLibrary {
                inner: MemoryLibrary::new(),
                os_files,
            },
            ControllerConfig {
                autosave_after_ms: None,
                ..ControllerConfig::default()
            },
        );
        ctl.set_collab_connector(hub.connector());
        Self {
            ctl,
            ids: IdGen::new(seed.wrapping_mul(7919)),
            next: 1,
            log: Vec::new(),
        }
    }

    fn id<I: Id>(&mut self) -> I {
        self.ids.next(T0)
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
        self.log.extend(out.iter().cloned());
        out
    }

    fn ok(&mut self, command: Command) -> ReplyValue {
        let out = self.send(command);
        ok(&out)
    }

    fn tick(&mut self) {
        self.ctl.host.now += 20;
        self.ctl.store.inner.now_ms = self.ctl.host.now;
        let now = self.ctl.host.now;
        let mut out = Vec::new();
        self.ctl.tick(now, &mut out);
        self.log.extend(out);
    }

    fn join(&mut self, name: &str) {
        self.ok(Command::Collab(CollabCommand::Join {
            server: "ws://hub".into(),
            session: "imports".into(),
            token: None,
            name: name.into(),
        }));
    }

    fn project(&self) -> &Project {
        self.ctl.project().expect("open project")
    }

    /// The bytes this site stores for `media` (its project copy).
    fn media_bytes(&self, pid: ProjectId, media: MediaId) -> Option<Vec<u8>> {
        let m = self.project().media.get(&media)?;
        self.ctl.store.inner.file(pid, &m.file).map(<[u8]>::to_vec)
    }

    fn missing(&self) -> bool {
        self.log.iter().any(|e| {
            matches!(
                e,
                ServerMessage::Event(Event::Media {
                    event: MediaEvent::Missing { .. }
                })
            )
        })
    }

    /// Import + a clip on a new audio track (what a drop below the tracks does).
    fn clip_of(&mut self, media: MediaId) -> ClipId {
        let track: TrackId = self.id();
        self.ok(Command::Track(TrackCommand::Create {
            id: track,
            kind: TrackKind::Audio,
            name: None,
            color: None,
            parent: None,
            before: None,
        }));
        let clip: ClipId = self.id();
        self.ok(Command::Clip(ClipCommand::CreateAudio {
            id: clip,
            track,
            start: Beats(0.0),
            media,
        }));
        clip
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

const DESKTOP_KICK: &str = "/Users/ana/Samples/Kick 01.wav";

#[test]
fn imported_files_play_on_every_site() {
    let hub = Hub::default();
    let kick = wav(48_000, &[vec![0.5; 2_400], vec![-0.5; 2_400]]);
    let pad = wav(44_100, &[vec![0.1; 44_100]]);
    // A: a desktop site with a file on its disk. B: a web (or remote) site.
    let mut a = Site::new(
        41,
        &hub,
        BTreeMap::from([(DESKTOP_KICK.into(), kick.clone())]),
    );
    let mut b = Site::new(42, &hub, BTreeMap::new());
    let pid: ProjectId = a.ids.next_project_id(T0);
    a.ok(Command::Project(ProjectCommand::Create {
        id: pid,
        name: "Jam".into(),
    }));
    a.join("A");
    settle(&mut [&mut a], &hub);
    b.join("B");
    settle(&mut [&mut a, &mut b], &hub);

    // A imports a path from its disk (the Tauri dialog / an OS drop).
    let from_disk: MediaId = a.id();
    a.ok(Command::Media(MediaCommand::Import {
        id: from_disk,
        source: MediaSource::Path {
            path: DESKTOP_KICK.into(),
        },
    }));
    let clip_a = a.clip_of(from_disk);

    // B uploads a file from its browser in chunks.
    let upload = "up-1".to_string();
    b.ok(Command::Media(MediaCommand::BeginUpload {
        upload: upload.clone(),
        name: "Pad.wav".into(),
        size: pad.len() as f64,
    }));
    for (i, chunk) in pad.chunks(8_192).enumerate() {
        b.ok(Command::Media(MediaCommand::UploadChunk {
            upload: upload.clone(),
            offset: (i * 8_192) as f64,
            data: Base64Bytes(chunk.to_vec()),
        }));
    }
    let uploaded: MediaId = b.id();
    b.ok(Command::Media(MediaCommand::Import {
        id: uploaded,
        source: MediaSource::Upload { upload },
    }));
    let clip_b = b.clip_of(uploaded);

    settle(&mut [&mut a, &mut b], &hub);
    for _ in 0..100 {
        a.tick();
        b.tick();
    }
    // Both sites have both files, and every clip.
    for s in [&a, &b] {
        assert_eq!(s.media_bytes(pid, from_disk), Some(kick.clone()));
        assert_eq!(s.media_bytes(pid, uploaded), Some(pad.clone()));
        assert!(s.project().clips.contains_key(&clip_a));
        assert!(s.project().clips.contains_key(&clip_b));
        assert!(!s.missing(), "never missing");
        assert!(!s.ctl.media_pending(), "decoded");
    }
    assert_eq!(a.project().media, b.project().media);

    // A late joiner gets them too (relay cache).
    let mut c = Site::new(43, &hub, BTreeMap::new());
    c.join("C");
    settle(&mut [&mut a, &mut b, &mut c], &hub);
    assert_eq!(c.media_bytes(pid, from_disk), Some(kick));
    assert_eq!(c.media_bytes(pid, uploaded), Some(pad));
}

//! Uploads from the UI machine (remote-engine): `BeginUpload` → `UploadChunk`s →
//! `Import { source: Upload }`, resume after a gap or a retried chunk, cancel, limits and
//! idle cleanup. The store is `MemoryStore` plus in-memory staging (the native staging is
//! tested in `ether-native/src/uploads.rs`).

mod common;

use std::collections::BTreeMap;

use common::*;
use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::store::{ProjectStore, StoreError};
use ether_controller::{Controller, ControllerConfig, EtherController};
use ether_core::protocol::media::{DirectoryListing, MediaCommand, MediaEvent, MediaSource};
use ether_core::protocol::model::*;
use ether_core::protocol::project::{ProjectCommand, ProjectSummary};
use ether_core::protocol::*;

/// `MemoryStore` with upload staging.
#[derive(Default)]
struct StagingStore {
    inner: MemoryStore,
    staged: BTreeMap<String, (u64, Vec<u8>)>,
}

impl ProjectStore for StagingStore {
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
    fn begin_upload(&mut self, upload: &str, size: u64) -> Result<(), StoreError> {
        self.staged.insert(upload.into(), (size, Vec::new()));
        Ok(())
    }
    fn append_upload(&mut self, upload: &str, offset: u64, b: &[u8]) -> Result<u64, StoreError> {
        let (_, data) = self
            .staged
            .get_mut(upload)
            .ok_or_else(|| StoreError::NotFound(upload.into()))?;
        assert_eq!(offset, data.len() as u64, "controller checks offsets first");
        data.extend_from_slice(b);
        Ok(data.len() as u64)
    }
    fn read_upload(&mut self, upload: &str) -> Result<Vec<u8>, StoreError> {
        self.staged
            .get(upload)
            .map(|(_, d)| d.clone())
            .ok_or_else(|| StoreError::NotFound(upload.into()))
    }
    fn discard_upload(&mut self, upload: &str) -> Result<(), StoreError> {
        self.staged.remove(upload);
        Ok(())
    }
}

struct H {
    ctl: EtherController<FakeBridge, FakeHost, StagingStore, MemoryLibrary>,
    next: u32,
    ids: IdGen,
}

impl H {
    fn new() -> Self {
        let mut h = Self {
            ctl: EtherController::with_config(
                FakeBridge::default(),
                FakeHost { now: T0 },
                StagingStore::default(),
                MemoryLibrary::new(),
                ControllerConfig::default(),
            ),
            next: 1,
            ids: IdGen::new(7),
        };
        let id = h.ids.next_project_id(T0);
        h.ok(Command::Project(ProjectCommand::Create {
            id,
            name: "Up".into(),
        }));
        h
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
        assert!(matches!(out.last(), Some(ServerMessage::Reply(r)) if r.id == id));
        out
    }

    fn ok(&mut self, command: Command) -> ReplyValue {
        ok(&self.send(command))
    }

    fn begin(&mut self, upload: &str, name: &str, size: usize) -> Vec<ServerMessage> {
        self.send(Command::Media(MediaCommand::BeginUpload {
            upload: upload.into(),
            name: name.into(),
            size: size as f64,
        }))
    }

    fn chunk(&mut self, upload: &str, offset: usize, data: &[u8]) -> Vec<ServerMessage> {
        self.send(Command::Media(MediaCommand::UploadChunk {
            upload: upload.into(),
            offset: offset as f64,
            data: Base64Bytes(data.to_vec()),
        }))
    }

    fn import(&mut self, upload: &str) -> (MediaId, Vec<ServerMessage>) {
        let id: MediaId = self.ids.next(T0);
        let out = self.send(Command::Media(MediaCommand::Import {
            id,
            source: MediaSource::Upload {
                upload: upload.into(),
            },
        }));
        (id, out)
    }
}

fn progress(out: &[ServerMessage]) -> Vec<f64> {
    events(out)
        .into_iter()
        .filter_map(|e| match e {
            Event::Media {
                event: MediaEvent::UploadProgress { received, .. },
            } => Some(received),
            _ => None,
        })
        .collect()
}

fn tone() -> Vec<u8> {
    wav(48_000, &[sine(48_000, 440.0, 4_800, 0.5)])
}

#[test]
fn upload_then_import_as_project_media() {
    let mut h = H::new();
    let bytes = tone();
    ok(&h.begin("u1", "dir/kick.wav", bytes.len()));
    let (a, b) = bytes.split_at(1000);
    assert_eq!(progress(&h.chunk("u1", 0, a)), vec![1000.0]);
    // Within the progress interval: throttled, except when the upload completes.
    let out = h.chunk("u1", 1000, b);
    assert_eq!(progress(&out), vec![bytes.len() as f64]);
    let (id, out) = h.import("u1");
    let ReplyValue::Media { media } = ok(&out) else {
        panic!()
    };
    assert_eq!(media.id, id);
    assert_eq!(media.name, "kick.wav", "display name only, never a path");
    assert!(media.file.starts_with("media/"));
    assert_eq!(patches(&out).len(), 1);
    let pid = h.ctl.project().unwrap().id;
    assert_eq!(
        h.ctl.store.inner.file(pid, &media.file).unwrap(),
        &bytes[..]
    );
    // Consumed: staging dropped, a second import is NotFound.
    assert!(h.ctl.store.staged.is_empty());
    assert_eq!(err(&h.import("u1").1).code, ErrorCode::NotFound);
}

#[test]
fn resume_after_gap_and_retried_chunk() {
    let mut h = H::new();
    let bytes = tone();
    ok(&h.begin("u", "a.wav", bytes.len()));
    ok(&h.chunk("u", 0, &bytes[..100]));
    // A gap is rejected and names the expected offset.
    let e = err(&h.chunk("u", 200, &bytes[200..300]));
    assert_eq!(e.code, ErrorCode::InvalidState);
    assert!(e.message.contains("expected offset 100"), "{}", e.message);
    // A retried chunk (reply lost) is a no-op.
    ok(&h.chunk("u", 0, &bytes[..100]));
    ok(&h.chunk("u", 50, &bytes[50..100]));
    // Import before completion: InvalidState, upload kept.
    assert_eq!(err(&h.import("u").1).code, ErrorCode::InvalidState);
    // Resume from the expected offset.
    ok(&h.chunk("u", 100, &bytes[100..]));
    // Too many bytes.
    assert_eq!(
        err(&h.chunk("u", bytes.len(), &[0])).code,
        ErrorCode::InvalidArgument
    );
    ok(&h.import("u").1);
}

#[test]
fn cancel_and_restart() {
    let mut h = H::new();
    ok(&h.begin("u", "a.wav", 10));
    ok(&h.chunk("u", 0, &[1; 5]));
    h.ok(Command::Media(MediaCommand::CancelUpload {
        upload: "u".into(),
    }));
    assert!(h.ctl.store.staged.is_empty());
    assert_eq!(err(&h.chunk("u", 5, &[1; 5])).code, ErrorCode::NotFound);
    // Unknown ids cancel fine.
    h.ok(Command::Media(MediaCommand::CancelUpload {
        upload: "x".into(),
    }));
    // Begin again with the same id restarts from 0.
    ok(&h.begin("u", "a.wav", 4));
    ok(&h.chunk("u", 0, &[1; 2]));
    ok(&h.begin("u", "a.wav", 4));
    assert_eq!(err(&h.chunk("u", 2, &[1; 2])).code, ErrorCode::InvalidState);
    ok(&h.chunk("u", 0, &[1; 4]));
    // Not audio: decode error, and the upload is consumed anyway.
    assert_eq!(err(&h.import("u").1).code, ErrorCode::Decode);
    assert!(h.ctl.store.staged.is_empty());
}

#[test]
fn limits_and_idle_cleanup() {
    let mut h = H::new();
    assert_eq!(
        err(&h.begin("u", "a.wav", 0)).code,
        ErrorCode::InvalidArgument
    );
    assert_eq!(
        err(&h.begin("u", "a.wav", (1usize << 30) + 1)).code,
        ErrorCode::InvalidArgument
    );
    assert_eq!(
        err(&h.begin("u", " / ", 3)).code,
        ErrorCode::InvalidArgument
    );
    ok(&h.begin("big", "a.wav", 3 << 20));
    assert_eq!(
        err(&h.chunk("big", 0, &vec![0; (1 << 20) + 1])).code,
        ErrorCode::InvalidArgument
    );
    for i in 1..16 {
        ok(&h.begin(&format!("u{i}"), "a.wav", 1));
    }
    assert_eq!(
        err(&h.begin("one-more", "a.wav", 1)).code,
        ErrorCode::InvalidState
    );
    // Abandoned uploads are dropped after the idle timeout.
    h.ctl.host.now += 10 * 60 * 1000;
    ok(&h.begin("fresh", "a.wav", 1));
    assert_eq!(h.ctl.store.staged.len(), 1);
    assert_eq!(err(&h.chunk("big", 0, &[0])).code, ErrorCode::NotFound);
}

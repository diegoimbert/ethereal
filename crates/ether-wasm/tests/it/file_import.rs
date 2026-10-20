//! `file-import` on the web: the OPFS store stages uploads (`uploads/<id>/<offset>` chunk
//! files), so the local wasm controller imports files the user drops or picks
//! (`BeginUpload` → `UploadChunk`s → `Import { source: Upload }`, copied into the project).
//! OS paths don't exist there: `Import { source: Path }` replies `Unsupported`.

use ether_controller::store::{ProjectStore, StoreError};
use ether_controller::{Controller, ControllerConfig, EtherController, HostServices};
use ether_core::protocol::media::{MediaCommand, MediaEvent, MediaSource};
use ether_core::protocol::message::{Command, Event, ReplyResult, ReplyValue};
use ether_core::protocol::model::{Base64Bytes, MediaId, ProjectId, Ulid};
use ether_core::protocol::project::ProjectCommand;
use ether_core::protocol::{ClientMessage, ErrorCode, ServerMessage};
use ether_wasm::bridge::{self, WebBridge};
use ether_wasm::ring::HeapMemory;
use ether_wasm::store::{MemFs, UPLOADS_ROOT, WebLibrary, WebStore};

struct TestHost;

impl HostServices for TestHost {
    fn now_ms(&self) -> u64 {
        1_750_000_000_000
    }
    fn random_seed(&mut self) -> u64 {
        42
    }
}

fn pid() -> ProjectId {
    ProjectId::v7(1_750_000_000_000, [7; 10])
}

fn staged(fs: &MemFs) -> Vec<String> {
    fs.files()
        .into_iter()
        .filter(|f| f.starts_with(&format!("{UPLOADS_ROOT}/")))
        .collect()
}

#[test]
fn opfs_staging_appends_reads_and_discards() {
    let fs = MemFs::new();
    let mut s = WebStore::new(fs.clone());
    // A leftover of a closed tab is dropped when the first upload of a session begins.
    use ether_wasm::store::Fs;
    let mut raw = fs.clone();
    raw.write(&format!("{UPLOADS_ROOT}/old/0000000000000000"), b"stale")
        .unwrap();

    s.begin_upload("a-1", 10).unwrap();
    assert!(staged(&fs).is_empty(), "{:?}", staged(&fs));
    assert_eq!(s.append_upload("a-1", 0, b"0123").unwrap(), 4);
    assert!(
        matches!(s.append_upload("a-1", 2, b"xx"), Err(StoreError::Io(_))),
        "gap"
    );
    assert!(
        matches!(
            s.append_upload("a-1", 4, b"4567890"),
            Err(StoreError::Io(_))
        ),
        "past the announced size"
    );
    assert!(
        matches!(s.read_upload("a-1"), Err(StoreError::Io(_))),
        "incomplete"
    );
    assert_eq!(s.append_upload("a-1", 4, b"456789").unwrap(), 10);
    assert_eq!(s.read_upload("a-1").unwrap(), b"0123456789");

    // Restarting an id starts over.
    s.begin_upload("a-1", 2).unwrap();
    assert_eq!(s.append_upload("a-1", 0, b"ab").unwrap(), 2);
    assert_eq!(s.read_upload("a-1").unwrap(), b"ab");

    s.discard_upload("a-1").unwrap();
    s.discard_upload("a-1").unwrap();
    assert!(staged(&fs).is_empty());
    assert!(matches!(s.read_upload("a-1"), Err(StoreError::NotFound(_))));
    for bad in ["", "../x", "a/b", "a.b", &"x".repeat(65)] {
        assert!(
            matches!(s.begin_upload(bad, 1), Err(StoreError::InvalidPath(_))),
            "{bad}"
        );
    }
    // Never listed as a project.
    assert!(s.list().unwrap().is_empty());
}

fn send(ctl: &mut impl Controller, id: u32, command: Command) -> Vec<ServerMessage> {
    let mut out = Vec::new();
    ctl.handle(
        ClientMessage {
            id,
            gesture: None,
            command,
        },
        &mut out,
    );
    out
}

fn reply(out: &[ServerMessage]) -> &ReplyResult {
    match out.last() {
        Some(ServerMessage::Reply(r)) => &r.result,
        other => panic!("last message must be the reply, got {other:?}"),
    }
}

#[test]
fn local_web_controller_imports_uploaded_files() {
    let control = HeapMemory::new(1 << 16);
    let reports = HeapMemory::new(1 << 14);
    let shared = bridge::shared(control, reports);
    let fs = MemFs::new();
    let mut ctl = EtherController::with_config(
        WebBridge::new(shared),
        TestHost,
        WebStore::new(fs.clone()),
        WebLibrary::new(fs.clone()),
        ControllerConfig {
            engine_sample_rate: 48_000,
            ..ControllerConfig::default()
        },
    );
    let out = send(
        &mut ctl,
        1,
        Command::Project(ProjectCommand::Create {
            id: pid(),
            name: "Web".into(),
        }),
    );
    assert!(matches!(reply(&out), ReplyResult::Ok { .. }), "{out:?}");

    let (_, kick) = ether_media::demo::demo_samples().remove(0);
    let upload = "u1".to_string();
    let media = MediaId(Ulid(5));
    let out = send(
        &mut ctl,
        2,
        Command::Media(MediaCommand::BeginUpload {
            upload: upload.clone(),
            name: "My Kick.wav".into(),
            size: kick.len() as f64,
        }),
    );
    assert!(matches!(reply(&out), ReplyResult::Ok { .. }), "{out:?}");
    let mut progress = Vec::new();
    for (i, chunk) in kick.chunks(4096).enumerate() {
        let out = send(
            &mut ctl,
            10 + i as u32,
            Command::Media(MediaCommand::UploadChunk {
                upload: upload.clone(),
                offset: (i * 4096) as f64,
                data: Base64Bytes(chunk.to_vec()),
            }),
        );
        assert!(matches!(reply(&out), ReplyResult::Ok { .. }), "{out:?}");
        for m in out {
            if let ServerMessage::Event(Event::Media {
                event: MediaEvent::UploadProgress { received, .. },
            }) = m
            {
                progress.push(received);
            }
        }
    }
    assert_eq!(progress.last().copied(), Some(kick.len() as f64));
    assert!(!staged(&fs).is_empty(), "staged in OPFS, not in memory");
    let out = send(
        &mut ctl,
        100,
        Command::Media(MediaCommand::Import {
            id: media,
            source: MediaSource::Upload { upload },
        }),
    );
    let ReplyResult::Ok {
        value: ReplyValue::Media { media: m },
    } = reply(&out)
    else {
        panic!("{out:?}")
    };
    assert_eq!(m.name, "My Kick.wav");
    assert!(m.channels >= 1 && m.sample_rate > 0);
    let copy = format!("projects/{}/{}", pid(), m.file);
    assert!(fs.files().contains(&copy), "{copy} in {:?}", fs.files());
    assert!(staged(&fs).is_empty(), "staging dropped after import");

    // No OS paths on the web.
    let out = send(
        &mut ctl,
        101,
        Command::Media(MediaCommand::Import {
            id: MediaId(Ulid(6)),
            source: MediaSource::Path {
                path: "/Users/me/kick.wav".into(),
            },
        }),
    );
    let ReplyResult::Err { error } = reply(&out) else {
        panic!("{out:?}")
    };
    assert_eq!(error.code, ErrorCode::Unsupported);
}

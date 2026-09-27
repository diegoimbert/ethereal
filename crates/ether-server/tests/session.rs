//! A full remote session over a real WebSocket: `ether-server` hosting the real native
//! engine (null audio backend, real controller, real disk store), driven by plain
//! tungstenite clients speaking the same JSON the browser sends.
//!
//! hello/auth → create project → MIDI track + synth + clip + notes → play (playhead frames
//! arrive) → a second client sees the first one's patches, gets only its own replies →
//! upload a WAV over binary frames (gap + resume) → import + audio clip → peaks as a binary
//! frame → an unfinished upload is cancelled when its client disconnects → save → restart
//! the server → reopen: identical document.
//!
//! No sleeps for ordering: every step waits on a reply or an event (with a deadline).

use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ether_native::audio::{AudioBackendKind, AudioSettings};
use ether_native::test_util::TempDir;
use ether_native::{DedicatedThread, HostConfig, HostOptions, LibraryRoot, NativeHost};
use ether_protocol::model::IdGen;
use ether_protocol::remote::{
    BinaryKind, ClientHello, HelloRejection, PROTOCOL_VERSION, ServerHello, ServerInfo,
    encode_binary_frame,
};
use ether_protocol::{Event, ServerMessage};
use ether_server::frames::{Frame, decode_server};
use ether_server::{Server, ServerConfig};
use serde_json::{Value, json};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

const TOKEN: &str = "s3cret-token";
const TIMEOUT: Duration = Duration::from_secs(20);

fn start(root: &Path) -> Server {
    let projects_root = root.join("projects");
    let host = NativeHost::start(
        HostConfig {
            audio: Some(AudioSettings {
                backend: AudioBackendKind::Null,
                max_block_size: 256,
                ..Default::default()
            }),
            data_dir: root.to_path_buf(),
            instance: "test".into(),
            projects_root: projects_root.clone(),
            library_roots: vec![LibraryRoot {
                id: "lib".into(),
                name: "Library".into(),
                path: root.join("library"),
            }],
        },
        HostOptions {
            main_thread: Arc::new(DedicatedThread::new()),
            ..HostOptions::default()
        },
    )
    .expect("host starts");
    Server::start(
        ServerConfig {
            token: Some(TOKEN.into()),
            name: Some("test-server".into()),
            instance: "test".into(),
            projects_root: Some(projects_root),
            ..ServerConfig::default()
        },
        host,
    )
    .expect("server starts")
}

type Ws = WebSocket<MaybeTlsStream<TcpStream>>;

fn open(addr: SocketAddr) -> Ws {
    let (ws, _) = tungstenite::connect(format!("ws://{addr}/")).expect("connect");
    if let MaybeTlsStream::Plain(s) = ws.get_ref() {
        s.set_read_timeout(Some(TIMEOUT)).unwrap();
    }
    ws
}

/// Send a hello; returns the server's answer and, for rejections, the close code.
fn hello(ws: &mut Ws, version: u32, token: Option<&str>) -> (ServerHello, Option<u16>) {
    let h = ClientHello {
        protocol_version: version,
        token: token.map(str::to_string),
        client: "session test".into(),
    };
    ws.send(Message::text(serde_json::to_string(&h).unwrap()))
        .unwrap();
    let Message::Text(t) = ws.read().unwrap() else {
        panic!("hello answer must be text")
    };
    let answer: ServerHello = serde_json::from_str(&t).unwrap();
    let mut code = None;
    if matches!(answer, ServerHello::Rejected { .. }) {
        loop {
            match ws.read() {
                Ok(Message::Close(Some(f))) => {
                    code = Some(u16::from(f.code));
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    }
    (answer, code)
}

struct Client {
    ws: Ws,
    next: u32,
    ids: IdGen,
    /// Replies and events received so far (not yet consumed by `reply`).
    log: Vec<ServerMessage>,
    playheads: usize,
    binary_frames: usize,
}

impl Client {
    fn connect(addr: SocketAddr, seed: u64) -> (Self, ServerInfo) {
        let mut ws = open(addr);
        let (answer, _) = hello(&mut ws, PROTOCOL_VERSION, Some(TOKEN));
        let ServerHello::Welcome { server, session } = answer else {
            panic!("rejected: {answer:?}")
        };
        assert_eq!(session.len(), 32);
        (
            Self {
                ws,
                next: 1,
                ids: IdGen::new(seed),
                log: Vec::new(),
                playheads: 0,
                binary_frames: 0,
            },
            server,
        )
    }

    fn id(&mut self) -> String {
        self.ids.next_ulid(now_ms()).to_string()
    }

    fn message(&mut self, domain: &str, command: Value) -> (u32, Value) {
        let id = self.next;
        self.next += 1;
        (
            id,
            json!({"id": id, "gesture": null, "command": {"domain": domain, "command": command}}),
        )
    }

    fn post(&mut self, domain: &str, command: Value) -> u32 {
        let (id, m) = self.message(domain, command);
        self.ws.send(Message::text(m.to_string())).unwrap();
        id
    }

    /// `Media::UploadChunk` as a binary frame.
    fn post_chunk(&mut self, upload: &str, offset: usize, data: &[u8]) -> u32 {
        let (id, m) = self.message(
            "Media",
            json!({"type": "UploadChunk", "upload": upload, "offset": offset, "data": ""}),
        );
        let frame = encode_binary_frame(BinaryKind::Bytes, &m.to_string(), data);
        self.ws.send(Message::binary(frame)).unwrap();
        id
    }

    fn read_one(&mut self) {
        let frame = match self.ws.read().expect("server message") {
            Message::Text(t) => Frame::Text(t.to_string()),
            Message::Binary(b) => {
                self.binary_frames += 1;
                Frame::Binary(b.to_vec())
            }
            _ => return,
        };
        match decode_server(&frame).expect("valid server frame") {
            ServerMessage::Playhead(_) => self.playheads += 1,
            ServerMessage::Meters(_) => {}
            m => self.log.push(m),
        }
    }

    fn wait<T>(&mut self, what: &str, mut f: impl FnMut(&mut Self) -> Option<T>) -> T {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(v) = f(self) {
                return v;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            self.read_one();
        }
    }

    fn reply(&mut self, id: u32) -> Result<Value, (String, String)> {
        let r = self.wait(&format!("reply {id}"), |c| {
            let pos = c
                .log
                .iter()
                .position(|m| matches!(m, ServerMessage::Reply(r) if r.id == id))?;
            let ServerMessage::Reply(r) = c.log.remove(pos) else {
                unreachable!()
            };
            Some(r)
        });
        let v = serde_json::to_value(&r.result).unwrap();
        if v["status"] == "Ok" {
            Ok(v["value"].clone())
        } else {
            Err((
                v["error"]["code"].as_str().unwrap().to_string(),
                v["error"]["message"].as_str().unwrap().to_string(),
            ))
        }
    }

    fn ok(&mut self, domain: &str, command: Value) -> Value {
        let id = self.post(domain, command.clone());
        self.reply(id)
            .unwrap_or_else(|e| panic!("{domain} {command}: {e:?}"))
    }

    fn event(&mut self, what: &str, pred: impl Fn(&Event) -> bool) -> Event {
        self.wait(what, |c| {
            let pos = c
                .log
                .iter()
                .position(|m| matches!(m, ServerMessage::Event(e) if pred(e)))?;
            let ServerMessage::Event(e) = c.log.remove(pos) else {
                unreachable!()
            };
            Some(e)
        })
    }

    fn close(mut self) {
        let _ = self.ws.close(None);
        let deadline = Instant::now() + TIMEOUT;
        while Instant::now() < deadline {
            if self.ws.read().is_err() {
                break;
            }
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn count(v: &Value) -> usize {
    v.as_object().map_or(0, |o| o.len())
}

/// 16-bit mono PCM WAV.
fn wav(sample_rate: u32, samples: &[f32]) -> Vec<u8> {
    let data_len = samples.len() * 2;
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&sample_rate.to_le_bytes());
    b.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(data_len as u32).to_le_bytes());
    for s in samples {
        b.extend_from_slice(&((s * 32767.0) as i16).to_le_bytes());
    }
    b
}

fn uploads_dir_is_empty(root: &Path) -> bool {
    std::fs::read_dir(root.join("projects/.uploads")).map_or(true, |mut d| d.next().is_none())
}

#[test]
fn hello_rejections() {
    let tmp = TempDir::new("server-hello");
    let server = start(tmp.path());
    let addr = server.local_addr();

    let mut ws = open(addr);
    let (answer, code) = hello(&mut ws, PROTOCOL_VERSION, Some("wrong"));
    assert!(matches!(
        answer,
        ServerHello::Rejected {
            reason: HelloRejection::BadToken,
            ..
        }
    ));
    assert_eq!(code, Some(4001));

    let mut ws = open(addr);
    let (answer, _) = hello(&mut ws, PROTOCOL_VERSION, None);
    assert!(matches!(
        answer,
        ServerHello::Rejected {
            reason: HelloRejection::BadToken,
            ..
        }
    ));

    let mut ws = open(addr);
    let (answer, code) = hello(&mut ws, PROTOCOL_VERSION + 1, Some(TOKEN));
    assert!(matches!(
        answer,
        ServerHello::Rejected {
            reason: HelloRejection::UnsupportedVersion,
            ..
        }
    ));
    assert_eq!(code, Some(4002));
    assert_eq!(server.client_count(), 0);

    // No token is only allowed on loopback.
    let host_dir = TempDir::new("server-insecure");
    let host = NativeHost::start(
        HostConfig {
            audio: Some(AudioSettings {
                backend: AudioBackendKind::Null,
                ..Default::default()
            }),
            data_dir: host_dir.path().into(),
            instance: "test".into(),
            projects_root: host_dir.path().join("projects"),
            library_roots: vec![],
        },
        HostOptions {
            main_thread: Arc::new(DedicatedThread::new()),
            ..HostOptions::default()
        },
    )
    .unwrap();
    let insecure = Server::start(
        ServerConfig {
            bind: "0.0.0.0:0".parse().unwrap(),
            token: None,
            ..ServerConfig::default()
        },
        host,
    );
    assert!(matches!(
        insecure,
        Err(ether_server::ServerError::InsecureBind(_))
    ));
}

#[test]
fn full_remote_session() {
    let tmp = TempDir::new("server-session");
    std::fs::create_dir_all(tmp.path().join("library")).unwrap();
    let server = start(tmp.path());
    let addr = server.local_addr();

    // --- Client A: hello, then what WsTransport.connect does on an empty store.
    let (mut a, info) = Client::connect(addr, 1);
    assert_eq!(info.name, "test-server");
    assert_eq!(info.instance, "test");
    assert!(info.auth_required);
    assert!(info.capabilities.upload && !info.capabilities.collab);
    let listed = a.ok("Project", json!({"type": "List"}));
    assert_eq!(listed["projects"].as_array().unwrap().len(), 0);
    let pid = a.ids.next_project_id(now_ms()).to_string();
    a.ok(
        "Project",
        json!({"type": "Create", "id": pid, "name": "Remote"}),
    );

    // --- MIDI track + synth + clip + notes.
    let midi = a.id();
    let synth = a.id();
    a.ok(
        "Edit",
        json!({"type": "Batch", "label": "Add MIDI track", "commands": [
            {"domain": "Track", "command": {"type": "Create", "id": midi, "kind": "Midi",
                "name": "Synth", "color": null, "parent": null, "before": null}},
            {"domain": "Device", "command": {"type": "Insert", "id": synth, "track": midi,
                "device": {"type": "Builtin", "device": {"type": "Synth"}}, "before": null}},
        ]}),
    );
    let clip = a.id();
    a.ok(
        "Clip",
        json!({"type": "CreateMidi", "id": clip, "track": midi, "start": 0.0, "length": 4.0,
            "name": null}),
    );
    let (n1, n2) = (a.id(), a.id());
    a.ok(
        "Note",
        json!({"type": "Add", "clip": clip, "notes": [
            {"id": n1, "pitch": 60, "velocity": 100, "start": 0.0, "duration": 1.0},
            {"id": n2, "pitch": 67, "velocity": 100, "start": 1.0, "duration": 2.0},
        ]}),
    );

    // --- Play: playhead frames stream to the client; stop.
    a.ok("Transport", json!({"type": "Play"}));
    a.wait("playhead frames", |c| (c.playheads >= 3).then_some(()));
    a.ok("Transport", json!({"type": "Stop"}));

    // --- Client B joins the same controller: same document, own request ids.
    let (mut b, _) = Client::connect(addr, 2);
    assert_eq!(server.client_count(), 2);
    let p = b.ok("Project", json!({"type": "Get"}))["project"].clone();
    assert_eq!(p["id"], pid.as_str());
    assert_eq!(count(&p["notes"]), 2);
    // A's edit reaches B as a patch; A's reply (id collides with B's numbering) doesn't.
    a.ok(
        "Track",
        json!({"type": "Rename", "id": midi, "name": "Lead"}),
    );
    let e = b.event("B sees A's rename", |e| {
        matches!(e, Event::Patch { patch } if serde_json::to_string(patch).unwrap().contains("Lead"))
    });
    assert!(matches!(e, Event::Patch { .. }));
    assert!(
        b.log.iter().all(|m| !matches!(m, ServerMessage::Reply(_))),
        "only B's own replies reach B"
    );

    // --- B uploads a WAV over binary frames, with a gap and a resume.
    let tone: Vec<f32> = (0..48_000)
        .map(|i| 0.5 * (2.0 * std::f32::consts::PI * 220.0 * i as f32 / 48_000.0).sin())
        .collect();
    let bytes = wav(48_000, &tone);
    let upload = b.id();
    b.ok(
        "Media",
        json!({"type": "BeginUpload", "upload": upload, "name": "tone.wav",
            "size": bytes.len()}),
    );
    let half = bytes.len() / 2;
    let first = b.post_chunk(&upload, 0, &bytes[..half]);
    b.reply(first).unwrap();
    let gap = b.post_chunk(&upload, half + 10, &bytes[half + 10..]);
    let (code, message) = b.reply(gap).unwrap_err();
    assert_eq!(code, "InvalidState");
    assert!(
        message.contains(&format!("expected offset {half}")),
        "{message}"
    );
    let rest = b.post_chunk(&upload, half, &bytes[half..]);
    b.reply(rest).unwrap();
    b.event("upload complete", |e| {
        matches!(e, Event::Media { event: ether_protocol::media::MediaEvent::UploadProgress { received, .. } } if *received as usize == bytes.len())
    });
    // JSON (base64) chunks are accepted too: a second, tiny upload.
    let small = b.id();
    b.ok(
        "Media",
        json!({"type": "BeginUpload", "upload": small, "name": "x.bin", "size": 3}),
    );
    b.ok(
        "Media",
        json!({"type": "UploadChunk", "upload": small, "offset": 0, "data": "AQID"}),
    );
    b.ok("Media", json!({"type": "CancelUpload", "upload": small}));

    let audio = b.id();
    b.ok(
        "Track",
        json!({"type": "Create", "id": audio, "kind": "Audio", "name": "Audio",
            "color": null, "parent": null, "before": null}),
    );
    let media = b.id();
    let imported = b.ok(
        "Media",
        json!({"type": "Import", "id": media, "source": {"type": "Upload", "upload": upload}}),
    );
    assert_eq!(imported["media"]["name"], "tone.wav");
    assert_eq!(imported["media"]["frames"], 48_000);
    let aclip = b.id();
    b.ok(
        "Clip",
        json!({"type": "CreateAudio", "id": aclip, "track": audio, "start": 4.0,
            "media": media}),
    );
    b.event("peaks ready", |e| {
        matches!(e, Event::Media { event: ether_protocol::media::MediaEvent::PeaksReady { media: m } } if m.to_string() == media)
    });
    let binary_before = b.binary_frames;
    let peaks = b.ok(
        "Media",
        json!({"type": "GetPeaks", "request": {"media": media, "samples_per_peak": 64,
            "start_frame": 0, "frame_count": 48_000}}),
    );
    assert_eq!(
        b.binary_frames,
        binary_before + 1,
        "peaks travel as a binary frame"
    );
    let max = peaks["peaks"]["max"][0].as_array().unwrap();
    let top = max.iter().map(|v| v.as_f64().unwrap()).fold(0.0, f64::max);
    assert!((top - 0.5).abs() < 0.05, "peak {top}");
    assert!(
        uploads_dir_is_empty(tmp.path()),
        "imported uploads are dropped"
    );

    // --- An unfinished upload is cancelled when its client disconnects.
    let dangling = b.id();
    b.ok(
        "Media",
        json!({"type": "BeginUpload", "upload": dangling, "name": "d.wav", "size": 100}),
    );
    let c = b.post_chunk(&dangling, 0, &[0; 10]);
    b.reply(c).unwrap();
    assert!(!uploads_dir_is_empty(tmp.path()));
    b.close();
    // A round trip on A after the disconnect: the cancel was queued before it.
    let deadline = Instant::now() + TIMEOUT;
    while !uploads_dir_is_empty(tmp.path()) {
        assert!(Instant::now() < deadline, "staging not cleaned up");
        a.ok("Project", json!({"type": "Get"}));
    }
    assert_eq!(server.client_count(), 1);

    // --- Save, stop the server, restart it on the same folders and reopen.
    let saved_doc = a.ok("Project", json!({"type": "Get"}))["project"].clone();
    a.ok("Project", json!({"type": "Save"}));
    a.close();
    drop(server);

    let server = start(tmp.path());
    let (mut c, _) = Client::connect(server.local_addr(), 3);
    let listed = c.ok("Project", json!({"type": "List"}));
    assert_eq!(listed["projects"][0]["id"], pid.as_str());
    let reopened = c.ok("Project", json!({"type": "Open", "id": pid}))["project"].clone();
    for table in ["tracks", "clips", "notes", "devices", "media"] {
        assert_eq!(
            reopened[table], saved_doc[table],
            "{table} survives a restart"
        );
    }
    assert_eq!(reopened["tracks"][&midi]["name"], "Lead");
    c.close();
}

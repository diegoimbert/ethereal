//! A tiny client for driving a real [`NativeHost`] (null audio backend + the real
//! controller) with the same JSON the UI sends. Waits on a condition variable for replies
//! and events: no sleeps.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use ether_core::protocol::message::{Event, PlayheadFrame, Reply, ReplyResult};
use ether_core::protocol::meters::MeterFrame;
use ether_core::protocol::model::IdGen;
use ether_core::protocol::{ClientMessage, ServerMessage};
use ether_native::audio::{AudioBackendKind, AudioSettings};
use ether_native::host::{HostConfig, HostOptions, NativeHost};
use ether_native::{DedicatedThread, LibraryRoot};
use serde_json::{Value, json};

pub const TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Default)]
pub struct Received {
    pub replies: Vec<Reply>,
    pub events: Vec<Event>,
    pub playhead: Vec<(Instant, PlayheadFrame)>,
    pub meters: Vec<(Instant, MeterFrame)>,
    /// Everything but high-rate frames, in delivery order (ordering checks).
    pub log: Vec<ServerMessage>,
}

pub struct Client {
    pub host: Option<NativeHost>,
    pub rx: Arc<(Mutex<Received>, Condvar)>,
    next_id: u32,
    ids: IdGen,
}

/// Paths of one host "installation" (reused across restarts).
pub struct Paths {
    pub data_dir: PathBuf,
    pub projects_root: PathBuf,
    pub library: Option<PathBuf>,
}

impl Paths {
    pub fn new(root: &Path) -> Self {
        Self {
            data_dir: root.to_path_buf(),
            projects_root: root.join("projects"),
            library: None,
        }
    }
}

impl Client {
    pub fn start(paths: &Paths) -> Self {
        let host = NativeHost::start(
            HostConfig {
                audio: Some(AudioSettings {
                    backend: AudioBackendKind::Null,
                    max_block_size: 256,
                    ..Default::default()
                }),
                data_dir: paths.data_dir.clone(),
                instance: "test".into(),
                projects_root: paths.projects_root.clone(),
                library_roots: paths
                    .library
                    .iter()
                    .map(|p| LibraryRoot {
                        id: "lib".into(),
                        name: "Library".into(),
                        path: p.clone(),
                    })
                    .collect(),
            },
            HostOptions {
                main_thread: Arc::new(DedicatedThread::new()),
                ..HostOptions::default()
            },
        )
        .expect("host starts");
        let rx: Arc<(Mutex<Received>, Condvar)> = Arc::default();
        let sink = rx.clone();
        host.subscribe(Arc::new(move |m| {
            let (lock, cv) = &*sink;
            let mut r = lock.lock().unwrap();
            let now = Instant::now();
            match &m {
                ServerMessage::Reply(reply) => {
                    r.replies.push(reply.clone());
                    r.log.push(m);
                }
                ServerMessage::Event(e) => {
                    r.events.push(e.clone());
                    r.log.push(m);
                }
                ServerMessage::Playhead(p) => r.playhead.push((now, p.clone())),
                ServerMessage::Meters(f) => r.meters.push((now, f.clone())),
            }
            cv.notify_all();
        }));
        Self {
            host: Some(host),
            rx,
            next_id: 1,
            ids: IdGen::new(0xa1fa),
        }
    }

    pub fn host(&self) -> &NativeHost {
        self.host.as_ref().expect("running")
    }

    /// A fresh ULID string (entity ids are chosen by the client).
    pub fn id(&mut self) -> String {
        self.ids.next_ulid(now_ms()).to_string()
    }

    pub fn project_id(&mut self) -> String {
        self.ids.next_project_id(now_ms()).to_string()
    }

    /// Send without waiting (a gesture sends several commands back to back). Returns the
    /// message id.
    pub fn post(&mut self, domain: &str, command: Value, gesture: Option<u32>) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        let msg: ClientMessage = serde_json::from_value(json!({
            "id": id,
            "gesture": gesture,
            "command": { "domain": domain, "command": command },
        }))
        .unwrap_or_else(|e| panic!("bad {domain} command {command}: {e}"));
        self.host().send(msg).expect("send");
        id
    }

    pub fn reply(&self, id: u32) -> Result<Value, (String, String)> {
        let r = self.wait(|r| r.replies.iter().find(|x| x.id == id).cloned(), "reply");
        match r.result {
            ReplyResult::Ok { value } => Ok(serde_json::to_value(value).unwrap()),
            ReplyResult::Err { error } => Err((
                serde_json::to_value(error.code)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string(),
                error.message,
            )),
        }
    }

    /// Send and wait for a successful reply value.
    pub fn ok(&mut self, domain: &str, command: Value) -> Value {
        let id = self.post(domain, command.clone(), None);
        self.reply(id)
            .unwrap_or_else(|e| panic!("{domain} {command} failed: {e:?}"))
    }

    pub fn project(&mut self) -> Value {
        self.ok("Project", json!({"type": "Get"}))["project"].clone()
    }

    /// Block until `f` returns `Some` (woken by every delivered message).
    pub fn wait<T>(&self, mut f: impl FnMut(&Received) -> Option<T>, what: &str) -> T {
        let deadline = Instant::now() + TIMEOUT;
        let (lock, cv) = &*self.rx;
        let mut r = lock.lock().unwrap();
        loop {
            if let Some(v) = f(&r) {
                return v;
            }
            let now = Instant::now();
            assert!(now < deadline, "timed out waiting for {what}");
            r = cv.wait_timeout(r, deadline - now).unwrap().0;
        }
    }

    pub fn with<T>(&self, f: impl FnOnce(&mut Received) -> T) -> T {
        f(&mut self.rx.0.lock().unwrap())
    }

    /// Quit the host (saving unsaved work, like the desktop app on exit).
    pub fn quit(mut self) {
        if let Some(h) = self.host.take() {
            h.shutdown();
        }
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// 16-bit PCM WAV bytes.
pub fn wav(sample_rate: u32, channels: &[Vec<f32>]) -> Vec<u8> {
    let n_ch = channels.len() as u16;
    let frames = channels[0].len();
    let data_len = frames * n_ch as usize * 2;
    let mut b = Vec::with_capacity(44 + data_len);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&n_ch.to_le_bytes());
    b.extend_from_slice(&sample_rate.to_le_bytes());
    b.extend_from_slice(&(sample_rate * n_ch as u32 * 2).to_le_bytes());
    b.extend_from_slice(&(n_ch * 2).to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(data_len as u32).to_le_bytes());
    for i in 0..frames {
        for ch in channels {
            let s = (ch[i].clamp(-1.0, 1.0) * 32767.0).round() as i16;
            b.extend_from_slice(&s.to_le_bytes());
        }
    }
    b
}

pub fn sine(sample_rate: u32, hz: f32, frames: usize, amp: f32) -> Vec<f32> {
    (0..frames)
        .map(|i| amp * (2.0 * std::f32::consts::PI * hz * i as f32 / sample_rate as f32).sin())
        .collect()
}

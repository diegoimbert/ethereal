//! Test support: an MCP client over a spawned `ether-mcp`, and an in-process engine.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ether_native::audio::{AudioBackendKind, AudioSettings};
use ether_native::{DedicatedThread, HostConfig, HostOptions, NativeHost};
use ether_protocol::{ClientMessage, ReplyResult, ReplyValue, ServerMessage};
use ether_server::agent_bridge::AgentBridge;
use serde_json::{Value, json};

pub const TIMEOUT: Duration = Duration::from_secs(60);

/// A spawned `ether-mcp` speaking JSON-RPC on its stdio.
pub struct McpClient {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    next: u64,
}

impl McpClient {
    pub fn spawn(args: &[&str]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ether-mcp"))
            .args(args)
            .env("ETHER_AUDIO", "null")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn ether-mcp");
        let stdout: ChildStdout = child.stdout.take().unwrap();
        let (tx, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            stdin: child.stdin.take(),
            child,
            lines,
            next: 1,
        }
    }

    fn write(&mut self, v: &Value) {
        let stdin = self.stdin.as_mut().expect("open stdin");
        writeln!(stdin, "{v}").unwrap();
        stdin.flush().unwrap();
    }

    /// Send a request and wait for its response (`result` or `error` object).
    pub fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        self.write(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let line = match self.lines.recv_timeout(left) {
                Ok(l) => l,
                Err(RecvTimeoutError::Timeout) => panic!("no answer to {method}"),
                Err(RecvTimeoutError::Disconnected) => panic!("ether-mcp exited during {method}"),
            };
            let v: Value = serde_json::from_str(&line)
                .unwrap_or_else(|e| panic!("stdout must be JSON-RPC only: {line:?} ({e})"));
            if v["id"] == json!(id) {
                return v;
            }
        }
    }

    pub fn notify(&mut self, method: &str) {
        self.write(&json!({ "jsonrpc": "2.0", "method": method }));
    }

    /// `initialize` + `notifications/initialized`; returns the server's result.
    pub fn initialize(&mut self) -> Value {
        let r = self.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "ether-mcp-test", "version": "1" }
            }),
        );
        self.notify("notifications/initialized");
        r["result"].clone()
    }

    pub fn tool_names(&mut self) -> Vec<String> {
        let r = self.request("tools/list", json!({}));
        r["result"]["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect()
    }

    /// `tools/call`: `(text content, isError)`.
    pub fn call(&mut self, name: &str, arguments: Value) -> (String, bool) {
        let r = self.request("tools/call", json!({ "name": name, "arguments": arguments }));
        let res = &r["result"];
        assert!(res.is_object(), "tools/call {name}: {r}");
        let text = res["content"][0]["text"].as_str().unwrap_or("").to_string();
        (text, res["isError"].as_bool().unwrap_or(false))
    }

    /// `tools/call` expecting success; the content parsed as JSON.
    pub fn ok(&mut self, name: &str, arguments: Value) -> Value {
        let (text, is_error) = self.call(name, arguments);
        assert!(!is_error, "{name} failed: {text}");
        serde_json::from_str(&text).unwrap_or(Value::String(text))
    }

    /// Close stdin and wait for a clean exit.
    pub fn finish(mut self) {
        drop(self.stdin.take());
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "ether-mcp exit: {status}");
                return;
            }
            assert!(Instant::now() < deadline, "ether-mcp did not exit");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The scenario every mode runs: a MIDI track with a one-bar drum clip. Returns
/// `(track_id, clip_id)`.
pub fn drum_scenario(c: &mut McpClient) -> (String, String) {
    let info = c.initialize();
    assert_eq!(info["serverInfo"]["name"], "ethereal");
    assert!(info["capabilities"]["tools"].is_object());
    assert!(info["capabilities"]["resources"].is_object());
    let names = c.tool_names();
    for t in ["create_track", "create_midi_clip", "add_notes", "get_project_overview"] {
        assert!(names.iter().any(|n| n == t), "tools/list has {t}: {names:?}");
    }
    let track = c.ok("create_track", json!({ "kind": "midi", "name": "Drums", "instrument": "drum-rack" }));
    let track_id = track["track_id"].as_str().unwrap().to_string();
    let clip = c.ok(
        "create_midi_clip",
        json!({ "track_id": track_id, "start_beats": 0, "length_beats": 4, "name": "Beat" }),
    );
    let clip_id = clip["clip_id"].as_str().unwrap().to_string();
    let notes = c.ok(
        "add_notes",
        json!({ "clip_id": clip_id, "notes": [
            { "pitch": 36, "start_beats": 0, "duration_beats": 0.5, "velocity": 120 },
            { "pitch": 38, "start_beats": 1, "duration_beats": 0.5, "velocity": 110 },
            { "pitch": 36, "start_beats": 2, "duration_beats": 0.5, "velocity": 120 },
            { "pitch": 38, "start_beats": 3, "duration_beats": 0.5, "velocity": 110 }
        ] }),
    );
    assert_eq!(notes["note_ids"].as_array().unwrap().len(), 4);
    // A bad call is a tool error the model sees, not a protocol error.
    let (text, is_error) = c.call("add_notes", json!({ "clip_id": clip_id, "notes": [{ "pitch": 300, "start_beats": 0, "duration_beats": 1 }] }));
    assert!(is_error && text.contains("pitch"), "{text}");
    let (text, is_error) = c.call("no_such_tool", json!({}));
    assert!(is_error && text.contains("unknown tool"), "{text}");
    // The overview resource.
    let r = c.request("resources/list", json!({}));
    assert_eq!(r["result"]["resources"][0]["uri"], "ethereal://project/overview");
    let r = c.request("resources/read", json!({ "uri": "ethereal://project/overview" }));
    let overview: Value =
        serde_json::from_str(r["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    let tracks = overview["tracks"].as_array().unwrap();
    let drums = tracks.iter().find(|t| t["id"] == track_id.as_str()).expect("track in overview");
    assert_eq!(drums["name"], "Drums");
    assert_eq!(drums["clips"][0]["notes"], 4);
    (track_id, clip_id)
}

/// Check a saved `.ether` document has the drum scenario.
pub fn assert_drum_document(doc: &Value, track_id: &str, clip_id: &str) {
    let p = &doc["project"];
    assert_eq!(p["tracks"][track_id]["name"], "Drums", "track saved");
    assert_eq!(p["clips"][clip_id]["name"], "Beat", "clip saved");
    let notes: Vec<&Value> = p["notes"]
        .as_object()
        .unwrap()
        .values()
        .filter(|n| n["clip"] == clip_id)
        .collect();
    assert_eq!(notes.len(), 4, "notes saved");
    assert!(notes.iter().any(|n| n["pitch"] == 38));
}

pub fn start_host(root: &Path) -> NativeHost {
    NativeHost::start(
        HostConfig {
            audio: Some(AudioSettings {
                backend: AudioBackendKind::Null,
                max_block_size: 256,
                ..Default::default()
            }),
            data_dir: root.join("data"),
            instance: "test".into(),
            projects_root: root.join("projects"),
            library_roots: Vec::new(),
        },
        HostOptions {
            main_thread: Arc::new(DedicatedThread::new()),
            ..HostOptions::default()
        },
    )
    .expect("host starts")
}

/// The desktop app's wiring, in process: a host shared by a "UI" (this test) and the agent
/// bridge.
pub struct Desktop {
    pub host: Arc<NativeHost>,
    /// The bridge slot (the desktop app keeps the same: `Mutex<Option<AgentBridge>>`).
    pub bridge: Arc<Mutex<Option<AgentBridge>>>,
    /// What the UI receives (replies to its requests, every event).
    pub ui: Mutex<Receiver<ServerMessage>>,
    next: Mutex<u32>,
}

impl Desktop {
    pub fn start(root: &Path) -> Self {
        let host = Arc::new(start_host(root));
        let bridge: Arc<Mutex<Option<AgentBridge>>> = Arc::default();
        let (tx, rx) = std::sync::mpsc::channel();
        {
            let bridge = bridge.clone();
            let tx = Mutex::new(tx);
            host.subscribe(Arc::new(move |m| {
                let m = match bridge.lock().unwrap().as_ref() {
                    Some(b) => b.route(m),
                    None => Some(m),
                };
                if let Some(m) = m {
                    let _ = tx.lock().unwrap().send(m);
                }
            }));
        }
        Self {
            host,
            bridge,
            ui: Mutex::new(rx),
            next: Mutex::new(1),
        }
    }

    /// Enable the bridge; returns its port.
    pub fn enable_bridge(&self, runtime_file: PathBuf) -> u16 {
        let b = AgentBridge::start(
            Box::new(self.host.clone()),
            runtime_file,
            "test".into(),
            "0.0.1",
        )
        .expect("bridge starts");
        let port = b.port();
        *self.bridge.lock().unwrap() = Some(b);
        port
    }

    /// Disable the bridge (stopped outside the routing lock).
    pub fn disable_bridge(&self) {
        let b = self.bridge.lock().unwrap().take();
        if let Some(b) = b {
            b.stop();
        }
    }

    pub fn bridge_clients(&self) -> usize {
        self.bridge.lock().unwrap().as_ref().map_or(0, |b| b.client_count())
    }

    /// A UI request; returns the reply and the events received before it.
    pub fn ui_request(&self, command: ether_protocol::Command) -> (ReplyValue, Vec<ServerMessage>) {
        let id = {
            let mut n = self.next.lock().unwrap();
            *n += 1;
            *n
        };
        self.host
            .send(ClientMessage {
                id,
                gesture: None,
                command,
            })
            .unwrap();
        let rx = self.ui.lock().unwrap();
        let mut events = Vec::new();
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let m = rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("UI reply");
            match m {
                ServerMessage::Reply(r) => {
                    assert!(r.id < ether_server::agent_bridge::REQUEST_BASE, "the UI never sees bridge replies: {}", r.id);
                    if r.id == id {
                        match r.result {
                            ReplyResult::Ok { value } => return (value, events),
                            ReplyResult::Err { error } => panic!("UI request failed: {error:?}"),
                        }
                    }
                }
                other => events.push(other),
            }
        }
    }

    /// Everything the UI received so far (without waiting).
    pub fn ui_drain(&self) -> Vec<ServerMessage> {
        self.ui.lock().unwrap().try_iter().collect()
    }
}

//! `ether-mcp` end to end: the real binary over stdio (JSON-RPC: initialize, tools/list,
//! tools/call, resources), in each mode:
//! - `--project` (headless, embedded engine): the saved file has the edits;
//! - desktop (`--runtime-file`): the agent bridge attached to an in-process "desktop"
//!   engine that a UI also drives; the UI sees the agent's patches, never its replies;
//! - `--server`: an in-process `ether-server`.

mod support;

use ether_native::test_util::TempDir;
use ether_protocol::project::ProjectCommand;
use ether_protocol::{Command, Event, ReplyValue, ServerMessage};
use ether_server::{Server, ServerConfig};
use serde_json::{Value, json};
use support::*;

#[test]
fn headless_project_file() {
    let tmp = TempDir::new("mcp-headless");
    let file = tmp.path().join("songs").join("beat.ether");
    let mut c = McpClient::spawn(&["--project", file.to_str().unwrap()]);
    let (track, clip) = drum_scenario(&mut c);
    assert!(file.is_file(), "a new project file is written at start");
    let saved = c.ok("save_project", json!({}));
    assert!(
        saved["file"].as_str().unwrap().contains("beat.ether"),
        "{saved}"
    );
    c.finish();
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(doc["project"]["settings"]["name"], "beat");
    assert_drum_document(&doc, &track, &clip);

    // Reopen the file: the edits are there, and undo history starts fresh.
    let mut c = McpClient::spawn(&["--project", file.to_str().unwrap()]);
    c.initialize();
    let notes = c.ok("get_clip_notes", json!({ "clip_id": clip }));
    assert_eq!(notes["total"], 4);
    c.finish();
}

#[test]
fn desktop_bridge() {
    let tmp = TempDir::new("mcp-bridge");
    let desktop = Desktop::start(tmp.path());
    let id = ether_protocol::model::IdGen::new(7).next_project_id(1_700_000_000_000);
    desktop.ui_request(Command::Project(ProjectCommand::Create {
        id,
        name: "Live".into(),
    }));
    let runtime = tmp.path().join("agent-bridge.json");
    desktop.enable_bridge(runtime.clone());
    desktop.ui_drain();

    let mut c = McpClient::spawn(&["--runtime-file", runtime.to_str().unwrap()]);
    let (track, clip) = drum_scenario(&mut c);
    assert_eq!(desktop.bridge_clients(), 1);
    // The UI saw the agent's edits as patches (and no bridge reply: `ui_request` checks).
    let seen = desktop.ui_drain();
    let patches = seen
        .iter()
        .filter(|m| matches!(m, ServerMessage::Event(Event::Patch { .. })))
        .count();
    assert!(patches >= 3, "UI got the agent's patches ({patches})");
    // The UI's document is the agent's.
    let (ReplyValue::Project { project }, _) =
        desktop.ui_request(Command::Project(ProjectCommand::Get))
    else {
        panic!("project");
    };
    let doc = json!({ "project": serde_json::to_value(&project).unwrap() });
    assert_drum_document(&doc, &track, &clip);
    // The UI's undo undoes the agent's last step (one history).
    desktop.ui_request(Command::Edit(ether_protocol::project::EditCommand::Undo));
    let notes = c.ok("get_clip_notes", json!({ "clip_id": clip }));
    assert_eq!(notes["total"], 0, "one undo removed the add_notes step");
    c.finish();
    desktop.disable_bridge();
}

#[test]
fn desktop_bridge_off_is_a_clear_tool_error() {
    let tmp = TempDir::new("mcp-bridge-off");
    let runtime = tmp.path().join("agent-bridge.json");
    let mut c = McpClient::spawn(&["--runtime-file", runtime.to_str().unwrap()]);
    c.initialize();
    // Tools are listed from the built-in registry, calls explain what to do.
    assert!(c.tool_names().iter().any(|n| n == "create_track"));
    let (text, is_error) = c.call("get_project_overview", json!({}));
    assert!(is_error, "{text}");
    assert!(text.contains("Allow AI agents (MCP)"), "{text}");
    c.finish();
}

#[test]
fn ether_server_mode() {
    let tmp = TempDir::new("mcp-server");
    let host = start_host(tmp.path());
    let server = Server::start(
        ServerConfig {
            token: Some("tok".into()),
            instance: "test".into(),
            ..ServerConfig::default()
        },
        host,
    )
    .unwrap();
    let url = format!("ws://{}/", server.local_addr());
    // A wrong token fails at start with a clear message.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_ether-mcp"))
        .args(["--server", &url, "--token", "nope"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("BadToken"), "{err}");

    let mut c = McpClient::spawn(&["--server", &url, "--token", "tok"]);
    c.initialize();
    // No project open on a fresh server: a tool error, not a crash.
    let (text, is_error) = c.call("get_project_overview", json!({}));
    assert!(is_error && text.contains("no project"), "{text}");
    c.finish();
    drop(server);
}

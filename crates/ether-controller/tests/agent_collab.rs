//! Agent API (`agent-api`) in a collab session: a tool edit replicates to the peers like any
//! UI edit, and the `undo` tool undoes it everywhere (one step).

#[path = "collab_support.rs"]
mod support;

use ether_collab::memory::Hub;
use ether_core::protocol::agent::AgentCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::*;
use serde_json::{Value, json};
use support::*;

fn tool(s: &mut Site, name: &str, input: Value) -> Value {
    let ReplyValue::AgentToolResult { content, is_error } =
        s.ok(Command::Agent(AgentCommand::CallTool {
            name: name.into(),
            input: input.to_string(),
        }))
    else {
        panic!("expected AgentToolResult");
    };
    assert!(!is_error, "{name}: {content}");
    serde_json::from_str(&content).unwrap_or(Value::Null)
}

#[test]
fn tool_edits_replicate_and_undo_everywhere() {
    let hub = Hub::default();
    let mut sites = session(&hub, 2);
    let (a, b) = sites.split_at_mut(1);
    let (a, b) = (&mut a[0], &mut b[0]);

    let track = tool(a, "create_track", json!({ "kind": "midi", "name": "Drums" }));
    let track_id: TrackId = track["track_id"].as_str().unwrap().parse().unwrap();
    let clip = tool(
        a,
        "create_midi_clip",
        json!({ "track_id": track["track_id"], "start_beats": 0, "length_beats": 4 }),
    );
    let notes = tool(
        a,
        "add_notes",
        json!({ "clip_id": clip["clip_id"], "notes": [
            { "pitch": 36, "start_beats": 0, "duration_beats": 0.5, "velocity": 120 },
            { "pitch": 38, "start_beats": 1, "duration_beats": 0.5, "velocity": 110 }
        ] }),
    );
    assert_eq!(notes["note_ids"].as_array().unwrap().len(), 2);
    settle(&mut [&mut *a, &mut *b], &hub);
    assert_converged(&[&*a, &*b]);
    assert_eq!(b.project().tracks[&track_id].name, "Drums");
    let clip_id: ClipId = clip["clip_id"].as_str().unwrap().parse().unwrap();
    assert_eq!(b.project().notes_of(clip_id).len(), 2, "notes replicated");

    // B (another agent or user) edits through a tool too.
    tool(b, "set_track_mix", json!({ "track_id": track["track_id"], "volume_db": -3 }));
    settle(&mut [&mut *a, &mut *b], &hub);
    assert_converged(&[&*a, &*b]);
    assert_eq!(a.project().tracks[&track_id].mixer.volume.0, -3.0);

    // A's `undo` tool undoes A's last step (the notes) on every replica.
    let v = tool(a, "undo", json!({}));
    assert_eq!(v["undone"], "AI: Add Notes");
    settle(&mut [&mut *a, &mut *b], &hub);
    assert_converged(&[&*a, &*b]);
    assert!(b.project().notes_of(clip_id).is_empty());
}

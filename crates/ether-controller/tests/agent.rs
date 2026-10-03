//! Agent API (`agent-api`): every tool of the registry through `Command::Agent` — happy
//! path, bad input (`is_error` results, never `CommandError`s), one undo step per call.

mod common;

use common::*;
use ether_core::protocol::agent::AgentCommand;
use ether_core::protocol::collab::{CollabCommand, PresenceState};
use ether_core::protocol::model::*;
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::*;
use serde_json::{Value, json};

struct Res {
    value: Value,
    text: String,
    is_error: bool,
    out: Vec<ServerMessage>,
}

fn call(h: &mut Harness, name: &str, input: Value) -> Res {
    call_raw(h, name, &input.to_string())
}

fn call_raw(h: &mut Harness, name: &str, input: &str) -> Res {
    let out = h.send(Command::Agent(AgentCommand::CallTool {
        name: name.into(),
        input: input.into(),
    }));
    let ReplyValue::AgentToolResult { content, is_error } = ok(&out) else {
        panic!("expected AgentToolResult");
    };
    Res {
        value: serde_json::from_str(&content).unwrap_or(Value::Null),
        text: content,
        is_error,
        out,
    }
}

/// Call and expect success.
fn ok_call(h: &mut Harness, name: &str, input: Value) -> Value {
    let r = call(h, name, input);
    assert!(!r.is_error, "{name} failed: {}", r.text);
    r.value
}

/// Call and expect an `is_error` result that changed nothing.
fn err_call(h: &mut Harness, name: &str, input: Value) -> String {
    let before = h.project().clone();
    let r = call(h, name, input);
    assert!(r.is_error, "{name} should fail, got {}", r.text);
    assert!(
        patches(&r.out).is_empty(),
        "{name}: a failed call edits nothing"
    );
    assert_eq!(h.project(), &before);
    r.text
}

/// Call an edit tool: exactly one undo step, undone by one `Undo`, redone by one `Redo`.
fn one_step(h: &mut Harness, name: &str, input: Value) -> Value {
    let before = h.project().clone();
    let r = call(h, name, input);
    assert!(!r.is_error, "{name} failed: {}", r.text);
    let ps = patches(&r.out);
    assert!(!ps.is_empty(), "{name}: patches arrive before the reply");
    assert!(
        ps.last()
            .unwrap()
            .history
            .undo_label
            .as_deref()
            .unwrap_or("")
            .starts_with("AI: "),
        "{name}: labelled undo step: {:?}",
        ps.last().unwrap().history
    );
    let after = h.project().clone();
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(
        h.project(),
        &before,
        "{name}: one Undo restores the project"
    );
    h.ok(Command::Edit(EditCommand::Redo));
    assert_eq!(h.project(), &after, "{name}: Redo re-applies it");
    r.value
}

fn s(v: &Value) -> String {
    v.as_str().expect("string").to_string()
}

fn midi_track(h: &mut Harness) -> String {
    s(&ok_call(h, "create_track", json!({ "kind": "midi" }))["track_id"])
}

fn midi_clip(h: &mut Harness, track: &str) -> String {
    s(&ok_call(
        h,
        "create_midi_clip",
        json!({ "track_id": track, "start_beats": 0, "length_beats": 4 }),
    )["clip_id"])
}

#[test]
fn list_tools_is_the_registry() {
    let mut h = Harness::new();
    let ReplyValue::AgentTools { tools } = h.ok(Command::Agent(AgentCommand::ListTools)) else {
        panic!("expected AgentTools");
    };
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    for want in [
        "get_project_overview",
        "get_track",
        "get_clip_notes",
        "list_device_types",
        "get_device_params",
        "create_track",
        "delete_track",
        "rename_track",
        "set_track_mix",
        "add_device",
        "remove_device",
        "set_device_param",
        "create_midi_clip",
        "add_notes",
        "remove_notes",
        "set_clip",
        "delete_clip",
        "duplicate_clip",
        "set_tempo",
        "set_time_signature",
        "transport",
        "undo",
        "redo",
        "search_browser",
        "load_browser_item",
        "export_audio",
    ] {
        assert!(names.contains(&want), "missing tool {want}");
    }
    for t in &tools {
        let schema: Value = serde_json::from_str(&t.input_schema).expect("schema is JSON");
        assert_eq!(schema["type"], "object");
        assert!(!t.description.is_empty());
    }
    // Pinned shapes (agent-contract "Pinned tool shapes").
    let schema = |name: &str| -> Value {
        let t = tools.iter().find(|t| t.name == name).unwrap();
        serde_json::from_str(&t.input_schema).unwrap()
    };
    assert_eq!(
        schema("create_midi_clip")["required"],
        json!(["track_id", "start_beats", "length_beats"])
    );
    assert_eq!(schema("add_notes")["required"], json!(["clip_id", "notes"]));
    assert_eq!(
        schema("add_notes")["properties"]["notes"]["items"]["required"],
        json!(["pitch", "start_beats", "duration_beats"])
    );
    assert_eq!(schema("set_track_mix")["required"], json!(["track_id"]));
    assert_eq!(schema("get_clip_notes")["required"], json!(["clip_id"]));
}

#[test]
fn bad_calls_are_tool_errors() {
    let mut h = Harness::with_project();
    let e = err_call(&mut h, "no_such_tool", json!({}));
    assert!(
        e.contains("unknown tool") && e.contains("create_track"),
        "{e}"
    );
    let r = call_raw(&mut h, "create_track", "{not json");
    assert!(r.is_error && r.text.contains("JSON"));
    let e = err_call(&mut h, "create_track", json!({}));
    assert!(e.contains("missing required property `kind`"), "{e}");
    let e = err_call(
        &mut h,
        "create_track",
        json!({ "kind": "midi", "volume": 3 }),
    );
    assert!(e.contains("unknown property `volume`"), "{e}");
    let e = err_call(&mut h, "create_track", json!({ "kind": "banjo" }));
    assert!(e.contains("must be one of"), "{e}");
    let e = err_call(
        &mut h,
        "rename_track",
        json!({ "track_id": "nope", "name": "x" }),
    );
    assert!(e.contains("not a valid track id"), "{e}");
    let ghost: TrackId = h.id();
    let e = err_call(
        &mut h,
        "rename_track",
        json!({ "track_id": ghost.to_string(), "name": "x" }),
    );
    assert!(e.starts_with("Not found"), "{e}");
    // Arrays are typed item by item.
    let t = midi_track(&mut h);
    let c = midi_clip(&mut h, &t);
    let e = err_call(
        &mut h,
        "add_notes",
        json!({ "clip_id": c, "notes": [{ "pitch": 60, "start_beats": 0, "duration_beats": 1 }, { "pitch": 200, "start_beats": 0, "duration_beats": 1 }] }),
    );
    assert!(e.contains("input.notes[1].pitch"), "{e}");
}

#[test]
fn no_project_is_a_tool_error() {
    let mut h = Harness::new();
    let r = call(&mut h, "get_project_overview", json!({}));
    assert!(r.is_error && r.text.contains("no project"), "{}", r.text);
    let r = call(&mut h, "create_track", json!({ "kind": "midi" }));
    assert!(r.is_error);
}

#[test]
fn tracks() {
    let mut h = Harness::with_project();
    let v = one_step(
        &mut h,
        "create_track",
        json!({ "kind": "midi", "name": "Lead" }),
    );
    let t = s(&v["track_id"]);
    let id: TrackId = t.parse().unwrap();
    let track = &h.project().tracks[&id];
    assert_eq!(track.name, "Lead");
    assert_eq!(track.kind, TrackKind::Midi);
    let devs = h.project().devices_of(id);
    assert_eq!(devs.len(), 1, "a default instrument");
    assert_eq!(s(&v["instrument_device_id"]), devs[0].id.to_string());

    let v = one_step(
        &mut h,
        "create_track",
        json!({ "kind": "midi", "instrument": "none", "color": "#ff0000", "before_track_id": t }),
    );
    let t2: TrackId = s(&v["track_id"]).parse().unwrap();
    assert!(h.project().devices_of(t2).is_empty());
    assert_eq!(h.project().tracks[&t2].color, Color(0xff0000));
    let e = err_call(
        &mut h,
        "create_track",
        json!({ "kind": "audio", "instrument": "synth" }),
    );
    assert!(e.contains("only for MIDI"), "{e}");
    let e = err_call(
        &mut h,
        "create_track",
        json!({ "kind": "midi", "instrument": "reverb" }),
    );
    assert!(e.contains("not an instrument"), "{e}");
    one_step(&mut h, "create_track", json!({ "kind": "audio" }));
    one_step(&mut h, "create_track", json!({ "kind": "return" }));
    one_step(&mut h, "create_track", json!({ "kind": "group" }));

    one_step(
        &mut h,
        "rename_track",
        json!({ "track_id": t, "name": "Bass" }),
    );
    assert_eq!(h.project().tracks[&id].name, "Bass");
    let e = err_call(&mut h, "rename_track", json!({ "track_id": t, "name": "" }));
    assert!(e.contains("at least 1"), "{e}");

    let v = one_step(
        &mut h,
        "set_track_mix",
        json!({ "track_id": t, "volume_db": -6.5, "pan": 0.25, "mute": true }),
    );
    assert_eq!(v["volume_db"], -6.5);
    let m = h.project().tracks[&id].mixer;
    assert_eq!((m.volume.0, m.pan.0, m.mute), (-6.5, 0.25, true));
    one_step(
        &mut h,
        "set_track_mix",
        json!({ "track_id": t, "solo": true }),
    );
    assert!(h.project().tracks[&id].mixer.solo);
    let e = err_call(
        &mut h,
        "set_track_mix",
        json!({ "track_id": t, "volume_db": 12 }),
    );
    assert!(e.contains("<= 6"), "{e}");
    let e = err_call(&mut h, "set_track_mix", json!({ "track_id": t }));
    assert!(e.contains("nothing to change"), "{e}");
    let v = ok_call(
        &mut h,
        "set_track_mix",
        json!({ "track_id": t, "arm": true }),
    );
    assert_eq!(v["armed"], true);
    assert_eq!(h.ctl.armed(), vec![id]);

    let master = h.project().master_track().id.to_string();
    let e = err_call(&mut h, "delete_track", json!({ "track_id": master }));
    assert!(e.contains("master"), "{e}");
    one_step(&mut h, "delete_track", json!({ "track_id": t }));
    assert!(!h.project().tracks.contains_key(&id));
}

#[test]
fn devices_and_params() {
    let mut h = Harness::with_project();
    let types = ok_call(
        &mut h,
        "list_device_types",
        json!({ "category": "instrument" }),
    );
    let list = types["device_types"].as_array().unwrap();
    assert!(list.iter().any(|t| t["type"] == "poly-synth"));
    assert!(list.iter().all(|t| t["category"] == "instrument"));

    let t = s(&ok_call(
        &mut h,
        "create_track",
        json!({ "kind": "midi", "instrument": "synth" }),
    )["track_id"]);
    let tid: TrackId = t.parse().unwrap();
    let v = one_step(
        &mut h,
        "add_device",
        json!({ "track_id": t, "type": "Reverb" }),
    );
    assert_eq!(v["type"], "reverb");
    assert_eq!(v["index"], 1);
    let rev = s(&v["device_id"]);
    let v = one_step(
        &mut h,
        "add_device",
        json!({ "track_id": t, "type": "delay", "index": 1 }),
    );
    let chain: Vec<String> = h
        .project()
        .devices_of(tid)
        .iter()
        .map(|d| d.id.to_string())
        .collect();
    assert_eq!(chain[1], s(&v["device_id"]), "inserted at index 1");
    assert_eq!(chain[2], rev);
    let e = err_call(
        &mut h,
        "add_device",
        json!({ "track_id": t, "type": "kazoo" }),
    );
    assert!(
        e.contains("unknown device type") && e.contains("poly-synth"),
        "{e}"
    );

    let params = ok_call(&mut h, "get_device_params", json!({ "device_id": rev }));
    let list = params["params"].as_array().unwrap();
    assert!(!list.is_empty());
    let p0 = &list[0];
    let (pid, pname) = (p0["id"].as_u64().unwrap(), s(&p0["name"]));
    let (min, max) = (p0["min"].as_f64().unwrap(), p0["max"].as_f64().unwrap());
    let mid = (min + max) / 2.0;
    let v = one_step(
        &mut h,
        "set_device_param",
        json!({ "device_id": rev, "param": pname.to_uppercase(), "value": mid }),
    );
    assert_eq!(v["param"], pid);
    let did: DeviceId = rev.parse().unwrap();
    let stored = h.project().devices[&did].params[&ParamId(pid as u32)];
    assert!(
        (stored
            - p0["step"]
                .as_f64()
                .map_or(mid, |st| min + ((mid - min) / st).round() * st))
        .abs()
            < 1e-6
    );
    // Out of range: clamped, with a note.
    let v = one_step(
        &mut h,
        "set_device_param",
        json!({ "device_id": rev, "param": pid, "value": max + 1000.0 }),
    );
    assert_eq!(v["value"].as_f64().unwrap(), max);
    assert!(v["note"].as_str().unwrap().contains("clamped"));
    let e = err_call(
        &mut h,
        "set_device_param",
        json!({ "device_id": rev, "param": "Nope", "value": 1 }),
    );
    assert!(e.contains("no parameter"), "{e}");

    // A choice parameter by label (any built-in with labels).
    let synth = h.project().devices_of(tid)[0].id;
    let sp = ok_call(
        &mut h,
        "get_device_params",
        json!({ "device_id": synth.to_string() }),
    );
    if let Some(p) = sp["params"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["labels"].as_array().is_some_and(|l| l.len() > 1))
    {
        let label = s(&p["labels"][1]);
        let v = one_step(
            &mut h,
            "set_device_param",
            json!({ "device_id": synth.to_string(), "param": p["id"], "value": label.to_lowercase() }),
        );
        assert_eq!(s(&v["label"]), label);
        let e = err_call(
            &mut h,
            "set_device_param",
            json!({ "device_id": synth.to_string(), "param": p["id"], "value": "zzz" }),
        );
        assert!(e.contains("is not a choice"), "{e}");
    }

    one_step(&mut h, "remove_device", json!({ "device_id": rev }));
    assert!(!h.project().devices.contains_key(&did));
    let e = err_call(&mut h, "remove_device", json!({ "device_id": rev }));
    assert!(e.starts_with("Not found"), "{e}");
}

#[test]
fn clips_and_notes() {
    let mut h = Harness::with_project();
    let t = midi_track(&mut h);
    let v = one_step(
        &mut h,
        "create_midi_clip",
        json!({
            "track_id": t, "start_beats": 4, "length_beats": 4, "name": "Beat",
            "notes": [{ "pitch": 36, "start_beats": 0, "duration_beats": 0.5, "velocity": 127 }]
        }),
    );
    let c = s(&v["clip_id"]);
    let cid: ClipId = c.parse().unwrap();
    assert_eq!(v["note_ids"].as_array().unwrap().len(), 1);
    let clip = &h.project().clips[&cid];
    assert_eq!(
        (clip.start.0, clip.length.0, clip.name.as_str()),
        (4.0, 4.0, "Beat")
    );

    let v = one_step(
        &mut h,
        "add_notes",
        json!({ "clip_id": c, "notes": [
            { "pitch": 38, "start_beats": 1, "duration_beats": 0.5 },
            { "pitch": 42, "start_beats": 2, "duration_beats": 0.25, "velocity": 64 },
            { "pitch": 42, "start_beats": 6, "duration_beats": 0.25 }
        ] }),
    );
    let ids = v["note_ids"].as_array().unwrap();
    assert_eq!(ids.len(), 3);
    assert!(v["note"].as_str().unwrap().contains("after the clip end"));
    let nid: NoteId = s(&ids[1]).parse().unwrap();
    let n = &h.project().notes[&nid];
    assert_eq!((n.pitch, n.start.0, n.duration.0), (42, 2.0, 0.25));
    assert!((n.velocity - 64.0 / 127.0).abs() < 1e-6);

    let notes = ok_call(&mut h, "get_clip_notes", json!({ "clip_id": c }));
    assert_eq!(notes["total"], 4);
    assert_eq!(notes["notes"][0]["pitch"], 36);
    assert_eq!(notes["notes"][0]["velocity"], 127);
    assert_eq!(notes["notes"][0]["duration_beats"], 0.5);
    let page = ok_call(
        &mut h,
        "get_clip_notes",
        json!({ "clip_id": c, "offset": 1, "limit": 2 }),
    );
    assert_eq!(page["notes"].as_array().unwrap().len(), 2);
    assert!(page["note"].as_str().unwrap().contains("offset 3"));

    // Remove by filter, then by id.
    let v = one_step(
        &mut h,
        "remove_notes",
        json!({ "clip_id": c, "pitch": 42, "from_beats": 0, "to_beats": 4 }),
    );
    assert_eq!(v["removed"], 1);
    let v = one_step(&mut h, "remove_notes", json!({ "note_ids": [s(&ids[0])] }));
    assert_eq!(v["removed"], 1);
    assert_eq!(
        ok_call(&mut h, "remove_notes", json!({ "clip_id": c, "pitch": 1 }))["removed"],
        0
    );
    let e = err_call(&mut h, "remove_notes", json!({}));
    assert!(e.contains("note_ids"), "{e}");

    // set_clip: move, resize, rename, loop.
    let v = one_step(
        &mut h,
        "set_clip",
        json!({ "clip_id": c, "start_beats": 8, "length_beats": 8, "name": "Groove", "loop_enabled": true }),
    );
    assert_eq!(
        (v["start_beats"].as_f64(), v["length_beats"].as_f64()),
        (Some(8.0), Some(8.0))
    );
    let clip = &h.project().clips[&cid];
    assert_eq!(clip.name, "Groove");
    assert!(clip.looping.enabled && clip.looping.end.0 > clip.looping.start.0);
    one_step(&mut h, "set_clip", json!({ "clip_id": c, "muted": true }));
    let e = err_call(&mut h, "set_clip", json!({ "clip_id": c }));
    assert!(e.contains("nothing to change"), "{e}");
    let audio = s(&ok_call(&mut h, "create_track", json!({ "kind": "audio" }))["track_id"]);
    let e = err_call(
        &mut h,
        "set_clip",
        json!({ "clip_id": c, "track_id": audio }),
    );
    assert!(
        e.starts_with("Rejected") || e.starts_with("Not possible"),
        "{e}"
    );

    let v = one_step(&mut h, "duplicate_clip", json!({ "clip_id": c }));
    assert_eq!(v["start_beats"], 16.0);
    let dup = s(&v["clip_id"]);
    let v = one_step(
        &mut h,
        "duplicate_clip",
        json!({ "clip_id": c, "start_beats": 32 }),
    );
    assert_eq!(v["start_beats"], 32.0);
    assert_eq!(
        h.project().notes_of(dup.parse().unwrap()).len(),
        2,
        "notes copied"
    );

    let e = err_call(
        &mut h,
        "create_midi_clip",
        json!({ "track_id": audio, "start_beats": 0, "length_beats": 4 }),
    );
    assert!(e.contains("not a MIDI track"), "{e}");
    one_step(
        &mut h,
        "delete_clip",
        json!({ "clip_ids": [dup, s(&v["clip_id"])] }),
    );
    one_step(&mut h, "delete_clip", json!({ "clip_id": c }));
    assert!(h.project().clips.is_empty());
    let e = err_call(&mut h, "delete_clip", json!({}));
    assert!(e.contains("clip_id"), "{e}");
}

#[test]
fn song_transport_and_history() {
    let mut h = Harness::with_project();
    let v = one_step(&mut h, "set_tempo", json!({ "bpm": 128 }));
    assert_eq!(v["tempo_bpm"], 128.0);
    let e = err_call(&mut h, "set_tempo", json!({ "bpm": 5 }));
    assert!(e.contains(">= 20"), "{e}");
    let v = one_step(
        &mut h,
        "set_time_signature",
        json!({ "numerator": 3, "denominator": 4 }),
    );
    assert_eq!(v["time_signature"], "3/4");
    let e = err_call(
        &mut h,
        "set_time_signature",
        json!({ "numerator": 3, "denominator": 5 }),
    );
    assert!(e.contains("must be one of"), "{e}");

    let v = ok_call(
        &mut h,
        "transport",
        json!({ "action": "seek", "position_beats": 8 }),
    );
    assert_eq!(v["playhead_beats"], 8.0);
    let v = ok_call(&mut h, "transport", json!({ "action": "play" }));
    assert_eq!(v["playing"], true);
    let v = ok_call(&mut h, "transport", json!({ "action": "stop" }));
    assert_eq!(v["playing"], false);
    let v = one_step(
        &mut h,
        "transport",
        json!({ "action": "loop", "loop_enabled": true, "loop_start_beats": 4, "loop_end_beats": 12 }),
    );
    assert_eq!(
        v["loop"],
        json!({ "enabled": true, "start_beats": 4.0, "end_beats": 12.0 })
    );
    let e = err_call(&mut h, "transport", json!({ "action": "seek" }));
    assert!(e.contains("position_beats"), "{e}");
    let e = err_call(
        &mut h,
        "transport",
        json!({ "action": "loop", "loop_start_beats": 8, "loop_end_beats": 4 }),
    );
    assert!(e.contains("after"), "{e}");

    // undo/redo tools act on the shared history.
    let t = midi_track(&mut h);
    let id: TrackId = t.parse().unwrap();
    let v = ok_call(&mut h, "undo", json!({}));
    assert_eq!(v["undone"], "AI: Create Track");
    assert!(!h.project().tracks.contains_key(&id));
    let v = ok_call(&mut h, "redo", json!({}));
    assert_eq!(v["redone"], "AI: Create Track");
    assert!(h.project().tracks.contains_key(&id));
    let r = call(&mut h, "redo", json!({}));
    assert!(
        r.is_error && r.text.contains("nothing to redo"),
        "{}",
        r.text
    );

    let v = ok_call(&mut h, "save_project", json!({}));
    assert_eq!(v["saved"], "Test");
    assert!(!h.ctl.is_dirty());
}

#[test]
fn overview_and_track_reads() {
    let mut h = Harness::with_project();
    let t = midi_track(&mut h);
    let c = midi_clip(&mut h, &t);
    ok_call(
        &mut h,
        "add_notes",
        json!({ "clip_id": c, "notes": [{ "pitch": 60, "start_beats": 0, "duration_beats": 1 }] }),
    );
    h.ok(Command::Collab(CollabCommand::SetPresence {
        presence: PresenceState {
            selected_tracks: vec![t.parse().unwrap()],
            ..PresenceState::default()
        },
    }));
    let o = ok_call(&mut h, "get_project_overview", json!({}));
    assert_eq!(o["project"]["name"], "Test");
    assert_eq!(o["tempo_bpm"], 120.0);
    assert_eq!(o["time_signature"], "4/4");
    assert_eq!(o["selection"]["tracks"], json!([t]));
    assert_eq!(o["history"]["can_undo"], true);
    let tracks = o["tracks"].as_array().unwrap();
    let mine = tracks.iter().find(|x| x["id"] == t.as_str()).unwrap();
    assert_eq!(mine["kind"], "midi");
    assert_eq!(mine["devices"][0]["type"], "synth");
    assert_eq!(mine["clips"][0]["notes"], 1);
    assert_eq!(mine["clips"][0]["length_beats"], 4.0);
    assert!(tracks.iter().any(|x| x["kind"] == "master"));

    // Many clips: truncated in the overview, paginated in get_track.
    for i in 1..40 {
        ok_call(
            &mut h,
            "create_midi_clip",
            json!({ "track_id": t, "start_beats": 4 * i, "length_beats": 4 }),
        );
    }
    let o = ok_call(&mut h, "get_project_overview", json!({}));
    let mine = o["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == t.as_str())
        .unwrap()
        .clone();
    assert_eq!(mine["clips"].as_array().unwrap().len(), 32);
    assert!(mine["clips_note"].as_str().unwrap().contains("8 more"));
    let tr = ok_call(
        &mut h,
        "get_track",
        json!({ "track_id": t, "clip_limit": 10 }),
    );
    assert_eq!(tr["clips"]["total"], 40);
    assert_eq!(tr["clips"]["items"].as_array().unwrap().len(), 10);
    assert!(
        tr["clips"]["note"]
            .as_str()
            .unwrap()
            .contains("clip_offset 10")
    );
    assert_eq!(tr["mixer"]["volume_db"], 0.0);
    // Reads never edit.
    let r = call(&mut h, "get_track", json!({ "track_id": t }));
    assert!(patches(&r.out).is_empty());
}

#[test]
fn library_and_export() {
    let mut h = Harness::with_project();
    // The library index exists (browser-v2) but has no roots on this test host: an empty page.
    let r = call(&mut h, "search_browser", json!({ "text": "kick" }));
    assert!(!r.is_error, "{}", r.text);
    assert_eq!(serde_json::from_str::<Value>(&r.text).unwrap()["total"], 0);
    let r = call(
        &mut h,
        "load_browser_item",
        json!({ "item_id": "lib/kick.wav" }),
    );
    assert!(r.is_error, "{}", r.text);
    let e = err_call(&mut h, "load_browser_item", json!({ "item_id": "nope" }));
    assert!(e.contains("not a library item id"), "{e}");

    let e = err_call(&mut h, "get_export_status", json!({}));
    assert!(e.contains("no export"), "{e}");
    let e = err_call(
        &mut h,
        "export_audio",
        json!({ "range": "custom", "start_beats": 4 }),
    );
    assert!(e.contains("end_beats") || e.contains("`end`"), "{e}");
    let t = midi_track(&mut h);
    let c = midi_clip(&mut h, &t);
    ok_call(
        &mut h,
        "add_notes",
        json!({ "clip_id": c, "notes": [{ "pitch": 60, "start_beats": 0, "duration_beats": 1 }] }),
    );
    let v = ok_call(
        &mut h,
        "export_audio",
        json!({ "range": "custom", "start_beats": 0, "end_beats": 1, "tail_seconds": 0, "bit_depth": 16 }),
    );
    let job = s(&v["job_id"]);
    let st = ok_call(&mut h, "get_export_status", json!({ "job_id": job }));
    assert_eq!(st["job_id"], job.as_str());
    let mut status = st["status"].clone();
    for _ in 0..2000 {
        if status != "running" {
            break;
        }
        h.advance(20);
        h.tick();
        status = ok_call(&mut h, "get_export_status", json!({}))["status"].clone();
    }
    assert!(
        status == "done" || status == "failed",
        "export finished: {status}"
    );
}

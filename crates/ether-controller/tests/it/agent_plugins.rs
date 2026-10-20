//! Devices and installed plugins through the agent tools (a fake plugin
//! catalog), factory device params, drum pads, rack chains, presets; project querying
//! (`find`, `list_clips`, the overview) and reference resolution ("duplicate the drum loop
//! to span a minute").

use crate::common;

use common::*;
use ether_controller::PluginCatalogView;
use ether_controller::memory::MemoryLibrary;
use ether_core::protocol::agent::{AgentCommand, AgentSelection};
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::devices::DeviceCategory;
use ether_core::protocol::markers::MarkerCommand;
use ether_core::protocol::media::{BrowseLocation, MediaCommand, MediaSource};
use ether_core::protocol::model::*;
use ether_core::protocol::plugins::PluginDescriptor;
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
    let out = h.send(Command::Agent(AgentCommand::CallTool {
        name: name.into(),
        input: input.to_string(),
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

fn ok_call(h: &mut Harness, name: &str, input: Value) -> Value {
    let r = call(h, name, input);
    assert!(!r.is_error, "{name} failed: {}", r.text);
    r.value
}

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

/// An edit tool call: exactly one undo step (one Undo restores, one Redo re-applies).
fn one_step(h: &mut Harness, name: &str, input: Value) -> Value {
    let before = h.project().clone();
    let r = call(h, name, input);
    assert!(!r.is_error, "{name} failed: {}", r.text);
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
    v.as_str()
        .unwrap_or_else(|| panic!("string expected, got {v}"))
        .to_string()
}

fn track(h: &mut Harness, kind: &str, name: &str, instrument: Option<&str>) -> String {
    let mut input = json!({ "kind": kind, "name": name });
    if let Some(i) = instrument {
        input["instrument"] = json!(i);
    }
    s(&ok_call(h, "create_track", input)["track_id"])
}

fn clip(h: &mut Harness, track: &str, name: &str, start: f64, len: f64) -> String {
    s(&ok_call(
        h,
        "create_midi_clip",
        json!({ "track_id": track, "start_beats": start, "length_beats": len, "name": name,
                "notes": [{ "pitch": 36, "start_beats": 0, "duration_beats": 0.5 }] }),
    )["clip_id"])
}

fn raw_clip(h: &mut Harness, track: &str, name: &str, start: f64, len: f64) -> String {
    let id: ClipId = h.id();
    h.ok(Command::Clip(ClipCommand::CreateMidi {
        id,
        track: track.parse().unwrap(),
        start: Beats(start),
        length: Beats(len),
        name: Some(name.into()),
    }));
    id.to_string()
}

fn select(h: &mut Harness, clips: &[&str], tracks: &[&str]) {
    h.ok(Command::Agent(AgentCommand::SetSelection {
        selection: AgentSelection {
            clips: clips.iter().map(|c| c.parse().unwrap()).collect(),
            tracks: tracks.iter().map(|t| t.parse().unwrap()).collect(),
            ..AgentSelection::default()
        },
    }));
}

fn marker(h: &mut Harness, name: &str, at: f64) {
    let id: MarkerId = h.id();
    h.ok(Command::Marker(MarkerCommand::Add {
        id,
        position: Beats(at),
        name: Some(name.into()),
        color: None,
    }));
}

/// The song of the reference-resolution tests: Drums (a "Drum Loop" MIDI clip, 4 bars at
/// bar 1), Bass ("Bassline"), Keys (two clips), sections Intro / Verse / Chorus.
struct Song {
    h: Harness,
    drums: String,
    bass: String,
    keys: String,
    drum_loop: String,
}

fn song() -> Song {
    let mut h = Harness::with_project();
    let drums = track(&mut h, "midi", "Drums", Some("drum-rack"));
    let bass = track(&mut h, "midi", "Bass", None);
    let keys = track(&mut h, "midi", "Keys", Some("poly-synth"));
    let drum_loop = clip(&mut h, &drums, "Drum Loop", 0.0, 16.0);
    clip(&mut h, &bass, "Bassline", 0.0, 16.0);
    clip(&mut h, &keys, "Chords A", 0.0, 8.0);
    clip(&mut h, &keys, "Chords B", 8.0, 8.0);
    marker(&mut h, "Intro", 0.0);
    marker(&mut h, "Verse", 16.0);
    marker(&mut h, "Chorus", 48.0);
    Song {
        h,
        drums,
        bass,
        keys,
        drum_loop,
    }
}

fn clips_of(h: &Harness, track: &str) -> Vec<(f64, f64, String)> {
    let t: TrackId = track.parse().unwrap();
    h.project()
        .arrangement_clips_of(t)
        .iter()
        .map(|c| (c.start.0, c.length.0, c.name.clone()))
        .collect()
}

// ─── Context and reference resolution ───────────────────────────────────────────────────

#[test]
fn duplicate_the_drum_loop_to_span_a_minute() {
    let Song {
        mut h,
        drums,
        drum_loop,
        ..
    } = song();
    // "duplicate the drum loop to span a minute": the model passes the words it heard.
    let v = one_step(
        &mut h,
        "duplicate_clip",
        json!({ "clip_id": "the drum loop", "span_seconds": 60 }),
    );
    assert_eq!(v["resolved"][0]["id"], drum_loop.as_str(), "{v}");
    assert_eq!(v["resolved"][0]["name"], "Drum Loop");
    // 120 BPM: 60 s = 120 beats = 30 bars; a 16-beat clip + 7 copies, the last one cut to 8.
    assert_eq!(v["covers"]["seconds"], 60.0, "{v}");
    assert_eq!(v["covers"]["to_beats"], 120.0);
    assert_eq!(v["covers"]["bars"], "1-30");
    assert_eq!(v["copies"], 7);
    let clips = clips_of(&h, &drums);
    assert_eq!(clips.len(), 8);
    let mut end = 0.0;
    for (start, len, name) in &clips {
        assert_eq!(*start, end, "back to back: {clips:?}");
        assert_eq!(name, "Drum Loop");
        end = start + len;
    }
    assert_eq!(end, 120.0);
    assert_eq!(clips.last().unwrap().1, 8.0);

    // Same with a tempo of 90: 60 s = 90 beats.
    let mut h = song().h;
    ok_call(&mut h, "set_tempo", json!({ "bpm": 90 }));
    let v = ok_call(
        &mut h,
        "duplicate_clip",
        json!({ "clip_id": "drum loop", "span_seconds": 60 }),
    );
    assert_eq!(v["covers"]["to_beats"], 90.0);
    assert_eq!(v["covers"]["seconds"], 60.0);
}

#[test]
fn a_clip_is_found_through_its_track_name() {
    let mut h = Harness::with_project();
    let drums = track(&mut h, "midi", "Drums", Some("drum-rack"));
    let only = clip(&mut h, &drums, "", 0.0, 8.0);
    track(&mut h, "midi", "Bass", None);
    // "duplicate the drum loop": no clip is named so, but Drums has one clip.
    let v = one_step(
        &mut h,
        "duplicate_clip",
        json!({ "clip_id": "drum loop", "span_bars": 8 }),
    );
    assert_eq!(v["resolved"][0]["id"], only.as_str());
    assert_eq!(v["resolved"][0]["track"], "Drums");
    assert_eq!(v["covers"]["to_beats"], 32.0);
    // French, plural and accents: "la batterie".
    let f = ok_call(
        &mut h,
        "find",
        json!({ "query": "la batterie", "prefer": "track" }),
    );
    assert_eq!(f["best"]["kind"], "track");
    assert_eq!(f["best"]["id"], drums.as_str());
}

#[test]
fn copies_resolve_to_the_first_or_the_selected_one() {
    let Song {
        mut h,
        drums,
        drum_loop,
        ..
    } = song();
    ok_call(
        &mut h,
        "duplicate_clip",
        json!({ "clip_id": drum_loop, "count": 2 }),
    );
    // Three "Drum Loop" clips on Drums: the first one, with a note.
    let v = ok_call(
        &mut h,
        "set_clip",
        json!({ "clip_id": "drum loop", "muted": true }),
    );
    assert_eq!(v["clip_id"], drum_loop.as_str());
    assert!(
        s(&v["resolved"][0]["note"]).contains("2 other clip(s)"),
        "{v}"
    );
    // The selected copy wins.
    let third = clips_of(&h, &drums);
    assert_eq!(third.len(), 3);
    let t: TrackId = drums.parse().unwrap();
    let last = h.project().arrangement_clips_of(t)[2].id.to_string();
    select(&mut h, &[&last], &[]);
    let v = ok_call(
        &mut h,
        "set_clip",
        json!({ "clip_id": "drum loop", "muted": true }),
    );
    assert_eq!(v["clip_id"], last.as_str());
}

#[test]
fn ambiguity_returns_candidates_and_selection_breaks_it() {
    let mut h = Harness::with_project();
    let a = track(&mut h, "midi", "Kit A", Some("drum-rack"));
    let b = track(&mut h, "midi", "Kit B", Some("drum-rack"));
    // Made by the user (not the agent: nothing is "recently edited").
    let ca = raw_clip(&mut h, &a, "Drum Loop", 0.0, 8.0);
    let cb = raw_clip(&mut h, &b, "Drum Loop", 8.0, 8.0);
    let e = err_call(
        &mut h,
        "duplicate_clip",
        json!({ "clip_id": "drum loop", "count": 1 }),
    );
    assert!(e.starts_with("Ambiguous:"), "{e}");
    assert!(e.contains(&ca) && e.contains(&cb), "candidates listed: {e}");
    assert!(e.contains("Kit A") && e.contains("Kit B"));
    // find says so too.
    let f = ok_call(
        &mut h,
        "find",
        json!({ "query": "drum loop", "prefer": "clip" }),
    );
    assert_eq!(f["ambiguous"], true, "{f}");
    assert!(f.get("best").is_none());
    // A track name narrows it.
    let v = ok_call(
        &mut h,
        "duplicate_clip",
        json!({ "clip_id": "the drum loop on kit B", "count": 1 }),
    );
    assert_eq!(v["resolved"][0]["id"], cb.as_str(), "{v}");
    // So does the selection (the user selected Kit B's track... and then the clip).
    select(&mut h, &[&ca], &[]);
    let f = ok_call(
        &mut h,
        "find",
        json!({ "query": "drum loop", "prefer": "clip" }),
    );
    assert_eq!(f["best"]["id"], ca.as_str(), "{f}");
    // And recency: the clip the agent just edited (the turn before: "mute the drum loop").
    select(&mut h, &[], &[]);
    ok_call(&mut h, "set_clip", json!({ "clip_id": ca, "muted": true }));
    let f = ok_call(
        &mut h,
        "find",
        json!({ "query": "drum loop", "kinds": ["clip"] }),
    );
    assert_eq!(f["best"]["id"], ca.as_str(), "{f}");
    assert_eq!(f["best"]["why"], "most recently edited");
    // Nothing matches: a clear error.
    let e = err_call(
        &mut h,
        "set_clip",
        json!({ "clip_id": "trombone solo", "muted": true }),
    );
    assert!(e.contains("no clip matches \"trombone solo\""), "{e}");
}

#[test]
fn find_overview_and_list_clips() {
    let Song {
        mut h,
        drums,
        bass,
        keys,
        drum_loop,
    } = song();
    let f = ok_call(&mut h, "find", json!({ "query": "drum loop" }));
    assert_eq!(f["best"]["kind"], "clip", "{f}");
    assert_eq!(f["best"]["id"], drum_loop.as_str());
    assert_eq!(f["best"]["bars"], "1-4");
    assert_eq!(f["best"]["track"], "Drums");
    // Tracks for track verbs, sections, devices by type or by "type on track".
    let f = ok_call(
        &mut h,
        "find",
        json!({ "query": "the bass", "prefer": "track" }),
    );
    assert_eq!(f["best"]["id"], bass.as_str());
    let f = ok_call(&mut h, "find", json!({ "query": "chorus" }));
    assert_eq!(f["best"]["kind"], "section");
    assert_eq!(f["best"]["start_beats"], 48.0);
    let f = ok_call(
        &mut h,
        "find",
        json!({ "query": "le refrain", "kinds": ["section"] }),
    );
    assert_eq!(f["best"]["name"], "Chorus");
    ok_call(
        &mut h,
        "add_device",
        json!({ "track_id": "drums", "type": "reverb" }),
    );
    ok_call(
        &mut h,
        "add_device",
        json!({ "track_id": "keys", "type": "reverb" }),
    );
    let f = ok_call(
        &mut h,
        "find",
        json!({ "query": "reverb on the keys", "kinds": ["device"] }),
    );
    assert_eq!(f["best"]["track_id"], keys.as_str(), "{f}");
    // Two reverbs: the one the agent added last.
    let f = ok_call(
        &mut h,
        "find",
        json!({ "query": "reverb", "kinds": ["device"] }),
    );
    assert_eq!(f["best"]["track_id"], keys.as_str(), "{f}");
    assert_eq!(f["best"]["why"], "most recently edited");
    // Device types and (no) plugins.
    let f = ok_call(
        &mut h,
        "find",
        json!({ "query": "poly synth", "kinds": ["device_type"] }),
    );
    assert_eq!(f["best"]["id"], "poly-synth");

    let o = ok_call(&mut h, "get_project_overview", json!({}));
    assert_eq!(o["beats_per_bar"], 4.0);
    assert_eq!(o["song_end_beats"], 16.0);
    assert_eq!(o["song_seconds"], 8.0);
    let sections = o["sections"].as_array().unwrap();
    assert_eq!(
        sections.iter().map(|x| s(&x["name"])).collect::<Vec<_>>(),
        ["Intro", "Verse", "Chorus"]
    );
    assert_eq!(sections[1]["start_beats"], 16.0);
    assert_eq!(sections[1]["end_beats"], 48.0);
    assert_eq!(sections[1]["bars"], "5-12");
    assert_eq!(o["transport"]["playhead_bar"], 1);
    let dt = o["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == drums.as_str())
        .unwrap()
        .clone();
    assert_eq!(dt["clips"][0]["name"], "Drum Loop");
    assert_eq!(dt["clips"][0]["end_beats"], 16.0);
    assert_eq!(dt["clips"][0]["bars"], "1-4");
    assert_eq!(dt["devices"][0]["type"], "drum-rack");

    let l = ok_call(&mut h, "list_clips", json!({ "track_id": "Keys" }));
    assert_eq!(l["total"], 2);
    assert_eq!(l["clips"][1]["name"], "Chords B");
    assert_eq!(l["clips"][1]["track"], "Keys");
    let l = ok_call(
        &mut h,
        "list_clips",
        json!({ "from_beats": 8, "to_beats": 12 }),
    );
    let names: Vec<String> = l["clips"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| s(&c["name"]))
        .collect();
    assert_eq!(names, ["Drum Loop", "Bassline", "Chords B"]);
    let l = ok_call(&mut h, "list_clips", json!({ "name": "chords" }));
    assert_eq!(l["total"], 2);
}

#[test]
fn spans_counts_split_and_loop() {
    let Song {
        mut h,
        drums,
        drum_loop,
        ..
    } = song();
    // "until the chorus" (Chorus starts at beat 48).
    let v = one_step(
        &mut h,
        "duplicate_clip",
        json!({ "clip_id": drum_loop, "until_beats": 48 }),
    );
    assert_eq!(v["copies"], 2);
    assert_eq!(clips_of(&h, &drums).len(), 3);
    h.ok(Command::Edit(EditCommand::Undo));
    // "twice".
    let v = one_step(
        &mut h,
        "duplicate_clip",
        json!({ "clip_id": drum_loop, "count": 2 }),
    );
    assert_eq!(v["covers"]["to_beats"], 48.0);
    h.ok(Command::Edit(EditCommand::Undo));
    // Already long enough.
    let v = ok_call(
        &mut h,
        "duplicate_clip",
        json!({ "clip_id": drum_loop, "span_bars": 2 }),
    );
    assert_eq!(v["copies"], 0);
    let e = err_call(
        &mut h,
        "duplicate_clip",
        json!({ "clip_id": drum_loop, "span_bars": 2, "count": 2 }),
    );
    assert!(e.contains("cannot be combined"), "{e}");
    // "loop the drum loop for 8 bars": loop on, 32 beats long, the 16-beat content repeats.
    let v = one_step(
        &mut h,
        "set_clip",
        json!({ "clip_id": "drum loop", "loop_enabled": true, "end_beats": 32 }),
    );
    assert_eq!(v["length_beats"], 32.0);
    assert_eq!(v["loop"]["end_beats"], 16.0, "{v}");
    // Split at bar 3.
    let v = one_step(
        &mut h,
        "split_clip",
        json!({ "clip_id": drum_loop, "at_beats": [8] }),
    );
    assert_eq!(v["parts"].as_array().unwrap().len(), 2);
}

#[test]
fn key_change_in_a_section() {
    let mut h = Harness::with_project();
    let drums = track(&mut h, "midi", "Drums", Some("drum-rack"));
    let keys = track(&mut h, "midi", "Keys", None);
    let dc = clip(&mut h, &drums, "Beat", 0.0, 32.0);
    let kc = s(&ok_call(
        &mut h,
        "create_midi_clip",
        json!({ "track_id": keys, "start_beats": 0, "length_beats": 32,
                "notes": [{ "pitch": 60, "start_beats": 0, "duration_beats": 1 },
                          { "pitch": 64, "start_beats": 20, "duration_beats": 1 }] }),
    )["clip_id"]);
    // "+3 in the bridge" (beats 16-32): the Keys clip is split at 16, its right part
    // transposed; drums untouched.
    let v = one_step(
        &mut h,
        "transpose",
        json!({ "semitones": 3, "from_beats": 16, "to_beats": 32 }),
    );
    assert_eq!(v["midi_clips"], 1, "{v}");
    assert_eq!(v["split"], 1);
    assert_eq!(v["skipped_drum_tracks"], json!(["Drums"]));
    let p = h.project();
    let k: TrackId = keys.parse().unwrap();
    let parts = p.arrangement_clips_of(k);
    assert_eq!(parts.len(), 2);
    let pitches = |c: ClipId| -> Vec<u8> { p.notes_of(c).iter().map(|n| n.pitch).collect() };
    // The left part plays its notes untouched (notes past its end stay silent).
    let left: ClipId = kc.parse().unwrap();
    let audible: Vec<u8> = p
        .notes_of(left)
        .iter()
        .filter(|n| n.start.0 < p.clips[&left].length.0)
        .map(|n| n.pitch)
        .collect();
    assert_eq!(audible, vec![60]);
    assert_eq!(parts[1].start.0, 16.0);
    assert!(
        pitches(parts[1].id).contains(&67),
        "{:?}",
        pitches(parts[1].id)
    );
    let d: ClipId = dc.parse().unwrap();
    assert_eq!(p.notes_of(d)[0].pitch, 36);
}

// ─── Devices and plugins ────────────────────────────────────────────────────────────────

fn catalog_entry(
    id: &str,
    name: &str,
    vendor: &str,
    format: PluginFormat,
    category: DeviceCategory,
) -> PluginDescriptor {
    PluginDescriptor {
        format,
        id: id.into(),
        name: name.into(),
        vendor: vendor.into(),
        version: "1.0".into(),
        description: String::new(),
        features: vec![match category {
            DeviceCategory::Instrument => "instrument".into(),
            _ => "audio-effect".into(),
        }],
        category,
        path: format!("/plugins/{name}"),
        sidechain_inputs: 0,
    }
}

/// A host with three installed plugins (Surge XT as CLAP and VST3, Valhalla Room,
/// Valhalla Delay); `instantiable` = what the fake bridge can create.
fn plugin_host() -> Harness {
    let mut bridge = FakeBridge::default();
    let mut surge = plugin_descriptor("Surge XT", DeviceCategory::Instrument);
    surge.params.push(ether_core::protocol::devices::ParamInfo {
        id: ParamId(12),
        name: "Filter 1 Cutoff".into(),
        group: Some("Filter".into()),
        unit: ether_core::protocol::devices::ParamUnit::Hertz,
        min: 20.0,
        max: 20000.0,
        default: 1000.0,
        scale: ether_core::protocol::devices::ParamScale::Log,
        labels: None,
        automatable: true,
        hidden: false,
        step: None,
        remote: None,
    });
    surge.params.push(ether_core::protocol::devices::ParamInfo {
        id: ParamId(99),
        name: "MIDI CC 7".into(),
        group: None,
        unit: ether_core::protocol::devices::ParamUnit::None,
        min: 0.0,
        max: 1.0,
        default: 0.0,
        scale: ether_core::protocol::devices::ParamScale::Linear,
        labels: None,
        automatable: true,
        hidden: false,
        step: None,
        remote: None,
    });
    let room = plugin_descriptor("Valhalla Room", DeviceCategory::AudioEffect);
    bridge.plugins = Some(
        [
            ("org.surge-synth-team.surge-xt".to_string(), surge),
            ("com.valhalla.room".to_string(), room),
        ]
        .into_iter()
        .collect(),
    );
    bridge.catalog = Some(PluginCatalogView {
        plugins: vec![
            catalog_entry(
                "org.surge-synth-team.surge-xt",
                "Surge XT",
                "Surge Synth Team",
                PluginFormat::Clap,
                DeviceCategory::Instrument,
            ),
            catalog_entry(
                "org.surge-synth-team.surge-xt",
                "Surge XT",
                "Surge Synth Team",
                PluginFormat::Vst3,
                DeviceCategory::Instrument,
            ),
            catalog_entry(
                "com.valhalla.room",
                "Valhalla Room",
                "Valhalla DSP",
                PluginFormat::Clap,
                DeviceCategory::AudioEffect,
            ),
            catalog_entry(
                "com.valhalla.delay",
                "Valhalla Delay",
                "Valhalla DSP",
                PluginFormat::Vst3,
                DeviceCategory::AudioEffect,
            ),
        ],
        scanning: false,
    });
    let mut h = Harness::with(bridge, MemoryLibrary::new(), Default::default());
    h.create_project("Plugins");
    h
}

#[test]
fn list_plugins_filters_and_pages() {
    let mut h = plugin_host();
    let l = ok_call(&mut h, "list_plugins", json!({}));
    assert_eq!(l["available"], true);
    assert_eq!(l["installed"], 4);
    assert_eq!(l["scanning"], false);
    let l = ok_call(&mut h, "list_plugins", json!({ "category": "instrument" }));
    assert_eq!(l["total"], 2);
    assert!(
        l["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["category"] == "instrument")
    );
    let l = ok_call(&mut h, "list_plugins", json!({ "query": "valhalla" }));
    assert_eq!(l["total"], 2);
    let l = ok_call(
        &mut h,
        "list_plugins",
        json!({ "format": "vst3", "limit": 1 }),
    );
    assert_eq!(l["total"], 2);
    assert!(s(&l["note"]).contains("offset 1"));
    let p = &l["plugins"][0];
    for k in [
        "plugin_id",
        "name",
        "vendor",
        "format",
        "category",
        "version",
    ] {
        assert!(p.get(k).is_some(), "{k} in {p}");
    }
    // The web host: no plugins.
    let mut web = Harness::with_project();
    let l = ok_call(&mut web, "list_plugins", json!({}));
    assert_eq!(l["available"], false);
    assert!(s(&l["note"]).contains("web"));
    let t = track(&mut web, "audio", "Vox", None);
    let e = err_call(
        &mut web,
        "add_device",
        json!({ "track_id": t, "query": "Valhalla Room" }),
    );
    assert!(e.contains("not available on this host"), "{e}");
}

#[test]
fn add_plugins_by_id_and_name_with_placement() {
    let mut h = plugin_host();
    let bass = track(&mut h, "midi", "Bass", Some("synth"));
    let tid: TrackId = bass.parse().unwrap();
    ok_call(
        &mut h,
        "add_device",
        json!({ "track_id": bass, "type": "eq" }),
    );
    ok_call(
        &mut h,
        "add_device",
        json!({ "track_id": bass, "type": "arpeggiator" }),
    );
    // "add Surge XT on the bass": by name, replaces the Synth in place, one undo step.
    let v = one_step(
        &mut h,
        "add_device",
        json!({ "track_id": "the bass", "query": "surge xt" }),
    );
    assert_eq!(v["plugin"], "Surge XT", "{v}");
    assert_eq!(
        v["format"], "clap",
        "the preferred format of a multi-format plugin"
    );
    assert_eq!(v["category"], "instrument");
    assert_eq!(v["replaced"]["name"], "synth");
    let chain: Vec<String> = h
        .project()
        .devices_of(tid)
        .iter()
        .map(|d| d.name.clone())
        .collect();
    assert_eq!(
        chain,
        ["arpeggiator", "Surge XT", "eq"],
        "MIDI fx, instrument, audio fx"
    );
    let surge = s(&v["device_id"]);
    // By id + format, an effect at the end.
    let v = one_step(
        &mut h,
        "add_device",
        json!({ "track_id": bass, "plugin_id": "com.valhalla.room", "format": "clap" }),
    );
    assert_eq!(v["index"], 3);
    assert!(v.get("replaced").is_none());
    // Ambiguous name: candidates.
    let e = err_call(
        &mut h,
        "add_device",
        json!({ "track_id": bass, "query": "valhalla" }),
    );
    assert!(
        e.starts_with("Ambiguous:") && e.contains("Valhalla Delay") && e.contains("Valhalla Room"),
        "{e}"
    );
    let e = err_call(
        &mut h,
        "add_device",
        json!({ "track_id": bass, "plugin_id": "com.nope" }),
    );
    assert!(e.contains("installed plugin `com.nope`"), "{e}");
    // An instrument on an audio track: a clear error.
    let vox = track(&mut h, "audio", "Vox", None);
    let e = err_call(
        &mut h,
        "add_device",
        json!({ "track_id": vox, "query": "Surge XT" }),
    );
    assert!(e.contains("instruments go on MIDI tracks"), "{e}");

    // Plugin params: plumbing hidden, display units, fuzzy names, relative changes.
    let p = ok_call(&mut h, "get_device_params", json!({ "device_id": surge }));
    let names: Vec<String> = p["params"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| s(&x["name"]))
        .collect();
    assert_eq!(names, ["Mix", "Filter 1 Cutoff"], "MIDI CC hidden");
    assert_eq!(p["params"][1]["display"], "1.00 kHz");
    let p = ok_call(
        &mut h,
        "get_device_params",
        json!({ "device_id": surge, "all": true }),
    );
    assert_eq!(p["total"], 3);
    let v = one_step(
        &mut h,
        "set_device_param",
        json!({ "device_id": surge, "param": "cutoff", "value": "2 kHz" }),
    );
    assert_eq!(v["value"], 2000.0, "{v}");
    assert_eq!(v["previous"], "1.00 kHz");
    // "brighter, a bit more": +12% of the (log) knob travel.
    let one = ok_call(
        &mut h,
        "get_device_param",
        json!({ "device_id": surge, "param": "Filter 1 Cutoff" }),
    );
    let more = s(&one["a_bit_more"]);
    let v = ok_call(
        &mut h,
        "set_device_param",
        json!({ "device_id": "surge xt", "param": 12, "delta_percent": 12 }),
    );
    assert_eq!(s(&v["display"]), more);
    assert!(v["value"].as_f64().unwrap() > 2000.0);
    let v = ok_call(
        &mut h,
        "set_device_param",
        json!({ "device_id": surge, "param": "mix", "normalized": 0.25 }),
    );
    assert_eq!(v["value"], 25.0);
    let e = err_call(
        &mut h,
        "set_device_param",
        json!({ "device_id": surge, "param": "mix", "value": 1, "delta": 2 }),
    );
    assert!(e.contains("exactly one of"), "{e}");
}

#[test]
fn factory_device_params_by_name() {
    let mut h = Harness::with_project();
    let t = track(&mut h, "midi", "Lead", Some("poly-synth"));
    // Every built-in device lists its params by name and takes a set by name.
    let types = ok_call(&mut h, "list_device_types", json!({}));
    let audio = track(&mut h, "audio", "FX", None);
    for ty in types["device_types"].as_array().unwrap() {
        let key = s(&ty["type"]);
        if ["external-instrument", "external-audio-effect"].contains(&key.as_str()) {
            continue;
        }
        let target = if ty["category"] == "audio_effect" {
            &audio
        } else {
            &t
        };
        let r = call(
            &mut h,
            "add_device",
            json!({ "track_id": target, "type": key }),
        );
        assert!(!r.is_error, "{key}: {}", r.text);
        let id = s(&r.value["device_id"]);
        let p = ok_call(
            &mut h,
            "get_device_params",
            json!({ "device_id": id, "limit": 500 }),
        );
        let params = p["params"].as_array().unwrap();
        if params.is_empty() {
            continue;
        }
        for x in params {
            assert!(x["display"].is_string(), "{key}: {x}");
            let n = x["normalized"].as_f64().unwrap();
            assert!((0.0..=1.0).contains(&n), "{key}: {x}");
        }
        // Set the first non-choice param by name to its max, then back by a relative step.
        if let Some(x) = params.iter().find(|x| x.get("labels").is_none()) {
            let name = s(&x["name"]);
            let v = ok_call(
                &mut h,
                "set_device_param",
                json!({ "device_id": id, "param": name, "normalized": 1 }),
            );
            assert_eq!(v["value"], x["max"], "{key}.{name}: {v}");
            let v = ok_call(
                &mut h,
                "set_device_param",
                json!({ "device_id": id, "param": name, "delta_percent": -50 }),
            );
            assert!(
                v["value"].as_f64().unwrap() < x["max"].as_f64().unwrap(),
                "{key}.{name}: {v}"
            );
        }
    }
    // A reverb's size, French-ish naming, and a display value.
    let rev = s(&ok_call(
        &mut h,
        "add_device",
        json!({ "track_id": audio, "query": "réverb" }),
    )["device_id"]);
    let one = ok_call(
        &mut h,
        "get_device_param",
        json!({ "device_id": rev, "param": "size" }),
    );
    assert!(one["display"].is_string(), "{one}");
    let eq = s(&ok_call(
        &mut h,
        "add_device",
        json!({ "track_id": audio, "type": "eq" }),
    )["device_id"]);
    let all = ok_call(
        &mut h,
        "get_device_params",
        json!({ "device_id": eq, "query": "freq" }),
    );
    assert!(all["total"].as_u64().unwrap() >= 1, "{all}");
}

#[test]
fn drum_pads_and_rack_chains() {
    let mut lib = MemoryLibrary::new();
    lib.add_root("lib", "Library");
    lib.add_file(
        "lib",
        "clap.wav",
        wav(48_000, &[sine(48_000, 220.0, 4_800, 0.5)]),
    );
    let mut h = Harness::with(FakeBridge::default(), lib, Default::default());
    h.create_project("Kit");
    h.ok(Command::Browser(
        ether_core::protocol::browser::BrowserCommand::ListRoots,
    ));
    for _ in 0..400 {
        h.advance(20);
        h.tick();
    }
    let drums = track(&mut h, "midi", "Drums", Some("drum-rack"));
    let rack = s(&ok_call(
        &mut h,
        "find",
        json!({ "query": "drum rack", "kinds": ["device"] }),
    )["best"]["id"]);
    // A sample from an audio clip of the project onto C1, then a library one onto D1.
    let audio = track(&mut h, "audio", "Samples", None);
    let media: MediaId = h.id();
    h.ok(Command::Media(MediaCommand::Import {
        id: media,
        source: MediaSource::Location {
            location: BrowseLocation::Library { id: "lib".into() },
            path: "clap.wav".into(),
        },
    }));
    h.drain_media();
    let cid: ClipId = h.id();
    h.ok(Command::Clip(ClipCommand::CreateAudio {
        id: cid,
        track: audio.parse().unwrap(),
        start: Beats(0.0),
        media,
    }));
    let v = one_step(
        &mut h,
        "load_drum_pad_sample",
        json!({ "device_id": rack, "note": 36, "clip_id": cid.to_string() }),
    );
    assert_eq!(v["note_name"], "C1");
    let v = ok_call(
        &mut h,
        "load_drum_pad_sample",
        json!({ "device_id": "drum rack", "pad": "D1", "item_id": "lib/clap.wav" }),
    );
    assert_eq!(v["note"], 38, "{v}");
    assert_eq!(v["name"], "clap");
    h.drain_media();
    let p = ok_call(&mut h, "get_device_params", json!({ "device_id": rack }));
    let pads = p["device"]["pads"].as_array().unwrap();
    assert_eq!(pads.len(), 2, "{p}");
    assert_eq!(pads[1]["name"], "clap");
    // Pad edits by name / note name, choke group, solo (live state).
    ok_call(
        &mut h,
        "set_drum_pad",
        json!({ "device_id": rack, "pad": "C1", "name": "Kick" }),
    );
    let v = one_step(
        &mut h,
        "set_drum_pad",
        json!({ "device_id": rack, "pad": "the clap", "volume_db": -6, "choke_group": 1 }),
    );
    assert_eq!(v["note"], 38);
    assert_eq!(v["volume_db"], -6.0);
    assert_eq!(v["choke_group"], 1);
    let v = ok_call(
        &mut h,
        "set_drum_pad",
        json!({ "device_id": rack, "pad": "kick", "mute": true }),
    );
    assert_eq!(v["note"], 36);
    assert_eq!(v["mute"], true);
    let e = err_call(
        &mut h,
        "set_drum_pad",
        json!({ "device_id": rack, "note": 50, "mute": true }),
    );
    assert!(e.contains("pad is empty"), "{e}");
    // Replacing a pad's sample keeps one pad on that note.
    ok_call(
        &mut h,
        "load_drum_pad_sample",
        json!({ "device_id": rack, "note": 38, "clip_id": cid.to_string() }),
    );
    let p = ok_call(&mut h, "get_device_params", json!({ "device_id": rack }));
    assert_eq!(p["device"]["pads"].as_array().unwrap().len(), 2);
    let e = err_call(
        &mut h,
        "set_drum_pad",
        json!({ "track_id": drums, "device_id": "Drums", "note": 36, "mute": false }),
    );
    assert!(
        e.contains("not a Drum Rack") || e.contains("unknown property"),
        "{e}"
    );

    // Rack chains: an Instrument Rack with chains, edited by chain name.
    let keys = track(&mut h, "midi", "Keys", Some("none"));
    let r = s(&ok_call(
        &mut h,
        "add_device",
        json!({ "track_id": keys, "type": "instrument-rack" }),
    )["device_id"]);
    let did: DeviceId = r.parse().unwrap();
    let chains: Vec<RackChainId> = h.project().chains_of(did).iter().map(|c| c.id).collect();
    if chains.is_empty() {
        let id: RackChainId = h.id();
        h.ok(Command::Rack(
            ether_core::protocol::racks::RackCommand::AddChain {
                id,
                rack: did,
                name: Some("Piano".into()),
                before: None,
            },
        ));
    }
    let p = ok_call(&mut h, "get_device_params", json!({ "device_id": r }));
    let chain = &p["device"]["chains"][0];
    let cname = s(&chain["name"]);
    let v = one_step(
        &mut h,
        "set_rack_chain",
        json!({ "chain": cname, "device_id": r, "volume_db": -3, "mute": true }),
    );
    assert_eq!(v["volume_db"], -3.0, "{v}");
    assert_eq!(v["mute"], true);
}

#[test]
fn device_presets() {
    let mut h = Harness::with_project();
    let t = track(&mut h, "midi", "Pads", Some("poly-synth"));
    let synth = s(&ok_call(
        &mut h,
        "find",
        json!({ "query": "poly synth", "kinds": ["device"] }),
    )["best"]["id"]);
    let l = ok_call(&mut h, "list_device_presets", json!({ "device_id": synth }));
    let presets = l["presets"].as_array().unwrap();
    if presets.is_empty() {
        return; // no factory presets for this type in this build
    }
    let name = s(&presets[0]["name"]);
    let v = one_step(
        &mut h,
        "load_device_preset",
        json!({ "device_id": synth, "preset": name.to_lowercase() }),
    );
    assert_eq!(v["preset"], name.as_str());
    let e = err_call(
        &mut h,
        "load_device_preset",
        json!({ "device_id": synth, "preset": "zzzz qqqq" }),
    );
    assert!(e.contains("no preset"), "{e}");
    let _ = t;
}

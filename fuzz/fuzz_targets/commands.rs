//! Engine commands against the controller: every command a UI, an agent or a collaborator can
//! send goes through `EtherController::handle`, so a malformed or extreme one must reply an
//! error (never panic, abort on a huge allocation or hang) and must never leave a document that
//! can't be saved and reopened.
//!
//! Input: newline-separated lines over a small seeded project (two tracks with clips, notes, a
//! synth, an eq, a return with a send, an imported sample). A line is
//! - a JSON `Command` (`{"domain":"Track","command":{...}}`). Strings `"@<k><n>"` become the
//!   `n`th existing id of a table (`t` tracks, `c` clips, `n` notes, `d` devices, `s` sends,
//!   `l` automation lanes, `p` automation points, `k` tempo points, `g` time signatures, `w` warp
//!   markers, `m` media, `r` markers, `i` MIDI mappings, `q` drum pads, `y` take lanes, `z` comp
//!   regions, `h` rack chains, `o` modulators, `x` modulation mappings, `e` expression lanes,
//!   `v` note expressions), `"@new"` a fresh id, `"@proj"` the open project's, `"@Pnew"` a
//!   fresh project id and `"@P<n>"` the `n`th project id seen (the seeded one, then each
//!   `"@Pnew"`), `"@R<n>"`/`"@T<n>"`/`"@K<n>"` the `n`th preset id, template id or export
//!   download token a reply or an event named, so mutations keep hitting real entities;
//! - `tick` (or `tick <n>`, at most 50): the clock moves 20 ms and the controller ticks (media
//!   and render jobs, autosave);
//! - empty or `#...`: ignored.
//!
//! Checked after every line: exactly one reply per command, the fake engine's invariants (no
//! graph references a dead node, no live node destroyed twice), and the open document saves and
//! loads back. At the end, when the seeded project is still open and its own history is intact,
//! undoing every step the script made gives the seeded document back, and redoing them the
//! edited one (but for mute and solo, never undo steps).
#![no_main]
use libfuzzer_sys::fuzz_target;

#[path = "../../crates/ether-controller/tests/common/mod.rs"]
mod common;

use std::collections::BTreeMap;

use common::*;
use ether_controller::Controller;
use ether_controller::memory::MemoryLibrary;
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::devices::{DeviceCategory, DeviceCommand, DeviceSpec};
use ether_core::protocol::media::*;
use ether_core::protocol::mixer::MixerCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteSpec};
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::undo_history::HistoryCommand;
use ether_core::protocol::*;
use serde_json::Value;

const SR: u32 = 48_000;
const MAX_LINES: usize = 64;

fn setup() -> Harness {
    let mut lib = MemoryLibrary::new().with_user_root("user");
    lib.add_root("lib", "Library");
    lib.add_file(
        "lib",
        "loop.wav",
        wav(SR, &[sine(SR, 440.0, SR as usize / 10, 0.5)]),
    );
    let mut bridge = FakeBridge::default();
    let fx = plugin_descriptor("Fx", DeviceCategory::AudioEffect);
    bridge.plugins = Some(BTreeMap::from([("com.test.Fx".to_string(), fx)]));
    bridge.offline_plugins = true;
    bridge.recording = Some(Default::default());
    let mut h = Harness::with(bridge, lib, Default::default());
    h.create_project("Fuzz");

    let media: MediaId = h.id();
    h.ok(Command::Media(MediaCommand::Import {
        id: media,
        source: MediaSource::Location {
            location: BrowseLocation::Library { id: "lib".into() },
            path: "loop.wav".into(),
        },
    }));
    h.drain_media();

    let track = |h: &mut Harness, kind| {
        let id: TrackId = h.id();
        h.ok(Command::Track(TrackCommand::Create {
            id,
            kind,
            name: None,
            color: None,
            parent: None,
            before: None,
        }));
        id
    };
    let device = |h: &mut Harness, track, device| {
        let id: DeviceId = h.id();
        h.ok(Command::Device(DeviceCommand::Insert {
            id,
            track,
            device: DeviceSpec::Builtin { device },
            before: None,
        }));
    };

    let audio = track(&mut h, TrackKind::Audio);
    let clip: ClipId = h.id();
    h.ok(Command::Clip(ClipCommand::CreateAudio {
        id: clip,
        track: audio,
        start: Beats(0.0),
        media,
    }));
    device(&mut h, audio, BuiltinDevice::Eq);

    let midi = track(&mut h, TrackKind::Midi);
    device(&mut h, midi, BuiltinDevice::Synth);
    let clip: ClipId = h.id();
    h.ok(Command::Clip(ClipCommand::CreateMidi {
        id: clip,
        track: midi,
        start: Beats(0.0),
        length: Beats(4.0),
        name: None,
    }));
    let notes = (0..4)
        .map(|i| NoteSpec {
            id: h.id(),
            pitch: 60 + i * 4,
            velocity: 100.0,
            start: Beats(i as f64),
            duration: Beats(0.5),
        })
        .collect();
    h.ok(Command::Note(NoteCommand::Add { clip, notes }));

    let ret = track(&mut h, TrackKind::Return);
    let send: SendId = h.id();
    h.ok(Command::Mixer(MixerCommand::CreateSend {
        id: send,
        from: midi,
        to: ret,
        level: Decibels(-6.0),
        pre_fader: false,
    }));
    h
}

/// The `n`th id (wrapping) of the table `k` stands for, or `None` (unknown table or empty).
fn existing(p: &Project, k: u8, n: usize) -> Option<String> {
    fn nth<K: ToString, V>(m: &BTreeMap<K, V>, n: usize) -> Option<String> {
        (!m.is_empty()).then(|| m.keys().nth(n % m.len()).unwrap().to_string())
    }
    match k {
        b't' => nth(&p.tracks, n),
        b'c' => nth(&p.clips, n),
        b'n' => nth(&p.notes, n),
        b'd' => nth(&p.devices, n),
        b's' => nth(&p.sends, n),
        b'l' => nth(&p.automation_lanes, n),
        b'p' => nth(&p.automation_points, n),
        b'k' => nth(&p.tempo_points, n),
        b'g' => nth(&p.time_signatures, n),
        b'w' => nth(&p.warp_markers, n),
        b'm' => nth(&p.media, n),
        b'r' => nth(&p.markers, n),
        b'i' => nth(&p.midi_mappings, n),
        b'q' => nth(&p.drum_pads, n),
        b'y' => nth(&p.take_lanes, n),
        b'z' => nth(&p.comp_regions, n),
        b'h' => nth(&p.rack_chains, n),
        b'o' => nth(&p.modulators, n),
        b'x' => nth(&p.mod_mappings, n),
        b'e' => nth(&p.expression_lanes, n),
        b'v' => nth(&p.note_expressions, n),
        _ => None,
    }
}

/// The fuzz run: the harness and the ids the script can name that live outside the document.
struct State {
    h: Harness,
    /// Project ids: the seeded one, then every `"@Pnew"`.
    projects: Vec<String>,
    presets: Vec<String>,
    templates: Vec<String>,
    tokens: Vec<String>,
}

fn nth(v: &[String], n: usize) -> Option<String> {
    (!v.is_empty()).then(|| v[n % v.len()].clone())
}

impl State {
    /// Remembers the preset ids, template ids and download tokens `out` names.
    fn harvest(&mut self, out: &[ServerMessage]) {
        fn add(v: &mut Vec<String>, id: &str) {
            if v.len() < 64 && !v.iter().any(|x| x == id) {
                v.push(id.to_string());
            }
        }
        fn walk(s: &mut State, v: &Value) {
            match v {
                Value::Object(o) => {
                    if let Some(Value::String(t)) = o.get("token") {
                        add(&mut s.tokens, t);
                    }
                    if let (Some(Value::String(src)), Some(Value::String(id))) =
                        (o.get("source"), o.get("id"))
                        && (src == "User" || src == "Factory")
                    {
                        add(&mut s.presets, id);
                    }
                    if let (Some(Value::String(id)), Some(_)) = (o.get("id"), o.get("factory")) {
                        add(&mut s.templates, id);
                    }
                    o.values().for_each(|v| walk(s, v));
                }
                Value::Array(a) => a.iter().for_each(|v| walk(s, v)),
                _ => {}
            }
        }
        for m in out {
            if let Ok(v) = serde_json::to_value(m) {
                walk(self, &v);
            }
        }
    }
}

/// `line` with its `"@..."` placeholders replaced (unknown ones are left as they are).
fn substitute(st: &mut State, line: &str) -> String {
    let h = &mut st.h;
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = rest.find("\"@") {
        out.push_str(&rest[..at + 1]);
        rest = &rest[at + 1..];
        let end = rest[1..].find(['"', '\\']).map_or(rest.len(), |e| e + 1);
        let token = &rest[1..end];
        let id = if token == "new" {
            Some(h.id::<TrackId>().to_string())
        } else if token == "proj" {
            h.ctl.project().map(|p| p.id.to_string())
        } else if token == "Pnew" {
            let id = h.project_id().to_string();
            st.projects.push(id.clone());
            Some(id)
        } else if let Some(table) = token
            .strip_prefix('P')
            .map(|n| (&st.projects, n))
            .or_else(|| token.strip_prefix('R').map(|n| (&st.presets, n)))
            .or_else(|| token.strip_prefix('T').map(|n| (&st.templates, n)))
            .or_else(|| token.strip_prefix('K').map(|n| (&st.tokens, n)))
        {
            table.1.parse().ok().and_then(|n| nth(table.0, n))
        } else {
            let (k, n) = token.split_at(token.chars().next().map_or(0, char::len_utf8));
            match (k.as_bytes().first(), n.parse::<usize>(), h.ctl.project()) {
                (Some(&k), Ok(n), Some(p)) => existing(p, k, n),
                _ => None,
            }
        };
        match id {
            Some(id) => out.push_str(&id),
            None => out.push_str(&rest[..end]),
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

fn check_document(h: &Harness, line: &str) {
    let Some(p) = h.ctl.project() else { return };
    let saved = ether_model::file::save(p, "fuzz").expect("the document serializes");
    if let Err(e) = ether_model::file::load(&saved) {
        panic!("the document no longer loads after `{line}`: {e}");
    }
}

/// Sends `command`: exactly one reply, and no document change after it (a publish that
/// follows the reply may still notify, e.g. a device the engine couldn't create).
fn send(h: &mut Harness, command: Command) -> Vec<ServerMessage> {
    const ID: u32 = 1 << 30;
    let mut out = Vec::new();
    h.ctl.handle(
        ClientMessage {
            id: ID,
            gesture: None,
            command,
        },
        &mut out,
    );
    let replies: Vec<usize> = (0..out.len())
        .filter(|&i| matches!(&out[i], ServerMessage::Reply(_)))
        .collect();
    assert!(
        matches!(replies[..], [i] if matches!(&out[i], ServerMessage::Reply(r) if r.id == ID)),
        "exactly one reply: {out:#?}"
    );
    assert!(
        !out[replies[0]..]
            .iter()
            .any(|m| matches!(m, ServerMessage::Event(Event::Patch { .. }))),
        "a patch after the reply: {out:#?}"
    );
    out
}

/// The applied steps of the open project's history (oldest first).
fn applied_steps(h: &mut Harness) -> Option<Vec<u32>> {
    match send(h, Command::History(HistoryCommand::List)).last() {
        Some(ServerMessage::Reply(Reply {
            result:
                ReplyResult::Ok {
                    value: ReplyValue::History { history },
                },
            ..
        })) => Some(
            history
                .steps
                .iter()
                .filter(|s| !s.undone)
                .map(|s| s.id)
                .collect(),
        ),
        _ => None,
    }
}

/// The document as JSON, without what isn't an undo step by design: mute and solo clicks
/// (docs/CONTRACTS.md, "Undoable vs. not undoable").
fn undoable_state(p: &Project) -> Value {
    let mut v = serde_json::to_value(p).expect("the document serializes");
    let clear = |v: &mut Value| {
        for k in ["mute", "solo"] {
            if let Some(b) = v.get_mut(k) {
                *b = Value::Bool(false);
            }
        }
    };
    if let Some(Value::Object(tracks)) = v.get_mut("tracks") {
        tracks
            .values_mut()
            .filter_map(|t| t.get_mut("mixer"))
            .for_each(clear);
    }
    if let Some(Value::Object(pads)) = v.get_mut("drum_pads") {
        pads.values_mut().for_each(clear);
    }
    v
}

/// Where `a` and `b` differ (JSON paths with both values; at most 8).
fn diff(path: &str, a: &Value, b: &Value, out: &mut Vec<String>) {
    if out.len() >= 8 || a == b {
        return;
    }
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let keys: std::collections::BTreeSet<&String> = x.keys().chain(y.keys()).collect();
            for k in keys {
                let null = Value::Null;
                let at = format!("{path}/{k}");
                diff(
                    &at,
                    x.get(k).unwrap_or(&null),
                    y.get(k).unwrap_or(&null),
                    out,
                );
            }
        }
        _ => out.push(format!("{path}: {a} -> {b}")),
    }
}

/// Undoes the script's steps; the document must then be the seeded one again. Skipped when
/// another project is open or the script undid into the seeding steps (its history no longer
/// starts with them).
fn check_undo_all(st: &mut State, seeded: &Value, seed_steps: &[u32]) {
    let h = &mut st.h;
    let edited = undoable_state(h.project());
    for undone in 0..=MAX_LINES * 2 {
        if h.ctl.project().map(|p| p.id.to_string()).as_ref() != st.projects.first() {
            return;
        }
        let Some(steps) = applied_steps(h) else {
            return;
        };
        if steps == seed_steps {
            if std::env::var_os("FUZZ_COMMANDS_DEBUG").is_some() {
                eprintln!("undo all: back to the seeded steps");
            }
            same(seeded, h.project(), "undoing every step");
            // And redoing them all gives the edited document back.
            for _ in 0..undone {
                send(h, Command::Edit(EditCommand::Redo));
            }
            same(&edited, h.project(), "redoing every step");
            return;
        }
        if steps.len() <= seed_steps.len() || !steps.starts_with(seed_steps) {
            return;
        }
        send(h, Command::Edit(EditCommand::Undo));
    }
    panic!("undo doesn't go back through the history");
}

fn same(expected: &Value, p: &Project, after: &str) {
    let mut diffs = Vec::new();
    diff("", expected, &undoable_state(p), &mut diffs);
    assert!(
        diffs.is_empty(),
        "{after} doesn't give the expected document: {diffs:#?}"
    );
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let mut h = setup();
    let debug = std::env::var_os("FUZZ_COMMANDS_DEBUG").is_some();
    if debug {
        let p = h.ctl.project().unwrap();
        let tracks: Vec<_> = p.tracks.values().map(|t| format!("{:?}", t.kind)).collect();
        let devices: Vec<_> = p
            .devices
            .values()
            .map(|d| format!("{:?}", d.kind))
            .collect();
        eprintln!("tracks {tracks:?}\ndevices {devices:?}");
    }
    let seeded = undoable_state(h.project());
    let seed_steps = applied_steps(&mut h).expect("a history");
    let mut st = State {
        projects: vec![h.project().id.to_string()],
        h,
        presets: Vec::new(),
        templates: Vec::new(),
        tokens: Vec::new(),
    };
    for line in text.lines().take(MAX_LINES) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(n) = line.strip_prefix("tick") {
            let n = n.trim().parse::<usize>().unwrap_or(1).min(50);
            for _ in 0..n {
                st.h.advance(20);
                let out = st.h.tick();
                st.harvest(&out);
            }
            continue;
        }
        let json = substitute(&mut st, line);
        let command = match serde_json::from_str::<Command>(&json) {
            Ok(c) => c,
            Err(e) => {
                if debug {
                    eprintln!("{json}\n  -> not a command: {e}");
                }
                continue;
            }
        };
        let out = send(&mut st.h, command);
        if debug {
            if let Some(ServerMessage::Reply(r)) = out.last() {
                eprintln!("{json}\n  -> {:?}", r.result);
            }
        }
        st.harvest(&out);
        check_document(&st.h, line);
    }
    check_undo_all(&mut st, &seeded, &seed_steps);
});

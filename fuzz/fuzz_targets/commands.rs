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
//!   `v` note expressions), `"@new"` a fresh id, `"@proj"` the open project's, so mutations
//!   keep hitting real entities;
//! - `tick` (or `tick <n>`, at most 50): the clock moves 20 ms and the controller ticks (media
//!   and render jobs, autosave);
//! - empty or `#...`: ignored.
//!
//! Checked after every line: exactly one reply per command, the fake engine's invariants (no
//! graph references a dead node, no live node destroyed twice), and the open document saves and
//! loads back.
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
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::*;

const SR: u32 = 48_000;
const MAX_LINES: usize = 64;

fn setup() -> Harness {
    let mut lib = MemoryLibrary::new();
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

/// `line` with its `"@..."` placeholders replaced (unknown ones are left as they are).
fn substitute(h: &mut Harness, line: &str) -> String {
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

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let mut h = setup();
    if std::env::var_os("FUZZ_COMMANDS_DEBUG").is_some() {
        let p = h.ctl.project().unwrap();
        let tracks: Vec<_> = p.tracks.values().map(|t| format!("{:?}", t.kind)).collect();
        let devices: Vec<_> = p
            .devices
            .values()
            .map(|d| format!("{:?}", d.kind))
            .collect();
        eprintln!("tracks {tracks:?}\ndevices {devices:?}");
    }
    for line in text.lines().take(MAX_LINES) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(n) = line.strip_prefix("tick") {
            let n = n.trim().parse::<usize>().unwrap_or(1).min(50);
            for _ in 0..n {
                h.advance(20);
                h.tick();
            }
            continue;
        }
        let json = substitute(&mut h, line);
        let debug = std::env::var_os("FUZZ_COMMANDS_DEBUG").is_some();
        let command = match serde_json::from_str::<Command>(&json) {
            Ok(c) => c,
            Err(e) => {
                if debug {
                    eprintln!("{json}\n  -> not a command: {e}");
                }
                continue;
            }
        };
        let out = h.send(command);
        if debug {
            if let Some(ServerMessage::Reply(r)) = out.last() {
                eprintln!("{json}\n  -> {:?}", r.result);
            }
        }
        check_document(&h, line);
    }
});

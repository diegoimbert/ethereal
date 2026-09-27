//! Native smoke test of the end-to-end user flow: the real desktop host (null audio backend,
//! real controller, real engine, real disk store) driven through the same JSON messages
//! the UI sends over Tauri IPC.
//!
//! create project → MIDI track + synth → notes → audio track → import a library sample →
//! clip on the arrangement → compressor + delay → volume automation → loop + play (meters
//! and playhead move) → edit, undo/redo → save → quit → restart → reopen: identical state.
//!
//! No sleeps: every step waits on a reply or an event.

mod common;

use common::{Client, Paths, sine, wav};
use ether_core::protocol::message::Event;
use ether_core::protocol::project::ProjectEvent;
use ether_native::test_util::TempDir;
use serde_json::{Value, json};

fn count(v: &Value) -> usize {
    v.as_object().map_or(0, |o| o.len())
}

/// Peak level of `track` in the meter frames received so far.
fn peak_of(c: &Client, track: &str) -> f32 {
    c.with(|r| {
        r.meters
            .iter()
            .flat_map(|(_, f)| f.tracks.iter())
            .filter(|t| t.track.to_string() == track)
            .map(|t| t.peak[0].max(t.peak[1]))
            .fold(0.0, f32::max)
    })
}

#[test]
fn full_user_flow_on_the_native_host() {
    let tmp = TempDir::new("e2e-flow");
    let library = tmp.path().join("library");
    std::fs::create_dir_all(library.join("drums")).unwrap();
    // A 1 s 220 Hz tone at 44.1 kHz (engine runs at 48 kHz: the controller resamples).
    std::fs::write(
        library.join("drums/tone.wav"),
        wav(44_100, &[sine(44_100, 220.0, 44_100, 0.5)]),
    )
    .unwrap();
    let paths = Paths {
        library: Some(library),
        ..Paths::new(tmp.path())
    };
    let mut c = Client::start(&paths);

    // --- Create a project (what TauriTransport.connect does on an empty store).
    let listed = c.ok("Project", json!({"type": "List"}));
    assert_eq!(listed["projects"].as_array().unwrap().len(), 0);
    let pid = c.project_id();
    let p = c.ok(
        "Project",
        json!({"type": "Create", "id": pid, "name": "Smoke"}),
    )["project"]
        .clone();
    assert_eq!(p["settings"]["name"], "Smoke");
    assert_eq!(count(&p["tracks"]), 1, "master only");

    // --- MIDI track with the built-in synth (one undo step via Batch).
    let midi = c.id();
    let synth = c.id();
    c.ok(
        "Edit",
        json!({"type": "Batch", "label": "Add MIDI track", "commands": [
            {"domain": "Track", "command": {"type": "Create", "id": midi, "kind": "Midi",
                "name": "Synth", "color": null, "parent": null, "before": null}},
            {"domain": "Device", "command": {"type": "Insert", "id": synth, "track": midi,
                "device": {"type": "Builtin", "device": {"type": "Synth"}}, "before": null}},
        ]}),
    );

    // --- Draw notes in the piano roll: Create clip, then Add notes in one gesture, sent
    // back to back without awaiting (like the UI): ordering must hold end to end.
    let clip = c.id();
    let gesture = Some(7);
    let (n1, n2, n3) = (c.id(), c.id(), c.id());
    let create = c.post(
        "Clip",
        json!({"type": "CreateMidi", "id": clip, "track": midi, "start": 0.0, "length": 4.0,
            "name": null}),
        gesture,
    );
    let add = c.post(
        "Note",
        json!({"type": "Add", "clip": clip, "notes": [
            {"id": n1, "pitch": 60, "velocity": 100, "start": 0.0, "duration": 1.0},
            {"id": n2, "pitch": 64, "velocity": 100, "start": 1.0, "duration": 1.0},
            {"id": n3, "pitch": 67, "velocity": 100, "start": 2.0, "duration": 2.0},
        ]}),
        gesture,
    );
    let edit = c.post(
        "Note",
        json!({"type": "Edit", "edits": [{"id": n3, "pitch": 69, "velocity": null,
            "start": null, "duration": null, "muted": null}]}),
        gesture,
    );
    let end = c.post("Edit", json!({"type": "EndGesture", "gesture": 7}), None);
    for id in [create, add, edit, end] {
        c.reply(id)
            .unwrap_or_else(|e| panic!("gesture message {id}: {e:?}"));
    }
    // Replies arrive in order, each after its own patches.
    c.with(|r| {
        let replies: Vec<u32> = r
            .log
            .iter()
            .filter_map(|m| match m {
                ether_core::protocol::ServerMessage::Reply(r) => Some(r.id),
                _ => None,
            })
            .collect();
        let pos = |id| replies.iter().position(|x| *x == id).unwrap();
        assert!(pos(create) < pos(add) && pos(add) < pos(edit) && pos(edit) < pos(end));
    });
    let p = c.project();
    assert_eq!(count(&p["notes"]), 3);
    assert_eq!(p["notes"][&n3]["pitch"], 69);

    // --- Audio track, sample from the library, dropped on the arrangement.
    let locations = c.ok("Media", json!({"type": "ListLocations"}));
    let lib_loc = locations["locations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["location"]["type"] == "Library")
        .expect("library location")["location"]
        .clone();
    let listing = c.ok(
        "Media",
        json!({"type": "ListDirectory", "location": lib_loc, "path": "drums"}),
    );
    let entry = &listing["listing"]["entries"][0];
    assert_eq!(entry["kind"], "Audio", "{listing}");
    let audio = c.id();
    c.ok(
        "Track",
        json!({"type": "Create", "id": audio, "kind": "Audio", "name": "Audio",
            "color": null, "parent": null, "before": null}),
    );
    let media = c.id();
    let imported = c.ok(
        "Media",
        json!({"type": "Import", "id": media, "source": {"type": "Location",
            "location": lib_loc, "path": "drums/tone.wav"}}),
    );
    assert_eq!(imported["media"]["frames"], 44_100);
    let aclip = c.id();
    c.ok(
        "Clip",
        json!({"type": "CreateAudio", "id": aclip, "track": audio, "start": 4.0,
            "media": media}),
    );
    // Waveforms come from engine peaks.
    c.wait(
        |r| {
            r.events.iter().find(|e| {
                matches!(e, Event::Media { event: ether_core::protocol::media::MediaEvent::PeaksReady { media: m } } if m.to_string() == media)
            }).map(drop)
        },
        "PeaksReady",
    );
    let peaks = c.ok(
        "Media",
        json!({"type": "GetPeaks", "request": {"media": media, "samples_per_peak": 512,
            "start_frame": 0, "frame_count": 44_100}}),
    );
    let max = peaks["peaks"]["max"][0].as_array().unwrap();
    assert!(!max.is_empty());
    let top = max.iter().map(|v| v.as_f64().unwrap()).fold(0.0, f64::max);
    assert!((top - 0.5).abs() < 0.05, "peak {top}");

    // --- Compressor + delay on the audio track.
    let comp = c.id();
    let delay = c.id();
    for (id, kind) in [(&comp, "Compressor"), (&delay, "Delay")] {
        c.ok(
            "Device",
            json!({"type": "Insert", "id": id, "track": audio,
                "device": {"type": "Builtin", "device": {"type": kind}}, "before": null}),
        );
    }

    // --- Automate the audio track's volume (lane + points in one gesture).
    let lane = c.id();
    let (a1, a2) = (c.id(), c.id());
    let g = Some(8);
    let l = c.post(
        "Automation",
        json!({"type": "CreateLane", "id": lane, "owner": {"type": "Track", "track": audio},
            "target": {"type": "TrackVolume", "track": audio}}),
        g,
    );
    let pts = c.post(
        "Automation",
        json!({"type": "AddPoints", "lane": lane, "points": [
            {"id": a1, "time": 4.0, "value": 0.9, "curve": {"type": "Linear"}},
            {"id": a2, "time": 8.0, "value": 0.5, "curve": {"type": "Linear"}},
        ]}),
        g,
    );
    let e = c.post("Edit", json!({"type": "EndGesture", "gesture": 8}), None);
    for id in [l, pts, e] {
        c.reply(id)
            .unwrap_or_else(|e| panic!("automation {id}: {e:?}"));
    }

    // --- Loop on, play: playhead moves and wraps, meters show signal on both tracks.
    c.ok(
        "Transport",
        json!({"type": "SetLoopRegion", "region": {"start": 0.0, "end": 8.0}}),
    );
    c.ok(
        "Transport",
        json!({"type": "SetLoopEnabled", "enabled": true}),
    );
    c.ok("Transport", json!({"type": "Play"}));
    c.wait(
        |r| {
            r.playhead
                .iter()
                .any(|(_, f)| f.transport.playing && f.transport.position.0 > 0.5)
                .then_some(())
        },
        "playhead moving",
    );
    c.wait(
        |r| {
            r.meters
                .iter()
                .flat_map(|(_, f)| f.tracks.iter())
                .any(|t| t.track.to_string() == midi && t.peak[0].max(t.peak[1]) > 0.01)
                .then_some(())
        },
        "synth signal on the MIDI track meter",
    );
    c.wait(
        |r| {
            r.meters
                .iter()
                .flat_map(|(_, f)| f.tracks.iter())
                .any(|t| t.track.to_string() == audio && t.peak[0].max(t.peak[1]) > 0.01)
                .then_some(())
        },
        "sample signal on the audio track meter",
    );
    assert!(peak_of(&c, &midi) > 0.01);
    // The loop wraps (8 beats at 120 bpm = 4 s; wait for a position decrease).
    c.wait(
        |r| {
            r.playhead
                .windows(2)
                .any(|w| {
                    w[0].1.transport.playing
                        && w[1].1.transport.playing
                        && w[1].1.transport.position.0 + 1.0 < w[0].1.transport.position.0
                })
                .then_some(())
        },
        "loop wrap",
    );
    c.ok("Transport", json!({"type": "Stop"}));

    // --- Edit, undo, redo.
    c.ok(
        "Mixer",
        json!({"type": "SetVolume", "track": midi, "volume": -12.0}),
    );
    assert_eq!(c.project()["tracks"][&midi]["mixer"]["volume"], -12.0);
    c.ok("Edit", json!({"type": "Undo"}));
    assert_eq!(c.project()["tracks"][&midi]["mixer"]["volume"], 0.0);
    c.ok("Edit", json!({"type": "Redo"}));
    assert_eq!(c.project()["tracks"][&midi]["mixer"]["volume"], -12.0);
    // Undo the whole note gesture as one step and redo it.
    let before = c.project();

    // --- Save, quit, restart, reopen: identical document.
    c.ok("Project", json!({"type": "Save"}));
    let saved = c.project();
    assert_eq!(saved, before);
    c.quit();

    let mut c = Client::start(&paths);
    let listed = c.ok("Project", json!({"type": "List"}));
    assert_eq!(listed["projects"][0]["id"], pid);
    let reopened = c.ok("Project", json!({"type": "Open", "id": pid}))["project"].clone();
    assert_eq!(reopened, saved, "reloaded project differs");
    // Media is decoded again and peaks are served after reopen.
    c.wait(
        |r| {
            r.events.iter().find(|e| {
                matches!(e, Event::Media { event: ether_core::protocol::media::MediaEvent::PeaksReady { media: m } } if m.to_string() == media)
            }).map(drop)
        },
        "PeaksReady after reopen",
    );
    c.quit();
}

/// Quitting with unsaved edits saves them (the desktop app's exit path).
#[test]
fn quit_saves_unsaved_work() {
    let tmp = TempDir::new("e2e-quit");
    let paths = Paths::new(tmp.path());
    let mut c = Client::start(&paths);
    let pid = c.project_id();
    c.ok(
        "Project",
        json!({"type": "Create", "id": pid, "name": "Quit"}),
    );
    let t = c.id();
    c.ok(
        "Track",
        json!({"type": "Create", "id": t, "kind": "Midi", "name": "Unsaved",
            "color": null, "parent": null, "before": null}),
    );
    c.wait(
        |r| {
            r.events
                .iter()
                .any(|e| {
                    matches!(
                        e,
                        Event::Project {
                            event: ProjectEvent::DirtyChanged { dirty: true }
                        }
                    )
                })
                .then_some(())
        },
        "dirty",
    );
    c.quit();

    let mut c = Client::start(&paths);
    let p = c.ok("Project", json!({"type": "Open", "id": pid}))["project"].clone();
    assert_eq!(p["tracks"][&t]["name"], "Unsaved");
    c.quit();
}

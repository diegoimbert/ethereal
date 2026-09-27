//! Native e2e of warping: the real desktop host (null audio backend, real controller,
//! engine with the Signalsmith stretcher factory) driven by the JSON messages the Warp
//! editor sends.
//!
//! import a loop → audio clip → warp off/on (BPM stub pins markers) → add + drag a marker
//! (one gesture, one undo) → Complex + transpose → play: signal on the track meter →
//! locate mid-clip: signal again → save → reopen: markers identical.

mod common;

use common::{Client, Paths, sine, wav};
use serde_json::{Value, json};

fn markers_of(p: &Value, clip: &str) -> Vec<(f64, f64)> {
    let mut v: Vec<(f64, f64)> = p["warp_markers"]
        .as_object()
        .unwrap()
        .values()
        .filter(|m| m["clip"] == clip)
        .map(|m| (m["beat"].as_f64().unwrap(), m["source"].as_f64().unwrap()))
        .collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    v
}

fn marker_id_at(p: &Value, clip: &str, beat: f64) -> String {
    p["warp_markers"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(_, m)| m["clip"] == clip && (m["beat"].as_f64().unwrap() - beat).abs() < 1e-9)
        .map(|(id, _)| id.clone())
        .expect("marker")
}

/// Wait for a meter frame of `track` above `level` received after `after` frames.
fn wait_signal(c: &Client, track: &str, after: usize, what: &str) {
    c.wait(
        |r| {
            r.meters
                .iter()
                .skip(after)
                .flat_map(|(_, f)| f.tracks.iter())
                .any(|t| t.track.to_string() == track && t.peak[0].max(t.peak[1]) > 0.05)
                .then_some(())
        },
        what,
    );
}

#[test]
fn warp_markers_and_stretching_on_the_native_host() {
    let tmp = ether_native::test_util::TempDir::new("e2e-warp");
    let library = tmp.path().join("library");
    std::fs::create_dir_all(&library).unwrap();
    // 4 s at 48 kHz: the BPM stub reads it as 2 bars at 120 BPM.
    std::fs::write(
        library.join("loop.wav"),
        wav(48_000, &[sine(48_000, 220.0, 4 * 48_000, 0.5)]),
    )
    .unwrap();
    let paths = Paths {
        library: Some(library),
        ..Paths::new(tmp.path())
    };
    let mut c = Client::start(&paths);
    let pid = c.project_id();
    c.ok(
        "Project",
        json!({"type": "Create", "id": pid, "name": "Warp"}),
    );
    let track = c.id();
    c.ok(
        "Track",
        json!({"type": "Create", "id": track, "kind": "Audio", "name": "Loop",
            "color": null, "parent": null, "before": null}),
    );
    let locations = c.ok("Media", json!({"type": "ListLocations"}));
    let lib = locations["locations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["location"]["type"] == "Library")
        .unwrap()["location"]
        .clone();
    let media = c.id();
    c.ok(
        "Media",
        json!({"type": "Import", "id": media, "source": {"type": "Location",
            "location": lib, "path": "loop.wav"}}),
    );
    let clip = c.id();
    c.ok(
        "Clip",
        json!({"type": "CreateAudio", "id": clip, "track": track, "start": 0.0,
            "media": media}),
    );
    assert_eq!(
        c.ok("Warp", json!({"type": "DetectTempo", "clip": clip}))["bpm"],
        120.0
    );

    // --- Warp off, then on: the BPM stub pins the media start and end.
    let warp = |enabled: bool, mode: &str| {
        json!({"type": "SetWarp", "clip": clip,
            "warp": {"enabled": enabled, "mode": mode, "source_bpm": null}})
    };
    c.ok("Warp", warp(false, "Repitch"));
    c.ok("Warp", warp(true, "Repitch"));
    let p = c.project();
    assert_eq!(markers_of(&p, &clip), vec![(0.0, 0.0), (8.0, 4.0)]);
    assert_eq!(
        p["clips"][&clip]["content"]["warp"]["source_bpm"],
        120.0,
        "{}",
        p["clips"][&clip]
    );

    // --- Add a marker, drag it in one gesture, undo it in one step.
    let m = c.id();
    c.ok(
        "Warp",
        json!({"type": "AddMarker", "id": m, "clip": clip, "beat": 4.0, "source": 2.0}),
    );
    let g = Some(11);
    let ids: Vec<u32> = [4.25, 4.5, 5.0]
        .iter()
        .map(|b| {
            c.post(
                "Warp",
                json!({"type": "MoveMarker", "id": m, "beat": b, "source": 2.0}),
                g,
            )
        })
        .collect();
    let end = c.post("Edit", json!({"type": "EndGesture", "gesture": 11}), None);
    for id in ids.into_iter().chain([end]) {
        c.reply(id).unwrap_or_else(|e| panic!("move {id}: {e:?}"));
    }
    assert_eq!(
        markers_of(&c.project(), &clip),
        vec![(0.0, 0.0), (5.0, 2.0), (8.0, 4.0)]
    );
    c.ok("Edit", json!({"type": "Undo"}));
    assert_eq!(
        markers_of(&c.project(), &clip),
        vec![(0.0, 0.0), (4.0, 2.0), (8.0, 4.0)]
    );
    c.ok("Edit", json!({"type": "Redo"}));
    let end_marker = marker_id_at(&c.project(), &clip, 8.0);
    c.ok("Warp", json!({"type": "RemoveMarker", "id": end_marker}));

    // --- Complex + transpose, play through the stretcher.
    c.ok("Warp", warp(true, "Complex"));
    c.ok(
        "Clip",
        json!({"type": "SetTranspose", "id": clip, "semitones": 5.0}),
    );
    let p = c.project();
    assert_eq!(p["clips"][&clip]["content"]["warp"]["mode"], "Complex");
    assert_eq!(p["clips"][&clip]["content"]["transpose"], 5.0);
    c.ok("Transport", json!({"type": "Play"}));
    wait_signal(&c, &track, 0, "stretched signal on the track meter");
    // Locate mid-clip: sound resumes (the stretcher seeks with pre-roll).
    c.ok("Transport", json!({"type": "Stop"}));
    let frames = c.with(|r| r.meters.len());
    c.ok(
        "Transport",
        json!({"type": "Locate", "position": 6.0}),
    );
    c.ok("Transport", json!({"type": "Play"}));
    wait_signal(&c, &track, frames, "signal after locate");
    c.ok("Transport", json!({"type": "Stop"}));

    // --- Save, reopen: markers and warp settings persist.
    c.ok("Project", json!({"type": "Save"}));
    let saved = c.project();
    c.quit();
    let mut c = Client::start(&paths);
    let reopened = c.ok("Project", json!({"type": "Open", "id": pid}))["project"].clone();
    assert_eq!(markers_of(&reopened, &clip), markers_of(&saved, &clip));
    assert_eq!(
        reopened["clips"][&clip]["content"],
        saved["clips"][&clip]["content"]
    );
    c.quit();
}

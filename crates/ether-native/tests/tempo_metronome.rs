//! Integration test (`tempo-metronome`): tempo map and metronome commands through the full
//! native host on the `null` audio backend. The edited tempo map drives the real engine
//! (the playhead's BPM follows it) while the metronome clicks, and each edit is one undo
//! step.

mod common;

use common::{Client, Paths};
use ether_native::test_util::TempDir;
use serde_json::json;

#[test]
fn tempo_map_and_metronome_drive_the_native_engine() {
    let tmp = TempDir::new("tempo-metronome");
    let mut c = Client::start(&Paths::new(tmp.path()));
    let pid = c.project_id();
    c.ok(
        "Project",
        json!({"type": "Create", "id": pid, "name": "Tempo"}),
    );

    // 120 BPM until beat 1, then 240 BPM; 3/4 from beat 3.
    let fast = c.id();
    c.ok(
        "Tempo",
        json!({"type": "AddTempoPoint", "id": fast, "time": 1.0, "bpm": 240.0, "curve": "Step"}),
    );
    let sig = c.id();
    c.ok(
        "Tempo",
        json!({"type": "AddTimeSignature", "id": sig, "time": 4.0,
               "signature": {"numerator": 3, "denominator": 4}}),
    );
    c.ok(
        "Tempo",
        json!({"type": "SetMetronomeSettings", "volume": -12.0, "accent": false, "sound": "Beep"}),
    );
    c.ok(
        "Transport",
        json!({"type": "SetMetronome", "enabled": true}),
    );
    let settings = c.project()["settings"].clone();
    assert_eq!(settings["metronome"], true);
    assert_eq!(settings["metronome_volume"], -12.0);
    assert_eq!(settings["metronome_sound"], "Beep");

    // Invalid edits are rejected (a second change at the same position; mid-bar changes
    // are allowed, CONTRACTS.md §11.3).
    let bad = c.id();
    let id = c.post(
        "Tempo",
        json!({"type": "AddTimeSignature", "id": bad, "time": 4.0,
               "signature": {"numerator": 7, "denominator": 8}}),
        None,
    );
    assert_eq!(c.reply(id).unwrap_err().0, "InvalidArgument");

    c.ok("Transport", json!({"type": "Play"}));
    let bpm = c.wait(
        |r| {
            r.playhead
                .last()
                .filter(|(_, f)| f.transport.playing && f.transport.position.0 > 2.0)
                .map(|(_, f)| f.transport.bpm)
        },
        "playback past the tempo change",
    );
    assert!((bpm - 240.0).abs() < 1e-9, "bpm {bpm}");
    c.ok("Transport", json!({"type": "Stop"}));

    // One undo step per edit: undo the metronome switch, settings, signature, tempo point.
    for _ in 0..4 {
        c.ok("Edit", json!({"type": "Undo"}));
    }
    let p = c.project();
    assert_eq!(p["tempo_points"].as_object().unwrap().len(), 1);
    assert_eq!(p["time_signatures"].as_object().unwrap().len(), 1);
    assert_eq!(p["settings"]["metronome"], false);
    assert_eq!(p["settings"]["metronome_sound"], "Classic");
    c.quit();
}

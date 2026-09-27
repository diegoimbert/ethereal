//! Integration test: the full native host on the `null` audio backend (no device) with the
//! real controller, checking that the engine runs on the audio thread and that playhead
//! (~60 Hz) and meter (~30 Hz) streams, replies and host-handled commands reach the
//! subscriber.

mod common;

use std::time::Duration;

use common::{Client, Paths};
use ether_native::audio::AudioBackendKind;
use ether_native::test_util::TempDir;
use serde_json::json;

#[test]
fn null_backend_streams_playhead_and_meters() {
    let tmp = TempDir::new("null-host");
    let mut c = Client::start(&Paths::new(tmp.path()));
    assert_eq!(c.host().audio_info().backend, AudioBackendKind::Null);

    // No project yet: document commands fail cleanly.
    let id = c.post("Project", json!({"type": "Get"}), None);
    assert!(c.reply(id).is_err());

    let pid = c.project_id();
    let project = c.ok(
        "Project",
        json!({"type": "Create", "id": pid, "name": "Null"}),
    )["project"]
        .clone();
    let master = project["tracks"]
        .as_object()
        .unwrap()
        .values()
        .find(|t| t["kind"] == "Master")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    c.ok("Transport", json!({"type": "Play"}));
    // ~1 beat at the default 120 bpm is 500 ms of audio.
    c.wait(
        |r| {
            r.playhead
                .last()
                .filter(|(_, f)| f.transport.playing && f.transport.position.0 > 1.0)
                .map(drop)
        },
        "one beat of playback",
    );
    c.with(|r| {
        let playing: Vec<_> = r
            .playhead
            .iter()
            .filter(|(_, p)| p.transport.playing)
            .collect();
        // Playhead at ~60 Hz: ~30 frames per 500 ms (loose bounds for loaded machines).
        let span = playing.last().unwrap().0 - playing.first().unwrap().0;
        let rate = playing.len() as f64 / span.as_secs_f64().max(1e-3);
        assert!((10.0..=120.0).contains(&rate), "playhead rate {rate}");
        // Positions are monotonic while playing (no loop).
        assert!(
            playing
                .windows(2)
                .all(|w| w[1].1.transport.position.0 >= w[0].1.transport.position.0)
        );
    });
    c.wait(
        |r| (r.meters.len() >= 5).then_some(()),
        "at least five meter frames",
    );
    c.with(|r| {
        // Meters coalesced to ~30 Hz, carrying the master track.
        assert!(
            r.meters
                .iter()
                .all(|(_, m)| m.tracks.iter().any(|t| t.track.to_string() == master))
        );
        let min_gap = r
            .meters
            .windows(2)
            .map(|w| w[1].0.duration_since(w[0].0))
            .min()
            .unwrap();
        assert!(min_gap >= Duration::from_millis(30), "{min_gap:?}");
    });

    // Host-handled commands.
    let status = c.ok("Engine", json!({"type": "GetStatus"}))["status"].clone();
    assert_eq!(status["running"], true);
    assert_eq!(status["backend"], "null");
    assert_eq!(status["sample_rate"], 48_000);
    assert_eq!(status["instance"], "test");
    let plugins = c.ok("Plugin", json!({"type": "List"}));
    assert_eq!(plugins["type"], "Plugins");
    c.ok("Transport", json!({"type": "Stop"}));

    // Switching the audio config moves the engine to a new backend (offline → null).
    let status = c.ok(
        "Engine",
        json!({"type": "SetAudioConfig", "config": {"backend": "offline", "host": null,
            "output_device": null, "input_device": null, "sample_rate": null,
            "buffer_size": 128}}),
    )["status"]
        .clone();
    assert_eq!(status["backend"], "offline");
    assert!(tmp.path().join("config/audio.json").is_file());
    c.quit();
}

#[test]
fn json_messages_and_bad_input() {
    let tmp = TempDir::new("null-host-json");
    let c = Client::start(&Paths::new(tmp.path()));
    c.host()
        .send_json(
            r#"{"id":9,"gesture":null,"command":{"domain":"Transport","command":{"type":"Play"}}}"#,
        )
        .unwrap();
    // Replies Ok or InvalidState (no project), but always exactly one reply.
    let _ = c.reply(9);
    assert!(c.host().send_json("{not json").is_err());
    c.quit();
}

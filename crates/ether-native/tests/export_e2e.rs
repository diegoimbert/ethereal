//! Native export end to end: the real host (null audio, real controller, disk store)
//! renders a mix and stems while "playing", and writes them under `<project>/exports/`.

mod common;

use common::Client;
use common::Paths;
use ether_core::protocol::export::{ExportEvent, ExportResult};
use ether_core::protocol::message::Event;
use ether_native::test_util::TempDir;
use serde_json::{Value, json};

fn done(c: &Client, job: &str) -> ExportResult {
    c.wait(
        |r| {
            r.events.iter().find_map(|e| match e {
                Event::Export {
                    event: ExportEvent::Done { job: j, result },
                } if j == job => Some(result.clone()),
                Event::Export {
                    event: ExportEvent::Failed { job: j, message },
                } if j == job => panic!("export failed: {message}"),
                _ => None,
            })
        },
        "export done",
    )
}

#[test]
fn exports_are_written_into_the_project_folder() {
    let tmp = TempDir::new("export-e2e");
    let paths = Paths::new(tmp.path());
    let mut c = Client::start(&paths);
    let pid = c.project_id();
    c.ok(
        "Project",
        json!({"type": "Create", "id": pid, "name": "Bounce"}),
    );
    let track = c.id();
    let synth = c.id();
    let clip = c.id();
    c.ok(
        "Edit",
        json!({"type": "Batch", "label": "setup", "commands": [
            {"domain": "Track", "command": {"type": "Create", "id": track, "kind": "Midi",
                "name": "Keys", "color": null, "parent": null, "before": null}},
            {"domain": "Device", "command": {"type": "Insert", "id": synth, "track": track,
                "device": {"type": "Builtin", "device": {"type": "Synth"}}, "before": null}},
            {"domain": "Clip", "command": {"type": "CreateMidi", "id": clip, "track": track,
                "start": 0.0, "length": 4.0, "name": null}},
        ]}),
    );
    let note = c.id();
    c.ok(
        "Note",
        json!({"type": "Add", "clip": clip, "notes": [
            {"id": note, "pitch": 60, "velocity": 0.9, "start": 0.0, "duration": 2.0}]}),
    );
    // Playback keeps running while exporting (independent offline engine).
    c.ok("Transport", json!({"type": "Play"}));

    let request = |mode: Value, container: &str, depth: &str| {
        json!({
            "range": {"type": "Project"},
            "format": {"container": container, "bit_depth": depth, "sample_rate": null},
            "mode": mode,
            "normalize": true,
            "tail_seconds": 0.5,
            "name": null,
        })
    };
    let v = c.ok(
        "Export",
        json!({"type": "Render", "job": "mix", "request": request(json!({"type": "Mix"}), "Wav", "Int24")}),
    );
    assert_eq!(v["job"], "mix");
    let ExportResult::Files { files } = done(&c, "mix") else {
        panic!("native exports are files")
    };
    assert_eq!(files, vec!["exports/Bounce.wav".to_string()]);
    let path = paths.projects_root.join(&pid).join("exports/Bounce.wav");
    let bytes = std::fs::read(&path).expect("file on disk");
    let audio = ether_media::decode(&bytes, Some("wav")).unwrap();
    // 4 beats at 120 bpm + 0.5 s tail.
    assert_eq!(
        audio.frames(),
        (2.5 * audio.sample_rate as f64).round() as usize
    );
    let peak = audio
        .channels
        .iter()
        .flatten()
        .fold(0.0f32, |m, s| m.max(s.abs()));
    assert!((peak - 0.9886).abs() < 1e-3, "normalized: {peak}");

    c.ok(
        "Export",
        json!({"type": "Render", "job": "stems",
            "request": request(json!({"type": "Stems", "tracks": [track]}), "Flac", "Int16")}),
    );
    let ExportResult::Files { files } = done(&c, "stems") else {
        panic!()
    };
    assert_eq!(files, vec!["exports/Bounce - Keys.flac".to_string()]);
    let flac = std::fs::read(paths.projects_root.join(&pid).join(&files[0])).unwrap();
    assert!(ether_media::decode(&flac, Some("flac")).unwrap().frames() > 0);
    c.quit();
}

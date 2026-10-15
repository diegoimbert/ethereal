//! Recording end to end on the real native host (null backend, real controller/engine/disk
//! store), driven with the UI's JSON: a click played on one track goes out, comes back
//! through the loopback input with a known round-trip latency, and is recorded on an armed
//! track. After stop the take is one undoable clip whose click sits exactly where the
//! original plays on the timeline (latency compensation).

mod common;

use common::{Client, Paths, wav};
use ether_core::protocol::message::Event;
use ether_core::protocol::recording::RecordingEvent;
use ether_native::test_util::TempDir;
use serde_json::{Value, json};

const SR: f64 = 48_000.0;
/// Samples per beat at 120 bpm.
const SPB: f64 = SR / 2.0;
const CLICK_AT: usize = 100;

fn tracks_of(p: &Value) -> Vec<Value> {
    p["tracks"].as_object().unwrap().values().cloned().collect()
}

#[test]
fn record_a_loopback_click_with_latency_compensation() {
    let tmp = TempDir::new("rec-e2e");
    let library = tmp.path().join("library");
    std::fs::create_dir_all(&library).unwrap();
    let mut click = vec![0.0f32; 4_800];
    click[CLICK_AT] = 0.9;
    std::fs::write(library.join("click.wav"), wav(48_000, &[click])).unwrap();
    let paths = Paths {
        library: Some(library),
        ..Paths::new(tmp.path())
    };
    let mut c = Client::start(&paths);
    let pid = c.project_id();
    c.ok(
        "Project",
        json!({"type": "Create", "id": pid, "name": "Rec"}),
    );

    // The input device is host-handled engine config: a loopback with 2048 samples latency.
    let status = c.ok(
        "Engine",
        json!({"type": "SetAudioConfig", "config": {"backend": null, "host": null,
            "output_device": null, "input_device": "loopback:2048", "sample_rate": null,
            "buffer_size": null}}),
    );
    assert_eq!(status["status"]["input_latency"], 2048);
    let inputs = c.ok("Recording", json!({"type": "ListInputs"}));
    assert_eq!(inputs["inputs"]["audio"].as_array().unwrap().len(), 2);

    // Player track: the click at beat 2 (+100 samples), unwarped.
    let player = c.id();
    c.ok(
        "Track",
        json!({"type": "Create", "id": player, "kind": "Audio", "name": "Player",
            "color": null, "parent": null, "before": null}),
    );
    let media = c.id();
    c.ok(
        "Media",
        json!({"type": "Import", "id": media, "source": {"type": "Location",
            "location": {"type": "Library", "id": "lib"}, "path": "click.wav"}}),
    );
    let clip = c.id();
    c.ok(
        "Clip",
        json!({"type": "CreateAudio", "id": clip, "track": player, "start": 2.0, "media": media}),
    );
    c.ok(
        "Warp",
        json!({"type": "SetWarp", "clip": clip, "warp": {"enabled": false, "mode": "Complex",
            "source_bpm": null}}),
    );
    c.wait(
        |r| {
            r.events.iter().find(|e| {
                matches!(e, Event::Media { event: ether_core::protocol::media::MediaEvent::ImportProgress { media: m, progress } } if m.to_string() == media && *progress >= 1.0)
            }).map(drop)
        },
        "click loaded into the engine",
    );

    // Recording track: mono input 1, no monitoring (no feedback), armed.
    let rec = c.id();
    c.ok(
        "Track",
        json!({"type": "Create", "id": rec, "kind": "Audio", "name": "Mic",
            "color": null, "parent": null, "before": null}),
    );
    c.ok(
        "Recording",
        json!({"type": "SetInput", "track": rec, "input": {"type": "Audio", "first": 0, "count": 1}}),
    );
    c.ok(
        "Recording",
        json!({"type": "SetMonitor", "track": rec, "monitor": "Off"}),
    );
    c.ok(
        "Recording",
        json!({"type": "Arm", "track": rec, "armed": true, "exclusive": true}),
    );

    // Record from beat 1 (no count-in) past the click, then stop the transport.
    c.ok("Transport", json!({"type": "Locate", "position": 1.0}));
    c.ok(
        "Recording",
        json!({"type": "SetRecording", "enabled": true}),
    );
    c.wait(
        |r| {
            r.events
                .iter()
                .any(|e| {
                    matches!(
                        e,
                        Event::Recording {
                            event: RecordingEvent::Started { .. }
                        }
                    )
                })
                .then_some(())
        },
        "recording started",
    );
    c.wait(
        |r| {
            r.playhead
                .last()
                .filter(|(_, f)| f.transport.position.0 > 2.6)
                .map(drop)
        },
        "playback past the click",
    );
    c.ok("Transport", json!({"type": "Stop"}));
    let clips = c.wait(
        |r| {
            r.events.iter().find_map(|e| match e {
                Event::Recording {
                    event: RecordingEvent::Stopped { clips },
                } => Some(clips.clone()),
                _ => None,
            })
        },
        "recording stopped",
    );
    assert_eq!(clips.len(), 1, "one take on the armed track");

    let p = c.project();
    let take = p["clips"][clips[0].to_string()].clone();
    assert_eq!(take["track"], json!(rec));
    let start = take["start"].as_f64().unwrap();
    assert!(
        (start - 1.0).abs() < 1e-6,
        "take starts at the record position: {start}"
    );
    let m = p["media"][take["content"]["media"].as_str().unwrap()].clone();
    assert_eq!(m["sample_rate"], 48_000);
    let file = tmp
        .path()
        .join("projects")
        .join(&pid)
        .join(m["file"].as_str().unwrap());
    let bytes = std::fs::read(&file).expect("take written to the project's media folder");
    let samples: Vec<f32> = bytes[44..]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    assert_eq!(samples.len() as u64, m["frames"].as_u64().unwrap());
    let (peak, value) = samples
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .unwrap();
    assert!(*value > 0.5, "click recorded ({value})");
    let at = start + peak as f64 / SPB;
    let expected = 2.0 + CLICK_AT as f64 / SPB;
    assert!(
        ((at - expected) * SPB).abs() < 1.0,
        "recorded click at {at} beats, plays at {expected}"
    );

    // One undo step removes the take (clip + media).
    let before = tracks_of(&p).len();
    c.ok("Edit", json!({"type": "Undo"}));
    let p = c.project();
    assert!(p["clips"].get(clips[0].to_string()).is_none());
    assert_eq!(
        p["media"].as_object().unwrap().len(),
        1,
        "only the imported click"
    );
    assert_eq!(tracks_of(&p).len(), before);
    c.quit();
}

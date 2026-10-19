//! Native e2e of the browser preview: the real desktop host (null audio backend, real
//! controller and engine) driven by the JSON the browser sends. A short sample plays to its
//! natural end (`Finished`, reported by the engine by id), a long one is replaced
//! (`Replaced`, then the new one starts and finishes) and another is stopped (`Stopped`).
//! Every preview gets exactly one `PreviewEnded`.

mod common;

use common::{Client, Paths, Received, sine, wav};
use ether_core::protocol::Event;
use ether_core::protocol::media::{MediaEvent, MediaSource, PreviewEndReason};
use serde_json::{Value, json};

/// (path, `Some(reason)` for an end, `None` for a start), in order.
fn preview_events(r: &Received) -> Vec<(String, Option<PreviewEndReason>)> {
    r.events
        .iter()
        .filter_map(|e| match e {
            Event::Media {
                event: MediaEvent::PreviewStarted { source },
            } => Some((path(source), None)),
            Event::Media {
                event: MediaEvent::PreviewEnded { source, reason },
            } => Some((path(source), Some(*reason))),
            _ => None,
        })
        .collect()
}

fn path(source: &MediaSource) -> String {
    match source {
        MediaSource::Location { path, .. } => path.clone(),
        other => format!("{other:?}"),
    }
}

fn wait_events(c: &Client, n: usize, what: &str) -> Vec<(String, Option<PreviewEndReason>)> {
    c.wait(
        |r| {
            let evs = preview_events(r);
            (evs.len() >= n).then_some(evs)
        },
        what,
    )
}

#[test]
fn browser_preview_on_the_native_host() {
    let tmp = ether_native::test_util::TempDir::new("e2e-preview");
    let library = tmp.path().join("library");
    std::fs::create_dir_all(&library).unwrap();
    // 0.2 s at 44.1 kHz (resampled to the engine rate) and 20 s.
    std::fs::write(
        library.join("blip.wav"),
        wav(44_100, &[sine(44_100, 880.0, 8_820, 0.5)]),
    )
    .unwrap();
    std::fs::write(
        library.join("drone.wav"),
        wav(48_000, &[sine(48_000, 110.0, 20 * 48_000, 0.3)]),
    )
    .unwrap();
    let paths = Paths {
        library: Some(library),
        ..Paths::new(tmp.path())
    };
    let mut c = Client::start(&paths);
    // No project needed to audition library files.
    let locations = c.ok("Media", json!({"type": "ListLocations"}));
    let lib: Value = locations["locations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["location"]["type"] == "Library")
        .unwrap()["location"]
        .clone();
    let preview = |c: &mut Client, file: &str| {
        c.ok(
            "Media",
            json!({"type": "Preview", "source": {"type": "Location", "location": lib,
                "path": file}}),
        )
    };
    use PreviewEndReason::*;
    let s = |p: &str| (p.to_string(), None);
    let e = |p: &str, r: PreviewEndReason| (p.to_string(), Some(r));

    // --- A short sample plays to its natural end.
    preview(&mut c, "blip.wav");
    assert_eq!(
        wait_events(&c, 2, "blip to finish"),
        vec![s("blip.wav"), e("blip.wav", Finished)]
    );

    // --- A long one, replaced by the short one: Replaced, then the new one finishes.
    preview(&mut c, "drone.wav");
    wait_events(&c, 3, "drone to start");
    preview(&mut c, "blip.wav");
    assert_eq!(
        wait_events(&c, 6, "replacement to finish")[2..],
        [
            s("drone.wav"),
            e("drone.wav", Replaced),
            s("blip.wav"),
            e("blip.wav", Finished)
        ]
    );

    // --- Stop: Stopped, and nothing else arrives for it.
    preview(&mut c, "drone.wav");
    wait_events(&c, 7, "drone to start again");
    c.ok("Media", json!({"type": "StopPreview"}));
    let evs = wait_events(&c, 8, "drone to stop");
    assert_eq!(evs[6..], [s("drone.wav"), e("drone.wav", Stopped)]);
    // Stopping again is a no-op; a short while later still exactly one end per preview.
    c.ok("Media", json!({"type": "StopPreview"}));
    preview(&mut c, "blip.wav");
    let evs = wait_events(&c, 10, "last blip to finish");
    assert_eq!(evs.len(), 10);
    assert_eq!(evs[8..], [s("blip.wav"), e("blip.wav", Finished)]);
    c.quit();
}

//! Native end-to-end drum rack (`drum-rack`): the real host (null audio backend) driven by
//! the UI's JSON messages. A 2-pad kit (a sample dropped on a pad + a synth pad in one choke
//! group) plays through the rack; a sampler is sliced while playing (in-place slice
//! updates) and turned into a rack; everything survives save → quit → reopen.

mod common;

use common::{Client, Paths, wav};
use ether_core::protocol::media::MediaEvent;
use ether_core::protocol::message::Event;
use ether_native::test_util::TempDir;
use serde_json::{Value, json};

/// Decaying noise hits at 0, 0.25, 0.5 and 0.75 s (1 s at 48 kHz).
fn hits() -> Vec<f32> {
    let sr = 48_000usize;
    let mut seed = 0x2545_f491u32;
    let mut x = vec![0.0f32; sr];
    for h in 0..4 {
        for (k, v) in x[h * sr / 4..].iter_mut().enumerate() {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let n = seed as f32 / u32::MAX as f32 * 2.0 - 1.0;
            *v += 0.8 * (-(k as f32) / 1500.0).exp() * n;
        }
    }
    x
}

fn count(v: &Value) -> usize {
    v.as_object().map_or(0, |o| o.len())
}

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
fn drum_rack_kit_plays_slices_and_survives_reopen() {
    let tmp = TempDir::new("e2e-drum-rack");
    let library = tmp.path().join("library");
    std::fs::create_dir_all(&library).unwrap();
    std::fs::write(library.join("hits.wav"), wav(48_000, &[hits()])).unwrap();
    let paths = Paths {
        library: Some(library),
        ..Paths::new(tmp.path())
    };
    let mut c = Client::start(&paths);
    let pid = c.project_id();
    c.ok(
        "Project",
        json!({"type": "Create", "id": pid, "name": "Kit"}),
    );

    // --- MIDI track + drum rack.
    let (track, rack) = (c.id(), c.id());
    c.ok(
        "Edit",
        json!({"type": "Batch", "label": "Add drum rack", "commands": [
            {"domain": "Track", "command": {"type": "Create", "id": track, "kind": "Midi",
                "name": "Drums", "color": null, "parent": null, "before": null}},
            {"domain": "Device", "command": {"type": "Insert", "id": rack, "track": track,
                "device": {"type": "Builtin", "device": {"type": "DrumRack"}}, "before": null}},
        ]}),
    );

    // --- A sample dropped on pad C1, a synth on pad D1, both in choke group 1.
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
            "location": lib, "path": "hits.wav"}}),
    );
    let (kick, kick_dev, snare, synth) = (c.id(), c.id(), c.id(), c.id());
    c.ok(
        "DrumRack",
        json!({"type": "AddSamplePad", "pad": kick, "device": kick_dev, "rack": rack,
            "note": 36, "media": media}),
    );
    c.ok(
        "DrumRack",
        json!({"type": "AddPad", "id": snare, "rack": rack, "note": 38, "name": "Snare"}),
    );
    c.ok(
        "DrumRack",
        json!({"type": "InsertDevice", "id": synth, "pad": snare,
            "device": {"type": "Builtin", "device": {"type": "Synth"}}, "before": null}),
    );
    for pad in [&kick, &snare] {
        c.ok(
            "DrumRack",
            json!({"type": "SetChokeGroup", "id": pad, "group": 1}),
        );
    }
    let p = c.project();
    assert_eq!(count(&p["drum_pads"]), 2);
    assert_eq!(p["drum_pads"][&kick]["name"], "hits");

    // --- Notes on both pads, loop, play: the rack sounds.
    let clip = c.id();
    c.ok(
        "Clip",
        json!({"type": "CreateMidi", "id": clip, "track": track, "start": 0.0,
            "length": 2.0, "name": null}),
    );
    let (n1, n2) = (c.id(), c.id());
    c.ok(
        "Note",
        json!({"type": "Add", "clip": clip, "notes": [
            {"id": n1, "pitch": 36, "velocity": 100, "start": 0.0, "duration": 0.5},
            {"id": n2, "pitch": 38, "velocity": 100, "start": 1.0, "duration": 0.5},
        ]}),
    );
    c.wait(
        |r| {
            r.events
                .iter()
                .find(|e| matches!(e, Event::Media { event: MediaEvent::PeaksReady { media: m } } if m.to_string() == media))
                .map(drop)
        },
        "PeaksReady",
    );
    c.ok(
        "Transport",
        json!({"type": "SetLoopRegion", "region": {"start": 0.0, "end": 2.0}}),
    );
    c.ok(
        "Transport",
        json!({"type": "SetLoopEnabled", "enabled": true}),
    );
    c.ok("Transport", json!({"type": "Play"}));
    c.wait(
        |r| {
            r.meters
                .iter()
                .flat_map(|(_, f)| f.tracks.iter())
                .any(|t| t.track.to_string() == track && t.peak[0].max(t.peak[1]) > 0.01)
                .then_some(())
        },
        "drum rack signal on the track meter",
    );
    assert!(peak_of(&c, &track) > 0.01);

    // --- A sampler on another track, sliced while playing (in-place updates).
    let (keys, sampler) = (c.id(), c.id());
    c.ok(
        "Edit",
        json!({"type": "Batch", "label": "Add sampler", "commands": [
            {"domain": "Track", "command": {"type": "Create", "id": keys, "kind": "Midi",
                "name": "Slices", "color": null, "parent": null, "before": null}},
            {"domain": "Device", "command": {"type": "Insert", "id": sampler, "track": keys,
                "device": {"type": "Builtin", "device": {"type": "Sampler", "sample": media,
                "slices": {"enabled": false, "base_note": 36, "markers": []}}},
                "before": null}},
        ]}),
    );
    c.ok(
        "Slice",
        json!({"type": "Auto", "device": sampler,
            "mode": {"type": "Transients", "sensitivity": 0.5}}),
    );
    let markers = c.project()["devices"][&sampler]["kind"]["device"]["slices"]["markers"].clone();
    let markers: Vec<f64> = markers
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m.as_f64().unwrap())
        .collect();
    assert_eq!(markers.len(), 4, "{markers:?}");
    for (m, t) in markers.iter().zip([0.0, 0.25, 0.5, 0.75]) {
        assert!((m - t).abs() < 0.001, "{m} vs {t}");
    }
    c.ok(
        "Slice",
        json!({"type": "Move", "device": sampler, "index": 3, "position": 0.8}),
    );
    let (r2, ids) = (c.id(), (0..4).map(|_| (c.id(), c.id())).collect::<Vec<_>>());
    let pads: Vec<Value> = ids
        .iter()
        .map(|(p, d)| json!({"pad": p, "device": d}))
        .collect();
    c.ok(
        "Slice",
        json!({"type": "ToDrumRack", "device": sampler, "rack": r2, "pads": pads}),
    );
    assert_eq!(count(&c.project()["drum_pads"]), 6);
    c.ok("Transport", json!({"type": "Stop"}));

    // --- Save, quit, reopen: identical.
    c.ok("Project", json!({"type": "Save"}));
    let saved = c.project();
    c.quit();
    let mut c = Client::start(&paths);
    let reopened = c.ok("Project", json!({"type": "Open", "id": pid}))["project"].clone();
    assert_eq!(reopened, saved);
    c.quit();
}

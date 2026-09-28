//! Native e2e of `media-references` (CONTRACTS.md §12.9) on the real desktop host (null
//! audio backend, real disk store): OS files and library files are referenced in place; a
//! file moved while the app was closed is reported missing when the project opens; a
//! folder search finds and relinks it; "collect all" copies it into the project folder.

mod common;

use common::{Client, Paths, sine, wav};
use ether_core::protocol::message::Event;
use serde_json::{Value, json};

fn import(c: &mut Client, source: Value) -> Value {
    let id = c.id();
    c.ok(
        "Media",
        json!({"type": "Import", "id": id, "source": source}),
    )["media"]
        .clone()
}

fn missing_reported(r: &common::Received, media: &str) -> bool {
    r.events.iter().any(|e| {
        let v = serde_json::to_value(e).unwrap();
        v["type"] == "Media" && v["event"]["type"] == "Missing" && v["event"]["media"] == media
    })
}

#[test]
fn referenced_in_place_missing_relinked_collected() {
    let tmp = ether_native::test_util::TempDir::new("e2e-media-refs");
    let desktop = tmp.path().join("Desktop");
    let library = tmp.path().join("Library");
    std::fs::create_dir_all(&desktop).unwrap();
    std::fs::create_dir_all(library.join("Drums")).unwrap();
    let kick = wav(44_100, &[sine(44_100, 60.0, 4_410, 0.8)]);
    let snare = wav(44_100, &[sine(44_100, 200.0, 2_205, 0.5)]);
    let kick_path = desktop.join("Kick.wav");
    std::fs::write(&kick_path, &kick).unwrap();
    std::fs::write(library.join("Drums/Snare.wav"), &snare).unwrap();
    let mut paths = Paths::new(tmp.path());
    paths.library = Some(library.clone());

    let mut c = Client::start(&paths);
    let pid = c.project_id();
    c.ok(
        "Project",
        json!({"type": "Create", "id": pid, "name": "Refs"}),
    );
    let k = import(
        &mut c,
        json!({"type": "Path", "path": kick_path.to_str().unwrap()}),
    );
    assert_eq!(k["location"]["type"], "External");
    let s = import(
        &mut c,
        json!({"type": "Location", "location": {"type": "Library", "id": "lib"}, "path": "Drums/Snare.wav"}),
    );
    assert_eq!(
        s["location"],
        json!({"type": "External", "path": library.join("Drums/Snare.wav").to_str().unwrap()})
    );
    let project_dir = tmp.path().join("projects").join(&pid);
    for m in [&k, &s] {
        assert!(!project_dir.join(m["file"].as_str().unwrap()).exists());
    }
    c.ok("Project", json!({"type": "Save"}));
    c.quit();

    // Moved while the app was closed.
    let archive = tmp.path().join("Archive/2026");
    std::fs::create_dir_all(&archive).unwrap();
    std::fs::rename(&kick_path, archive.join("Kick.wav")).unwrap();
    let mut c = Client::start(&paths);
    c.ok("Project", json!({"type": "Open", "id": pid}));
    let kid = k["id"].as_str().unwrap().to_string();
    c.wait(|r| missing_reported(r, &kid).then_some(()), "missing kick");
    let missing = c.ok("MediaRef", json!({"type": "ListMissing"}));
    assert_eq!(missing["media"], json!([kid]));

    // Search in a folder: the same content is relinked automatically.
    let archive_root = tmp.path().join("Archive");
    c.ok(
        "MediaRef",
        json!({"type": "Search", "media": null, "folder": archive_root.to_str().unwrap()}),
    );
    c.wait(
        |r| {
            r.events.iter().find_map(|e| match e {
                Event::MediaRef { .. } => {
                    let v = serde_json::to_value(e).unwrap();
                    (v["event"]["type"] == "Resolved" && v["event"]["media"] == kid.as_str())
                        .then_some(())
                }
                _ => None,
            })
        },
        "kick resolved",
    );
    let p = c.project();
    assert_eq!(
        p["media"][&kid]["location"],
        json!({"type": "External", "path": archive.join("Kick.wav").to_str().unwrap()})
    );

    // Collect all: both files are copied into the project folder, then saved.
    c.ok("MediaRef", json!({"type": "CollectAll"}));
    let p = c.project();
    for m in [&k, &s] {
        let id = m["id"].as_str().unwrap();
        assert_eq!(p["media"][id]["location"], json!({"type": "Project"}));
    }
    assert_eq!(
        std::fs::read(project_dir.join(k["file"].as_str().unwrap())).unwrap(),
        kick
    );
    assert_eq!(
        std::fs::read(project_dir.join(s["file"].as_str().unwrap())).unwrap(),
        snare
    );
    let saved = std::fs::read_to_string(project_dir.join("project.ether")).unwrap();
    assert!(!saved.contains("External"), "saved after collecting");

    // Bad folders.
    let id = c.post(
        "MediaRef",
        json!({"type": "Search", "media": null, "folder": "relative/folder"}),
        None,
    );
    assert!(c.reply(id).is_err());
    c.quit();
}

#[test]
fn list_external_dir_lists_one_folder() {
    let tmp = ether_native::test_util::TempDir::new("media-refs-listdir");
    std::fs::create_dir_all(tmp.path().join("sub")).unwrap();
    std::fs::write(tmp.path().join("a.wav"), b"x").unwrap();
    std::fs::write(tmp.path().join(".hidden.wav"), b"x").unwrap();
    let root = tmp.path().to_str().unwrap();
    let entries = ether_native::store::list_external_dir(root).unwrap();
    assert_eq!(
        entries,
        vec![
            (
                tmp.path().join("a.wav").to_str().unwrap().to_string(),
                false
            ),
            (tmp.path().join("sub").to_str().unwrap().to_string(), true),
        ]
    );
    assert!(ether_native::store::list_external_dir("relative").is_err());
    assert!(
        ether_native::store::list_external_dir(tmp.path().join("a.wav").to_str().unwrap()).is_err()
    );
}

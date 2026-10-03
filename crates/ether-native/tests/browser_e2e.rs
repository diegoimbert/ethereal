//! Native e2e of the library index (`browser-v2`, CONTRACTS.md §12.8) on the real desktop
//! host (null audio backend, `DiskStore`): a user folder added by path (as the desktop folder
//! picker sends it) is indexed in the background and searchable; a dropped item imports as an
//! in-place reference (`media-references`); a synced preview starts; and user folders,
//! favourites and tags survive a restart.

mod common;

use common::{Client, Paths, sine, wav};
use ether_core::protocol::Event;
use ether_core::protocol::browser::BrowserEvent;
use ether_core::protocol::media::MediaEvent;
use serde_json::{Value, json};

fn query(c: &mut Client, text: &str) -> Value {
    c.ok(
        "Browser",
        json!({"type": "Query", "query": {
            "text": text, "kinds": [], "tags": [], "favourites_only": false, "roots": [],
            "folder": null, "device": null, "sort": "Relevance", "offset": 0, "limit": 50,
        }}),
    )["page"]
        .clone()
}

/// Wait until the index has `n` matches for `text` (the scan runs in the background).
fn wait_for(c: &mut Client, text: &str, n: u64) -> Value {
    for _ in 0..200 {
        let page = query(c, text);
        if page["total"].as_u64() == Some(n) {
            return page;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    panic!("index never had {n} items for {text:?}: {}", query(c, text));
}

#[test]
fn user_folder_index_import_preview_and_restart() {
    let tmp = ether_native::test_util::TempDir::new("e2e-browser");
    let folder = tmp.path().join("My Samples");
    std::fs::create_dir_all(folder.join("Loops")).unwrap();
    let beat = wav(48_000, &[sine(48_000, 110.0, 24_000, 0.5)]);
    std::fs::write(folder.join("Loops/Beat Am 100bpm.wav"), &beat).unwrap();
    std::fs::write(folder.join("Kick.wav"), &beat).unwrap();
    let paths = Paths::new(tmp.path());

    let mut c = Client::start(&paths);
    let pid = c.project_id();
    c.ok(
        "Project",
        json!({"type": "Create", "id": pid, "name": "Browse"}),
    );
    let path = folder.to_str().unwrap().to_string();
    let roots = c.ok("Browser", json!({"type": "AddFolder", "path": path}))["roots"].clone();
    let user_folder = roots
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "Folder")
        .expect("the folder is a root")
        .clone();
    assert_eq!(user_folder["name"], "My Samples");
    assert_eq!(user_folder["path"], path.as_str());
    let root = user_folder["id"].as_str().unwrap().to_string();

    let page = wait_for(&mut c, "beat", 1);
    let item = page["items"][0].clone();
    assert_eq!(item["id"], format!("{root}/Loops/Beat Am 100bpm.wav"));
    assert_eq!(item["meta"]["bpm"], 100.0);
    assert_eq!(item["meta"]["key"], "A minor");
    c.wait(
        |r| {
            r.events
                .iter()
                .any(|e| {
                    matches!(
                        e,
                        Event::Browser {
                            event: BrowserEvent::IndexChanged
                        }
                    )
                })
                .then_some(())
        },
        "IndexChanged",
    );

    // A drop imports the item's source: referenced in place, not copied.
    let id = c.id();
    let media = c.ok(
        "Media",
        json!({"type": "Import", "id": id, "source": item["source"].clone()}),
    )["media"]
        .clone();
    let abs = folder.join("Loops/Beat Am 100bpm.wav");
    assert_eq!(
        media["location"],
        json!({"type": "External", "path": abs.to_str().unwrap()})
    );

    // Tempo-synced preview plays.
    c.ok(
        "Browser",
        json!({"type": "Preview", "item": item["id"].clone(), "sync": true}),
    );
    c.wait(
        |r| {
            r.events
                .iter()
                .any(|e| {
                    matches!(
                        e,
                        Event::Media {
                            event: MediaEvent::PreviewStarted { .. }
                        }
                    )
                })
                .then_some(())
        },
        "PreviewStarted",
    );

    c.ok(
        "Browser",
        json!({"type": "SetFavourite", "item": item["id"].clone(), "favourite": true}),
    );
    c.ok(
        "Browser",
        json!({"type": "SetTags", "item": item["id"].clone(), "tags": ["Groove"]}),
    );
    c.quit();

    // Restart: the folder, favourite and tags are back.
    let mut c = Client::start(&paths);
    let roots = c.ok("Browser", json!({"type": "ListRoots"}))["roots"].clone();
    assert!(
        roots
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"] == root.as_str() && r["kind"] == "Folder"),
        "{roots}"
    );
    let page = wait_for(&mut c, "groove", 1);
    assert_eq!(page["items"][0]["favourite"], true);
    assert_eq!(page["items"][0]["tags"], json!(["groove"]));

    // Removing the folder drops its items (files untouched).
    c.ok("Browser", json!({"type": "RemoveFolder", "root": root}));
    assert_eq!(query(&mut c, "beat")["total"], 0);
    assert!(abs.exists());
    c.quit();
}

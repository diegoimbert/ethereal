//! A host whose UI is not on the engine machine (`HostOptions::os_paths` off: `ether-server`,
//! headless hosts) replies `Unsupported` to every command naming a raw engine-machine path,
//! as the protocol documents, while library files, uploads and imported folders keep
//! working. The desktop host (`os_paths` on) is covered by the other e2e tests.

use crate::common;

use common::{Client, Paths, sine, wav};
use serde_json::{Value, json};

fn unsupported(c: &mut Client, domain: &str, command: Value) {
    let id = c.post(domain, command.clone(), None);
    match c.reply(id) {
        Err((code, _)) if code == "Unsupported" => {}
        other => panic!("{domain} {command}: expected Unsupported, got {other:?}"),
    }
}

#[test]
fn engine_machine_paths_are_unsupported_without_os_paths() {
    let tmp = ether_native::test_util::TempDir::new("e2e-os-paths");
    let kick = wav(44_100, &[sine(44_100, 60.0, 4_410, 0.8)]);
    let lib = tmp.path().join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("kick.wav"), &kick).unwrap();
    // Files of the engine machine outside the host's own folders.
    let desktop = tmp.path().join("Desktop");
    std::fs::create_dir_all(&desktop).unwrap();
    std::fs::write(desktop.join("kick.wav"), &kick).unwrap();
    std::fs::write(desktop.join("song.mid"), b"MThd").unwrap();
    std::fs::write(desktop.join("song.ether"), b"PK").unwrap();
    let file = |name: &str| desktop.join(name).to_str().unwrap().to_string();
    let folder = desktop.to_str().unwrap().to_string();

    let mut paths = Paths::new(tmp.path());
    paths.library = Some(lib.clone());
    paths.os_paths = false;
    let mut c = Client::start(&paths);
    let pid = c.project_id();
    c.ok(
        "Project",
        json!({"type": "Create", "id": pid, "name": "Remote"}),
    );

    let path = |p: String| json!({"type": "Path", "path": p});
    let id = c.id();
    unsupported(
        &mut c,
        "Media",
        json!({"type": "Import", "id": id, "source": path(file("kick.wav"))}),
    );
    // A `..` path leaving the library is no library file.
    let escape = lib
        .join("../Desktop/kick.wav")
        .to_str()
        .unwrap()
        .to_string();
    let id = c.id();
    unsupported(
        &mut c,
        "Media",
        json!({"type": "Import", "id": id, "source": path(escape)}),
    );
    unsupported(
        &mut c,
        "Media",
        json!({"type": "ReadMidiFile", "source": path(file("song.mid"))}),
    );
    unsupported(
        &mut c,
        "Browser",
        json!({"type": "AddFolder", "path": folder}),
    );
    unsupported(
        &mut c,
        "Plugin",
        json!({"type": "AddFolder", "path": folder, "format": null}),
    );
    unsupported(
        &mut c,
        "MediaRef",
        json!({"type": "Search", "media": null, "folder": folder}),
    );
    unsupported(
        &mut c,
        "Project",
        json!({"type": "ExportBundle", "id": pid, "path": file("out.ether"), "include_media": true}),
    );
    let new_id = c.project_id();
    unsupported(
        &mut c,
        "Project",
        json!({"type": "ImportBundle", "new_id": new_id, "source": path(file("song.ether")), "name": null}),
    );
    assert!(!desktop.join("out.ether").exists(), "nothing written");
    let folders = c.ok("Plugin", json!({"type": "ListFolders"}))["folders"].clone();
    assert_eq!(folders["folders"], json!([]), "no plugin folder added");

    // Library files are still referenced in place (by a path inside the library).
    let id = c.id();
    let media = c.ok(
        "Media",
        json!({"type": "Import", "id": id,
               "source": {"type": "Location", "location": {"type": "Library", "id": "lib"}, "path": "kick.wav"}}),
    )["media"]
        .clone();
    assert_eq!(media["location"]["type"], "External");
    // Folders imported from the UI machine are the host's own.
    let roots = c.ok(
        "Browser",
        json!({"type": "ImportFolder", "name": "Field Recordings"}),
    )["roots"]
        .clone();
    assert!(
        roots
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["kind"] == "Folder" && r["name"] == "Field Recordings")
    );
    c.quit();
}

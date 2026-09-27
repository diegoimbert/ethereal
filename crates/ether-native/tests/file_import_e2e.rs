//! Native e2e of importing an OS file by path (`file-import`, CONTRACTS.md §12.13): the real
//! desktop host (null audio backend) gets `Media::Import { source: Path }` like the Tauri
//! file dialog / OS drop sends it. The engine validates and reads the file itself; bad
//! paths reply with clear errors and change nothing.

mod common;

use common::{Client, Paths, sine, wav};
use serde_json::{Value, json};

fn import(c: &mut Client, path: &str) -> Result<Value, (String, String)> {
    let id = c.id();
    let msg = c.post(
        "Media",
        json!({"type": "Import", "id": id, "source": {"type": "Path", "path": path}}),
        None,
    );
    c.reply(msg)
}

#[test]
fn import_os_files_by_path() {
    let tmp = ether_native::test_util::TempDir::new("e2e-file-import");
    // Outside any library root: an arbitrary folder of the user's computer.
    let desktop = tmp.path().join("Desktop");
    std::fs::create_dir_all(desktop.join("folder.wav")).unwrap();
    let kick = wav(44_100, &[sine(44_100, 60.0, 4_410, 0.8)]);
    std::fs::write(desktop.join("Kick 01.wav"), &kick).unwrap();
    std::fs::write(desktop.join("notes.txt"), b"hello").unwrap();
    std::fs::write(desktop.join("broken.wav"), b"RIFF nope").unwrap();

    let mut c = Client::start(&Paths::new(tmp.path()));
    let pid = c.project_id();
    c.ok(
        "Project",
        json!({"type": "Create", "id": pid, "name": "Import"}),
    );

    // A path the dialog returned: imported (copied into the project until
    // `media-references` makes it a reference in place).
    let path = desktop.join("Kick 01.wav");
    let media = import(&mut c, path.to_str().unwrap()).expect("imports")["media"].clone();
    assert_eq!(media["name"], "Kick 01.wav");
    assert_eq!(media["sample_rate"], 44_100);
    assert_eq!(media["channels"], 1);
    let file = media["file"].as_str().unwrap().to_string();
    assert!(file.starts_with("media/"), "{file}");
    let copy = tmp.path().join("projects").join(&pid).join(&file);
    assert_eq!(
        std::fs::read(copy).unwrap(),
        kick,
        "the project holds the bytes"
    );
    let project = c.project();
    assert_eq!(project["media"].as_object().unwrap().len(), 1);

    // Errors: nothing changes.
    let code = |r: Result<Value, (String, String)>| r.expect_err("fails").0;
    assert_eq!(
        code(import(&mut c, "Kick 01.wav")),
        "InvalidArgument",
        "relative"
    );
    let txt = desktop.join("notes.txt");
    assert_eq!(
        code(import(&mut c, txt.to_str().unwrap())),
        "InvalidArgument"
    );
    let dir = desktop.join("folder.wav");
    assert_eq!(
        code(import(&mut c, dir.to_str().unwrap())),
        "InvalidArgument"
    );
    let missing = desktop.join("gone.wav");
    assert_eq!(code(import(&mut c, missing.to_str().unwrap())), "NotFound");
    let broken = desktop.join("broken.wav");
    assert_eq!(code(import(&mut c, broken.to_str().unwrap())), "Decode");
    assert_eq!(c.project()["media"].as_object().unwrap().len(), 1);
    c.quit();
}

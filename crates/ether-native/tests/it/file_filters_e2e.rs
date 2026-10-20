//! Native e2e of the library file filters (CONTRACTS.md §12.8) on the real
//! desktop host (`DiskLibrary` over a real folder): the folder view lists only supported
//! files by default, a whitelist rule shows a new type, a bad rule is refused, and the
//! filter survives a restart.

use crate::common;

use common::{Client, Paths, sine, wav};
use serde_json::{Value, json};

fn names(c: &mut Client, location: &Value, path: &str) -> Vec<String> {
    c.ok(
        "Media",
        json!({"type": "ListDirectory", "location": location, "path": path}),
    )["listing"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap().to_string())
        .collect()
}

fn library_location(c: &mut Client) -> Value {
    c.ok("Media", json!({"type": "ListLocations"}))["locations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["location"]["type"] == "Library")
        .expect("library location")["location"]
        .clone()
}

#[test]
fn folder_view_filters_and_the_filter_persists() {
    let tmp = ether_native::test_util::TempDir::new("e2e-file-filters");
    let library = tmp.path().join("library");
    std::fs::create_dir_all(library.join("Loops")).unwrap();
    let tone = wav(44_100, &[sine(44_100, 220.0, 4_410, 0.5)]);
    for f in ["Kick.wav", "Loops/Break.wav"] {
        std::fs::write(library.join(f), &tone).unwrap();
    }
    for f in [
        "Kick.wav.asd",
        "Song.als",
        "Song.rpp",
        "Kick.wav.reapeaks",
        "Thumbs.db",
        "desktop.ini",
        ".DS_Store",
        "notes.txt",
    ] {
        std::fs::write(library.join(f), b"x").unwrap();
    }
    let paths = Paths {
        library: Some(library),
        ..Paths::new(tmp.path())
    };

    let mut c = Client::start(&paths);
    let lib = library_location(&mut c);
    assert_eq!(names(&mut c, &lib, ""), ["Loops", "Kick.wav"]);

    let got = c.ok("Browser", json!({"type": "GetFileFilter"}));
    let mut filter = got["defaults"].clone();
    assert_eq!(got["filter"], filter);
    filter["whitelist"]
        .as_array_mut()
        .unwrap()
        .push(json!(r"\.als$"));
    let set = c.ok(
        "Browser",
        json!({"type": "SetFileFilter", "filter": filter}),
    );
    assert_eq!(set["filter"], filter);
    assert_eq!(names(&mut c, &lib, ""), ["Loops", "Kick.wav", "Song.als"]);

    // A rule that doesn't compile is refused (nothing changes).
    let mut bad = filter.clone();
    bad["blacklist"]
        .as_array_mut()
        .unwrap()
        .push(json!("[z-a]"));
    let id = c.post(
        "Browser",
        json!({"type": "SetFileFilter", "filter": bad}),
        None,
    );
    let (code, message) = c.reply(id).unwrap_err();
    assert_eq!(code, "InvalidArgument");
    assert!(message.starts_with("blacklist rule 6:"), "{message}");
    c.quit();

    // Restart: still the stored filter.
    let mut c = Client::start(&paths);
    let lib = library_location(&mut c);
    assert_eq!(
        c.ok("Browser", json!({"type": "GetFileFilter"}))["filter"],
        filter
    );
    assert_eq!(names(&mut c, &lib, ""), ["Loops", "Kick.wav", "Song.als"]);
    c.ok("Browser", json!({"type": "ResetFileFilter"}));
    assert_eq!(names(&mut c, &lib, ""), ["Loops", "Kick.wav"]);
    c.quit();
}

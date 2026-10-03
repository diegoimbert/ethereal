//! User plugin folders through the desktop host (null audio backend) with the real
//! scanner: add a folder → incremental rescan finds its plugin; the settings persist
//! across a restart; the defaults switch; remove → the plugin is gone; full rescan.
#![cfg(any(target_os = "macos", target_os = "linux"))]

mod common;

use std::path::{Path, PathBuf};

use common::{Client, Paths, Received};
use ether_core::protocol::Event;
use ether_core::protocol::plugins::PluginEvent;
use ether_native::test_util::TempDir;
use serde_json::{Value, json};

/// The scanner binary: next to this test's target dir when the workspace was built, else
/// built into a separate target dir.
fn scanner() -> PathBuf {
    let file = format!("ether-plugin-scanner{}", std::env::consts::EXE_SUFFIX);
    let exe = std::env::current_exe().expect("current exe");
    if let Some(p) = exe
        .ancestors()
        .skip(1)
        .take(2)
        .map(|d| d.join(&file))
        .find(|p| p.is_file())
    {
        return p;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = root.join("target/ether-native-helper");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = std::process::Command::new(cargo)
        .args([
            "build",
            "-q",
            "-p",
            "ether-plugin-scanner",
            "--bin",
            "ether-plugin-scanner",
        ])
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(&target)
        .status()
        .expect("run cargo build for the scanner");
    assert!(status.success(), "building ether-plugin-scanner failed");
    target.join("debug").join(file)
}

fn scans_finished(r: &Received) -> usize {
    r.events
        .iter()
        .filter(|e| {
            matches!(
                e,
                Event::Plugin {
                    event: PluginEvent::ScanFinished { .. }
                }
            )
        })
        .count()
}

fn wait_scans(c: &Client, n: usize) {
    c.wait(|r| (scans_finished(r) >= n).then_some(()), "scan finished");
}

fn plugin_names(c: &mut Client) -> Vec<String> {
    let mut v: Vec<String> = c.ok("Plugin", json!({"type": "List"}))["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap().to_owned())
        .collect();
    v.sort();
    v
}

#[test]
fn user_folders_are_scanned_persisted_and_removed() {
    // SAFETY: the only test in this binary; set before any host thread starts.
    unsafe { std::env::set_var("ETHER_PLUGIN_SCANNER", scanner()) };
    let tmp = TempDir::new("plugin-folders-e2e");
    let folder = tmp.path().join("My Plugins");
    std::fs::create_dir_all(&folder).unwrap();
    ether_clap::testing::make_bundle(&folder, "EtherFolderPlugin");
    let paths = Paths::new(&tmp.path().join("data"));

    let mut c = Client::start(&paths);
    let l = c.ok("Plugin", json!({"type": "ListFolders"}))["folders"].clone();
    assert_eq!(l["include_defaults"], true);
    assert_eq!(l["folders"], json!([]));
    assert!(!l["defaults"].as_array().unwrap().is_empty());

    // Defaults off (keeps the devbox's own plugins out), then add the folder.
    let l = c.ok(
        "Plugin",
        json!({"type": "SetIncludeDefaults", "include": false}),
    )["folders"]
        .clone();
    assert_eq!(l["include_defaults"], false);
    wait_scans(&c, 1);
    let l = c.ok(
        "Plugin",
        json!({"type": "AddFolder", "path": folder.display().to_string(), "format": null}),
    )["folders"]
        .clone();
    assert_eq!(
        l["folders"],
        json!([{"path": folder.display().to_string(), "format": null}])
    );
    wait_scans(&c, 2);
    assert_eq!(plugin_names(&mut c), ["Ethereal Test Plugin"]);
    let id = c.post(
        "Plugin",
        json!({"type": "AddFolder", "path": "relative", "format": null}),
        None,
    );
    assert_eq!(c.reply(id).unwrap_err().0, "InvalidArgument");
    assert!(
        tmp.path()
            .join("data/plugin-db")
            .join(ether_plugin_host::SCAN_CACHE_FILE)
            .is_file()
    );
    c.quit();

    // Persisted; a VST3-only filter hides the CLAP plugin.
    let mut c = Client::start(&paths);
    let l = c.ok("Plugin", json!({"type": "ListFolders"}))["folders"].clone();
    assert_eq!(l["include_defaults"], false);
    assert_eq!(l["folders"].as_array().unwrap().len(), 1);
    c.ok(
        "Plugin",
        json!({"type": "AddFolder", "path": folder.display().to_string(), "format": "Vst3"}),
    );
    wait_scans(&c, 1);
    assert!(plugin_names(&mut c).is_empty());
    c.ok(
        "Plugin",
        json!({"type": "AddFolder", "path": folder.display().to_string(), "format": "Clap"}),
    );
    wait_scans(&c, 2);
    assert_eq!(plugin_names(&mut c), ["Ethereal Test Plugin"]);

    // Full rescan still finds it.
    c.ok("Plugin", json!({"type": "Rescan", "full": true}));
    wait_scans(&c, 3);
    assert_eq!(plugin_names(&mut c), ["Ethereal Test Plugin"]);

    let l: Value = c.ok(
        "Plugin",
        json!({"type": "RemoveFolder", "path": folder.display().to_string()}),
    )["folders"]
        .clone();
    assert_eq!(l["folders"], json!([]));
    wait_scans(&c, 4);
    assert!(plugin_names(&mut c).is_empty());
    let id = c.post(
        "Plugin",
        json!({"type": "RemoveFolder", "path": folder.display().to_string()}),
        None,
    );
    assert_eq!(c.reply(id).unwrap_err().0, "NotFound");
    c.quit();
}

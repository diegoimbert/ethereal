//! Test support shared by this crate's tests and `ether-plugin-scanner`'s tests: locating the
//! `ether_test_plugin` example cdylib and wrapping it into a platform `.clap` bundle.
//! Not part of the hosting API.

use std::path::{Path, PathBuf};

pub use ether_plugin_host::testing::{instance_id, temp_dir};

fn dylib_name() -> String {
    format!(
        "{}ether_test_plugin{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    )
}

/// Path of the built `ether_test_plugin` example. `cargo test` builds examples, so it is
/// normally already there; otherwise it is built into a separate target dir (to avoid
/// contending for the lock of the running cargo).
pub fn fixture_dylib() -> PathBuf {
    let exe = std::env::current_exe().expect("current exe");
    // target/<profile>/deps/<test> or target/<profile>/<bin>
    for dir in exe.ancestors().skip(1).take(3) {
        let candidate = dir.join("examples").join(dylib_name());
        if candidate.is_file() {
            return candidate;
        }
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let target = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/ether-clap-fixture")
        .to_path_buf();
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = std::process::Command::new(cargo)
        .args([
            "build",
            "-q",
            "--example",
            "ether_test_plugin",
            "--manifest-path",
        ])
        .arg(&manifest)
        .arg("--target-dir")
        .arg(&target)
        .status()
        .expect("run cargo build for the test plugin");
    assert!(status.success(), "building ether_test_plugin failed");
    target.join("debug/examples").join(dylib_name())
}

/// Wrap the fixture into a `.clap` bundle named `<name>.clap` inside `dir`.
/// macOS: a real bundle directory (Info.plist + Contents/MacOS/<name>); elsewhere: the
/// library copied to `<name>.clap`.
pub fn make_bundle(dir: &Path, name: &str) -> PathBuf {
    let dylib = fixture_dylib();
    let bundle = dir.join(format!("{name}.clap"));
    if cfg!(target_os = "macos") {
        let macos = bundle.join("Contents/MacOS");
        std::fs::create_dir_all(&macos).expect("create bundle");
        std::fs::copy(&dylib, macos.join(name)).expect("copy dylib");
        std::fs::write(
            bundle.join("Contents/Info.plist"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>{name}</string>
  <key>CFBundleIdentifier</key><string>dev.ethereal.{name}</string>
  <key>CFBundlePackageType</key><string>BNDL</string>
</dict>
</plist>
"#
            ),
        )
        .expect("write Info.plist");
    } else {
        // The bundle is a single file here, so nothing above creates `dir` (the macOS branch
        // gets it from `create_dir_all` of the bundle's Contents/MacOS).
        std::fs::create_dir_all(dir).expect("create bundle dir");
        std::fs::copy(&dylib, &bundle).expect("copy dylib");
    }
    bundle
}

//! Test support shared by this crate's tests and the scanner/sandbox VST3 tests: locating the
//! `ether_vst3_test_plugin` example cdylib and wrapping it into a platform `.vst3` bundle.
//! Not part of the hosting API.
//!
//! The fixture (`examples/ether_vst3_test_plugin.rs`) exports two classes; see its docs.

use std::path::{Path, PathBuf};

use ether_core::plugin::ipc_name;

/// Class id of the fixture's gain effect (separate controller, `IConnectionPoint`).
pub const EFFECT_ID: &str = "E7E1E4A1000000000000000000000001";
/// Class id of the fixture's instrument (single component).
pub const INSTRUMENT_ID: &str = "E7E1E4A1000000000000000000000002";

/// Sanitized dev instance id (`ETHER_INSTANCE`, see README "Running multiple dev instances").
pub fn instance_id() -> String {
    let raw = std::env::var("ETHER_INSTANCE").unwrap_or_else(|_| "default".into());
    let s: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if s.is_empty() { "default".into() } else { s }
}

/// A fresh temp directory named with the instance + pid rule (`ipc_name`).
pub fn temp_dir(purpose: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(ipc_name(
        &instance_id(),
        std::process::id(),
        &format!("{purpose}-{n}"),
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn dylib_name() -> String {
    format!(
        "{}ether_vst3_test_plugin{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    )
}

/// Path of the built `ether_vst3_test_plugin` example. `cargo test -p ether-vst3` builds
/// examples, so it is normally already there; otherwise (tests of other crates) it is built
/// into a separate target dir (to avoid contending for the lock of the running cargo).
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
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/ether-vst3-fixture");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = std::process::Command::new(cargo)
        .args([
            "build",
            "-q",
            "--example",
            "ether_vst3_test_plugin",
            "--manifest-path",
        ])
        .arg(&manifest)
        .arg("--target-dir")
        .arg(&target)
        .status()
        .expect("run cargo build for the VST3 test plugin");
    assert!(status.success(), "building ether_vst3_test_plugin failed");
    target.join("debug/examples").join(dylib_name())
}

/// `Contents/<dir>` of this platform's binary (Linux/Windows bundle layout).
#[cfg(not(target_os = "macos"))]
fn arch_dir() -> String {
    let arch = match std::env::consts::ARCH {
        "aarch64" if cfg!(windows) => "arm64",
        a => a,
    };
    format!("{arch}-{}", if cfg!(windows) { "win" } else { "linux" })
}

/// Wrap the fixture into a `.vst3` bundle named `<name>.vst3` inside `dir` (created).
/// macOS: `Contents/Info.plist` + `Contents/MacOS/<name>`; Linux:
/// `Contents/<arch>-linux/<name>.so`; Windows: `Contents/<arch>-win/<name>.vst3`.
pub fn make_bundle(dir: &Path, name: &str) -> PathBuf {
    let dylib = fixture_dylib();
    let bundle = dir.join(format!("{name}.vst3"));
    #[cfg(target_os = "macos")]
    {
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
  <key>CFBundleIdentifier</key><string>dev.ethereal.vst3.{name}</string>
  <key>CFBundlePackageType</key><string>BNDL</string>
</dict>
</plist>
"#
            ),
        )
        .expect("write Info.plist");
    }
    #[cfg(not(target_os = "macos"))]
    {
        let bin = bundle.join("Contents").join(arch_dir());
        std::fs::create_dir_all(&bin).expect("create bundle");
        let ext = if cfg!(windows) { "vst3" } else { "so" };
        std::fs::copy(&dylib, bin.join(format!("{name}.{ext}"))).expect("copy dylib");
    }
    bundle
}

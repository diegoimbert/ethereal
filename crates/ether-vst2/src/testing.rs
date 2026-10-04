//! Test support shared by this crate's tests and the scanner/sandbox VST2 tests: locating the
//! test-plugin example cdylibs and installing them as platform VST2 plugins. Not part of the
//! hosting API.
//!
//! The fixture source (`examples/ether_vst2_test_plugin.rs`) is built twice:
//! - [`Fixture::Plugin`] (`ether_vst2_test_plugin`): one gain effect, [`GAIN_ID`];
//! - [`Fixture::Shell`] (`ether_vst2_test_shell`): a shell with a gain effect
//!   ([`SHELL_GAIN_ID`], 64-bit processing only) and a synth ([`SYNTH_ID`]).

use std::path::{Path, PathBuf};

use ether_core::plugin::ipc_name;

/// Plugin id of the standalone gain effect (`'EtG2'`).
pub const GAIN_ID: &str = "45744732";
/// Plugin id of the shell's gain effect (`'EtSg'`, `processDoubleReplacing` only).
pub const SHELL_GAIN_ID: &str = "45745367";
/// Plugin id of the shell's synth (`'EtSy'`).
pub const SYNTH_ID: &str = "45745379";

/// Which fixture library.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fixture {
    Plugin,
    Shell,
}

impl Fixture {
    fn example(self) -> &'static str {
        match self {
            Fixture::Plugin => "ether_vst2_test_plugin",
            Fixture::Shell => "ether_vst2_test_shell",
        }
    }
}

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

fn dylib_name(example: &str) -> String {
    format!(
        "{}{example}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    )
}

/// Path of a built fixture. `cargo test -p ether-vst2` builds examples, so it is normally
/// already there; otherwise (tests of other crates) both are built into a separate target
/// dir (to avoid contending for the lock of the running cargo).
pub fn fixture_dylib(fixture: Fixture) -> PathBuf {
    let name = dylib_name(fixture.example());
    let exe = std::env::current_exe().expect("current exe");
    // target/<profile>/deps/<test> or target/<profile>/<bin>
    for dir in exe.ancestors().skip(1).take(3) {
        let candidate = dir.join("examples").join(&name);
        if candidate.is_file() {
            return candidate;
        }
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/ether-vst2-fixture");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = std::process::Command::new(cargo)
        .args([
            "build",
            "-q",
            "--example",
            "ether_vst2_test_plugin",
            "--example",
            "ether_vst2_test_shell",
            "--manifest-path",
        ])
        .arg(&manifest)
        .arg("--target-dir")
        .arg(&target)
        .status()
        .expect("run cargo build for the VST2 test plugins");
    assert!(status.success(), "building the VST2 test plugins failed");
    target.join("debug/examples").join(name)
}

/// Install `fixture` as a VST2 plugin named `name` inside `dir` (created): `<name>.so`
/// (Linux), `<name>.dll` (Windows), or a `<name>.vst` bundle (macOS: `Contents/Info.plist` +
/// `Contents/MacOS/<name>`). Returns the plugin path.
pub fn make_plugin(dir: &Path, name: &str, fixture: Fixture) -> PathBuf {
    let dylib = fixture_dylib(fixture);
    std::fs::create_dir_all(dir).expect("create plugin dir");
    #[cfg(target_os = "macos")]
    {
        let bundle = dir.join(format!("{name}.vst"));
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
  <key>CFBundleIdentifier</key><string>dev.ethereal.vst2.{name}</string>
  <key>CFBundlePackageType</key><string>BNDL</string>
</dict>
</plist>
"#
            ),
        )
        .expect("write Info.plist");
        bundle
    }
    #[cfg(not(target_os = "macos"))]
    {
        let path = dir.join(format!("{name}.{}", crate::EXTENSION));
        std::fs::copy(&dylib, &path).expect("copy dylib");
        path
    }
}

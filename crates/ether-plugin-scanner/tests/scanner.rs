//! Out-of-process scanning against the `ether_test_plugin` fixture (built by `ether-clap`).
//! The fixture aborts when its bundle path contains `ether-crash` and hangs when it contains
//! `ether-hang`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use ether_clap::{ScanRunner, testing};
use ether_core::protocol::devices::DeviceCategory;
use ether_core::protocol::model::PluginFormat;
use ether_core::protocol::plugins::{PluginDescriptor, ScanFailure};

const SCANNER: &str = env!("CARGO_BIN_EXE_ether-plugin-scanner");

fn runner() -> ScanRunner {
    ScanRunner::new(SCANNER).with_timeout(Duration::from_secs(20))
}

fn check_descriptor(d: &PluginDescriptor, bundle: &Path) {
    assert_eq!(d.format, PluginFormat::Clap);
    assert_eq!(d.id, "dev.ethereal.test-plugin");
    assert_eq!(d.name, "Ethereal Test Plugin");
    assert_eq!(d.vendor, "Ethereal");
    assert_eq!(d.version, "1.2.3");
    assert_eq!(d.description, "Test fixture");
    assert_eq!(d.features, vec!["audio-effect", "stereo"]);
    assert_eq!(d.category, DeviceCategory::AudioEffect);
    assert_eq!(d.path, bundle.to_string_lossy());
}

#[test]
fn scans_a_bundle_out_of_process() {
    let dir = testing::temp_dir("scan-ok");
    let bundle = testing::make_bundle(&dir, "EtherScanOk");
    let plugins = runner().scan_bundle(&bundle).expect("scan");
    assert_eq!(plugins.len(), 1);
    check_descriptor(&plugins[0], &bundle);
}

#[test]
fn survives_a_crashing_plugin() {
    let dir = testing::temp_dir("scan-crash");
    let bundle = testing::make_bundle(&dir, "ether-crash");
    let err = runner().scan_bundle(&bundle).unwrap_err();
    assert!(err.contains("crashed"), "{err}");
}

#[test]
fn kills_a_hanging_plugin_after_timeout() {
    let dir = testing::temp_dir("scan-hang");
    let bundle = testing::make_bundle(&dir, "ether-hang");
    let start = Instant::now();
    let err = ScanRunner::new(SCANNER)
        .with_timeout(Duration::from_millis(1500))
        .scan_bundle(&bundle)
        .unwrap_err();
    assert!(err.contains("timed out"), "{err}");
    assert!(start.elapsed() < Duration::from_secs(10));
}

#[test]
fn reports_non_plugins_and_missing_files() {
    let dir = testing::temp_dir("scan-bad");
    let bogus = dir.join("Bogus.clap");
    std::fs::write(&bogus, b"not a library").unwrap();
    let err = runner().scan_bundle(&bogus).unwrap_err();
    assert!(!err.is_empty());
    let err = runner().scan_bundle(&dir.join("Missing.clap")).unwrap_err();
    assert!(err.contains("not found"), "{err}");
    let err = ScanRunner::new(dir.join("no-such-scanner"))
        .scan_bundle(&bogus)
        .unwrap_err();
    assert!(err.contains("failed to start scanner"), "{err}");
}

#[test]
fn scan_all_collects_plugins_and_failures() {
    let dir = testing::temp_dir("scan-all");
    let ok = testing::make_bundle(&dir.join("a"), "EtherAllOk");
    let crash = testing::make_bundle(&dir.join("b"), "ether-crash");
    let bundles = ether_clap::find_bundles(std::slice::from_ref(&dir));
    assert_eq!(bundles, vec![ok.clone(), crash.clone()]);

    let mut calls = Vec::new();
    let report = runner().scan_all(&bundles, |done, total, current| {
        calls.push((done, total, current.map(Path::to_path_buf)));
    });
    assert_eq!(report.plugins.len(), 1);
    check_descriptor(&report.plugins[0], &ok);
    assert_eq!(report.failed.len(), 1);
    assert_eq!(report.failed[0].path, crash.to_string_lossy());
    assert_eq!(
        calls,
        vec![
            (0, 2, Some(ok.clone())),
            (1, 2, Some(crash.clone())),
            (2, 2, None)
        ]
    );
}

#[test]
fn binary_scan_all_and_paths_modes() {
    let dir = testing::temp_dir("scan-cli");
    let ok = testing::make_bundle(&dir, "EtherCliOk");
    let _crash = testing::make_bundle(&dir, "ether-crash");
    let out = Command::new(SCANNER)
        .arg("--scan-all")
        .arg("--timeout-ms")
        .arg("20000")
        .arg(&dir)
        .stderr(Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let plugins: Vec<PluginDescriptor> = serde_json::from_value(v["plugins"].clone()).unwrap();
    let failed: Vec<ScanFailure> = serde_json::from_value(v["failed"].clone()).unwrap();
    assert_eq!(plugins.len(), 1);
    check_descriptor(&plugins[0], &ok);
    assert_eq!(failed.len(), 1);

    let out = Command::new(SCANNER)
        .arg("--paths")
        .env("CLAP_PATH", &dir)
        .output()
        .unwrap();
    let paths: Vec<PathBuf> = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(paths.first(), Some(&dir));
}

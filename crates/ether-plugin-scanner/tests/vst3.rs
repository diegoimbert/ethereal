//! Out-of-process scanning of VST3 bundles against `ether-vst3`'s `ether_vst3_test_plugin`
//! fixture. The fixture aborts when its module path contains `ether-crash` and hangs when it
//! contains `ether-hang`.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use ether_core::protocol::devices::DeviceCategory;
use ether_core::protocol::model::PluginFormat;
use ether_core::protocol::plugins::{PluginDescriptor, ScanFailure};
use ether_plugin_host::{ScanRunner, ScanTarget};
use ether_vst3::testing::{self, EFFECT_ID, INSTRUMENT_ID};

const SCANNER: &str = env!("CARGO_BIN_EXE_ether-plugin-scanner");

fn runner() -> ScanRunner {
    ScanRunner::new(SCANNER).with_timeout(Duration::from_secs(20))
}

fn check(plugins: &[PluginDescriptor], bundle: &Path) {
    assert_eq!(plugins.len(), 2, "{plugins:?}");
    let (fx, synth) = (&plugins[0], &plugins[1]);
    for p in plugins {
        assert_eq!(p.format, PluginFormat::Vst3);
        assert_eq!(p.vendor, "Ethereal");
        assert_eq!(p.version, "1.2.3");
        assert_eq!(p.path, bundle.to_string_lossy());
    }
    assert_eq!(fx.id, EFFECT_ID);
    assert_eq!(fx.name, "Ether VST3 Gain");
    assert_eq!(fx.features, ["fx", "dynamics"]);
    assert_eq!(fx.category, DeviceCategory::AudioEffect);
    assert_eq!(synth.id, INSTRUMENT_ID);
    assert_eq!(synth.name, "Ether VST3 Synth");
    assert_eq!(synth.features, ["instrument", "synth"]);
    assert_eq!(synth.category, DeviceCategory::Instrument);
}

#[test]
fn scans_a_vst3_bundle_out_of_process() {
    let dir = testing::temp_dir("vst3-scan-ok");
    let bundle = testing::make_bundle(&dir, "EtherVst3ScanOk");
    // Format inferred from the extension, and explicit.
    check(&runner().scan_bundle(&bundle).expect("scan"), &bundle);
    let target = ScanTarget {
        format: PluginFormat::Vst3,
        path: bundle.clone(),
    };
    check(&runner().scan_target(&target).expect("scan"), &bundle);
}

#[test]
fn survives_a_crashing_vst3() {
    let dir = testing::temp_dir("vst3-scan-crash");
    let bundle = testing::make_bundle(&dir, "ether-crash");
    let err = runner().scan_bundle(&bundle).unwrap_err();
    assert!(err.contains("crashed"), "{err}");
}

#[test]
fn kills_a_hanging_vst3_after_timeout() {
    let dir = testing::temp_dir("vst3-scan-hang");
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
fn reports_bad_vst3_bundles() {
    let dir = testing::temp_dir("vst3-scan-bad");
    let err = runner().scan_bundle(&dir.join("Missing.vst3")).unwrap_err();
    assert!(err.contains("not found"), "{err}");
    let empty = dir.join("Empty.vst3");
    std::fs::create_dir_all(empty.join("Contents")).unwrap();
    let err = runner().scan_bundle(&empty).unwrap_err();
    assert!(err.contains("no module binary"), "{err}");
}

#[test]
fn binary_scan_all_finds_vst3_bundles() {
    let dir = testing::temp_dir("vst3-scan-cli");
    let ok = testing::make_bundle(&dir.join("a"), "EtherVst3CliOk");
    let crash = testing::make_bundle(&dir.join("b"), "ether-crash");
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
    check(&plugins, &ok);
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert_eq!(failed[0].path, crash.to_string_lossy());
}

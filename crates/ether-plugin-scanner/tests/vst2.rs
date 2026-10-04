//! Out-of-process scanning of VST2 libraries against `ether-vst2`'s test plugins (a plain gain
//! effect and a shell). The fixture aborts when its path contains `ether-crash` and hangs when
//! it contains `ether-hang` (Unix).

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use ether_core::protocol::devices::DeviceCategory;
use ether_core::protocol::model::PluginFormat;
use ether_core::protocol::plugins::{PluginDescriptor, ScanFailure};
use ether_plugin_host::{ScanRunner, ScanTarget};
use ether_vst2::testing::{self, Fixture, GAIN_ID, SHELL_GAIN_ID, SYNTH_ID};

const SCANNER: &str = env!("CARGO_BIN_EXE_ether-plugin-scanner");

fn runner() -> ScanRunner {
    ScanRunner::new(SCANNER).with_timeout(Duration::from_secs(20))
}

fn check_gain(plugins: &[PluginDescriptor], path: &Path) {
    assert_eq!(plugins.len(), 1, "{plugins:?}");
    let fx = &plugins[0];
    assert_eq!(fx.format, PluginFormat::Vst2);
    assert_eq!(fx.id, GAIN_ID);
    assert_eq!(fx.name, "Ether VST2 Gain");
    assert_eq!(fx.vendor, "Ethereal");
    assert_eq!(fx.category, DeviceCategory::AudioEffect);
    assert_eq!(fx.path, path.to_string_lossy());
}

#[test]
fn scans_vst2_libraries_out_of_process() {
    let dir = testing::temp_dir("vst2-scan-ok");
    let gain = testing::make_plugin(&dir, "EtherVst2ScanOk", Fixture::Plugin);
    // Format inferred from the extension, and explicit.
    check_gain(&runner().scan_bundle(&gain).expect("scan"), &gain);
    let target = ScanTarget {
        format: PluginFormat::Vst2,
        path: gain.clone(),
    };
    check_gain(&runner().scan_target(&target).expect("scan"), &gain);

    // A shell lists every sub-plugin.
    let shell = testing::make_plugin(&dir, "EtherVst2ScanShell", Fixture::Shell);
    let plugins = runner().scan_bundle(&shell).expect("scan shell");
    let ids: Vec<_> = plugins
        .iter()
        .map(|p| (p.id.as_str(), p.category))
        .collect();
    assert_eq!(
        ids,
        [
            (SHELL_GAIN_ID, DeviceCategory::AudioEffect),
            (SYNTH_ID, DeviceCategory::Instrument)
        ]
    );
}

#[cfg(unix)]
#[test]
fn survives_a_crashing_vst2() {
    let dir = testing::temp_dir("vst2-scan-crash");
    let path = testing::make_plugin(&dir, "ether-crash", Fixture::Plugin);
    let err = runner().scan_bundle(&path).unwrap_err();
    assert!(err.contains("crashed"), "{err}");
}

#[cfg(unix)]
#[test]
fn kills_a_hanging_vst2_after_timeout() {
    let dir = testing::temp_dir("vst2-scan-hang");
    let path = testing::make_plugin(&dir, "ether-hang", Fixture::Plugin);
    let start = Instant::now();
    let err = ScanRunner::new(SCANNER)
        .with_timeout(Duration::from_millis(1500))
        .scan_bundle(&path)
        .unwrap_err();
    assert!(err.contains("timed out"), "{err}");
    assert!(start.elapsed() < Duration::from_secs(10));
}

#[test]
fn reports_missing_vst2_libraries() {
    let dir = testing::temp_dir("vst2-scan-bad");
    let missing = dir.join(format!("Missing.{}", ether_vst2::EXTENSION));
    let err = runner().scan_bundle(&missing).unwrap_err();
    assert!(err.contains("not found"), "{err}");
}

#[cfg(unix)]
#[test]
fn binary_scan_all_finds_vst2_libraries() {
    let dir = testing::temp_dir("vst2-scan-cli");
    let ok = testing::make_plugin(&dir.join("a"), "EtherVst2CliOk", Fixture::Plugin);
    let crash = testing::make_plugin(&dir.join("b"), "ether-crash", Fixture::Plugin);
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
    check_gain(&plugins, &ok);
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert_eq!(failed[0].path, crash.to_string_lossy());
}

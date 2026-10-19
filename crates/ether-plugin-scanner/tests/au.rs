//! Out-of-process AU scanning through the real scanner binary, against Apple's built-in
//! units (present on every Mac). Owned by the `au` node.
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::time::Duration;

use ether_core::protocol::devices::DeviceCategory;
use ether_core::protocol::model::PluginFormat;
use ether_plugin_host::{Formats, PluginFormatHost, ScanRunner, ScanTarget};

const SCANNER: &str = env!("CARGO_BIN_EXE_ether-plugin-scanner");

fn runner() -> ScanRunner {
    ScanRunner::new(SCANNER).with_timeout(Duration::from_secs(20))
}

fn au(id: &str) -> ScanTarget {
    ScanTarget {
        format: PluginFormat::Au,
        path: PathBuf::from(id),
    }
}

#[test]
fn scans_builtin_units_out_of_process() {
    let plugins = runner().scan_target(&au("aufx:dely:appl")).expect("scan");
    assert_eq!(plugins.len(), 1);
    let d = &plugins[0];
    assert_eq!(d.format, PluginFormat::Au);
    assert_eq!(d.id, "aufx:dely:appl");
    assert_eq!(d.path, "aufx:dely:appl");
    assert_eq!(d.name, "AUDelay");
    assert_eq!(d.vendor, "Apple");
    assert_eq!(d.features, vec!["aufx"]);
    assert_eq!(d.category, DeviceCategory::AudioEffect);

    // Format inferred from the id's shape.
    let plugins = runner()
        .scan_bundle(Path::new("aumu:dls :appl"))
        .expect("scan");
    assert_eq!(plugins[0].category, DeviceCategory::Instrument);
    assert_eq!(plugins[0].format, PluginFormat::Au);
}

#[test]
fn reports_missing_components() {
    let err = runner().scan_target(&au("aufx:none:zzzz")).unwrap_err();
    assert!(err.contains("not found"), "{err}");
}

#[test]
fn registry_targets_scan_in_batch() {
    let formats = Formats::new(vec![std::sync::Arc::new(ether_au::AuFormat)]);
    let all = formats.discover(None);
    let wanted = ["aufx:dely:appl", "aufx:lpas:appl", "aumu:dls :appl"];
    let targets: Vec<ScanTarget> = all
        .into_iter()
        .filter(|t| wanted.iter().any(|w| t.path == Path::new(w)))
        .collect();
    assert_eq!(targets.len(), 3, "built-ins discovered from the registry");
    assert!(targets.iter().all(|t| ether_au::AuFormat.claims(&t.path)));
    let report = runner().scan_targets(&targets, |_, _, _| {});
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    let mut ids: Vec<&str> = report.plugins.iter().map(|p| p.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, wanted);
}

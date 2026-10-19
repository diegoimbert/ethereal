//! Frozen `.ether` documents, one set per format version (`fixtures/v<N>_*.ether`).
//!
//! - Every fixture loads, whatever its version (old ones through the migrations), forever.
//! - Fixtures at [`file::CURRENT_VERSION`] re-save **byte for byte**: any change to how a
//!   field, table or enum variant is written fails here. Such a change needs a format bump
//!   and a migration (`file.rs` docs); then add `v<N+1>_*` fixtures and keep these.

use ether_model::*;
use serde_json::Value;

const FIXTURES: &[(&str, &str)] = &[
    ("v1_full", include_str!("fixtures/v1_full.ether")),
    (
        "v1_clap_plugin",
        include_str!("fixtures/v1_clap_plugin.ether"),
    ),
];

fn version(json: &str) -> u32 {
    let v: Value = serde_json::from_str(json).unwrap();
    v["format_version"].as_u64().unwrap() as u32
}

#[test]
fn every_fixture_loads() {
    for (name, json) in FIXTURES {
        let p = file::load(json).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(p.tracks.len() > 1, "{name}");
    }
}

#[test]
fn current_fixtures_resave_byte_for_byte() {
    let mut current = 0;
    for (name, json) in FIXTURES {
        if version(json) != file::CURRENT_VERSION {
            continue;
        }
        current += 1;
        let app = serde_json::from_str::<Value>(json).unwrap()["app_version"]
            .as_str()
            .unwrap()
            .to_string();
        let saved = file::save(&file::load(json).unwrap(), &app).unwrap();
        assert!(
            saved == *json,
            "{name}: the format changed; bump file::CURRENT_VERSION and add a migration"
        );
    }
    assert!(current > 0, "no fixture at the current format version");
}

#[test]
fn full_fixture_content() {
    let p = file::load(FIXTURES[0].1).unwrap();
    assert_eq!(p.settings.name, "Fixture v1");
    for (table, n) in [
        ("tracks", p.tracks.len()),
        ("clips", p.clips.len()),
        ("notes", p.notes.len()),
        ("devices", p.devices.len()),
        ("sends", p.sends.len()),
        ("automation_lanes", p.automation_lanes.len()),
        ("automation_points", p.automation_points.len()),
        ("warp_markers", p.warp_markers.len()),
        ("media", p.media.len()),
        ("markers", p.markers.len()),
    ] {
        assert!(n > 0, "{table}");
    }
    let keyed = p
        .devices
        .values()
        .find(|d| d.sidechain.is_some())
        .expect("a sidechained device");
    assert_eq!(keyed.sidechain_tap.map(|t| t.point), Some(InputTap::PreFx));
}

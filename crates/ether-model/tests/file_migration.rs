//! Loading a realistic v1 `.ether` document (with Session view data) through `file::load`.

use ether_model::*;
use serde_json::Value;

const V1: &str = include_str!("fixtures/v1_with_session.ether");

#[test]
fn files_without_scale_metadata_default_to_unrestricted() {
    let legacy = file::load(V1).unwrap();
    assert_eq!(legacy.settings.scale, MusicalScale::default());
    assert!(
        legacy
            .tracks
            .values()
            .all(|t| t.scale == TrackScale::FollowProject)
    );
    let mut v2 = serde_json::json!({
        "format": file::FORMAT_TAG, "version": file::CURRENT_VERSION,
        "app_version": "0.0.1", "project": legacy,
    });
    v2["project"]["settings"]
        .as_object_mut()
        .unwrap()
        .remove("scale");
    for track in v2["project"]["tracks"]
        .as_object_mut()
        .unwrap()
        .values_mut()
    {
        track.as_object_mut().unwrap().remove("scale");
    }
    assert_eq!(file::load(&v2.to_string()).unwrap(), legacy);
}

fn keys(v: &Value, table: &str) -> Vec<String> {
    v["project"][table]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect()
}

#[test]
fn v1_with_session_loads_as_v2() {
    let raw: Value = serde_json::from_str(V1).unwrap();
    assert_eq!(raw["version"], 1);
    let p = file::load(V1).expect("v1 fixture migrates and validates");

    // The fixture's shape: one arrangement MIDI clip (with notes) and one session audio clip
    // (with a warp marker and a clip-envelope lane with points), scenes, launch settings.
    let clips = raw["project"]["clips"].as_object().unwrap();
    let (arr_id, arr) = clips
        .iter()
        .find(|(_, c)| c["location"]["type"] == "Arrangement")
        .unwrap();
    let (ses_id, ses) = clips
        .iter()
        .find(|(_, c)| c["location"]["type"] == "Session")
        .unwrap();
    assert!(arr["launch"].is_object() && ses["launch"].is_object());
    assert_eq!(ses["content"]["type"], "Audio");
    assert!(raw["project"]["settings"]["launch_quantization"].is_object());
    assert!(!keys(&raw, "scenes").is_empty());

    // Arrangement clip: kept, `start` == old location start, notes kept.
    let arr_clip: ClipId = serde_json::from_value(Value::from(arr_id.as_str())).unwrap();
    let clip = &p.clips[&arr_clip];
    assert_eq!(clip.start.0, arr["location"]["start"].as_f64().unwrap());
    assert_eq!(clip.start, Beats(8.0));
    let arr_notes = raw["project"]["notes"]
        .as_object()
        .unwrap()
        .values()
        .filter(|n| n["clip"] == arr_id.as_str())
        .count();
    assert!(arr_notes > 0);
    assert_eq!(p.notes_of(arr_clip).len(), arr_notes);

    // Every session-owned entity is gone.
    let ses_clip: ClipId = serde_json::from_value(Value::from(ses_id.as_str())).unwrap();
    assert!(!p.clips.contains_key(&ses_clip));
    assert_eq!(p.clips.len(), 1);
    assert!(p.notes.values().all(|n| n.clip != ses_clip));
    let raw_markers = keys(&raw, "warp_markers");
    assert!(!raw_markers.is_empty());
    assert!(p.warp_markers.is_empty());
    let raw_lanes = keys(&raw, "automation_lanes");
    let raw_points = keys(&raw, "automation_points");
    assert!(!raw_lanes.is_empty() && !raw_points.is_empty());
    assert!(p.automation_lanes.is_empty());
    assert!(p.automation_points.is_empty());

    // Scenes and launch settings are gone from the v2 shape.
    let saved: Value = serde_json::from_str(&file::save(&p, "0.1.0").unwrap()).unwrap();
    assert_eq!(saved["version"], file::CURRENT_VERSION);
    assert!(saved["project"].get("scenes").is_none());
    assert!(
        saved["project"]["settings"]
            .get("launch_quantization")
            .is_none()
    );
    for c in saved["project"]["clips"].as_object().unwrap().values() {
        assert!(c.get("location").is_none() && c.get("launch").is_none());
    }
    // Everything else survives untouched.
    assert_eq!(p.tracks.len(), 3);
    assert_eq!(p.media.len(), 1);
    assert_eq!(file::load(&file::save(&p, "0.1.0").unwrap()).unwrap(), p);
}

//! The `.ether` file format: versioned JSON with migrations from day one.
//!
//! ```json
//! { "format": "ethereal-project", "version": 4, "app_version": "0.2.0", "project": { ... } }
//! ```
//!
//! Loading: parse to `serde_json::Value`, read `version`, run every migration from that
//! version up to [`CURRENT_VERSION`] on the untyped value, then deserialize and `validate`.
//! On disk (engine-side `ProjectStore`), a project is a folder named by its UUIDv7:
//! `<projects_root>/<project-uuid>/{project.ether, media/, cache/}`. Media paths in the
//! document are relative to that folder (`media/...`). The display name lives in the file
//! (`project.settings.name`), so renaming never moves the folder.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::FileError;
use crate::project::Project;

/// Magic string in the `format` field.
pub const FORMAT_TAG: &str = "ethereal-project";
/// Current `.ether` version. Bump + add a [`Migration`] for every breaking schema change.
pub const CURRENT_VERSION: u32 = 4;
/// File extension (without dot).
pub const EXTENSION: &str = "ether";
/// Document file name inside a project folder.
pub const PROJECT_FILE: &str = "project.ether";
/// Imported media folder inside a project folder.
pub const MEDIA_DIR: &str = "media";
/// Regenerable caches (peak mipmaps, decoded audio) inside a project folder.
pub const CACHE_DIR: &str = "cache";
/// Rendered exports (roadmap v2, `export`) inside a project folder (native hosts). Not
/// referenced by the document.
pub const EXPORTS_DIR: &str = "exports";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct EtherFile {
    pub format: String,
    pub version: u32,
    /// Version of the app that wrote the file (informational).
    pub app_version: String,
    pub project: Project,
}

/// Upgrades an untyped document from `source_version()` to `source_version() + 1`.
pub trait Migration: Send + Sync {
    fn source_version(&self) -> u32;
    fn migrate(&self, doc: &mut serde_json::Value) -> Result<(), FileError>;
}

/// All migrations, in order.
pub fn migrations() -> Vec<Box<dyn Migration>> {
    vec![
        Box::new(V1RemoveSession),
        Box::new(V2RoadmapDefaults),
        Box::new(V3ContractsV3Defaults),
    ]
}

/// v1 → v2: Session view removed.
///
/// - arrangement clips: `location: {type: "Arrangement", start}` → `start`; `launch` dropped;
/// - session clips (`location.type == "Session"`) are dropped together with their notes, warp
///   markers and clip-envelope automation lanes (+ their points);
/// - the `scenes` table and `settings.launch_quantization` are dropped.
pub struct V1RemoveSession;

impl Migration for V1RemoveSession {
    fn source_version(&self) -> u32 {
        1
    }

    fn migrate(&self, doc: &mut serde_json::Value) -> Result<(), FileError> {
        use serde_json::Value;
        let err = |message: &str| FileError::Migration {
            from: 1,
            message: message.into(),
        };
        let project = doc["project"]
            .as_object_mut()
            .ok_or_else(|| err("no project object"))?;
        project.remove("scenes");
        if let Some(settings) = project.get_mut("settings").and_then(Value::as_object_mut) {
            settings.remove("launch_quantization");
        }

        let mut dropped = std::collections::HashSet::new();
        if let Some(clips) = project.get_mut("clips").and_then(Value::as_object_mut) {
            clips.retain(|id, clip| {
                let Some(c) = clip.as_object_mut() else {
                    return true;
                };
                c.remove("launch");
                let location = c.remove("location").unwrap_or(Value::Null);
                match location["type"].as_str() {
                    Some("Session") => {
                        dropped.insert(id.clone());
                        false
                    }
                    _ => {
                        let start = location.get("start").cloned().unwrap_or(Value::from(0.0));
                        c.insert("start".into(), start);
                        true
                    }
                }
            });
        }
        if dropped.is_empty() {
            return Ok(());
        }
        let owned = |v: &Value, field: &str| v[field].as_str().is_some_and(|c| dropped.contains(c));
        for table in ["notes", "warp_markers"] {
            if let Some(t) = project.get_mut(table).and_then(Value::as_object_mut) {
                t.retain(|_, v| !owned(v, "clip"));
            }
        }
        let mut dropped_lanes = std::collections::HashSet::new();
        if let Some(t) = project
            .get_mut("automation_lanes")
            .and_then(Value::as_object_mut)
        {
            t.retain(|id, v| {
                let gone = v["owner"]["type"] == "Clip" && owned(&v["owner"], "clip");
                if gone {
                    dropped_lanes.insert(id.clone());
                }
                !gone
            });
        }
        if let Some(t) = project
            .get_mut("automation_points")
            .and_then(Value::as_object_mut)
        {
            t.retain(|_, v| {
                !v["lane"]
                    .as_str()
                    .is_some_and(|l| dropped_lanes.contains(l))
            });
        }
        Ok(())
    }
}

/// v2 → v3: roadmap v2 fields and tables, all with neutral defaults (a migrated project
/// sounds and behaves exactly as before).
///
/// - tables `markers`, `midi_mappings`, `drum_pads` (empty);
/// - settings: `metronome_volume` -6 dB, `metronome_accent` true, `metronome_sound`
///   `Classic`, `swing` 0, `swing_grid` 0.25;
/// - devices: `sidechain` and `pad` null; samplers get default `slices` (off, base note 36,
///   no markers);
/// - audio clips: `fade_in_curve`/`fade_out_curve` `Linear` (the v2 fade law), `reversed`
///   false.
///
/// Fields already present are kept (idempotent).
pub struct V2RoadmapDefaults;

impl Migration for V2RoadmapDefaults {
    fn source_version(&self) -> u32 {
        2
    }

    fn migrate(&self, doc: &mut serde_json::Value) -> Result<(), FileError> {
        use serde_json::{Value, json};
        fn set_default(obj: &mut serde_json::Map<String, Value>, key: &str, value: Value) {
            obj.entry(key.to_string()).or_insert(value);
        }
        let project = doc["project"]
            .as_object_mut()
            .ok_or_else(|| FileError::Migration {
                from: 2,
                message: "no project object".into(),
            })?;
        for table in ["markers", "midi_mappings", "drum_pads"] {
            set_default(project, table, json!({}));
        }
        if let Some(settings) = project.get_mut("settings").and_then(Value::as_object_mut) {
            set_default(settings, "metronome_volume", json!(-6.0));
            set_default(settings, "metronome_accent", json!(true));
            set_default(settings, "metronome_sound", json!("Classic"));
            set_default(settings, "swing", json!(0.0));
            set_default(settings, "swing_grid", json!(0.25));
        }
        if let Some(devices) = project.get_mut("devices").and_then(Value::as_object_mut) {
            for d in devices.values_mut().filter_map(Value::as_object_mut) {
                set_default(d, "sidechain", Value::Null);
                set_default(d, "pad", Value::Null);
                if let Some(device) = d
                    .get_mut("kind")
                    .filter(|k| k["type"] == "Builtin")
                    .and_then(|k| k.get_mut("device"))
                    .and_then(Value::as_object_mut)
                    .filter(|b| b.get("type").and_then(Value::as_str) == Some("Sampler"))
                {
                    set_default(
                        device,
                        "slices",
                        json!({"enabled": false, "base_note": 36, "markers": []}),
                    );
                }
            }
        }
        if let Some(clips) = project.get_mut("clips").and_then(Value::as_object_mut) {
            for c in clips.values_mut() {
                if let Some(content) = c
                    .get_mut("content")
                    .filter(|c| c["type"] == "Audio")
                    .and_then(Value::as_object_mut)
                {
                    set_default(content, "fade_in_curve", json!({"type": "Linear"}));
                    set_default(content, "fade_out_curve", json!({"type": "Linear"}));
                    set_default(content, "reversed", json!(false));
                }
            }
        }
        Ok(())
    }
}

/// v3 → v4: v0.2 (contracts-3) tables and fields, all neutral (a migrated project sounds and
/// behaves exactly as before).
///
/// - tables `take_lanes`, `comp_regions`, `rack_chains`, `modulators`, `mod_mappings`,
///   and base-62's `chat`, `pinned_notes` (empty);
/// - media: `location` `Project` (v0.1 imports were copied into the project; v0.2 imports
///   reference library files in place, see `crate::media`);
/// - `Clip::lane`, `Device::chain`, `Track::freeze` stay absent (= `None`).
///
/// The version bump itself is the point: a v0.1 app refuses v4 files (`TooNew`) instead of
/// silently dropping takes, racks, modulation, the chat journal and pinned notes on re-save. Fields already present are kept
/// (idempotent).
pub struct V3ContractsV3Defaults;

impl Migration for V3ContractsV3Defaults {
    fn source_version(&self) -> u32 {
        3
    }

    fn migrate(&self, doc: &mut serde_json::Value) -> Result<(), FileError> {
        use serde_json::{Value, json};
        let project = doc["project"]
            .as_object_mut()
            .ok_or_else(|| FileError::Migration {
                from: 3,
                message: "no project object".into(),
            })?;
        for table in [
            "take_lanes",
            "comp_regions",
            "rack_chains",
            "modulators",
            "mod_mappings",
            "chat",
            "pinned_notes",
        ] {
            project.entry(table.to_string()).or_insert(json!({}));
        }
        if let Some(media) = project.get_mut("media").and_then(Value::as_object_mut) {
            for m in media.values_mut().filter_map(Value::as_object_mut) {
                m.entry("location".to_string())
                    .or_insert(json!({"type": "Project"}));
            }
        }
        Ok(())
    }
}

/// Parse, migrate and validate an `.ether` document.
pub fn load(json: &str) -> Result<Project, FileError> {
    load_with(json, &migrations(), CURRENT_VERSION)
}

/// [`load`] with an explicit migration set and target version (tests the framework; the
/// app always uses [`load`]).
///
/// Each migration receives the whole file document (`{format, version, app_version,
/// project}`); the loader bumps `version` after each step.
pub fn load_with(
    json: &str,
    migrations: &[Box<dyn Migration>],
    target_version: u32,
) -> Result<Project, FileError> {
    let mut doc: serde_json::Value = serde_json::from_str(json)?;
    let obj = doc
        .as_object()
        .ok_or_else(|| FileError::NotAnEtherFile("top level is not an object".into()))?;
    if obj.get("format").and_then(|f| f.as_str()) != Some(FORMAT_TAG) {
        return Err(FileError::NotAnEtherFile(format!(
            "missing or wrong \"format\" (expected {FORMAT_TAG:?})"
        )));
    }
    let version = obj
        .get("version")
        .and_then(|v| v.as_u64())
        .and_then(|v| u32::try_from(v).ok())
        .filter(|v| *v >= 1)
        .ok_or_else(|| FileError::NotAnEtherFile("missing or invalid \"version\"".into()))?;
    if version > target_version {
        return Err(FileError::TooNew {
            found: version,
            supported: target_version,
        });
    }
    for from in version..target_version {
        let m = migrations
            .iter()
            .find(|m| m.source_version() == from)
            .ok_or_else(|| FileError::Migration {
                from,
                message: "no migration registered".into(),
            })?;
        m.migrate(&mut doc)?;
        doc["version"] = serde_json::Value::from(from + 1);
    }
    let file: EtherFile = serde_json::from_value(doc)?;
    file.project.validate()?;
    Ok(file.project)
}

/// Serialize a project to pretty JSON at [`CURRENT_VERSION`] (stable key order: tables are
/// `BTreeMap`s, so saves are diff-friendly).
pub fn save(project: &Project, app_version: &str) -> Result<String, FileError> {
    #[derive(Serialize)]
    struct Out<'a> {
        format: &'a str,
        version: u32,
        app_version: &'a str,
        project: &'a Project,
    }
    let mut s = serde_json::to_string_pretty(&Out {
        format: FORMAT_TAG,
        version: CURRENT_VERSION,
        app_version,
        project,
    })?;
    s.push('\n');
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::IdGen;

    struct RenameTitle;
    impl Migration for RenameTitle {
        fn source_version(&self) -> u32 {
            1
        }
        fn migrate(&self, doc: &mut serde_json::Value) -> Result<(), FileError> {
            let settings =
                doc["project"]["settings"]
                    .as_object_mut()
                    .ok_or(FileError::Migration {
                        from: 1,
                        message: "no settings".into(),
                    })?;
            let title = settings.remove("title").unwrap_or_default();
            settings.insert("name".into(), title);
            Ok(())
        }
    }

    struct Noop(u32);
    impl Migration for Noop {
        fn source_version(&self) -> u32 {
            self.0
        }
        fn migrate(&self, _: &mut serde_json::Value) -> Result<(), FileError> {
            Ok(())
        }
    }

    fn project() -> Project {
        Project::new(&mut IdGen::new(7), 1_700_000_000_000)
    }

    #[test]
    fn save_load_roundtrip() {
        let p = project();
        let json = save(&p, "0.1.0").unwrap();
        assert!(json.contains("\"format\": \"ethereal-project\""));
        assert_eq!(load(&json).unwrap(), p);
        // Saves are deterministic.
        assert_eq!(save(&load(&json).unwrap(), "0.1.0").unwrap(), json);
    }

    #[test]
    fn v1_session_migration() {
        let p = project();
        let mut doc: serde_json::Value = serde_json::from_str(&save(&p, "0.1.0").unwrap()).unwrap();
        doc["version"] = 1.into();
        let project = &mut doc["project"];
        project["settings"]["launch_quantization"] =
            serde_json::json!({"type": "Bars", "count": 1});
        project["scenes"] = serde_json::json!({"Scene_01": {"id": "Scene_01", "name": "1"}});
        project["notes"] = serde_json::json!({
            "Note_s": {"clip": "Clip_s"}
        });
        project["automation_lanes"] = serde_json::json!({
            "Lane_s": {"owner": {"type": "Clip", "clip": "Clip_s"}}
        });
        project["automation_points"] = serde_json::json!({
            "Point_s": {"lane": "Lane_s"}
        });
        project["clips"] = serde_json::json!({
            "Clip_s": {"location": {"type": "Session", "scene": "Scene_01"}}
        });
        let json = doc.to_string();
        // Only session things were added: migration restores the v2 project exactly.
        assert_eq!(load(&json).unwrap(), p);

        // Arrangement clips get a flat `start`.
        let mut v: serde_json::Value = serde_json::json!({"project": {"clips": {
            "Clip_a": {"location": {"type": "Arrangement", "start": 8.0}, "launch": {}}
        }}});
        V1RemoveSession.migrate(&mut v).unwrap();
        assert_eq!(
            v["project"]["clips"]["Clip_a"],
            serde_json::json!({"start": 8.0})
        );
    }

    #[test]
    fn migrations_run_in_order_up_to_target() {
        let p = project();
        let mut doc: serde_json::Value = serde_json::from_str(&save(&p, "0.0.1").unwrap()).unwrap();
        // Pretend a v1 file stored the name as `title`.
        doc["version"] = 1.into();
        let settings = doc["project"]["settings"].as_object_mut().unwrap();
        let name = settings.remove("name").unwrap();
        settings.insert("title".into(), name);
        let json = doc.to_string();

        let ms: Vec<Box<dyn Migration>> = vec![Box::new(Noop(2)), Box::new(RenameTitle)];
        assert_eq!(load_with(&json, &ms, 3).unwrap(), p);
        // Missing step.
        let ms: Vec<Box<dyn Migration>> = vec![Box::new(RenameTitle)];
        assert!(matches!(
            load_with(&json, &ms, 3),
            Err(FileError::Migration { from: 2, .. })
        ));
        // Without the migration the old shape fails to deserialize.
        assert!(matches!(load(&json), Err(FileError::Json(_))));
    }

    #[test]
    fn v2_roadmap_migration_is_idempotent_and_neutral() {
        let p = project();
        let mut doc: serde_json::Value = serde_json::from_str(&save(&p, "0.1.0").unwrap()).unwrap();
        // Already-v3 content passes through unchanged.
        let before = doc.clone();
        V2RoadmapDefaults.migrate(&mut doc).unwrap();
        assert_eq!(doc, before);
        // Strip the v3 additions: a v2 document loads back to the same project.
        doc["version"] = 2.into();
        let project = doc["project"].as_object_mut().unwrap();
        for t in ["markers", "midi_mappings", "drum_pads"] {
            project.remove(t);
        }
        let settings = project["settings"].as_object_mut().unwrap();
        for f in [
            "metronome_volume",
            "metronome_accent",
            "metronome_sound",
            "swing",
            "swing_grid",
        ] {
            settings.remove(f);
        }
        assert_eq!(load(&doc.to_string()).unwrap(), p);
    }

    #[test]
    fn v3_migration_is_idempotent_and_neutral() {
        let p = project();
        let mut doc: serde_json::Value = serde_json::from_str(&save(&p, "0.2.0").unwrap()).unwrap();
        let before = doc.clone();
        V3ContractsV3Defaults.migrate(&mut doc).unwrap();
        assert_eq!(doc, before);
        doc["version"] = 3.into();
        let project = doc["project"].as_object_mut().unwrap();
        for t in [
            "take_lanes",
            "comp_regions",
            "rack_chains",
            "modulators",
            "mod_mappings",
            "chat",
            "pinned_notes",
        ] {
            assert!(project.remove(t).is_some(), "{t}");
        }
        assert_eq!(load(&doc.to_string()).unwrap(), p);
    }

    #[test]
    fn rejects_bad_files() {
        assert!(matches!(load("[]"), Err(FileError::NotAnEtherFile(_))));
        assert!(matches!(
            load(r#"{"format":"other","version":1}"#),
            Err(FileError::NotAnEtherFile(_))
        ));
        assert!(matches!(
            load(r#"{"format":"ethereal-project","version":0}"#),
            Err(FileError::NotAnEtherFile(_))
        ));
        assert!(matches!(
            load(r#"{"format":"ethereal-project","version":99}"#),
            Err(FileError::TooNew {
                found: 99,
                supported: CURRENT_VERSION
            })
        ));
        assert!(matches!(load("{"), Err(FileError::Json(_))));

        // Structurally fine but invalid: no master track.
        let mut p = project();
        p.tracks.clear();
        let json = save(&p, "0.1.0").unwrap();
        assert!(matches!(load(&json), Err(FileError::Invalid(_))));
    }
}

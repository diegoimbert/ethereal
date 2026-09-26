//! The `.ether` file format: versioned JSON with migrations from day one.
//!
//! ```json
//! { "format": "ethereal-project", "version": 1, "app_version": "0.1.0", "project": { ... } }
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
pub const CURRENT_VERSION: u32 = 1;
/// File extension (without dot).
pub const EXTENSION: &str = "ether";
/// Document file name inside a project folder.
pub const PROJECT_FILE: &str = "project.ether";
/// Imported media folder inside a project folder.
pub const MEDIA_DIR: &str = "media";
/// Regenerable caches (peak mipmaps, decoded audio) inside a project folder.
pub const CACHE_DIR: &str = "cache";

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

/// All migrations, in order. Empty at version 1.
pub fn migrations() -> Vec<Box<dyn Migration>> {
    Vec::new()
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
    fn migrations_run_in_order_up_to_target() {
        let p = project();
        let mut doc: serde_json::Value = serde_json::from_str(&save(&p, "0.0.1").unwrap()).unwrap();
        // Pretend a v1 file stored the name as `title`.
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
                supported: 1
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

//! The `.ether` file format: versioned JSON with up/down migrations.
//!
//! ```json
//! { "format": "ethereal-project", "format_version": 1, "app_version": "0.6.0", "project": { ... } }
//! ```
//!
//! - `format` names the kind of document; `format_version` is the version of the document
//!   schema (one integer for the whole document, independent of the app version);
//!   `app_version` is informational (the app that wrote the file).
//! - **Format 1 is the public-release baseline.** Documents written by pre-release builds
//!   (`"version": 1..=5`, no `format_version`) are refused with a clear message; they are
//!   not migrated.
//! - Every breaking schema change bumps [`CURRENT_VERSION`] and registers a [`Migration`]
//!   from the previous version. "Breaking" includes *additive* changes that an older reader
//!   would silently drop on re-save (a new field, table or enum variant): older readers
//!   ignore unknown fields, so the bump is what makes them refuse the file ([`FileError::TooNew`])
//!   instead of losing data.
//!
//! Loading ([`load`]): parse to `serde_json::Value`, read `format_version`, run the `up`
//! migrations to [`CURRENT_VERSION`] on the untyped value, then deserialize and `validate`.
//! A document newer than [`CURRENT_VERSION`] is refused (an app cannot know the migrations
//! of a future version, so in practice there is never a down path at load time).
//!
//! `down` migrations exist so every change is reversible and tested both ways, and so a
//! document can be written for an older reader ([`convert`], e.g. a future "save for an
//! older Ethereal"). No app path uses them yet.
//!
//! On disk (engine-side `ProjectStore`), a project is a folder named by its UUIDv7:
//! `<projects_root>/<project-uuid>/{project.ether, media/, cache/}`. Media paths in the
//! document are relative to that folder (`media/...`). The display name lives in the file
//! (`project.settings.name`), so renaming never moves the folder. The same code reads and
//! writes the document on desktop (folders) and on the web (OPFS in the engine worker),
//! in portable bundles (`project.ether` inside the ZIP), versions (`versions/*.ether`),
//! collab snapshots and static shares.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::error::FileError;
use crate::project::Project;

/// Magic string in the `format` field.
pub const FORMAT_TAG: &str = "ethereal-project";
/// Current `.ether` format version. Bump + add a [`Migration`] for every schema change an
/// older reader cannot round-trip (module docs).
pub const CURRENT_VERSION: u32 = 1;
/// Name of the version field.
pub const VERSION_FIELD: &str = "format_version";
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
    /// Document schema version ([`CURRENT_VERSION`] when written by this app).
    pub format_version: u32,
    /// Version of the app that wrote the file (informational).
    pub app_version: String,
    pub project: Project,
}

/// One schema step between `source_version()` and `source_version() + 1`, on the untyped
/// document (`{format, format_version, app_version, project}`). The runner updates
/// `format_version` after each step; a migration only rewrites the content.
pub trait Migration: Send + Sync {
    /// The version `up` reads (and `down` writes).
    fn source_version(&self) -> u32;
    /// `source_version()` → `source_version() + 1`.
    fn up(&self, doc: &mut Value) -> Result<(), FileError>;
    /// `source_version() + 1` → `source_version()`: the document an older reader can load.
    /// Lossy when the newer version stores something the older one cannot express (the
    /// migration documents what is dropped).
    fn down(&self, doc: &mut Value) -> Result<(), FileError>;
}

/// All migrations, in order (`source_version` 1, 2, ...). Empty while format 1 is current.
pub fn migrations() -> Vec<Box<dyn Migration>> {
    Vec::new()
}

/// The `format_version` of an `.ether` document (after checking `format`).
pub fn document_version(doc: &Value) -> Result<u32, FileError> {
    let obj = doc
        .as_object()
        .ok_or_else(|| FileError::NotAnEtherFile("top level is not an object".into()))?;
    if obj.get("format").and_then(Value::as_str) != Some(FORMAT_TAG) {
        return Err(FileError::NotAnEtherFile(format!(
            "missing or wrong \"format\" (expected {FORMAT_TAG:?})"
        )));
    }
    match obj.get(VERSION_FIELD) {
        Some(v) => v
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .filter(|v| *v >= 1)
            .ok_or_else(|| FileError::NotAnEtherFile(format!("invalid \"{VERSION_FIELD}\""))),
        // Pre-release documents carried `"version"` (1..=5) instead.
        None if obj.contains_key("version") => Err(FileError::PreRelease),
        None => Err(FileError::NotAnEtherFile(format!(
            "missing \"{VERSION_FIELD}\""
        ))),
    }
}

/// Migrate an untyped document from its version to `target`, up or down, with `migrations`.
pub fn migrate(
    doc: &mut Value,
    migrations: &[Box<dyn Migration>],
    target: u32,
) -> Result<(), FileError> {
    let version = document_version(doc)?;
    let step = |from: u32| {
        migrations
            .iter()
            .find(|m| m.source_version() == from)
            .ok_or_else(|| FileError::Migration {
                from,
                message: "no migration registered".into(),
            })
    };
    if version > target {
        // Down needs every step from `version` to `target`; without them the file is just
        // too new for this reader.
        if (target..version).any(|from| step(from).is_err()) {
            return Err(FileError::TooNew {
                found: version,
                supported: target,
            });
        }
        for from in (target..version).rev() {
            step(from)?.down(doc)?;
            doc[VERSION_FIELD] = Value::from(from);
        }
    } else {
        for from in version..target {
            step(from)?.up(doc)?;
            doc[VERSION_FIELD] = Value::from(from + 1);
        }
    }
    Ok(())
}

/// Parse, migrate and validate an `.ether` document.
pub fn load(json: &str) -> Result<Project, FileError> {
    load_with(json, &migrations(), CURRENT_VERSION)
}

/// [`load`] with an explicit migration set and target version (tests the framework; the
/// app always uses [`load`]).
pub fn load_with(
    json: &str,
    migrations: &[Box<dyn Migration>],
    target_version: u32,
) -> Result<Project, FileError> {
    let mut doc: Value = serde_json::from_str(json)?;
    migrate(&mut doc, migrations, target_version)?;
    let mut file: EtherFile = serde_json::from_value(doc)?;
    file.project.ensure_index();
    file.project.validate()?;
    Ok(file.project)
}

/// Rewrite an `.ether` document (JSON text) at `target` with `migrations` (up or down;
/// pretty JSON). For writing a document an older reader can load.
pub fn convert(
    json: &str,
    migrations: &[Box<dyn Migration>],
    target: u32,
) -> Result<String, FileError> {
    let mut doc: Value = serde_json::from_str(json)?;
    migrate(&mut doc, migrations, target)?;
    let mut s = serde_json::to_string_pretty(&doc)?;
    s.push('\n');
    Ok(s)
}

/// Serialize a project to pretty JSON at [`CURRENT_VERSION`] (stable key order: tables are
/// `BTreeMap`s, so saves are diff-friendly).
pub fn save(project: &Project, app_version: &str) -> Result<String, FileError> {
    let mut s = serde_json::to_string_pretty(&Out::new(project, app_version))?;
    s.push('\n');
    Ok(s)
}

/// [`save`] as compact JSON: the same document, for files only programs read (a static
/// share's `view.json`).
pub fn save_compact(project: &Project, app_version: &str) -> Result<String, FileError> {
    Ok(serde_json::to_string(&Out::new(project, app_version))?)
}

/// The document [`save`] writes ([`EtherFile`] by reference).
#[derive(Serialize)]
struct Out<'a> {
    format: &'a str,
    format_version: u32,
    app_version: &'a str,
    project: &'a Project,
}

impl<'a> Out<'a> {
    fn new(project: &'a Project, app_version: &'a str) -> Self {
        Self {
            format: FORMAT_TAG,
            format_version: CURRENT_VERSION,
            app_version,
            project,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::IdGen;

    /// Test-only v1 → v2: `settings.name` is renamed `settings.title`. Down renames it back.
    struct RenameNameToTitle;
    impl Migration for RenameNameToTitle {
        fn source_version(&self) -> u32 {
            1
        }
        fn up(&self, doc: &mut Value) -> Result<(), FileError> {
            rename(doc, "name", "title", 1)
        }
        fn down(&self, doc: &mut Value) -> Result<(), FileError> {
            rename(doc, "title", "name", 1)
        }
    }

    /// Test-only v2 → v3 that changes nothing.
    struct Noop(u32);
    impl Migration for Noop {
        fn source_version(&self) -> u32 {
            self.0
        }
        fn up(&self, _: &mut Value) -> Result<(), FileError> {
            Ok(())
        }
        fn down(&self, _: &mut Value) -> Result<(), FileError> {
            Ok(())
        }
    }

    fn rename(doc: &mut Value, from: &str, to: &str, version: u32) -> Result<(), FileError> {
        let settings = doc["project"]["settings"]
            .as_object_mut()
            .ok_or(FileError::Migration {
                from: version,
                message: "no settings".into(),
            })?;
        let v = settings.remove(from).ok_or(FileError::Migration {
            from: version,
            message: format!("no settings.{from}"),
        })?;
        settings.insert(to.into(), v);
        Ok(())
    }

    fn project() -> Project {
        Project::new(&mut IdGen::new(7), 1_700_000_000_000)
    }

    #[test]
    fn save_load_roundtrip() {
        let p = project();
        let json = save(&p, "0.1.0").unwrap();
        assert!(json.contains("\"format\": \"ethereal-project\""));
        assert!(json.contains("\"format_version\": 1"));
        assert_eq!(load(&json).unwrap(), p);
        // Saves are deterministic.
        assert_eq!(save(&load(&json).unwrap(), "0.1.0").unwrap(), json);
        // The compact form is the same document.
        let compact = save_compact(&p, "0.1.0").unwrap();
        assert!(!compact.contains('\n'));
        let parse = |s: &str| serde_json::from_str::<Value>(s).unwrap();
        assert_eq!(parse(&compact), parse(&json));
    }

    #[test]
    fn up_and_down_migrations() {
        let p = project();
        let v1 = save(&p, "0.6.0").unwrap();
        let ms: Vec<Box<dyn Migration>> = vec![Box::new(Noop(2)), Box::new(RenameNameToTitle)];

        // Up: v1 → v3 renames the field and stamps the version.
        let v3 = convert(&v1, &ms, 3).unwrap();
        let doc: Value = serde_json::from_str(&v3).unwrap();
        assert_eq!(doc[VERSION_FIELD], 3);
        assert_eq!(doc["project"]["settings"]["title"], "Untitled");
        assert!(doc["project"]["settings"].get("name").is_none());

        // Down: v3 → v1 restores the original document exactly.
        let back = convert(&v3, &ms, 1).unwrap();
        let parse = |s: &str| serde_json::from_str::<Value>(s).unwrap();
        assert_eq!(parse(&back), parse(&v1));
        assert_eq!(load(&back).unwrap(), p);

        // A reader at v1 that knows the down path loads the v3 document by going down.
        assert_eq!(load_with(&v3, &ms, 1).unwrap(), p);

        // Missing step: refused, naming it.
        let partial: Vec<Box<dyn Migration>> = vec![Box::new(RenameNameToTitle)];
        assert!(matches!(
            convert(&v1, &partial, 3),
            Err(FileError::Migration { from: 2, .. })
        ));
        // Without a down path, a newer document is too new.
        assert!(matches!(
            convert(&v3, &partial, 1),
            Err(FileError::TooNew {
                found: 3,
                supported: 1
            })
        ));
        // The real reader has no migration from 1 and does not know `title`: the v3
        // document is refused before deserialization.
        assert!(matches!(
            load(&v3),
            Err(FileError::TooNew {
                found: 3,
                supported: CURRENT_VERSION
            })
        ));
    }

    #[test]
    fn too_new_is_refused_with_a_clear_message() {
        let mut doc: Value = serde_json::from_str(&save(&project(), "9.0.0").unwrap()).unwrap();
        doc[VERSION_FIELD] = (CURRENT_VERSION + 1).into();
        let e = load(&doc.to_string()).unwrap_err();
        assert!(matches!(e, FileError::TooNew { .. }));
        assert!(
            e.to_string()
                .starts_with("this file was made with a newer version of Ethereal"),
            "{e}"
        );
    }

    #[test]
    fn rejects_bad_files() {
        assert!(matches!(load("[]"), Err(FileError::NotAnEtherFile(_))));
        assert!(matches!(
            load(r#"{"format":"other","format_version":1}"#),
            Err(FileError::NotAnEtherFile(_))
        ));
        assert!(matches!(
            load(r#"{"format":"ethereal-project","format_version":0}"#),
            Err(FileError::NotAnEtherFile(_))
        ));
        assert!(matches!(
            load(r#"{"format":"ethereal-project","format_version":"1"}"#),
            Err(FileError::NotAnEtherFile(_))
        ));
        // No version at all.
        assert!(matches!(
            load(r#"{"format":"ethereal-project","project":{}}"#),
            Err(FileError::NotAnEtherFile(_))
        ));
        // A pre-release document (`"version"`, no `format_version`).
        let e = load(r#"{"format":"ethereal-project","version":5,"project":{}}"#).unwrap_err();
        assert!(matches!(e, FileError::PreRelease), "{e}");
        assert!(e.to_string().contains("pre-release"), "{e}");
        assert!(matches!(load("{"), Err(FileError::Json(_))));

        // Structurally fine but invalid: no master track.
        let mut p = project();
        p.tracks.clear();
        let json = save(&p, "0.1.0").unwrap();
        assert!(matches!(load(&json), Err(FileError::Invalid(_))));
    }
}

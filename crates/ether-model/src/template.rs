//! Project and track templates (v0.3, `templates`; CONTRACTS.md §13.10).
//!
//! ```json
//! { "format": "ethereal-template", "format_version": 1, "project_format_version": 1,
//!   "app_version": "0.6.0",
//!   "template": { "name": "Vocal chain", "meta": { "tags": ["vocal"] },
//!                 "body": { "type": "Tracks", "entities": [ ... ], "samples": [] } } }
//! ```
//!
//! - **Project templates** (`TemplateBody::Project`) hold a whole document. "New project from
//!   template" copies it with fresh ids (a new `ProjectId`; entity ids are kept: they are
//!   unique per project) and the new name; its media are carried as [`PresetSample`]s
//!   (library locations, imported/referenced first, like sample-based presets).
//! - **Track templates** (`TemplateBody::Tracks`) hold one or more tracks with everything
//!   that hangs off them except clips: their devices (with drum pads, rack chains, chain and
//!   pad devices), modulators and modulation mappings, sends between template tracks, and
//!   track automation lanes and points; as entities in [`crate::Project::entities`] order
//!   (parents first). Inserting a track template is one undo step; every entity gets
//!   `derive_id(seed, i)` where `i` is its index in `entities` (collab-safe,
//!   [`crate::derive_id`]), and references between template entities are remapped. Sends to
//!   return tracks outside the template are dropped; outputs to tracks outside the template
//!   reset to `Default`.
//! - Templates are files in the engine-side user library
//!   (`<user library>/Templates/{Projects,Tracks}/<name>.ethertemplate`); the default
//!   project template's id is stored in `<user library>/Templates/default.json`. The UI never
//!   touches files.
//! - `format_version` is the version of the template container ([`TEMPLATE_VERSION`]);
//!   `project_format_version` is the `.ether` format ([`crate::file::CURRENT_VERSION`]) the
//!   body was written with. Loading an older body runs the `.ether` migrations on it
//!   ([`migrate_body`]: a project body is wrapped in a document; a tracks body's entities are
//!   put in their tables of a minimal document, then read back in their order); newer ones
//!   are rejected (`TooNew`). Pre-release templates (`version` / `ether_version`) are
//!   refused (`PreRelease`).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use ts_rs::TS;

use crate::entity::Entity;
use crate::error::FileError;
use crate::file::{self, CURRENT_VERSION, FORMAT_TAG, Migration, VERSION_FIELD};
use crate::preset::{PresetMeta, PresetSample};
use crate::project::Project;

/// Magic string in the `format` field.
pub const TEMPLATE_FORMAT_TAG: &str = "ethereal-template";
/// Current template file version.
pub const TEMPLATE_VERSION: u32 = 1;
/// File extension (without dot).
pub const TEMPLATE_EXTENSION: &str = "ethertemplate";
/// Template folder inside the user library root.
pub const TEMPLATES_DIR: &str = "Templates";

/// A template file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct TemplateFile {
    pub format: String,
    /// Template container version ([`TEMPLATE_VERSION`]).
    pub format_version: u32,
    /// `.ether` format version of the entities/project inside.
    pub project_format_version: u32,
    /// Version of the app that wrote the file (informational).
    pub app_version: String,
    /// When the file was written (Unix ms; `TemplateInfo::modified_ms`, since library
    /// listings carry no file times). Omitted when 0 (`templates`, additive).
    #[serde(default, skip_serializing_if = "is_zero")]
    #[ts(type = "number")]
    pub saved_ms: u64,
    pub template: Template,
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Template {
    pub name: String,
    #[serde(default)]
    pub meta: PresetMeta,
    pub body: TemplateBody,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum TemplateBody {
    Project {
        project: Box<Project>,
        /// Media of the project, by library location.
        #[serde(default)]
        samples: Vec<PresetSample>,
    },
    Tracks {
        /// Parents first (see the module docs). At least one track.
        entities: Vec<Entity>,
        #[serde(default)]
        samples: Vec<PresetSample>,
    },
}

/// Parse and check a template file (format tag, versions, a project body validates, a
/// tracks body starts with a track).
pub fn load_template(json: &str) -> Result<Template, FileError> {
    load_template_file(json).map(|f| f.template)
}

/// [`load_template`] keeping the file header (`saved_ms`, versions).
pub fn load_template_file(json: &str) -> Result<TemplateFile, FileError> {
    load_template_file_with(json, &file::migrations(), CURRENT_VERSION)
}

/// [`load_template_file`] with an explicit `.ether` migration set and target (tests).
pub fn load_template_file_with(
    json: &str,
    migrations: &[Box<dyn Migration>],
    target: u32,
) -> Result<TemplateFile, FileError> {
    let mut doc: Value = serde_json::from_str(json)?;
    if doc.get("format").and_then(|f| f.as_str()) != Some(TEMPLATE_FORMAT_TAG) {
        return Err(FileError::NotAnEtherFile(format!(
            "missing or wrong \"format\" (expected {TEMPLATE_FORMAT_TAG:?})"
        )));
    }
    if doc.get(VERSION_FIELD).is_none() && doc.get("version").is_some() {
        return Err(FileError::PreRelease);
    }
    let version = |field: &str| {
        doc.get(field)
            .and_then(|v| v.as_u64())
            .and_then(|v| u32::try_from(v).ok())
            .filter(|v| *v >= 1)
            .ok_or_else(|| FileError::NotAnEtherFile(format!("missing or invalid \"{field}\"")))
    };
    let (version, body_version) = (version(VERSION_FIELD)?, version("project_format_version")?);
    if version > TEMPLATE_VERSION {
        return Err(FileError::TooNew {
            found: version,
            supported: TEMPLATE_VERSION,
        });
    }
    if body_version != target {
        let body = doc
            .pointer_mut("/template/body")
            .ok_or_else(|| FileError::NotAnEtherFile("no template body".into()))?;
        migrate_body(body, body_version, migrations, target)?;
        doc["project_format_version"] = target.into();
    }
    let file: TemplateFile = serde_json::from_value(doc)?;
    match &file.template.body {
        TemplateBody::Project { project, .. } => project.validate()?,
        TemplateBody::Tracks { entities, .. } => {
            if !matches!(entities.first(), Some(Entity::Track(_))) {
                return Err(FileError::Migration {
                    from: body_version,
                    message: "a track template starts with a track".into(),
                });
            }
        }
    }
    Ok(file)
}

/// The project table holding each entity type (`Entity`'s `type` tag).
fn entity_table(ty: &str) -> Option<&'static str> {
    Some(match ty {
        "Track" => "tracks",
        "Clip" => "clips",
        "Note" => "notes",
        "Device" => "devices",
        "Send" => "sends",
        "AutomationLane" => "automation_lanes",
        "AutomationPoint" => "automation_points",
        "TempoPoint" => "tempo_points",
        "TimeSignature" => "time_signatures",
        "WarpMarker" => "warp_markers",
        "Media" => "media",
        "Marker" => "markers",
        "MidiMapping" => "midi_mappings",
        "DrumPad" => "drum_pads",
        "TakeLane" => "take_lanes",
        "CompRegion" => "comp_regions",
        "RackChain" => "rack_chains",
        "Modulator" => "modulators",
        "ModMapping" => "mod_mappings",
        "ChatMessage" => "chat",
        "PinnedNote" => "pinned_notes",
        "ExpressionLane" => "expression_lanes",
        "NoteExpression" => "note_expressions",
        _ => return None,
    })
}

/// Migrate a template body (untyped) from `.ether` format `from` to `target` (up or down)
/// with the document migrations. A project body is migrated as a document. A tracks body's
/// entities are put in their tables of a minimal document (only the tables they use; no
/// `settings`: document migrations must tolerate missing tables), migrated, and read back
/// in their original order (entities a migration removes are dropped; ones it adds are not
/// picked up).
pub fn migrate_body(
    body: &mut Value,
    from: u32,
    migrations: &[Box<dyn Migration>],
    target: u32,
) -> Result<(), FileError> {
    let wrap = |project: Value| json!({ "format": FORMAT_TAG, VERSION_FIELD: from, "app_version": "", "project": project });
    let bad = |m: &str| FileError::Migration {
        from,
        message: m.into(),
    };
    match body.get("type").and_then(Value::as_str) {
        Some("Project") => {
            let mut doc = wrap(body["project"].take());
            file::migrate(&mut doc, migrations, target)?;
            body["project"] = doc["project"].take();
        }
        Some("Tracks") => {
            let entities = body["entities"]
                .as_array()
                .ok_or_else(|| bad("tracks template without entities"))?;
            let mut order = Vec::with_capacity(entities.len());
            let mut tables: Map<String, Value> = Map::new();
            for e in entities {
                let ty = e["type"]
                    .as_str()
                    .ok_or_else(|| bad("entity without type"))?;
                let table = entity_table(ty).ok_or_else(|| bad("unknown entity type"))?;
                let id = e["value"]["id"]
                    .as_str()
                    .ok_or_else(|| bad("entity without id"))?
                    .to_string();
                if let Some(t) = tables
                    .entry(table)
                    .or_insert_with(|| json!({}))
                    .as_object_mut()
                {
                    t.insert(id.clone(), e["value"].clone());
                }
                order.push((ty.to_string(), table, id));
            }
            let mut doc = wrap(Value::Object(tables));
            file::migrate(&mut doc, migrations, target)?;
            let project = &doc["project"];
            body["entities"] = order
                .into_iter()
                .filter_map(|(ty, table, id)| {
                    let v = project.get(table)?.get(&id)?;
                    Some(json!({ "type": ty, "value": v }))
                })
                .collect();
        }
        _ => return Err(bad("unknown template body")),
    }
    Ok(())
}

/// Serialize a template at the current versions (pretty JSON).
pub fn save_template(template: &Template, app_version: &str) -> Result<String, FileError> {
    save_template_at(template, app_version, 0)
}

/// [`save_template`] stamped with the time it is written (`saved_ms`, Unix ms).
pub fn save_template_at(
    template: &Template,
    app_version: &str,
    saved_ms: u64,
) -> Result<String, FileError> {
    let mut s = serde_json::to_string_pretty(&TemplateFile {
        format: TEMPLATE_FORMAT_TAG.into(),
        format_version: TEMPLATE_VERSION,
        project_format_version: CURRENT_VERSION,
        app_version: app_version.into(),
        saved_ms,
        template: template.clone(),
    })?;
    s.push('\n');
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::IdGen;

    #[test]
    fn roundtrip_and_checks() {
        let project = Project::new(&mut IdGen::new(3), 1_700_000_000_000);
        let master = project.master_track().clone();
        for body in [
            TemplateBody::Project {
                project: Box::new(project.clone()),
                samples: vec![],
            },
            TemplateBody::Tracks {
                entities: vec![Entity::Track(master.clone())],
                samples: vec![],
            },
        ] {
            let t = Template {
                name: "T".into(),
                meta: PresetMeta::default(),
                body,
            };
            let json = save_template(&t, "0.3.0").unwrap();
            assert_eq!(load_template(&json).unwrap(), t);
        }
        let empty = Template {
            name: "T".into(),
            meta: PresetMeta::default(),
            body: TemplateBody::Tracks {
                entities: vec![],
                samples: vec![],
            },
        };
        assert!(load_template(&save_template(&empty, "x").unwrap()).is_err());
        assert!(matches!(
            load_template(
                r#"{"format":"ethereal-template","format_version":9,"project_format_version":1}"#
            ),
            Err(FileError::TooNew { .. })
        ));
        let mut doc: Value = serde_json::from_str(&save_template(&empty, "x").unwrap()).unwrap();
        doc["project_format_version"] = 99.into();
        assert!(matches!(
            load_template(&doc.to_string()),
            Err(FileError::TooNew { found: 99, .. })
        ));
        assert!(matches!(
            load_template(r#"{"format":"ethereal-template","version":1,"ether_version":5}"#),
            Err(FileError::PreRelease)
        ));
        assert!(load_template(r#"{"format":"ethereal-preset","version":1}"#).is_err());
    }

    /// Test-only `.ether` v1 → v2: tracks' `name` is stored as `label`.
    struct TrackLabel;
    impl Migration for TrackLabel {
        fn source_version(&self) -> u32 {
            1
        }
        fn up(&self, doc: &mut Value) -> Result<(), FileError> {
            rename_track_field(doc, "name", "label");
            Ok(())
        }
        fn down(&self, doc: &mut Value) -> Result<(), FileError> {
            rename_track_field(doc, "label", "name");
            Ok(())
        }
    }

    fn rename_track_field(doc: &mut Value, from: &str, to: &str) {
        if let Some(tracks) = doc["project"]
            .get_mut("tracks")
            .and_then(Value::as_object_mut)
        {
            for t in tracks.values_mut().filter_map(Value::as_object_mut) {
                if let Some(v) = t.remove(from) {
                    t.insert(to.into(), v);
                }
            }
        }
    }

    #[test]
    fn bodies_migrate_with_the_document_migrations() {
        let project = Project::new(&mut IdGen::new(3), 1_700_000_000_000);
        let master = project.master_track().clone();
        let ms: Vec<Box<dyn Migration>> = vec![Box::new(TrackLabel)];
        for body in [
            TemplateBody::Project {
                project: Box::new(project.clone()),
                samples: vec![],
            },
            TemplateBody::Tracks {
                entities: vec![Entity::Track(master.clone())],
                samples: vec![],
            },
        ] {
            let t = Template {
                name: "T".into(),
                meta: PresetMeta::default(),
                body,
            };
            // Written at v2 (by a newer app): tracks carry `label`.
            let mut doc: Value = serde_json::from_str(&save_template(&t, "x").unwrap()).unwrap();
            migrate_body(&mut doc["template"]["body"], 1, &ms, 2).unwrap();
            doc["project_format_version"] = 2.into();
            let v2 = doc.to_string();
            assert!(v2.contains(r#""label":"master""#), "{v2}");
            assert!(!v2.contains(r#""name":"master""#));
            // This reader (v1) refuses it; one that knows the down path loads it.
            assert!(matches!(
                load_template(&v2),
                Err(FileError::TooNew { found: 2, .. })
            ));
            let f = load_template_file_with(&v2, &ms, 1).unwrap();
            assert_eq!((f.template, f.project_format_version), (t, 1));
        }
    }
}

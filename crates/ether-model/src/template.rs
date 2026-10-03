//! Project and track templates (v0.3, owned by the `templates` node; CONTRACTS.md §13.10).
//!
//! ```json
//! { "format": "ethereal-template", "version": 1, "ether_version": 5, "app_version": "0.3.0",
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
//! - `ether_version` is the `.ether` version the entities were written with; loading an
//!   older one migrates them with the `.ether` migrations (the `templates` node wraps them in
//!   a minimal project document), newer ones are rejected (`TooNew`).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::entity::Entity;
use crate::error::FileError;
use crate::file::CURRENT_VERSION;
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
    pub version: u32,
    /// `.ether` version of the entities/project inside.
    pub ether_version: u32,
    /// Version of the app that wrote the file (informational).
    pub app_version: String,
    pub template: Template,
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
    let doc: serde_json::Value = serde_json::from_str(json)?;
    if doc.get("format").and_then(|f| f.as_str()) != Some(TEMPLATE_FORMAT_TAG) {
        return Err(FileError::NotAnEtherFile(format!(
            "missing or wrong \"format\" (expected {TEMPLATE_FORMAT_TAG:?})"
        )));
    }
    let version = |field: &str| {
        doc.get(field)
            .and_then(|v| v.as_u64())
            .and_then(|v| u32::try_from(v).ok())
            .ok_or_else(|| FileError::NotAnEtherFile(format!("missing or invalid \"{field}\"")))
    };
    let (version, ether_version) = (version("version")?, version("ether_version")?);
    if version > TEMPLATE_VERSION {
        return Err(FileError::TooNew {
            found: version,
            supported: TEMPLATE_VERSION,
        });
    }
    if ether_version > CURRENT_VERSION {
        return Err(FileError::TooNew {
            found: ether_version,
            supported: CURRENT_VERSION,
        });
    }
    if ether_version < CURRENT_VERSION {
        // No template predates `.ether` v5; the `templates` node adds the migration path when
        // the document format next changes.
        return Err(FileError::Migration {
            from: ether_version,
            message: "templates from older .ether versions are not migrated yet".into(),
        });
    }
    let file: TemplateFile = serde_json::from_value(doc)?;
    match &file.template.body {
        TemplateBody::Project { project, .. } => project.validate()?,
        TemplateBody::Tracks { entities, .. } => {
            if !matches!(entities.first(), Some(Entity::Track(_))) {
                return Err(FileError::Migration {
                    from: ether_version,
                    message: "a track template starts with a track".into(),
                });
            }
        }
    }
    Ok(file.template)
}

/// Serialize a template at the current versions (pretty JSON).
pub fn save_template(template: &Template, app_version: &str) -> Result<String, FileError> {
    let mut s = serde_json::to_string_pretty(&TemplateFile {
        format: TEMPLATE_FORMAT_TAG.into(),
        version: TEMPLATE_VERSION,
        ether_version: CURRENT_VERSION,
        app_version: app_version.into(),
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
            load_template(r#"{"format":"ethereal-template","version":9,"ether_version":5}"#),
            Err(FileError::TooNew { .. })
        ));
        assert!(load_template(r#"{"format":"ethereal-preset","version":1}"#).is_err());
    }
}

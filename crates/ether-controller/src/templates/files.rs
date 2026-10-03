//! Template files in the user library: ids, paths, listing, the default project template
//! (pure helpers over [`Library`]; no controller state).
//!
//! Layout (`ether_model::template`): `<user root>/Templates/Projects/<stem>.ethertemplate`,
//! `<user root>/Templates/Tracks/<stem>.ethertemplate`, and `<user root>/Templates/default.json`
//! = `{ "template": "projects/<stem>" | null }`. Ids are `"projects/<stem>"`,
//! `"tracks/<stem>"` (the stem is the sanitized name, see [`file_stem`]) and
//! `"factory/<slug>"` (embedded, read-only; [`super::factory`]).

use std::collections::BTreeSet;

use ether_core::protocol::model::PresetMeta;
use ether_core::protocol::model::template::{
    TEMPLATE_EXTENSION, TEMPLATES_DIR, TemplateBody, TemplateFile, load_template_file,
};
use ether_core::protocol::templates::{TemplateId, TemplateInfo, TemplateKind};

use crate::store::{Library, StoreError, check_relative_path};

/// Most user template files read by one listing (a runaway folder can't stall the
/// controller).
const MAX_FILES: usize = 1024;
/// Longest file stem derived from a template name (characters).
const MAX_STEM: usize = 80;
/// The default project template's file, relative to the user root.
pub(crate) const DEFAULT_FILE: &str = "Templates/default.json";
/// Prefix of factory template ids.
pub(crate) const FACTORY: &str = "factory/";

/// Where a template id points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Location {
    /// A user template: its kind and its path relative to the user root.
    User {
        kind: TemplateKind,
        path: String,
    },
    Factory {
        slug: String,
    },
}

/// Folder name and id prefix of a kind.
pub(crate) fn kind_dir(kind: TemplateKind) -> (&'static str, &'static str) {
    match kind {
        TemplateKind::Project => ("Projects", "projects"),
        TemplateKind::Tracks => ("Tracks", "tracks"),
    }
}

pub(crate) fn body_kind(body: &TemplateBody) -> TemplateKind {
    match body {
        TemplateBody::Project { .. } => TemplateKind::Project,
        TemplateBody::Tracks { .. } => TemplateKind::Tracks,
    }
}

/// The user template id of `stem`.
pub(crate) fn user_id(kind: TemplateKind, stem: &str) -> TemplateId {
    format!("{}/{stem}", kind_dir(kind).1)
}

/// Path (relative to the user root) of a user template file.
pub(crate) fn user_path(kind: TemplateKind, stem: &str) -> String {
    format!(
        "{TEMPLATES_DIR}/{}/{stem}.{TEMPLATE_EXTENSION}",
        kind_dir(kind).0
    )
}

/// Parse a template id.
pub(crate) fn locate(id: &str) -> Result<Location, StoreError> {
    let bad = || StoreError::InvalidPath(id.to_string());
    let (prefix, rest) = id.split_once('/').ok_or_else(bad)?;
    if rest.is_empty() || rest.contains('/') {
        return Err(bad());
    }
    check_relative_path(rest)?;
    let kind = match prefix {
        "factory" => {
            return Ok(Location::Factory {
                slug: rest.to_string(),
            });
        }
        "projects" => TemplateKind::Project,
        "tracks" => TemplateKind::Tracks,
        _ => return Err(bad()),
    };
    Ok(Location::User {
        kind,
        path: user_path(kind, rest),
    })
}

/// A file stem for a template name: characters that are unsafe in file names on any OS
/// become `-`, leading dots and surrounding spaces are trimmed, at most [`MAX_STEM`]
/// characters.
pub(crate) fn file_stem(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '-'
            } else {
                c
            }
        })
        .take(MAX_STEM)
        .collect();
    let trimmed = cleaned
        .trim()
        .trim_start_matches('.')
        .trim_end_matches('.')
        .trim();
    if trimmed.is_empty() {
        "Template".into()
    } else {
        trimmed.to_string()
    }
}

/// Tags lowercased, trimmed, deduplicated and sorted; empty author/description dropped
/// (same rules as presets).
pub(crate) fn normalize_meta(meta: &PresetMeta) -> PresetMeta {
    let tags: BTreeSet<String> = meta
        .tags
        .iter()
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    let text = |s: &Option<String>| {
        s.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    PresetMeta {
        tags: tags.into_iter().collect(),
        author: text(&meta.author),
        description: text(&meta.description),
    }
}

pub(crate) fn info(id: TemplateId, file: &TemplateFile, default: Option<&str>) -> TemplateInfo {
    TemplateInfo {
        default: default == Some(id.as_str()),
        factory: id.starts_with(FACTORY),
        id,
        kind: body_kind(&file.template.body),
        name: file.template.name.clone(),
        meta: file.template.meta.clone(),
        modified_ms: file.saved_ms,
    }
}

/// Read and parse a user template file.
pub(crate) fn read_user<L: Library>(
    library: &mut L,
    root: &str,
    path: &str,
) -> Result<TemplateFile, String> {
    let bytes = library.read(root, path).map_err(|e| e.to_string())?;
    let json = std::str::from_utf8(&bytes).map_err(|_| "not UTF-8".to_string())?;
    load_template_file(json).map_err(|e| e.to_string())
}

/// Every parsed user template of `kind`: `(stem, file)`. Unreadable or invalid files are
/// skipped (and so are files whose body is of the other kind); missing folders are empty.
pub(crate) fn user_templates<L: Library>(
    library: &mut L,
    root: &str,
    kind: TemplateKind,
) -> Vec<(String, TemplateFile)> {
    let dir = format!("{TEMPLATES_DIR}/{}", kind_dir(kind).0);
    let Ok(listing) = library.list_dir(root, &dir) else {
        return Vec::new();
    };
    let ext = format!(".{TEMPLATE_EXTENSION}");
    listing
        .entries
        .into_iter()
        .filter(|e| {
            !e.name.starts_with('.')
                && e.kind != ether_core::protocol::media::FileKind::Directory
                && e.name.ends_with(&ext)
        })
        .take(MAX_FILES)
        .filter_map(|e| {
            let stem = e.name.strip_suffix(&ext)?.to_string();
            let file = read_user(library, root, &format!("{dir}/{}", e.name)).ok()?;
            (body_kind(&file.template.body) == kind).then_some((stem, file))
        })
        .collect()
}

/// Stems of every template file of `kind` (valid or not).
pub(crate) fn user_stems<L: Library>(
    library: &mut L,
    root: &str,
    kind: TemplateKind,
) -> Vec<String> {
    let dir = format!("{TEMPLATES_DIR}/{}", kind_dir(kind).0);
    let ext = format!(".{TEMPLATE_EXTENSION}");
    library
        .list_dir(root, &dir)
        .map(|l| {
            l.entries
                .into_iter()
                .filter_map(|e| e.name.strip_suffix(&ext).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// The stored default project template id (`None`: none, or unreadable).
pub(crate) fn read_default<L: Library>(library: &mut L, root: &str) -> Option<TemplateId> {
    let bytes = library.read(root, DEFAULT_FILE).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.get("template")?.as_str().map(str::to_string)
}

pub(crate) fn write_default<L: Library>(
    library: &mut L,
    root: &str,
    template: Option<&str>,
) -> Result<(), StoreError> {
    let mut json = serde_json::to_string_pretty(&serde_json::json!({ "template": template }))
        .expect("plain JSON");
    json.push('\n');
    library.write_file(root, DEFAULT_FILE, json.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_stems() {
        assert_eq!(
            locate("projects/Band").unwrap(),
            Location::User {
                kind: TemplateKind::Project,
                path: "Templates/Projects/Band.ethertemplate".into()
            }
        );
        assert_eq!(
            locate("tracks/Vocal chain").unwrap(),
            Location::User {
                kind: TemplateKind::Tracks,
                path: "Templates/Tracks/Vocal chain.ethertemplate".into()
            }
        );
        assert_eq!(
            locate("factory/vocal").unwrap(),
            Location::Factory {
                slug: "vocal".into()
            }
        );
        for bad in [
            "",
            "projects",
            "projects/",
            "x/y",
            "tracks/a/b",
            "tracks/..",
            "Band",
        ] {
            assert!(locate(bad).is_err(), "{bad}");
        }
        assert_eq!(file_stem("a/b:c?"), "a-b-c-");
        assert_eq!(file_stem("  ..hidden. "), "hidden");
        assert_eq!(file_stem(""), "Template");
        assert_eq!(user_id(TemplateKind::Tracks, "Keys"), "tracks/Keys");
    }
}

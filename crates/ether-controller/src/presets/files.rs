//! Preset files in the user library: folders, file names, listing and filtering (pure
//! helpers over [`Library`]; no controller state).

use std::collections::BTreeSet;

use ether_core::protocol::model::{
    BuiltinDeviceType, PRESET_EXTENSION, PRESETS_DIR, PluginFormat, Preset, PresetDevice,
    PresetMeta, load_preset,
};
use ether_core::protocol::presets::{PresetInfo, PresetRef, PresetSource};

use crate::store::{Library, StoreError, check_relative_path};

/// Deepest user preset file below `Presets/` (`plugins/<format>/<id>/<file>`).
const MAX_DEPTH: usize = 4;
/// Most user preset files read by one `List` (a runaway folder can't stall the controller).
const MAX_FILES: usize = 4096;
/// Longest file stem derived from a preset name (characters).
const MAX_STEM: usize = 80;

/// Folder of a device type's user presets, relative to `Presets/`.
pub(crate) fn device_dir(device: &PresetDevice) -> String {
    match device {
        PresetDevice::Builtin { device } => device.key(),
        PresetDevice::Plugin {
            format, plugin_id, ..
        } => format!(
            "plugins/{}/{}",
            format_key(*format),
            safe_component(plugin_id)
        ),
    }
}

fn format_key(format: PluginFormat) -> &'static str {
    match format {
        PluginFormat::Clap => "clap",
        PluginFormat::Vst3 => "vst3",
        PluginFormat::Au => "au",
        PluginFormat::Vst2 => "vst2",
    }
}

/// A plugin id as one path component (`aufx:dely:appl` → `aufx_dely_appl`).
fn safe_component(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let out = out.trim_start_matches('.').to_string();
    if out.is_empty() { "_".into() } else { out }
}

/// A file stem for a preset name: characters that are unsafe in file names on any OS become
/// `-`, leading dots and surrounding spaces are trimmed, at most [`MAX_STEM`] characters.
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
        "Preset".into()
    } else {
        trimmed.to_string()
    }
}

/// Library path (relative to the user root) of a user preset id.
pub(crate) fn library_path(id: &str) -> Result<String, StoreError> {
    check_relative_path(id)?;
    if id.is_empty() || !id.ends_with(&format!(".{PRESET_EXTENSION}")) {
        return Err(StoreError::InvalidPath(id.to_string()));
    }
    Ok(format!("{PRESETS_DIR}/{id}"))
}

/// Same device type (plugins: same format and id; display name/vendor may differ).
pub(crate) fn same_device(a: &PresetDevice, b: &PresetDevice) -> bool {
    match (a, b) {
        (PresetDevice::Builtin { device: x }, PresetDevice::Builtin { device: y }) => x == y,
        (
            PresetDevice::Plugin {
                format: f1,
                plugin_id: i1,
                ..
            },
            PresetDevice::Plugin {
                format: f2,
                plugin_id: i2,
                ..
            },
        ) => f1 == f2 && i1 == i2,
        _ => false,
    }
}

/// Tags lowercased, trimmed, deduplicated and sorted; empty author/description dropped.
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

/// Case-insensitive substring match on name, tags and author.
pub(crate) fn matches_text(name: &str, meta: &PresetMeta, text: Option<&str>) -> bool {
    let Some(q) = text.map(str::trim).filter(|q| !q.is_empty()) else {
        return true;
    };
    let q = q.to_lowercase();
    name.to_lowercase().contains(&q)
        || meta.tags.iter().any(|t| t.to_lowercase().contains(&q))
        || meta
            .author
            .as_deref()
            .is_some_and(|a| a.to_lowercase().contains(&q))
}

pub(crate) fn info(source: PresetSource, id: String, p: &Preset) -> PresetInfo {
    PresetInfo {
        preset: PresetRef { source, id },
        name: p.name.clone(),
        device: p.device.clone(),
        meta: p.meta.clone(),
    }
}

/// Built-in types whose factory presets a `List { device }` covers.
pub(crate) fn factory_types(device: Option<&PresetDevice>) -> Vec<BuiltinDeviceType> {
    match device {
        None => BuiltinDeviceType::ALL.to_vec(),
        Some(PresetDevice::Builtin { device }) => vec![*device],
        Some(PresetDevice::Plugin { .. }) => Vec::new(),
    }
}

/// The embedded factory preset `id` (`"<device-key>/<slug>"`), parsed.
pub(crate) fn factory_preset(id: &str) -> Option<Preset> {
    let key = id.split_once('/')?.0;
    let ty = BuiltinDeviceType::ALL
        .into_iter()
        .find(|t| t.key() == key)?;
    ether_devices::factory_presets(ty)
        .iter()
        .find(|p| p.id == id)
        .and_then(|p| load_preset(p.json).ok())
}

/// Every parsed user preset (id relative to `Presets/`) under `dir` (relative to
/// `Presets/`, `""` = all). Unreadable or invalid files are skipped; missing folders are
/// empty.
pub(crate) fn user_presets<L: Library>(
    library: &mut L,
    root: &str,
    dir: &str,
) -> Vec<(String, Preset)> {
    let mut files = Vec::new();
    walk(library, root, dir, 0, &mut files);
    files
        .into_iter()
        .filter_map(|id| {
            let bytes = library.read(root, &format!("{PRESETS_DIR}/{id}")).ok()?;
            let json = std::str::from_utf8(&bytes).ok()?;
            load_preset(json).ok().map(|p| (id, p))
        })
        .collect()
}

fn walk<L: Library>(library: &mut L, root: &str, dir: &str, depth: usize, out: &mut Vec<String>) {
    if depth > MAX_DEPTH || out.len() >= MAX_FILES {
        return;
    }
    let path = if dir.is_empty() {
        PRESETS_DIR.to_string()
    } else {
        format!("{PRESETS_DIR}/{dir}")
    };
    let Ok(listing) = library.list_dir(root, &path) else {
        return;
    };
    let ext = format!(".{PRESET_EXTENSION}");
    for e in listing.entries {
        if e.name.starts_with('.') {
            continue;
        }
        let rel = if dir.is_empty() {
            e.name.clone()
        } else {
            format!("{dir}/{}", e.name)
        };
        if e.kind == ether_core::protocol::media::FileKind::Directory {
            walk(library, root, &rel, depth + 1, out);
        } else if e.name.ends_with(&ext) && out.len() < MAX_FILES {
            out.push(rel);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems_and_folders() {
        assert_eq!(file_stem("Warm Pad"), "Warm Pad");
        assert_eq!(file_stem("a/b:c?"), "a-b-c-");
        assert_eq!(file_stem("  ..hidden. "), "hidden");
        assert_eq!(file_stem(" / "), "-");
        assert_eq!(file_stem(""), "Preset");
        assert_eq!(file_stem(&"x".repeat(200)).len(), MAX_STEM);
        assert_eq!(
            device_dir(&PresetDevice::Builtin {
                device: BuiltinDeviceType::PolySynth
            }),
            "poly-synth"
        );
        assert_eq!(
            device_dir(&PresetDevice::Plugin {
                format: PluginFormat::Au,
                plugin_id: "aufx:dely:appl".into(),
                name: "AUDelay".into(),
                vendor: "Apple".into(),
            }),
            "plugins/au/aufx_dely_appl"
        );
        assert!(library_path("synth/a.etherpreset").is_ok());
        for bad in ["../x.etherpreset", "synth/a.json", "", "/a.etherpreset"] {
            assert!(library_path(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn meta_and_text() {
        let m = normalize_meta(&PresetMeta {
            tags: vec!["Pad".into(), " pad ".into(), "".into(), "Bass".into()],
            author: Some("  ".into()),
            description: Some(" d ".into()),
        });
        assert_eq!(m.tags, ["bass", "pad"]);
        assert_eq!(m.author, None);
        assert_eq!(m.description.as_deref(), Some("d"));
        assert!(matches_text("Warm Pad", &m, Some("warm")));
        assert!(matches_text("X", &m, Some("BASS")));
        assert!(!matches_text("X", &m, Some("lead")));
        assert!(matches_text("X", &m, Some("  ")));
    }

    #[test]
    fn factory_lookup() {
        let p = factory_preset("synth/soft-pad").expect("embedded");
        assert_eq!(p.name, "Soft Pad");
        assert!(factory_preset("synth/nope").is_none());
        assert!(factory_preset("nope").is_none());
    }
}

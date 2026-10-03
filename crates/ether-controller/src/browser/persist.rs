//! The persisted index (`<user library>/.ethereal/index.json`): user folders, favourites,
//! tags, and the scanned files with their probed metadata (so a restart only re-lists
//! folders instead of re-probing every file). Presets and projects are re-derived on load.
//!
//! Hand-written JSON (`serde_json::Value`; the crate has no serde derive): compact keys,
//! unknown keys ignored, another `version` starts empty.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

/// Path inside the user library root.
pub(crate) const INDEX_PATH: &str = ".ethereal/index.json";
pub(crate) const VERSION: u64 = 1;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct IndexFile {
    pub folders: Vec<UserFolder>,
    pub favourites: BTreeSet<String>,
    pub tags: BTreeMap<String, Vec<String>>,
    /// Sample packs: root id → pack folder → display name.
    pub packs: BTreeMap<String, BTreeMap<String, String>>,
    pub items: Vec<StoredItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UserFolder {
    pub id: String,
    pub path: String,
}

/// A scanned audio/MIDI file (`id` = `<root>/<path>`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct StoredItem {
    pub root: String,
    pub path: String,
    pub midi: bool,
    pub size: f64,
    /// First seen (ms since the epoch).
    pub added: Option<f64>,
    pub probed: bool,
    pub duration: Option<f64>,
    pub rate: Option<u32>,
    pub channels: Option<u16>,
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k)?.as_str().map(str::to_string)
}

fn f(v: &Value, k: &str) -> Option<f64> {
    v.get(k)?.as_f64()
}

/// Parse an index file; anything unreadable or of another version is `None`.
pub(crate) fn parse(bytes: &[u8]) -> Option<IndexFile> {
    let v: Value = serde_json::from_slice(bytes).ok()?;
    if v.get("version")?.as_u64()? != VERSION {
        return None;
    }
    let arr = |k: &str| v.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
    let folders = arr("folders")
        .iter()
        .filter_map(|x| {
            Some(UserFolder {
                id: s(x, "id")?,
                path: s(x, "path")?,
            })
        })
        .collect();
    let favourites = arr("favourites")
        .iter()
        .filter_map(|x| x.as_str().map(str::to_string))
        .collect();
    let tags = v
        .get("tags")
        .and_then(Value::as_object)
        .map(|o| {
            o.iter()
                .map(|(id, t)| {
                    let t = t
                        .as_array()
                        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                        .unwrap_or_default();
                    (id.clone(), t)
                })
                .collect()
        })
        .unwrap_or_default();
    let packs = v
        .get("packs")
        .and_then(Value::as_object)
        .map(|o| {
            o.iter()
                .map(|(root, p)| {
                    let p = p
                        .as_object()
                        .map(|p| {
                            p.iter()
                                .filter_map(|(k, n)| Some((k.clone(), n.as_str()?.to_string())))
                                .collect()
                        })
                        .unwrap_or_default();
                    (root.clone(), p)
                })
                .collect()
        })
        .unwrap_or_default();
    let items = arr("items")
        .iter()
        .filter_map(|x| {
            Some(StoredItem {
                root: s(x, "r")?,
                path: s(x, "p")?,
                midi: x.get("m").and_then(Value::as_bool).unwrap_or(false),
                size: f(x, "s").unwrap_or(0.0),
                added: f(x, "a"),
                probed: x.get("pr").and_then(Value::as_bool).unwrap_or(false),
                duration: f(x, "d"),
                rate: x.get("sr").and_then(Value::as_u64).map(|n| n as u32),
                channels: x.get("ch").and_then(Value::as_u64).map(|n| n as u16),
            })
        })
        .collect();
    Some(IndexFile {
        folders,
        favourites,
        tags,
        packs,
        items,
    })
}

pub(crate) fn serialize(file: &IndexFile) -> Vec<u8> {
    let items: Vec<Value> = file
        .items
        .iter()
        .map(|i| {
            let mut o = Map::new();
            o.insert("r".into(), json!(i.root));
            o.insert("p".into(), json!(i.path));
            o.insert("s".into(), json!(i.size));
            if i.midi {
                o.insert("m".into(), json!(true));
            }
            if i.probed {
                o.insert("pr".into(), json!(true));
            }
            for (k, v) in [("a", i.added), ("d", i.duration)] {
                if let Some(v) = v {
                    o.insert(k.into(), json!(v));
                }
            }
            if let Some(r) = i.rate {
                o.insert("sr".into(), json!(r));
            }
            if let Some(c) = i.channels {
                o.insert("ch".into(), json!(c));
            }
            Value::Object(o)
        })
        .collect();
    let v = json!({
        "version": VERSION,
        "folders": file.folders.iter().map(|f| json!({"id": f.id, "path": f.path})).collect::<Vec<_>>(),
        "favourites": file.favourites,
        "tags": file.tags,
        "packs": file.packs,
        "items": items,
    });
    serde_json::to_vec(&v).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_version_gate() {
        let file = IndexFile {
            folders: vec![UserFolder {
                id: "folder-1".into(),
                path: "/x".into(),
            }],
            favourites: ["a/b.wav".to_string()].into(),
            tags: [("a/b.wav".to_string(), vec!["dry".to_string()])].into(),
            packs: [("a".to_string(), [("P".to_string(), "Pack".to_string())].into())].into(),
            items: vec![
                StoredItem {
                    root: "a".into(),
                    path: "b.wav".into(),
                    midi: false,
                    size: 10.0,
                    added: Some(1.0),
                    probed: true,
                    duration: Some(0.5),
                    rate: Some(44_100),
                    channels: Some(2),
                },
                StoredItem {
                    root: "a".into(),
                    path: "c.mid".into(),
                    midi: true,
                    ..Default::default()
                },
            ],
        };
        let bytes = serialize(&file);
        assert_eq!(parse(&bytes), Some(file));
        assert_eq!(parse(br#"{"version":99}"#), None);
        assert_eq!(parse(b"garbage"), None);
        assert_eq!(parse(br#"{"version":1}"#), Some(IndexFile::default()));
    }
}

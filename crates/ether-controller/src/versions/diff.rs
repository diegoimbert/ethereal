//! `Version::Compare`: a per-table summary of what changed between two documents.
//!
//! Pure functions over the serialized documents (`serde_json::Value`), so tables added by
//! later format versions are compared without changes here.

use ether_core::protocol::model::Project;
use ether_core::protocol::versions::{TableDiff, VersionDiff};
use serde_json::Value;

/// Names listed per table, at most.
pub const MAX_NAMES: usize = 50;

/// Tables whose entries are listed by name (others: counts only).
const NAMED_TABLES: &[&str] = &["tracks", "clips"];

/// What changed from `base` to `target`: `added` = only in `target`, `removed` = only in
/// `base`, `changed` = in both but different. The project id is ignored.
pub fn diff(base: &Project, target: &Project) -> VersionDiff {
    let (Ok(base), Ok(target)) = (serde_json::to_value(base), serde_json::to_value(target)) else {
        return VersionDiff::default();
    };
    diff_values(&base, &target)
}

pub fn diff_values(base: &Value, target: &Value) -> VersionDiff {
    let empty = serde_json::Map::new();
    let b = base.as_object().unwrap_or(&empty);
    let t = target.as_object().unwrap_or(&empty);
    let mut tables = Vec::new();
    let mut keys: Vec<&String> = b.keys().chain(t.keys()).collect();
    keys.sort();
    keys.dedup();
    for key in keys {
        if key == "id" || key == "settings" {
            continue;
        }
        let (bt, tt) = (b.get(key), t.get(key));
        let (bm, tm) = (
            bt.and_then(Value::as_object).unwrap_or(&empty),
            tt.and_then(Value::as_object).unwrap_or(&empty),
        );
        let named = NAMED_TABLES.contains(&key.as_str());
        let mut row = TableDiff {
            table: key.clone(),
            added: 0,
            removed: 0,
            changed: 0,
            names: Vec::new(),
        };
        let note = |row: &mut TableDiff, v: &Value| {
            if named
                && row.names.len() < MAX_NAMES
                && let Some(name) = v.get("name").and_then(Value::as_str)
                && !name.trim().is_empty()
                && !row.names.iter().any(|n| n == name)
            {
                row.names.push(name.to_string());
            }
        };
        for (id, v) in tm {
            match bm.get(id) {
                None => {
                    row.added += 1;
                    note(&mut row, v);
                }
                Some(old) if old != v => {
                    row.changed += 1;
                    note(&mut row, v);
                }
                Some(_) => {}
            }
        }
        for (id, v) in bm {
            if !tm.contains_key(id) {
                row.removed += 1;
                note(&mut row, v);
            }
        }
        if row.added + row.removed + row.changed > 0 {
            tables.push(row);
        }
    }
    VersionDiff {
        tables,
        settings_changed: b.get("settings") != t.get("settings"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn counts_and_names_per_table() {
        let base = json!({
            "id": "a",
            "settings": {"name": "Song", "bpm": 120},
            "tracks": {"1": {"name": "Drums"}, "2": {"name": "Bass"}},
            "clips": {"9": {"name": "Intro"}},
            "notes": {"5": {"pitch": 60}},
        });
        let target = json!({
            "id": "b",
            "settings": {"name": "Song", "bpm": 120},
            "tracks": {"1": {"name": "Drums 2"}, "3": {"name": "Keys"}},
            "clips": {"9": {"name": "Intro"}},
            "notes": {"5": {"pitch": 61}, "6": {"pitch": 64}},
            "markers": {},
        });
        let d = diff_values(&base, &target);
        assert!(!d.settings_changed, "the id is ignored");
        assert_eq!(d.tables.len(), 2, "{d:?}");
        let notes = &d.tables[0];
        assert_eq!(
            (
                notes.table.as_str(),
                notes.added,
                notes.removed,
                notes.changed
            ),
            ("notes", 1, 0, 1)
        );
        assert!(notes.names.is_empty(), "notes are counted only");
        let tracks = &d.tables[1];
        assert_eq!((tracks.added, tracks.removed, tracks.changed), (1, 1, 1));
        assert_eq!(tracks.names, ["Drums 2", "Keys", "Bass"]);
    }

    #[test]
    fn settings_and_name_cap() {
        let tracks: serde_json::Map<String, Value> = (0..80)
            .map(|i| (i.to_string(), json!({"name": format!("T{i}")})))
            .collect();
        let base = json!({"settings": {"bpm": 120}, "tracks": {}});
        let target = json!({"settings": {"bpm": 128}, "tracks": tracks});
        let d = diff_values(&base, &target);
        assert!(d.settings_changed);
        assert_eq!(d.tables[0].added, 80);
        assert_eq!(d.tables[0].names.len(), MAX_NAMES);
    }

    #[test]
    fn identical_documents_have_no_rows() {
        let v = json!({"settings": {}, "tracks": {"1": {"name": "A"}}});
        assert_eq!(diff_values(&v, &v), VersionDiff::default());
    }
}

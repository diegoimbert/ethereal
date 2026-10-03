//! The in-memory library index: items, user data (favourites, tags) and paged queries.
//!
//! Queries are a linear scan over pre-lowercased search text in name order (kept sorted
//! lazily), then a stable sort by the requested key, so ties stay in name order. 50k items
//! query in a few milliseconds (release; `tests/browser_bench.rs`).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use ether_core::protocol::browser::{
    BrowserPage, BrowserQuery, BrowserSort, LibraryItem, LibraryItemKind,
};
use ether_core::protocol::model::PresetDevice;

/// Largest page.
pub(crate) const MAX_LIMIT: u32 = 200;

/// One indexed item plus what queries need.
#[derive(Clone, Debug)]
pub(crate) struct Entry {
    /// `favourite` and `tags` are kept in sync with the index's user data.
    pub item: LibraryItem,
    /// Presets: the device type (the `device` filter).
    pub device: Option<PresetDevice>,
    /// Tags that come with the item (preset meta); user tags replace them when set.
    pub base_tags: Vec<String>,
    /// The header probe ran (audio).
    pub probed: bool,
    name_lc: String,
    /// Lowercased name, path, pack, key and device name, `\n`-separated.
    hay: String,
}

impl Entry {
    pub fn new(item: LibraryItem, device: Option<PresetDevice>, base_tags: Vec<String>) -> Self {
        let mut e = Self {
            item,
            device,
            base_tags,
            probed: false,
            name_lc: String::new(),
            hay: String::new(),
        };
        e.refresh_text();
        e
    }

    /// Recompute the search text (after a name/meta change).
    pub fn refresh_text(&mut self) {
        let i = &self.item;
        self.name_lc = i.name.to_lowercase();
        let device = match &self.device {
            Some(PresetDevice::Builtin { device }) => device.key().to_string(),
            Some(PresetDevice::Plugin { name, vendor, .. }) => format!("{name} {vendor}"),
            None => String::new(),
        };
        self.hay = [
            i.name.as_str(),
            i.path.as_str(),
            i.meta.pack.as_deref().unwrap_or(""),
            i.meta.key.as_deref().unwrap_or(""),
            device.as_str(),
        ]
        .join("\n")
        .to_lowercase();
    }
}

/// Same device type (plugins by format and id).
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

/// Lowercase, trimmed, non-empty, deduplicated, sorted.
pub(crate) fn normalize_tags(tags: &[String]) -> Vec<String> {
    tags.iter()
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[derive(Default)]
pub(crate) struct Index {
    entries: Vec<Entry>,
    by_id: HashMap<String, usize>,
    /// Entry positions sorted by (lowercase name, id); rebuilt lazily.
    order: Vec<u32>,
    order_dirty: bool,
    /// User data, kept across rescans (and for items not indexed yet).
    pub favourites: BTreeSet<String>,
    pub tags: BTreeMap<String, Vec<String>>,
}

impl Index {
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.by_id.get(id).map(|&i| &self.entries[i])
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Entry> {
        self.by_id.get(id).map(|&i| &mut self.entries[i])
    }

    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter()
    }

    /// Insert or replace (user data applied). Returns the previous entry.
    pub fn upsert(&mut self, mut entry: Entry) -> Option<Entry> {
        entry.item.favourite = self.favourites.contains(&entry.item.id);
        entry.item.tags = self
            .tags
            .get(&entry.item.id)
            .cloned()
            .unwrap_or_else(|| entry.base_tags.clone());
        match self.by_id.get(&entry.item.id) {
            Some(&i) => {
                let old = std::mem::replace(&mut self.entries[i], entry);
                if old.name_lc != self.entries[i].name_lc {
                    self.order_dirty = true;
                }
                Some(old)
            }
            None => {
                self.by_id
                    .insert(entry.item.id.clone(), self.entries.len());
                self.entries.push(entry);
                self.order_dirty = true;
                None
            }
        }
    }

    /// Remove every entry for which `drop` is true. Returns how many were removed.
    pub fn remove_where(&mut self, mut drop: impl FnMut(&Entry) -> bool) -> usize {
        let before = self.entries.len();
        self.entries.retain(|e| !drop(e));
        let removed = before - self.entries.len();
        if removed > 0 {
            self.by_id = self
                .entries
                .iter()
                .enumerate()
                .map(|(i, e)| (e.item.id.clone(), i))
                .collect();
            self.order_dirty = true;
        }
        removed
    }

    pub fn set_favourite(&mut self, id: &str, favourite: bool) -> bool {
        let Some(&i) = self.by_id.get(id) else {
            return false;
        };
        self.entries[i].item.favourite = favourite;
        if favourite {
            self.favourites.insert(id.to_string());
        } else {
            self.favourites.remove(id);
        }
        true
    }

    pub fn set_tags(&mut self, id: &str, tags: &[String]) -> bool {
        let Some(&i) = self.by_id.get(id) else {
            return false;
        };
        let tags = normalize_tags(tags);
        self.entries[i].item.tags = tags.clone();
        self.tags.insert(id.to_string(), tags);
        true
    }

    fn ensure_order(&mut self) {
        if !self.order_dirty && self.order.len() == self.entries.len() {
            return;
        }
        let mut order: Vec<u32> = (0..self.entries.len() as u32).collect();
        let e = &self.entries;
        order.sort_unstable_by(|&a, &b| {
            let (a, b) = (&e[a as usize], &e[b as usize]);
            a.name_lc
                .cmp(&b.name_lc)
                .then_with(|| a.item.id.cmp(&b.item.id))
        });
        self.order = order;
        self.order_dirty = false;
    }

    pub fn query(&mut self, q: &BrowserQuery) -> BrowserPage {
        self.ensure_order();
        let words: Vec<String> = q
            .text
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        let tags = normalize_tags(&q.tags);
        let roots: Vec<(&str, Option<String>)> = q
            .roots
            .iter()
            .map(|r| match r.split_once('/') {
                // A pack: `<root>/<folder>`.
                Some((root, folder)) => (root, Some(format!("{folder}/"))),
                None => (r.as_str(), None),
            })
            .collect();
        let folder = q
            .folder
            .as_deref()
            .filter(|f| !f.is_empty())
            .map(|f| format!("{}/", f.trim_end_matches('/')));
        let folder_root = q.roots.first().map(String::as_str);

        let matches = |e: &Entry| -> bool {
            let i = &e.item;
            if !q.kinds.is_empty() && !q.kinds.contains(&i.kind) {
                return false;
            }
            if q.favourites_only && !i.favourite {
                return false;
            }
            if !tags.iter().all(|t| i.tags.contains(t)) {
                return false;
            }
            if !roots.is_empty()
                && !roots.iter().any(|(root, sub)| {
                    i.root == *root && sub.as_ref().is_none_or(|s| i.path.starts_with(s.as_str()))
                })
            {
                return false;
            }
            if let Some(folder) = &folder {
                if folder_root.is_some_and(|r| r.split('/').next() != Some(i.root.as_str())) {
                    return false;
                }
                if !i.path.starts_with(folder.as_str()) {
                    return false;
                }
            }
            if let (Some(d), LibraryItemKind::Preset) = (&q.device, i.kind)
                && !e.device.as_ref().is_some_and(|x| same_device(d, x))
            {
                return false;
            }
            words
                .iter()
                .all(|w| e.hay.contains(w.as_str()) || i.tags.iter().any(|t| t.contains(w.as_str())))
        };

        let mut hits: Vec<u32> = self
            .order
            .iter()
            .copied()
            .filter(|&i| matches(&self.entries[i as usize]))
            .collect();
        let e = &self.entries;
        let desc_none_last = |a: Option<f64>, b: Option<f64>| match (a, b) {
            (Some(a), Some(b)) => b.total_cmp(&a),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        };
        let asc_none_last = |a: Option<f64>, b: Option<f64>| match (a, b) {
            (Some(a), Some(b)) => a.total_cmp(&b),
            _ => desc_none_last(a, b),
        };
        match q.sort {
            BrowserSort::Name => {}
            BrowserSort::Recent => hits.sort_by(|&a, &b| {
                desc_none_last(
                    e[a as usize].item.meta.modified_ms,
                    e[b as usize].item.meta.modified_ms,
                )
            }),
            BrowserSort::Duration => hits.sort_by(|&a, &b| {
                asc_none_last(
                    e[a as usize].item.meta.duration_seconds,
                    e[b as usize].item.meta.duration_seconds,
                )
            }),
            BrowserSort::Bpm => hits.sort_by(|&a, &b| {
                asc_none_last(e[a as usize].item.meta.bpm, e[b as usize].item.meta.bpm)
            }),
            BrowserSort::Relevance if !words.is_empty() => {
                let phrase = words.join(" ");
                hits.sort_by_cached_key(|&i| {
                    std::cmp::Reverse(relevance(&e[i as usize].name_lc, &phrase, &words))
                })
            }
            BrowserSort::Relevance => {}
        }
        let total = hits.len() as u32;
        let limit = q.limit.clamp(1, MAX_LIMIT) as usize;
        let items = hits
            .iter()
            .skip(q.offset as usize)
            .take(limit)
            .map(|&i| e[i as usize].item.clone())
            .collect();
        BrowserPage {
            items,
            total,
            offset: q.offset,
        }
    }
}

/// How well `name` matches: exact phrase > phrase prefix > per-word (word start > contains).
fn relevance(name: &str, phrase: &str, words: &[String]) -> u32 {
    let mut score = 0;
    if name == phrase {
        score += 1000;
    } else if name.starts_with(phrase) {
        score += 500;
    }
    for w in words {
        score += if name.starts_with(w.as_str()) {
            30
        } else if name
            .match_indices(w.as_str())
            .any(|(i, _)| !name[..i].ends_with(|c: char| c.is_alphanumeric()))
        {
            20
        } else if name.contains(w.as_str()) {
            10
        } else {
            0
        };
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use ether_core::protocol::browser::LibraryItemMeta;

    fn item(root: &str, path: &str, kind: LibraryItemKind) -> Entry {
        let name = path.rsplit('/').next().unwrap().to_string();
        Entry::new(
            LibraryItem {
                id: format!("{root}/{path}"),
                kind,
                name,
                root: root.into(),
                path: path.into(),
                source: None,
                preset: None,
                tags: vec![],
                favourite: false,
                meta: LibraryItemMeta::default(),
            },
            None,
            vec![],
        )
    }

    fn q(text: &str) -> BrowserQuery {
        BrowserQuery {
            text: text.into(),
            kinds: vec![],
            tags: vec![],
            favourites_only: false,
            roots: vec![],
            folder: None,
            device: None,
            sort: BrowserSort::Name,
            offset: 0,
            limit: 50,
        }
    }

    fn names(p: &BrowserPage) -> Vec<&str> {
        p.items.iter().map(|i| i.name.as_str()).collect()
    }

    fn index() -> Index {
        let mut ix = Index::default();
        for p in [
            "Drums/Kick.wav",
            "Drums/Snare.wav",
            "Drums/Loops/Kick Loop 120.wav",
            "Keys/Big Kickstarter.wav",
            "Pack/One/Hat.wav",
        ] {
            ix.upsert(item("lib", p, LibraryItemKind::Audio));
        }
        ix.upsert(item("lib", "Midi/Groove.mid", LibraryItemKind::Midi));
        ix
    }

    #[test]
    fn text_kinds_roots_folder() {
        let mut ix = index();
        assert_eq!(
            names(&ix.query(&q("kick"))),
            ["Big Kickstarter.wav", "Kick Loop 120.wav", "Kick.wav"]
        );
        assert_eq!(names(&ix.query(&q("KICK loop"))), ["Kick Loop 120.wav"]);
        assert_eq!(names(&ix.query(&q("drums snare"))), ["Snare.wav"]);
        let mut midi = q("");
        midi.kinds = vec![LibraryItemKind::Midi];
        assert_eq!(names(&ix.query(&midi)), ["Groove.mid"]);
        let mut folder = q("");
        folder.roots = vec!["lib".into()];
        folder.folder = Some("Drums".into());
        assert_eq!(ix.query(&folder).total, 3);
        let mut pack = q("");
        pack.roots = vec!["lib/Pack".into()];
        assert_eq!(names(&ix.query(&pack)), ["Hat.wav"]);
        let mut other = q("");
        other.roots = vec!["nope".into()];
        assert_eq!(ix.query(&other).total, 0);
    }

    #[test]
    fn relevance_and_paging() {
        let mut ix = index();
        let mut r = q("kick");
        r.sort = BrowserSort::Relevance;
        assert_eq!(
            names(&ix.query(&r)),
            ["Kick Loop 120.wav", "Kick.wav", "Big Kickstarter.wav"]
        );
        let mut page = q("");
        page.limit = 2;
        page.offset = 4;
        let p = ix.query(&page);
        assert_eq!((p.total, p.offset, p.items.len()), (6, 4, 2));
        page.limit = 0;
        assert_eq!(ix.query(&page).items.len(), 1, "limit is clamped to >= 1");
    }

    #[test]
    fn favourites_and_tags_survive_rescans() {
        let mut ix = index();
        assert!(ix.set_favourite("lib/Drums/Kick.wav", true));
        assert!(ix.set_tags("lib/Drums/Kick.wav", &[" Punchy".into(), "punchy".into(), "".into(), "Dry".into()]));
        assert!(!ix.set_favourite("lib/none", true));
        let mut fav = q("");
        fav.favourites_only = true;
        assert_eq!(names(&ix.query(&fav)), ["Kick.wav"]);
        // A rescan replaces the entry; user data is re-applied.
        ix.upsert(item("lib", "Drums/Kick.wav", LibraryItemKind::Audio));
        let mut tagged = q("punchy");
        tagged.tags = vec!["DRY".into()];
        let p = ix.query(&tagged);
        assert_eq!(names(&p), ["Kick.wav"]);
        assert_eq!(p.items[0].tags, ["dry", "punchy"]);
        assert!(p.items[0].favourite);
        ix.remove_where(|e| e.item.path.starts_with("Drums/"));
        assert_eq!(ix.len(), 3);
        assert!(ix.get("lib/Keys/Big Kickstarter.wav").is_some());
    }

    #[test]
    fn sorts_put_unknowns_last() {
        let mut ix = index();
        ix.get_mut("lib/Drums/Kick.wav").unwrap().item.meta.bpm = Some(128.0);
        ix.get_mut("lib/Drums/Snare.wav").unwrap().item.meta.bpm = Some(90.0);
        ix.get_mut("lib/Drums/Kick.wav").unwrap().item.meta.modified_ms = Some(5.0);
        ix.get_mut("lib/Drums/Snare.wav").unwrap().item.meta.modified_ms = Some(9.0);
        let mut s = q("");
        s.sort = BrowserSort::Bpm;
        assert_eq!(&names(&ix.query(&s))[..3], ["Snare.wav", "Kick.wav", "Big Kickstarter.wav"]);
        s.sort = BrowserSort::Recent;
        assert_eq!(&names(&ix.query(&s))[..2], ["Snare.wav", "Kick.wav"]);
    }
}

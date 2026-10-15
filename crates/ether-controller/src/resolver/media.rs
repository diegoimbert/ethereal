//! Missing audio files: scoring a found file against what the project expects (pure,
//! unit-tested) and the folder search job (`Resolve::SearchMedia`).
//!
//! The walk follows `media_refs::Search` (the Relink dialog's search: library roots and an
//! OS folder, bounded per tick, hidden entries skipped) and the import's own media lookup
//! (the project folder, its `Samples/`, parents' `Samples/`), but proposes instead of
//! relinking: every file with the same name (or the same stem with another audio
//! extension) is read, probed and scored ([`score`]).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use ether_core::protocol::media::{BrowseLocation, FileKind, MediaSource};
use ether_core::protocol::model::{MediaId, MediaRef};
use ether_core::protocol::resolver::MediaCandidate;

use crate::content_hash;
use crate::media::{IncrementalDecoder, extension_of};
use crate::store::{Library, file_kind};

/// Work units per tick (a folder listing = 1, reading + probing a file = [`READ_COST`]).
pub const UNITS_PER_TICK: u32 = 32;
const READ_COST: u32 = 16;
const MAX_DEPTH: u8 = 16;
const MAX_FOLDERS: u32 = 50_000;
/// Candidates kept per file.
pub const MAX_MEDIA_CANDIDATES: usize = 5;

/// What the project expects of a missing file.
#[derive(Clone, Debug, PartialEq)]
pub struct Expected {
    pub name: String,
    pub hash: Option<String>,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
}

impl Expected {
    pub fn of(m: &MediaRef) -> Self {
        Self {
            name: m.name.clone(),
            hash: m.hash.clone(),
            sample_rate: m.sample_rate,
            channels: m.channels,
            frames: m.frames,
        }
    }
}

/// What a found file is.
#[derive(Clone, Debug, PartialEq)]
pub struct Probed {
    pub name: String,
    pub hash: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: u64,
}

/// Lowercase file name of a path (either separator).
pub fn file_key(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_lowercase()
}

/// Lowercase file name without its extension.
pub fn stem_key(path: &str) -> String {
    let f = file_key(path);
    match f.rfind('.') {
        Some(i) if i > 0 => f[..i].to_string(),
        _ => f,
    }
}

fn khz(rate: u32) -> String {
    let k = f64::from(rate) / 1000.0;
    if k.fract() == 0.0 {
        format!("{k:.0} kHz")
    } else {
        format!("{k:.1} kHz")
    }
}

/// How likely `found` is the missing file: `(confidence 0..=1, reason)`, or `None` when it
/// can't be (another name, or a format the project's clips can't use: a project that knows
/// the file's content needs the same sample rate and channel count, like Relink).
pub fn score(expected: &Expected, found: &Probed) -> Option<(f32, String)> {
    if expected.hash.as_deref() == Some(found.hash.as_str()) {
        return Some((1.0, "Same content".into()));
    }
    let (mut c, mut why) = if file_key(&expected.name) == file_key(&found.name) {
        (0.6f32, vec!["Same file name".to_string()])
    } else if stem_key(&expected.name) == stem_key(&found.name) {
        (0.45, vec!["Same name, another file type".to_string()])
    } else {
        return None;
    };
    let same_format =
        found.sample_rate == expected.sample_rate && found.channels == expected.channels;
    if same_format {
        c += 0.15;
        why.push(format!(
            "same format ({}, {} ch)",
            khz(found.sample_rate),
            found.channels
        ));
    } else if expected.hash.is_some() {
        return None;
    } else {
        why.push(format!(
            "{}, {} ch (the project's hint: {}, {} ch)",
            khz(found.sample_rate),
            found.channels,
            khz(expected.sample_rate),
            expected.channels
        ));
    }
    if expected.frames > 0 && found.frames > 0 {
        // Compare durations (the hint may be at another rate).
        let e = expected.frames as f64 / f64::from(expected.sample_rate.max(1));
        let f = found.frames as f64 / f64::from(found.sample_rate.max(1));
        if (e - f).abs() <= (e.max(f) * 0.005).max(0.01) {
            c += 0.2;
            why.push("same length".into());
        } else {
            c -= 0.15;
            why.push(format!("length {f:.1} s instead of {e:.1} s"));
        }
    }
    if expected.hash.is_some() {
        c = c.min(0.7);
        why.push("different content".into());
    }
    Some((c.clamp(0.05, 0.95), capitalize(&why.join(", "))))
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Header probe + content hash of a file's bytes.
pub fn probe(bytes: Vec<u8>, name: &str) -> Option<Probed> {
    let bytes: Arc<[u8]> = bytes.into();
    let mut d = IncrementalDecoder::new(bytes.clone(), extension_of(name)).ok()?;
    if d.sample_rate == 0 || d.channels == 0 {
        d.step(1).ok()?;
    }
    (d.sample_rate > 0 && d.channels > 0).then(|| Probed {
        name: name.to_string(),
        hash: content_hash(&bytes),
        sample_rate: d.sample_rate,
        channels: u16::try_from(d.channels).unwrap_or(2),
        frames: d.n_frames.unwrap_or(0),
    })
}

/// A file found by the walk.
#[derive(Clone, Debug, PartialEq)]
pub enum Found {
    Library { root: String, rel: String },
    External { path: String },
}

impl Found {
    pub fn source(&self) -> MediaSource {
        match self {
            Found::Library { root, rel } => MediaSource::Location {
                location: BrowseLocation::Library { id: root.clone() },
                path: rel.clone(),
            },
            Found::External { path } => MediaSource::Path { path: path.clone() },
        }
    }
}

pub enum Folder {
    Library {
        root: String,
        rel: String,
        depth: u8,
    },
    External {
        path: String,
        depth: u8,
    },
}

/// A running `Resolve::SearchMedia`.
pub struct MediaSearch {
    pub targets: BTreeMap<MediaId, Expected>,
    /// File name and stem → media.
    names: BTreeMap<String, Vec<MediaId>>,
    pub folders: VecDeque<Folder>,
    /// Library root names (display).
    pub root_names: BTreeMap<String, String>,
    to_read: VecDeque<(MediaId, Found)>,
    seen: BTreeSet<(MediaId, String)>,
    found: BTreeMap<MediaId, Vec<MediaCandidate>>,
    pub scanned: u32,
}

impl MediaSearch {
    pub fn new(targets: BTreeMap<MediaId, Expected>) -> Self {
        let mut names: BTreeMap<String, Vec<MediaId>> = BTreeMap::new();
        for (id, t) in &targets {
            for k in BTreeSet::from([file_key(&t.name), stem_key(&t.name)]) {
                names.entry(k).or_default().push(*id);
            }
        }
        Self {
            targets,
            names,
            folders: VecDeque::new(),
            root_names: BTreeMap::new(),
            to_read: VecDeque::new(),
            seen: BTreeSet::new(),
            found: BTreeMap::new(),
            scanned: 0,
        }
    }

    fn saw_file(&mut self, name: &str, found: Found) {
        if file_kind(name) != FileKind::Audio {
            return;
        }
        let mut ids: BTreeSet<MediaId> = BTreeSet::new();
        for k in [file_key(name), stem_key(name)] {
            if let Some(v) = self.names.get(&k) {
                ids.extend(v.iter().copied());
            }
        }
        for id in ids {
            self.to_read.push_back((id, found.clone()));
        }
    }

    fn list<L: Library>(&mut self, library: &mut L, folder: Folder) {
        self.scanned += 1;
        match folder {
            Folder::Library { root, rel, depth } => {
                let Ok(listing) = library.list_dir(&root, &rel) else {
                    return;
                };
                for e in listing.entries {
                    if e.name.starts_with('.') {
                        continue;
                    }
                    match e.kind {
                        FileKind::Directory if depth < MAX_DEPTH => {
                            self.folders.push_back(Folder::Library {
                                root: root.clone(),
                                rel: e.path,
                                depth: depth + 1,
                            });
                        }
                        FileKind::Directory => {}
                        _ => self.saw_file(
                            &e.name,
                            Found::Library {
                                root: root.clone(),
                                rel: e.path,
                            },
                        ),
                    }
                }
            }
            Folder::External { path, depth } => {
                let Ok(entries) = library.list_external_dir(&path) else {
                    return;
                };
                for (child, is_dir) in entries {
                    let name = child.rsplit(['/', '\\']).next().unwrap_or("").to_string();
                    if name.starts_with('.') {
                        continue;
                    }
                    if is_dir {
                        if depth < MAX_DEPTH {
                            self.folders.push_back(Folder::External {
                                path: child,
                                depth: depth + 1,
                            });
                        }
                    } else {
                        self.saw_file(&name, Found::External { path: child });
                    }
                }
            }
        }
    }

    fn display(&self, found: &Found) -> String {
        match found {
            Found::External { path } => path.clone(),
            Found::Library { root, rel } => format!(
                "{}/{rel}",
                self.root_names
                    .get(root)
                    .map_or(root.as_str(), String::as_str)
            ),
        }
    }

    fn read<L: Library>(&mut self, library: &mut L, id: MediaId, found: Found) {
        let key = match &found {
            Found::External { path } => path.clone(),
            Found::Library { root, rel } => format!("{root}\u{0}{rel}"),
        };
        if !self.seen.insert((id, key)) {
            return;
        }
        let (bytes, name) = match &found {
            Found::Library { root, rel } => (library.read(root, rel), rel.clone()),
            Found::External { path } => (library.read_external(path), path.clone()),
        };
        let Ok(bytes) = bytes else { return };
        let Some(probed) = probe(bytes, &name) else {
            return;
        };
        let Some((confidence, reason)) = score(&self.targets[&id], &probed) else {
            return;
        };
        let display = self.display(&found);
        self.found.entry(id).or_default().push(MediaCandidate {
            source: found.source(),
            display,
            confidence,
            reason,
        });
    }

    /// Advance by about `units`. `true` when finished.
    pub fn step<L: Library>(&mut self, library: &mut L, mut units: u32) -> bool {
        while units > 0 {
            if let Some((id, found)) = self.to_read.pop_front() {
                self.read(library, id, found);
                units = units.saturating_sub(READ_COST);
            } else if self.scanned < MAX_FOLDERS
                && let Some(folder) = self.folders.pop_front()
            {
                self.list(library, folder);
                units -= 1;
            } else {
                return true;
            }
        }
        false
    }

    /// Candidates per searched file, best first.
    pub fn results(mut self) -> Vec<(MediaId, Vec<MediaCandidate>)> {
        self.targets
            .keys()
            .map(|id| {
                let mut c = self.found.remove(id).unwrap_or_default();
                c.sort_by(|a, b| {
                    b.confidence
                        .total_cmp(&a.confidence)
                        .then(a.display.cmp(&b.display))
                });
                c.truncate(MAX_MEDIA_CANDIDATES);
                (*id, c)
            })
            .collect()
    }
}

/// Folders worth searching for a project imported from `dir` (its folder, then
/// `Samples/` there and in up to three parents: Ableton keeps samples in the project
/// folder's `Samples/`, often one or two levels up when sets live in subfolders).
pub fn import_folders(dir: &str) -> Vec<String> {
    let sep = if dir.contains('\\') && !dir.contains('/') {
        '\\'
    } else {
        '/'
    };
    let mut out = vec![dir.to_string()];
    let mut cur = dir.to_string();
    for _ in 0..3 {
        let Some(i) = cur.rfind(['/', '\\']) else {
            break;
        };
        if i == 0 {
            break;
        }
        cur.truncate(i);
        out.push(format!("{cur}{sep}Samples"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expected(name: &str, hash: Option<&str>) -> Expected {
        Expected {
            name: name.into(),
            hash: hash.map(str::to_string),
            sample_rate: 44_100,
            channels: 2,
            frames: 441_000,
        }
    }

    fn probed(name: &str, rate: u32, ch: u16, frames: u64) -> Probed {
        Probed {
            name: name.into(),
            hash: "h".into(),
            sample_rate: rate,
            channels: ch,
            frames,
        }
    }

    #[test]
    fn same_name_format_and_length_scores_high() {
        let (c, why) = score(
            &expected("Kick.wav", None),
            &probed("/x/kick.WAV", 44_100, 2, 441_000),
        )
        .unwrap();
        assert!((c - 0.95).abs() < 1e-6, "{c}");
        assert_eq!(
            why,
            "Same file name, same format (44.1 kHz, 2 ch), same length"
        );
        // Same content beats everything.
        let mut p = probed("Other.wav", 48_000, 1, 1);
        p.hash = "abc".into();
        assert_eq!(
            score(&expected("Kick.wav", Some("abc")), &p).unwrap().0,
            1.0
        );
    }

    #[test]
    fn length_format_and_names_lower_the_score() {
        let e = expected("Loop.wav", None);
        let (c, why) = score(&e, &probed("Loop.wav", 44_100, 2, 88_200)).unwrap();
        assert!(c < 0.7, "{c}");
        assert!(why.contains("length 2.0 s instead of 10.0 s"), "{why}");
        // Another rate, same duration: allowed (the project only had a hint).
        let (c, why) = score(&e, &probed("Loop.wav", 48_000, 2, 480_000)).unwrap();
        assert!((c - 0.8).abs() < 1e-6, "{c} {why}");
        assert!(
            why.contains("48 kHz, 2 ch (the project's hint: 44.1 kHz, 2 ch)"),
            "{why}"
        );
        // Another type with the same stem.
        let (c, _) = score(&e, &probed("loop.aif", 44_100, 2, 441_000)).unwrap();
        assert!((c - 0.8).abs() < 1e-6, "{c}");
        assert!(score(&e, &probed("Other.wav", 44_100, 2, 441_000)).is_none());
    }

    #[test]
    fn known_content_needs_the_same_format() {
        let e = expected("Kick.wav", Some("abc"));
        assert!(score(&e, &probed("Kick.wav", 48_000, 2, 480_000)).is_none());
        let (c, why) = score(&e, &probed("Kick.wav", 44_100, 2, 441_000)).unwrap();
        assert!((c - 0.7).abs() < 1e-6);
        assert!(why.ends_with("different content"));
    }

    #[test]
    fn keys_and_import_folders() {
        assert_eq!(file_key("C:\\a\\Kick 01.WAV"), "kick 01.wav");
        assert_eq!(stem_key("/a/b/Kick 01.WAV"), "kick 01");
        assert_eq!(stem_key(".hidden"), ".hidden");
        assert_eq!(
            import_folders("/Users/me/Music/Song Project/Sets"),
            vec![
                "/Users/me/Music/Song Project/Sets".to_string(),
                "/Users/me/Music/Song Project/Samples".to_string(),
                "/Users/me/Music/Samples".to_string(),
                "/Users/me/Samples".to_string(),
            ]
        );
        assert_eq!(
            import_folders("C:\\P\\S"),
            vec![
                "C:\\P\\S".to_string(),
                "C:\\P\\Samples".to_string(),
                "C:\\Samples".to_string()
            ]
        );
    }
}

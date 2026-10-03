//! Browser v2: engine-side library index (v0.2, owned by the `browser-v2` node; protocol
//! `ether_protocol::browser`, CONTRACTS.md §12.8).
//!
//! [`EtherController::browser_command`] (dispatched from `handlers.rs`) and
//! [`EtherController::browser_tick`] (every tick: bounded indexing work, progress events).
//!
//! - **Lazy start.** Nothing happens until the first `Browser` command (the v2 browser sends
//!   `ListRoots` when it mounts): the persisted index is loaded from the user library
//!   (`<library>/.ethereal/index.json` + the `items.json` scan cache, [`persist`]), user
//!   folders are re-added through `Library::add_folder`, and every root is queued for an
//!   incremental rescan. User edits (favourites, tags, folders) are written at once; the
//!   scan cache after scans settle.
//! - **Never blocks the controller.** Scanning is a breadth-first walk of each root, a
//!   bounded number of folder listings and entries per tick ([`DIRS_PER_TICK`],
//!   [`ENTRIES_PER_TICK`]); queries answer from the partial index meanwhile. A second,
//!   lower-priority pass probes audio headers (duration, rate, channels) within a byte
//!   budget per tick, only for files up to [`MAX_PROBE_BYTES`], once per file (persisted).
//!   `IndexProgress` is throttled; `IndexChanged` follows each finished root, probe batches
//!   (throttled), and favourite/tag/folder edits.
//! - **Items.** Audio and MIDI files of every library root, user folder and the user
//!   library (id `<root>/<path>`, source = the library location, so a drop imports it like
//!   the folder browser: referenced in place on desktop); user presets
//!   (`user/Presets/...`, parsed while scanning the user library) and factory presets
//!   (`factory/<device-key>/<slug>`); projects (`projects/<uuid>`, from the store, refreshed
//!   with every full rescan). Tempo and key come from file names ([`names`]). `modified_ms`
//!   is when the file was first indexed (listings carry no file times; projects use their
//!   save time), so `Recent` means "recently added".
//! - **Packs.** A root's top-level folder holding a `pack.json` (optional `"name"`) is a
//!   pack: its items get that pack name (others the root name) and it is listed as a root
//!   `"<root>/<folder>"` (usable in `Query::roots`).
//! - **Incremental.** A rescan keeps the probed metadata of files whose size did not change
//!   and drops items no longer found. Favourites and tags are keyed by item id and survive
//!   rescans (and items that come back). A full rescan also runs every [`RESCAN_EVERY_MS`].
//! - **Preview.** `Preview { item, sync }` plays the item's source through the media preview
//!   (`media_preview`): with `sync`, repitched by project bpm / item bpm (a resample to
//!   `engine rate / ratio`, played back at the engine rate) and, while the transport plays,
//!   delayed to the next beat.

mod index;
mod names;
mod persist;

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::Cursor;

use ether_core::protocol::browser::{
    BrowserCommand, BrowserEvent, BrowserRoot, BrowserRootKind, LibraryItem, LibraryItemKind,
    LibraryItemMeta,
};
use ether_core::protocol::media::{BrowseLocation, FileKind, MediaSource};
use ether_core::protocol::model::{
    BuiltinDeviceType, PRESET_EXTENSION, PRESETS_DIR, load_preset,
};
use ether_core::protocol::presets::{PresetRef, PresetSource};
use ether_core::protocol::{Event, ReplyValue};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::handlers::{event, store_err};
use crate::media::extension_of;
use crate::media_preview::PreviewSync;
use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, invalid, not_found};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

use index::{Entry, Index};
use persist::{IndexFile, StoredItem, UserFolder};

/// Folder listings per tick.
pub(crate) const DIRS_PER_TICK: usize = 24;
/// Directory entries handled per tick (a listing is never split).
pub(crate) const ENTRIES_PER_TICK: usize = 4000;
/// Header probes per tick, and the bytes they may read.
const PROBE_FILES_PER_TICK: usize = 16;
const PROBE_BYTES_PER_TICK: f64 = (4 << 20) as f64;
/// Larger files are not probed (their header metadata stays unknown).
pub(crate) const MAX_PROBE_BYTES: f64 = (16 << 20) as f64;
/// Deepest folder level walked below a root.
const MAX_DEPTH: usize = 16;
const PROGRESS_EVERY_MS: u64 = 250;
const CHANGED_EVERY_MS: u64 = 2000;
/// Persist this long after the last change (later while a scan runs).
const PERSIST_AFTER_MS: u64 = 2000;
const PERSIST_LATEST_MS: u64 = 15_000;
pub(crate) const RESCAN_EVERY_MS: u64 = 15 * 60_000;
/// Tempo-sync ratios are clamped to this range.
const MIN_RATIO: f64 = 0.25;
const MAX_RATIO: f64 = 4.0;

/// Root id of the factory presets.
pub(crate) const FACTORY_ROOT: &str = "factory";
/// Root id of project items (not listed as a root).
pub(crate) const PROJECTS_ROOT: &str = "projects";
const USER_LIBRARY_NAME: &str = "User Library";
const PACK_FILE: &str = "pack.json";

/// One root being walked.
struct RootScan {
    root: String,
    name: String,
    /// Folders still to list (relative path, depth).
    dirs: VecDeque<(String, usize)>,
    /// Item ids found in this pass.
    seen: HashSet<String>,
    /// Files seen.
    scanned: u32,
    /// Pack folders found in this pass (folder → name).
    packs: BTreeMap<String, String>,
}

/// Browser runtime state (one field on `EtherController`).
#[derive(Default)]
pub(crate) struct BrowserState {
    started: bool,
    index: Index,
    folders: Vec<UserFolder>,
    /// Root id → pack folder → name.
    packs: BTreeMap<String, BTreeMap<String, String>>,
    scans: VecDeque<RootScan>,
    /// Audio items whose header is still to probe.
    probe: VecDeque<String>,
    /// Probes changed metadata since the last `IndexChanged`.
    probe_changed: bool,
    last_progress: u64,
    last_changed: u64,
    last_full_scan: u64,
    /// When the item cache should be written (`None` = clean).
    persist_due: Option<u64>,
    /// First change since the last write.
    dirty_since: Option<u64>,
    /// The user data changed: write `index.json` at the end of the command/tick.
    user_dirty: bool,
}

impl BrowserState {
    /// The items changed (written after a quiet period).
    fn mark_dirty(&mut self, now: u64) {
        self.persist_due = Some(now + PERSIST_AFTER_MS);
        self.dirty_since.get_or_insert(now);
    }
}

fn changed(out: &mut dyn MessageSink) {
    event(
        out,
        Event::Browser {
            event: BrowserEvent::IndexChanged,
        },
    );
}

fn progress(out: &mut dyn MessageSink, root: &str, scanned: u32, total: Option<u32>) {
    event(
        out,
        Event::Browser {
            event: BrowserEvent::IndexProgress {
                root: root.to_string(),
                scanned,
                total,
            },
        },
    );
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn stem(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(s, _)| s)
}

/// An audio/MIDI file item.
fn file_entry(
    root: &str,
    path: &str,
    midi: bool,
    size: f64,
    pack: Option<String>,
    added: Option<f64>,
) -> Entry {
    let name = file_name(path);
    let parsed = names::parse(stem(name), path);
    Entry::new(
        LibraryItem {
            id: format!("{root}/{path}"),
            kind: if midi {
                LibraryItemKind::Midi
            } else {
                LibraryItemKind::Audio
            },
            name: name.to_string(),
            root: root.to_string(),
            path: path.to_string(),
            source: Some(MediaSource::Location {
                location: BrowseLocation::Library {
                    id: root.to_string(),
                },
                path: path.to_string(),
            }),
            preset: None,
            tags: Vec::new(),
            favourite: false,
            meta: LibraryItemMeta {
                bpm: parsed.bpm,
                key: parsed.key,
                pack,
                modified_ms: added,
                size: Some(size),
                ..Default::default()
            },
        },
        None,
        Vec::new(),
    )
}

/// A preset item (`root`/`path` as listed; `preset` as `Preset::Load` takes it).
fn preset_entry(root: &str, path: &str, preset: PresetRef, p: &ether_core::protocol::model::Preset) -> Entry {
    Entry::new(
        LibraryItem {
            id: format!("{root}/{path}"),
            kind: LibraryItemKind::Preset,
            name: p.name.clone(),
            root: root.to_string(),
            path: path.to_string(),
            source: None,
            preset: Some(preset),
            tags: Vec::new(),
            favourite: false,
            meta: LibraryItemMeta::default(),
        },
        Some(p.device.clone()),
        index::normalize_tags(&p.meta.tags),
    )
}

/// Header metadata of an audio file: (sample rate, channels, duration in seconds).
fn probe(bytes: Vec<u8>, ext: Option<&str>) -> Option<(u32, u16, Option<f64>)> {
    let mss = MediaSourceStream::new(
        Box::new(Cursor::new(bytes)),
        MediaSourceStreamOptions::default(),
    );
    let mut hint = Hint::new();
    if let Some(ext) = ext {
        hint.with_extension(&ext.trim_start_matches('.').to_ascii_lowercase());
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .ok()?;
    let track = probed.format.default_track()?;
    let p = &track.codec_params;
    let rate = p.sample_rate.filter(|r| *r > 0)?;
    let channels = p.channels.map_or(0, |c| c.count()) as u16;
    let duration = p.n_frames.map(|n| n as f64 / rate as f64);
    Some((rate, channels, duration))
}

/// List one folder of `scan`; returns the number of entries handled.
fn list_folder<L: Library>(
    st: &mut BrowserState,
    library: &mut L,
    user_root: Option<&str>,
    dir: &str,
    depth: usize,
    now: u64,
) -> usize {
    let scan = st.scans.front_mut().expect("a scan is running");
    let Ok(listing) = library.list_dir(&scan.root, dir) else {
        return 1;
    };
    let root = scan.root.clone();
    if depth == 1 && listing.entries.iter().any(|e| e.name == PACK_FILE && e.kind != FileKind::Directory) {
        let name = library
            .read(&root, &format!("{dir}/{PACK_FILE}"))
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| v.get("name")?.as_str().map(str::to_string))
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| file_name(dir).to_string());
        scan.packs.insert(dir.to_string(), name);
    }
    let is_user = user_root == Some(root.as_str());
    let preset_ext = format!(".{PRESET_EXTENSION}");
    let preset_prefix = format!("{PRESETS_DIR}/");
    let n = listing.entries.len().max(1);
    for e in listing.entries {
        if e.name.starts_with('.') {
            continue;
        }
        let scan = st.scans.front_mut().expect("a scan is running");
        match e.kind {
            FileKind::Directory => {
                if depth < MAX_DEPTH {
                    scan.dirs.push_back((e.path, depth + 1));
                }
            }
            FileKind::Audio | FileKind::Midi => {
                scan.scanned += 1;
                let midi = e.kind == FileKind::Midi;
                let id = format!("{root}/{}", e.path);
                scan.seen.insert(id.clone());
                let pack = match e.path.split_once('/') {
                    Some((top, _)) => scan.packs.get(top).cloned(),
                    None => None,
                }
                .unwrap_or_else(|| scan.name.clone());
                let old = st.index.get(&id);
                let unchanged = old.is_some_and(|o| {
                    o.item.meta.size == Some(e.size)
                        && (o.item.kind == LibraryItemKind::Midi) == midi
                });
                if unchanged {
                    let o = st.index.get_mut(&id).expect("checked");
                    if o.item.meta.pack.as_deref() != Some(pack.as_str()) {
                        o.item.meta.pack = Some(pack);
                        o.refresh_text();
                    }
                    continue;
                }
                let added = old
                    .and_then(|o| o.item.meta.modified_ms)
                    .or(Some(now as f64));
                st.index
                    .upsert(file_entry(&root, &e.path, midi, e.size, Some(pack), added));
                if !midi {
                    st.probe.push_back(id);
                }
            }
            FileKind::Other => {
                if is_user && e.path.starts_with(&preset_prefix) && e.name.ends_with(&preset_ext) {
                    scan.scanned += 1;
                    let Some(p) = library
                        .read(&root, &e.path)
                        .ok()
                        .and_then(|b| String::from_utf8(b).ok())
                        .and_then(|json| load_preset(&json).ok())
                    else {
                        continue;
                    };
                    let id = format!("{root}/{}", e.path);
                    let scan = st.scans.front_mut().expect("a scan is running");
                    scan.seen.insert(id);
                    let preset = PresetRef {
                        source: PresetSource::User,
                        id: e.path[preset_prefix.len()..].to_string(),
                    };
                    st.index.upsert(preset_entry(&root, &e.path, preset, &p));
                }
            }
        }
    }
    n
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    pub(crate) fn browser_command(
        &mut self,
        command: &BrowserCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        self.browser_start(now);
        match command {
            BrowserCommand::Query { query } => Ok(ReplyValue::BrowserPage {
                page: self.browser.index.query(query),
            }),
            BrowserCommand::ListRoots => Ok(ReplyValue::BrowserRoots {
                roots: self.browser_roots(),
            }),
            BrowserCommand::SetFavourite { item, favourite } => {
                if !self.browser.index.set_favourite(item, *favourite) {
                    return Err(not_found(format!("library item {item}")));
                }
                self.browser_save_user();
                changed(out);
                Ok(ReplyValue::Unit)
            }
            BrowserCommand::SetTags { item, tags } => {
                if !self.browser.index.set_tags(item, tags) {
                    return Err(not_found(format!("library item {item}")));
                }
                self.browser_save_user();
                changed(out);
                Ok(ReplyValue::Unit)
            }
            BrowserCommand::AddFolder { path } => {
                if path.trim().is_empty() {
                    return Err(invalid("folder path is empty"));
                }
                let id = self.library.add_folder(path).map_err(store_err)?;
                let st = &mut self.browser;
                st.folders.retain(|f| f.id != id);
                st.folders.push(UserFolder {
                    id: id.clone(),
                    path: path.clone(),
                });
                self.browser_queue_scan(&id);
                self.browser_save_user();
                Ok(ReplyValue::BrowserRoots {
                    roots: self.browser_roots(),
                })
            }
            BrowserCommand::RemoveFolder { root } => {
                if !self.browser.folders.iter().any(|f| &f.id == root) {
                    return Err(not_found(format!("user folder {root}")));
                }
                self.library.remove_folder(root).map_err(store_err)?;
                let st = &mut self.browser;
                st.folders.retain(|f| &f.id != root);
                st.scans.retain(|s| &s.root != root);
                st.packs.remove(root);
                st.index.remove_where(|e| &e.item.root == root);
                st.mark_dirty(now);
                self.browser_save_user();
                changed(out);
                Ok(ReplyValue::Unit)
            }
            BrowserCommand::Rescan { root } => {
                match root.as_deref() {
                    None => self.browser_scan_all(now),
                    Some(FACTORY_ROOT) => {
                        self.browser_refresh_factory();
                        changed(out);
                    }
                    Some(PROJECTS_ROOT) => {
                        self.browser_refresh_projects(now);
                        changed(out);
                    }
                    Some(r) => {
                        // A pack rescans its root.
                        let base = r.split('/').next().unwrap_or(r);
                        if !self.browser_scan_roots().iter().any(|(id, _)| id == base) {
                            return Err(not_found(format!("library root {r}")));
                        }
                        self.browser_queue_scan(base);
                    }
                }
                Ok(ReplyValue::Unit)
            }
            BrowserCommand::Preview { item, sync } => {
                let entry = self
                    .browser
                    .index
                    .get(item)
                    .ok_or_else(|| not_found(format!("library item {item}")))?;
                let source = match (&entry.item.kind, &entry.item.source) {
                    (LibraryItemKind::Audio, Some(s)) => s.clone(),
                    _ => return Err(invalid("only audio items can be previewed")),
                };
                let bpm = entry.item.meta.bpm;
                let sync = if *sync { self.browser_sync(bpm) } else { None };
                self.start_preview(&source, sync, out)?;
                Ok(ReplyValue::Unit)
            }
        }
    }

    /// Called every tick.
    pub(crate) fn browser_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        if !self.browser.started {
            return;
        }
        let user_root = self.library.user_root();
        let (mut dirs, mut entries) = (0, 0);
        let mut worked = false;
        while dirs < DIRS_PER_TICK && entries < ENTRIES_PER_TICK {
            let Some(scan) = self.browser.scans.front_mut() else {
                break;
            };
            worked = true;
            match scan.dirs.pop_front() {
                Some((dir, depth)) => {
                    dirs += 1;
                    entries += list_folder(
                        &mut self.browser,
                        &mut self.library,
                        user_root.as_deref(),
                        &dir,
                        depth,
                        now,
                    );
                }
                None => {
                    let scan = self.browser.scans.pop_front().expect("checked");
                    let st = &mut self.browser;
                    st.index
                        .remove_where(|e| e.item.root == scan.root && !scan.seen.contains(&e.item.id));
                    let packs_changed = st.packs.get(&scan.root).map_or(!scan.packs.is_empty(), |p| *p != scan.packs);
                    if scan.packs.is_empty() {
                        st.packs.remove(&scan.root);
                    } else {
                        st.packs.insert(scan.root.clone(), scan.packs);
                    }
                    st.user_dirty |= packs_changed;
                    st.mark_dirty(now);
                    st.last_progress = now;
                    progress(out, &scan.root, scan.scanned, Some(scan.scanned));
                    changed(out);
                    st.last_changed = now;
                }
            }
        }
        if worked
            && now >= self.browser.last_progress + PROGRESS_EVERY_MS
            && let Some(scan) = self.browser.scans.front()
        {
            progress(out, &scan.root, scan.scanned, None);
            self.browser.last_progress = now;
        }
        if self.browser.scans.is_empty() {
            self.browser_probe(now, out);
        }
        if self.browser.user_dirty {
            self.browser_save_user();
        }
        self.browser_persist(now);
        let st = &self.browser;
        if st.scans.is_empty() && st.probe.is_empty() && now >= st.last_full_scan + RESCAN_EVERY_MS {
            self.browser_scan_all(now);
        }
    }

    /// Load the persisted index and queue the first scan (once).
    fn browser_start(&mut self, now: u64) {
        if self.browser.started {
            return;
        }
        self.browser.started = true;
        let file = self.library.user_root().and_then(|root| {
            let index = self.library.read(&root, persist::INDEX_PATH).ok()?;
            let items = self.library.read(&root, persist::ITEMS_PATH).ok();
            persist::parse(&index, items.as_deref())
        });
        if let Some(file) = file {
            for folder in file.folders {
                if let Ok(id) = self.library.add_folder(&folder.path) {
                    self.browser.folders.push(UserFolder {
                        id,
                        path: folder.path,
                    });
                }
            }
            let roots: HashMap<String, String> = self.browser_scan_roots().into_iter().collect();
            let st = &mut self.browser;
            st.index.favourites = file.favourites;
            st.index.tags = file.tags;
            st.packs = file.packs;
            for item in file.items {
                let Some(root_name) = roots.get(&item.root) else {
                    continue;
                };
                let pack = item
                    .path
                    .split_once('/')
                    .and_then(|(top, _)| st.packs.get(&item.root)?.get(top).cloned())
                    .unwrap_or_else(|| root_name.clone());
                let mut e = file_entry(&item.root, &item.path, item.midi, item.size, Some(pack), item.added);
                e.probed = item.probed;
                e.item.meta.duration_seconds = item.duration;
                e.item.meta.sample_rate = item.rate;
                e.item.meta.channels = item.channels;
                let id = e.item.id.clone();
                st.index.upsert(e);
                if !item.probed && !item.midi {
                    st.probe.push_back(id);
                }
            }
        }
        self.browser_scan_all(now);
    }

    /// Scanned roots: (id, display name). Library roots, user folders, the user library.
    fn browser_scan_roots(&self) -> Vec<(String, String)> {
        let mut roots: Vec<(String, String)> = self
            .library
            .roots()
            .into_iter()
            .filter_map(|r| match r.location {
                BrowseLocation::Library { id } => Some((id, r.name)),
                BrowseLocation::ProjectMedia => None,
            })
            .collect();
        if let Some(user) = self.library.user_root()
            && !roots.iter().any(|(id, _)| *id == user)
        {
            roots.push((user, USER_LIBRARY_NAME.to_string()));
        }
        roots
    }

    fn browser_queue_scan(&mut self, root: &str) {
        let Some((_, name)) = self
            .browser_scan_roots()
            .into_iter()
            .find(|(id, _)| id == root)
        else {
            return;
        };
        let st = &mut self.browser;
        st.scans.retain(|s| s.root != root);
        st.scans.push_back(RootScan {
            root: root.to_string(),
            name,
            dirs: VecDeque::from([(String::new(), 0)]),
            seen: HashSet::new(),
            scanned: 0,
            packs: BTreeMap::new(),
        });
    }

    fn browser_scan_all(&mut self, now: u64) {
        self.browser.last_full_scan = now;
        self.browser_refresh_factory();
        self.browser_refresh_projects(now);
        // Roots that disappeared (unconfigured) drop their items.
        let roots = self.browser_scan_roots();
        let known: HashSet<&str> = roots
            .iter()
            .map(|(id, _)| id.as_str())
            .chain([FACTORY_ROOT, PROJECTS_ROOT])
            .collect();
        if self.browser.index.remove_where(|e| !known.contains(e.item.root.as_str())) > 0 {
            self.browser.mark_dirty(now);
        }
        for (id, _) in &roots {
            self.browser_queue_scan(id);
        }
    }

    fn browser_refresh_factory(&mut self) {
        let st = &mut self.browser;
        st.index.remove_where(|e| e.item.root == FACTORY_ROOT);
        for ty in BuiltinDeviceType::ALL {
            for f in ether_devices::factory_presets(ty) {
                let Ok(p) = load_preset(f.json) else {
                    continue;
                };
                let preset = PresetRef {
                    source: PresetSource::Factory,
                    id: f.id.to_string(),
                };
                st.index.upsert(preset_entry(FACTORY_ROOT, f.id, preset, &p));
            }
        }
    }

    fn browser_refresh_projects(&mut self, now: u64) {
        let _ = now;
        let Ok(projects) = self.store.list() else {
            return;
        };
        let st = &mut self.browser;
        st.index.remove_where(|e| e.item.root == PROJECTS_ROOT);
        for p in projects {
            let path = p.id.to_string();
            st.index.upsert(Entry::new(
                LibraryItem {
                    id: format!("{PROJECTS_ROOT}/{path}"),
                    kind: LibraryItemKind::Project,
                    name: p.name,
                    root: PROJECTS_ROOT.to_string(),
                    path,
                    source: None,
                    preset: None,
                    tags: Vec::new(),
                    favourite: false,
                    meta: LibraryItemMeta {
                        modified_ms: Some(p.modified_ms),
                        ..Default::default()
                    },
                },
                None,
                Vec::new(),
            ));
        }
    }

    fn browser_roots(&self) -> Vec<BrowserRoot> {
        let st = &self.browser;
        let mut counts: HashMap<&str, u32> = HashMap::new();
        let mut pack_counts: HashMap<(&str, &str), u32> = HashMap::new();
        for e in st.index.entries() {
            let i = &e.item;
            *counts.entry(i.root.as_str()).or_default() += 1;
            if let Some((top, _)) = i.path.split_once('/') {
                *pack_counts.entry((i.root.as_str(), top)).or_default() += 1;
            }
        }
        let mut out = Vec::new();
        for (id, name) in self.browser_scan_roots() {
            let folder = st.folders.iter().find(|f| f.id == id);
            out.push(BrowserRoot {
                id: id.clone(),
                name,
                kind: if folder.is_some() {
                    BrowserRootKind::Folder
                } else {
                    BrowserRootKind::Library
                },
                path: folder.map(|f| f.path.clone()),
                items: counts.get(id.as_str()).copied().unwrap_or(0),
            });
            for (dir, name) in st.packs.get(&id).into_iter().flatten() {
                out.push(BrowserRoot {
                    id: format!("{id}/{dir}"),
                    name: name.clone(),
                    kind: BrowserRootKind::Pack,
                    path: None,
                    items: pack_counts
                        .get(&(id.as_str(), dir.as_str()))
                        .copied()
                        .unwrap_or(0),
                });
            }
        }
        out.push(BrowserRoot {
            id: FACTORY_ROOT.to_string(),
            name: "Factory Presets".to_string(),
            kind: BrowserRootKind::Factory,
            path: None,
            items: counts.get(FACTORY_ROOT).copied().unwrap_or(0),
        });
        out
    }

    /// Tempo sync of a preview: the repitch ratio (project bpm / item bpm; 1 without an item
    /// bpm or a project) and whether to wait for the next beat (transport playing).
    fn browser_sync(&self, bpm: Option<f64>) -> Option<PreviewSync> {
        let doc = self.doc.as_ref()?;
        let project_bpm = doc.project.tempo_map().bpm_at(self.transport.position);
        let ratio = match bpm {
            Some(b) if b > 0.0 && project_bpm > 0.0 => (project_bpm / b).clamp(MIN_RATIO, MAX_RATIO),
            _ => 1.0,
        };
        Some(PreviewSync {
            ratio,
            align: self.transport.playing,
        })
    }

    /// Probe audio headers within the per-tick budget.
    fn browser_probe(&mut self, now: u64, out: &mut dyn MessageSink) {
        let (mut files, mut bytes) = (0, 0.0);
        while files < PROBE_FILES_PER_TICK && bytes < PROBE_BYTES_PER_TICK {
            let Some(id) = self.browser.probe.pop_front() else {
                break;
            };
            let Some(e) = self.browser.index.get(&id).filter(|e| !e.probed) else {
                continue;
            };
            files += 1;
            let (root, path) = (e.item.root.clone(), e.item.path.clone());
            let size = e.item.meta.size.unwrap_or(0.0);
            let meta = if size <= MAX_PROBE_BYTES {
                bytes += size;
                self.library
                    .read(&root, &path)
                    .ok()
                    .and_then(|b| probe(b, extension_of(&path)))
            } else {
                None
            };
            let st = &mut self.browser;
            if let Some(e) = st.index.get_mut(&id) {
                e.probed = true;
                if let Some((rate, channels, duration)) = meta {
                    e.item.meta.sample_rate = Some(rate);
                    e.item.meta.channels = Some(channels);
                    e.item.meta.duration_seconds = duration;
                    st.probe_changed = true;
                }
            }
            st.mark_dirty(now);
        }
        let st = &mut self.browser;
        if st.probe_changed && (st.probe.is_empty() || now >= st.last_changed + CHANGED_EVERY_MS) {
            st.probe_changed = false;
            st.last_changed = now;
            changed(out);
        }
    }

    /// Write the index when due.
    fn browser_persist(&mut self, now: u64) {
        let st = &self.browser;
        let Some(due) = st.persist_due else {
            return;
        };
        let busy = !st.scans.is_empty();
        let overdue = st
            .dirty_since
            .is_some_and(|t| now >= t + PERSIST_LATEST_MS);
        if now < due || (busy && !overdue) {
            return;
        }
        let Some(root) = self.library.user_root() else {
            self.browser.persist_due = None;
            self.browser.dirty_since = None;
            return;
        };
        let items: Vec<StoredItem> = st
                .index
                .entries()
                .filter(|e| matches!(e.item.kind, LibraryItemKind::Audio | LibraryItemKind::Midi))
                .map(|e| StoredItem {
                    root: e.item.root.clone(),
                    path: e.item.path.clone(),
                    midi: e.item.kind == LibraryItemKind::Midi,
                    size: e.item.meta.size.unwrap_or(0.0),
                    added: e.item.meta.modified_ms,
                    probed: e.probed,
                    duration: e.item.meta.duration_seconds,
                    rate: e.item.meta.sample_rate,
                    channels: e.item.meta.channels,
                })
                .collect();
        // A failed write is retried with the next change.
        let _ = self
            .library
            .write_file(&root, persist::ITEMS_PATH, &persist::serialize_items(&items));
        self.browser.persist_due = None;
        self.browser.dirty_since = None;
    }

    /// Write `index.json` (user folders, favourites, tags, packs) now.
    fn browser_save_user(&mut self) {
        let st = &mut self.browser;
        st.user_dirty = false;
        let Some(root) = self.library.user_root() else {
            return;
        };
        let file = IndexFile {
            folders: st.folders.clone(),
            favourites: st.index.favourites.clone(),
            tags: st.index.tags.clone(),
            packs: st.packs.clone(),
            items: Vec::new(),
        };
        // Best effort (a read-only library keeps them for the session).
        let _ = self
            .library
            .write_file(&root, persist::INDEX_PATH, &persist::serialize(&file));
    }
}

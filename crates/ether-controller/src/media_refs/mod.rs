//! External media references: missing media, relink, search, collect (v0.2, owned by the
//! `media-references` node; model `ether_model::MediaLocation`, protocol
//! `ether_protocol::media_refs`, CONTRACTS.md §12.9).
//!
//! - **Import in place** (`handlers.rs`): `MediaSource::Path` and library files the host can
//!   reference (`Library::external_path`) become `MediaLocation::External { path }` with a
//!   content hash; nothing is copied. Uploads, recordings, bounces and freezes stay project
//!   media.
//! - **Resolution** (`media/`, [`crate::media::read_media_bytes`]): an external path whose
//!   hash matches → the project copy at `MediaRef::file` → missing (`MediaEvent::Missing`,
//!   silence). The media pipeline checks every media in the background when a project
//!   opens and keeps the missing set ([`crate::media::MediaState::missing`]).
//! - **Commands** ([`EtherController::media_ref_command`]): `ListMissing` (also re-checks
//!   the missing files: a restored file reports `MediaRefEvent::Resolved`), `Relink` (one
//!   undo step; content changes update the hash with a warning), `Search` (a background job
//!   stepped by [`EtherController::media_refs_tick`]: library roots and an optional OS
//!   folder, by file name, then by content hash; hash matches are relinked in one undo
//!   step, the rest reported as `Candidates`), `CollectAll` (copy every external reference
//!   into `media/`, one undo step, then save).
//! - **Collab**: a media's location is per-site (`collab/`): local paths never leave this
//!   machine; peers get the bytes by hash at `MediaRef::file`.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use ether_core::protocol::media::{BrowseLocation, FileKind, MediaSource};
use ether_core::protocol::media_refs::{MediaRefCommand, MediaRefEvent};
use ether_core::protocol::model::file::MEDIA_DIR;
use ether_core::protocol::model::{
    EntityUpdate, MediaChange, MediaId, MediaLocation, MediaRef, ProjectId,
};
use ether_core::protocol::{ErrorCode, Event, NotificationLevel, ReplyValue};

use crate::handlers::{event, no_project, notify, store_err};
use crate::media::{IncrementalDecoder, extension_of, read_media_bytes};
use crate::store::{Library, ProjectStore, StoreError, check_relative_path, file_kind};
use crate::tx::{CmdResult, cmd_err, invalid, invalid_state, not_found};
use crate::{EngineBridge, EtherController, HostServices, MessageSink, content_hash};

/// Folders listed per tick by a search (a file read for the hash check counts as
/// [`HASH_COST`] listings).
const SEARCH_UNITS_PER_TICK: u32 = 32;
const HASH_COST: u32 = 16;
/// A search gives up on deeper folders / after this many folders.
const MAX_DEPTH: u8 = 24;
const MAX_FOLDERS: u32 = 50_000;

/// Runtime state (lives in `MediaState::refs`; reset with the project).
#[derive(Default)]
pub(crate) struct RefsState {
    search: Option<Box<Search>>,
}

/// A file found by a search.
#[derive(Clone, Debug, PartialEq)]
enum Found {
    Library { root: String, rel: String },
    External { path: String },
}

impl Found {
    fn source(&self) -> MediaSource {
        match self {
            Found::Library { root, rel } => MediaSource::Location {
                location: BrowseLocation::Library { id: root.clone() },
                path: rel.clone(),
            },
            Found::External { path } => MediaSource::Path { path: path.clone() },
        }
    }
}

enum Folder {
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

struct Search {
    /// The media searched for (missing ones, or the one asked for).
    targets: BTreeMap<MediaId, MediaRef>,
    /// Lowercase file name → media with that name.
    names: BTreeMap<String, Vec<MediaId>>,
    folders: VecDeque<Folder>,
    /// Name matches waiting for the content-hash check.
    to_hash: VecDeque<(MediaId, Found)>,
    matched: BTreeMap<MediaId, Found>,
    candidates: BTreeMap<MediaId, Vec<Found>>,
    /// Folders listed so far.
    scanned: u32,
}

/// The file name of a path (either separator), lowercased for matching.
fn name_key(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_lowercase()
}

impl Search {
    fn new(targets: Vec<MediaRef>) -> Self {
        let mut names: BTreeMap<String, Vec<MediaId>> = BTreeMap::new();
        for m in &targets {
            // The display name and, for a reference, the referenced file's name.
            let mut keys = BTreeSet::from([name_key(&m.name)]);
            if let MediaLocation::External { path } = &m.location {
                keys.insert(name_key(path));
            }
            for k in keys {
                names.entry(k).or_default().push(m.id);
            }
        }
        Self {
            targets: targets.into_iter().map(|m| (m.id, m)).collect(),
            names,
            folders: VecDeque::new(),
            to_hash: VecDeque::new(),
            matched: BTreeMap::new(),
            candidates: BTreeMap::new(),
            scanned: 0,
        }
    }

    /// A file seen while walking: queued for the hash check when its name matches.
    fn saw_file(&mut self, name: &str, found: Found) {
        if file_kind(name) != FileKind::Audio {
            return;
        }
        if let Some(ids) = self.names.get(&name_key(name)) {
            for id in ids {
                self.to_hash.push_back((*id, found.clone()));
            }
        }
    }

    /// List one folder. Hidden entries and too-deep folders are skipped.
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
                    let name = name_key(&child);
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
                        let file = child.rsplit(['/', '\\']).next().unwrap_or("").to_string();
                        self.saw_file(&file, Found::External { path: child });
                    }
                }
            }
        }
    }

    /// Check one name match's content hash.
    fn check<L: Library>(&mut self, library: &mut L, id: MediaId, found: Found) {
        if self.matched.contains_key(&id) {
            return;
        }
        let bytes = match &found {
            Found::Library { root, rel } => library.read(root, rel),
            Found::External { path } => library.read_external(path),
        };
        let Ok(bytes) = bytes else { return };
        let target = &self.targets[&id];
        if target.hash.as_deref() == Some(content_hash(&bytes).as_str()) {
            self.matched.insert(id, found);
        } else {
            let c = self.candidates.entry(id).or_default();
            if !c.contains(&found) {
                c.push(found);
            }
        }
    }

    /// Advance by about `units`. `true` when finished.
    fn step<L: Library>(&mut self, library: &mut L, mut units: u32) -> bool {
        while units > 0 {
            if let Some((id, found)) = self.to_hash.pop_front() {
                self.check(library, id, found);
                units = units.saturating_sub(HASH_COST);
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
}

/// Header check for a relink: the new file must have the same sample rate and channel
/// count (clip timing and routing depend on them). The length may differ (the document's
/// length stays the reference, like any decode).
fn check_format(m: &MediaRef, bytes: Vec<u8>, ext_of: &str) -> CmdResult<Vec<u8>> {
    let bytes: Arc<[u8]> = bytes.into();
    let mut decoder = IncrementalDecoder::new(bytes.clone(), extension_of(ext_of))
        .map_err(|e| cmd_err(ErrorCode::Decode, e.to_string()))?;
    if decoder.sample_rate == 0 || decoder.channels == 0 {
        decoder
            .step(1)
            .map_err(|e| cmd_err(ErrorCode::Decode, e.to_string()))?;
    }
    if decoder.sample_rate != m.sample_rate || decoder.channels as u16 != m.channels {
        return Err(invalid(format!(
            "\"{}\" is {} Hz, {} ch; the file is {} Hz, {} ch",
            m.name, m.sample_rate, m.channels, decoder.sample_rate, decoder.channels
        )));
    }
    Ok(bytes.to_vec())
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    pub(crate) fn media_ref_command(
        &mut self,
        command: &MediaRefCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let doc = self.doc.as_ref().ok_or_else(no_project)?;
        match command {
            MediaRefCommand::ListMissing => {
                let media = self.media.missing();
                // Files may have come back: check them again (`Resolved` when they load).
                if !media.is_empty() {
                    self.media.recheck_missing();
                    self.media.sync(&mut self.bridge, Some(&doc.project));
                }
                Ok(ReplyValue::MissingMedia { media })
            }
            MediaRefCommand::Relink { media, source } => {
                self.relink(*media, source, now, out)?;
                Ok(ReplyValue::Unit)
            }
            MediaRefCommand::Search { media, folder } => {
                let targets: Vec<MediaRef> = match media {
                    Some(id) => vec![
                        doc.project
                            .media
                            .get(id)
                            .cloned()
                            .ok_or_else(|| not_found(format!("media {id}")))?,
                    ],
                    None => self
                        .media
                        .missing()
                        .iter()
                        .filter_map(|id| doc.project.media.get(id).cloned())
                        .collect(),
                };
                let mut search = Search::new(targets);
                if let Some(folder) = folder {
                    // Validates it (absolute, a folder; `Unsupported` without OS files).
                    self.library.list_external_dir(folder).map_err(store_err)?;
                    search.folders.push_back(Folder::External {
                        path: folder.clone(),
                        depth: 0,
                    });
                }
                for root in self.library.roots() {
                    if let BrowseLocation::Library { id } = root.location {
                        search.folders.push_back(Folder::Library {
                            root: id,
                            rel: String::new(),
                            depth: 0,
                        });
                    }
                }
                if search.targets.is_empty() {
                    search.folders.clear();
                }
                self.media.refs.search = Some(Box::new(search));
                event(
                    out,
                    Event::MediaRef {
                        event: MediaRefEvent::SearchProgress {
                            scanned: 0,
                            total: None,
                        },
                    },
                );
                Ok(ReplyValue::Unit)
            }
            MediaRefCommand::CollectAll => {
                self.collect_all(now, out)?;
                Ok(ReplyValue::Unit)
            }
        }
    }

    /// Called every tick: steps a running search.
    pub(crate) fn media_refs_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        let Some(search) = self.media.refs.search.as_mut() else {
            return;
        };
        let before = search.scanned;
        let done = search.step(&mut self.library, SEARCH_UNITS_PER_TICK);
        if !done {
            if search.scanned / 16 != before / 16 {
                event(
                    out,
                    Event::MediaRef {
                        event: MediaRefEvent::SearchProgress {
                            scanned: search.scanned,
                            total: None,
                        },
                    },
                );
            }
            return;
        }
        let search = self.media.refs.search.take().expect("running");
        self.finish_search(*search, now, out);
    }

    fn finish_search(&mut self, search: Search, now: u64, out: &mut dyn MessageSink) {
        let Some(pid) = self.doc.as_ref().map(|d| d.project.id) else {
            return;
        };
        // Hash matches: the same content, relinked in one undo step.
        let mut relinks: Vec<(MediaId, MediaLocation)> = Vec::new();
        for (id, found) in &search.matched {
            let Some(m) = search.targets.get(id) else {
                continue;
            };
            let location = match found {
                Found::External { path } => MediaLocation::External { path: path.clone() },
                Found::Library { root, rel } => match self.library.external_path(root, rel) {
                    Some(path) => MediaLocation::External { path },
                    // This host cannot reference it: copy it into the project.
                    None => {
                        let copied = self
                            .library
                            .read(root, rel)
                            .and_then(|bytes| self.store.write(pid, &m.file, &bytes));
                        if copied.is_err() {
                            continue;
                        }
                        MediaLocation::Project
                    }
                },
            };
            relinks.push((*id, location));
        }
        let still_here = |c: &Self, id: &MediaId| {
            c.doc
                .as_ref()
                .is_some_and(|d| d.project.media.contains_key(id))
        };
        relinks.retain(|(id, _)| still_here(self, id));
        let relinked: BTreeSet<MediaId> = relinks.iter().map(|(id, _)| *id).collect();
        let r = self.edit_with("Relink", None, now, out, |ctx| {
            for (id, location) in relinks {
                if ctx.p().media[&id].location != location {
                    ctx.tx.update(EntityUpdate::Media {
                        id,
                        change: MediaChange::Location(location),
                    })?;
                }
            }
            Ok(())
        });
        if let Err(e) = r {
            notify(
                out,
                NotificationLevel::Error,
                format!("could not relink: {}", e.message),
            );
        }
        self.recheck(&relinked);
        for id in search.targets.keys() {
            if relinked.contains(id) {
                continue;
            }
            let candidates = search
                .candidates
                .get(id)
                .map(|c| c.iter().map(Found::source).collect())
                .unwrap_or_default();
            event(
                out,
                Event::MediaRef {
                    event: MediaRefEvent::Candidates {
                        media: *id,
                        candidates,
                    },
                },
            );
        }
        event(
            out,
            Event::MediaRef {
                event: MediaRefEvent::SearchProgress {
                    scanned: search.scanned,
                    total: Some(search.scanned),
                },
            },
        );
    }

    /// Retry loading `media` that were missing and whose document entry didn't change
    /// (e.g. the file came back at the same place).
    fn recheck(&mut self, media: &BTreeSet<MediaId>) {
        if media.iter().any(|m| self.media.is_missing(*m)) {
            self.media.recheck_missing();
            if let Some(doc) = self.doc.as_ref() {
                self.media.sync(&mut self.bridge, Some(&doc.project));
            }
        }
    }

    fn relink(
        &mut self,
        id: MediaId,
        source: &MediaSource,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<()> {
        let doc = self.doc.as_ref().ok_or_else(no_project)?;
        let pid = doc.project.id;
        let m = doc
            .project
            .media
            .get(&id)
            .cloned()
            .ok_or_else(|| not_found(format!("media {id}")))?;
        let (bytes, location, name) = match source {
            MediaSource::Path { path } => {
                let (bytes, name) = crate::file_import::read_path(&mut self.library, path)?;
                (bytes, MediaLocation::External { path: path.clone() }, name)
            }
            MediaSource::Location { location, path } => {
                check_relative_path(path).map_err(store_err)?;
                if path.is_empty() {
                    return Err(invalid("path must name a file"));
                }
                match location {
                    BrowseLocation::Library { id: root } => {
                        let bytes = self.library.read(root, path).map_err(store_err)?;
                        let at = match self.library.external_path(root, path) {
                            Some(p) => MediaLocation::External { path: p },
                            None => MediaLocation::Project,
                        };
                        (bytes, at, path.clone())
                    }
                    BrowseLocation::ProjectMedia => {
                        let bytes = self
                            .store
                            .read(pid, &format!("{MEDIA_DIR}/{path}"))
                            .map_err(store_err)?;
                        (bytes, MediaLocation::Project, path.clone())
                    }
                }
            }
            MediaSource::Upload { upload } => {
                let (bytes, name) =
                    crate::upload::take_upload(&mut self.uploads, &mut self.store, upload)?;
                (bytes, MediaLocation::Project, name)
            }
            MediaSource::Project { .. } => {
                return Err(invalid("relink to a file, not to another media"));
            }
        };
        let bytes = check_format(&m, bytes, &name)?;
        let hash = content_hash(&bytes);
        let changed = m.hash.as_deref() != Some(hash.as_str());
        if changed && self.collab_active() {
            // Peers hold these bytes at `MediaRef::file` (and the relay caches them for late
            // joiners): stored media are never replaced, so the content can't change here.
            return Err(invalid_state(format!(
                "\"{}\" has different content: in a session, relink to the same file (or import it as a new sample)",
                m.name
            )));
        }
        if location == MediaLocation::Project {
            // Project media live at `MediaRef::file` (named per media id).
            let same = self.store.read(pid, &m.file).is_ok_and(|b| b == bytes);
            if !same {
                self.store.write(pid, &m.file, &bytes).map_err(store_err)?;
            }
        }
        let moved = m.location != location;
        self.edit_with("Relink", None, now, out, |ctx| {
            if moved {
                ctx.tx.update(EntityUpdate::Media {
                    id,
                    change: MediaChange::Location(location),
                })?;
            }
            if changed {
                ctx.tx.update(EntityUpdate::Media {
                    id,
                    change: MediaChange::Hash(Some(hash)),
                })?;
            }
            Ok(())
        })?;
        if changed {
            notify(
                out,
                NotificationLevel::Warning,
                format!(
                    "\"{}\" was relinked to a file with different content",
                    m.name
                ),
            );
        }
        if !moved && !changed {
            // The same file at the same place (restored): load it again.
            self.recheck(&BTreeSet::from([id]));
        }
        Ok(())
    }

    fn collect_all(&mut self, now: u64, out: &mut dyn MessageSink) -> CmdResult<()> {
        let doc = self.doc.as_ref().ok_or_else(no_project)?;
        let pid = doc.project.id;
        let external: Vec<MediaRef> = doc
            .project
            .media
            .values()
            .filter(|m| matches!(m.location, MediaLocation::External { .. }))
            .cloned()
            .collect();
        let total = external.len() as u32;
        let mut collected = Vec::new();
        let mut skipped = Vec::new();
        for (i, m) in external.iter().enumerate() {
            match self.collect_one(pid, m) {
                Ok(()) => collected.push(m.id),
                Err(_) => skipped.push(m.name.clone()),
            }
            event(
                out,
                Event::MediaRef {
                    event: MediaRefEvent::CollectProgress {
                        done: i as u32 + 1,
                        total,
                    },
                },
            );
        }
        self.edit_with("Collect All", None, now, out, |ctx| {
            for id in collected {
                ctx.tx.update(EntityUpdate::Media {
                    id,
                    change: MediaChange::Location(MediaLocation::Project),
                })?;
            }
            Ok(())
        })?;
        if !skipped.is_empty() {
            notify(
                out,
                NotificationLevel::Warning,
                format!(
                    "{} missing file(s) could not be collected: {}",
                    skipped.len(),
                    skipped.join(", ")
                ),
            );
        }
        self.collab_before_save(now, out);
        self.save_current(out)?;
        Ok(())
    }

    /// Copy one external media into the project (its resolved bytes, at `MediaRef::file`).
    fn collect_one(&mut self, pid: ProjectId, m: &MediaRef) -> Result<(), StoreError> {
        let bytes = read_media_bytes(&mut self.store, &mut self.library, pid, m)?;
        if self.store.read(pid, &m.file).is_ok_and(|b| b == bytes) {
            return Ok(());
        }
        self.store.write(pid, &m.file, &bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_case_insensitively_on_either_separator() {
        assert_eq!(name_key("/a/b/Kick 01.WAV"), "kick 01.wav");
        assert_eq!(name_key("C:\\x\\Snare.aif"), "snare.aif");
        assert_eq!(name_key("loop.flac"), "loop.flac");
    }
}

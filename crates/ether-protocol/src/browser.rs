//! Browser v2 (v0.2; CONTRACTS.md §12.8): an engine-side library index with
//! search, tags, favourites, sample packs and user folders, paged queries, and tempo-synced
//! preview. v0.1's `Media::{ListLocations, ListDirectory}` keep working (folder view).
//!
//! The index lives engine-side (controller `browser/` module), persisted in the user library
//! (`<library>/.ethereal/index.json`); the UI never touches files. User folders are added by
//! path (native only: the desktop shell picks the folder with a native dialog and sends the
//! path; web and remote reply `Unsupported`). Indexing runs in the background (bounded work
//! per tick) and reports `BrowserEvent::IndexProgress`.
//!
//! Folders from the UI machine (additive): where the engine can't see the user's
//! folder (web: OPFS; a remote engine), the UI copies it: `ImportFolder { name }` creates an
//! empty, engine-owned user folder, `ImportFile` writes each uploaded audio file into it
//! (`Media::BeginUpload`/`UploadChunk` first), then `Rescan` indexes it. Removing an imported
//! folder deletes the copy. `RenameFolder` renames any user folder (display only).
//!
//! File filters (additive): which files the library shows, as regex rules on the
//! file name ([`FileFilter`]); applied engine-side to `Media::ListDirectory`, the index and
//! folder imports. `GetFileFilter` / `SetFileFilter` / `ResetFileFilter` /
//! `CheckFileFilter`; stored with the user's settings (`<user library>/Settings/
//! library-filters.json`); `BrowserEvent::FileFilterChanged` when it changes.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::media::MediaSource;
use crate::model::PresetDevice;
use crate::presets::PresetRef;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum BrowserCommand {
    /// Replies `BrowserPage`.
    Query { query: BrowserQuery },
    /// Replies `BrowserRoots`: library roots, sample packs and user folders.
    ListRoots,
    /// Mark/unmark a favourite (persisted in the index).
    SetFavourite { item: String, favourite: bool },
    /// Replace an item's user tags (lowercase, deduplicated).
    SetTags { item: String, tags: Vec<String> },
    /// Add a user folder (absolute engine-side path from the desktop's OS dialog). Replies
    /// `BrowserRoots`. Hosts whose UI is not on the engine machine (web, remote) reply
    /// `Unsupported` (`ImportFolder` instead).
    AddFolder { path: String },
    /// Remove a user folder from the index (files untouched; an imported folder's copy is
    /// deleted).
    RemoveFolder { root: String },
    /// Create an empty user folder for files copied from the UI machine, named
    /// after `name` (made unique; web: an OPFS folder, native: under the app data). Fill it
    /// with `ImportFile`, then `Rescan` it. Replies `BrowserRoots` (the new root is the
    /// `Folder` root that was not listed before).
    ImportFolder { name: String },
    /// Write a completed upload (`Media::BeginUpload`/`UploadChunk`) to `path`
    /// (relative; an audio or MIDI file name; parents created) inside an imported folder
    /// `root`, replacing an existing file. The upload is consumed either way.
    ImportFile {
        root: String,
        path: String,
        upload: String,
    },
    /// Rename a user folder (display only, persisted; files untouched). An empty
    /// name restores the folder's own name. Replies `BrowserRoots`.
    RenameFolder { root: String, name: String },
    /// Re-scan one root (`None` = all).
    Rescan { root: Option<String> },
    /// Preview an item: audio through the preview voice (`Media::Preview` semantics and
    /// events). `sync`: repitch to the project tempo using the item's detected bpm (no-op
    /// without one) and, while the transport plays, start on the next beat.
    Preview { item: String, sync: bool },
    /// Replies `BrowserFileFilter` (the stored filter, or the defaults).
    GetFileFilter,
    /// Store the file filter (every rule must compile, limits
    /// [`MAX_FILE_FILTER_RULES`] / [`MAX_FILE_FILTER_RULE_LEN`]; `InvalidArgument` naming
    /// the first bad rule otherwise). The index drops the files it now hides and rescans to
    /// pick up the ones it now shows. Emits `FileFilterChanged`; replies `BrowserFileFilter`.
    SetFileFilter { filter: FileFilter },
    /// Back to the defaults (removes the stored filter). Emits
    /// `FileFilterChanged`; replies `BrowserFileFilter`.
    ResetFileFilter,
    /// Evaluate `filter` (`None` = the current one) without storing it. Replies
    /// `BrowserFileFilterCheck`: the rules that don't compile, which of `paths` (relative
    /// file paths, e.g. a folder about to be imported) it shows, and how many of the files
    /// the library index has seen it would show.
    CheckFileFilter {
        filter: Option<FileFilter>,
        paths: Vec<String>,
    },
}

/// Most rules in each list of a [`FileFilter`].
pub const MAX_FILE_FILTER_RULES: usize = 64;
/// Longest rule (bytes).
pub const MAX_FILE_FILTER_RULE_LEN: usize = 256;

/// Which files the library browser shows. Rules are regular expressions (Rust
/// `regex` syntax) matched case-insensitively against a file's name (not its folder), unanchored
/// unless the rule anchors itself (`^`, `$`).
///
/// A file is shown when it matches a `whitelist` rule (an empty whitelist allows every
/// file) and no `blacklist` rule: the blacklist wins. Folders are hidden only by the
/// blacklist (so a whitelist of extensions never hides folders); a hidden folder hides
/// everything below it. Hidden files (names starting with `.`) are never listed, whatever
/// the rules.
///
/// Defaults: the whitelist allows what Ethereal can use from the browser (decodable audio,
/// MIDI, Ethereal presets and templates), the blacklist hides system and sidecar files
/// (`Thumbs.db`, `desktop.ini`, Ableton `.asd`, REAPER `.reapeaks`). DAW projects
/// (`.als`, `.flp`, `.rpp`, `.dawproject`) are hidden: they open with "Import project…".
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct FileFilter {
    pub whitelist: Vec<String>,
    pub blacklist: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum FileFilterList {
    Whitelist,
    Blacklist,
}

/// A rule that doesn't compile (or is too long).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct FileFilterRuleError {
    pub list: FileFilterList,
    /// Index in its list.
    pub index: u32,
    /// Human-readable, one line.
    pub message: String,
}

/// How many of the files the index has seen a filter shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct FileFilterCount {
    pub shown: u32,
    pub total: u32,
}

/// `CheckFileFilter`'s answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct FileFilterCheck {
    /// Empty when every rule compiles (then `visible` and `library` use the whole filter;
    /// otherwise bad rules are left out).
    pub errors: Vec<FileFilterRuleError>,
    /// One per `CheckFileFilter::paths`, in order.
    pub visible: Vec<bool>,
    /// Over the file names the last index scans saw (every kind, before filtering);
    /// `None` until the index has scanned (the library browser was never opened).
    pub library: Option<FileFilterCount>,
}

/// A paged query. Every filter is ANDed; an empty list means "no filter".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct BrowserQuery {
    /// Case-insensitive words matched against name, path, tags and pack (all must match).
    pub text: String,
    pub kinds: Vec<LibraryItemKind>,
    /// Items having every tag.
    pub tags: Vec<String>,
    pub favourites_only: bool,
    /// Restrict to these root ids.
    pub roots: Vec<String>,
    /// Restrict to a folder: items whose path starts with `folder` inside `roots[0]`.
    pub folder: Option<String>,
    /// Restrict presets to a device type.
    pub device: Option<PresetDevice>,
    pub sort: BrowserSort,
    /// Page start (0-based) and size (`1..=200`; larger is clamped).
    pub offset: u32,
    pub limit: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum BrowserSort {
    Name,
    /// Most recently modified first.
    Recent,
    Duration,
    Bpm,
    /// Best text match first (ties by name).
    Relevance,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub enum LibraryItemKind {
    Audio,
    Midi,
    Preset,
    Project,
}

/// One indexed item.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct LibraryItem {
    /// Stable id: `"<root id>/<relative path>"`.
    pub id: String,
    pub kind: LibraryItemKind,
    pub name: String,
    pub root: String,
    /// Path relative to the root.
    pub path: String,
    /// Import / preview source (audio, MIDI).
    pub source: Option<MediaSource>,
    /// Presets.
    pub preset: Option<PresetRef>,
    pub tags: Vec<String>,
    pub favourite: bool,
    pub meta: LibraryItemMeta,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct LibraryItemMeta {
    pub duration_seconds: Option<f64>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    /// Detected or file-embedded tempo.
    pub bpm: Option<f64>,
    /// Detected key, e.g. `"A minor"`.
    pub key: Option<String>,
    /// Sample pack name (a root's top-level folder with a `pack.json`, or the root name).
    pub pack: Option<String>,
    /// Modification time (ms since the Unix epoch).
    pub modified_ms: Option<f64>,
    pub size: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct BrowserPage {
    pub items: Vec<LibraryItem>,
    /// Total matches (for the pager).
    pub total: u32,
    pub offset: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct BrowserRoot {
    pub id: String,
    pub name: String,
    pub kind: BrowserRootKind,
    /// Display only (user folders).
    pub path: Option<String>,
    pub items: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum BrowserRootKind {
    /// The built-in user library.
    Library,
    /// A sample pack.
    Pack,
    /// A user-added folder.
    Folder,
    /// Factory presets.
    Factory,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum BrowserEvent {
    IndexProgress {
        root: String,
        scanned: u32,
        /// `None` while still counting.
        total: Option<u32>,
    },
    /// The index changed (scan finished, favourites/tags edited): re-query visible pages.
    IndexChanged,
    /// The file filter changed (`SetFileFilter`, `ResetFileFilter`, another
    /// window): re-list folders.
    FileFilterChanged { filter: FileFilter },
}

//! Browser v2 (v0.2, `browser-v2` node; CONTRACTS.md §12.8): an engine-side library index with
//! search, tags, favourites, sample packs and user folders, paged queries, and tempo-synced
//! preview. v0.1's `Media::{ListLocations, ListDirectory}` keep working (folder view).
//!
//! The index lives engine-side (controller `browser/` module), persisted in the user library
//! (`<library>/.ethereal/index.json`); the UI never touches files. User folders are added by
//! path (native only: the desktop shell picks the folder with a native dialog and sends the
//! path; web and remote reply `Unsupported`). Indexing runs in the background (bounded work
//! per tick) and reports `BrowserEvent::IndexProgress`.

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
    /// Add a user folder (absolute engine-side path; native only). Replies `BrowserRoots`.
    AddFolder { path: String },
    /// Remove a user folder from the index (files untouched).
    RemoveFolder { root: String },
    /// Re-scan one root (`None` = all).
    Rescan { root: Option<String> },
    /// Preview an item: audio through the preview voice (`Media::Preview` semantics and
    /// events). `sync`: repitch to the project tempo using the item's detected bpm (no-op
    /// without one) and, while the transport plays, start on the next beat.
    Preview { item: String, sync: bool },
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
}

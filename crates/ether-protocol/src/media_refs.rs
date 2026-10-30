//! External media references: missing media, relink and collect (v0.2
//! node; CONTRACTS.md §12.9). Model: `ether_model::MediaLocation`.
//!
//! Behaviour change in v0.2: `Media::Import { source: Location }` references the library file
//! in place (`MediaLocation::External`, content hash computed, nothing copied); uploads from a
//! remote UI, recordings, bounces and freezes still land in the project's `media/`
//! (`MediaLocation::Project`). Opening a project checks every external reference in the
//! background and reports `MediaEvent::Missing` for unresolved media (silence until
//! relinked).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::media::MediaSource;
use crate::model::MediaId;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum MediaRefCommand {
    /// Replies `MissingMedia` with the currently unresolved media.
    ListMissing,
    /// Search the library roots and user folders for missing media (`None` = all missing) by
    /// content hash, then by file name. Unambiguous hash matches are relinked automatically
    /// (one undo step); the rest are reported as `MediaRefEvent::Candidates`. Progress:
    /// `MediaRefEvent::SearchProgress` (the last one has `total: Some(scanned)`). Replies
    /// `Unit` when started.
    ///
    /// `folder` (v0.2 additive): also search this absolute engine-side
    /// folder, recursively (the Relink dialog's "Search in folder…", desktop only: hosts
    /// without OS files reply `Unsupported`). Missing = library roots only.
    Search {
        media: Option<MediaId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        folder: Option<String>,
    },
    /// Point `media` at another file (a library location; undoable). If its content hash
    /// differs, the hash is updated and a warning notification says so.
    Relink { media: MediaId, source: MediaSource },
    /// Copy every external reference into the project's `media/` (switching them to
    /// `MediaLocation::Project`, one undo step), then save. Progress:
    /// `MediaRefEvent::CollectProgress`, then `ProjectEvent::Saved`.
    CollectAll,
    /// Media manager: remove `media` from the project (one undo step). What uses them is
    /// unlinked first: their audio clips are deleted, samplers and convolution reverbs
    /// unloaded, multisampler zones emptied. The files stay on disk (undo puts the media
    /// back). A freeze render can't be removed (unfreeze its track): `InvalidArgument`.
    Remove { media: Vec<MediaId> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum MediaRefEvent {
    SearchProgress {
        scanned: u32,
        total: Option<u32>,
    },
    /// Possible files for a missing media (by name; hash differs or several matches).
    Candidates {
        media: MediaId,
        candidates: Vec<MediaSource>,
    },
    /// A missing media was found again (relink or file restored).
    Resolved {
        media: MediaId,
    },
    CollectProgress {
        done: u32,
        total: u32,
    },
}

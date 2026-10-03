//! `share.json` next to a project (docs/SHARING.md §4.5), through the store's file API (the
//! same code natively, on OPFS and in memory).
//!
//! The store has no "delete a file" call, so a project that is no longer shared (Stop,
//! Detach, a private copy) gets an **empty** `share.json`: readers (Recents) treat an empty
//! or unreadable file as "not shared". A copy whose sharing ended keeps its file with an
//! empty `key` (Recents shows "Sharing ended").

use ether_collab::share::file::{SHARE_FILE, ShareFile};
use ether_core::protocol::model::ProjectId;

use crate::store::ProjectStore;

/// The project's sharing state (`None`: not shared, or unreadable).
pub(crate) fn read_share<S: ProjectStore>(store: &mut S, pid: ProjectId) -> Option<ShareFile> {
    let bytes = store.read(pid, SHARE_FILE).ok()?;
    if bytes.is_empty() {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

pub(crate) fn write_share<S: ProjectStore>(
    store: &mut S,
    pid: ProjectId,
    file: &ShareFile,
) -> Result<(), String> {
    let json = serde_json::to_vec_pretty(file).map_err(|e| e.to_string())?;
    store
        .write(pid, SHARE_FILE, &json)
        .map_err(|e| format!("could not save the sharing state: {e}"))
}

/// Make `pid` unshared (an empty `share.json`), if it has a sharing state.
pub(crate) fn clear_share_file<S: ProjectStore>(store: &mut S, pid: ProjectId) {
    if store.read(pid, SHARE_FILE).is_ok_and(|b| !b.is_empty()) {
        let _ = store.write(pid, SHARE_FILE, b"");
    }
}

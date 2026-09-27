//! Importing audio from the user's computer (v0.2, owned by the `file-import` node, priority
//! 1; CONTRACTS.md §12.13).
//!
//! - Desktop: `Media::Import { source: MediaSource::Path { path } }` (Tauri file dialog or
//!   OS drop; the UI passes the path, never the bytes). [`read_path`] validates the path
//!   (absolute, regular readable file, audio extension via `store::file_kind`) and reads it
//!   with `Library::read_external`. Until `media-references` lands the import copies the file
//!   into the project (v0.1 behaviour); then it becomes an external reference in place.
//!   Placeholder: `Unsupported`.
//! - Web (local wasm controller) and remote: bytes go through the frozen upload staging
//!   (`Media::{BeginUpload, UploadChunk, CancelUpload}` → `MediaSource::Upload`, copied into
//!   the project). Implement `ProjectStore::{begin, append, read, discard}_upload` for the
//!   web OPFS store (`ether-wasm/src/store.rs`) so the local web build can import too.
//! - Collab: imported media replicate through the existing media push (`collab_push_media`);
//!   make sure an external-path media pushes its bytes (read with `read_external`).

use crate::store::Library;
use crate::tx::{CmdResult, unsupported};

/// Bytes and display name of an OS file of the engine machine (see the module docs).
pub(crate) fn read_path<L: Library>(library: &mut L, path: &str) -> CmdResult<(Vec<u8>, String)> {
    let _ = (library, path);
    Err(unsupported(
        "importing OS files by path is not implemented yet (file-import)",
    ))
}

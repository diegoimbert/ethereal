//! Importing audio from the user's computer (v0.2, owned by the `file-import` node, priority
//! 1; CONTRACTS.md §12.13).
//!
//! - Desktop: `Media::Import { source: MediaSource::Path { path } }` (Tauri file dialog or
//!   OS drop; the UI passes the path, never the bytes). [`read_path`] validates the path
//!   (absolute, audio extension via `store::file_kind`, no NUL) and reads it with
//!   `Library::read_external` (the host checks it is a regular readable file within
//!   [`MAX_IMPORT_BYTES`]). Until `media-references` lands the import copies the file into
//!   the project (v0.1 behaviour); then it becomes an external reference in place.
//! - Web (local wasm controller) and remote: bytes go through the frozen upload staging
//!   (`Media::{BeginUpload, UploadChunk, CancelUpload}` → `MediaSource::Upload`, copied into
//!   the project); the web OPFS store stages them (`ether-wasm/src/store.rs`). Hosts without
//!   OS paths (web) reply `Unsupported` to `Path` (the default `read_external`).
//! - Collab: imported media replicate through the existing media push (`collab_push_media`);
//!   an external-path media pushes its bytes, read with `read_external` ([`media_bytes`]).

use ether_core::protocol::media::FileKind;
use ether_core::protocol::model::{MediaLocation, MediaRef, ProjectId};

use crate::handlers::store_err;
use crate::store::{Library, ProjectStore, StoreError, file_kind};
use crate::tx::{CmdResult, invalid};
use crate::upload::MAX_UPLOAD_BYTES;

/// Largest file imported by path (same as an upload: 1 GiB).
pub const MAX_IMPORT_BYTES: u64 = MAX_UPLOAD_BYTES;

/// `true` for an absolute path on the engine machine: `/…` (Unix), `C:\…`/`C:/…` or
/// `\\server\…` (Windows).
fn is_absolute(path: &str) -> bool {
    let b = path.as_bytes();
    path.starts_with('/')
        || path.starts_with("\\\\")
        || (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'/' | b'\\'))
}

/// The file name of a path (either separator).
fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// Validate a path handed over by the UI for `MediaSource::Path` (see the module docs):
/// the file name to display and decode with.
pub(crate) fn check_path(path: &str) -> CmdResult<&str> {
    if path.is_empty() || path.contains('\0') {
        return Err(invalid("path must name a file"));
    }
    if !is_absolute(path) {
        return Err(invalid(format!("not an absolute path: {path}")));
    }
    let name = file_name(path);
    if name.is_empty() {
        return Err(invalid(format!("not a file: {path}")));
    }
    if file_kind(name) != FileKind::Audio {
        return Err(invalid(format!("not a supported audio file: {name}")));
    }
    Ok(name)
}

/// Bytes and display name of an OS file of the engine machine (see the module docs).
pub(crate) fn read_path<L: Library>(library: &mut L, path: &str) -> CmdResult<(Vec<u8>, String)> {
    let name = check_path(path)?.to_string();
    let bytes = library.read_external(path).map_err(store_err)?;
    if bytes.len() as u64 > MAX_IMPORT_BYTES {
        return Err(invalid(format!(
            "{name} is larger than {} MiB",
            MAX_IMPORT_BYTES >> 20
        )));
    }
    Ok((bytes, name))
}

/// The bytes of a media for the collab push: the project copy at `MediaRef::file`, else,
/// for an external reference, the referenced file (read with `read_external`).
pub(crate) fn media_bytes<S: ProjectStore, L: Library>(
    store: &mut S,
    library: &mut L,
    pid: ProjectId,
    m: &MediaRef,
) -> Result<Vec<u8>, StoreError> {
    let project = store.read(pid, &m.file);
    match (&project, &m.location) {
        (Err(_), MediaLocation::External { path }) => library.read_external(path),
        _ => project,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(check_path("/Users/me/kick.wav").unwrap(), "kick.wav");
        assert_eq!(check_path("C:\\Samples\\Snare.AIFF").unwrap(), "Snare.AIFF");
        assert_eq!(check_path("D:/x/loop.flac").unwrap(), "loop.flac");
        assert_eq!(check_path("\\\\nas\\s\\a.mp3").unwrap(), "a.mp3");
        for bad in [
            "",
            "kick.wav",
            "./kick.wav",
            "media/kick.wav",
            "/Users/me/notes.txt",
            "/Users/me/song.mid",
            "/Users/me/",
            "/Users/me/k\0.wav",
            "C:kick.wav",
        ] {
            assert!(check_path(bad).is_err(), "{bad:?}");
        }
    }
}

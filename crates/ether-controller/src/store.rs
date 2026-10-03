//! Engine-side persistence: the [`ProjectStore`] and the sample [`Library`].
//!
//! **Only the engine/controller side touches files.** The UI may be on another machine and
//! addresses everything by id (`ProjectId`, `MediaId`) or by location-relative path.
//!
//! Layout under `projects_root`:
//!
//! ```text
//! <projects_root>/<project-uuid>/project.ether   the document (display name inside)
//!                               /media/          imported audio (copied in; self-contained)
//!                               /cache/          peaks, decoded audio (regenerable)
//! ```
//!
//! `projects_root` comes from engine/host config: default `~/Documents/Ethereal/Projects`;
//! in dev builds `<data_dir>/ethereal-dev/<instance>/projects` (per-instance isolation).
//!
//! Implementations (owned by the host nodes):
//! - native (`ether-native`): folders on disk under `projects_root`;
//! - web (`ether-wasm`): OPFS, accessed from the engine's Worker (still engine-side).
//!
//! Paths passed to these traits are always *relative* (to a project folder or a library
//! root). Implementations must reject absolute paths and `..` components.

use ether_core::protocol::media::{BrowseRoot, DirectoryListing};
use ether_core::protocol::model::ProjectId;
use ether_core::protocol::project::ProjectSummary;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum StoreError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("already exists: {0}")]
    AlreadyExists(String),
    #[error("invalid path: {0}")]
    InvalidPath(String),
    #[error("io: {0}")]
    Io(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
}

/// Project folders keyed by `ProjectId`.
pub trait ProjectStore {
    /// All stored projects (name read from each `project.ether`), newest first.
    fn list(&mut self) -> Result<Vec<ProjectSummary>, StoreError>;

    /// Create an empty project folder (with `media/`, `cache/`). Fails if it exists.
    fn create(&mut self, id: ProjectId) -> Result<(), StoreError>;

    /// Read `project.ether`.
    fn load(&mut self, id: ProjectId) -> Result<String, StoreError>;

    /// Atomically replace `project.ether` (write temp + rename). Returns the new summary.
    fn save(&mut self, id: ProjectId, ether_json: &str) -> Result<ProjectSummary, StoreError>;

    /// Copy the whole folder (document + media; cache optional) to `to`. The caller then
    /// rewrites the copy's name via `load`/`save`.
    fn duplicate(&mut self, from: ProjectId, to: ProjectId) -> Result<(), StoreError>;

    /// Delete the project folder.
    fn delete(&mut self, id: ProjectId) -> Result<(), StoreError>;

    /// Read a file relative to the project folder (e.g. `media/<file>`).
    fn read(&mut self, id: ProjectId, rel_path: &str) -> Result<Vec<u8>, StoreError>;

    /// Write a file relative to the project folder (media import, caches).
    fn write(&mut self, id: ProjectId, rel_path: &str, bytes: &[u8]) -> Result<(), StoreError>;

    /// List a folder relative to the project folder (browse location `ProjectMedia`).
    fn list_dir(&mut self, id: ProjectId, rel_path: &str) -> Result<DirectoryListing, StoreError>;

    // --- Roadmap v2 (contracts-2), defaulted so existing stores keep compiling ------------

    /// `export`: store a finished export as `<project>/exports/<file_name>` (see
    /// [`export_path`]) and return that project-relative path. `Err(Unsupported)` (the
    /// default) means this host doesn't keep exports on disk: the controller offers the
    /// bytes as a download (`ExportResult::Download`) instead (web, remote).
    fn write_export(
        &mut self,
        id: ProjectId,
        file_name: &str,
        bytes: &[u8],
    ) -> Result<String, StoreError> {
        let _ = (id, file_name, bytes);
        Err(StoreError::Unsupported(
            "exports are delivered as downloads".into(),
        ))
    }

    /// `remote-engine`: start staging an upload (`Media::BeginUpload`) outside any project
    /// (native: `<projects_root>/.uploads/<upload>`), `size` bytes expected. Replaces a
    /// previous upload with the same id.
    fn begin_upload(&mut self, upload: &str, size: u64) -> Result<(), StoreError> {
        let _ = (upload, size);
        Err(StoreError::Unsupported("uploads".into()))
    }

    /// Append bytes at `offset` (must equal the bytes received so far). Returns the new total.
    fn append_upload(
        &mut self,
        upload: &str,
        offset: u64,
        bytes: &[u8],
    ) -> Result<u64, StoreError> {
        let _ = (upload, offset, bytes);
        Err(StoreError::Unsupported("uploads".into()))
    }

    /// The complete staged bytes (read by `Media::Import { source: Upload }`).
    fn read_upload(&mut self, upload: &str) -> Result<Vec<u8>, StoreError> {
        let _ = upload;
        Err(StoreError::Unsupported("uploads".into()))
    }

    /// Drop a staged upload (cancel, after import, on disconnect). Missing = `Ok`.
    fn discard_upload(&mut self, upload: &str) -> Result<(), StoreError> {
        let _ = upload;
        Ok(())
    }

    // --- v0.3 (contracts-4), defaulted ---

    /// `project-versions`: delete a file relative to the project folder (old versions, the
    /// session marker). Missing = `Ok`. Default: unsupported (the node implements it in the
    /// native, OPFS and memory stores).
    fn remove(&mut self, id: ProjectId, rel_path: &str) -> Result<(), StoreError> {
        let _ = (id, rel_path);
        Err(StoreError::Unsupported("removing project files".into()))
    }
}

/// Project-relative path of an export file (`exports/<file_name>`): `file_name` must be a
/// single, non-hidden path segment.
pub fn export_path(file_name: &str) -> Result<String, StoreError> {
    if file_name.is_empty() || file_name.starts_with('.') || file_name.contains(['/', '\\', '\0']) {
        return Err(StoreError::InvalidPath(file_name.to_string()));
    }
    Ok(format!(
        "{}/{file_name}",
        ether_core::protocol::model::file::EXPORTS_DIR
    ))
}

/// Engine-visible sample library folders (configured on the engine side, never by path
/// from the UI).
pub trait Library {
    /// Library roots (`BrowseLocation::Library { id }`).
    fn roots(&self) -> Vec<BrowseRoot>;

    fn list_dir(&mut self, root: &str, rel_path: &str) -> Result<DirectoryListing, StoreError>;

    fn read(&mut self, root: &str, rel_path: &str) -> Result<Vec<u8>, StoreError>;

    // --- v0.2 (contracts-3), defaulted ---

    /// Write a file in a writable root (the user library: presets, the browser index).
    /// Creates parent folders. Default: unsupported (read-only library).
    fn write_file(&mut self, root: &str, rel_path: &str, bytes: &[u8]) -> Result<(), StoreError> {
        let _ = (root, rel_path, bytes);
        Err(StoreError::Unsupported(
            "the library is read-only on this host".into(),
        ))
    }

    /// Delete a file (user presets). Default: unsupported.
    fn remove_file(&mut self, root: &str, rel_path: &str) -> Result<(), StoreError> {
        let _ = (root, rel_path);
        Err(StoreError::Unsupported(
            "the library is read-only on this host".into(),
        ))
    }

    /// Rename/move a file within a root (user presets). Default: unsupported.
    fn rename_file(&mut self, root: &str, from: &str, to: &str) -> Result<(), StoreError> {
        let _ = (root, from, to);
        Err(StoreError::Unsupported(
            "the library is read-only on this host".into(),
        ))
    }

    /// The writable user-library root id (presets, index), if any. Default: none.
    fn user_root(&self) -> Option<String> {
        None
    }

    /// `browser-v2`: add a user folder by absolute engine-side path; returns its root id.
    /// Native only. Default: unsupported.
    fn add_folder(&mut self, path: &str) -> Result<String, StoreError> {
        let _ = path;
        Err(StoreError::Unsupported(
            "user folders are not available on this host".into(),
        ))
    }

    /// `browser-v2`: forget a user folder. Default: unsupported.
    fn remove_folder(&mut self, root: &str) -> Result<(), StoreError> {
        let _ = root;
        Err(StoreError::Unsupported(
            "user folders are not available on this host".into(),
        ))
    }

    /// `media-references`: the absolute engine-side path of a library file, for an external
    /// reference (`MediaLocation::External`). `None` = cannot be referenced in place (web,
    /// remote): the import copies it into the project instead.
    fn external_path(&self, root: &str, rel_path: &str) -> Option<String> {
        let _ = (root, rel_path);
        None
    }

    /// `media-references`: the entries of an absolute engine-side folder (the Relink
    /// dialog's folder search): `(absolute path, is_dir)` per entry, hidden entries left out.
    /// Default: unsupported (hosts without OS files).
    fn list_external_dir(&mut self, path: &str) -> Result<Vec<(String, bool)>, StoreError> {
        let _ = path;
        Err(StoreError::Unsupported(
            "external folders are not available on this host".into(),
        ))
    }

    /// `media-references`: read an external reference by its absolute path. Default:
    /// unsupported.
    fn read_external(&mut self, path: &str) -> Result<Vec<u8>, StoreError> {
        let _ = path;
        Err(StoreError::Unsupported(
            "external media is not available on this host".into(),
        ))
    }
}

/// Validate a relative path from the UI or a document: no absolute paths, drive letters,
/// backslashes, `.`/`..` or empty components. `""` (a location root) is accepted.
pub fn check_relative_path(path: &str) -> Result<(), StoreError> {
    if path.is_empty() {
        return Ok(());
    }
    let bad = path.starts_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path.contains('\0')
        || path
            .split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..");
    if bad {
        return Err(StoreError::InvalidPath(path.to_string()));
    }
    Ok(())
}

/// Classify a file by extension (for directory listings).
pub fn file_kind(name: &str) -> ether_core::protocol::media::FileKind {
    use ether_core::protocol::media::FileKind;
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "wav" | "wave" | "aif" | "aiff" | "aifc" | "flac" | "mp3" | "ogg" | "oga" => {
            FileKind::Audio
        }
        "mid" | "midi" => FileKind::Midi,
        _ => FileKind::Other,
    }
}

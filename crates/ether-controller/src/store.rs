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
}

/// Engine-visible sample library folders (configured on the engine side, never by path
/// from the UI).
pub trait Library {
    /// Library roots (`BrowseLocation::Library { id }`).
    fn roots(&self) -> Vec<BrowseRoot>;

    fn list_dir(&mut self, root: &str, rel_path: &str) -> Result<DirectoryListing, StoreError>;

    fn read(&mut self, root: &str, rel_path: &str) -> Result<Vec<u8>, StoreError>;
}

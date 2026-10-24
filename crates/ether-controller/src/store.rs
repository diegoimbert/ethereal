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
//! The native store names the folder `<title> - <short id>` instead (readable in a file
//! manager; still found by id, see [`ProjectStore::settle_folder`]).
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

use ether_core::protocol::ext::ExtValue;
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

/// Builds the JSON of a document for [`ProjectStore::save_in_background`] (runs on the
/// store's writer thread when it has one).
pub type RenderDocument = Box<dyn FnOnce() -> Result<String, StoreError> + Send>;

/// A file written right after the document by [`ProjectStore::save_in_background_with`],
/// built from the document's JSON: `(relative path, bytes)`, or nothing.
pub type RenderSidecar = Box<dyn FnOnce(&str) -> Option<(String, Vec<u8>)> + Send>;

/// `export`: an export file being written piece by piece
/// ([`ProjectStore::create_export`]). It is seekable because encoders fill their header in
/// at the end. Dropped without [`ProjectStore::commit_export`] (cancel, failure), it leaves
/// nothing behind.
pub trait ExportFile: std::io::Write + std::io::Seek + Send {
    /// Make the finished file durable under its final name.
    fn commit(self: Box<Self>) -> Result<(), StoreError>;
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
    /// Waits for [`Self::save_in_background`] writes first, so the newest document wins.
    fn save(&mut self, id: ProjectId, ether_json: &str) -> Result<ProjectSummary, StoreError>;

    /// A routine autosave that may finish in the background: `render` builds the document
    /// and the store writes it like [`Self::save`], on another thread where the host can,
    /// so the controller isn't blocked serializing a big project and waiting for the fsync.
    /// Writes stay in order and coalesce (a queued write that hasn't started is replaced by
    /// a newer one), and every other access to `project.ether` (load, save, list,
    /// duplicate, delete, ...) waits for them. The summary is built without reading the
    /// file back (`name` is the document's name). A failure is reported later, by
    /// [`Self::take_save_error`]. Default: synchronous.
    fn save_in_background(
        &mut self,
        id: ProjectId,
        name: &str,
        render: RenderDocument,
    ) -> Result<ProjectSummary, StoreError> {
        let _ = name;
        let json = render()?;
        self.save(id, &json)
    }

    /// [`Self::save_in_background`], then `sidecar`'s file (from the document's JSON) is
    /// written right after the document, in the same job: a newer save replaces both, and
    /// the sidecar never lands before its document (an extension's file that names the
    /// document it goes with). Default: synchronous.
    fn save_in_background_with(
        &mut self,
        id: ProjectId,
        name: &str,
        render: RenderDocument,
        sidecar: RenderSidecar,
    ) -> Result<ProjectSummary, StoreError> {
        let _ = name;
        let json = render()?;
        let summary = self.save(id, &json)?;
        if let Some((path, bytes)) = sidecar(&json) {
            self.write(id, &path, &bytes)?;
        }
        Ok(summary)
    }

    /// Wait until every [`Self::save_in_background`] write is on disk (or failed).
    fn flush_saves(&mut self) {}

    /// The oldest [`Self::save_in_background`] failure not reported yet.
    fn take_save_error(&mut self) -> Option<(ProjectId, StoreError)> {
        None
    }

    /// Copy the whole folder (document + media; cache optional) to `to`, **except
    /// an extension's [`private_files`]** (a copy is a private project). The caller then rewrites the copy's
    /// name via `load`/`save`.
    fn duplicate(&mut self, from: ProjectId, to: ProjectId) -> Result<(), StoreError>;

    /// Delete the project folder.
    fn delete(&mut self, id: ProjectId) -> Result<(), StoreError>;

    /// Nothing has files open in project `id`'s folder right now (it was just created,
    /// copied or closed, or is about to be opened): a store whose folder names follow the
    /// project's title may rename it now (native: `<title> - <short id>`). Best effort,
    /// silent. Default: nothing (folders named by id).
    fn settle_folder(&mut self, id: ProjectId) {
        let _ = id;
    }

    /// Read a file relative to the project folder (e.g. `media/<file>`).
    fn read(&mut self, id: ProjectId, rel_path: &str) -> Result<Vec<u8>, StoreError>;

    /// Write a file relative to the project folder (media import, caches).
    fn write(&mut self, id: ProjectId, rel_path: &str, bytes: &[u8]) -> Result<(), StoreError>;

    /// List a folder relative to the project folder (browse location `ProjectMedia`).
    fn list_dir(&mut self, id: ProjectId, rel_path: &str) -> Result<DirectoryListing, StoreError>;

    // --- Roadmap v2, defaulted so existing stores keep compiling ------------

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

    /// `export`: start writing `exports/<file_name>` piece by piece, so an export
    /// streams to disk instead of being held in memory; [`Self::commit_export`] then gives
    /// it its name. `Err(Unsupported)` (the default) means the controller encodes the file
    /// in memory and hands it whole to [`Self::write_export`].
    fn create_export(
        &mut self,
        id: ProjectId,
        file_name: &str,
    ) -> Result<Box<dyn ExportFile>, StoreError> {
        let _ = (id, file_name);
        Err(StoreError::Unsupported("streamed exports".into()))
    }

    /// Finish a file from [`Self::create_export`]; returns its project-relative path (see
    /// [`export_path`]). Default: [`ExportFile::commit`].
    fn commit_export(
        &mut self,
        id: ProjectId,
        file_name: &str,
        file: Box<dyn ExportFile>,
    ) -> Result<String, StoreError> {
        let _ = id;
        let path = export_path(file_name)?;
        file.commit()?;
        Ok(path)
    }

    /// `export` into a folder the user picked (`ExportRequest::folder`, an absolute
    /// engine-machine path; desktop): the names of the files already in it (lowercase is
    /// not required), so an export never overwrites one. A missing folder has none.
    /// `Err(Unsupported)` (the default): this host doesn't write exports outside projects.
    fn export_folder_names(&mut self, folder: &str) -> Result<Vec<String>, StoreError> {
        let _ = folder;
        Err(StoreError::Unsupported(
            "exporting to a folder is not available on this host".into(),
        ))
    }

    /// Start writing `<folder>/<file_name>` (see [`Self::export_folder_names`]; the folder
    /// is created when missing); [`ExportFile::commit`] gives it its name. Returns the file
    /// and its absolute path. Default: `Unsupported`.
    fn create_export_in(
        &mut self,
        folder: &str,
        file_name: &str,
    ) -> Result<(Box<dyn ExportFile>, String), StoreError> {
        let _ = (folder, file_name);
        Err(StoreError::Unsupported(
            "exporting to a folder is not available on this host".into(),
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

    // --- v0.3, defaulted ---

    /// Delete a file relative to the project folder (old versions, the
    /// session marker). Missing = `Ok`. Default: unsupported (the node implements it in the
    /// native, OPFS and memory stores).
    fn remove(&mut self, id: ProjectId, rel_path: &str) -> Result<(), StoreError> {
        let _ = (id, rel_path);
        Err(StoreError::Unsupported("removing project files".into()))
    }

    // --- project bundles, defaulted ---

    /// Write a project bundle to an absolute engine-machine path chosen in the desktop's OS
    /// save dialog (`Project::ExportBundle { path }`). Implementations require a `.ether`
    /// extension. Default: unsupported (hosts without OS files deliver a download instead).
    fn write_bundle_file(&mut self, path: &str, bytes: &[u8]) -> Result<(), StoreError> {
        let _ = (path, bytes);
        Err(StoreError::Unsupported(
            "bundles are delivered as downloads on this host".into(),
        ))
    }

    /// Read a project bundle from an absolute engine-machine path chosen in the desktop's
    /// OS open dialog (`BundleSource::Path`). Default: unsupported (upload it instead).
    fn read_bundle_file(&mut self, path: &str) -> Result<Vec<u8>, StoreError> {
        let _ = path;
        Err(StoreError::Unsupported(
            "bundles are uploaded on this host".into(),
        ))
    }

    /// Write a Standard MIDI File (`Media::WriteMidiFile { path }`) to an absolute
    /// engine-machine path chosen in the desktop's OS save dialog. Implementations require a
    /// MIDI extension ([`MIDI_EXTENSIONS`]). Default: unsupported (the bytes go back in the
    /// reply instead).
    fn write_midi_file(&mut self, path: &str, bytes: &[u8]) -> Result<(), StoreError> {
        let _ = (path, bytes);
        Err(StoreError::Unsupported(
            "MIDI files are returned as bytes on this host".into(),
        ))
    }

    // --- DAWproject import, defaulted ---

    /// Read a project file of another DAW from an absolute engine-machine path chosen in the
    /// desktop's OS open dialog or dropped on the window (`Project::Import { source: Path }`).
    /// Implementations require an import extension ([`IMPORT_EXTENSIONS`]). Default:
    /// unsupported (upload it instead).
    fn read_import_file(&mut self, path: &str) -> Result<Vec<u8>, StoreError> {
        let _ = path;
        Err(StoreError::Unsupported(
            "project files are uploaded on this host".into(),
        ))
    }

    // --- project export, defaulted ---

    /// Write a project exported for another DAW (`Project::ExportTo { path }`): `project` at
    /// the absolute engine-machine path chosen in the desktop's OS save dialog, and each of
    /// `files` (relative paths with `/`, e.g. `Samples/Imported/kick.wav`, checked with
    /// [`check_relative_path`]) relative to that file's folder. Implementations require
    /// one of [`PROJECT_EXPORT_EXTENSIONS`]. Default: unsupported (a download instead).
    fn write_project_export(
        &mut self,
        path: &str,
        project: &[u8],
        files: &[(String, Vec<u8>)],
    ) -> Result<(), StoreError> {
        let _ = (path, project, files);
        Err(StoreError::Unsupported(
            "exports are delivered as downloads on this host".into(),
        ))
    }

    // --- the OS file manager (`Media::Reveal`), defaulted ---

    /// The engine-machine path of the folder holding the projects. Default: none (the
    /// projects aren't OS files).
    fn projects_os_path(&self) -> Option<String> {
        None
    }

    /// The engine-machine path of project `id`'s folder (`rel_path` empty) or of a file in
    /// it (`rel_path` relative, checked). Hosts resolve the folder themselves (never
    /// `projects_os_path` joined with the id). Default: none.
    fn project_os_path(&self, id: ProjectId, rel_path: &str) -> Option<String> {
        let _ = (id, rel_path);
        None
    }
}

/// File extensions `Project::ExportTo` writes to a path (lowercase, no dot).
pub const PROJECT_EXPORT_EXTENSIONS: [&str; 3] = ["dawproject", "als", "flp"];

/// File extensions `Project::Import` reads from a path (lowercase, no dot).
pub const IMPORT_EXTENSIONS: [&str; 5] = ["dawproject", "als", "flp", "rpp", "rpp-bak"];

/// `true` if `path` ends with one of [`IMPORT_EXTENSIONS`] (any case).
pub fn is_import_file(path: &str) -> bool {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    name.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty()
            && IMPORT_EXTENSIONS
                .iter()
                .any(|e| e.eq_ignore_ascii_case(ext))
    })
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

    // --- v0.2, defaulted ---

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

    /// Add a user folder by absolute engine-side path; returns its root id.
    /// Native only. Default: unsupported.
    fn add_folder(&mut self, path: &str) -> Result<String, StoreError> {
        let _ = path;
        Err(StoreError::Unsupported(
            "user folders are not available on this host".into(),
        ))
    }

    /// Forget a user folder. Default: unsupported.
    fn remove_folder(&mut self, root: &str) -> Result<(), StoreError> {
        let _ = root;
        Err(StoreError::Unsupported(
            "user folders are not available on this host".into(),
        ))
    }

    /// Create an empty, engine-owned folder for a folder copied from the UI
    /// machine (`Browser::ImportFolder`), named after `name` ([`import_folder_name`], made
    /// unique among the imported folders). Returns its engine-side path, which
    /// [`Library::add_folder`] accepts (the controller then treats it as a user folder and
    /// remembers it). Once added, the folder's root is writable with
    /// [`Library::write_file`], and [`Library::remove_folder`] deletes it. Default:
    /// unsupported.
    fn create_import_folder(&mut self, name: &str) -> Result<String, StoreError> {
        let _ = name;
        Err(StoreError::Unsupported(
            "importing folders is not available on this host".into(),
        ))
    }

    /// The absolute engine-side path of a library file, for an external
    /// reference (`MediaLocation::External`). `None` = cannot be referenced in place (web,
    /// remote): the import copies it into the project instead.
    fn external_path(&self, root: &str, rel_path: &str) -> Option<String> {
        let _ = (root, rel_path);
        None
    }

    /// The entries of an absolute engine-side folder (the Relink
    /// dialog's folder search): `(absolute path, is_dir)` per entry, hidden entries left out.
    /// Default: unsupported (hosts without OS files).
    fn list_external_dir(&mut self, path: &str) -> Result<Vec<(String, bool)>, StoreError> {
        let _ = path;
        Err(StoreError::Unsupported(
            "external folders are not available on this host".into(),
        ))
    }

    /// Read an external reference by its absolute path. Default:
    /// unsupported.
    fn read_external(&mut self, path: &str) -> Result<Vec<u8>, StoreError> {
        let _ = path;
        Err(StoreError::Unsupported(
            "external media is not available on this host".into(),
        ))
    }

    // --- the sample pack cache (docs/SAMPLES.md) ---

    /// Read a file of the sample pack cache: `rel` (checked by the caller, `/`-separated) under
    /// the host's samples folder (natively `<app data>/samples`, OPFS `samples/` on the web).
    /// Default: unsupported (no cache: the built-in kit only).
    fn sample_cache_read(&mut self, rel: &str) -> Result<Vec<u8>, StoreError> {
        let _ = rel;
        Err(StoreError::Unsupported(
            "the sample cache is not available on this host".into(),
        ))
    }

    /// Write a cache file, creating its folders; replaces it atomically where the host can.
    fn sample_cache_write(&mut self, rel: &str, bytes: &[u8]) -> Result<(), StoreError> {
        let _ = (rel, bytes);
        Err(StoreError::Unsupported(
            "the sample cache is not available on this host".into(),
        ))
    }

    /// Delete a cache file (a missing one is not an error).
    fn sample_cache_remove(&mut self, rel: &str) -> Result<(), StoreError> {
        let _ = rel;
        Err(StoreError::Unsupported(
            "the sample cache is not available on this host".into(),
        ))
    }

    /// The file names in a cache folder (empty when it doesn't exist).
    fn sample_cache_list(&mut self, dir: &str) -> Result<Vec<String>, StoreError> {
        let _ = dir;
        Err(StoreError::Unsupported(
            "the sample cache is not available on this host".into(),
        ))
    }
}

/// What an extension keeps in a project folder (docs/EXTENSIONS.md), set once at start with
/// [`set_store_ext`]. Every store honours it.
#[derive(Clone, Copy)]
pub struct StoreExt {
    /// Files `duplicate` never copies (so `SaveAs` / `Duplicate` make a private project).
    pub private_files: &'static [&'static str],
    /// Files a save writes right after the document ([`RenderSidecar`]): reading or replacing
    /// one waits for a queued save, like the document.
    pub sidecar_files: &'static [&'static str],
    /// The project list's [`ProjectSummary::ext`], from the project folder's files (`read`: a
    /// file's bytes, `None` when it is absent or unreadable).
    pub summary: fn(read: ReadProjectFile) -> Option<ExtValue>,
}

/// Reads a file of a project folder for [`StoreExt::summary`] (`None`: absent or unreadable).
pub type ReadProjectFile<'a> = &'a mut dyn FnMut(&str) -> Option<Vec<u8>>;

static STORE_EXT: std::sync::OnceLock<StoreExt> = std::sync::OnceLock::new();

/// Extension point: what an extension keeps in a project folder (set once, at start).
pub fn set_store_ext(ext: StoreExt) {
    let _ = STORE_EXT.set(ext);
}

/// The extension's files `duplicate` never copies (relative to the project folder).
pub fn private_files() -> &'static [&'static str] {
    STORE_EXT.get().map_or(&[], |e| e.private_files)
}

/// `rel` (relative to the project folder) is written with the document by a save.
pub fn is_sidecar_file(rel: &str) -> bool {
    STORE_EXT
        .get()
        .is_some_and(|e| e.sidecar_files.contains(&rel))
}

/// [`ProjectSummary::ext`] of a project folder (`read`: one of its files).
pub fn summary_ext(read: ReadProjectFile) -> Option<ExtValue> {
    (STORE_EXT.get()?.summary)(read)
}

/// A test [`StoreExt`]: `ext-private.json` is private and its JSON is the summary's `ext`;
/// `ext-sidecar.json` is a save's sidecar.
#[doc(hidden)]
pub mod test_store_ext {
    /// The private file (never copied by `duplicate`; its JSON is the summary's `ext`).
    pub const PRIVATE: &str = "ext-private.json";
    /// The sidecar file (written after the document).
    pub const SIDECAR: &str = "ext-sidecar.json";

    /// Set the test [`super::StoreExt`] (idempotent; every test of a binary sets the same one).
    pub fn install() {
        super::set_store_ext(super::StoreExt {
            private_files: &[PRIVATE],
            sidecar_files: &[SIDECAR],
            summary: |read| {
                let bytes = read(PRIVATE)?;
                serde_json::from_slice(&bytes).ok().map(super::ExtValue)
            },
        });
    }
}

/// The folder name of an imported folder: `name` without path separators,
/// control characters, `:` and leading dots, trimmed and at most 64 characters; `None` when
/// nothing is left.
pub fn import_folder_name(name: &str) -> Option<String> {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c == ':' || c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_start_matches('.').trim();
    let out: String = trimmed.chars().take(64).collect();
    let out = out.trim_end().to_string();
    (!out.is_empty()).then_some(out)
}

/// `base`, or `base 2`, `base 3`… the first one `taken` refuses
/// (case-insensitively: OPFS and most desktop file systems).
pub fn unique_folder_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    (1..)
        .map(|n| {
            if n == 1 {
                base.to_string()
            } else {
                format!("{base} {n}")
            }
        })
        .find(|c| !taken(c))
        .expect("unbounded")
}

/// Stable root id of a user folder: `folder-` + the FNV-1a hash of its engine-side path
/// (native absolute paths and web OPFS paths alike).
pub fn user_folder_id(path: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in path.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{USER_FOLDER_PREFIX}{h:016x}")
}

/// Id prefix of user folders ([`user_folder_id`]).
pub const USER_FOLDER_PREFIX: &str = "folder-";

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

/// Audio extensions the engine decodes (the symphonia features of the workspace), lowercase.
pub const AUDIO_EXTENSIONS: &[&str] = &[
    "wav", "wave", "aif", "aiff", "aifc", "flac", "mp3", "ogg", "oga", "m4a", "aac",
];
/// MIDI file extensions, lowercase.
pub const MIDI_EXTENSIONS: &[&str] = &["mid", "midi"];

/// Classify a file by extension (for directory listings).
pub fn file_kind(name: &str) -> ether_core::protocol::media::FileKind {
    use ether_core::protocol::media::FileKind;
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    if AUDIO_EXTENSIONS.contains(&ext.as_str()) {
        FileKind::Audio
    } else if MIDI_EXTENSIONS.contains(&ext.as_str()) {
        FileKind::Midi
    } else {
        FileKind::Other
    }
}

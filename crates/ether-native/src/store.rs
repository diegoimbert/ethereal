//! [`DiskStore`]: the native [`ProjectStore`] and [`Library`] (folders on disk).
//!
//! Layout (see `ether_controller::store`):
//!
//! ```text
//! <projects_root>/<project-uuid>/project.ether
//!                               /media/
//!                               /cache/
//! ```
//!
//! - `projects_root` is per instance in dev builds (`default_projects_root`), so parallel
//!   dev instances never share projects.
//! - Every write of `project.ether` (and of any other file) is atomic: the bytes go to a
//!   temp file in the same folder, which is fsynced and renamed over the target.
//! - Relative paths are sandboxed: absolute paths, drive prefixes, `..` components,
//!   backslashes and NUL bytes are rejected, and existing targets are canonicalized and
//!   must stay inside their root (so a symlink can't escape it either).
//! - `list_dir` paths: for projects, `rel_path` and the returned entry paths are relative to
//!   the project folder (e.g. `media/kick.wav`); for the library they are relative to the
//!   library root.

use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use ether_controller::store::{
    Library, ProjectStore, StoreError, file_kind, import_folder_name, unique_folder_name,
};
pub use ether_controller::store::{USER_FOLDER_PREFIX, user_folder_id};
use ether_core::protocol::media::{
    BrowseLocation, BrowseRoot, DirectoryEntry, DirectoryListing, FileKind,
};
use ether_core::protocol::model::ProjectId;
use ether_core::protocol::project::ProjectSummary;

/// File name of the document inside a project folder.
pub const PROJECT_FILE: &str = "project.ether";
/// Sub-folders created with every project.
pub const PROJECT_SUBDIRS: &[&str] = &["media", "cache"];

/// A configured sample library folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryRoot {
    /// Stable id used by `BrowseLocation::Library { id }`.
    pub id: String,
    /// Display name.
    pub name: String,
    pub path: PathBuf,
}

/// Id of the writable user library root (`Library::user_root`; presets, the browser index).
pub const USER_LIBRARY_ID: &str = "user";
/// Folder name of the default user library, next to the projects root
/// (`~/Documents/Ethereal/User Library`, or `<instance data>/User Library` in dev builds).
pub const USER_LIBRARY_DIR: &str = "User Library";
/// `base-136`: folder of the folders imported from a remote UI (`Library::create_import_folder`),
/// next to the user library (`~/Documents/Ethereal/Imported Folders`).
pub const IMPORTED_FOLDERS_DIR: &str = "Imported Folders";

/// Disk-backed project store and sample library. Cheap to clone (paths only).
#[derive(Clone, Debug)]
pub struct DiskStore {
    pub projects_root: PathBuf,
    pub library_roots: Vec<LibraryRoot>,
    /// v0.2 (`presets`): the writable user library (`Library::user_root`, id
    /// [`USER_LIBRARY_ID`]). Created on first write. Not listed in `roots()` (the sample
    /// browser shows the configured folders); presets live under its `Presets/` folder.
    pub user_library: Option<PathBuf>,
}

fn io_err(e: std::io::Error) -> StoreError {
    StoreError::Io(e.to_string())
}

/// Validate a relative path and turn it into a `PathBuf` of normal components.
/// `""` and `"."` mean the root itself. Same rules as the shared
/// [`ether_controller::store::check_relative_path`], except that `.` segments are tolerated
/// (normalized away); [`resolve_in`] additionally canonicalizes against symlink escapes.
pub fn sanitize_rel(rel: &str) -> Result<PathBuf, StoreError> {
    let bad = || StoreError::InvalidPath(rel.to_string());
    if rel.contains('\0') || rel.contains('\\') {
        return Err(bad());
    }
    // Windows drive letters (`C:foo`) are prefixes on Windows only; reject them everywhere.
    if rel.len() >= 2 && rel.as_bytes()[1] == b':' {
        return Err(bad());
    }
    let mut out = PathBuf::new();
    for c in Path::new(rel).components() {
        match c {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return Err(bad()),
        }
    }
    Ok(out)
}

/// Join `rel` under `root`, rejecting anything that could leave it. The nearest existing
/// ancestor of the target (the target itself if it exists) is canonicalized and must stay
/// inside the canonical root, so a symlinked folder or file can't escape it, including
/// when creating a new file under a symlinked directory. Dangling symlinks are rejected.
fn resolve_in(root: &Path, rel: &str) -> Result<PathBuf, StoreError> {
    let path = root.join(sanitize_rel(rel)?);
    let invalid = || StoreError::InvalidPath(rel.to_string());
    let canon_root = root.canonicalize().map_err(io_err)?;
    let mut probe = path.as_path();
    loop {
        if fs::symlink_metadata(probe).is_ok() {
            let canon = probe.canonicalize().map_err(|_| invalid())?;
            if !canon.starts_with(&canon_root) {
                return Err(invalid());
            }
            return Ok(path);
        }
        probe = probe.parent().ok_or_else(invalid)?;
    }
}

/// Write `bytes` to `path` atomically (temp file in the same folder + fsync + rename).
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let dir = path
        .parent()
        .ok_or_else(|| StoreError::InvalidPath(path.display().to_string()))?;
    fs::create_dir_all(dir).map_err(io_err)?;
    let name = path
        .file_name()
        .ok_or_else(|| StoreError::InvalidPath(path.display().to_string()))?
        .to_string_lossy();
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let result = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, path)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(io_err(e));
    }
    // Persist the rename itself (directory entry). Best effort: not supported everywhere.
    #[cfg(unix)]
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

fn modified_ms(path: &Path) -> f64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0.0, |d| d.as_millis() as f64)
}

/// Display name stored in a `.ether` file (`project.settings.name`), read leniently so it
/// works across format versions.
fn name_from_ether(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    v.get("project")?
        .get("settings")?
        .get("name")?
        .as_str()
        .map(str::to_string)
}

/// List `dir`; entry paths are `rel` + name. Hidden files are skipped; folders first, then
/// by name (case-insensitive).
fn list_folder(
    dir: &Path,
    rel: &Path,
    location: BrowseLocation,
) -> Result<DirectoryListing, StoreError> {
    let rd = fs::read_dir(dir).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => StoreError::NotFound(rel.display().to_string()),
        _ => io_err(e),
    })?;
    let mut entries = Vec::new();
    for entry in rd {
        let entry = entry.map_err(io_err)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        // `metadata` follows symlinks; broken links are skipped.
        let Ok(meta) = fs::metadata(entry.path()) else {
            continue;
        };
        let path = rel_string(&rel.join(&name));
        let (kind, size) = if meta.is_dir() {
            (FileKind::Directory, 0.0)
        } else {
            (file_kind(&name), meta.len() as f64)
        };
        entries.push(DirectoryEntry {
            name,
            path,
            kind,
            size,
        });
    }
    entries.sort_by(|a, b| {
        let da = a.kind != FileKind::Directory;
        let db = b.kind != FileKind::Directory;
        da.cmp(&db)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(DirectoryListing {
        location,
        path: rel_string(rel),
        entries,
    })
}

/// `/`-separated relative path string.
fn rel_string(p: &Path) -> String {
    p.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn copy_dir(from: &Path, to: &Path, skip_top: &[&str]) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if skip_top.iter().any(|s| name == **s) {
            continue;
        }
        let ty = entry.file_type()?;
        let dst = to.join(&name);
        if ty.is_dir() {
            copy_dir(&entry.path(), &dst, &[])?;
        } else if ty.is_file() {
            fs::copy(entry.path(), dst)?;
        }
        // Symlinks are not copied (projects are self-contained).
    }
    Ok(())
}

impl DiskStore {
    /// The user library defaults to [`USER_LIBRARY_DIR`] next to `projects_root`; see
    /// [`Self::with_user_library`].
    pub fn new(projects_root: impl Into<PathBuf>, library_roots: Vec<LibraryRoot>) -> Self {
        let projects_root: PathBuf = projects_root.into();
        let user_library = projects_root.parent().map(|p| p.join(USER_LIBRARY_DIR));
        Self {
            projects_root,
            library_roots,
            user_library,
        }
    }

    /// Use `path` as the writable user library (`None`: read-only library).
    pub fn with_user_library(mut self, path: Option<PathBuf>) -> Self {
        self.user_library = path;
        self
    }

    /// The user library folder, created if missing (so `resolve_in` can canonicalize it).
    fn user_dir(&self, root: &str) -> Result<PathBuf, StoreError> {
        match &self.user_library {
            Some(dir) if root == USER_LIBRARY_ID => {
                fs::create_dir_all(dir).map_err(io_err)?;
                Ok(dir.clone())
            }
            _ => Err(StoreError::Unsupported(format!(
                "library {root} is read-only"
            ))),
        }
    }

    /// `base-136`: the folder holding imported folders (next to the user library; none
    /// without a user library).
    pub fn imports_dir(&self) -> Option<PathBuf> {
        Some(
            self.user_library
                .as_ref()?
                .parent()?
                .join(IMPORTED_FOLDERS_DIR),
        )
    }

    /// `base-136`: the folder of root `root` if it is an imported folder.
    fn imported_dir(&self, root: &str) -> Option<PathBuf> {
        let imports = self.imports_dir()?;
        let r = self.library_roots.iter().find(|r| r.id == root)?;
        (r.path.parent() == Some(imports.as_path())).then(|| r.path.clone())
    }

    /// A writable root's folder: the user library or an imported folder.
    fn writable_dir(&self, root: &str) -> Result<PathBuf, StoreError> {
        match self.imported_dir(root) {
            Some(dir) => {
                fs::create_dir_all(&dir).map_err(io_err)?;
                Ok(dir)
            }
            None => self.user_dir(root),
        }
    }

    /// Folder of a library root id (configured roots, then the user library).
    fn root_dir(&self, root: &str) -> Result<PathBuf, StoreError> {
        if root == USER_LIBRARY_ID && self.user_library.is_some() {
            return self.user_dir(root);
        }
        self.library_root(root).map(|r| r.path.clone())
    }

    /// Folder of a project (the id is a UUID, so it is always a safe single component).
    pub fn project_dir(&self, id: ProjectId) -> PathBuf {
        self.projects_root.join(id.to_string())
    }

    fn existing_project_dir(&self, id: ProjectId) -> Result<PathBuf, StoreError> {
        let dir = self.project_dir(id);
        if dir.is_dir() {
            Ok(dir)
        } else {
            Err(StoreError::NotFound(id.to_string()))
        }
    }

    fn summary(&self, id: ProjectId) -> Result<ProjectSummary, StoreError> {
        let file = self.project_dir(id).join(PROJECT_FILE);
        let json = fs::read_to_string(&file).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => StoreError::NotFound(id.to_string()),
            _ => io_err(e),
        })?;
        Ok(ProjectSummary {
            id,
            name: name_from_ether(&json).unwrap_or_else(|| "Untitled".to_string()),
            modified_ms: modified_ms(&file),
            // base-115: `recents-shared` reads the project's `share.json`.
            share: None,
        })
    }

    fn library_root(&self, root: &str) -> Result<&LibraryRoot, StoreError> {
        self.library_roots
            .iter()
            .find(|r| r.id == root)
            .ok_or_else(|| StoreError::NotFound(format!("library {root}")))
    }
}

impl ProjectStore for DiskStore {
    fn list(&mut self) -> Result<Vec<ProjectSummary>, StoreError> {
        let rd = match fs::read_dir(&self.projects_root) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io_err(e)),
        };
        let mut out = Vec::new();
        for entry in rd.flatten() {
            let Ok(id) = entry.file_name().to_string_lossy().parse::<ProjectId>() else {
                continue;
            };
            if let Ok(summary) = self.summary(id) {
                out.push(summary);
            }
        }
        out.sort_by(|a, b| b.modified_ms.total_cmp(&a.modified_ms));
        Ok(out)
    }

    fn create(&mut self, id: ProjectId) -> Result<(), StoreError> {
        let dir = self.project_dir(id);
        if dir.exists() {
            return Err(StoreError::AlreadyExists(id.to_string()));
        }
        fs::create_dir_all(&self.projects_root).map_err(io_err)?;
        fs::create_dir(&dir).map_err(|e| match e.kind() {
            std::io::ErrorKind::AlreadyExists => StoreError::AlreadyExists(id.to_string()),
            _ => io_err(e),
        })?;
        for sub in PROJECT_SUBDIRS {
            fs::create_dir_all(dir.join(sub)).map_err(io_err)?;
        }
        Ok(())
    }

    fn load(&mut self, id: ProjectId) -> Result<String, StoreError> {
        let dir = self.existing_project_dir(id)?;
        fs::read_to_string(dir.join(PROJECT_FILE)).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => StoreError::NotFound(format!("{id}/{PROJECT_FILE}")),
            _ => io_err(e),
        })
    }

    fn save(&mut self, id: ProjectId, ether_json: &str) -> Result<ProjectSummary, StoreError> {
        let dir = self.existing_project_dir(id)?;
        atomic_write(&dir.join(PROJECT_FILE), ether_json.as_bytes())?;
        self.summary(id)
    }

    fn duplicate(&mut self, from: ProjectId, to: ProjectId) -> Result<(), StoreError> {
        let src = self.existing_project_dir(from)?;
        let dst = self.project_dir(to);
        if dst.exists() {
            return Err(StoreError::AlreadyExists(to.to_string()));
        }
        // Copy into a temp folder first, then rename: a failed copy leaves no half project.
        let tmp = self
            .projects_root
            .join(format!(".dup-{to}-{}.tmp", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let result = copy_dir(&src, &tmp, &["cache"])
            .and_then(|()| fs::create_dir_all(tmp.join("cache")))
            .and_then(|()| fs::rename(&tmp, &dst));
        if let Err(e) = result {
            let _ = fs::remove_dir_all(&tmp);
            return Err(io_err(e));
        }
        Ok(())
    }

    fn delete(&mut self, id: ProjectId) -> Result<(), StoreError> {
        let dir = self.existing_project_dir(id)?;
        fs::remove_dir_all(dir).map_err(io_err)
    }

    fn read(&mut self, id: ProjectId, rel_path: &str) -> Result<Vec<u8>, StoreError> {
        let dir = self.existing_project_dir(id)?;
        let path = resolve_in(&dir, rel_path)?;
        fs::read(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => StoreError::NotFound(rel_path.to_string()),
            _ => io_err(e),
        })
    }

    fn write(&mut self, id: ProjectId, rel_path: &str, bytes: &[u8]) -> Result<(), StoreError> {
        let dir = self.existing_project_dir(id)?;
        if sanitize_rel(rel_path)?.as_os_str().is_empty() {
            return Err(StoreError::InvalidPath(rel_path.to_string()));
        }
        let path = resolve_in(&dir, rel_path)?;
        atomic_write(&path, bytes)
    }

    fn write_export(
        &mut self,
        id: ProjectId,
        file_name: &str,
        bytes: &[u8],
    ) -> Result<String, StoreError> {
        let path = ether_controller::store::export_path(file_name)?;
        self.write(id, &path, bytes)?;
        Ok(path)
    }

    // Upload staging lives in `crate::uploads` (owned by the remote-engine node).
    fn begin_upload(&mut self, upload: &str, size: u64) -> Result<(), StoreError> {
        crate::uploads::begin(&self.projects_root, upload, size)
    }

    fn append_upload(
        &mut self,
        upload: &str,
        offset: u64,
        bytes: &[u8],
    ) -> Result<u64, StoreError> {
        crate::uploads::append(&self.projects_root, upload, offset, bytes)
    }

    fn read_upload(&mut self, upload: &str) -> Result<Vec<u8>, StoreError> {
        crate::uploads::read(&self.projects_root, upload)
    }

    fn discard_upload(&mut self, upload: &str) -> Result<(), StoreError> {
        crate::uploads::discard(&self.projects_root, upload)
    }

    fn write_bundle_file(&mut self, path: &str, bytes: &[u8]) -> Result<(), StoreError> {
        let p = bundle_path(path)?;
        atomic_write(p, bytes)
    }

    fn read_bundle_file(&mut self, path: &str) -> Result<Vec<u8>, StoreError> {
        let p = bundle_path(path)?;
        let meta = fs::metadata(p).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => StoreError::NotFound(path.to_string()),
            _ => io_err(e),
        })?;
        if !meta.is_file() {
            return Err(StoreError::InvalidPath(format!("not a file: {path}")));
        }
        if meta.len() > u32::MAX as u64 {
            return Err(StoreError::Io(format!("{path} is larger than 4 GiB")));
        }
        fs::read(p).map_err(io_err)
    }

    /// v0.3 (`project-versions`): delete one file (never a folder); missing = `Ok`.
    fn remove(&mut self, id: ProjectId, rel_path: &str) -> Result<(), StoreError> {
        let dir = self.existing_project_dir(id)?;
        if sanitize_rel(rel_path)?.as_os_str().is_empty() {
            return Err(StoreError::InvalidPath(rel_path.to_string()));
        }
        let path = resolve_in(&dir, rel_path)?;
        match fs::symlink_metadata(&path) {
            Ok(m) if m.is_dir() => Err(StoreError::InvalidPath(format!("{rel_path} is a folder"))),
            Ok(_) => fs::remove_file(&path).map_err(io_err),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io_err(e)),
        }
    }

    fn list_dir(&mut self, id: ProjectId, rel_path: &str) -> Result<DirectoryListing, StoreError> {
        let dir = self.existing_project_dir(id)?;
        let rel = sanitize_rel(rel_path)?;
        let path = resolve_in(&dir, rel_path)?;
        list_folder(&path, &rel, BrowseLocation::ProjectMedia)
    }
}

impl Library for DiskStore {
    fn roots(&self) -> Vec<BrowseRoot> {
        self.library_roots
            .iter()
            .map(|r| BrowseRoot {
                location: BrowseLocation::Library { id: r.id.clone() },
                name: r.name.clone(),
            })
            .collect()
    }

    fn list_dir(&mut self, root: &str, rel_path: &str) -> Result<DirectoryListing, StoreError> {
        let dir = self.root_dir(root)?;
        let rel = sanitize_rel(rel_path)?;
        let path = resolve_in(&dir, rel_path)?;
        list_folder(
            &path,
            &rel,
            BrowseLocation::Library {
                id: root.to_string(),
            },
        )
    }

    fn read(&mut self, root: &str, rel_path: &str) -> Result<Vec<u8>, StoreError> {
        let dir = self.root_dir(root)?;
        let path = resolve_in(&dir, rel_path)?;
        fs::read(&path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => StoreError::NotFound(rel_path.to_string()),
            _ => io_err(e),
        })
    }

    /// v0.2 (`presets`): only the user library (and, `base-136`, imported folders) is
    /// writable (atomic write, parents created).
    fn write_file(&mut self, root: &str, rel_path: &str, bytes: &[u8]) -> Result<(), StoreError> {
        let dir = self.writable_dir(root)?;
        if sanitize_rel(rel_path)?.as_os_str().is_empty() {
            return Err(StoreError::InvalidPath(rel_path.to_string()));
        }
        atomic_write(&resolve_in(&dir, rel_path)?, bytes)
    }

    /// v0.2 (`presets`): delete a file of the user library (never a folder).
    fn remove_file(&mut self, root: &str, rel_path: &str) -> Result<(), StoreError> {
        let dir = self.user_dir(root)?;
        let path = resolve_in(&dir, rel_path)?;
        match fs::symlink_metadata(&path) {
            Ok(m) if m.is_file() => fs::remove_file(&path).map_err(io_err),
            Ok(_) => Err(StoreError::InvalidPath(format!("not a file: {rel_path}"))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(StoreError::NotFound(rel_path.to_string()))
            }
            Err(e) => Err(io_err(e)),
        }
    }

    /// v0.2 (`presets`): rename/move a file inside the user library. The target must not
    /// exist (`AlreadyExists`); parents are created.
    fn rename_file(&mut self, root: &str, from: &str, to: &str) -> Result<(), StoreError> {
        let dir = self.user_dir(root)?;
        let src = resolve_in(&dir, from)?;
        if sanitize_rel(to)?.as_os_str().is_empty() {
            return Err(StoreError::InvalidPath(to.to_string()));
        }
        let dst = resolve_in(&dir, to)?;
        if !fs::symlink_metadata(&src).is_ok_and(|m| m.is_file()) {
            return Err(StoreError::NotFound(from.to_string()));
        }
        // Case-only renames on case-insensitive file systems see the target as existing.
        let same_file =
            src.to_string_lossy().to_lowercase() == dst.to_string_lossy().to_lowercase();
        if dst.exists() && !same_file {
            return Err(StoreError::AlreadyExists(to.to_string()));
        }
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(io_err)?;
        }
        fs::rename(&src, &dst).map_err(io_err)
    }

    fn user_root(&self) -> Option<String> {
        self.user_library
            .as_ref()
            .map(|_| USER_LIBRARY_ID.to_string())
    }

    /// `file-import` (shared with `media-references`): an OS file chosen by the user (file
    /// dialog or drop) or an external reference. Absolute, an audio file by extension, a
    /// regular file (symlinks followed) of at most [`MAX_EXTERNAL_BYTES`].
    fn read_external(&mut self, path: &str) -> Result<Vec<u8>, StoreError> {
        read_external_file(path)
    }

    /// `media-references`: library files are referenced in place, by their absolute path
    /// (the root joined with the checked relative path; symlinks are not resolved, so the
    /// reference keeps the path the user sees).
    fn external_path(&self, root: &str, rel_path: &str) -> Option<String> {
        let dir = self.root_dir(root).ok()?;
        let path = resolve_in(&dir, rel_path).ok()?;
        if !path.is_absolute() {
            return None;
        }
        path.to_str().map(str::to_string)
    }

    /// `media-references`: the Relink dialog's folder search.
    fn list_external_dir(&mut self, path: &str) -> Result<Vec<(String, bool)>, StoreError> {
        list_external_dir(path)
    }

    /// `browser-v2`: a user folder becomes a library root (listed by `roots()`, browsable,
    /// importable, referenced in place) with the stable id [`user_folder_id`]. The folder
    /// must be an existing absolute directory. Not persisted here: the controller's browser
    /// index remembers user folders and re-adds them on start.
    fn add_folder(&mut self, path: &str) -> Result<String, StoreError> {
        let p = Path::new(path);
        if path.contains('\0') || !p.is_absolute() {
            return Err(StoreError::InvalidPath(path.to_string()));
        }
        let meta = fs::metadata(p).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => StoreError::NotFound(path.to_string()),
            _ => io_err(e),
        })?;
        if !meta.is_dir() {
            return Err(StoreError::InvalidPath(format!("not a folder: {path}")));
        }
        let id = user_folder_id(path);
        if !self.library_roots.iter().any(|r| r.id == id) {
            let name = p
                .file_name()
                .map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned());
            self.library_roots.push(LibraryRoot {
                id: id.clone(),
                name,
                path: p.to_path_buf(),
            });
        }
        Ok(id)
    }

    /// `browser-v2`: forget a user folder (files untouched; `base-136`: an imported
    /// folder's copy is deleted). Configured roots can't be removed.
    fn remove_folder(&mut self, root: &str) -> Result<(), StoreError> {
        if !root.starts_with(USER_FOLDER_PREFIX) {
            return Err(StoreError::InvalidPath(format!(
                "{root} is not a user folder"
            )));
        }
        let imported = self.imported_dir(root);
        let before = self.library_roots.len();
        self.library_roots.retain(|r| r.id != root);
        if self.library_roots.len() == before {
            return Err(StoreError::NotFound(root.to_string()));
        }
        if let Some(dir) = imported {
            match fs::remove_dir_all(&dir) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(io_err(e)),
                _ => {}
            }
        }
        Ok(())
    }

    /// `base-136`: `<imports_dir>/<name>` (de-duplicated against existing folders), created.
    fn create_import_folder(&mut self, name: &str) -> Result<String, StoreError> {
        let base = import_folder_name(name)
            .ok_or_else(|| StoreError::InvalidPath(format!("bad folder name {name:?}")))?;
        let imports = self
            .imports_dir()
            .ok_or_else(|| StoreError::Unsupported("this host has no writable library".into()))?;
        fs::create_dir_all(&imports).map_err(io_err)?;
        let existing: Vec<String> = fs::read_dir(&imports)
            .map_err(io_err)?
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .map(|n| n.to_lowercase())
            .collect();
        let name = unique_folder_name(&base, |c| existing.contains(&c.to_lowercase()));
        let dir = imports.join(&name);
        fs::create_dir(&dir).map_err(io_err)?;
        dir.to_str()
            .map(str::to_string)
            .ok_or(StoreError::InvalidPath(name))
    }
}

/// See `Library::list_external_dir` for `DiskStore`: an absolute folder's entries
/// (absolute path, is a folder), hidden ones left out, sorted by name. Symlinked entries are
/// followed for the kind; unreadable entries are skipped.
pub fn list_external_dir(path: &str) -> Result<Vec<(String, bool)>, StoreError> {
    let p = Path::new(path);
    if path.contains('\0') || !p.is_absolute() {
        return Err(StoreError::InvalidPath(path.to_string()));
    }
    let meta = fs::metadata(p).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => StoreError::NotFound(path.to_string()),
        _ => io_err(e),
    })?;
    if !meta.is_dir() {
        return Err(StoreError::InvalidPath(format!("not a folder: {path}")));
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(p).map_err(io_err)?.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with('.') {
            continue;
        }
        let child = entry.path();
        let Ok(meta) = fs::metadata(&child) else {
            continue;
        };
        if let Some(child) = child.to_str() {
            out.push((child.to_string(), meta.is_dir()));
        }
    }
    out.sort();
    Ok(out)
}

/// Largest external file read (same as the upload limit: 1 GiB).
pub const MAX_EXTERNAL_BYTES: u64 = 1 << 30;

/// See `Library::read_external` for `DiskStore`.
/// base-114: an absolute `.ether` path from the desktop's OS save/open dialog.
fn bundle_path(path: &str) -> Result<&Path, StoreError> {
    let p = Path::new(path);
    let ether = p
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("ether"));
    if path.contains('\0') || !p.is_absolute() || !ether {
        return Err(StoreError::InvalidPath(format!(
            "not an absolute .ether path: {path}"
        )));
    }
    Ok(p)
}

pub fn read_external_file(path: &str) -> Result<Vec<u8>, StoreError> {
    let p = Path::new(path);
    if path.contains('\0') || !p.is_absolute() {
        return Err(StoreError::InvalidPath(path.to_string()));
    }
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    if file_kind(name) != FileKind::Audio {
        return Err(StoreError::InvalidPath(format!(
            "not an audio file: {path}"
        )));
    }
    let not_found = |e: std::io::Error| match e.kind() {
        std::io::ErrorKind::NotFound => StoreError::NotFound(path.to_string()),
        _ => io_err(e),
    };
    let meta = fs::metadata(p).map_err(not_found)?;
    if !meta.is_file() {
        return Err(StoreError::InvalidPath(format!("not a file: {path}")));
    }
    if meta.len() > MAX_EXTERNAL_BYTES {
        return Err(StoreError::Io(format!(
            "{name} is larger than {} MiB",
            MAX_EXTERNAL_BYTES >> 20
        )));
    }
    fs::read(p).map_err(not_found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempDir;

    fn pid(n: u8) -> ProjectId {
        ProjectId::v7(1_700_000_000_000 + n as u64, [n; 10])
    }

    fn ether(name: &str) -> String {
        serde_json::json!({
            "format": "ethereal-project",
            "version": 1,
            "app_version": "test",
            "project": { "settings": { "name": name } }
        })
        .to_string()
    }

    fn store(tmp: &TempDir) -> DiskStore {
        let lib = tmp.path().join("lib");
        fs::create_dir_all(lib.join("drums")).unwrap();
        fs::write(lib.join("drums/kick.wav"), b"RIFF").unwrap();
        fs::write(lib.join("loop.flac"), b"fLaC1234").unwrap();
        fs::write(lib.join("notes.txt"), b"x").unwrap();
        fs::write(lib.join(".DS_Store"), b"x").unwrap();
        DiskStore::new(
            tmp.path().join("projects"),
            vec![LibraryRoot {
                id: "lib".into(),
                name: "Library".into(),
                path: lib,
            }],
        )
    }

    #[test]
    fn user_library_write_rename_remove() {
        let tmp = TempDir::new("store-user-lib");
        let mut s = store(&tmp);
        assert_eq!(s.user_root().as_deref(), Some(USER_LIBRARY_ID));
        // Not a browse root; the configured roots are read-only.
        assert_eq!(s.roots().len(), 1);
        assert!(matches!(
            s.write_file("lib", "x.etherpreset", b"x"),
            Err(StoreError::Unsupported(_))
        ));
        // Missing folders are created; atomic write leaves no temp file.
        s.write_file("user", "Presets/synth/A.etherpreset", b"a")
            .unwrap();
        let dir = tmp.path().join(USER_LIBRARY_DIR).join("Presets/synth");
        assert_eq!(fs::read(dir.join("A.etherpreset")).unwrap(), b"a");
        assert_eq!(
            Library::read(&mut s, "user", "Presets/synth/A.etherpreset").unwrap(),
            b"a"
        );
        let listing = Library::list_dir(&mut s, "user", "Presets/synth").unwrap();
        assert_eq!(listing.entries.len(), 1);
        assert_eq!(listing.entries[0].path, "Presets/synth/A.etherpreset");
        // Rename: target must not exist; case-only renames work.
        s.write_file("user", "Presets/synth/B.etherpreset", b"b")
            .unwrap();
        assert!(matches!(
            s.rename_file(
                "user",
                "Presets/synth/A.etherpreset",
                "Presets/synth/B.etherpreset"
            ),
            Err(StoreError::AlreadyExists(_))
        ));
        s.rename_file(
            "user",
            "Presets/synth/A.etherpreset",
            "Presets/synth/a.etherpreset",
        )
        .unwrap();
        s.rename_file(
            "user",
            "Presets/synth/a.etherpreset",
            "Presets/other/C.etherpreset",
        )
        .unwrap();
        assert_eq!(
            Library::read(&mut s, "user", "Presets/other/C.etherpreset").unwrap(),
            b"a"
        );
        assert!(matches!(
            s.rename_file("user", "Presets/synth/nope", "Presets/x"),
            Err(StoreError::NotFound(_))
        ));
        // Remove files only; escapes are rejected.
        assert!(matches!(
            s.remove_file("user", "Presets"),
            Err(StoreError::InvalidPath(_))
        ));
        s.remove_file("user", "Presets/other/C.etherpreset")
            .unwrap();
        assert!(matches!(
            s.remove_file("user", "Presets/other/C.etherpreset"),
            Err(StoreError::NotFound(_))
        ));
        for bad in ["../x", "/etc/x", ""] {
            assert!(s.write_file("user", bad, b"x").is_err(), "{bad}");
        }
        // Read-only without a user library.
        let mut ro = store(&tmp).with_user_library(None);
        assert_eq!(ro.user_root(), None);
        assert!(ro.write_file("user", "a", b"x").is_err());
    }

    #[test]
    fn sanitize_rejects_escapes() {
        for bad in [
            "..",
            "../x",
            "media/../../x",
            "/etc/passwd",
            "C:/x",
            "c:x",
            "a\\..\\b",
            "a\0b",
        ] {
            assert!(
                matches!(sanitize_rel(bad), Err(StoreError::InvalidPath(_))),
                "{bad}"
            );
        }
        assert_eq!(sanitize_rel("").unwrap(), PathBuf::new());
        assert_eq!(
            sanitize_rel("./media/a.wav").unwrap(),
            PathBuf::from("media/a.wav")
        );
        assert_eq!(
            sanitize_rel("media//a.wav").unwrap(),
            PathBuf::from("media/a.wav")
        );
    }

    #[test]
    fn create_layout_save_load_list() {
        let tmp = TempDir::new("store-layout");
        let mut s = store(&tmp);
        assert!(ProjectStore::list(&mut s).unwrap().is_empty());

        let a = pid(1);
        s.create(a).unwrap();
        let dir = s.project_dir(a);
        assert!(dir.join("media").is_dir() && dir.join("cache").is_dir());
        assert!(matches!(s.create(a), Err(StoreError::AlreadyExists(_))));
        assert!(matches!(s.load(a), Err(StoreError::NotFound(_))));

        let summary = s.save(a, &ether("First")).unwrap();
        assert_eq!(summary.name, "First");
        assert_eq!(summary.id, a);
        assert!(summary.modified_ms > 0.0);
        assert_eq!(s.load(a).unwrap(), ether("First"));
        // No temp files left behind.
        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().all(|n| !n.ends_with(".tmp")), "{names:?}");

        let b = pid(2);
        s.create(b).unwrap();
        s.save(b, &ether("Second")).unwrap();
        // Make `a` the newest.
        std::thread::sleep(std::time::Duration::from_millis(20));
        s.save(a, &ether("First v2")).unwrap();
        // Junk in projects_root is ignored.
        fs::create_dir_all(s.projects_root.join("not-a-uuid")).unwrap();
        fs::write(s.projects_root.join("stray.txt"), b"x").unwrap();
        let list = ProjectStore::list(&mut s).unwrap();
        assert_eq!(
            list.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            ["First v2", "Second"]
        );
    }

    #[test]
    fn save_is_atomic_replace() {
        let tmp = TempDir::new("store-atomic");
        let mut s = store(&tmp);
        let a = pid(3);
        s.create(a).unwrap();
        s.save(a, &ether("x".repeat(10_000).as_str())).unwrap();
        s.save(a, &ether("short")).unwrap();
        assert_eq!(s.load(a).unwrap(), ether("short"));
        assert!(matches!(s.save(pid(9), "{}"), Err(StoreError::NotFound(_))));
    }

    #[test]
    fn media_read_write_list_sandboxed() {
        let tmp = TempDir::new("store-media");
        let mut s = store(&tmp);
        let a = pid(4);
        s.create(a).unwrap();
        s.write(a, "media/kick.wav", b"RIFFdata").unwrap();
        s.write(a, "cache/peaks/kick.peaks", b"pk").unwrap();
        assert_eq!(
            ProjectStore::read(&mut s, a, "media/kick.wav").unwrap(),
            b"RIFFdata"
        );
        assert_eq!(
            ProjectStore::read(&mut s, a, "./media/kick.wav").unwrap(),
            b"RIFFdata"
        );

        let listing = ProjectStore::list_dir(&mut s, a, "media").unwrap();
        assert_eq!(listing.location, BrowseLocation::ProjectMedia);
        assert_eq!(listing.path, "media");
        assert_eq!(listing.entries.len(), 1);
        assert_eq!(listing.entries[0].path, "media/kick.wav");
        assert_eq!(listing.entries[0].kind, FileKind::Audio);
        assert_eq!(listing.entries[0].size, 8.0);

        for bad in ["../x", "/etc/passwd", "media/../../x"] {
            assert!(
                matches!(
                    ProjectStore::read(&mut s, a, bad),
                    Err(StoreError::InvalidPath(_))
                ),
                "{bad}"
            );
            assert!(
                matches!(s.write(a, bad, b"x"), Err(StoreError::InvalidPath(_))),
                "{bad}"
            );
            assert!(
                matches!(
                    ProjectStore::list_dir(&mut s, a, bad),
                    Err(StoreError::InvalidPath(_))
                ),
                "{bad}"
            );
        }
        assert!(matches!(
            s.write(a, "", b"x"),
            Err(StoreError::InvalidPath(_))
        ));
        assert!(matches!(
            ProjectStore::read(&mut s, a, "media/nope.wav"),
            Err(StoreError::NotFound(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_rejected() {
        let tmp = TempDir::new("store-symlink");
        let mut s = store(&tmp);
        let a = pid(5);
        s.create(a).unwrap();
        let outside = tmp.path().join("secret.txt");
        fs::write(&outside, b"secret").unwrap();
        std::os::unix::fs::symlink(&outside, s.project_dir(a).join("media/link.wav")).unwrap();
        assert!(matches!(
            ProjectStore::read(&mut s, a, "media/link.wav"),
            Err(StoreError::InvalidPath(_))
        ));
        // Writing through the symlinked file is rejected too.
        assert!(matches!(
            s.write(a, "media/link.wav", b"x"),
            Err(StoreError::InvalidPath(_))
        ));
        assert_eq!(fs::read(&outside).unwrap(), b"secret");
    }

    #[cfg(unix)]
    #[test]
    fn new_file_under_symlinked_dir_rejected() {
        let tmp = TempDir::new("store-symlink-dir");
        let mut s = store(&tmp);
        let a = pid(12);
        s.create(a).unwrap();
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        let dir = s.project_dir(a);
        fs::remove_dir(dir.join("media")).unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("media")).unwrap();
        for rel in ["media/new.wav", "media/sub/deeper/new.wav"] {
            assert!(
                matches!(s.write(a, rel, b"x"), Err(StoreError::InvalidPath(_))),
                "{rel}"
            );
        }
        assert!(fs::read_dir(&outside).unwrap().next().is_none());
        assert!(matches!(
            ProjectStore::list_dir(&mut s, a, "media"),
            Err(StoreError::InvalidPath(_))
        ));
        // Dangling symlink as target.
        std::os::unix::fs::symlink(tmp.path().join("nowhere"), dir.join("cache/dangling")).unwrap();
        assert!(matches!(
            s.write(a, "cache/dangling", b"x"),
            Err(StoreError::InvalidPath(_))
        ));
        // A symlink that stays inside the project is fine.
        std::os::unix::fs::symlink(dir.join("cache"), dir.join("alias")).unwrap();
        s.write(a, "alias/ok.bin", b"ok").unwrap();
        assert_eq!(fs::read(dir.join("cache/ok.bin")).unwrap(), b"ok");
    }

    #[test]
    fn bundle_files_at_absolute_ether_paths_only() {
        let tmp = TempDir::new("store-bundle");
        let mut s = store(&tmp);
        let path = tmp.path().join("Song.ether");
        let path = path.to_str().unwrap();
        s.write_bundle_file(path, b"PK").unwrap();
        assert_eq!(s.read_bundle_file(path).unwrap(), b"PK");
        for bad in ["Song.ether", "/tmp/song.wav", "/tmp/a\0.ether"] {
            assert!(matches!(
                s.write_bundle_file(bad, b"x"),
                Err(StoreError::InvalidPath(_))
            ));
        }
        let missing = tmp.path().join("missing.ether");
        assert!(matches!(
            s.read_bundle_file(missing.to_str().unwrap()),
            Err(StoreError::NotFound(_))
        ));
    }

    #[test]
    fn remove_deletes_files_only() {
        let tmp = TempDir::new("store-remove");
        let mut s = store(&tmp);
        let a = pid(8);
        s.create(a).unwrap();
        s.write(a, "versions/.session", b"{}").unwrap();
        s.write(a, "versions/1-manual.ether", b"{}").unwrap();
        s.remove(a, "versions/.session").unwrap();
        assert!(!s.project_dir(a).join("versions/.session").exists());
        s.remove(a, "versions/.session").unwrap();
        assert!(matches!(
            s.remove(a, "versions"),
            Err(StoreError::InvalidPath(_))
        ));
        assert!(matches!(s.remove(a, ""), Err(StoreError::InvalidPath(_))));
        assert!(matches!(
            s.remove(a, "../x"),
            Err(StoreError::InvalidPath(_))
        ));
        assert!(s.project_dir(a).join("versions/1-manual.ether").exists());
    }

    #[test]
    fn duplicate_and_delete() {
        let tmp = TempDir::new("store-dup");
        let mut s = store(&tmp);
        let a = pid(6);
        let b = pid(7);
        s.create(a).unwrap();
        s.save(a, &ether("Orig")).unwrap();
        s.write(a, "media/a.wav", b"A").unwrap();
        s.write(a, "cache/a.peaks", b"P").unwrap();
        s.duplicate(a, b).unwrap();
        assert_eq!(s.load(b).unwrap(), ether("Orig"));
        assert_eq!(ProjectStore::read(&mut s, b, "media/a.wav").unwrap(), b"A");
        assert!(s.project_dir(b).join("cache").is_dir());
        assert!(matches!(
            ProjectStore::read(&mut s, b, "cache/a.peaks"),
            Err(StoreError::NotFound(_))
        ));
        assert!(matches!(
            s.duplicate(a, b),
            Err(StoreError::AlreadyExists(_))
        ));
        assert!(matches!(
            s.duplicate(pid(8), pid(9)),
            Err(StoreError::NotFound(_))
        ));

        s.delete(a).unwrap();
        assert!(!s.project_dir(a).exists());
        assert!(matches!(s.delete(a), Err(StoreError::NotFound(_))));
        assert_eq!(ProjectStore::list(&mut s).unwrap().len(), 1);
    }

    #[test]
    fn library_roots_list_read() {
        let tmp = TempDir::new("store-lib");
        let mut s = store(&tmp);
        let roots = s.roots();
        assert_eq!(roots.len(), 1);
        assert_eq!(
            roots[0].location,
            BrowseLocation::Library { id: "lib".into() }
        );

        let top = Library::list_dir(&mut s, "lib", "").unwrap();
        let names: Vec<_> = top.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["drums", "loop.flac", "notes.txt"]);
        assert_eq!(top.entries[0].kind, FileKind::Directory);
        assert_eq!(top.entries[1].kind, FileKind::Audio);
        assert_eq!(top.entries[2].kind, FileKind::Other);

        let drums = Library::list_dir(&mut s, "lib", "drums").unwrap();
        assert_eq!(drums.entries[0].path, "drums/kick.wav");
        assert_eq!(
            Library::read(&mut s, "lib", "drums/kick.wav").unwrap(),
            b"RIFF"
        );
        assert!(matches!(
            Library::read(&mut s, "lib", "../projects"),
            Err(StoreError::InvalidPath(_))
        ));
        assert!(matches!(
            Library::list_dir(&mut s, "nope", ""),
            Err(StoreError::NotFound(_))
        ));
    }
    #[test]
    fn user_folders_add_list_remove() {
        let tmp = TempDir::new("store-folders");
        let mut s = store(&tmp);
        let dir = tmp.path().join("My Samples");
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("sub/hit.wav"), b"RIFF").unwrap();
        let path = dir.to_str().unwrap();
        let id = s.add_folder(path).unwrap();
        assert_eq!(id, user_folder_id(path));
        assert_eq!(s.add_folder(path).unwrap(), id, "idempotent");
        let roots = s.roots();
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[1].name, "My Samples");
        let sub = Library::list_dir(&mut s, &id, "sub").unwrap();
        assert_eq!(sub.entries[0].path, "sub/hit.wav");
        assert!(
            s.external_path(&id, "sub/hit.wav")
                .unwrap()
                .ends_with("hit.wav")
        );
        assert!(matches!(
            s.add_folder("relative"),
            Err(StoreError::InvalidPath(_))
        ));
        assert!(matches!(
            s.add_folder(dir.join("sub/hit.wav").to_str().unwrap()),
            Err(StoreError::InvalidPath(_))
        ));
        assert!(matches!(
            s.add_folder(tmp.path().join("missing").to_str().unwrap()),
            Err(StoreError::NotFound(_))
        ));
        assert!(matches!(
            s.remove_folder("lib"),
            Err(StoreError::InvalidPath(_))
        ));
        s.remove_folder(&id).unwrap();
        assert_eq!(s.roots().len(), 1);
        assert!(matches!(s.remove_folder(&id), Err(StoreError::NotFound(_))));
    }
}

//! [`DiskStore`]: the native [`ProjectStore`] and [`Library`] (folders on disk).
//!
//! Layout (see `ether_controller::store`):
//!
//! ```text
//! <projects_root>/<title> - <short id>/project.ether
//!                                    /media/
//!                                    /cache/
//! ```
//!
//! - Folder names are readable (the project's title, then the end of its id) and folders
//!   are found by id through [`crate::project_dirs`]: the id is the identity. A folder is
//!   renamed to follow its title only while nothing has files open in it
//!   ([`ProjectStore::settle_folder`]); bare-UUID folders of older versions are migrated at
//!   startup ([`DiskStore::settle_all`]).
//! - `projects_root` is per instance in dev builds (`default_projects_root`), so parallel
//!   dev instances never share projects.
//! - Every write of `project.ether` (and of any other file) is atomic: the bytes go to a
//!   temp file in the same folder, which is fsynced and renamed over the target.
//! - Routine autosaves (`save_in_background`) are serialized and written on a writer thread
//!   (`save_writer`); every other access to a `project.ether` waits for it first.
//! - Relative paths are sandboxed: absolute paths, drive prefixes, `..` components,
//!   backslashes and NUL bytes are rejected, and existing targets are canonicalized and
//!   must stay inside their root (so a symlink can't escape it either).
//! - `list_dir` paths: for projects, `rel_path` and the returned entry paths are relative to
//!   the project folder (e.g. `media/kick.wav`); for the library they are relative to the
//!   library root.
//! - Raw engine-machine paths from the client (`MediaSource::Path`, `BundleSource::Path`,
//!   the save-dialog paths of exports, `Browser::AddFolder`, the relink folder search) are
//!   honoured only with [`DiskStore::with_os_paths`] (the desktop app, whose UI is on this
//!   machine). Without it they reply `Unsupported`, except that a path inside one of the
//!   store's own folders (library roots, user library, imported folders, projects, sample
//!   cache) is still read: library files are referenced in place by such paths.

use std::fs;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ether_controller::store::{
    ExportFile, Library, ProjectStore, RenderDocument, RenderSidecar, StoreError, file_kind,
    import_folder_name, is_sidecar_file, private_files, summary_ext, unique_folder_name,
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
/// Folder of the folders imported from a remote UI (`Library::create_import_folder`),
/// next to the user library (`~/Documents/Ethereal/Imported Folders`).
pub const IMPORTED_FOLDERS_DIR: &str = "Imported Folders";

/// Disk-backed project store and sample library. Cheap to clone (paths, and a handle on the
/// shared background writer).
#[derive(Clone, Debug)]
pub struct DiskStore {
    pub projects_root: PathBuf,
    pub library_roots: Vec<LibraryRoot>,
    /// v0.2 (`presets`): the writable user library (`Library::user_root`, id
    /// [`USER_LIBRARY_ID`]). Created on first write. Not listed in `roots()` (the sample
    /// browser shows the configured folders); presets live under its `Presets/` folder.
    pub user_library: Option<PathBuf>,
    /// Routine autosaves in flight (`save_in_background`), shared by the clones.
    writer: Arc<crate::save_writer::SaveWriter>,
    /// The sample pack cache (`Library::sample_cache_*`): `<app data>/samples` in the
    /// hosts; `None` = no cache (the built-in kit only).
    pub sample_cache: Option<PathBuf>,
    /// Raw engine-machine paths from the client are honoured ([`Self::with_os_paths`]).
    os_paths: bool,
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
    // On Windows a `:` in a name opens an alternate data stream of another file, a name
    // ending in `.` or a space names the file without them, and device names (`CON`,
    // `NUL`, ...) open devices: no such file can exist there, so such paths only alias.
    if cfg!(windows) && rel.split('/').any(windows_aliasing_segment) {
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

/// `a` and `b` (both existing) name the same file: a case-only rename on a case-insensitive
/// file system. On Unix by file identity (a case-sensitive file system can hold both
/// `pad` and `Pad`); elsewhere by a case-insensitive path comparison.
fn same_entry(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // The entries themselves: a symlink at `b` to `a` is another entry (renaming over
        // it would replace the link).
        match (fs::symlink_metadata(a), fs::symlink_metadata(b)) {
            (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    }
}

/// A path segment Windows resolves to something else than a file of that name (see
/// [`sanitize_rel`]). `.` and `..` are left to the component checks.
fn windows_aliasing_segment(seg: &str) -> bool {
    seg != "."
        && seg != ".."
        && (seg.contains(':')
            || seg.ends_with(['.', ' '])
            || ether_core::protocol::model::is_windows_device_name(seg))
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

/// Create a fresh hidden temp file next to `name` in `dir` (`.{name}.{pid}-{n}.{ext}`):
/// never an existing entry (`create_new`: an entry planted at the name, a symlink included, is
/// never written through), and two writers of one file in this process never share one.
fn create_temp(dir: &Path, name: &str, ext: &str) -> std::io::Result<(PathBuf, fs::File)> {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut last = None;
    for _ in 0..16 {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        // Unpredictable across runs: a name can't be planted ahead of the write.
        let salt = std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish()
            & 0xffff_ffff;
        let tmp = dir.join(format!(
            ".{name}.{}-{n}-{salt:08x}.{ext}",
            std::process::id()
        ));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
        {
            Ok(f) => return Ok((tmp, f)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("no free temp name")))
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
    let (tmp, mut f) = create_temp(dir, &name, "tmp").map_err(io_err)?;
    let result = (|| {
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

/// A streamed export (`ProjectStore::create_export`): written to a hidden temp file next
/// to its target, fsynced and renamed over it on commit, removed when dropped unfinished.
/// An export folder the user picked (`ExportRequest::folder`): absolute, no NUL.
fn export_folder(folder: &str) -> Result<PathBuf, StoreError> {
    let p = Path::new(folder);
    if folder.contains('\0') || !p.is_absolute() {
        return Err(StoreError::InvalidPath(folder.to_string()));
    }
    Ok(p.to_path_buf())
}

struct PartFile {
    file: Option<BufWriter<fs::File>>,
    tmp: PathBuf,
    path: PathBuf,
}

impl PartFile {
    fn create(path: PathBuf) -> Result<Self, StoreError> {
        let dir = path
            .parent()
            .ok_or_else(|| StoreError::InvalidPath(path.display().to_string()))?;
        fs::create_dir_all(dir).map_err(io_err)?;
        let name = path
            .file_name()
            .ok_or_else(|| StoreError::InvalidPath(path.display().to_string()))?
            .to_string_lossy();
        let (tmp, file) = create_temp(dir, &name, "part").map_err(io_err)?;
        Ok(Self {
            file: Some(BufWriter::with_capacity(1 << 20, file)),
            tmp,
            path,
        })
    }

    fn file(&mut self) -> std::io::Result<&mut BufWriter<fs::File>> {
        self.file
            .as_mut()
            .ok_or_else(|| std::io::Error::other("export file closed"))
    }
}

impl Write for PartFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.file()?.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file()?.flush()
    }
}

impl Seek for PartFile {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.file()?.seek(pos)
    }
}

impl ExportFile for PartFile {
    fn commit(mut self: Box<Self>) -> Result<(), StoreError> {
        let result = (|| {
            let file = self.file.take().expect("open until committed");
            let file = file.into_inner().map_err(|e| e.into_error())?;
            file.sync_all()?;
            drop(file);
            fs::rename(&self.tmp, &self.path)
        })();
        result.map_err(io_err)?;
        #[cfg(unix)]
        if let Some(dir) = self.path.parent()
            && let Ok(d) = fs::File::open(dir)
        {
            let _ = d.sync_all();
        }
        Ok(())
    }
}

impl Drop for PartFile {
    fn drop(&mut self) {
        // Unfinished (cancelled, failed) or a failed commit: leave nothing behind.
        drop(self.file.take());
        let _ = fs::remove_file(&self.tmp);
    }
}

fn modified_ms(path: &Path) -> f64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0.0, |d| d.as_millis() as f64)
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
            writer: Arc::default(),
            sample_cache: None,
            os_paths: false,
        }
    }

    /// Honour raw engine-machine paths from the client (`crate::HostOptions::os_paths`):
    /// only where the UI runs on this machine (the desktop app). Off by default: they reply
    /// `Unsupported` (see the module docs).
    pub fn with_os_paths(mut self, on: bool) -> Self {
        self.os_paths = on;
        self
    }

    /// `Unsupported` unless raw engine-machine paths are honoured.
    fn check_os_paths(&self) -> Result<(), StoreError> {
        if self.os_paths {
            Ok(())
        } else {
            Err(StoreError::Unsupported(
                "engine-machine paths are not available on this host".into(),
            ))
        }
    }

    /// [`Self::check_os_paths`], also passing an absolute `path` inside one of the store's own
    /// folders (library roots, user library, imported folders, projects, sample cache), that
    /// stays inside it once symlinks are resolved.
    fn check_os_path(&self, path: &str) -> Result<(), StoreError> {
        if self.os_paths || self.owns_path(path) {
            Ok(())
        } else {
            self.check_os_paths()
        }
    }

    fn owns_path(&self, path: &str) -> bool {
        let p = Path::new(path);
        let plain = p
            .components()
            .all(|c| !matches!(c, Component::ParentDir | Component::CurDir));
        if path.contains('\0') || !p.is_absolute() || !plain {
            return false;
        }
        let roots = self
            .library_roots
            .iter()
            .map(|r| r.path.clone())
            .chain(self.user_library.clone())
            .chain(self.imports_dir())
            .chain(self.sample_cache.clone())
            .chain(Some(self.projects_root.clone()));
        roots.into_iter().any(|root| {
            let Ok(rel) = p.strip_prefix(&root) else {
                return false;
            };
            let parts: Option<Vec<&str>> =
                rel.components().map(|c| c.as_os_str().to_str()).collect();
            parts.is_some_and(|parts| resolve_in(&root, &parts.join("/")).is_ok())
        })
    }

    /// Keep the sample pack cache in `dir`.
    pub fn with_sample_cache(mut self, dir: Option<PathBuf>) -> Self {
        self.sample_cache = dir;
        self
    }

    /// The sample cache folder, created if missing.
    fn sample_dir(&self) -> Result<PathBuf, StoreError> {
        let dir = self
            .sample_cache
            .as_ref()
            .ok_or_else(|| StoreError::Unsupported("no sample cache".into()))?;
        fs::create_dir_all(dir).map_err(io_err)?;
        Ok(dir.clone())
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

    /// The folder holding imported folders (next to the user library; none
    /// without a user library).
    pub fn imports_dir(&self) -> Option<PathBuf> {
        Some(
            self.user_library
                .as_ref()?
                .parent()?
                .join(IMPORTED_FOLDERS_DIR),
        )
    }

    /// The folder of root `root` if it is an imported folder.
    /// Only a real folder directly inside [`Self::imports_dir`] (a plain name, not `..`, not a
    /// symlink): it is written to and deleted with the root.
    fn imported_dir(&self, root: &str) -> Option<PathBuf> {
        let imports = self.imports_dir()?;
        let r = self.library_roots.iter().find(|r| r.id == root)?;
        let mut parts = r.path.components();
        let named = matches!(parts.next_back(), Some(Component::Normal(_)));
        let inside = named && parts.as_path() == imports.as_path();
        let plain = !r
            .path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir));
        let link = fs::symlink_metadata(&r.path).is_ok_and(|m| m.file_type().is_symlink());
        (inside && plain && !link).then(|| r.path.clone())
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

    /// Folder of a project: where it is (readable name, [`crate::project_dirs`]), else
    /// where a new one is created (`<projects_root>/<uuid>`).
    pub fn project_dir(&self, id: ProjectId) -> PathBuf {
        crate::project_dirs::dir(&self.projects_root, id)
    }

    fn existing_project_dir(&self, id: ProjectId) -> Result<PathBuf, StoreError> {
        crate::project_dirs::existing(&self.projects_root, id)
            .ok_or_else(|| StoreError::NotFound(id.to_string()))
    }

    /// Rename every project folder after its title (at startup, nothing open), migrating
    /// bare-UUID folders. Returns how many were renamed.
    pub fn settle_all(&self) -> usize {
        self.writer.flush();
        crate::project_dirs::settle_all(&self.projects_root)
    }

    fn summary(&self, id: ProjectId) -> Result<ProjectSummary, StoreError> {
        let dir = self.existing_project_dir(id)?;
        let reader = open_regular_file(&dir.join(PROJECT_FILE)).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => StoreError::NotFound(id.to_string()),
            _ => io_err(e),
        })?;
        let head = crate::project_dirs::read_head(std::io::BufReader::new(reader));
        Ok(Self::summary_at(id, &dir, head.name, head.forked_from))
    }

    /// The summary of project `id` in `dir`, its head already read.
    fn summary_at(
        id: ProjectId,
        dir: &Path,
        name: Option<String>,
        forked_from: Option<ether_core::protocol::model::ForkOrigin>,
    ) -> ProjectSummary {
        ProjectSummary {
            id,
            name: name.unwrap_or_else(|| "Untitled".to_string()),
            modified_ms: modified_ms(&dir.join(PROJECT_FILE)),
            ext: Self::summary_ext_at(dir),
            forked_from,
            // Status (archive, bin): the controller's (`project_status`).
            archived: false,
            binned_ms: None,
        }
    }

    /// An extension's [`ProjectSummary::ext`] of the project folder.
    fn summary_ext(&self, id: ProjectId) -> Option<ether_core::protocol::ext::ExtValue> {
        Self::summary_ext_at(&self.existing_project_dir(id).ok()?)
    }

    fn summary_ext_at(dir: &Path) -> Option<ether_core::protocol::ext::ExtValue> {
        summary_ext(&mut |f| {
            let rel = sanitize_rel(f).ok()?;
            read_regular_file(&dir.join(rel), f, MAX_SIDECAR_BYTES).ok()
        })
    }

    /// Accesses to `project.ether` (or a save's sidecar) through the generic file methods
    /// wait for background saves too (media and other files don't: they never race with
    /// them).
    fn flush_if_document(&self, rel_path: &str) {
        if sanitize_rel(rel_path)
            .is_ok_and(|p| p == Path::new(PROJECT_FILE) || p.to_str().is_some_and(is_sidecar_file))
        {
            self.writer.flush();
        }
    }

    /// `save_in_background(_with)`: queue the document (and its sidecar) on the writer.
    fn submit_save(
        &mut self,
        id: ProjectId,
        name: &str,
        render: RenderDocument,
        sidecar: Option<RenderSidecar>,
    ) -> Result<ProjectSummary, StoreError> {
        let dir = self.existing_project_dir(id)?;
        self.writer
            .submit(id, dir.join(PROJECT_FILE), render, sidecar);
        // What `summary` would read once the write lands (without waiting for it).
        let modified_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |d| d.as_millis() as f64);
        Ok(ProjectSummary {
            id,
            name: name.to_string(),
            modified_ms,
            ext: self.summary_ext(id),
            // The controller fills it in from the document it saved.
            forked_from: None,
            // Status (archive, bin): the controller's (`project_status`).
            archived: false,
            binned_ms: None,
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
        self.writer.flush();
        match fs::metadata(&self.projects_root) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io_err(e)),
        }
        let mut out = Vec::new();
        for folder in crate::project_dirs::scan(&self.projects_root) {
            if folder.dir.join(PROJECT_FILE).is_file() {
                out.push(Self::summary_at(
                    folder.id,
                    &folder.dir,
                    folder.name,
                    folder.forked_from,
                ));
            }
        }
        out.sort_by(|a, b| b.modified_ms.total_cmp(&a.modified_ms));
        Ok(out)
    }

    fn create(&mut self, id: ProjectId) -> Result<(), StoreError> {
        // Named by its id until its first `settle_folder`.
        let dir = self.projects_root.join(id.to_string());
        if dir.exists() || crate::project_dirs::existing(&self.projects_root, id).is_some() {
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
        self.writer.flush();
        let dir = self.existing_project_dir(id)?;
        let bytes = read_regular_file(
            &dir.join(PROJECT_FILE),
            &format!("{id}/{PROJECT_FILE}"),
            MAX_LIBRARY_BYTES,
        )?;
        String::from_utf8(bytes)
            .map_err(|_| StoreError::Io(format!("{id}/{PROJECT_FILE} is not UTF-8")))
    }

    fn save(&mut self, id: ProjectId, ether_json: &str) -> Result<ProjectSummary, StoreError> {
        // A queued autosave must not land over this newer document.
        self.writer.flush();
        let started = std::time::Instant::now();
        let dir = self.existing_project_dir(id)?;
        atomic_write(&dir.join(PROJECT_FILE), ether_json.as_bytes())?;
        tracing::info!(
            project = %id,
            kb = ether_json.len() / 1024,
            write_ms = started.elapsed().as_millis() as u64,
            "project saved"
        );
        self.summary(id)
    }

    fn save_in_background(
        &mut self,
        id: ProjectId,
        name: &str,
        render: RenderDocument,
    ) -> Result<ProjectSummary, StoreError> {
        self.submit_save(id, name, render, None)
    }

    fn save_in_background_with(
        &mut self,
        id: ProjectId,
        name: &str,
        render: RenderDocument,
        sidecar: RenderSidecar,
    ) -> Result<ProjectSummary, StoreError> {
        self.submit_save(id, name, render, Some(sidecar))
    }

    fn flush_saves(&mut self) {
        self.writer.flush();
    }

    fn take_save_error(&mut self) -> Option<(ProjectId, StoreError)> {
        self.writer.take_error()
    }

    fn duplicate(&mut self, from: ProjectId, to: ProjectId) -> Result<(), StoreError> {
        self.writer.flush();
        let src = self.existing_project_dir(from)?;
        // Named by its id until its first `settle_folder`.
        let dst = self.projects_root.join(to.to_string());
        if dst.exists() || crate::project_dirs::existing(&self.projects_root, to).is_some() {
            return Err(StoreError::AlreadyExists(to.to_string()));
        }
        // Copy into a temp folder first, then rename: a failed copy leaves no half project.
        let tmp = self
            .projects_root
            .join(format!(".dup-{to}-{}.tmp", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        // A copy is private: no cache, and never an extension's private files.
        let skip: Vec<&str> = ["cache"]
            .into_iter()
            .chain(private_files().iter().copied())
            .collect();
        let result = copy_dir(&src, &tmp, &skip)
            .and_then(|()| fs::create_dir_all(tmp.join("cache")))
            .and_then(|()| fs::rename(&tmp, &dst));
        if let Err(e) = result {
            let _ = fs::remove_dir_all(&tmp);
            return Err(io_err(e));
        }
        Ok(())
    }

    fn delete(&mut self, id: ProjectId) -> Result<(), StoreError> {
        self.writer.flush();
        let dir = self.existing_project_dir(id)?;
        fs::remove_dir_all(dir).map_err(io_err)?;
        crate::project_dirs::forget(&self.projects_root, id);
        Ok(())
    }

    fn settle_folder(&mut self, id: ProjectId) {
        // A queued autosave holds the folder's current path.
        self.writer.flush();
        crate::project_dirs::settle(&self.projects_root, id);
    }

    fn read(&mut self, id: ProjectId, rel_path: &str) -> Result<Vec<u8>, StoreError> {
        self.flush_if_document(rel_path);
        let dir = self.existing_project_dir(id)?;
        let path = resolve_in(&dir, rel_path)?;
        read_regular_file(&path, rel_path, MAX_LIBRARY_BYTES)
    }

    fn write(&mut self, id: ProjectId, rel_path: &str, bytes: &[u8]) -> Result<(), StoreError> {
        self.flush_if_document(rel_path);
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

    fn create_export(
        &mut self,
        id: ProjectId,
        file_name: &str,
    ) -> Result<Box<dyn ExportFile>, StoreError> {
        let rel = ether_controller::store::export_path(file_name)?;
        let dir = self.existing_project_dir(id)?;
        Ok(Box::new(PartFile::create(resolve_in(&dir, &rel)?)?))
    }

    fn export_folder_names(&mut self, folder: &str) -> Result<Vec<String>, StoreError> {
        self.check_os_paths()?;
        let dir = export_folder(folder)?;
        match fs::read_dir(&dir) {
            Ok(rd) => Ok(rd
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(io_err(e)),
        }
    }

    fn create_export_in(
        &mut self,
        folder: &str,
        file_name: &str,
    ) -> Result<(Box<dyn ExportFile>, String), StoreError> {
        self.check_os_paths()?;
        // A single visible segment, like `exports/<file_name>`.
        ether_controller::store::export_path(file_name)?;
        let path = export_folder(folder)?.join(file_name);
        let shown = path
            .to_str()
            .ok_or_else(|| StoreError::InvalidPath(path.display().to_string()))?
            .to_string();
        Ok((Box::new(PartFile::create(path)?), shown))
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
        self.check_os_paths()?;
        let p = bundle_path(path)?;
        atomic_write(p, bytes)
    }

    fn write_midi_file(&mut self, path: &str, bytes: &[u8]) -> Result<(), StoreError> {
        self.check_os_paths()?;
        let p = Path::new(path);
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if path.contains('\0') || !p.is_absolute() || file_kind(name) != FileKind::Midi {
            return Err(StoreError::InvalidPath(format!(
                "not an absolute .mid/.midi path: {path}"
            )));
        }
        atomic_write(p, bytes)
    }

    fn write_project_export(
        &mut self,
        path: &str,
        project: &[u8],
        files: &[(String, Vec<u8>)],
    ) -> Result<(), StoreError> {
        self.check_os_paths()?;
        let p = Path::new(path);
        let ok_ext = p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
            ether_controller::store::PROJECT_EXPORT_EXTENSIONS
                .iter()
                .any(|x| x.eq_ignore_ascii_case(e))
        });
        if path.contains('\0') || !p.is_absolute() || !ok_ext {
            return Err(StoreError::InvalidPath(format!(
                "not an absolute .dawproject/.als/.flp path: {path}"
            )));
        }
        let dir = p
            .parent()
            .ok_or_else(|| StoreError::InvalidPath(path.to_string()))?;
        // Check every path before writing anything.
        for (rel, _) in files {
            ether_controller::store::check_relative_path(rel)?;
        }
        for (rel, bytes) in files {
            atomic_write(&dir.join(rel), bytes)?;
        }
        atomic_write(p, project)
    }

    fn read_import_file(&mut self, path: &str) -> Result<Vec<u8>, StoreError> {
        self.check_os_paths()?;
        let p = Path::new(path);
        if path.contains('\0') || !p.is_absolute() || !ether_controller::store::is_import_file(path)
        {
            return Err(StoreError::InvalidPath(format!(
                "not an absolute DAWproject/.als/.flp/.rpp path: {path}"
            )));
        }
        read_regular_file(p, path, u32::MAX as u64)
    }

    fn read_bundle_file(&mut self, path: &str) -> Result<Vec<u8>, StoreError> {
        self.check_os_paths()?;
        let p = bundle_path(path)?;
        read_regular_file(p, path, u32::MAX as u64)
    }

    fn projects_os_path(&self) -> Option<String> {
        self.projects_root.to_str().map(str::to_string)
    }

    /// The project's folder as the store resolves it ([`Self::project_dir`]), and a
    /// sandboxed file inside it.
    fn project_os_path(&self, id: ProjectId, rel_path: &str) -> Option<String> {
        let dir = self.existing_project_dir(id).ok()?;
        let path = if rel_path.is_empty() {
            dir
        } else {
            resolve_in(&dir, rel_path).ok()?
        };
        path.to_str().map(str::to_string)
    }

    /// v0.3: delete one file (never a folder); missing = `Ok`.
    fn remove(&mut self, id: ProjectId, rel_path: &str) -> Result<(), StoreError> {
        self.flush_if_document(rel_path);
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
    fn sample_cache_read(&mut self, rel: &str) -> Result<Vec<u8>, StoreError> {
        let path = resolve_in(&self.sample_dir()?, rel)?;
        read_regular_file(&path, rel, MAX_LIBRARY_BYTES)
    }

    fn sample_cache_write(&mut self, rel: &str, bytes: &[u8]) -> Result<(), StoreError> {
        let path = resolve_in(&self.sample_dir()?, rel)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io_err)?;
        }
        atomic_write(&path, bytes)
    }

    fn sample_cache_remove(&mut self, rel: &str) -> Result<(), StoreError> {
        let path = resolve_in(&self.sample_dir()?, rel)?;
        match fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(io_err(e)),
            _ => Ok(()),
        }
    }

    fn sample_cache_list(&mut self, dir: &str) -> Result<Vec<String>, StoreError> {
        let path = resolve_in(&self.sample_dir()?, dir)?;
        let Ok(entries) = fs::read_dir(&path) else {
            return Ok(Vec::new());
        };
        Ok(entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| !n.starts_with('.'))
            .collect())
    }

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
        read_regular_file(&path, rel_path, MAX_LIBRARY_BYTES)
    }

    /// v0.2 (`presets`): only the user library (and, imported folders) is
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
        // Case-only renames on case-insensitive file systems see the target as existing. Any
        // entry counts (a dangling symlink too: `rename` would replace it).
        if fs::symlink_metadata(&dst).is_ok() && !same_entry(&src, &dst) {
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

    /// `file-import` (shared with media references): an OS file chosen by the user (file
    /// dialog or drop) or an external reference. Absolute, an audio file by extension, a
    /// regular file (symlinks followed) of at most [`MAX_EXTERNAL_BYTES`]. Without
    /// [`Self::with_os_paths`], only inside the store's own folders.
    fn read_external(&mut self, path: &str) -> Result<Vec<u8>, StoreError> {
        self.check_os_path(path)?;
        read_external_file(path)
    }

    /// Library files are referenced in place, by their absolute path
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

    /// The Relink dialog's folder search (`Unsupported` without [`Self::with_os_paths`]).
    fn list_external_dir(&mut self, path: &str) -> Result<Vec<(String, bool)>, StoreError> {
        self.check_os_paths()?;
        list_external_dir(path)
    }

    /// A user folder becomes a library root (listed by `roots()`, browsable,
    /// importable, referenced in place) with the stable id [`user_folder_id`]. The folder
    /// must be an existing absolute directory. Not persisted here: the controller's browser
    /// index remembers user folders and re-adds them on start. Without
    /// [`Self::with_os_paths`], only a folder inside the store's own folders (an imported
    /// folder, `Browser::ImportFolder`).
    fn add_folder(&mut self, path: &str) -> Result<String, StoreError> {
        self.check_os_path(path)?;
        let p = Path::new(path);
        let dotted = p
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir));
        if path.contains('\0') || !p.is_absolute() || dotted {
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

    /// Forget a user folder (files untouched; an imported
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

    /// `<imports_dir>/<name>` (de-duplicated against existing folders), created.
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

/// Largest file read from a library root (as bundle files: what a 4 GiB WAV can hold).
pub const MAX_LIBRARY_BYTES: u64 = u32::MAX as u64;

/// Largest extension sidecar a project summary reads (`summary_ext`).
const MAX_SIDECAR_BYTES: u64 = 16 << 20;

/// Opens `path` for reading if it is a regular file (symlinks followed): a FIFO or a device
/// would block or never end. `NotFound` when missing, `InvalidInput` when not a file.
///
/// Checked on the opened handle, not before: a path swapped for a FIFO between a check and
/// the open would block the open itself. On Unix the open doesn't wait for a FIFO's writer
/// (`O_NONBLOCK`, which changes nothing for a regular file's reads).
pub(crate) fn open_regular_file(path: &Path) -> std::io::Result<fs::File> {
    #[cfg(unix)]
    let file = {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)?
    };
    // Windows opens no folder without backup semantics ("access denied"): a folder is told
    // apart first, as "not a regular file". FIFOs don't exist there.
    #[cfg(not(unix))]
    let file = {
        if fs::metadata(path)?.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "not a regular file",
            ));
        }
        fs::File::open(path)?
    };
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    Ok(file)
}

/// Reads the regular file at `path` (symlinks followed) of at most `max` bytes; `shown` names
/// it in errors. Anything else is refused before it is opened: a FIFO or a device in a
/// folder the user browses would block the read (and the controller) forever.
fn read_regular_file(path: &Path, shown: &str, max: u64) -> Result<Vec<u8>, StoreError> {
    use std::io::Read;
    let not_found = |e: std::io::Error| match e.kind() {
        std::io::ErrorKind::NotFound => StoreError::NotFound(shown.to_string()),
        _ => io_err(e),
    };
    let file = open_regular_file(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::InvalidInput => StoreError::InvalidPath(format!("not a file: {shown}")),
        _ => not_found(e),
    })?;
    let len = file.metadata().map_err(io_err)?.len();
    if len > max {
        return Err(StoreError::Io(format!(
            "{shown} is larger than {} MiB",
            max >> 20
        )));
    }
    // Up to 4 GiB: a failed reservation is an error, not an abort of the engine.
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(len as usize)
        .map_err(|_| StoreError::Io(format!("not enough memory to read {shown}")))?;
    file.take(max + 1).read_to_end(&mut bytes).map_err(io_err)?;
    if bytes.len() as u64 > max {
        return Err(StoreError::Io(format!(
            "{shown} is larger than {} MiB",
            max >> 20
        )));
    }
    Ok(bytes)
}

/// See `Library::read_external` for `DiskStore`.
/// An absolute `.ether` path from the desktop's OS save/open dialog.
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
    // Audio media, and MIDI files read by `Media::ReadMidiFile`.
    if !matches!(file_kind(name), FileKind::Audio | FileKind::Midi) {
        return Err(StoreError::InvalidPath(format!(
            "not an audio or MIDI file: {path}"
        )));
    }
    read_regular_file(p, path, MAX_EXTERNAL_BYTES)
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
            "format_version": 1,
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
        .with_os_paths(true)
    }

    /// Without `with_os_paths` (a host whose UI is elsewhere) raw engine-machine paths are
    /// `Unsupported`, but paths inside the store's own folders (library files referenced in
    /// place, imported folders) still work.
    #[test]
    fn engine_machine_paths_need_os_paths() {
        let tmp = TempDir::new("store-os-paths");
        let mut s = store(&tmp).with_os_paths(false);
        let outside = tmp.path().join("Desktop");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("hit.wav"), b"RIFF").unwrap();
        fs::write(outside.join("song.ether"), b"PK").unwrap();
        let at = |n: &str| outside.join(n).to_str().unwrap().to_string();
        let dir = outside.to_str().unwrap();
        let unsupported = |r: Result<(), StoreError>, what: &str| {
            assert!(
                matches!(r, Err(StoreError::Unsupported(_))),
                "{what}: {r:?}"
            );
        };
        unsupported(s.read_external(&at("hit.wav")).map(drop), "read outside");
        let lib = tmp.path().join("lib");
        let escape = lib.join("../Desktop/hit.wav");
        unsupported(s.read_external(escape.to_str().unwrap()).map(drop), "..");
        unsupported(s.add_folder(dir).map(drop), "add folder");
        unsupported(s.list_external_dir(dir).map(drop), "list folder");
        unsupported(s.list_external_dir("").map(drop), "folder search probe");
        unsupported(s.write_bundle_file(&at("out.ether"), b"PK"), "write bundle");
        unsupported(
            s.read_bundle_file(&at("song.ether")).map(drop),
            "read bundle",
        );
        unsupported(s.write_midi_file(&at("out.mid"), b"MThd"), "write midi");
        unsupported(s.read_import_file(&at("a.als")).map(drop), "read import");
        unsupported(
            s.write_project_export(&at("out.dawproject"), b"x", &[]),
            "project export",
        );
        unsupported(s.export_folder_names(dir).map(drop), "export folder");
        unsupported(s.create_export_in(dir, "Song.wav").map(drop), "export file");
        assert!(!outside.join("out.ether").exists());
        assert!(!outside.join("out.mid").exists());

        // A library file by its in-place path.
        let kick = s.external_path("lib", "drums/kick.wav").unwrap();
        assert_eq!(s.read_external(&kick).unwrap(), b"RIFF");
        // A symlink inside the library pointing out of it is not the library's.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.join("hit.wav"), lib.join("link.wav")).unwrap();
            let link = lib.join("link.wav");
            unsupported(s.read_external(link.to_str().unwrap()).map(drop), "symlink");
        }
        // An imported folder (created by the store) is added.
        let imported = s.create_import_folder("Field Recordings").unwrap();
        let id = s.add_folder(&imported).unwrap();
        assert_eq!(id, user_folder_id(&imported));
    }

    #[test]
    fn the_sample_cache_lives_in_its_folder() {
        let tmp = TempDir::new("store-sample-cache");
        let mut s = store(&tmp);
        assert!(
            s.sample_cache_read("files/a.wav").is_err(),
            "no cache configured"
        );
        let mut s = s.with_sample_cache(Some(tmp.path().join("samples")));
        assert_eq!(s.sample_cache_list("files").unwrap(), Vec::<String>::new());
        s.sample_cache_write("files/a.wav", b"one").unwrap();
        s.sample_cache_write("abc/manifest.json", b"{}").unwrap();
        assert_eq!(
            fs::read(tmp.path().join("samples/files/a.wav")).unwrap(),
            b"one"
        );
        assert_eq!(s.sample_cache_list("files").unwrap(), ["a.wav"]);
        s.sample_cache_remove("files/a.wav").unwrap();
        s.sample_cache_remove("files/a.wav").unwrap();
        assert!(matches!(
            s.sample_cache_read("files/a.wav"),
            Err(StoreError::NotFound(_))
        ));
        assert!(s.sample_cache_read("../projects/x").is_err());
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
        // On a case-sensitive file system, two names differing by case are two files: a
        // rename onto the other one is refused (it overwrote it).
        s.write_file("user", "Presets/synth/pad.etherpreset", b"low")
            .unwrap();
        s.write_file("user", "Presets/synth/Pad.etherpreset", b"up")
            .unwrap();
        if fs::read(dir.join("pad.etherpreset")).unwrap() == b"low" {
            assert!(matches!(
                s.rename_file(
                    "user",
                    "Presets/synth/pad.etherpreset",
                    "Presets/synth/Pad.etherpreset"
                ),
                Err(StoreError::AlreadyExists(_))
            ));
            assert_eq!(fs::read(dir.join("Pad.etherpreset")).unwrap(), b"up");
        }
        s.remove_file("user", "Presets/synth/pad.etherpreset")
            .unwrap();
        let _ = s.remove_file("user", "Presets/synth/Pad.etherpreset");
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
        // Names Windows resolves to another file, a data stream or a device: refused there
        // (they can't be files on Windows), kept elsewhere.
        for alias in [
            "media/a.wav:stream",
            "media/a.wav.",
            "media/a.wav ",
            "media/CON",
            "media/nul.wav",
            "Com1.txt",
            "media/LPT\u{b9}.wav",
            "conin$",
        ] {
            assert_eq!(sanitize_rel(alias).is_err(), cfg!(windows), "{alias}");
        }
        for fine in [
            "media/console.wav",
            "media/COM10.wav",
            "media/a.b.wav",
            "media/.hidden",
        ] {
            assert!(
                !windows_aliasing_segment(fine.rsplit('/').next().unwrap()),
                "{fine}"
            );
            assert!(sanitize_rel(fine).is_ok(), "{fine}");
        }
        assert!(windows_aliasing_segment("a.wav:s"));
        assert!(windows_aliasing_segment("LPT\u{b9}"));
        assert!(windows_aliasing_segment("aux.mid"));
        assert!(windows_aliasing_segment("x. "));
    }

    /// `ExportRequest::folder`: files go into the chosen folder (created when missing),
    /// listed by absolute path; relative folders and unsafe names are refused.
    #[test]
    fn exports_into_a_chosen_folder() {
        let tmp = TempDir::new("store-export-folder");
        let mut s = store(&tmp);
        let folder = tmp.path().join("bounces");
        let f = folder.to_str().unwrap();
        assert!(
            s.export_folder_names(f).unwrap().is_empty(),
            "missing = empty"
        );
        let (mut file, path) = s.create_export_in(f, "Song.wav").unwrap();
        assert_eq!(Path::new(&path), folder.join("Song.wav"));
        file.write_all(b"RIFF").unwrap();
        file.commit().unwrap();
        assert_eq!(fs::read(folder.join("Song.wav")).unwrap(), b"RIFF");
        assert_eq!(
            s.export_folder_names(f).unwrap(),
            vec!["Song.wav".to_string()]
        );
        assert!(s.create_export_in("bounces", "Song.wav").is_err());
        assert!(s.create_export_in(f, "../Song.wav").is_err());
        assert!(s.create_export_in(f, ".hidden").is_err());
    }

    /// A FIFO in a project folder (copied in by hand) is refused without being opened:
    /// opening it would wait for a writer and block the controller (listing at startup,
    /// loading, reading a media file).
    #[cfg(unix)]
    #[test]
    fn project_fifos_refused() {
        let tmp = TempDir::new("store-project-fifos");
        let mut s = store(&tmp);
        let a = pid(1);
        s.create(a).unwrap();
        let dir = s.project_dir(a);
        let fifo = |p: &Path| {
            std::process::Command::new("mkfifo")
                .arg(p)
                .status()
                .is_ok_and(|s| s.success())
        };
        if !fifo(&dir.join(PROJECT_FILE)) || !fifo(&dir.join("media/pipe.wav")) {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let listed = ProjectStore::list(&mut s).map(|l| l.len());
            let loaded = s.load(a).map(|_| ());
            let read = ProjectStore::read(&mut s, a, "media/pipe.wav").map(|_| ());
            let _ = tx.send((listed, loaded, read));
        });
        let (listed, loaded, read) = rx
            .recv_timeout(std::time::Duration::from_secs(20))
            .expect("no read blocks on a FIFO");
        assert_eq!(listed.unwrap(), 0);
        assert!(matches!(loaded, Err(StoreError::InvalidPath(_))));
        assert!(matches!(read, Err(StoreError::InvalidPath(_))));
    }

    /// The regular-file check is made on the opened handle, and opening a FIFO doesn't wait
    /// for a writer: a path swapped for one after any earlier check can't block the open.
    #[cfg(unix)]
    #[test]
    fn opening_a_fifo_never_blocks() {
        let tmp = TempDir::new("store-open-fifo");
        let fifo = tmp.path().join("pipe.als");
        let file = tmp.path().join("song.als");
        fs::write(&file, b"als").unwrap();
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .is_ok_and(|s| s.success());
        if !made {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let p = fifo.clone();
        std::thread::spawn(move || {
            let _ = tx.send(open_regular_file(&p).map(|_| ()).map_err(|e| e.kind()));
        });
        let opened = rx
            .recv_timeout(std::time::Duration::from_secs(20))
            .expect("opening a FIFO doesn't block");
        assert_eq!(opened, Err(std::io::ErrorKind::InvalidInput));
        assert!(matches!(
            read_regular_file(&fifo, "pipe.als", 1 << 20),
            Err(StoreError::InvalidPath(_))
        ));
        assert_eq!(
            read_regular_file(&file, "song.als", 1 << 20).unwrap(),
            b"als"
        );
        assert!(matches!(
            read_regular_file(&file, "song.als", 2),
            Err(StoreError::Io(_))
        ));
    }

    /// Writers of one file (an autosave racing a save) never share a temp file: each write
    /// lands whole, and no temp file is left behind.
    #[test]
    fn concurrent_atomic_writes_never_mix() {
        let tmp = TempDir::new("store-atomic-race");
        let path = tmp.path().join("project.ether");
        let writers: Vec<_> = (0..4u8)
            .map(|w| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for _ in 0..50 {
                        atomic_write(&path, &vec![w; 64 * 1024]).unwrap();
                    }
                })
            })
            .collect();
        for w in writers {
            w.join().unwrap();
        }
        let bytes = fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 64 * 1024);
        assert!(bytes.iter().all(|b| *b == bytes[0]), "one writer's bytes");
        let names: Vec<String> = fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["project.ether"]);
    }

    /// A rename never replaces an entry at the target: a symlink to the source (not "the same
    /// file") or a dangling one.
    #[cfg(unix)]
    #[test]
    fn renames_never_replace_a_link() {
        let tmp = TempDir::new("store-rename-links");
        let mut s = store(&tmp);
        s.write_file("user", "Presets/A.etherpreset", b"a").unwrap();
        let dir = tmp.path().join(USER_LIBRARY_DIR).join("Presets");
        std::os::unix::fs::symlink(dir.join("A.etherpreset"), dir.join("L.etherpreset")).unwrap();
        std::os::unix::fs::symlink(dir.join("gone"), dir.join("D.etherpreset")).unwrap();
        assert!(matches!(
            s.rename_file("user", "Presets/A.etherpreset", "Presets/L.etherpreset"),
            Err(StoreError::AlreadyExists(_))
        ));
        // A dangling one is refused as well (it resolves nowhere in the library).
        assert!(
            s.rename_file("user", "Presets/A.etherpreset", "Presets/D.etherpreset")
                .is_err()
        );
        assert!(
            fs::symlink_metadata(dir.join("D.etherpreset"))
                .unwrap()
                .is_symlink()
        );
        assert!(
            fs::symlink_metadata(dir.join("L.etherpreset"))
                .unwrap()
                .is_symlink()
        );
        assert_eq!(fs::read(dir.join("A.etherpreset")).unwrap(), b"a");
    }

    /// A streamed export is written under a hidden temp name, seekable (headers are
    /// patched last), renamed on commit; dropped unfinished, it leaves nothing.
    #[test]
    fn streamed_exports_commit_or_vanish() {
        let tmp = TempDir::new("store-streamed-export");
        let mut s = store(&tmp);
        let a = pid(1);
        s.create(a).unwrap();
        let exports = s.project_dir(a).join("exports");
        let mut f = s.create_export(a, "Song.wav").unwrap();
        f.write_all(b"....data").unwrap();
        f.seek(SeekFrom::Start(0)).unwrap();
        f.write_all(b"HEAD").unwrap();
        f.seek(SeekFrom::End(0)).unwrap();
        f.write_all(b"!").unwrap();
        assert!(
            !exports.join("Song.wav").exists(),
            "not visible until committed"
        );
        assert_eq!(
            s.commit_export(a, "Song.wav", f).unwrap(),
            "exports/Song.wav"
        );
        assert_eq!(fs::read(exports.join("Song.wav")).unwrap(), b"HEADdata!");
        let f = s.create_export(a, "Cancelled.flac").unwrap();
        drop(f);
        let names: Vec<_> = fs::read_dir(&exports)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["Song.wav"], "no temp file left");
        assert!(s.create_export(a, ".hidden").is_err());
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

    /// A document as the app writes it (`file::save`) named `name`, with `blob_bytes` of
    /// data after the settings (where plugin states and the rest of a big project go).
    fn saved_ether(name: &str, blob_bytes: usize) -> String {
        use ether_core::protocol::model::{file, ids::IdGen, project::Project};
        let mut project = Project::new(&mut IdGen::new(7), 1_700_000_000_000);
        project.settings.name = name.to_string();
        let mut json = file::save(&project, "test").unwrap();
        // The end of `project` (the last `}` at its indentation).
        let end = json.rfind("\n  }").unwrap();
        let item =
            r#"{"id": "01HZY3K8Q2J7W9X4V6B5N1M0PA", "value": 0.5, "state": "AAECAwQFBgc="},"#;
        let items = item.repeat(blob_bytes / item.len());
        json.insert_str(end, &format!(",\n    \"blob\": [{}0]", items));
        json
    }

    /// Counts the bytes read through it.
    struct Counting<'a, R>(R, &'a std::cell::Cell<usize>);

    impl<R: std::io::Read> std::io::Read for Counting<'_, R> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.0.read(buf)?;
            self.1.set(self.1.get() + n);
            Ok(n)
        }
    }

    #[test]
    fn the_name_is_read_from_the_head_of_the_document() {
        let name = "mix \"2\" \u{e9}t\u{e9}";
        let json = saved_ether(name, 4 << 20);
        serde_json::from_str::<serde_json::Value>(&json).expect("valid JSON");
        let read = std::cell::Cell::new(0);
        let reader = std::io::BufReader::new(Counting(json.as_bytes(), &read));
        assert_eq!(
            crate::project_dirs::read_head(reader).name.as_deref(),
            Some(name)
        );
        assert!(
            read.get() <= 16 << 10,
            "read {} of {} bytes",
            read.get(),
            json.len()
        );

        // Other key orders (`ether()` sorts them), with data before the name.
        let late = serde_json::json!({
            "format": "ethereal-project",
            "project": { "a": ["x".repeat(100_000), { "name": "no" }], "settings": { "name": "late" } }
        })
        .to_string();
        assert_eq!(
            crate::project_dirs::read_head(late.as_bytes())
                .name
                .as_deref(),
            Some("late")
        );
        for bad in [
            "",
            "not json",
            "[]",
            r#"{"project": 3}"#,
            r#"{"project": {"settings": {"name": 3}}}"#,
            r#"{"project": {"settings": {}}, "name": "x"}"#,
            r#"{"project": {"settings": {"name": "trunc"#,
        ] {
            assert_eq!(
                crate::project_dirs::read_head(bad.as_bytes()).name,
                None,
                "{bad}"
            );
        }
    }

    /// `cargo test -p ether-native --release -- --ignored --nocapture list_a_big_library`:
    /// list 25 projects of 20 MB (500 MB).
    #[test]
    #[ignore = "writes 500 MB"]
    fn list_a_big_library() {
        let tmp = TempDir::new("store-big-library");
        let mut s = store(&tmp);
        for n in 0..25 {
            s.create(pid(n)).unwrap();
            s.save(pid(n), &saved_ether(&format!("song {n}"), 20 << 20))
                .unwrap();
        }
        let started = std::time::Instant::now();
        let list = ProjectStore::list(&mut s).unwrap();
        let elapsed = started.elapsed();
        eprintln!("listed {} projects in {elapsed:?}", list.len());
        assert_eq!(list.len(), 25);
        assert!(list.iter().all(|p| p.name.starts_with("song ")));
    }

    fn render(json: String) -> RenderDocument {
        Box::new(move || Ok(json))
    }

    #[test]
    fn background_saves_land_before_any_other_access() {
        let tmp = TempDir::new("store-background");
        let mut s = store(&tmp);
        let a = pid(4);
        s.create(a).unwrap();
        let summary = s
            .save_in_background(a, "Named", render(ether("bg 1")))
            .unwrap();
        assert_eq!((summary.id, summary.name.as_str()), (a, "Named"));
        assert!(summary.modified_ms > 0.0);
        // `load` (like `list`, `duplicate`, `read` of the document...) waits for it.
        assert_eq!(s.load(a).unwrap(), ether("bg 1"));
        // A synchronous save is never overwritten by an older queued one.
        s.save_in_background(a, "x", render(ether("bg 2"))).unwrap();
        s.save(a, &ether("sync")).unwrap();
        assert_eq!(s.load(a).unwrap(), ether("sync"));
        s.save_in_background(a, "x", render(ether("bg 3"))).unwrap();
        assert_eq!(
            ProjectStore::read(&mut s, a, PROJECT_FILE).unwrap(),
            ether("bg 3").into_bytes()
        );
        s.save_in_background(a, "x", render(ether("bg 4"))).unwrap();
        let list = ProjectStore::list(&mut s).unwrap();
        assert_eq!(list[0].name, "bg 4");
        assert!(s.take_save_error().is_none());
        // Missing project: refused at once.
        assert!(matches!(
            s.save_in_background(pid(9), "x", render(String::new())),
            Err(StoreError::NotFound(_))
        ));
    }

    #[test]
    fn queued_background_saves_coalesce() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::mpsc;
        let tmp = TempDir::new("store-coalesce");
        let mut s = store(&tmp);
        let a = pid(5);
        s.create(a).unwrap();
        let renders = Arc::new(AtomicUsize::new(0));
        let (started_tx, started) = mpsc::channel();
        let (go, go_rx) = mpsc::channel::<()>();
        let job = |json: &str, gate: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>| {
            let (renders, json) = (renders.clone(), ether(json));
            let job: RenderDocument = Box::new(move || {
                if let Some((started, go)) = gate {
                    started.send(()).unwrap();
                    go.recv().unwrap();
                }
                renders.fetch_add(1, Ordering::SeqCst);
                Ok(json)
            });
            job
        };
        // The first write is in progress while two more are queued: only the newest runs.
        s.save_in_background(a, "x", job("first", Some((started_tx, go_rx))))
            .unwrap();
        started.recv().unwrap();
        s.save_in_background(a, "x", job("second", None)).unwrap();
        s.save_in_background(a, "x", job("third", None)).unwrap();
        go.send(()).unwrap();
        s.flush_saves();
        assert_eq!(renders.load(Ordering::SeqCst), 2);
        assert_eq!(s.load(a).unwrap(), ether("third"));
    }

    #[test]
    fn background_save_failures_are_reported_once() {
        let tmp = TempDir::new("store-bg-error");
        let mut s = store(&tmp);
        let a = pid(6);
        s.create(a).unwrap();
        s.save(a, &ether("kept")).unwrap();
        s.save_in_background(
            a,
            "x",
            Box::new(|| Err(StoreError::Io("render failed".into()))),
        )
        .unwrap();
        s.flush_saves();
        assert_eq!(
            s.take_save_error(),
            Some((a, StoreError::Io("render failed".into())))
        );
        assert_eq!(s.take_save_error(), None);
        assert_eq!(s.load(a).unwrap(), ether("kept"));
    }

    #[test]
    fn dropping_the_store_finishes_queued_saves() {
        let tmp = TempDir::new("store-bg-drop");
        let mut s = store(&tmp);
        let a = pid(7);
        s.create(a).unwrap();
        let file = s.project_dir(a).join(PROJECT_FILE);
        s.save_in_background(a, "x", render(ether("last words")))
            .unwrap();
        drop(s);
        assert_eq!(fs::read_to_string(file).unwrap(), ether("last words"));
    }

    #[test]
    fn reveal_paths_come_from_the_stores_own_folders() {
        use ether_controller::reveal::{Resolved, resolve};
        use ether_core::protocol::media::{MediaSource, RevealTarget};
        let tmp = TempDir::new("store-reveal");
        let s = {
            let mut s = store(&tmp);
            s.create(pid(5)).unwrap();
            s.write(pid(5), "media/kick.wav", b"RIFF").unwrap();
            s
        };
        let dir = s.project_dir(pid(5));
        let at = |p: &Path| p.to_str().unwrap().to_string();
        assert_eq!(s.projects_os_path(), Some(at(&s.projects_root)));
        assert_eq!(s.project_os_path(pid(5), ""), Some(at(&dir)));
        assert_eq!(
            s.project_os_path(pid(5), "media/kick.wav"),
            Some(at(&dir.join("media").join("kick.wav")))
        );
        assert_eq!(s.project_os_path(pid(5), "../x"), None);
        assert_eq!(s.project_os_path(pid(5), "/etc/passwd"), None);
        assert_eq!(s.project_os_path(pid(6), ""), None, "no such project");

        let r = |t: RevealTarget| resolve(&t, &s, &s, None, |_| false);
        assert_eq!(
            r(RevealTarget::Projects).unwrap(),
            Resolved {
                path: at(&s.projects_root),
                open: true
            }
        );
        assert_eq!(
            r(RevealTarget::Project { id: pid(5) }).unwrap(),
            Resolved {
                path: at(&dir),
                open: false
            }
        );
        assert_eq!(
            r(RevealTarget::Media {
                source: MediaSource::Location {
                    location: BrowseLocation::Library { id: "lib".into() },
                    path: "drums/kick.wav".into(),
                },
            })
            .unwrap()
            .path,
            at(&tmp.path().join("lib").join("drums").join("kick.wav"))
        );
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

    /// `Media::WriteMidiFile { path }` and `ReadMidiFile { Path }`.
    #[test]
    fn midi_files_at_absolute_midi_paths_only() {
        let tmp = TempDir::new("store-midi");
        let mut s = store(&tmp);
        let path = tmp.path().join("Riff.mid");
        let path = path.to_str().unwrap();
        s.write_midi_file(path, b"MThd").unwrap();
        assert_eq!(read_external_file(path).unwrap(), b"MThd");
        for bad in ["Riff.mid", "/tmp/riff.wav", "/tmp/a\0.mid"] {
            assert!(matches!(
                s.write_midi_file(bad, b"x"),
                Err(StoreError::InvalidPath(_))
            ));
        }
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

    /// The tiered pruning works on the disk folder (an existing project
    /// with the old 50 five-minute autosaves is thinned to 30; named versions stay).
    #[test]
    fn versions_are_pruned_on_disk() {
        use ether_controller::versions::prune_versions;
        use ether_core::protocol::versions::MAX_AUTOSAVE_VERSIONS;
        let tmp = TempDir::new("store-versions-prune");
        let mut s = store(&tmp);
        let a = pid(9);
        s.create(a).unwrap();
        let t0 = 1_750_000_000_000u64;
        for i in 0..50 {
            let ms = t0 + i * 300_000;
            s.write(a, &format!("versions/{ms}-autosave.ether"), b"{}")
                .unwrap();
        }
        s.write(a, &format!("versions/{t0}-manual.ether"), b"{}")
            .unwrap();
        let named = t0 + 300_000;
        s.write(
            a,
            "versions/names.json",
            format!("{{\"{named}-autosave\": \"Named\"}}").as_bytes(),
        )
        .unwrap();
        let now = t0 + 50 * 300_000;
        assert_eq!(
            prune_versions(&mut s, a, now),
            50 - 1 - MAX_AUTOSAVE_VERSIONS
        );
        let dir = s.project_dir(a).join("versions");
        let autosaves = std::fs::read_dir(&dir)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with("-autosave.ether")
            })
            .count();
        assert_eq!(autosaves, MAX_AUTOSAVE_VERSIONS + 1);
        assert!(dir.join(format!("{t0}-manual.ether")).exists());
        assert!(dir.join(format!("{named}-autosave.ether")).exists());
        assert!(
            dir.join(format!("{}-autosave.ether", now - 300_000))
                .exists()
        );
        assert_eq!(prune_versions(&mut s, a, now), 0, "stable");
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

    /// Readable folder names (`project_dirs`): a project keeps working through renames of
    /// its folder, and autosaves queued before one land in the renamed folder.
    #[test]
    fn folders_follow_titles_when_settled() {
        let tmp = TempDir::new("store-settle");
        let mut s = store(&tmp);
        let a = pid(30);
        let doc = |name: &str| {
            serde_json::json!({
                "format": "ethereal-project",
                "format_version": 1,
                "app_version": "test",
                "project": { "id": a.to_string(), "settings": { "name": name } }
            })
            .to_string()
        };
        s.create(a).unwrap();
        assert_eq!(s.project_dir(a), s.projects_root.join(a.to_string()));
        s.save(a, &doc("Song")).unwrap();
        s.write(a, "media/a.wav", b"A").unwrap();
        s.settle_folder(a);
        let readable = s
            .projects_root
            .join(crate::project_dirs::folder_name("Song", a));
        assert_eq!(s.project_dir(a), readable);
        assert!(!s.projects_root.join(a.to_string()).exists());
        assert_eq!(ProjectStore::read(&mut s, a, "media/a.wav").unwrap(), b"A");

        // Renamed while open: autosaves keep landing; the folder follows once settled.
        let json = doc("Final");
        s.save_in_background(a, "Final", render(json.clone()))
            .unwrap();
        s.settle_folder(a);
        let renamed = s
            .projects_root
            .join(crate::project_dirs::folder_name("Final", a));
        assert_eq!(s.project_dir(a), renamed);
        assert_eq!(s.load(a).unwrap(), json);
        assert_eq!(ProjectStore::read(&mut s, a, "media/a.wav").unwrap(), b"A");
        let list = ProjectStore::list(&mut s).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].id, list[0].name.as_str()), (a, "Final"));

        // A copy gets a folder of its own; deleting finds the readable folder.
        let b = pid(31);
        s.duplicate(a, b).unwrap();
        assert_eq!(s.project_dir(b), s.projects_root.join(b.to_string()));
        s.delete(a).unwrap();
        assert!(!renamed.exists());
        assert!(matches!(s.load(a), Err(StoreError::NotFound(_))));
        assert_eq!(ProjectStore::list(&mut s).unwrap().len(), 1);
    }

    /// An extension's files (`store::set_store_ext`): its summary, never copied by
    /// `duplicate` (SaveAs / Duplicate make a private project).
    #[test]
    fn an_extensions_private_files_and_summary() {
        use ether_controller::store::test_store_ext::{self, PRIVATE};
        test_store_ext::install();
        let tmp = TempDir::new("store-ext");
        let mut s = store(&tmp);
        let (host, dup) = (pid(10), pid(11));
        s.create(host).unwrap();
        s.save(host, &ether("Song")).unwrap();
        assert_eq!(ProjectStore::list(&mut s).unwrap()[0].ext, None);
        s.write(host, PRIVATE, br#"{"badge":1}"#).unwrap();
        let ext = ProjectStore::list(&mut s).unwrap()[0].ext.clone();
        assert_eq!(ext.unwrap().0, serde_json::json!({"badge": 1}));
        // `save` reports it too (the `Saved` event's summary).
        assert!(s.save(host, &ether("Song")).unwrap().ext.is_some());
        // An unreadable file lists the project without it, never hides it.
        s.write(host, PRIVATE, b"{ corrupt").unwrap();
        assert_eq!(ProjectStore::list(&mut s).unwrap()[0].ext, None);

        s.write(host, PRIVATE, b"{}").unwrap();
        s.write(host, "media/a.wav", b"A").unwrap();
        s.duplicate(host, dup).unwrap();
        assert!(!s.project_dir(dup).join(PRIVATE).exists());
        assert_eq!(
            ProjectStore::read(&mut s, dup, "media/a.wav").unwrap(),
            b"A"
        );
        let list = ProjectStore::list(&mut s).unwrap();
        assert_eq!(list.iter().find(|p| p.id == dup).unwrap().ext, None);
    }

    /// A save's sidecar (`save_in_background_with`) is written right after its document,
    /// from its JSON; reading or replacing it waits for the queue.
    #[test]
    fn a_background_save_writes_its_sidecar_after_the_document() {
        use ether_controller::store::test_store_ext::{self, SIDECAR};
        test_store_ext::install();
        let tmp = TempDir::new("store-sidecar");
        let mut s = store(&tmp);
        let a = pid(6);
        s.create(a).unwrap();
        let sidecar = |tag: &'static str| -> RenderSidecar {
            Box::new(move |json: &str| {
                Some((
                    SIDECAR.to_string(),
                    format!("{tag}:{}", json.len()).into_bytes(),
                ))
            })
        };
        let doc = ether("with sidecar");
        s.save_in_background_with(a, "x", render(doc.clone()), sidecar("j1"))
            .unwrap();
        assert_eq!(
            ProjectStore::read(&mut s, a, SIDECAR).unwrap(),
            format!("j1:{}", doc.len()).into_bytes()
        );
        assert_eq!(s.load(a).unwrap(), doc);
        // A path that leaves the folder is never written.
        s.save_in_background_with(
            a,
            "x",
            render(ether("evil")),
            Box::new(|_: &str| Some(("../escape.json".to_string(), b"x".to_vec()))),
        )
        .unwrap();
        s.flush_saves();
        assert!(!tmp.path().join("projects/escape.json").exists());
        // Clearing it (a write) waits for a queued save that carries one.
        s.save_in_background_with(a, "x", render(ether("again")), sidecar("j2"))
            .unwrap();
        ProjectStore::write(&mut s, a, SIDECAR, b"").unwrap();
        assert!(ProjectStore::read(&mut s, a, SIDECAR).unwrap().is_empty());
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
            Library::read(&mut s, "lib", "drums"),
            Err(StoreError::InvalidPath(_))
        ));
        assert!(matches!(
            Library::read(&mut s, "lib", "drums/none.wav"),
            Err(StoreError::NotFound(_))
        ));
        // A FIFO is refused without being opened (opening it would wait for a writer).
        #[cfg(unix)]
        {
            let fifo = tmp.path().join("lib/drums/pipe.wav");
            let made = std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .is_ok_and(|s| s.success());
            if made {
                assert!(matches!(
                    Library::read(&mut s, "lib", "drums/pipe.wav"),
                    Err(StoreError::InvalidPath(_))
                ));
            }
        }
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

    /// Only a folder directly inside the imports folder is deleted with its root: a `..` path
    /// or a symlink there is refused or forgotten without touching the disk.
    #[test]
    fn removing_a_folder_deletes_only_a_real_imported_folder() {
        let tmp = TempDir::new("store-imported-dir");
        let mut s = store(&tmp);
        let imported = s.create_import_folder("x").unwrap();
        let id = s.add_folder(&imported).unwrap();
        let up = Path::new(&imported).join("..");
        assert!(matches!(
            s.add_folder(up.to_str().unwrap()),
            Err(StoreError::InvalidPath(_))
        ));
        #[cfg(unix)]
        {
            let keep = tmp.path().join("keep");
            fs::create_dir_all(&keep).unwrap();
            fs::write(keep.join("a.wav"), b"RIFF").unwrap();
            let link = s.imports_dir().unwrap().join("link");
            std::os::unix::fs::symlink(&keep, &link).unwrap();
            let lid = s.add_folder(link.to_str().unwrap()).unwrap();
            s.remove_folder(&lid).unwrap();
            assert!(
                keep.join("a.wav").exists(),
                "a symlinked folder's target is kept"
            );
        }
        assert!(Path::new(&imported).is_dir());
        s.remove_folder(&id).unwrap();
        assert!(
            !Path::new(&imported).exists(),
            "an imported folder's copy is deleted"
        );
        assert!(tmp.path().join("projects").parent().unwrap().is_dir());
    }
}

//! OPFS-backed `ProjectStore` and `Library` (inside the controller Worker).
//!
//! The store logic is written against a small synchronous [`Fs`] trait:
//! - on the web, [`crate::web::JsFs`] calls into JS, which reaches OPFS through a helper
//!   Worker (OPFS is async; the controller Worker blocks on `Atomics.wait` for the answer),
//! - in tests, [`MemFs`] (in-memory OPFS fake).
//!
//! OPFS layout (same as native, under a `projects/` root):
//!
//! ```text
//! projects/<project-uuid>/project.ether
//!                        /media/
//!                        /cache/
//! library/                            the web sample library root
//! user-library/                       the writable user library (v0.2: `Presets/`, ...)
//! imported/<folder>/                  folders imported from the user's computer (base-136)
//! ```

use std::collections::BTreeMap;

use ether_controller::store::{
    Library, ProjectStore, SHARE_FILE, StoreError, USER_FOLDER_PREFIX, check_relative_path,
    file_kind, import_folder_name, share_info, unique_folder_name, user_folder_id,
};
use ether_core::protocol::media::{
    BrowseLocation, BrowseRoot, DirectoryEntry, DirectoryListing, FileKind,
};
use ether_core::protocol::model::ProjectId;
use ether_core::protocol::project::ProjectSummary;

pub const PROJECTS_ROOT: &str = "projects";
pub const LIBRARY_ROOT: &str = "library";
pub const LIBRARY_ID: &str = "browser";
/// v0.2 (`presets`): OPFS folder of the writable user library (`Library::user_root`).
pub const USER_LIBRARY_ROOT: &str = "user-library";
/// v0.2 (`presets`): root id of the user library (not listed in `roots()`).
pub const USER_LIBRARY_ID: &str = "user";
/// `base-136`: OPFS folder of the folders imported from the user's computer, one sub-folder
/// each (`Library::create_import_folder`; their engine-side path is `imported/<folder>`).
pub const IMPORTED_ROOT: &str = "imported";
pub const PROJECT_FILE: &str = "project.ether";
pub const MEDIA_DIR: &str = "media";
pub const CACHE_DIR: &str = "cache";

/// A file or directory entry.
#[derive(Clone, Debug, PartialEq)]
pub struct FsEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    /// Last modification, Unix ms (0 for directories).
    pub modified_ms: f64,
}

/// Minimal synchronous file system. Paths are `/`-separated and relative to the OPFS root;
/// the store validates every user-supplied component before calling it.
pub trait Fs {
    fn read(&mut self, path: &str) -> Result<Vec<u8>, StoreError>;
    /// Create or replace a file (parent directories are created).
    fn write(&mut self, path: &str, bytes: &[u8]) -> Result<(), StoreError>;
    /// Move a file, replacing the destination.
    fn rename(&mut self, from: &str, to: &str) -> Result<(), StoreError>;
    /// Entries of a directory (`NotFound` if missing).
    fn list(&mut self, dir: &str) -> Result<Vec<FsEntry>, StoreError>;
    /// Create a directory and its parents (ok if it exists).
    fn mkdir(&mut self, path: &str) -> Result<(), StoreError>;
    /// Remove a file or a directory recursively (`NotFound` if missing).
    fn remove(&mut self, path: &str) -> Result<(), StoreError>;
    fn stat(&mut self, path: &str) -> Result<Option<FsEntry>, StoreError>;
}

/// Validate a relative path from the controller/UI (same rules as native:
/// [`check_relative_path`]). `""` is the root itself.
fn relative(path: &str) -> Result<&str, StoreError> {
    check_relative_path(path)?;
    Ok(path)
}

fn join(base: &str, rel: &str) -> String {
    if rel.is_empty() {
        base.to_string()
    } else if base.is_empty() {
        rel.to_string()
    } else {
        format!("{base}/{rel}")
    }
}

fn listing<F: Fs>(
    fs: &mut F,
    base: &str,
    rel: &str,
    location: BrowseLocation,
) -> Result<DirectoryListing, StoreError> {
    let rel = relative(rel)?;
    let mut entries: Vec<DirectoryEntry> = fs
        .list(&join(base, rel))?
        .into_iter()
        .filter(|e| !e.name.starts_with('.'))
        .map(|e| DirectoryEntry {
            path: join(rel, &e.name),
            kind: if e.is_dir {
                FileKind::Directory
            } else {
                file_kind(&e.name)
            },
            size: e.size as f64,
            name: e.name,
        })
        .collect();
    entries.sort_by(|a, b| {
        (a.kind != FileKind::Directory)
            .cmp(&(b.kind != FileKind::Directory))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(DirectoryListing {
        location,
        path: rel.to_string(),
        entries,
    })
}

/// Display name inside a `.ether` document (`project.settings.name`), without depending on
/// the full model loader (a partially valid file still lists).
fn name_of(ether_json: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Value>(ether_json)
        .ok()
        .and_then(|v| {
            v.pointer("/project/settings/name")
                .and_then(|n| n.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "Untitled".to_string())
}

/// Upload staging root (`file-import`), outside `projects/` so it never lists as a project.
pub const UPLOADS_ROOT: &str = "uploads";

/// `true` for upload ids that are safe as a single path segment (as natively).
pub fn valid_upload_id(upload: &str) -> bool {
    !upload.is_empty()
        && upload.len() <= 64
        && upload
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// A staged upload: `uploads/<id>/<offset>` chunk files (OPFS has no append, and rewriting
/// one growing file would be quadratic), concatenated in offset order by `read_upload`.
#[derive(Debug, Clone, Copy)]
struct Staged {
    size: u64,
    received: u64,
}

/// `ProjectStore` over an [`Fs`] (OPFS on the web).
pub struct WebStore<F: Fs> {
    fs: F,
    uploads: BTreeMap<String, Staged>,
}

impl<F: Fs> WebStore<F> {
    pub fn new(fs: F) -> Self {
        Self {
            fs,
            uploads: BTreeMap::new(),
        }
    }

    fn upload_dir(upload: &str) -> Result<String, StoreError> {
        if !valid_upload_id(upload) {
            return Err(StoreError::InvalidPath(format!("upload id {upload:?}")));
        }
        Ok(format!("{UPLOADS_ROOT}/{upload}"))
    }

    fn dir(id: ProjectId) -> String {
        format!("{PROJECTS_ROOT}/{id}")
    }

    fn tmp_file(id: ProjectId) -> String {
        format!("{}/.{PROJECT_FILE}.tmp", Self::dir(id))
    }

    /// Read the document and the path it came from. `save` writes a temp file and then
    /// renames it over `project.ether`; OPFS has no atomic replace everywhere, so a tab
    /// closed mid-save can leave `project.ether` missing, empty or truncated while the
    /// complete temp file is still there. Fall back to it in that case.
    fn read_document(&mut self, id: ProjectId) -> Result<(Vec<u8>, String), StoreError> {
        let main = format!("{}/{PROJECT_FILE}", Self::dir(id));
        let valid = |b: &[u8]| serde_json::from_slice::<serde_json::Value>(b).is_ok();
        let first = match self.fs.read(&main) {
            Ok(bytes) if valid(&bytes) => return Ok((bytes, main)),
            Ok(_) => StoreError::Io(format!("{main}: empty or corrupt")),
            Err(e) => e,
        };
        let tmp = Self::tmp_file(id);
        match self.fs.read(&tmp) {
            Ok(bytes) if valid(&bytes) => Ok((bytes, tmp)),
            _ => Err(first),
        }
    }

    fn summary(&mut self, id: ProjectId) -> Result<ProjectSummary, StoreError> {
        let (bytes, path) = self.read_document(id)?;
        let modified_ms = self.fs.stat(&path)?.map_or(0.0, |e| e.modified_ms);
        // A missing or unreadable `share.json` lists the project as not shared.
        let share = self
            .fs
            .read(&format!("{}/{SHARE_FILE}", Self::dir(id)))
            .ok()
            .and_then(|b| share_info(&b));
        Ok(ProjectSummary {
            id,
            name: name_of(&bytes),
            modified_ms,
            share,
        })
    }

    fn exists(&mut self, id: ProjectId) -> Result<bool, StoreError> {
        Ok(self.fs.stat(&Self::dir(id))?.is_some())
    }

    fn copy_tree(&mut self, from: &str, to: &str, skip: &[&str]) -> Result<(), StoreError> {
        self.fs.mkdir(to)?;
        for e in self.fs.list(from)? {
            if skip.contains(&e.name.as_str()) {
                continue;
            }
            let (src, dst) = (format!("{from}/{}", e.name), format!("{to}/{}", e.name));
            if e.is_dir {
                self.copy_tree(&src, &dst, &[])?;
            } else {
                let bytes = self.fs.read(&src)?;
                self.fs.write(&dst, &bytes)?;
            }
        }
        Ok(())
    }
}

impl<F: Fs> ProjectStore for WebStore<F> {
    fn list(&mut self) -> Result<Vec<ProjectSummary>, StoreError> {
        let entries = match self.fs.list(PROJECTS_ROOT) {
            Ok(e) => e,
            Err(StoreError::NotFound(_)) => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut out = Vec::new();
        for e in entries.into_iter().filter(|e| e.is_dir) {
            // Skip folders that aren't projects (no valid id or no document yet).
            let Ok(id) = e.name.parse::<ProjectId>() else {
                continue;
            };
            match self.summary(id) {
                Ok(s) => out.push(s),
                Err(StoreError::NotFound(_)) => {}
                Err(err) => return Err(err),
            }
        }
        out.sort_by(|a, b| b.modified_ms.total_cmp(&a.modified_ms));
        Ok(out)
    }

    fn create(&mut self, id: ProjectId) -> Result<(), StoreError> {
        if self.exists(id)? {
            return Err(StoreError::AlreadyExists(id.to_string()));
        }
        let dir = Self::dir(id);
        self.fs.mkdir(&format!("{dir}/{MEDIA_DIR}"))?;
        self.fs.mkdir(&format!("{dir}/{CACHE_DIR}"))
    }

    fn load(&mut self, id: ProjectId) -> Result<String, StoreError> {
        let (bytes, _) = self.read_document(id)?;
        String::from_utf8(bytes).map_err(|e| StoreError::Io(e.to_string()))
    }

    fn save(&mut self, id: ProjectId, ether_json: &str) -> Result<ProjectSummary, StoreError> {
        if !self.exists(id)? {
            return Err(StoreError::NotFound(id.to_string()));
        }
        let tmp = Self::tmp_file(id);
        self.fs.write(&tmp, ether_json.as_bytes())?;
        self.fs
            .rename(&tmp, &format!("{}/{PROJECT_FILE}", Self::dir(id)))?;
        self.summary(id)
    }

    fn duplicate(&mut self, from: ProjectId, to: ProjectId) -> Result<(), StoreError> {
        if !self.exists(from)? {
            return Err(StoreError::NotFound(from.to_string()));
        }
        if self.exists(to)? {
            return Err(StoreError::AlreadyExists(to.to_string()));
        }
        // The cache is regenerable: copy an empty one. A copy is private: never copy the
        // sharing state (and its secrets).
        self.copy_tree(&Self::dir(from), &Self::dir(to), &[CACHE_DIR, SHARE_FILE])?;
        self.fs.mkdir(&format!("{}/{CACHE_DIR}", Self::dir(to)))
    }

    fn delete(&mut self, id: ProjectId) -> Result<(), StoreError> {
        self.fs.remove(&Self::dir(id))
    }

    fn read(&mut self, id: ProjectId, rel_path: &str) -> Result<Vec<u8>, StoreError> {
        let rel = relative(rel_path)?;
        if rel.is_empty() {
            return Err(StoreError::InvalidPath(rel_path.to_string()));
        }
        self.fs.read(&join(&Self::dir(id), rel))
    }

    fn write(&mut self, id: ProjectId, rel_path: &str, bytes: &[u8]) -> Result<(), StoreError> {
        let rel = relative(rel_path)?;
        if rel.is_empty() {
            return Err(StoreError::InvalidPath(rel_path.to_string()));
        }
        if !self.exists(id)? {
            return Err(StoreError::NotFound(id.to_string()));
        }
        self.fs.write(&join(&Self::dir(id), rel), bytes)
    }

    /// v0.3 (`project-versions`): delete one file (never a folder); missing = `Ok`.
    fn remove(&mut self, id: ProjectId, rel_path: &str) -> Result<(), StoreError> {
        let rel = relative(rel_path)?;
        if rel.is_empty() {
            return Err(StoreError::InvalidPath(rel_path.to_string()));
        }
        let path = join(&Self::dir(id), rel);
        match self.fs.stat(&path)? {
            Some(e) if e.is_dir => Err(StoreError::InvalidPath(format!("{rel_path} is a folder"))),
            Some(_) => match self.fs.remove(&path) {
                Err(StoreError::NotFound(_)) => Ok(()),
                r => r,
            },
            None => Ok(()),
        }
    }

    fn list_dir(&mut self, id: ProjectId, rel_path: &str) -> Result<DirectoryListing, StoreError> {
        listing(
            &mut self.fs,
            &Self::dir(id),
            rel_path,
            BrowseLocation::ProjectMedia,
        )
    }

    // ─── Upload staging (`file-import`: the local web build imports OS files too) ───────

    fn begin_upload(&mut self, upload: &str, size: u64) -> Result<(), StoreError> {
        let dir = Self::upload_dir(upload)?;
        if self.uploads.is_empty() {
            // Nothing staged in this session: drop leftovers of a closed tab.
            match self.fs.remove(UPLOADS_ROOT) {
                Ok(()) | Err(StoreError::NotFound(_)) => {}
                Err(e) => return Err(e),
            }
        } else {
            self.discard_upload(upload)?;
        }
        self.fs.mkdir(&dir)?;
        self.uploads
            .insert(upload.to_string(), Staged { size, received: 0 });
        Ok(())
    }

    fn append_upload(
        &mut self,
        upload: &str,
        offset: u64,
        bytes: &[u8],
    ) -> Result<u64, StoreError> {
        let dir = Self::upload_dir(upload)?;
        let s = *self
            .uploads
            .get(upload)
            .ok_or_else(|| StoreError::NotFound(format!("upload {upload}")))?;
        if offset != s.received {
            return Err(StoreError::Io(format!(
                "upload {upload}: chunk at {offset}, expected {}",
                s.received
            )));
        }
        let total = s.received + bytes.len() as u64;
        if total > s.size {
            return Err(StoreError::Io(format!(
                "upload {upload}: {total} bytes exceed the announced {}",
                s.size
            )));
        }
        if !bytes.is_empty() {
            // Zero-padded so the names sort in offset order.
            self.fs.write(&format!("{dir}/{offset:016}"), bytes)?;
        }
        self.uploads.get_mut(upload).expect("checked").received = total;
        Ok(total)
    }

    fn read_upload(&mut self, upload: &str) -> Result<Vec<u8>, StoreError> {
        let dir = Self::upload_dir(upload)?;
        let s = *self
            .uploads
            .get(upload)
            .ok_or_else(|| StoreError::NotFound(format!("upload {upload}")))?;
        if s.received != s.size {
            return Err(StoreError::Io(format!(
                "upload {upload} is incomplete ({} of {} bytes)",
                s.received, s.size
            )));
        }
        let mut parts = self.fs.list(&dir)?;
        parts.sort_by(|a, b| a.name.cmp(&b.name));
        let mut out = Vec::with_capacity(s.size as usize);
        for p in parts.iter().filter(|p| !p.is_dir) {
            out.extend_from_slice(&self.fs.read(&format!("{dir}/{}", p.name))?);
        }
        if out.len() as u64 != s.size {
            return Err(StoreError::Io(format!(
                "upload {upload}: staging is corrupt"
            )));
        }
        Ok(out)
    }

    fn discard_upload(&mut self, upload: &str) -> Result<(), StoreError> {
        let dir = Self::upload_dir(upload)?;
        self.uploads.remove(upload);
        match self.fs.remove(&dir) {
            Ok(()) | Err(StoreError::NotFound(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }
}

/// Folder of the generated demo samples inside the library.
pub const DEMO_SAMPLES_DIR: &str = "Demo Samples";

/// Write the demo samples ([`ether_media::demo::demo_samples`]) into
/// `library/Demo Samples/` (files that exist are left alone).
pub fn ensure_demo_samples<F: Fs>(fs: &mut F) -> Result<(), StoreError> {
    let dir = format!("{LIBRARY_ROOT}/{DEMO_SAMPLES_DIR}");
    for (name, bytes) in ether_media::demo::demo_samples() {
        let path = format!("{dir}/{name}");
        if fs.stat(&path)?.is_none() {
            fs.write(&path, &bytes)?;
        }
    }
    Ok(())
}

/// The browser's sample library: one OPFS folder (`library/`) holding the generated demo
/// samples ([`ensure_demo_samples`]), the writable user library, and (`base-136`) the
/// folders imported from the user's computer (`imported/<folder>/`, user folders added by
/// the controller with their OPFS path; the controller remembers them).
pub struct WebLibrary<F: Fs> {
    fs: F,
    /// Imported folders added as roots: (root id, OPFS folder `imported/<folder>`).
    imported: Vec<(String, String)>,
}

impl<F: Fs> WebLibrary<F> {
    pub fn new(fs: F) -> Self {
        Self {
            fs,
            imported: Vec::new(),
        }
    }

    fn base(&self, root: &str) -> Result<String, StoreError> {
        match root {
            LIBRARY_ID => Ok(LIBRARY_ROOT.to_string()),
            USER_LIBRARY_ID => Ok(USER_LIBRARY_ROOT.to_string()),
            _ => self
                .imported
                .iter()
                .find(|(id, _)| id == root)
                .map(|(_, dir)| dir.clone())
                .ok_or_else(|| StoreError::NotFound(format!("library root {root}"))),
        }
    }

    /// A writable root's folder (the user library or an imported folder) + a checked,
    /// non-empty relative path in it.
    fn user_path(&self, root: &str, rel_path: &str) -> Result<String, StoreError> {
        let base = match root {
            USER_LIBRARY_ID => USER_LIBRARY_ROOT.to_string(),
            _ => match self.imported.iter().find(|(id, _)| id == root) {
                Some((_, dir)) => dir.clone(),
                None => {
                    return Err(StoreError::Unsupported(format!(
                        "library {root} is read-only"
                    )));
                }
            },
        };
        let rel = relative(rel_path)?;
        if rel.is_empty() {
            return Err(StoreError::InvalidPath(rel_path.to_string()));
        }
        Ok(join(&base, rel))
    }

    /// `imported/<folder>` with a single valid folder segment, else `None`.
    fn imported_folder(path: &str) -> Option<&str> {
        let folder = path.strip_prefix(IMPORTED_ROOT)?.strip_prefix('/')?;
        let valid = check_relative_path(folder).is_ok()
            && !folder.is_empty()
            && !folder.contains('/')
            && !folder.starts_with('.');
        valid.then_some(folder)
    }
}

impl<F: Fs> Library for WebLibrary<F> {
    fn roots(&self) -> Vec<BrowseRoot> {
        let mut roots = vec![BrowseRoot {
            location: BrowseLocation::Library {
                id: LIBRARY_ID.to_string(),
            },
            name: "Browser library".to_string(),
        }];
        roots.extend(self.imported.iter().map(|(id, dir)| BrowseRoot {
            location: BrowseLocation::Library { id: id.clone() },
            name: dir.rsplit('/').next().unwrap_or(dir).to_string(),
        }));
        roots
    }

    fn list_dir(&mut self, root: &str, rel_path: &str) -> Result<DirectoryListing, StoreError> {
        let base = self.base(root)?;
        let location = BrowseLocation::Library {
            id: root.to_string(),
        };
        match listing(&mut self.fs, &base, rel_path, location.clone()) {
            // A fresh browser profile has no library folder yet: show it empty.
            Err(StoreError::NotFound(_)) if relative(rel_path)?.is_empty() => {
                Ok(DirectoryListing {
                    location,
                    path: String::new(),
                    entries: Vec::new(),
                })
            }
            r => r,
        }
    }

    fn read(&mut self, root: &str, rel_path: &str) -> Result<Vec<u8>, StoreError> {
        let base = self.base(root)?;
        let rel = relative(rel_path)?;
        if rel.is_empty() {
            return Err(StoreError::InvalidPath(rel_path.to_string()));
        }
        self.fs.read(&join(&base, rel))
    }

    /// v0.2 (`presets`): only the user library (`user-library/`) is writable.
    fn write_file(&mut self, root: &str, rel_path: &str, bytes: &[u8]) -> Result<(), StoreError> {
        let path = self.user_path(root, rel_path)?;
        if self.fs.stat(&path)?.is_some_and(|e| e.is_dir) {
            return Err(StoreError::InvalidPath(format!("{rel_path} is a folder")));
        }
        self.fs.write(&path, bytes)
    }

    /// v0.2 (`presets`): delete a file of the user library (never a folder).
    fn remove_file(&mut self, root: &str, rel_path: &str) -> Result<(), StoreError> {
        let path = self.user_path(root, rel_path)?;
        match self.fs.stat(&path)? {
            Some(e) if !e.is_dir => self.fs.remove(&path),
            Some(_) => Err(StoreError::InvalidPath(format!("not a file: {rel_path}"))),
            None => Err(StoreError::NotFound(rel_path.to_string())),
        }
    }

    /// v0.2 (`presets`): rename/move a file inside the user library; the target must not
    /// exist (OPFS names are case-sensitive).
    fn rename_file(&mut self, root: &str, from: &str, to: &str) -> Result<(), StoreError> {
        let src = self.user_path(root, from)?;
        let dst = self.user_path(root, to)?;
        if !self.fs.stat(&src)?.is_some_and(|e| !e.is_dir) {
            return Err(StoreError::NotFound(from.to_string()));
        }
        if src == dst {
            return Ok(());
        }
        if self.fs.stat(&dst)?.is_some() {
            return Err(StoreError::AlreadyExists(to.to_string()));
        }
        self.fs.rename(&src, &dst)
    }

    fn user_root(&self) -> Option<String> {
        Some(USER_LIBRARY_ID.to_string())
    }

    /// `base-136`: only imported folders (`imported/<folder>`, existing) can be added: the
    /// web has no other folders. Id: [`user_folder_id`] of the OPFS path.
    fn add_folder(&mut self, path: &str) -> Result<String, StoreError> {
        if Self::imported_folder(path).is_none() {
            return Err(StoreError::Unsupported(
                "the web app can only import folders (Import folder…)".into(),
            ));
        }
        match self.fs.stat(path)? {
            Some(e) if e.is_dir => {}
            Some(_) => return Err(StoreError::InvalidPath(format!("not a folder: {path}"))),
            None => return Err(StoreError::NotFound(path.to_string())),
        }
        let id = user_folder_id(path);
        if !self.imported.iter().any(|(i, _)| *i == id) {
            self.imported.push((id.clone(), path.to_string()));
        }
        Ok(id)
    }

    /// `base-136`: forget an imported folder and delete its copy.
    fn remove_folder(&mut self, root: &str) -> Result<(), StoreError> {
        if !root.starts_with(USER_FOLDER_PREFIX) {
            return Err(StoreError::InvalidPath(format!(
                "{root} is not a user folder"
            )));
        }
        let Some(i) = self.imported.iter().position(|(id, _)| id == root) else {
            return Err(StoreError::NotFound(root.to_string()));
        };
        let (_, dir) = self.imported.remove(i);
        match self.fs.remove(&dir) {
            Ok(()) | Err(StoreError::NotFound(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// `base-136`: `imported/<name>` (de-duplicated, case-insensitively), created empty.
    fn create_import_folder(&mut self, name: &str) -> Result<String, StoreError> {
        let base = import_folder_name(name)
            .ok_or_else(|| StoreError::InvalidPath(format!("bad folder name {name:?}")))?;
        let existing: Vec<String> = match self.fs.list(IMPORTED_ROOT) {
            Ok(entries) => entries.into_iter().map(|e| e.name.to_lowercase()).collect(),
            Err(StoreError::NotFound(_)) => Vec::new(),
            Err(e) => return Err(e),
        };
        let name = unique_folder_name(&base, |c| existing.contains(&c.to_lowercase()));
        let path = join(IMPORTED_ROOT, &name);
        self.fs.mkdir(&path)?;
        Ok(path)
    }
}

/// In-memory [`Fs`] (OPFS fake for tests). Clones share the same tree.
#[derive(Clone, Default)]
pub struct MemFs {
    inner: std::rc::Rc<std::cell::RefCell<MemTree>>,
}

#[derive(Default)]
struct MemTree {
    files: BTreeMap<String, (Vec<u8>, f64)>,
    dirs: std::collections::BTreeSet<String>,
    clock: f64,
}

impl MemFs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the modification time used for subsequent writes.
    pub fn set_clock(&self, ms: f64) {
        self.inner.borrow_mut().clock = ms;
    }

    /// All file paths (for assertions).
    pub fn files(&self) -> Vec<String> {
        self.inner.borrow().files.keys().cloned().collect()
    }
}

impl MemTree {
    fn mkdirs(&mut self, path: &str) {
        let mut acc = String::new();
        for c in path.split('/').filter(|c| !c.is_empty()) {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(c);
            self.dirs.insert(acc.clone());
        }
    }

    fn parent(path: &str) -> &str {
        path.rsplit_once('/').map_or("", |(p, _)| p)
    }
}

impl Fs for MemFs {
    fn read(&mut self, path: &str) -> Result<Vec<u8>, StoreError> {
        self.inner
            .borrow()
            .files
            .get(path)
            .map(|(b, _)| b.clone())
            .ok_or_else(|| StoreError::NotFound(path.to_string()))
    }

    fn write(&mut self, path: &str, bytes: &[u8]) -> Result<(), StoreError> {
        let mut t = self.inner.borrow_mut();
        if t.dirs.contains(path) {
            return Err(StoreError::Io(format!("{path} is a directory")));
        }
        t.mkdirs(MemTree::parent(path));
        let clock = t.clock;
        t.files.insert(path.to_string(), (bytes.to_vec(), clock));
        Ok(())
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), StoreError> {
        let mut t = self.inner.borrow_mut();
        let f = t
            .files
            .remove(from)
            .ok_or_else(|| StoreError::NotFound(from.to_string()))?;
        t.mkdirs(MemTree::parent(to));
        t.files.insert(to.to_string(), f);
        Ok(())
    }

    fn list(&mut self, dir: &str) -> Result<Vec<FsEntry>, StoreError> {
        let t = self.inner.borrow();
        if !dir.is_empty() && !t.dirs.contains(dir) {
            return Err(StoreError::NotFound(dir.to_string()));
        }
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        let direct = |p: &String| {
            p.strip_prefix(&prefix)
                .filter(|rest| !rest.contains('/'))
                .map(str::to_string)
        };
        let mut out: Vec<FsEntry> = t
            .dirs
            .iter()
            .filter_map(direct)
            .map(|name| FsEntry {
                name,
                is_dir: true,
                size: 0,
                modified_ms: 0.0,
            })
            .collect();
        for (p, (bytes, m)) in &t.files {
            if let Some(name) = direct(p) {
                out.push(FsEntry {
                    name,
                    is_dir: false,
                    size: bytes.len() as u64,
                    modified_ms: *m,
                });
            }
        }
        Ok(out)
    }

    fn mkdir(&mut self, path: &str) -> Result<(), StoreError> {
        let mut t = self.inner.borrow_mut();
        if t.files.contains_key(path) {
            return Err(StoreError::Io(format!("{path} is a file")));
        }
        t.mkdirs(path);
        Ok(())
    }

    fn remove(&mut self, path: &str) -> Result<(), StoreError> {
        let mut t = self.inner.borrow_mut();
        if t.files.remove(path).is_some() {
            return Ok(());
        }
        if !t.dirs.remove(path) {
            return Err(StoreError::NotFound(path.to_string()));
        }
        let prefix = format!("{path}/");
        t.dirs.retain(|d| !d.starts_with(&prefix));
        t.files.retain(|f, _| !f.starts_with(&prefix));
        Ok(())
    }

    fn stat(&mut self, path: &str) -> Result<Option<FsEntry>, StoreError> {
        let t = self.inner.borrow();
        let name = path.rsplit('/').next().unwrap_or(path).to_string();
        if let Some((bytes, m)) = t.files.get(path) {
            return Ok(Some(FsEntry {
                name,
                is_dir: false,
                size: bytes.len() as u64,
                modified_ms: *m,
            }));
        }
        Ok(t.dirs.contains(path).then_some(FsEntry {
            name,
            is_dir: true,
            size: 0,
            modified_ms: 0.0,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(n: u8) -> ProjectId {
        ProjectId::v7(1_700_000_000_000 + n as u64, [n; 10])
    }

    fn doc(name: &str) -> String {
        serde_json::json!({
            "format": "ethereal-project",
            "version": 1,
            "app_version": "test",
            "project": { "settings": { "name": name } },
        })
        .to_string()
    }

    fn store() -> (MemFs, WebStore<MemFs>) {
        let fs = MemFs::new();
        (fs.clone(), WebStore::new(fs))
    }

    #[test]
    fn create_save_load_list() {
        let (fs, mut s) = store();
        assert_eq!(s.list().unwrap(), vec![]);
        s.create(pid(1)).unwrap();
        s.create(pid(2)).unwrap();
        assert_eq!(
            s.create(pid(1)),
            Err(StoreError::AlreadyExists(pid(1).to_string()))
        );
        // Created but never saved: not listed yet.
        assert_eq!(s.list().unwrap(), vec![]);
        fs.set_clock(10.0);
        let a = s.save(pid(1), &doc("First")).unwrap();
        assert_eq!((a.name.as_str(), a.modified_ms), ("First", 10.0));
        fs.set_clock(20.0);
        s.save(pid(2), &doc("Second")).unwrap();
        let names: Vec<_> = s.list().unwrap().into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["Second", "First"], "newest first");
        assert_eq!(s.load(pid(1)).unwrap(), doc("First"));
        let dir = format!("projects/{}", pid(1));
        assert!(fs.files().contains(&format!("{dir}/project.ether")));
        assert!(!fs.files().iter().any(|f| f.ends_with(".tmp")));
        let mut fs2 = fs.clone();
        assert!(fs2.stat(&format!("{dir}/media")).unwrap().unwrap().is_dir);
        assert!(fs2.stat(&format!("{dir}/cache")).unwrap().unwrap().is_dir);
    }

    #[test]
    fn load_recovers_from_an_interrupted_save() {
        let (mut fs, mut s) = store();
        s.create(pid(1)).unwrap();
        s.save(pid(1), &doc("Saved")).unwrap();
        let main = format!("projects/{}/project.ether", pid(1));
        let tmp = format!("projects/{}/.project.ether.tmp", pid(1));

        // Crash after the temp file was written, while project.ether was being replaced:
        // truncated, emptied or already removed.
        for broken in [Some(&b"{\"format\": \"ethe"[..]), Some(&b""[..]), None] {
            fs.write(&tmp, doc("Newer").as_bytes()).unwrap();
            match broken {
                Some(b) => fs.write(&main, b).unwrap(),
                None => fs.remove(&main).unwrap(),
            }
            assert_eq!(s.load(pid(1)).unwrap(), doc("Newer"), "{broken:?}");
            assert_eq!(s.list().unwrap()[0].name, "Newer");
            // The next save repairs the folder.
            s.save(pid(1), &doc("Repaired")).unwrap();
            assert_eq!(s.load(pid(1)).unwrap(), doc("Repaired"));
            assert!(fs.stat(&tmp).unwrap().is_none());
        }

        // A stale temp file never shadows a valid document.
        fs.write(&tmp, doc("Stale").as_bytes()).unwrap();
        assert_eq!(s.load(pid(1)).unwrap(), doc("Repaired"));

        // Both unusable: the original error.
        fs.write(&main, b"").unwrap();
        fs.write(&tmp, b"garbage").unwrap();
        assert!(matches!(s.load(pid(1)), Err(StoreError::Io(_))));
    }

    #[test]
    fn save_requires_existing_project_and_replaces() {
        let (_, mut s) = store();
        assert!(matches!(
            s.save(pid(1), &doc("x")),
            Err(StoreError::NotFound(_))
        ));
        s.create(pid(1)).unwrap();
        s.save(pid(1), &doc("a")).unwrap();
        s.save(pid(1), &doc("b")).unwrap();
        assert_eq!(s.list().unwrap()[0].name, "b");
    }

    #[test]
    fn unnamed_or_foreign_folders() {
        let (mut fs, mut s) = store();
        s.create(pid(1)).unwrap();
        s.save(pid(1), "{\"garbage\": true}").unwrap();
        fs.mkdir("projects/not-a-uuid").unwrap();
        fs.write("projects/stray.txt", b"x").unwrap();
        let list = s.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Untitled");
    }

    #[test]
    fn media_files_and_path_validation() {
        let (_, mut s) = store();
        s.create(pid(1)).unwrap();
        s.write(pid(1), "media/kick.wav", b"RIFF").unwrap();
        s.write(pid(1), "media/sub/snare.flac", b"fLaC").unwrap();
        assert_eq!(s.read(pid(1), "media/kick.wav").unwrap(), b"RIFF");
        for bad in [
            "../x",
            "/etc/passwd",
            "media/../../x",
            "media/./a",
            "a//b",
            "c:\\x",
            "",
        ] {
            assert!(
                matches!(s.read(pid(1), bad), Err(StoreError::InvalidPath(_))),
                "{bad}"
            );
            assert!(
                matches!(s.write(pid(1), bad, b""), Err(StoreError::InvalidPath(_))),
                "{bad}"
            );
        }
        assert!(matches!(
            s.write(pid(9), "media/a.wav", b""),
            Err(StoreError::NotFound(_))
        ));
        let l = s.list_dir(pid(1), "media").unwrap();
        assert_eq!(l.location, BrowseLocation::ProjectMedia);
        let got: Vec<_> = l
            .entries
            .iter()
            .map(|e| (e.path.as_str(), e.kind, e.size))
            .collect();
        assert_eq!(
            got,
            [
                ("media/sub", FileKind::Directory, 0.0),
                ("media/kick.wav", FileKind::Audio, 4.0)
            ]
        );
        assert!(s.list_dir(pid(1), "..").is_err());
    }

    #[test]
    fn duplicate_copies_document_and_media_not_cache() {
        let (fs, mut s) = store();
        s.create(pid(1)).unwrap();
        s.save(pid(1), &doc("Orig")).unwrap();
        s.write(pid(1), "media/a.wav", b"A").unwrap();
        s.write(pid(1), "cache/a.peaks", b"P").unwrap();
        s.duplicate(pid(1), pid(2)).unwrap();
        assert_eq!(s.read(pid(2), "media/a.wav").unwrap(), b"A");
        assert!(s.read(pid(2), "cache/a.peaks").is_err());
        assert_eq!(s.load(pid(2)).unwrap(), doc("Orig"));
        let mut fs2 = fs.clone();
        assert!(
            fs2.stat(&format!("projects/{}/cache", pid(2)))
                .unwrap()
                .is_some()
        );
        assert!(matches!(
            s.duplicate(pid(1), pid(2)),
            Err(StoreError::AlreadyExists(_))
        ));
        assert!(matches!(
            s.duplicate(pid(7), pid(8)),
            Err(StoreError::NotFound(_))
        ));
    }

    /// base-115 (`recents-shared`): `share.json` → `ProjectSummary.share`; never copied by
    /// `duplicate` (SaveAs/Duplicate), deleted with the project.
    #[test]
    fn share_json_summary_duplicate_delete() {
        use ether_controller::store::share_fixtures as fx;
        use ether_core::protocol::share::ParticipantRole;

        let (fs, mut s) = store();
        for (n, name) in [(1, "Song"), (2, "Their song"), (3, "Mine")] {
            s.create(pid(n)).unwrap();
            s.save(pid(n), &doc(name)).unwrap();
        }
        s.write(pid(1), SHARE_FILE, fx::HOST.as_bytes()).unwrap();
        s.write(pid(2), SHARE_FILE, fx::COPY.as_bytes()).unwrap();

        let list = s.list().unwrap();
        let by_id = |n| list.iter().find(|p| p.id == pid(n)).unwrap().clone();
        let h = by_id(1).share.expect("host share");
        assert_eq!(h.role, ParticipantRole::Host);
        assert_eq!(h.participants[0].name, "Tom");
        let c = by_id(2).share.expect("copy share");
        assert_eq!(
            (c.role, c.host_name.as_str(), c.active),
            (ParticipantRole::Edit, "Diego", true)
        );
        assert_eq!(by_id(3).share, None);
        // `save` reports the share info too (the `Saved` event's summary).
        assert!(s.save(pid(2), &doc("Their song")).unwrap().share.is_some());
        let json = serde_json::to_string(&list).unwrap();
        for secret in fx::SECRETS {
            assert!(!json.contains(secret), "{secret} leaked");
        }

        // An ended copy, and an unreadable file (listed as not shared, never hidden).
        s.write(pid(2), SHARE_FILE, fx::COPY_ENDED.as_bytes())
            .unwrap();
        s.write(pid(3), SHARE_FILE, b"{ corrupt").unwrap();
        let list = s.list().unwrap();
        assert_eq!(list.len(), 3);
        let ended = list.iter().find(|p| p.id == pid(2)).unwrap();
        assert!(!ended.share.as_ref().unwrap().active);
        assert_eq!(list.iter().find(|p| p.id == pid(3)).unwrap().share, None);

        // Duplicate (also SaveAs) makes a private project.
        s.write(pid(1), "media/a.wav", b"A").unwrap();
        s.duplicate(pid(1), pid(4)).unwrap();
        assert!(s.read(pid(4), SHARE_FILE).is_err());
        assert_eq!(s.read(pid(4), "media/a.wav").unwrap(), b"A");
        let list = s.list().unwrap();
        assert_eq!(list.iter().find(|p| p.id == pid(4)).unwrap().share, None);

        // Deleting the project deletes its share.json.
        s.delete(pid(1)).unwrap();
        let prefix = format!("projects/{}/", pid(1));
        assert!(fs.files().iter().all(|f| !f.starts_with(&prefix)));
    }

    #[test]
    fn remove_deletes_files_only() {
        let (_fs, mut s) = store();
        s.create(pid(3)).unwrap();
        s.write(pid(3), "versions/.session", b"{}").unwrap();
        s.write(pid(3), "versions/1-manual.ether", b"{}").unwrap();
        s.remove(pid(3), "versions/.session").unwrap();
        assert!(matches!(
            ProjectStore::read(&mut s, pid(3), "versions/.session"),
            Err(StoreError::NotFound(_))
        ));
        s.remove(pid(3), "versions/.session").unwrap();
        assert!(matches!(
            s.remove(pid(3), "versions"),
            Err(StoreError::InvalidPath(_))
        ));
        assert!(matches!(
            s.remove(pid(3), ""),
            Err(StoreError::InvalidPath(_))
        ));
        assert!(matches!(
            s.remove(pid(3), "../x"),
            Err(StoreError::InvalidPath(_))
        ));
        assert!(ProjectStore::read(&mut s, pid(3), "versions/1-manual.ether").is_ok());
    }

    #[test]
    fn delete_removes_folder() {
        let (fs, mut s) = store();
        s.create(pid(1)).unwrap();
        s.save(pid(1), &doc("x")).unwrap();
        s.write(pid(1), "media/a.wav", b"A").unwrap();
        s.delete(pid(1)).unwrap();
        assert!(fs.files().is_empty());
        assert_eq!(s.list().unwrap(), vec![]);
        assert!(matches!(s.delete(pid(1)), Err(StoreError::NotFound(_))));
    }

    #[test]
    fn library_lists_and_reads() {
        let mut fs = MemFs::new();
        let mut lib = WebLibrary::new(fs.clone());
        assert_eq!(lib.roots().len(), 1);
        let empty = lib.list_dir(LIBRARY_ID, "").unwrap();
        assert!(empty.entries.is_empty());
        fs.write("library/drums/kick.wav", b"K").unwrap();
        fs.write("library/readme.txt", b"R").unwrap();
        let root = lib.list_dir(LIBRARY_ID, "").unwrap();
        let names: Vec<_> = root.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["drums", "readme.txt"]);
        assert_eq!(root.entries[1].kind, FileKind::Other);
        // Entry paths are relative to the root (no leading `/`) and can be listed/read back.
        let paths: Vec<_> = root.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, ["drums", "readme.txt"]);
        let drums = lib.list_dir(LIBRARY_ID, &root.entries[0].path).unwrap();
        assert_eq!(drums.entries[0].path, "drums/kick.wav");
        assert_eq!(lib.read(LIBRARY_ID, "drums/kick.wav").unwrap(), b"K");
        assert!(lib.read(LIBRARY_ID, "../projects").is_err());
        assert!(lib.read("other", "drums/kick.wav").is_err());
        assert!(lib.list_dir(LIBRARY_ID, "missing").is_err());
    }

    #[test]
    fn user_library_is_writable() {
        let fs = MemFs::new();
        let mut lib = WebLibrary::new(fs.clone());
        assert_eq!(lib.user_root().as_deref(), Some(USER_LIBRARY_ID));
        assert_eq!(
            lib.roots().len(),
            1,
            "the user library is not a browse root"
        );
        assert!(matches!(
            lib.write_file(LIBRARY_ID, "a.etherpreset", b"x"),
            Err(StoreError::Unsupported(_))
        ));
        assert!(
            lib.list_dir(USER_LIBRARY_ID, "")
                .unwrap()
                .entries
                .is_empty()
        );
        lib.write_file(USER_LIBRARY_ID, "Presets/synth/A.etherpreset", b"a")
            .unwrap();
        assert_eq!(fs.files(), ["user-library/Presets/synth/A.etherpreset"]);
        assert_eq!(
            lib.read(USER_LIBRARY_ID, "Presets/synth/A.etherpreset")
                .unwrap(),
            b"a"
        );
        lib.write_file(USER_LIBRARY_ID, "Presets/synth/B.etherpreset", b"b")
            .unwrap();
        assert!(matches!(
            lib.rename_file(
                USER_LIBRARY_ID,
                "Presets/synth/A.etherpreset",
                "Presets/synth/B.etherpreset"
            ),
            Err(StoreError::AlreadyExists(_))
        ));
        lib.rename_file(
            USER_LIBRARY_ID,
            "Presets/synth/A.etherpreset",
            "Presets/synth/C.etherpreset",
        )
        .unwrap();
        let names: Vec<_> = lib
            .list_dir(USER_LIBRARY_ID, "Presets/synth")
            .unwrap()
            .entries
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["B.etherpreset", "C.etherpreset"]);
        assert!(matches!(
            lib.remove_file(USER_LIBRARY_ID, "Presets"),
            Err(StoreError::InvalidPath(_))
        ));
        lib.remove_file(USER_LIBRARY_ID, "Presets/synth/C.etherpreset")
            .unwrap();
        assert!(matches!(
            lib.remove_file(USER_LIBRARY_ID, "Presets/synth/C.etherpreset"),
            Err(StoreError::NotFound(_))
        ));
        for bad in ["../projects/x", "/x", ""] {
            assert!(lib.write_file(USER_LIBRARY_ID, bad, b"x").is_err(), "{bad}");
        }
    }

    #[test]
    fn demo_samples_land_in_the_library_once() {
        let mut fs = MemFs::new();
        ensure_demo_samples(&mut fs).unwrap();
        let mut lib = WebLibrary::new(fs.clone());
        let demo = lib.list_dir(LIBRARY_ID, DEMO_SAMPLES_DIR).unwrap();
        assert_eq!(demo.entries.len(), 5);
        assert!(demo.entries.iter().all(|e| e.kind == FileKind::Audio));
        // An existing file is kept.
        fs.write("library/Demo Samples/Kick.wav", b"mine").unwrap();
        ensure_demo_samples(&mut fs).unwrap();
        assert_eq!(fs.read("library/Demo Samples/Kick.wav").unwrap(), b"mine");
    }
}

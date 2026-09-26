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
//! ```

use std::collections::BTreeMap;

use ether_controller::store::{Library, ProjectStore, StoreError, check_relative_path, file_kind};
use ether_core::protocol::media::{
    BrowseLocation, BrowseRoot, DirectoryEntry, DirectoryListing, FileKind,
};
use ether_core::protocol::model::ProjectId;
use ether_core::protocol::project::ProjectSummary;

pub const PROJECTS_ROOT: &str = "projects";
pub const LIBRARY_ROOT: &str = "library";
pub const LIBRARY_ID: &str = "browser";
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

/// `ProjectStore` over an [`Fs`] (OPFS on the web).
pub struct WebStore<F: Fs> {
    fs: F,
}

impl<F: Fs> WebStore<F> {
    pub fn new(fs: F) -> Self {
        Self { fs }
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
        Ok(ProjectSummary {
            id,
            name: name_of(&bytes),
            modified_ms,
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
        // The cache is regenerable: copy an empty one.
        self.copy_tree(&Self::dir(from), &Self::dir(to), &[CACHE_DIR])?;
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

    fn list_dir(&mut self, id: ProjectId, rel_path: &str) -> Result<DirectoryListing, StoreError> {
        listing(
            &mut self.fs,
            &Self::dir(id),
            rel_path,
            BrowseLocation::ProjectMedia,
        )
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

/// The browser's sample library: one OPFS folder (`library/`). There is no way to add
/// files to it from the UI in v0.1 (uploads are reserved in the protocol); it holds the
/// generated demo samples ([`ensure_demo_samples`]).
pub struct WebLibrary<F: Fs> {
    fs: F,
}

impl<F: Fs> WebLibrary<F> {
    pub fn new(fs: F) -> Self {
        Self { fs }
    }

    fn base(&self, root: &str) -> Result<&'static str, StoreError> {
        if root == LIBRARY_ID {
            Ok(LIBRARY_ROOT)
        } else {
            Err(StoreError::NotFound(format!("library root {root}")))
        }
    }
}

impl<F: Fs> Library for WebLibrary<F> {
    fn roots(&self) -> Vec<BrowseRoot> {
        vec![BrowseRoot {
            location: BrowseLocation::Library {
                id: LIBRARY_ID.to_string(),
            },
            name: "Browser library".to_string(),
        }]
    }

    fn list_dir(&mut self, root: &str, rel_path: &str) -> Result<DirectoryListing, StoreError> {
        let base = self.base(root)?;
        let location = BrowseLocation::Library {
            id: root.to_string(),
        };
        match listing(&mut self.fs, base, rel_path, location.clone()) {
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
        self.fs.read(&join(base, rel))
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
        assert_eq!(lib.read(LIBRARY_ID, "drums/kick.wav").unwrap(), b"K");
        assert!(lib.read(LIBRARY_ID, "../projects").is_err());
        assert!(lib.read("other", "drums/kick.wav").is_err());
        assert!(lib.list_dir(LIBRARY_ID, "missing").is_err());
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

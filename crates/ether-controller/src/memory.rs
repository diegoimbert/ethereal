//! In-memory [`ProjectStore`] and [`Library`]: for tests, and as a reference for host
//! implementations (same path rules and listing conventions as the disk store).

use std::collections::BTreeMap;

use ether_core::protocol::media::{
    BrowseLocation, BrowseRoot, DirectoryEntry, DirectoryListing, FileKind,
};
use ether_core::protocol::model::ProjectId;
use ether_core::protocol::model::file::{CACHE_DIR, MEDIA_DIR, PROJECT_FILE};
use ether_core::protocol::project::ProjectSummary;

use crate::store::{
    Library, ProjectStore, StoreError, USER_FOLDER_PREFIX, check_relative_path, file_kind,
    import_folder_name, unique_folder_name, user_folder_id,
};

/// `base-136`: engine-side path prefix of [`MemoryLibrary`]'s imported folders.
pub const IMPORTED_PREFIX: &str = "imported/";

/// List the direct children of `dir` among `files` (paths relative to the same root).
fn list(files: &BTreeMap<String, Vec<u8>>, dir: &str) -> Vec<DirectoryEntry> {
    let prefix = if dir.is_empty() {
        String::new()
    } else {
        format!("{dir}/")
    };
    let mut out: BTreeMap<String, DirectoryEntry> = BTreeMap::new();
    for (path, bytes) in files {
        let Some(rest) = path.strip_prefix(&prefix) else {
            continue;
        };
        match rest.split_once('/') {
            Some((sub, _)) => {
                out.entry(sub.to_string())
                    .or_insert_with(|| DirectoryEntry {
                        name: sub.to_string(),
                        path: format!("{prefix}{sub}"),
                        kind: FileKind::Directory,
                        size: 0.0,
                    });
            }
            None if !rest.is_empty() => {
                out.insert(
                    rest.to_string(),
                    DirectoryEntry {
                        name: rest.to_string(),
                        path: path.clone(),
                        kind: file_kind(rest),
                        size: bytes.len() as f64,
                    },
                );
            }
            None => {}
        }
    }
    // Directories first, then files, by name.
    let mut v: Vec<DirectoryEntry> = out.into_values().collect();
    v.sort_by(|a, b| {
        (a.kind != FileKind::Directory)
            .cmp(&(b.kind != FileKind::Directory))
            .then(a.name.cmp(&b.name))
    });
    v
}

#[derive(Debug, Clone, Default)]
struct MemProject {
    files: BTreeMap<String, Vec<u8>>,
    modified_ms: u64,
}

/// In-memory project store. `now_ms` is the clock used for `modified_ms` (tests set it).
#[derive(Debug, Clone, Default)]
pub struct MemoryStore {
    projects: BTreeMap<ProjectId, MemProject>,
    pub now_ms: u64,
    /// Upload staging (`base-136`: browser folder imports are tested with it).
    uploads: BTreeMap<String, (u64, Vec<u8>)>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn project(&mut self, id: ProjectId) -> Result<&mut MemProject, StoreError> {
        self.projects
            .get_mut(&id)
            .ok_or_else(|| StoreError::NotFound(id.to_string()))
    }

    /// Raw file access (tests).
    pub fn file(&self, id: ProjectId, rel_path: &str) -> Option<&[u8]> {
        self.projects
            .get(&id)?
            .files
            .get(rel_path)
            .map(Vec::as_slice)
    }

    pub fn contains(&self, id: ProjectId) -> bool {
        self.projects.contains_key(&id)
    }

    fn summary(id: ProjectId, p: &MemProject) -> ProjectSummary {
        let name = p
            .files
            .get(PROJECT_FILE)
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(b).ok())
            .and_then(|v| v["project"]["settings"]["name"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "Untitled".into());
        ProjectSummary {
            id,
            name,
            modified_ms: p.modified_ms as f64,
            // base-115: `recents-shared` reads the project's `share.json`.
            share: None,
        }
    }
}

impl ProjectStore for MemoryStore {
    fn list(&mut self) -> Result<Vec<ProjectSummary>, StoreError> {
        let mut v: Vec<ProjectSummary> = self
            .projects
            .iter()
            .filter(|(_, p)| p.files.contains_key(PROJECT_FILE))
            .map(|(id, p)| Self::summary(*id, p))
            .collect();
        v.sort_by(|a, b| {
            b.modified_ms
                .total_cmp(&a.modified_ms)
                .then(b.id.0.cmp(&a.id.0))
        });
        Ok(v)
    }

    fn create(&mut self, id: ProjectId) -> Result<(), StoreError> {
        if self.projects.contains_key(&id) {
            return Err(StoreError::AlreadyExists(id.to_string()));
        }
        self.projects.insert(
            id,
            MemProject {
                files: BTreeMap::new(),
                modified_ms: self.now_ms,
            },
        );
        Ok(())
    }

    fn load(&mut self, id: ProjectId) -> Result<String, StoreError> {
        let bytes = self
            .project(id)?
            .files
            .get(PROJECT_FILE)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(format!("{id}/{PROJECT_FILE}")))?;
        String::from_utf8(bytes).map_err(|e| StoreError::Io(e.to_string()))
    }

    fn save(&mut self, id: ProjectId, ether_json: &str) -> Result<ProjectSummary, StoreError> {
        let now = self.now_ms;
        let p = self.project(id)?;
        p.files
            .insert(PROJECT_FILE.into(), ether_json.as_bytes().to_vec());
        p.modified_ms = now;
        Ok(Self::summary(id, p))
    }

    fn duplicate(&mut self, from: ProjectId, to: ProjectId) -> Result<(), StoreError> {
        if self.projects.contains_key(&to) {
            return Err(StoreError::AlreadyExists(to.to_string()));
        }
        let mut copy = self.project(from)?.clone();
        copy.files
            .retain(|path, _| !path.starts_with(&format!("{CACHE_DIR}/")));
        copy.modified_ms = self.now_ms;
        self.projects.insert(to, copy);
        Ok(())
    }

    fn delete(&mut self, id: ProjectId) -> Result<(), StoreError> {
        self.projects
            .remove(&id)
            .map(drop)
            .ok_or_else(|| StoreError::NotFound(id.to_string()))
    }

    fn read(&mut self, id: ProjectId, rel_path: &str) -> Result<Vec<u8>, StoreError> {
        check_relative_path(rel_path)?;
        self.project(id)?
            .files
            .get(rel_path)
            .cloned()
            .ok_or_else(|| StoreError::NotFound(rel_path.to_string()))
    }

    fn write(&mut self, id: ProjectId, rel_path: &str, bytes: &[u8]) -> Result<(), StoreError> {
        check_relative_path(rel_path)?;
        if rel_path.is_empty() {
            return Err(StoreError::InvalidPath(rel_path.into()));
        }
        self.project(id)?
            .files
            .insert(rel_path.to_string(), bytes.to_vec());
        Ok(())
    }

    fn write_export(
        &mut self,
        id: ProjectId,
        file_name: &str,
        bytes: &[u8],
    ) -> Result<String, StoreError> {
        let path = crate::store::export_path(file_name)?;
        self.write(id, &path, bytes)?;
        Ok(path)
    }

    fn begin_upload(&mut self, upload: &str, size: u64) -> Result<(), StoreError> {
        self.uploads.insert(upload.to_string(), (size, Vec::new()));
        Ok(())
    }

    fn append_upload(
        &mut self,
        upload: &str,
        offset: u64,
        bytes: &[u8],
    ) -> Result<u64, StoreError> {
        let (size, data) = self
            .uploads
            .get_mut(upload)
            .ok_or_else(|| StoreError::NotFound(format!("upload {upload}")))?;
        if offset != data.len() as u64 || offset + bytes.len() as u64 > *size {
            return Err(StoreError::Io(format!("upload {upload}: bad chunk")));
        }
        data.extend_from_slice(bytes);
        Ok(data.len() as u64)
    }

    fn read_upload(&mut self, upload: &str) -> Result<Vec<u8>, StoreError> {
        match self.uploads.get(upload) {
            Some((size, data)) if *size == data.len() as u64 => Ok(data.clone()),
            Some(_) => Err(StoreError::Io(format!("upload {upload} is incomplete"))),
            None => Err(StoreError::NotFound(format!("upload {upload}"))),
        }
    }

    fn discard_upload(&mut self, upload: &str) -> Result<(), StoreError> {
        self.uploads.remove(upload);
        Ok(())
    }

    /// v0.3 (`project-versions`): delete one file; missing = `Ok`.
    fn remove(&mut self, id: ProjectId, rel_path: &str) -> Result<(), StoreError> {
        check_relative_path(rel_path)?;
        if rel_path.is_empty() {
            return Err(StoreError::InvalidPath(rel_path.into()));
        }
        let files = &mut self.project(id)?.files;
        let prefix = format!("{rel_path}/");
        if files.keys().any(|k| k.starts_with(&prefix)) {
            return Err(StoreError::InvalidPath(format!("{rel_path} is a folder")));
        }
        files.remove(rel_path);
        Ok(())
    }

    fn list_dir(&mut self, id: ProjectId, rel_path: &str) -> Result<DirectoryListing, StoreError> {
        check_relative_path(rel_path)?;
        let entries = list(&self.project(id)?.files, rel_path);
        Ok(DirectoryListing {
            location: BrowseLocation::ProjectMedia,
            path: rel_path.to_string(),
            entries,
        })
    }
}

/// In-memory sample library: roots of files keyed by relative path, plus an optional
/// writable user library (v0.2, [`MemoryLibrary::with_user_root`]).
#[derive(Debug, Clone, Default)]
pub struct MemoryLibrary {
    roots: BTreeMap<String, (String, BTreeMap<String, Vec<u8>>)>,
    /// Writable user root id (`Library::user_root`); not listed in `roots()`.
    user: Option<String>,
    /// `base-136`: imported folders' files by engine-side path (`imported/<name>`), whether
    /// added as a root or not (the "disk").
    imported: BTreeMap<String, BTreeMap<String, Vec<u8>>>,
    /// Imported folders added as roots (`add_folder`): root id → path.
    added: BTreeMap<String, String>,
}

impl MemoryLibrary {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a root `id` named `name`.
    pub fn add_root(&mut self, id: &str, name: &str) {
        self.roots
            .entry(id.to_string())
            .or_insert_with(|| (name.to_string(), BTreeMap::new()));
    }

    /// v0.2 (`presets`): a library with a writable user root `id` (presets live under its
    /// `Presets/` folder).
    pub fn with_user_root(mut self, id: &str) -> Self {
        self.add_root(id, "User Library");
        self.user = Some(id.to_string());
        self
    }

    /// The files of root `id` (for assertions).
    pub fn files(&self, id: &str) -> Vec<String> {
        self.root_files(id)
            .map(|f| f.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// `base-136`: the same files, with no user folder added (a host restart: the
    /// controller re-adds them from its index).
    pub fn restarted(&self) -> Self {
        Self {
            added: BTreeMap::new(),
            ..self.clone()
        }
    }

    /// `base-136`: the imported folders' paths (for assertions).
    pub fn imported_folders(&self) -> Vec<String> {
        self.imported.keys().cloned().collect()
    }

    fn root_files(&self, root: &str) -> Option<&BTreeMap<String, Vec<u8>>> {
        match self.added.get(root) {
            Some(path) => self.imported.get(path),
            None => self.roots.get(root).map(|(_, f)| f),
        }
    }

    fn user_files(
        &mut self,
        root: &str,
        rel_path: &str,
    ) -> Result<&mut BTreeMap<String, Vec<u8>>, StoreError> {
        let imported = self.added.get(root).cloned();
        if self.user.as_deref() != Some(root) && imported.is_none() {
            return Err(StoreError::Unsupported(format!(
                "library {root} is read-only"
            )));
        }
        check_relative_path(rel_path)?;
        if rel_path.is_empty() {
            return Err(StoreError::InvalidPath(rel_path.to_string()));
        }
        match imported {
            Some(path) => Ok(self.imported.entry(path).or_default()),
            None => Ok(&mut self.roots.get_mut(root).expect("user root exists").1),
        }
    }

    /// Add a file under root `id` (created if needed).
    pub fn add_file(&mut self, id: &str, path: &str, bytes: Vec<u8>) {
        self.add_root(id, id);
        self.roots
            .get_mut(id)
            .expect("added")
            .1
            .insert(path.to_string(), bytes);
    }
}

impl Library for MemoryLibrary {
    fn roots(&self) -> Vec<BrowseRoot> {
        self.roots
            .iter()
            .filter(|(id, _)| self.user.as_deref() != Some(id.as_str()))
            .map(|(id, (name, _))| BrowseRoot {
                location: BrowseLocation::Library { id: id.clone() },
                name: name.clone(),
            })
            .chain(self.added.iter().map(|(id, path)| BrowseRoot {
                location: BrowseLocation::Library { id: id.clone() },
                name: path[IMPORTED_PREFIX.len()..].to_string(),
            }))
            .collect()
    }

    fn list_dir(&mut self, root: &str, rel_path: &str) -> Result<DirectoryListing, StoreError> {
        check_relative_path(rel_path)?;
        let files = self
            .root_files(root)
            .ok_or_else(|| StoreError::NotFound(root.to_string()))?;
        Ok(DirectoryListing {
            location: BrowseLocation::Library {
                id: root.to_string(),
            },
            path: rel_path.to_string(),
            entries: list(files, rel_path),
        })
    }

    fn read(&mut self, root: &str, rel_path: &str) -> Result<Vec<u8>, StoreError> {
        check_relative_path(rel_path)?;
        self.root_files(root)
            .and_then(|f| f.get(rel_path))
            .cloned()
            .ok_or_else(|| StoreError::NotFound(format!("{root}/{rel_path}")))
    }

    fn write_file(&mut self, root: &str, rel_path: &str, bytes: &[u8]) -> Result<(), StoreError> {
        let files = self.user_files(root, rel_path)?;
        let dir = format!("{rel_path}/");
        if files.keys().any(|k| k.starts_with(&dir)) {
            return Err(StoreError::InvalidPath(format!("{rel_path} is a folder")));
        }
        files.insert(rel_path.to_string(), bytes.to_vec());
        Ok(())
    }

    fn remove_file(&mut self, root: &str, rel_path: &str) -> Result<(), StoreError> {
        let files = self.user_files(root, rel_path)?;
        match files.remove(rel_path) {
            Some(_) => Ok(()),
            None => Err(StoreError::NotFound(rel_path.to_string())),
        }
    }

    fn rename_file(&mut self, root: &str, from: &str, to: &str) -> Result<(), StoreError> {
        self.user_files(root, to)?;
        let files = self.user_files(root, from)?;
        if !files.contains_key(from) {
            return Err(StoreError::NotFound(from.to_string()));
        }
        if from == to {
            return Ok(());
        }
        if files.contains_key(to) {
            return Err(StoreError::AlreadyExists(to.to_string()));
        }
        let bytes = files.remove(from).expect("checked");
        files.insert(to.to_string(), bytes);
        Ok(())
    }

    fn user_root(&self) -> Option<String> {
        self.user.clone()
    }

    /// `base-136`: only imported folders (`imported/<name>`, created) can be added.
    fn add_folder(&mut self, path: &str) -> Result<String, StoreError> {
        if !path.starts_with(IMPORTED_PREFIX) {
            return Err(StoreError::Unsupported(
                "the memory library has no OS folders".into(),
            ));
        }
        if !self.imported.contains_key(path) {
            return Err(StoreError::NotFound(path.to_string()));
        }
        let id = user_folder_id(path);
        self.added.insert(id.clone(), path.to_string());
        Ok(id)
    }

    /// `base-136`: forget a user folder and delete its files.
    fn remove_folder(&mut self, root: &str) -> Result<(), StoreError> {
        if !root.starts_with(USER_FOLDER_PREFIX) {
            return Err(StoreError::InvalidPath(format!(
                "{root} is not a user folder"
            )));
        }
        let path = self
            .added
            .remove(root)
            .ok_or_else(|| StoreError::NotFound(root.to_string()))?;
        self.imported.remove(&path);
        Ok(())
    }

    fn create_import_folder(&mut self, name: &str) -> Result<String, StoreError> {
        let base = import_folder_name(name)
            .ok_or_else(|| StoreError::InvalidPath(format!("bad folder name {name:?}")))?;
        let name = unique_folder_name(&base, |c| {
            let c = format!("{IMPORTED_PREFIX}{c}").to_lowercase();
            self.imported.keys().any(|k| k.to_lowercase() == c)
        });
        let path = format!("{IMPORTED_PREFIX}{name}");
        self.imported.insert(path.clone(), BTreeMap::new());
        Ok(path)
    }
}

/// `media/` sub-folder name (re-exported for hosts' convenience).
pub const MEDIA: &str = MEDIA_DIR;

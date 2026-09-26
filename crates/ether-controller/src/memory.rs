//! In-memory [`ProjectStore`] and [`Library`]: for tests, and as a reference for host
//! implementations (same path rules and listing conventions as the disk store).

use std::collections::BTreeMap;

use ether_core::protocol::media::{
    BrowseLocation, BrowseRoot, DirectoryEntry, DirectoryListing, FileKind,
};
use ether_core::protocol::model::ProjectId;
use ether_core::protocol::model::file::{CACHE_DIR, MEDIA_DIR, PROJECT_FILE};
use ether_core::protocol::project::ProjectSummary;

use crate::store::{Library, ProjectStore, StoreError, check_relative_path, file_kind};

/// List the direct children of `dir` among `files` (paths relative to the same root).
fn list(files: &BTreeMap<String, Vec<u8>>, dir: &str) -> Vec<DirectoryEntry> {
    let prefix = if dir.is_empty() { String::new() } else { format!("{dir}/") };
    let mut out: BTreeMap<String, DirectoryEntry> = BTreeMap::new();
    for (path, bytes) in files {
        let Some(rest) = path.strip_prefix(&prefix) else { continue };
        match rest.split_once('/') {
            Some((sub, _)) => {
                out.entry(sub.to_string()).or_insert_with(|| DirectoryEntry {
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
        self.projects.get(&id)?.files.get(rel_path).map(Vec::as_slice)
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
        v.sort_by(|a, b| b.modified_ms.total_cmp(&a.modified_ms).then(b.id.0.cmp(&a.id.0)));
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
        p.files.insert(PROJECT_FILE.into(), ether_json.as_bytes().to_vec());
        p.modified_ms = now;
        Ok(Self::summary(id, p))
    }

    fn duplicate(&mut self, from: ProjectId, to: ProjectId) -> Result<(), StoreError> {
        if self.projects.contains_key(&to) {
            return Err(StoreError::AlreadyExists(to.to_string()));
        }
        let mut copy = self.project(from)?.clone();
        copy.files.retain(|path, _| !path.starts_with(&format!("{CACHE_DIR}/")));
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
        self.project(id)?.files.insert(rel_path.to_string(), bytes.to_vec());
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

/// In-memory sample library: roots of files keyed by relative path.
#[derive(Debug, Clone, Default)]
pub struct MemoryLibrary {
    roots: BTreeMap<String, (String, BTreeMap<String, Vec<u8>>)>,
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

    /// Add a file under root `id` (created if needed).
    pub fn add_file(&mut self, id: &str, path: &str, bytes: Vec<u8>) {
        self.add_root(id, id);
        self.roots.get_mut(id).expect("added").1.insert(path.to_string(), bytes);
    }
}

impl Library for MemoryLibrary {
    fn roots(&self) -> Vec<BrowseRoot> {
        self.roots
            .iter()
            .map(|(id, (name, _))| BrowseRoot {
                location: BrowseLocation::Library { id: id.clone() },
                name: name.clone(),
            })
            .collect()
    }

    fn list_dir(&mut self, root: &str, rel_path: &str) -> Result<DirectoryListing, StoreError> {
        check_relative_path(rel_path)?;
        let (_, files) = self
            .roots
            .get(root)
            .ok_or_else(|| StoreError::NotFound(root.to_string()))?;
        Ok(DirectoryListing {
            location: BrowseLocation::Library { id: root.to_string() },
            path: rel_path.to_string(),
            entries: list(files, rel_path),
        })
    }

    fn read(&mut self, root: &str, rel_path: &str) -> Result<Vec<u8>, StoreError> {
        check_relative_path(rel_path)?;
        self.roots
            .get(root)
            .and_then(|(_, f)| f.get(rel_path))
            .cloned()
            .ok_or_else(|| StoreError::NotFound(format!("{root}/{rel_path}")))
    }
}

/// `media/` sub-folder name (re-exported for hosts' convenience).
pub const MEDIA: &str = MEDIA_DIR;

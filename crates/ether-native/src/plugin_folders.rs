//! Where plugins are looked for: the OS default folders of each format (on/off) plus the
//! folders the user added, each optionally limited to one format. Persisted as
//! `<plugin-db>/folders.json`, next to the plugin list and the scan cache.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ether_core::protocol::model::PluginFormat;
use ether_core::protocol::plugins::{DefaultPluginFolder, PluginFolder, PluginFolders};
use ether_plugin_host::{Formats, ScanTarget};
use serde::{Deserialize, Serialize};

/// File name of the folder settings in the plugin DB folder.
pub const FOLDERS_FILE: &str = "folders.json";

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Settings {
    #[serde(default = "yes")]
    include_defaults: bool,
    #[serde(default)]
    folders: Vec<PluginFolder>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            include_defaults: true,
            folders: Vec::new(),
        }
    }
}

/// Why a folder edit was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderError {
    Invalid(String),
    NotFound(String),
}

impl std::fmt::Display for FolderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FolderError::Invalid(m) | FolderError::NotFound(m) => f.write_str(m),
        }
    }
}

/// The plugin folder settings. Thread-safe; cheap to clone.
#[derive(Clone, Debug, Default)]
pub struct PluginFolderSettings {
    inner: Arc<Mutex<Settings>>,
    file: Option<PathBuf>,
}

fn same_folder(a: &str, b: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    Path::new(a) == b || canon(Path::new(a)) == canon(b)
}

impl PluginFolderSettings {
    /// Load `<plugin_db_dir>/folders.json` if present (defaults on, no user folders
    /// otherwise).
    pub fn open(plugin_db_dir: &Path) -> Self {
        let file = plugin_db_dir.join(FOLDERS_FILE);
        let settings = std::fs::read_to_string(&file)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self {
            inner: Arc::new(Mutex::new(settings)),
            file: Some(file),
        }
    }

    fn get(&self) -> Settings {
        self.inner.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn edit<R>(&self, f: impl FnOnce(&mut Settings) -> R) -> R {
        let mut s = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let r = f(&mut s);
        if let Some(file) = &self.file
            && let Ok(json) = serde_json::to_string_pretty(&*s)
            && let Err(e) = crate::store::atomic_write(file, json.as_bytes())
        {
            tracing::warn!(%e, "failed to write plugin folder settings");
        }
        r
    }

    /// The folders, with the OS defaults of `formats` (read-only).
    pub fn list(&self, formats: &Formats) -> PluginFolders {
        let s = self.get();
        PluginFolders {
            include_defaults: s.include_defaults,
            defaults: formats
                .default_search_paths()
                .into_iter()
                .map(|(format, path)| DefaultPluginFolder {
                    exists: path.is_dir(),
                    path: path.display().to_string(),
                    format,
                })
                .collect(),
            folders: s.folders,
        }
    }

    /// Add a folder (an existing directory, absolute). Adding one already listed (same
    /// path, or the same folder through a symlink) changes its format filter instead.
    pub fn add(&self, path: &str, format: Option<PluginFormat>) -> Result<(), FolderError> {
        let path = path.trim();
        let p = Path::new(path);
        if path.is_empty() || !p.is_absolute() {
            return Err(FolderError::Invalid(format!(
                "plugin folder must be an absolute path: {path:?}"
            )));
        }
        if !p.is_dir() {
            return Err(FolderError::Invalid(format!("not a folder: {path}")));
        }
        self.edit(
            |s| match s.folders.iter_mut().find(|f| same_folder(&f.path, p)) {
                Some(f) => f.format = format,
                None => s.folders.push(PluginFolder {
                    path: path.to_owned(),
                    format,
                }),
            },
        );
        Ok(())
    }

    /// Remove a user folder (by its path as listed, or the same folder another way).
    pub fn remove(&self, path: &str) -> Result<(), FolderError> {
        let p = Path::new(path.trim());
        self.edit(|s| {
            let before = s.folders.len();
            s.folders.retain(|f| !same_folder(&f.path, p));
            if s.folders.len() == before {
                Err(FolderError::NotFound(format!(
                    "not a plugin folder: {}",
                    p.display()
                )))
            } else {
                Ok(())
            }
        })
    }

    pub fn set_include_defaults(&self, include: bool) {
        self.edit(|s| s.include_defaults = include);
    }

    /// Scan targets of `formats` under these folders (nothing is loaded).
    pub fn discover(&self, formats: &Formats) -> Vec<ScanTarget> {
        let s = self.get();
        let folders: Vec<_> = s
            .folders
            .iter()
            .map(|f| (PathBuf::from(&f.path), f.format))
            .collect();
        formats.discover_folders(s.include_defaults, &folders)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempDir;

    #[test]
    fn add_remove_and_persist() {
        let tmp = TempDir::new("plugin-folders");
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let db = tmp.path().join("plugin-db");
        let s = PluginFolderSettings::open(&db);
        let formats = crate::plugins::formats();
        let l = s.list(&formats);
        assert!(l.include_defaults);
        assert!(l.folders.is_empty());
        assert!(!l.defaults.is_empty());

        s.add(a.to_str().unwrap(), None).unwrap();
        s.add(b.to_str().unwrap(), Some(PluginFormat::Vst3))
            .unwrap();
        // Re-adding (with a trailing `/.`) updates the filter instead of duplicating.
        s.add(&format!("{}/.", a.display()), Some(PluginFormat::Clap))
            .unwrap();
        assert!(matches!(
            s.add("relative/dir", None),
            Err(FolderError::Invalid(_))
        ));
        assert!(matches!(
            s.add(tmp.path().join("missing").to_str().unwrap(), None),
            Err(FolderError::Invalid(_))
        ));
        s.set_include_defaults(false);

        let again = PluginFolderSettings::open(&db).list(&formats);
        assert!(!again.include_defaults);
        assert_eq!(
            again.folders,
            [
                PluginFolder {
                    path: a.display().to_string(),
                    format: Some(PluginFormat::Clap)
                },
                PluginFolder {
                    path: b.display().to_string(),
                    format: Some(PluginFormat::Vst3)
                },
            ]
        );

        s.remove(a.to_str().unwrap()).unwrap();
        assert!(matches!(
            s.remove(a.to_str().unwrap()),
            Err(FolderError::NotFound(_))
        ));
        assert_eq!(
            PluginFolderSettings::open(&db).list(&formats).folders.len(),
            1
        );
    }

    #[test]
    fn discovery_includes_user_folders_by_format_and_honors_the_defaults_switch() {
        let tmp = TempDir::new("plugin-folders-discover");
        let any = tmp.path().join("any");
        let vst = tmp.path().join("vst");
        for d in [&any, &vst] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(any.join("A.clap"), "").unwrap();
        std::fs::create_dir_all(any.join("B.vst3")).unwrap();
        std::fs::write(vst.join("C.clap"), "").unwrap();
        std::fs::create_dir_all(vst.join("D.vst3")).unwrap();
        let s = PluginFolderSettings::open(&tmp.path().join("db"));
        s.add(any.to_str().unwrap(), None).unwrap();
        s.add(vst.to_str().unwrap(), Some(PluginFormat::Vst3))
            .unwrap();
        let formats = crate::plugins::formats();
        let user = |t: &Vec<ScanTarget>| {
            let mut v: Vec<_> = t
                .iter()
                .filter(|t| t.path.starts_with(tmp.path()))
                .map(|t| t.path.file_name().unwrap().to_string_lossy().into_owned())
                .collect();
            v.sort();
            v
        };
        let with_defaults = s.discover(&formats);
        assert_eq!(user(&with_defaults), ["A.clap", "B.vst3", "D.vst3"]);

        s.set_include_defaults(false);
        let only_user = s.discover(&formats);
        assert_eq!(user(&only_user), ["A.clap", "B.vst3", "D.vst3"]);
        assert!(
            only_user.iter().all(|t| t.path.starts_with(tmp.path())),
            "{only_user:?}"
        );
    }
}

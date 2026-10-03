//! Headless mode: an engine embedded in this process (`ether-native` with the null audio
//! backend: it renders in real time, nothing is played) on one project.
//!
//! `--project <path>` accepts:
//! - a project folder of an Ethereal project store (`<projects root>/<uuid>/`) or the
//!   `project.ether` inside it: opened in place, with its media; `save_project` saves it
//!   there;
//! - any other `.ether` path, existing or not: the document is loaded into a private work
//!   store (a temporary folder) or created there (named after the file), and every
//!   `save_project` also writes the document to the path. Media imported in this mode stay
//!   in the work store (removed on exit).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ether_native::audio::{AudioBackendKind, AudioSettings};
use ether_native::store::{PROJECT_FILE, PROJECT_SUBDIRS, atomic_write};
use ether_native::{DedicatedThread, HostConfig, HostOptions, LibraryRoot, NativeHost};
use ether_protocol::model::{IdGen, ProjectId};
use ether_protocol::project::ProjectCommand;
use ether_protocol::{ClientMessage, Command, ReplyValue, ServerMessage};

use crate::backend::{Backend, Pending, wait};

/// An embedded engine on one project.
pub struct HeadlessBackend {
    /// `None` once dropped (stopped before the work dir is removed).
    host: Option<NativeHost>,
    pending: Arc<Pending>,
    project: ProjectId,
    /// The project's file in the store.
    store_file: PathBuf,
    /// Standalone mode: the user's file (written on save) and the work dir (removed).
    standalone: Option<(PathBuf, PathBuf)>,
}

impl Drop for HeadlessBackend {
    fn drop(&mut self) {
        if let Some(host) = self.host.take() {
            host.unsubscribe();
            host.shutdown();
        }
        if let Some((_, work)) = &self.standalone {
            let _ = std::fs::remove_dir_all(work);
        }
    }
}

fn seed() -> u64 {
    let mut b = [0u8; 8];
    if getrandom::fill(&mut b).is_err() {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        return t.as_nanos() as u64 ^ u64::from(std::process::id());
    }
    u64::from_le_bytes(b)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// `Some((projects_root, id))` if `path` is a project folder of a store (or its
/// `project.ether`).
fn in_store(path: &Path) -> Option<(PathBuf, ProjectId)> {
    let dir = if path.file_name().is_some_and(|n| n == PROJECT_FILE) {
        path.parent()?
    } else {
        path
    };
    if !dir.join(PROJECT_FILE).is_file() {
        return None;
    }
    let id: ProjectId = dir.file_name()?.to_str()?.parse().ok()?;
    Some((dir.parent()?.to_path_buf(), id))
}

/// The project id stored in a `.ether` document.
fn id_of(json: &str) -> Result<ProjectId, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("not an .ether file: {e}"))?;
    v.pointer("/project/id")
        .and_then(|i| i.as_str())
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| "not an .ether file: no project id".to_string())
}

fn library_roots() -> Vec<LibraryRoot> {
    std::env::var_os("ETHER_LIBRARY")
        .map(|list| {
            std::env::split_paths(&list)
                .filter(|p| p.is_dir())
                .enumerate()
                .map(|(i, path)| LibraryRoot {
                    id: format!("lib{i}"),
                    name: path
                        .file_name()
                        .map_or_else(|| "Library".into(), |n| n.to_string_lossy().into_owned()),
                    path,
                })
                .collect()
        })
        .unwrap_or_default()
}

impl HeadlessBackend {
    /// Start an engine and open (or create) the project at `path`.
    pub fn open(path: &Path) -> Result<Self, String> {
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|e| e.to_string())?
                .join(path)
        };
        let work = std::env::temp_dir().join(format!("ether-mcp-{}-{:x}", std::process::id(), seed() as u32));
        std::fs::create_dir_all(&work).map_err(|e| format!("work dir: {e}"))?;
        let mut ids = IdGen::new(seed());
        let created = !path.exists();
        // What to open, and how.
        let (projects_root, open, standalone) = match in_store(&path) {
            Some((root, id)) => (root, Command::Project(ProjectCommand::Open { id }), None),
            None => {
                let root = work.join("projects");
                let open = if path.exists() {
                    let json = std::fs::read_to_string(&path)
                        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
                    let id = id_of(&json)?;
                    let dir = root.join(id.to_string());
                    for sub in PROJECT_SUBDIRS {
                        std::fs::create_dir_all(dir.join(sub)).map_err(|e| e.to_string())?;
                    }
                    std::fs::write(dir.join(PROJECT_FILE), json).map_err(|e| e.to_string())?;
                    Command::Project(ProjectCommand::Open { id })
                } else {
                    let name = path
                        .file_stem()
                        .map_or_else(|| "Untitled".into(), |s| s.to_string_lossy().into_owned());
                    Command::Project(ProjectCommand::Create {
                        id: ids.next_project_id(now_ms()),
                        name,
                    })
                };
                (root, open, Some((path.clone(), work.clone())))
            }
        };
        let data_dir = work.join("data");
        std::fs::create_dir_all(data_dir.join("config")).map_err(|e| e.to_string())?;
        let host = NativeHost::start(
            HostConfig {
                audio: Some(AudioSettings {
                    backend: AudioBackendKind::Null,
                    ..Default::default()
                }),
                data_dir,
                instance: "ether-mcp".into(),
                projects_root: projects_root.clone(),
                library_roots: library_roots(),
            },
            HostOptions {
                main_thread: Arc::new(DedicatedThread::new()),
                ..HostOptions::default()
            },
        )
        .map_err(|e| format!("engine failed to start: {e}"))?;
        let pending = Arc::new(Pending::default());
        {
            let pending = pending.clone();
            host.subscribe(Arc::new(move |m| {
                if let ServerMessage::Reply(r) = m {
                    pending.complete(r.id, Ok(r.result));
                }
            }));
        }
        let project = match request(&host, &pending, open)? {
            ReplyValue::Project { project } => project.id,
            other => return Err(format!("unexpected reply to open: {other:?}")),
        };
        let me = Self {
            host: Some(host),
            pending,
            project,
            store_file: projects_root.join(project.to_string()).join(PROJECT_FILE),
            standalone,
        };
        if me.standalone.is_some() && created {
            // A new document exists on disk from the start (an empty song is a fine file).
            me.after_save()?;
        }
        Ok(me)
    }

    pub fn project_id(&self) -> ProjectId {
        self.project
    }
}

fn request(host: &NativeHost, pending: &Pending, command: Command) -> Result<ReplyValue, String> {
    let (id, rx) = pending.register();
    host.send(ClientMessage {
        id,
        gesture: None,
        command,
    })
    .map_err(|e| {
        pending.forget(id);
        e.to_string()
    })?;
    wait(pending, id, rx)
}

impl Backend for HeadlessBackend {
    fn request(&self, command: Command) -> Result<ReplyValue, String> {
        let host = self.host.as_ref().ok_or("the engine is stopped")?;
        request(host, &self.pending, command)
    }

    fn after_save(&self) -> Result<Option<String>, String> {
        let Some((file, _)) = &self.standalone else {
            return Ok(Some(format!("saved to {}", self.store_file.display())));
        };
        let json = std::fs::read(&self.store_file)
            .map_err(|e| format!("cannot read the saved document: {e}"))?;
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        atomic_write(file, &json).map_err(|e| format!("cannot write {}: {e}", file.display()))?;
        Ok(Some(format!("saved to {}", file.display())))
    }

    fn describe(&self) -> String {
        let file = match &self.standalone {
            Some((f, _)) => f.clone(),
            None => self.store_file.clone(),
        };
        format!(
            "a headless Ethereal engine (no audio output) on {}; call save_project to write it",
            file.display()
        )
    }
}

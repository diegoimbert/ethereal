//! Project lifecycle over the [`ProjectStore`]: list, create, open, save (and autosave),
//! save-as, duplicate, rename, delete; dirty tracking and project switching (engine and
//! media state are rebuilt for the new document).

use ether_core::TransportControl;
use ether_core::protocol::export::ExportDownload;
use ether_core::protocol::model::file;
use ether_core::protocol::model::*;
use ether_core::protocol::project::{BundleSource, ProjectCommand, ProjectEvent, ProjectSummary};
use ether_core::protocol::{CommandError, ErrorCode, Event, ReplyValue};

use crate::bundle;
use crate::handlers::{event, no_project, store_err};
use crate::store::{Library, ProjectStore, StoreError};
use crate::tx::{CmdResult, cmd_err, invalid, invalid_state};
use crate::{EngineBridge, EtherController, HostServices, MessageSink, OpenDoc, TransportRt};

fn file_err(e: FileError) -> CommandError {
    cmd_err(ErrorCode::Io, format!("project file: {e}"))
}

fn check_name(name: &str) -> CmdResult<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(invalid("project name must not be empty"));
    }
    Ok(name.to_string())
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    pub(crate) fn project_command(
        &mut self,
        c: &ProjectCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        match c {
            ProjectCommand::SetScale { .. } => unreachable!("scale is a document command"),
            ProjectCommand::List => Ok(ReplyValue::Projects {
                projects: self.list_projects()?,
            }),
            ProjectCommand::Create { id, name } => self.create(*id, name, now, out),
            ProjectCommand::Open { id } => self.open(*id, now, out),
            ProjectCommand::OpenSafe { id } => self.open_safe(*id, now, out),
            ProjectCommand::LoadPlugins => {
                self.doc.as_ref().ok_or_else(no_project)?;
                if self.engine.load_deferred() {
                    self.emit_safe_mode(out);
                    self.publish_if_due(now, true, out);
                }
                Ok(ReplyValue::Unit)
            }
            ProjectCommand::Save => Ok(ReplyValue::Saved {
                project: self.save_current(out)?,
            }),
            ProjectCommand::SaveAs { new_id, name } => self.save_as(*new_id, name, out),
            ProjectCommand::Duplicate { id, new_id, name } => {
                self.duplicate(*id, *new_id, name, out)
            }
            ProjectCommand::Rename { id, name } => {
                // The current project's rename is a document edit (handled by dispatch).
                let name = check_name(name)?;
                let json = self.store.load(*id).map_err(store_err)?;
                let mut project = file::load(&json).map_err(file_err)?;
                project.settings.name = name;
                let json = file::save(&project, &self.config.app_version).map_err(file_err)?;
                self.store.save(*id, &json).map_err(store_err)?;
                self.emit_list_changed(out);
                Ok(ReplyValue::Unit)
            }
            ProjectCommand::Delete { id } => {
                if self.doc.as_ref().is_some_and(|d| d.project.id == *id) {
                    return Err(invalid_state("the open project cannot be deleted"));
                }
                self.store.delete(*id).map_err(store_err)?;
                self.emit_list_changed(out);
                Ok(ReplyValue::Unit)
            }
            ProjectCommand::ExportBundle { id, path } => self.export_bundle(*id, path.as_deref()),
            ProjectCommand::ImportBundle {
                new_id,
                source,
                name,
            } => self.import_bundle(*new_id, source, name.as_deref(), out),
            ProjectCommand::Get => {
                let project = self.doc.as_ref().ok_or_else(no_project)?.project.clone();
                // (Re)connect: resend the runtime state the UI cannot derive.
                self.last_transport = None;
                self.emit_transport_if_changed(out);
                self.emit_armed(out);
                Ok(ReplyValue::Project {
                    project: Box::new(project),
                })
            }
        }
    }

    /// Stored projects, newest first (the open project shows its current name).
    pub(crate) fn list_projects(&mut self) -> CmdResult<Vec<ProjectSummary>> {
        let mut list = self.store.list().map_err(store_err)?;
        if let Some(doc) = &self.doc {
            for s in &mut list {
                if s.id == doc.project.id {
                    s.name = doc.project.settings.name.clone();
                }
            }
        }
        Ok(list)
    }

    pub(crate) fn emit_list_changed(&mut self, out: &mut dyn MessageSink) {
        if let Ok(projects) = self.list_projects() {
            event(
                out,
                Event::Project {
                    event: ProjectEvent::ListChanged { projects },
                },
            );
        }
    }

    /// Serialize a project, with every plugin's live state read from the engine.
    pub(crate) fn serialize(&mut self, project: &Project) -> CmdResult<String> {
        let mut copy = project.clone();
        for d in copy.devices.values_mut() {
            if let DeviceKind::Plugin { plugin } = &mut d.kind
                && let Ok(Some(state)) = self.bridge.plugin_state(d.id)
            {
                plugin.state = Some(state);
            }
        }
        file::save(&copy, &self.config.app_version).map_err(file_err)
    }

    /// Save the open project to the store (Save, autosave, before switching projects).
    pub(crate) fn save_current(&mut self, out: &mut dyn MessageSink) -> CmdResult<ProjectSummary> {
        let doc = self.doc.as_ref().ok_or_else(no_project)?;
        let (id, project) = (doc.project.id, doc.project.clone());
        let json = self.serialize(&project)?;
        let summary = self.store.save(id, &json).map_err(store_err)?;
        event(
            out,
            Event::Project {
                event: ProjectEvent::Saved {
                    project: summary.clone(),
                },
            },
        );
        self.set_dirty(false, out);
        self.emit_list_changed(out);
        Ok(summary)
    }

    pub(crate) fn autosave_before_switch(&mut self, out: &mut dyn MessageSink) -> CmdResult<()> {
        if self.doc.as_ref().is_some_and(|d| d.dirty) {
            self.save_current(out)?;
        }
        Ok(())
    }

    fn create(
        &mut self,
        id: ProjectId,
        name: &str,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        self.create_with(id, name, now, out, |_, _| Ok(()))
    }

    /// `Project::Create`; `init` fills the new document (and may write files to the new
    /// project folder) before it is saved and opened (v0.3 `templates`: `NewProject`). Its
    /// id and name are set afterwards; if `init` fails the folder is removed again.
    pub(crate) fn create_with(
        &mut self,
        id: ProjectId,
        name: &str,
        now: u64,
        out: &mut dyn MessageSink,
        init: impl FnOnce(&mut Self, &mut Project) -> CmdResult<()>,
    ) -> CmdResult<ReplyValue> {
        let name = check_name(name)?;
        if let Some(doc) = &self.doc
            && doc.project.id == id
        {
            // Retried message: already created and open.
            return Ok(ReplyValue::Project {
                project: Box::new(doc.project.clone()),
            });
        }
        self.autosave_before_switch(out)?;
        self.store.create(id).map_err(|e| match e {
            StoreError::AlreadyExists(_) => invalid(format!("project {id} already exists")),
            e => store_err(e),
        })?;
        let mut project = Project::new(&mut self.ids, now);
        if let Err(e) = init(self, &mut project) {
            let _ = self.store.delete(id);
            return Err(e);
        }
        project.id = id;
        project.settings.name = name;
        let json = file::save(&project, &self.config.app_version).map_err(file_err)?;
        self.store.save(id, &json).map_err(store_err)?;
        self.load_project(project.clone(), now, out);
        self.emit_list_changed(out);
        Ok(ReplyValue::Project {
            project: Box::new(project),
        })
    }

    pub(crate) fn open(
        &mut self,
        id: ProjectId,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        self.autosave_before_switch(out)?;
        let json = self.store.load(id).map_err(store_err)?;
        let mut project = file::load(&json).map_err(file_err)?;
        // The folder name is the identity.
        project.id = id;
        self.load_project(project.clone(), now, out);
        Ok(ReplyValue::Project {
            project: Box::new(project),
        })
    }

    /// base-131: open `id` with its plugin devices held as bypassed placeholders.
    fn open_safe(
        &mut self,
        id: ProjectId,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        self.autosave_before_switch(out)?;
        let json = self.store.load(id).map_err(store_err)?;
        let mut project = file::load(&json).map_err(file_err)?;
        project.id = id;
        self.load_project_with(project.clone(), now, true, out);
        Ok(ReplyValue::Project {
            project: Box::new(project),
        })
    }

    /// `Event::Project { SafeMode }` with the current placeholders.
    pub(crate) fn emit_safe_mode(&self, out: &mut dyn MessageSink) {
        event(
            out,
            Event::Project {
                event: ProjectEvent::SafeMode {
                    active: self.engine.safe_mode(),
                    devices: self.engine.deferred(),
                },
            },
        );
    }

    fn save_as(
        &mut self,
        new_id: ProjectId,
        name: &str,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let name = check_name(name)?;
        let doc = self.doc.as_ref().ok_or_else(no_project)?;
        if doc.project.id == new_id {
            return Ok(ReplyValue::Project {
                project: Box::new(doc.project.clone()),
            });
        }
        let old = doc.project.id;
        // Copies media too; the document is then replaced by the current state.
        self.store.duplicate(old, new_id).map_err(|e| match e {
            StoreError::AlreadyExists(_) => invalid(format!("project {new_id} already exists")),
            e => store_err(e),
        })?;
        // base-115: a copy is a private project (never the original's `share.json`).
        crate::share::clear_share_file(&mut self.store, new_id);
        let mut project = doc.project.clone();
        project.id = new_id;
        project.settings.name = name;
        let json = self.serialize(&project)?;
        let summary = self.store.save(new_id, &json).map_err(store_err)?;
        // Same content: the engine keeps its nodes and media; only identity/name/history change.
        let doc = self.doc.as_mut().expect("checked");
        doc.project = project.clone();
        doc.history.clear();
        event(
            out,
            Event::ProjectLoaded {
                project: Box::new(project.clone()),
            },
        );
        event(
            out,
            Event::Project {
                event: ProjectEvent::Saved { project: summary },
            },
        );
        let was_dirty = doc.dirty;
        doc.dirty = false;
        if was_dirty {
            event(
                out,
                Event::Project {
                    event: ProjectEvent::DirtyChanged { dirty: false },
                },
            );
        }
        // project-versions: the session moves to the new folder.
        let now = self.host.now_ms();
        self.versions_on_identity_change(now);
        self.emit_list_changed(out);
        Ok(ReplyValue::Project {
            project: Box::new(project),
        })
    }

    fn duplicate(
        &mut self,
        id: ProjectId,
        new_id: ProjectId,
        name: &str,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let name = check_name(name)?;
        if let Some(existing) = self.list_projects()?.into_iter().find(|s| s.id == new_id) {
            // Retried message.
            return Ok(ReplyValue::Saved { project: existing });
        }
        self.store.duplicate(id, new_id).map_err(store_err)?;
        // base-115: a duplicate is a private project (never the original's `share.json`).
        crate::share::clear_share_file(&mut self.store, new_id);
        // project-versions: the copy is not open anywhere.
        self.versions_forget_marker(new_id);
        let current = self
            .doc
            .as_ref()
            .filter(|d| d.project.id == id)
            .map(|d| d.project.clone());
        let mut project = match current {
            // Duplicating the open project copies its current state.
            Some(p) => p,
            None => file::load(&self.store.load(new_id).map_err(store_err)?).map_err(file_err)?,
        };
        project.id = new_id;
        project.settings.name = name;
        let json = self.serialize(&project)?;
        let summary = self.store.save(new_id, &json).map_err(store_err)?;
        self.emit_list_changed(out);
        Ok(ReplyValue::Saved { project: summary })
    }

    /// `ExportBundle`: the project's document (the open one: its current state, plugin
    /// states included) and its `media/` files, packed (see [`bundle`]).
    fn export_bundle(&mut self, id: ProjectId, path: Option<&str>) -> CmdResult<ReplyValue> {
        let current = self.doc.as_ref().map(|d| d.project.id);
        let (json, name) = if current == Some(id) {
            let project = self.doc.as_ref().expect("checked").project.clone();
            (self.serialize(&project)?, project.settings.name)
        } else {
            let json = self.store.load(id).map_err(store_err)?;
            let name = file::load(&json).map_err(file_err)?.settings.name;
            (json, name)
        };
        let mut entries = vec![(bundle::DOCUMENT.to_string(), json.into_bytes())];
        // Breadth-first walk of `media/` (missing = no media).
        let mut dirs = vec![bundle::MEDIA_PREFIX.trim_end_matches('/').to_string()];
        while let Some(dir) = dirs.pop() {
            let listing = match self.store.list_dir(id, &dir) {
                Ok(l) => l,
                Err(StoreError::NotFound(_)) => continue,
                Err(e) => return Err(store_err(e)),
            };
            for e in listing.entries {
                if e.kind == ether_core::protocol::media::FileKind::Directory {
                    dirs.push(e.path);
                } else {
                    let bytes = self.store.read(id, &e.path).map_err(store_err)?;
                    entries.push((e.path, bytes));
                }
            }
        }
        let bytes = bundle::pack(&entries).map_err(invalid)?;
        if let Some(path) = path {
            self.store
                .write_bundle_file(path, &bytes)
                .map_err(store_err)?;
            return Ok(ReplyValue::Unit);
        }
        let token = format!("bundle-{id}");
        let download = ExportDownload {
            token: token.clone(),
            name: format!("{}.ether", bundle_file_name(&name)),
            mime: "application/zip".into(),
            size: bytes.len() as f64,
        };
        self.export
            .add_download(token, current.unwrap_or(id), bytes);
        Ok(ReplyValue::Bundle { download })
    }

    /// `ImportBundle`: a new stored project from a bundle (or a bare `project.ether`).
    fn import_bundle(
        &mut self,
        new_id: ProjectId,
        source: &BundleSource,
        name: Option<&str>,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let name = name.map(check_name).transpose()?;
        if let Some(existing) = self.list_projects()?.into_iter().find(|s| s.id == new_id) {
            // Retried message.
            return Ok(ReplyValue::Saved { project: existing });
        }
        let bytes = match source {
            BundleSource::Upload { upload } => {
                let bytes = self.store.read_upload(upload).map_err(store_err)?;
                let _ = self.store.discard_upload(upload);
                bytes
            }
            BundleSource::Path { path } => self.store.read_bundle_file(path).map_err(store_err)?,
        };
        let entries: Vec<(String, &[u8])> = if bundle::is_archive(&bytes) {
            bundle::unpack(&bytes).map_err(invalid)?
        } else {
            vec![(bundle::DOCUMENT.to_string(), bytes.as_slice())]
        };
        let doc = entries
            .iter()
            .find(|(n, _)| n == bundle::DOCUMENT)
            .ok_or_else(|| invalid("the bundle has no project.ether"))?;
        let json = std::str::from_utf8(doc.1)
            .map_err(|_| invalid("project.ether is not a text document"))?;
        let mut project = file::load(json).map_err(|e| invalid(format!("project file: {e}")))?;
        project.id = new_id;
        if let Some(name) = name {
            project.settings.name = name;
        }
        let json = file::save(&project, &self.config.app_version).map_err(file_err)?;
        self.store.create(new_id).map_err(|e| match e {
            StoreError::AlreadyExists(_) => invalid(format!("project {new_id} already exists")),
            e => store_err(e),
        })?;
        let written = (|| {
            for (rel, data) in entries.iter().filter(|(n, _)| n != bundle::DOCUMENT) {
                self.store.write(new_id, rel, data)?;
            }
            self.store.save(new_id, &json)
        })();
        let summary = match written {
            Ok(s) => s,
            Err(e) => {
                // No half-imported project left behind.
                let _ = self.store.delete(new_id);
                return Err(store_err(e));
            }
        };
        self.emit_list_changed(out);
        Ok(ReplyValue::Saved { project: summary })
    }

    /// Make `project` the open document: reset history, runtime state, engine nodes and
    /// media, then announce it.
    pub(crate) fn load_project(&mut self, project: Project, now: u64, out: &mut dyn MessageSink) {
        self.load_project_with(project, now, false, out);
    }

    /// `safe`: plugin devices are held as placeholders (base-131 safe mode).
    fn load_project_with(
        &mut self,
        project: Project,
        now: u64,
        safe: bool,
        out: &mut dyn MessageSink,
    ) {
        self.engine.reset();
        if safe {
            self.engine.defer_plugins(&project);
        }
        self.media.reset(&mut self.bridge);
        self.armed.clear();
        self.plugin_gestures.clear();
        let _ = self.bridge.transport(TransportControl::Stop);
        let _ = self.bridge.transport(TransportControl::Locate {
            position: Beats::ZERO,
        });
        self.transport = TransportRt {
            last_engine_playing: self.transport.last_engine_playing,
            ..TransportRt::default()
        };
        self.doc = Some(OpenDoc {
            project: project.clone(),
            history: History::new(self.config.history_depth),
            dirty: false,
            last_edit_ms: now,
        });
        event(
            out,
            Event::ProjectLoaded {
                project: Box::new(project),
            },
        );
        self.emit_armed(out);
        event(
            out,
            Event::Project {
                event: ProjectEvent::DirtyChanged { dirty: false },
            },
        );
        self.last_transport = None;
        self.emit_transport_if_changed(out);
        if safe {
            self.emit_safe_mode(out);
        }
        // project-versions: session marker and version clock. Written BEFORE the devices
        // (plugins) are instantiated, so a crash while loading them counts as a crash
        // (base-131: no reopen on launch, recovery offered).
        self.versions_on_load(now);
        let doc = self.doc.as_ref().expect("just set");
        self.media.sync(&mut self.bridge, Some(&doc.project));
        self.engine.graph_dirty = true;
        self.publish_if_due(now, true, out);
        // base-115: hosting/joined sessions of another project pause; this one resumes
        // sharing or reconnects (docs/SHARING.md §7).
        let pid = self.doc.as_ref().expect("just set").project.id;
        self.share_project_loaded(pid, out);
    }
}

/// A project name as a file name: path separators and control/reserved characters become
/// `_`; never empty.
fn bundle_file_name(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim_start_matches('.').trim();
    if cleaned.is_empty() {
        "Project".into()
    } else {
        cleaned.into()
    }
}

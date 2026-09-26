//! Project lifecycle over the [`ProjectStore`]: list, create, open, save (and autosave),
//! save-as, duplicate, rename, delete; dirty tracking and project switching (engine and
//! media state are rebuilt for the new document).

use ether_core::TransportControl;
use ether_core::protocol::model::file;
use ether_core::protocol::model::*;
use ether_core::protocol::project::{ProjectCommand, ProjectEvent, ProjectSummary};
use ether_core::protocol::{CommandError, ErrorCode, Event, ReplyValue};

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
            ProjectCommand::List => Ok(ReplyValue::Projects {
                projects: self.list_projects()?,
            }),
            ProjectCommand::Create { id, name } => self.create(*id, name, now, out),
            ProjectCommand::Open { id } => self.open(*id, now, out),
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
    fn serialize(&mut self, project: &Project) -> CmdResult<String> {
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

    fn autosave_before_switch(&mut self, out: &mut dyn MessageSink) -> CmdResult<()> {
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

    /// Make `project` the open document: reset history, runtime state, engine nodes and
    /// media, then announce it.
    fn load_project(&mut self, project: Project, now: u64, out: &mut dyn MessageSink) {
        self.engine.reset();
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
        let doc = self.doc.as_ref().expect("just set");
        self.media.sync(&mut self.bridge, Some(&doc.project));
        self.engine.graph_dirty = true;
        self.publish_if_due(now, true, out);
    }
}

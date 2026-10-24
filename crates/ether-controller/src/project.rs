//! Project lifecycle over the [`ProjectStore`]: list, create, open, save (and autosave),
//! save-as, duplicate, rename, delete; dirty tracking and project switching (engine and
//! media state are rebuilt for the new document).

use ether_core::TransportControl;
use ether_core::protocol::export::ExportDownload;
use ether_core::protocol::model::file;
use ether_core::protocol::model::patch::without_plugin_states;
use ether_core::protocol::model::*;
use ether_core::protocol::project::{BundleSource, ProjectCommand, ProjectEvent, ProjectSummary};
use ether_core::protocol::{CommandError, ErrorCode, Event, ReplyValue};

use crate::bundle;
use crate::handlers::{event, no_project, notify, store_err};
use crate::saving::SaveKind;
use crate::store::{Library, ProjectStore, RenderDocument, StoreError};
use crate::tx::{CmdResult, invalid};
use crate::{EngineBridge, EtherController, HostServices, MessageSink, OpenDoc, TransportRt};

fn file_err(e: FileError) -> CommandError {
    version_err(&e).unwrap_or_else(|| {
        crate::tx::keyed_err(
            ErrorCode::Io,
            crate::text::msg("error.project.file").arg("error", e.to_string()),
        )
    })
}

/// An `.ether` document this app cannot read because of its format version (made by a
/// newer Ethereal, or by a pre-release build): `Unsupported`, with the message as the user
/// should read it.
pub fn version_err(e: &FileError) -> Option<CommandError> {
    let key = match e {
        FileError::TooNew { found, supported } => crate::text::msg("error.project.too_new")
            .arg("found", i64::from(*found))
            .arg("supported", i64::from(*supported)),
        FileError::PreRelease => crate::text::msg("error.project.pre_release"),
        _ => return None,
    };
    Some(crate::tx::keyed_err(ErrorCode::Unsupported, key))
}

/// A document coming from outside the store (a bundle, an export source): version errors
/// as [`version_err`], anything else `InvalidArgument`.
pub fn import_err(e: FileError) -> CommandError {
    version_err(&e).unwrap_or_else(|| {
        crate::tx::keyed_err(
            ErrorCode::InvalidArgument,
            crate::text::msg("error.project.file").arg("error", e.to_string()),
        )
    })
}

fn check_name(name: &str) -> CmdResult<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(crate::tx::keyed_err(
            ErrorCode::InvalidArgument,
            crate::text::msg("error.project.name_empty"),
        ));
    }
    Ok(name.to_string())
}

impl<B, H, S, L, X> EtherController<B, H, S, L, X>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    pub fn project_command(
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
                    self.begin_load();
                    self.emit_safe_mode(out);
                    self.publish_if_due(now, true, out);
                    self.load_tick(out);
                }
                Ok(ReplyValue::Unit)
            }
            ProjectCommand::Save { autosave } => {
                let kind = if *autosave {
                    SaveKind::Routine
                } else {
                    SaveKind::Full
                };
                Ok(ReplyValue::Saved {
                    project: self.save_current_with(kind, out)?,
                })
            }
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
                    return Err(crate::tx::keyed_err(
                        ether_core::protocol::ErrorCode::InvalidState,
                        crate::text::msg("error.project.delete_open"),
                    ));
                }
                self.store.delete(*id).map_err(store_err)?;
                self.emit_list_changed(out);
                Ok(ReplyValue::Unit)
            }
            ProjectCommand::Discard { id } => {
                if self.doc.as_ref().is_some_and(|d| d.project.id == *id) {
                    // Deleted once it closes (`close_discarded`); never saved meanwhile.
                    self.discarded = Some(*id);
                    self.set_dirty(false, out);
                    return Ok(ReplyValue::Unit);
                }
                self.store.delete(*id).map_err(store_err)?;
                self.emit_list_changed(out);
                Ok(ReplyValue::Unit)
            }
            ProjectCommand::ExportBundle {
                id,
                path,
                include_media,
                hide_devices,
            } => self.export_bundle(
                *id,
                path.as_deref(),
                *include_media,
                hide_devices.unwrap_or(false),
            ),
            ProjectCommand::ImportBundle {
                new_id,
                source,
                name,
            } => self.import_bundle(*new_id, source, name.as_deref(), out),
            // Runs from the tick (`import` module).
            ProjectCommand::Import {
                new_id,
                source,
                name,
            } => self.import_command(*new_id, source, name.as_deref(), out),
            ProjectCommand::CancelImport { id } => self.cancel_import(*id, out),
            ProjectCommand::ExportTo { id, format, path } => {
                self.export_to(*id, *format, path.as_deref())
            }
            ProjectCommand::Get => {
                let project = self.doc.as_ref().ok_or_else(no_project)?.project.clone();
                // (Re)connect: resend the runtime state the UI cannot derive.
                self.last_transport = None;
                self.emit_transport_if_changed(out);
                self.emit_armed(out);
                Ok(ReplyValue::Project {
                    project: Box::new(without_plugin_states(project)),
                })
            }
            // A project command of an extension's protocol that no extension answered.
            #[allow(unreachable_patterns)]
            _ => Err(crate::tx::unsupported("not available in this build")),
        }
    }

    /// Stored projects, newest first (the open project shows its current name).
    pub fn list_projects(&mut self) -> CmdResult<Vec<ProjectSummary>> {
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

    /// Tell the UI the stored projects changed (`Event::ProjectList`).
    pub fn emit_list_changed(&mut self, out: &mut dyn MessageSink) {
        if let Ok(projects) = self.list_projects() {
            event(
                out,
                Event::Project {
                    event: ProjectEvent::ListChanged { projects },
                },
            );
        }
    }

    /// Bring the plugin state cache up to date for saving `project`'s plugin devices:
    /// every instance is read for a [`SaveKind::Full`] save, only the stale or never read
    /// ones for a routine one (`crate::saving`). A failed read keeps what was there (the
    /// document's state if nothing was ever read) and is listed in
    /// `PluginStates::read_failures` until the next read.
    pub fn read_plugin_states(&mut self, project: &Project, kind: SaveKind) {
        for id in self.engine.take_created_plugins() {
            self.plugin_states.invalidate(id);
        }
        self.plugin_states.read_failures.clear();
        for d in project.devices.values() {
            if matches!(d.kind, DeviceKind::Plugin { .. })
                && self.plugin_states.needs_read(d.id, kind)
            {
                match self.bridge.plugin_state(d.id) {
                    Ok(state) => self.plugin_states.record(d.id, state),
                    Err(e) => self.plugin_states.read_failures.push((d.id, e.to_string())),
                }
            }
        }
    }

    /// A copy of `project` with the cached plugin states (see [`Self::read_plugin_states`]).
    pub fn with_plugin_states(&self, project: &Project) -> Project {
        let mut copy = project.clone();
        for d in copy.devices.values_mut() {
            if let DeviceKind::Plugin { plugin } = &mut d.kind
                && let Some(state) = self.plugin_states.cached(d.id)
            {
                plugin.state = Some(state.clone());
            }
        }
        copy
    }

    /// Serialize a project, with every plugin's live state read from the engine (copies,
    /// exports, versions).
    pub fn serialize(&mut self, project: &Project) -> CmdResult<String> {
        self.serialize_with(project, SaveKind::Full)
    }

    /// [`Self::serialize`], reading plugin states as `kind` says.
    pub fn serialize_with(&mut self, project: &Project, kind: SaveKind) -> CmdResult<String> {
        self.read_plugin_states(project, kind);
        file::save(&self.with_plugin_states(project), &self.config.app_version).map_err(file_err)
    }

    /// A plugin of the open project reported a state change no save has written yet.
    pub fn has_unsaved_plugin_states(&self) -> bool {
        self.doc.is_some() && self.plugin_states.has_unsaved()
    }

    /// Unsaved work: document edits or plugin state changes.
    pub fn has_unsaved_work(&self) -> bool {
        self.doc.as_ref().is_some_and(|d| d.dirty) || self.has_unsaved_plugin_states()
    }

    /// Save the open project to the store with every plugin's live state (explicit Save,
    /// before switching projects or quitting).
    pub fn save_current(&mut self, out: &mut dyn MessageSink) -> CmdResult<ProjectSummary> {
        self.save_current_with(SaveKind::Full, out)
    }

    /// Save the open project to the store.
    ///
    /// [`SaveKind::Routine`] (autosaves) reads only the plugins that changed and hands the
    /// serialization and the write to the store's background writer
    /// (`ProjectStore::save_in_background`): the controller thread only clones the document,
    /// so commands queued behind an autosave of a big project aren't held up. The reply
    /// and the `Saved` event then mean "accepted"; a failed write comes back through
    /// `ProjectStore::take_save_error` (the tick reports it and makes the project dirty
    /// again). The project list isn't re-read either (that reads every stored project):
    /// `Saved` carries the new summary. [`SaveKind::Full`] reads every plugin and is on
    /// disk when this returns.
    pub fn save_current_with(
        &mut self,
        kind: SaveKind,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ProjectSummary> {
        if let Some(e) = self.save_refused() {
            return Err(e);
        }
        let doc = self.doc.take().ok_or_else(no_project)?;
        let (id, name) = (doc.project.id, doc.project.settings.name.clone());
        self.plugin_states
            .retain(|d| doc.project.devices.contains_key(&d));
        self.read_plugin_states(&doc.project, kind);
        let project = self.with_plugin_states(&doc.project);
        self.doc = Some(doc);
        let background = kind == SaveKind::Routine && !self.plugin_states.write_failed;
        // An extension's file written right after the document (`ControllerExt::save_sidecar`).
        let sidecar = self.with_extensions(|e, ctl| e.save_sidecar(ctl, id));
        let with_sidecar = sidecar.is_some();
        let summary = if background {
            let version = self.config.app_version.clone();
            let render: RenderDocument = Box::new(move || {
                file::save(&project, &version)
                    .map_err(|e| StoreError::Io(format!("project file: {e}")))
            });
            match sidecar {
                Some(sidecar) => self
                    .store
                    .save_in_background_with(id, &name, render, sidecar),
                None => self.store.save_in_background(id, &name, render),
            }
        } else {
            let json = file::save(&project, &self.config.app_version).map_err(file_err)?;
            let saved = self.store.save(id, &json);
            if saved.is_ok()
                && let Some((path, bytes)) = sidecar.and_then(|f| f(&json))
                && let Err(e) = self.store.write(id, &path, &bytes)
            {
                tracing::warn!(error = %e, path, "could not write a sidecar file");
            }
            saved
        }
        .map_err(store_err)?;
        self.ext_each(|e, ctl| e.saved(ctl, id, with_sidecar));
        if !background {
            self.plugin_states.write_failed = false;
        }
        self.plugin_states.saved();
        event(
            out,
            Event::Project {
                event: ProjectEvent::Saved {
                    project: summary.clone(),
                },
            },
        );
        self.set_dirty(false, out);
        if kind == SaveKind::Full {
            self.emit_list_changed(out);
        }
        Ok(summary)
    }

    /// A background save failed (`ProjectStore::take_save_error`, polled by the tick):
    /// report it, and make the project dirty again so the next autosave (synchronous now,
    /// so a persisting error shows in its reply) writes it.
    pub fn background_save_failed(
        &mut self,
        id: ProjectId,
        e: StoreError,
        now: u64,
        out: &mut dyn MessageSink,
    ) {
        notify(
            out,
            ether_core::protocol::NotificationLevel::Error,
            crate::text::msg("notice.project.autosave_failed").arg("error", e.to_string()),
        );
        self.plugin_states.write_failed = true;
        if let Some(doc) = self.doc.as_mut().filter(|d| d.project.id == id) {
            doc.last_edit_ms = now;
        } else {
            return;
        }
        self.set_dirty(true, out);
    }

    /// Wait for the background autosaves (`ProjectStore::save_in_background`) and report
    /// those that failed: a failure for the open project makes it dirty again, so the
    /// caller's save writes it (the routine save already said `Saved`).
    pub fn settle_background_saves(&mut self, out: &mut dyn MessageSink) {
        self.store.flush_saves();
        while let Some((id, e)) = self.store.take_save_error() {
            let now = self.doc.as_ref().map_or(0, |d| d.last_edit_ms);
            self.background_save_failed(id, e, now, out);
        }
    }

    /// Before the open project is closed (another one opens, a session is adopted, the
    /// host quits): finish a running recording (its takes go into this project),
    /// wait for the background autosaves, then save every unsaved change with every
    /// plugin's live state, synchronously. An error means the project is NOT safely on
    /// disk: the caller must not drop the document.
    pub fn autosave_before_switch(&mut self, out: &mut dyn MessageSink) -> CmdResult<()> {
        if self.transport.recording || self.recording.is_active() {
            let now = self.host.now_ms();
            self.finish_recording(now, out);
        }
        if self.discarded_open() {
            self.close_discarded(out);
            return Ok(());
        }
        self.settle_background_saves(out);
        // Work an extension holds back (`save_refused`) is dropped, never saved.
        if self.has_unsaved_work() && self.save_refused().is_none() {
            self.save_current(out)?;
        }
        Ok(())
    }

    /// The open project was discarded (`ProjectCommand::Discard`).
    pub fn discarded_open(&self) -> bool {
        self.discarded
            .is_some_and(|id| self.doc.as_ref().is_some_and(|d| d.project.id == id))
    }

    /// The discarded project closes (another one opens, the host quits): wait for the
    /// background writes, then delete its folder (document, media, versions, recovery
    /// marker). A recording in progress is dropped with it.
    pub fn close_discarded(&mut self, out: &mut dyn MessageSink) {
        let Some(id) = self.discarded.take() else {
            return;
        };
        self.store.flush_saves();
        while self.store.take_save_error().is_some() {}
        self.versions_forget_project(id);
        if let Err(e) = self.store.delete(id) {
            tracing::warn!(error = %e, %id, "could not delete the discarded project");
        }
        self.emit_list_changed(out);
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
    pub fn create_with(
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
                project: Box::new(without_plugin_states(doc.project.clone())),
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
            project: Box::new(without_plugin_states(project)),
        })
    }

    pub fn open(
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
            project: Box::new(without_plugin_states(project)),
        })
    }

    /// Open `id` with its plugin devices held as bypassed placeholders.
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
            project: Box::new(without_plugin_states(project)),
        })
    }

    /// `Event::Project { SafeMode }` with the current placeholders.
    pub fn emit_safe_mode(&self, out: &mut dyn MessageSink) {
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
                project: Box::new(without_plugin_states(doc.project.clone())),
            });
        }
        let old = doc.project.id;
        // Copies media too; the document is then replaced by the current state.
        self.store.duplicate(old, new_id).map_err(|e| match e {
            StoreError::AlreadyExists(_) => invalid(format!("project {new_id} already exists")),
            e => store_err(e),
        })?;
        let mut project = doc.project.clone();
        // A copy is a private project: the extensions drop their files from it.
        self.ext_each(|e, ctl| e.project_copied(ctl, new_id));
        project.id = new_id;
        project.settings.name = name;
        // The copy still holds the old document until this lands: remove it on failure.
        let summary = self.save_copy(new_id, &project)?;
        // Same content: the engine keeps its nodes and media; only identity/name/history change.
        let doc = self.doc.as_mut().expect("checked");
        doc.project = project.clone();
        doc.history.clear();
        event(
            out,
            Event::ProjectLoaded {
                project: Box::new(without_plugin_states(project.clone())),
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
        // Clients leave safe mode on `ProjectLoaded`; the engine still holds the plugins as
        // placeholders, so say so again.
        if self.engine.safe_mode() {
            self.emit_safe_mode(out);
        }
        // The session moves to the new folder.
        let now = self.host.now_ms();
        self.versions_on_identity_change(now);
        self.emit_list_changed(out);
        Ok(ReplyValue::Project {
            project: Box::new(without_plugin_states(project)),
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
        self.ext_each(|e, ctl| e.project_copied(ctl, new_id));
        // A duplicate is a private project: `duplicate` never copies the original's
        // `share.json`.
        // The copy is not open anywhere.
        self.versions_forget_marker(new_id);
        let current = self
            .doc
            .as_ref()
            .filter(|d| d.project.id == id)
            .map(|d| d.project.clone());
        let mut project = match current {
            // Duplicating the open project copies its current state.
            Some(p) => p,
            None => match self
                .store
                .load(new_id)
                .map_err(store_err)
                .and_then(|json| file::load(&json).map_err(file_err))
            {
                Ok(p) => p,
                Err(e) => {
                    let _ = self.store.delete(new_id);
                    return Err(e);
                }
            },
        };
        project.id = new_id;
        project.settings.name = name;
        let summary = self.save_copy(new_id, &project)?;
        self.emit_list_changed(out);
        Ok(ReplyValue::Saved { project: summary })
    }

    /// Write the document of a copy `ProjectStore::duplicate` just made. On failure the copy
    /// is deleted: it would list as a second project with the old name and content.
    fn save_copy(&mut self, id: ProjectId, project: &Project) -> CmdResult<ProjectSummary> {
        let saved = self
            .serialize(project)
            .and_then(|json| self.store.save(id, &json).map_err(store_err));
        if saved.is_err() {
            let _ = self.store.delete(id);
        }
        saved
    }

    /// `ExportBundle`: the project's document (the open one: its current state, plugin
    /// states included) and, with `include_media`, its media (its `media/` files and copies
    /// of its external references), packed (see [`bundle`]).
    fn export_bundle(
        &mut self,
        id: ProjectId,
        path: Option<&str>,
        include_media: bool,
        hide_devices: bool,
    ) -> CmdResult<ReplyValue> {
        let current = self.doc.as_ref().map(|d| d.project.id);
        let mut project = if current == Some(id) {
            self.doc.as_ref().expect("checked").project.clone()
        } else {
            let json = self.store.load(id).map_err(store_err)?;
            file::load(&json).map_err(file_err)?
        };
        let name = project.settings.name.clone();
        if hide_devices {
            project.strip_devices();
        }
        let mut media = std::collections::BTreeMap::new();
        if include_media {
            self.collect_bundle_media(id, &mut project, &mut media)?;
        }
        let json = if current == Some(id) {
            self.serialize(&project)?
        } else {
            file::save(&project, &self.config.app_version).map_err(file_err)?
        };
        let mut entries = vec![(bundle::DOCUMENT.to_string(), json.into_bytes())];
        entries.extend(media);
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

    /// The media files of a bundle: the project's `media/` folder, plus a copy of each
    /// readable external reference at its `MediaRef::file` (made a project copy in
    /// `project`, the packed document).
    fn collect_bundle_media(
        &mut self,
        id: ProjectId,
        project: &mut Project,
        media: &mut std::collections::BTreeMap<String, Vec<u8>>,
    ) -> CmdResult<()> {
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
                    media.insert(e.path, bytes);
                }
            }
        }
        for m in project.media.values_mut() {
            let MediaLocation::External { path } = &m.location else {
                continue;
            };
            // Unreadable (missing, or a host without OS files): stays a reference.
            let Ok(bytes) = self.library.read_external(path) else {
                continue;
            };
            media.insert(m.file.clone(), bytes);
            m.location = MediaLocation::Project;
        }
        Ok(())
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
        let mut project = file::load(json).map_err(import_err)?;
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
    pub fn load_project(&mut self, project: Project, now: u64, out: &mut dyn MessageSink) {
        self.load_project_with(project, now, false, out);
    }

    /// `safe`: plugin devices are held as placeholders (safe mode).
    fn load_project_with(
        &mut self,
        project: Project,
        now: u64,
        safe: bool,
        out: &mut dyn MessageSink,
    ) {
        // Replaced without `autosave_before_switch` (an extension's adopt): deleted all the same.
        if self.discarded.is_some_and(|d| d != project.id) {
            self.close_discarded(out);
        }
        self.engine_on_project_load(project.id, now);
        self.engine.reset();
        self.plugin_states.reset();
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
        // The engine is silent until every plugin and media is in (`loading`).
        self.begin_load();
        self.doc = Some(OpenDoc {
            project: project.clone(),
            history: History::new(self.config.history_depth),
            dirty: false,
            last_edit_ms: now,
        });
        event(
            out,
            Event::ProjectLoaded {
                project: Box::new(without_plugin_states(project)),
            },
        );
        self.engine_emit_status(false, out);
        // After `ProjectLoaded`, which is always the first event of a load.
        self.ext_engine_changed(out);
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
        // Session marker and version clock. Written BEFORE the devices
        // (plugins) are instantiated, so a crash while loading them counts as a crash
        // (no reopen on launch, recovery offered).
        self.versions_on_load(now);
        let doc = self.doc.as_ref().expect("just set");
        self.media.sync(&mut self.bridge, Some(&doc.project));
        self.engine.graph_dirty = true;
        self.publish_if_due(now, true, out);
        self.load_tick(out);
        // Hosting/joined sessions of another project pause; this one resumes
        // sharing or reconnects.
        let pid = self.doc.as_ref().expect("just set").project.id;
        self.ext_project_loaded(pid, out);
    }
}

/// A project name as a file name: path separators and control/reserved characters become
/// `_`; never empty.
pub fn bundle_file_name(name: &str) -> String {
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

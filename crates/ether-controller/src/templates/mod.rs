//! Project and track templates (v0.3, owned by the `templates` node; file format
//! `ether_model::template`, protocol `ether_protocol::templates`; CONTRACTS.md §13.10).
//!
//! - **Library** ([`files`]): user templates are files under `<user root>/Templates/`
//!   (`Projects/`, `Tracks/`, `default.json`); factory track templates ([`factory`]) are
//!   built in code, read-only. `List` = factory then user, each sorted by name
//!   (case-insensitive). Names are unique per kind (case-insensitive): a save replaces an
//!   existing one only with `overwrite`, `Rename` refuses a taken name. Every library change
//!   emits `TemplateEvent::Changed`. Hosts without a user library list factory templates
//!   only and reply `Unsupported` to the writes.
//! - **SaveTracks** ([`collect`]): the tracks (with group children), their devices, racks,
//!   pads, modulators, sends between them and track automation; plugin states from the live
//!   instances; samples as library references (project copies are written to the user
//!   library's `Samples/`, like presets).
//! - **SaveProject**: the whole document (chat left out, plugin states from the live
//!   instances), its media as library references.
//! - **Insert** ([`insert`]): a document command. `dispatch` calls
//!   [`EtherController::template_insert_command`] first: it loads the template, imports
//!   its samples (matched by hash first) and applies the insert in one gesture (one undo
//!   step). Inside an `Edit::Batch`, [`EtherController::template_scope`] loads the
//!   templates the batch inserts for the duration of the batch ([`TemplateScope`]; the
//!   document handler only gets a `DocCtx`), and sample imports join the message's
//!   gesture.
//! - **NewProject**: like `Project::Create` (`project.rs`), then the template's document
//!   with the new id and name, its media copied from the library into the new project
//!   (unavailable ones are left missing, with a warning). `template: None` = the default
//!   project template, or an empty project when there is none (or it is gone).

mod collect;
mod factory;
mod files;
mod insert;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use ether_core::protocol::media::{BrowseLocation, MediaCommand, MediaSource};
use ether_core::protocol::model::template::{
    Template, TemplateBody, TemplateFile, save_template_at,
};
use ether_core::protocol::model::*;
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::templates::{
    TemplateCommand, TemplateEvent, TemplateId, TemplateInfo, TemplateKind,
};
use ether_core::protocol::{ClientMessage, Command, Event, NotificationLevel, ReplyValue};

use crate::doc::DocCtx;
use crate::handlers::{event, no_project, notify, store_err};
use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, internal, invalid, invalid_state, not_found, unsupported};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

use files::{Location, file_stem, info, locate, normalize_meta, user_id, user_path};
pub(crate) use insert::Prepared;

/// User-library folder for template (and preset) sample copies.
const SAMPLES_DIR: &str = "Samples";

thread_local! {
    /// Templates loaded for the command being dispatched (see the module docs).
    static PREPARED: RefCell<BTreeMap<TemplateId, Rc<Prepared>>> =
        const { RefCell::new(BTreeMap::new()) };
}

/// Makes loaded templates visible to [`insert`] for the duration of one command's
/// dispatch. Restores the previous set on drop.
pub(crate) struct TemplateScope(BTreeMap<TemplateId, Rc<Prepared>>);

impl TemplateScope {
    fn enter(prepared: BTreeMap<TemplateId, Rc<Prepared>>) -> Self {
        Self(PREPARED.with(|p| p.replace(prepared)))
    }
}

impl Drop for TemplateScope {
    fn drop(&mut self) {
        let prev = std::mem::take(&mut self.0);
        PREPARED.with(|p| *p.borrow_mut() = prev);
    }
}

/// `Template::Insert` (document command, from `doc::apply`).
pub(crate) fn insert(
    ctx: &mut DocCtx,
    template: &TemplateId,
    seed: TrackId,
    parent: Option<TrackId>,
    before: Option<TrackId>,
) -> CmdResult<()> {
    let first: TrackId = derive_id(seed, 0);
    if ctx.p().tracks.contains_key(&first) {
        return Ok(());
    }
    let prepared = PREPARED
        .with(|p| p.borrow().get(template).cloned())
        .ok_or_else(|| invalid_state(format!("template {template} is not loaded")))?;
    insert::insert(ctx, &prepared, seed, parent, before)
}

fn changed(out: &mut dyn MessageSink) {
    event(
        out,
        Event::Template {
            event: TemplateEvent::Changed,
        },
    );
}

fn by_name(a: &TemplateInfo, b: &TemplateInfo) -> std::cmp::Ordering {
    a.name
        .to_lowercase()
        .cmp(&b.name.to_lowercase())
        .then_with(|| a.id.cmp(&b.id))
}

/// A template id's file for display in errors.
fn kind_of(template: &Template) -> TemplateKind {
    files::body_kind(&template.body)
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// Runtime `Template` commands (`Insert` goes through `doc::apply`).
    pub(crate) fn template_command(
        &mut self,
        command: &TemplateCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        match command {
            TemplateCommand::List { kind } => Ok(ReplyValue::Templates {
                templates: self.template_list(*kind),
            }),
            TemplateCommand::SaveProject {
                name,
                meta,
                overwrite,
            } => {
                let template = self.template_save_project(name, meta, *overwrite, now)?;
                changed(out);
                Ok(ReplyValue::Template { template })
            }
            TemplateCommand::SaveTracks {
                tracks,
                name,
                meta,
                overwrite,
            } => {
                let template = self.template_save_tracks(tracks, name, meta, *overwrite, now)?;
                changed(out);
                Ok(ReplyValue::Template { template })
            }
            TemplateCommand::Insert { .. } => Err(internal(
                "Template::Insert is a document command (dispatch)",
            )),
            TemplateCommand::NewProject { id, name, template } => {
                self.template_new_project(*id, name, template.as_ref(), now, out)
            }
            TemplateCommand::Rename { template, name } => {
                let template = self.template_rename(template, name, now)?;
                changed(out);
                Ok(ReplyValue::Template { template })
            }
            TemplateCommand::Delete { template } => {
                let (root, _, path) = self.template_user_file(template)?;
                self.library.remove_file(&root, &path).map_err(store_err)?;
                if files::read_default(&mut self.library, &root).as_deref() == Some(template) {
                    files::write_default(&mut self.library, &root, None).map_err(store_err)?;
                }
                changed(out);
                Ok(ReplyValue::Unit)
            }
            TemplateCommand::SetDefault { template } => {
                let root = self.template_user_root()?;
                if let Some(t) = template {
                    let file = self.template_read(t)?;
                    if kind_of(&file.template) != TemplateKind::Project {
                        return Err(invalid(format!("{t} is not a project template")));
                    }
                }
                files::write_default(&mut self.library, &root, template.as_deref())
                    .map_err(store_err)?;
                changed(out);
                Ok(ReplyValue::Unit)
            }
        }
    }

    fn template_user_root(&self) -> CmdResult<String> {
        self.library
            .user_root()
            .ok_or_else(|| unsupported("this host has no writable user library for templates"))
    }

    /// A user template's root, kind and path (factory templates are read-only).
    fn template_user_file(&mut self, id: &str) -> CmdResult<(String, TemplateKind, String)> {
        match locate(id).map_err(store_err)? {
            Location::Factory { .. } => Err(invalid("factory templates are read-only")),
            Location::User { kind, path } => {
                let root = self.template_user_root()?;
                Ok((root, kind, path))
            }
        }
    }

    /// Read any template by id.
    fn template_read(&mut self, id: &str) -> CmdResult<TemplateFile> {
        match locate(id).map_err(store_err)? {
            Location::Factory { slug } => factory::get(&slug)
                .map(|template| TemplateFile {
                    format: String::new(),
                    version: 0,
                    ether_version: 0,
                    app_version: String::new(),
                    saved_ms: 0,
                    template,
                })
                .ok_or_else(|| not_found(format!("template {id}"))),
            Location::User { kind, path } => {
                let root = self.template_user_root()?;
                let file = files::read_user(&mut self.library, &root, &path).map_err(|e| {
                    if self.library.read(&root, &path).is_err() {
                        not_found(format!("template {id}"))
                    } else {
                        invalid(format!("invalid template {id}: {e}"))
                    }
                })?;
                if kind_of(&file.template) != kind {
                    return Err(invalid(format!("template {id} is of another kind")));
                }
                Ok(file)
            }
        }
    }

    fn template_list(&mut self, kind: Option<TemplateKind>) -> Vec<TemplateInfo> {
        let root = self.library.user_root();
        let default = root
            .as_deref()
            .and_then(|r| files::read_default(&mut self.library, r));
        let wanted = |k: TemplateKind| kind.is_none_or(|x| x == k);
        let mut factory: Vec<TemplateInfo> = factory::all()
            .into_iter()
            .filter(|(_, t)| wanted(kind_of(t)))
            .map(|(slug, template)| {
                let file = TemplateFile {
                    format: String::new(),
                    version: 0,
                    ether_version: 0,
                    app_version: String::new(),
                    saved_ms: 0,
                    template,
                };
                info(
                    format!("{}{slug}", files::FACTORY),
                    &file,
                    default.as_deref(),
                )
            })
            .collect();
        factory.sort_by(by_name);
        let mut user = Vec::new();
        if let Some(root) = root {
            for k in [TemplateKind::Project, TemplateKind::Tracks] {
                if !wanted(k) {
                    continue;
                }
                for (stem, file) in files::user_templates(&mut self.library, &root, k) {
                    user.push(info(user_id(k, &stem), &file, default.as_deref()));
                }
            }
        }
        user.sort_by(by_name);
        factory.extend(user);
        factory
    }

    /// The stem of another user template of `kind` named `name` (case-insensitive).
    fn template_named(
        &mut self,
        root: &str,
        kind: TemplateKind,
        name: &str,
        except: Option<&str>,
    ) -> Option<String> {
        let name = name.to_lowercase();
        files::user_templates(&mut self.library, root, kind)
            .into_iter()
            .find(|(stem, f)| {
                Some(stem.as_str()) != except && f.template.name.to_lowercase() == name
            })
            .map(|(stem, _)| stem)
    }

    /// A free stem `<stem>[ n]` of `kind` (never an existing file, except `except`).
    fn template_free_stem(
        &mut self,
        root: &str,
        kind: TemplateKind,
        stem: &str,
        except: Option<&str>,
    ) -> String {
        let taken: Vec<String> = files::user_stems(&mut self.library, root, kind)
            .into_iter()
            .map(|s| s.to_lowercase())
            .collect();
        let except = except.map(str::to_lowercase);
        (1..)
            .map(|n| {
                if n == 1 {
                    stem.to_string()
                } else {
                    format!("{stem} {n}")
                }
            })
            .find(|s| {
                let lower = s.to_lowercase();
                except.as_deref() == Some(lower.as_str()) || !taken.contains(&lower)
            })
            .expect("unbounded")
    }

    /// Write a new or replaced user template.
    fn template_write_new(
        &mut self,
        root: &str,
        name: &str,
        meta: &PresetMeta,
        overwrite: bool,
        body: TemplateBody,
        now: u64,
    ) -> CmdResult<TemplateInfo> {
        let kind = files::body_kind(&body);
        let existing = self.template_named(root, kind, name, None);
        if existing.is_some() && !overwrite {
            return Err(invalid(format!(
                "a template named \"{name}\" already exists"
            )));
        }
        let stem = match existing {
            Some(s) => s,
            None => self.template_free_stem(root, kind, &file_stem(name), None),
        };
        let template = Template {
            name: name.to_string(),
            meta: normalize_meta(meta),
            body,
        };
        self.template_write(root, kind, &stem, &template, now)
    }

    fn template_write(
        &mut self,
        root: &str,
        kind: TemplateKind,
        stem: &str,
        template: &Template,
        now: u64,
    ) -> CmdResult<TemplateInfo> {
        let json = save_template_at(template, &self.config.app_version, now)
            .map_err(|e| internal(e.to_string()))?;
        self.library
            .write_file(root, &user_path(kind, stem), json.as_bytes())
            .map_err(store_err)?;
        let default = files::read_default(&mut self.library, root);
        let id = user_id(kind, stem);
        Ok(TemplateInfo {
            default: default.as_deref() == Some(id.as_str()),
            factory: false,
            id,
            kind,
            name: template.name.clone(),
            meta: template.meta.clone(),
            modified_ms: now,
        })
    }

    fn template_name(name: &str) -> CmdResult<String> {
        let name = name.trim();
        if name.is_empty() {
            return Err(invalid("a template needs a name"));
        }
        Ok(name.to_string())
    }

    fn template_save_tracks(
        &mut self,
        tracks: &[TrackId],
        name: &str,
        meta: &PresetMeta,
        overwrite: bool,
        now: u64,
    ) -> CmdResult<TemplateInfo> {
        let root = self.template_user_root()?;
        let name = Self::template_name(name)?;
        let doc = self.doc.as_ref().ok_or_else(no_project)?;
        let pid = doc.project.id;
        let mut entities = collect::collect_tracks(&doc.project, tracks)?;
        let mut media = Vec::new();
        for e in &mut entities {
            if let Entity::Device(d) = e {
                match &mut d.kind {
                    DeviceKind::Plugin { plugin } => {
                        if self.engine.node(d.id).is_some()
                            && let Ok(Some(state)) = self.bridge.plugin_state(d.id)
                        {
                            plugin.state = Some(state);
                        }
                    }
                    DeviceKind::Builtin { device } => media.extend(device.media()),
                }
            }
        }
        let samples = self.template_sample_refs(&root, pid, &media)?;
        let body = TemplateBody::Tracks { entities, samples };
        self.template_write_new(&root, &name, meta, overwrite, body, now)
    }

    fn template_save_project(
        &mut self,
        name: &str,
        meta: &PresetMeta,
        overwrite: bool,
        now: u64,
    ) -> CmdResult<TemplateInfo> {
        let root = self.template_user_root()?;
        let name = Self::template_name(name)?;
        let doc = self.doc.as_ref().ok_or_else(no_project)?;
        let mut project = doc.project.clone();
        project.chat.clear();
        for d in project.devices.values_mut() {
            if let DeviceKind::Plugin { plugin } = &mut d.kind
                && self.engine.node(d.id).is_some()
                && let Ok(Some(state)) = self.bridge.plugin_state(d.id)
            {
                plugin.state = Some(state);
            }
        }
        let media: Vec<MediaId> = project.media.keys().copied().collect();
        let samples = self.template_sample_refs(&root, project.id, &media)?;
        let body = TemplateBody::Project {
            project: Box::new(project),
            samples,
        };
        self.template_write_new(&root, &name, meta, overwrite, body, now)
    }

    /// Library references for project media: library files and external references inside
    /// a library root keep their location; anything else is copied to the user library's
    /// `Samples/` (content-addressed by hash, shared with presets).
    fn template_sample_refs(
        &mut self,
        root: &str,
        pid: ProjectId,
        media: &[MediaId],
    ) -> CmdResult<Vec<PresetSample>> {
        let doc = self.doc.as_ref().ok_or_else(no_project)?;
        let refs: Vec<MediaRef> = media
            .iter()
            .filter_map(|m| doc.project.media.get(m).cloned())
            .collect();
        let mut roots: Vec<String> = self
            .library
            .roots()
            .into_iter()
            .filter_map(|r| match r.location {
                BrowseLocation::Library { id } => Some(id),
                BrowseLocation::ProjectMedia => None,
            })
            .collect();
        roots.push(root.to_string());
        let mut out: Vec<PresetSample> = Vec::new();
        for m in refs {
            if out.iter().any(|s| s.media == m.id) {
                continue;
            }
            let in_library = match &m.location {
                MediaLocation::External { path } => roots.iter().find_map(|r| {
                    let base = self.library.external_path(r, "")?;
                    let rel = path.strip_prefix(&base)?.trim_start_matches('/');
                    (!rel.is_empty()).then(|| (r.clone(), rel.to_string()))
                }),
                MediaLocation::Project => None,
            };
            let (location, path) = match in_library {
                Some(found) => found,
                None => {
                    let bytes =
                        crate::media::read_media_bytes(&mut self.store, &mut self.library, pid, &m)
                            .map_err(store_err)?;
                    let hash = m
                        .hash
                        .clone()
                        .unwrap_or_else(|| crate::content_hash(&bytes));
                    let short: String = hash.chars().take(12).collect();
                    let path = format!("{SAMPLES_DIR}/{short}-{}", file_stem(&m.name));
                    if self.library.read(root, &path).is_err() {
                        self.library
                            .write_file(root, &path, &bytes)
                            .map_err(store_err)?;
                    }
                    (root.to_string(), path)
                }
            };
            out.push(PresetSample {
                media: m.id,
                location,
                path,
                hash: m.hash.clone(),
            });
        }
        Ok(out)
    }

    fn template_rename(&mut self, id: &str, name: &str, now: u64) -> CmdResult<TemplateInfo> {
        let name = Self::template_name(name)?;
        let (root, kind, path) = self.template_user_file(id)?;
        let mut file = self.template_read(id)?;
        let stem = id
            .split_once('/')
            .map(|(_, s)| s.to_string())
            .unwrap_or_default();
        if self
            .template_named(&root, kind, &name, Some(&stem))
            .is_some()
        {
            return Err(invalid(format!(
                "a template named \"{name}\" already exists"
            )));
        }
        file.template.name = name.clone();
        let new_stem = self.template_free_stem(&root, kind, &file_stem(&name), Some(&stem));
        let info = self.template_write(&root, kind, &stem, &file.template, now)?;
        if new_stem == stem {
            return Ok(info);
        }
        self.library
            .rename_file(&root, &path, &user_path(kind, &new_stem))
            .map_err(store_err)?;
        let new_id = user_id(kind, &new_stem);
        let was_default = files::read_default(&mut self.library, &root).as_deref() == Some(id);
        if was_default {
            files::write_default(&mut self.library, &root, Some(&new_id)).map_err(store_err)?;
        }
        Ok(TemplateInfo {
            id: new_id,
            default: was_default,
            ..info
        })
    }

    // ─── NewProject ─────────────────────────────────────────────────────────────────────

    fn template_new_project(
        &mut self,
        id: ProjectId,
        name: &str,
        template: Option<&TemplateId>,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let mut warnings = Vec::new();
        let source = match template {
            Some(t) => Some(self.template_read(t)?),
            None => {
                let default = self
                    .library
                    .user_root()
                    .and_then(|r| files::read_default(&mut self.library, &r));
                match default {
                    Some(t) => match self.template_read(&t) {
                        Ok(f) => Some(f),
                        Err(e) => {
                            warnings.push(format!(
                                "the default project template is unavailable ({}); created an empty project",
                                e.message
                            ));
                            None
                        }
                    },
                    None => None,
                }
            }
        };
        let source = match source {
            None => None,
            Some(f) => match f.template.body {
                TemplateBody::Project { project, samples } => Some((*project, samples)),
                TemplateBody::Tracks { .. } => {
                    return Err(invalid(format!(
                        "\"{}\" is a track template",
                        f.template.name
                    )));
                }
            },
        };
        let reply = self.create_with(id, name, now, out, |ctl, project| {
            let Some((template, samples)) = source else {
                return Ok(());
            };
            *project = template;
            project.chat.clear();
            for m in project.media.values_mut() {
                let Some(s) = samples.iter().find(|s| s.media == m.id) else {
                    continue;
                };
                match ctl.library.read(&s.location, &s.path) {
                    Ok(bytes) => {
                        ctl.store.write(id, &m.file, &bytes).map_err(store_err)?;
                        m.location = MediaLocation::Project;
                    }
                    Err(e) => {
                        warnings.push(format!("template sample {} is unavailable ({e})", s.path))
                    }
                }
            }
            Ok(())
        })?;
        for w in warnings {
            notify(out, NotificationLevel::Warning, w);
        }
        Ok(reply)
    }

    // ─── Insert ─────────────────────────────────────────────────────────────────────────

    /// Load track template `id` for an insert: its entities, and its samples matched to
    /// project media by hash or imported (in `gesture`; unavailable ones are left out with a
    /// warning).
    fn template_prepare(
        &mut self,
        id: &TemplateId,
        gesture: Option<GestureId>,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<Prepared> {
        let file = self.template_read(id)?;
        let name = file.template.name.clone();
        let TemplateBody::Tracks { entities, samples } = file.template.body else {
            return Err(invalid(format!("\"{name}\" is a project template")));
        };
        let mut media = BTreeMap::new();
        for s in &samples {
            let existing = self.doc.as_ref().and_then(|doc| {
                let hash = s.hash.as_deref()?;
                doc.project
                    .media
                    .values()
                    .find(|m| m.hash.as_deref() == Some(hash))
                    .map(|m| m.id)
            });
            if let Some(m) = existing {
                media.insert(s.media, m);
                continue;
            }
            let new: MediaId = self.ids.next(now);
            let msg = ClientMessage {
                id: 0,
                gesture,
                command: Command::Media(MediaCommand::Import {
                    id: new,
                    source: MediaSource::Location {
                        location: BrowseLocation::Library {
                            id: s.location.clone(),
                        },
                        path: s.path.clone(),
                    },
                }),
            };
            match self.dispatch(&msg, now, out) {
                Ok(_) => {
                    media.insert(s.media, new);
                }
                Err(e) => notify(
                    out,
                    NotificationLevel::Warning,
                    format!(
                        "template \"{name}\": sample {} is unavailable ({})",
                        s.path, e.message
                    ),
                ),
            }
        }
        Ok(Prepared { entities, media })
    }

    /// `dispatch` hook: a lone `Template::Insert` loads its template, imports its samples
    /// and inserts it, all in one gesture (one undo step). `None` for any other command.
    pub(crate) fn template_insert_command(
        &mut self,
        msg: &ClientMessage,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> Option<CmdResult<ReplyValue>> {
        let Command::Template(TemplateCommand::Insert {
            template,
            seed,
            parent,
            before,
        }) = &msg.command
        else {
            return None;
        };
        let doc = match self.doc.as_ref() {
            Some(d) => d,
            None => return Some(Err(no_project())),
        };
        let first: TrackId = derive_id(*seed, 0);
        if doc.project.tracks.contains_key(&first) {
            // Retried message.
            return Some(Ok(ReplyValue::Unit));
        }
        let own = msg.gesture.is_none();
        let gesture = msg.gesture.unwrap_or_else(|| self.new_gesture());
        let result = self
            .template_prepare(template, Some(gesture), now, out)
            .and_then(|prepared| {
                let _scope =
                    TemplateScope::enter(BTreeMap::from([(template.clone(), Rc::new(prepared))]));
                self.edit_with("Insert Template", Some(gesture), now, out, |ctx| {
                    insert(ctx, template, *seed, *parent, *before)
                })
            });
        if own && let Some(doc) = self.doc.as_mut() {
            doc.history.end_gesture(gesture);
        }
        Some(result.map(|()| ReplyValue::Unit))
    }

    /// `dispatch` hook: the templates an `Edit::Batch` inserts, loaded for its duration
    /// (`None` for anything else).
    pub(crate) fn template_scope(
        &mut self,
        command: &Command,
        gesture: Option<GestureId>,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<Option<TemplateScope>> {
        let Command::Edit(EditCommand::Batch { commands, .. }) = command else {
            return Ok(None);
        };
        let mut prepared = BTreeMap::new();
        for c in commands {
            if let Command::Template(TemplateCommand::Insert { template, seed, .. }) = c {
                let first: TrackId = derive_id(*seed, 0);
                let exists = self
                    .doc
                    .as_ref()
                    .is_some_and(|d| d.project.tracks.contains_key(&first));
                if exists || prepared.contains_key(template) {
                    continue;
                }
                let p = self.template_prepare(template, gesture, now, out)?;
                prepared.insert(template.clone(), Rc::new(p));
            }
        }
        Ok((!prepared.is_empty()).then(|| TemplateScope::enter(prepared)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_restores_previous() {
        let p = Rc::new(Prepared {
            entities: Vec::new(),
            media: BTreeMap::new(),
        });
        let outer = TemplateScope::enter(BTreeMap::from([("tracks/a".to_string(), p.clone())]));
        {
            let _inner =
                TemplateScope::enter(BTreeMap::from([("tracks/b".to_string(), p.clone())]));
            assert!(PREPARED.with(|x| x.borrow().contains_key("tracks/b")));
            assert!(!PREPARED.with(|x| x.borrow().contains_key("tracks/a")));
        }
        assert!(PREPARED.with(|x| x.borrow().contains_key("tracks/a")));
        drop(outer);
        assert!(PREPARED.with(|x| x.borrow().is_empty()));
    }
}

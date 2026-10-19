//! Resolving missing plugins and audio files (`import-resolver`, owner request; protocol
//! `ether_protocol::resolver`, CONTRACTS.md §14.2). Always compiled (not only with the
//! `import` feature): the resolver also serves projects opened later.
//!
//! - **Propose** ([`EtherController::resolve_command`]): plugin devices whose plugin is not
//!   in the UI's installed list, grouped per plugin, each with deterministic candidates
//!   ([`plugins::candidates`]); and the missing audio files (the media pipeline's missing
//!   set, plus the files a just-finished import could not find).
//! - **SearchMedia**: a job stepped from the tick ([`media::MediaSearch`]): library roots,
//!   the imported project's folders ([`ImportHints::dir`]) and an optional OS folder.
//! - **Apply**: every choice in one undo step "Resolve Missing Items": plugin devices switch
//!   plugin (state carried or dropped, params and their automation lanes remapped), media
//!   are relinked (external path, or copied into the project when the host can't reference
//!   the file).
//!
//! The import job leaves [`ImportHints`] (source folder, missing files, plugin parameter
//! names from the source) for the project it opened; they live until another project opens.

mod devices;
pub(crate) mod media;
pub(crate) mod normalize;
pub(crate) mod plugins;
mod replace;

use std::collections::{BTreeMap, BTreeSet};

use ether_core::protocol::media::{BrowseLocation, MediaSource};
use ether_core::protocol::model::file::MEDIA_DIR;
use ether_core::protocol::model::{
    AutomationTarget, DeviceId, DeviceKind, Entity, EntityKey, EntityUpdate, MediaChange, MediaId,
    MediaLocation, ParamId, PluginFormat, PluginInstance, ProjectId,
};
use ether_core::protocol::resolver::{
    DeviceResolution, MediaResolution, MissingDeviceGroup, MissingMedia, MissingPlugin, ParamName,
    ParamPlan, PluginParams, PluginResolution, Replacement, ResolveCommand, ResolveEvent,
    ResolveProposal, SavedParam,
};
use ether_core::protocol::{Event, NotificationLevel, ReplyValue};

use crate::handlers::{event, no_project, notify, store_err};
use crate::store::{Library, ProjectStore, StoreError, check_relative_path};
use crate::tx::{CmdResult, invalid};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

use self::media::{Expected, Folder, MediaSearch};

/// What an import knows about the project it just opened.
#[derive(Clone, Debug, Default)]
pub(crate) struct ImportHints {
    pub project: Option<ProjectId>,
    /// Folder of the source project file (desktop path imports).
    pub dir: Option<String>,
    /// Media the import could not find, with the location written then.
    pub missing: BTreeMap<MediaId, MediaLocation>,
    /// Plugin parameter names from the source, per device.
    pub param_names: BTreeMap<DeviceId, Vec<(u32, String)>>,
}

#[derive(Default)]
pub(crate) struct ResolverState {
    search: Option<Box<MediaSearch>>,
    hints: ImportHints,
    /// Descriptors of installed plugins probed for `Replace` (by format and upper-case id).
    probed: BTreeMap<(PluginFormat, String), ether_core::protocol::devices::DeviceDescriptor>,
}

impl ResolverState {
    /// The import opened `hints.project`. Only the private DAW-import overlay sets these, so
    /// this is unused in the OSS build.
    #[allow(dead_code)]
    pub(crate) fn set_import_hints(&mut self, hints: ImportHints) {
        self.search = None;
        self.hints = hints;
    }
}

/// A plugin resolution checked against the document, ready to apply.
pub(crate) struct PluginEdit {
    device: DeviceId,
    /// What the device becomes (a plugin, or a built-in replacing a missing device).
    kind: DeviceKind,
    rename: Option<String>,
    /// `None` = keep params as they are (state carried); else `from → to`.
    map: Option<Vec<(ParamId, ParamId)>>,
    /// The device's parameter values after the switch (else they follow `map`).
    params: Option<BTreeMap<ParamId, f64>>,
    /// The new device has no sidechain input: its sidechain source goes.
    clear_sidechain: bool,
}

/// A media resolution with its bytes checked (and copied when needed).
struct MediaEdit {
    media: MediaId,
    location: MediaLocation,
    hash: String,
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    fn hints(&self) -> Option<&ImportHints> {
        let id = self.doc.as_ref()?.project.id;
        (self.resolver.hints.project == Some(id)).then_some(&self.resolver.hints)
    }

    /// Missing media: the media pipeline's set, plus what the import couldn't find (while
    /// still referenced where it was).
    fn missing_media(&self) -> Vec<MediaId> {
        let Some(doc) = self.doc.as_ref() else {
            return Vec::new();
        };
        let mut ids: BTreeSet<MediaId> = self.media.missing().into_iter().collect();
        if let Some(h) = self.hints() {
            for (id, at) in &h.missing {
                if doc.project.media.get(id).is_some_and(|m| &m.location == at) {
                    ids.insert(*id);
                }
            }
        }
        ids.retain(|id| doc.project.media.contains_key(id));
        ids.into_iter().collect()
    }

    pub(crate) fn resolve_command(
        &mut self,
        c: &ResolveCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        self.doc.as_ref().ok_or_else(no_project)?;
        match c {
            ResolveCommand::Propose {
                installed,
                params,
                plugins,
            } => Ok(ReplyValue::ResolveProposal {
                proposal: self.propose(installed, params, *plugins),
            }),
            ResolveCommand::SearchMedia { media, folder } => {
                self.start_media_search(media.as_deref(), folder.as_deref(), out)?;
                Ok(ReplyValue::Unit)
            }
            ResolveCommand::CancelSearch => {
                self.resolver.search = None;
                Ok(ReplyValue::Unit)
            }
            ResolveCommand::Apply {
                plugins,
                media,
                devices,
            } => {
                self.apply_resolution(plugins, media, devices, now, out)?;
                Ok(ReplyValue::Unit)
            }
            ResolveCommand::Replace {
                device,
                plugin,
                builtin,
                retain,
                preview,
            } => {
                let to = match (plugin, builtin) {
                    (Some(plugin), None) => Replacement::Plugin {
                        plugin: plugin.clone(),
                    },
                    (None, Some(device)) => Replacement::Builtin { device: *device },
                    _ => return Err(invalid("replace with either a plugin or a built-in device")),
                };
                Ok(ReplyValue::PluginReplaced {
                    report: self.replace_device(*device, &to, *retain, *preview, now, out)?,
                })
            }
        }
    }

    /// Known parameter names: the UI's, plus those of installed plugins loaded in the
    /// project (their engine descriptors).
    fn known_params(
        &mut self,
        installed: &BTreeSet<(PluginFormat, String)>,
        extra: &[PluginParams],
    ) -> BTreeMap<(PluginFormat, String), Vec<ParamName>> {
        let mut known: BTreeMap<(PluginFormat, String), Vec<ParamName>> = extra
            .iter()
            .map(|p| ((p.format, p.id.clone()), p.params.clone()))
            .collect();
        let loaded: Vec<(DeviceId, PluginFormat, String)> = self
            .doc
            .as_ref()
            .map(|d| {
                d.project
                    .devices
                    .values()
                    .filter_map(|d| match &d.kind {
                        DeviceKind::Plugin { plugin }
                            if installed
                                .contains(&(plugin.format, plugin.plugin_id.to_uppercase())) =>
                        {
                            Some((d.id, plugin.format, plugin.plugin_id.clone()))
                        }
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        for (device, format, id) in loaded {
            if known.contains_key(&(format, id.clone())) {
                continue;
            }
            if let Some(desc) = self.bridge.descriptor(device)
                && !desc.params.is_empty()
            {
                known.insert(
                    (format, id),
                    desc.params
                        .iter()
                        .map(|p| ParamName {
                            id: p.id.0,
                            name: p.name.clone(),
                        })
                        .collect(),
                );
            }
        }
        known
    }

    fn saved_params(&self, device: DeviceId) -> Vec<SavedParam> {
        let Some(doc) = self.doc.as_ref() else {
            return Vec::new();
        };
        let names: BTreeMap<u32, &str> = self
            .hints()
            .and_then(|h| h.param_names.get(&device))
            .map(|v| v.iter().map(|(i, n)| (*i, n.as_str())).collect())
            .unwrap_or_default();
        doc.project
            .devices
            .get(&device)
            .map(|d| {
                d.params
                    .iter()
                    .map(|(id, v)| SavedParam {
                        id: id.0,
                        value: *v,
                        name: names.get(&id.0).map(|s| s.to_string()),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn propose(
        &mut self,
        installed: &[ether_core::protocol::plugins::PluginDescriptor],
        extra: &[PluginParams],
        with_plugins: bool,
    ) -> ResolveProposal {
        let installed_keys: BTreeSet<(PluginFormat, String)> = installed
            .iter()
            .map(|p| (p.format, p.id.to_uppercase()))
            .collect();
        let mut plugins = Vec::new();
        if with_plugins {
            let known = self.known_params(&installed_keys, extra);
            // Missing plugins, grouped by (format, id) in device order.
            let mut groups: Vec<((PluginFormat, String), Vec<DeviceId>, PluginInstance)> =
                Vec::new();
            if let Some(doc) = self.doc.as_ref() {
                let mut devices: Vec<_> = doc.project.devices.values().collect();
                devices.sort_by(|a, b| (a.track, &a.order).cmp(&(b.track, &b.order)));
                for d in devices {
                    let DeviceKind::Plugin { plugin } = &d.kind else {
                        continue;
                    };
                    let key = (plugin.format, plugin.plugin_id.to_uppercase());
                    if installed_keys.contains(&key) {
                        continue;
                    }
                    match groups.iter_mut().find(|g| g.0 == key) {
                        Some(g) => g.1.push(d.id),
                        None => groups.push((key, vec![d.id], plugin.clone())),
                    }
                }
            }
            for (_, devices, p) in groups {
                let params = self.saved_params(devices[0]);
                let has_state = p.state.is_some();
                let m = plugins::Missing {
                    format: p.format,
                    id: &p.plugin_id,
                    name: &p.name,
                    vendor: &p.vendor,
                    has_state,
                    params: &params,
                };
                let candidates = plugins::candidates(&m, installed, &known);
                plugins.push(MissingPlugin {
                    devices,
                    format: p.format,
                    plugin_id: p.plugin_id.clone(),
                    name: p.name.clone(),
                    vendor: p.vendor.clone(),
                    version: p.version.clone(),
                    has_state,
                    params,
                    candidates,
                });
            }
        }
        let media = self
            .missing_media()
            .into_iter()
            .filter_map(|id| {
                let m = self.doc.as_ref()?.project.media.get(&id)?;
                Some(MissingMedia {
                    media: id,
                    name: m.name.clone(),
                    path: match &m.location {
                        MediaLocation::External { path } => Some(path.clone()),
                        _ => None,
                    },
                    sample_rate: m.sample_rate,
                    channels: m.channels,
                    frames: m.frames,
                    hashed: m.hash.is_some(),
                })
            })
            .collect();
        let folder_search = !matches!(
            self.library.list_external_dir(""),
            Err(StoreError::Unsupported(_))
        );
        ResolveProposal {
            plugins,
            media,
            devices: self.missing_devices(),
            folder_search,
        }
    }

    /// Missing devices (placeholders), grouped by what they were, in device order.
    fn missing_devices(&self) -> Vec<MissingDeviceGroup> {
        let Some(doc) = self.doc.as_ref() else {
            return Vec::new();
        };
        let mut all: Vec<_> = doc.project.devices.values().collect();
        all.sort_by(|a, b| (a.track, &a.order).cmp(&(b.track, &b.order)));
        let mut groups: Vec<MissingDeviceGroup> = Vec::new();
        for d in all {
            let DeviceKind::Missing { missing } = &d.kind else {
                continue;
            };
            if let Some(g) = groups.iter_mut().find(|g| g.source_id == missing.source_id) {
                g.devices.push(d.id);
                continue;
            }
            groups.push(MissingDeviceGroup {
                devices: vec![d.id],
                source_id: missing.source_id.clone(),
                description: missing.description.clone(),
                origin: missing.origin.clone(),
                role: missing.role,
                params: missing
                    .params
                    .iter()
                    .filter_map(|p| {
                        Some(SavedParam {
                            id: p.id.0,
                            value: *d.params.get(&p.id)?,
                            name: Some(p.name.clone()),
                        })
                    })
                    .collect(),
                candidates: devices::candidates(missing),
            });
        }
        groups
    }

    fn start_media_search(
        &mut self,
        media: Option<&[MediaId]>,
        folder: Option<&str>,
        out: &mut dyn MessageSink,
    ) -> CmdResult<()> {
        let doc = self.doc.as_ref().ok_or_else(no_project)?;
        let ids: Vec<MediaId> = match media {
            Some(ids) => ids.to_vec(),
            None => self.missing_media(),
        };
        let targets: BTreeMap<MediaId, Expected> = ids
            .iter()
            .filter_map(|id| doc.project.media.get(id).map(|m| (*id, Expected::of(m))))
            .collect();
        let mut search = MediaSearch::new(targets);
        if let Some(folder) = folder {
            // Validates it (absolute, a folder; `Unsupported` without OS files).
            self.library.list_external_dir(folder).map_err(store_err)?;
            search.folders.push_back(Folder::External {
                path: folder.to_string(),
                depth: 0,
            });
        }
        if let Some(dir) = self.hints().and_then(|h| h.dir.clone()) {
            for path in media::import_folders(&dir) {
                search
                    .folders
                    .push_back(Folder::External { path, depth: 0 });
            }
        }
        for root in self.library.roots() {
            if let BrowseLocation::Library { id } = root.location {
                search.root_names.insert(id.clone(), root.name.clone());
                search.folders.push_back(Folder::Library {
                    root: id,
                    rel: String::new(),
                    depth: 0,
                });
            }
        }
        if search.targets.is_empty() {
            search.folders.clear();
        }
        self.resolver.search = Some(Box::new(search));
        event(
            out,
            Event::Resolve {
                event: ResolveEvent::MediaProgress { scanned: 0 },
            },
        );
        Ok(())
    }

    /// Called every tick: steps a running media search.
    pub(crate) fn resolve_tick(&mut self, _now: u64, out: &mut dyn MessageSink) {
        let Some(search) = self.resolver.search.as_mut() else {
            return;
        };
        let before = search.scanned;
        if !search.step(&mut self.library, media::UNITS_PER_TICK) {
            if search.scanned / 16 != before / 16 {
                event(
                    out,
                    Event::Resolve {
                        event: ResolveEvent::MediaProgress {
                            scanned: search.scanned,
                        },
                    },
                );
            }
            return;
        }
        let search = self.resolver.search.take().expect("running");
        let scanned = search.scanned;
        for (media, candidates) in search.results() {
            event(
                out,
                Event::Resolve {
                    event: ResolveEvent::MediaCandidates { media, candidates },
                },
            );
        }
        event(
            out,
            Event::Resolve {
                event: ResolveEvent::MediaSearchDone { scanned },
            },
        );
    }

    /// Check one plugin resolution against the document.
    fn plugin_edits(
        &mut self,
        r: &PluginResolution,
        edits: &mut Vec<PluginEdit>,
    ) -> Result<(), String> {
        if r.plugin_id.trim().is_empty() {
            return Err("no plugin id".into());
        }
        for device in &r.devices {
            let doc = self.doc.as_ref().ok_or("no project")?;
            let d = doc
                .project
                .devices
                .get(device)
                .ok_or_else(|| format!("device {device} no longer exists"))?;
            let DeviceKind::Plugin { plugin: old } = &d.kind else {
                return Err(format!("\"{}\" is not a plugin", d.name));
            };
            let old = old.clone();
            let device_name = d.name.clone();
            let params = self.saved_params(*device);
            let m = plugins::Missing {
                format: old.format,
                id: &old.plugin_id,
                name: &old.name,
                vendor: &old.vendor,
                has_state: old.state.is_some(),
                params: &params,
            };
            let plan = match &r.plan {
                ParamPlan::Auto => {
                    plugins::plan(&m, r.format, &r.plugin_id, &r.name, &r.vendor, None).0
                }
                p => p.clone(),
            };
            let keep = matches!(plan, ParamPlan::KeepState);
            if keep && old.state.is_some() && r.format != old.format {
                return Err(format!(
                    "\"{}\": a {} state can't be loaded by a {} plugin",
                    device_name,
                    plugins::format_label(old.format),
                    plugins::format_label(r.format)
                ));
            }
            let map = match plan {
                ParamPlan::Map { map } => {
                    let mut seen = BTreeSet::new();
                    let mut out = Vec::new();
                    for p in map {
                        // One source per target; unknown sources are ignored.
                        if params.iter().any(|s| s.id == p.from) && seen.insert(p.to) {
                            out.push((ParamId(p.from), ParamId(p.to)));
                        }
                    }
                    Some(out)
                }
                _ => None,
            };
            edits.push(PluginEdit {
                device: *device,
                kind: DeviceKind::Plugin {
                    plugin: PluginInstance {
                        format: r.format,
                        plugin_id: r.plugin_id.clone(),
                        name: r.name.clone(),
                        vendor: r.vendor.clone(),
                        version: r.version.clone(),
                        sandboxed: old.sandboxed,
                        state: if keep { old.state.clone() } else { None },
                    },
                },
                rename: (device_name == old.name && !r.name.is_empty() && r.name != device_name)
                    .then(|| r.name.clone()),
                map,
                params: None,
                clear_sidechain: false,
            });
        }
        Ok(())
    }

    /// Read and check one media resolution (copying the file into the project when the
    /// host can't reference it in place).
    fn media_edit(&mut self, media: MediaId, source: &MediaSource) -> Result<MediaEdit, String> {
        let doc = self.doc.as_ref().ok_or("no project")?;
        let pid = doc.project.id;
        let m = doc
            .project
            .media
            .get(&media)
            .cloned()
            .ok_or_else(|| format!("media {media} no longer exists"))?;
        let err = |e: StoreError| store_err(e).message;
        let (bytes, location, name) = match source {
            MediaSource::Path { path } => {
                let bytes = self.library.read_external(path).map_err(err)?;
                (
                    bytes,
                    MediaLocation::External { path: path.clone() },
                    path.clone(),
                )
            }
            MediaSource::Location { location, path } => {
                check_relative_path(path).map_err(err)?;
                match location {
                    BrowseLocation::Library { id: root } => {
                        let bytes = self.library.read(root, path).map_err(err)?;
                        let at = match self.library.external_path(root, path) {
                            Some(p) => MediaLocation::External { path: p },
                            None => MediaLocation::Project,
                        };
                        (bytes, at, path.clone())
                    }
                    BrowseLocation::ProjectMedia => {
                        let bytes = self
                            .store
                            .read(pid, &format!("{MEDIA_DIR}/{path}"))
                            .map_err(err)?;
                        (bytes, MediaLocation::Project, path.clone())
                    }
                }
            }
            MediaSource::Upload { upload } => {
                let (bytes, name) =
                    crate::upload::take_upload(&mut self.uploads, &mut self.store, upload)
                        .map_err(|e| e.message)?;
                (bytes, MediaLocation::Project, name)
            }
            MediaSource::Project { .. } => {
                return Err("relink to a file, not to another media".into());
            }
        };
        let probed = media::probe(bytes.clone(), &name)
            .ok_or_else(|| format!("\"{}\": the file is not readable audio", m.name))?;
        if m.hash.is_some()
            && (probed.sample_rate != m.sample_rate || probed.channels != m.channels)
        {
            return Err(format!(
                "\"{}\" is {} hz, {} ch; the file is {} hz, {} ch",
                m.name, m.sample_rate, m.channels, probed.sample_rate, probed.channels
            ));
        }
        if m.hash.as_deref().is_some_and(|h| h != probed.hash) && self.collab_active() {
            return Err(format!(
                "\"{}\" has different content: in a session, relink to the same file",
                m.name
            ));
        }
        if location == MediaLocation::Project
            && !self.store.read(pid, &m.file).is_ok_and(|b| b == bytes)
        {
            self.store.write(pid, &m.file, &bytes).map_err(err)?;
        }
        Ok(MediaEdit {
            media,
            location,
            hash: probed.hash,
        })
    }

    fn apply_resolution(
        &mut self,
        plugins: &[PluginResolution],
        media: &[MediaResolution],
        devices: &[DeviceResolution],
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<()> {
        let mut skipped: Vec<String> = Vec::new();
        let mut plugin_edits = Vec::new();
        let mut seen_devices = BTreeSet::new();
        // Missing devices: like `Replace` (values, lanes and mappings by parameter name).
        let mut not_kept: Vec<String> = Vec::new();
        for r in devices {
            for device in &r.devices {
                if !seen_devices.insert(*device) {
                    skipped.push(format!("device {device} is listed twice"));
                    continue;
                }
                let is_missing = self.doc.as_ref().is_some_and(|doc| {
                    matches!(
                        doc.project.devices.get(device).map(|d| &d.kind),
                        Some(DeviceKind::Missing { .. })
                    )
                });
                if !is_missing {
                    skipped.push(format!("device {device} is not a missing device"));
                    continue;
                }
                match self.prepare_replacement(*device, &r.replacement, true) {
                    Ok(p) => {
                        if !p.report.lost.is_empty() {
                            not_kept.push(format!(
                                "{} → {}: {}",
                                p.old_name,
                                p.new_name,
                                p.report.lost.join(", ")
                            ));
                        }
                        plugin_edits.push(p.edit);
                    }
                    Err(e) => skipped.push(e.message),
                }
            }
        }
        for r in plugins {
            if r.devices.iter().any(|d| !seen_devices.insert(*d)) {
                skipped.push(format!("{}: a device is listed twice", r.name));
                continue;
            }
            let mut edits = Vec::new();
            match self.plugin_edits(r, &mut edits) {
                Ok(()) => plugin_edits.extend(edits),
                Err(e) => skipped.push(e),
            }
        }
        let mut media_edits = Vec::new();
        for r in media {
            match self.media_edit(r.media, &r.source) {
                Ok(e) => media_edits.push(e),
                Err(e) => skipped.push(e),
            }
        }
        if plugin_edits.is_empty() && media_edits.is_empty() {
            if skipped.is_empty() {
                return Ok(());
            }
            return Err(invalid(skipped.join("; ")));
        }
        let relinked: Vec<MediaId> = media_edits.iter().map(|e| e.media).collect();
        self.edit_with(
            crate::doc::undo("undo.resolve.apply"),
            None,
            now,
            out,
            |ctx| {
                for e in plugin_edits {
                    replace_plugin_device(ctx, e)?;
                }
                for e in media_edits {
                    let m = ctx
                        .p()
                        .media
                        .get(&e.media)
                        .cloned()
                        .ok_or_else(|| invalid(format!("media {}", e.media)))?;
                    if m.location != e.location {
                        ctx.tx.update(EntityUpdate::Media {
                            id: e.media,
                            change: MediaChange::Location(e.location),
                        })?;
                    }
                    if m.hash.as_deref() != Some(e.hash.as_str()) {
                        ctx.tx.update(EntityUpdate::Media {
                            id: e.media,
                            change: MediaChange::Hash(Some(e.hash)),
                        })?;
                    }
                }
                Ok(())
            },
        )?;
        for id in &relinked {
            self.resolver.hints.missing.remove(id);
        }
        if !relinked.is_empty() {
            self.media.recheck_missing();
            if let Some(doc) = self.doc.as_ref() {
                self.media.sync(&mut self.bridge, Some(&doc.project));
            }
        }
        if !skipped.is_empty() {
            notify(
                out,
                NotificationLevel::Warning,
                crate::text::msg("notice.resolver.skipped").arg("items", skipped.join("; ")),
            );
        }
        if !not_kept.is_empty() {
            notify(
                out,
                NotificationLevel::Warning,
                crate::text::msg("notice.resolver.not_kept").arg("items", not_kept.join("; ")),
            );
        }
        Ok(())
    }
}

/// Switch a device's plugin. The model keeps a device's plugin id (`DeviceChange::Plugin`),
/// so the device is re-created with the same id: what refers to it (automation lanes and
/// their points, MIDI and modulation mappings, modulators) is taken off first and put back
/// after, following the parameter map (references to unmapped parameters are dropped;
/// references to the device itself, like its on/off switch, come back unchanged).
/// Mappings from the device's own modulators to other devices come back unchanged.
fn replace_plugin_device(ctx: &mut crate::doc::DocCtx<'_, '_>, e: PluginEdit) -> CmdResult<()> {
    use ether_core::protocol::model::{MidiMapTarget, ModSource};
    let id = e.device;
    let remap = |param: ParamId| -> Option<ParamId> {
        match &e.map {
            None => Some(param),
            Some(map) => map.iter().find(|(f, _)| *f == param).map(|(_, t)| *t),
        }
    };
    let p = ctx.p();
    let lanes: Vec<_> = p
        .automation_lanes
        .values()
        .filter(|l| l.target.device() == Some(id))
        .cloned()
        .collect();
    let points: Vec<_> = lanes
        .iter()
        .flat_map(|l| p.points_of(l.id).into_iter().cloned())
        .collect();
    let midi: Vec<_> = p
        .midi_mappings
        .values()
        .filter(
            |m| matches!(&m.target, MidiMapTarget::Param { target } if target.device() == Some(id)),
        )
        .cloned()
        .collect();
    let modulators: Vec<_> = p
        .modulators
        .values()
        .filter(|m| m.device == id)
        .cloned()
        .collect();
    // Mappings to this device's params, and those of its own modulators to other devices
    // (they go and come back with their modulator).
    let mods: Vec<_> = p
        .mod_mappings
        .values()
        .filter(|m| {
            m.device == id
                || matches!(m.source, ModSource::Modulator { modulator }
                    if modulators.iter().any(|x| x.id == modulator))
        })
        .cloned()
        .collect();
    let mut d = ctx.device(id)?;
    for x in &points {
        ctx.tx.remove(EntityKey::AutomationPoint(x.id))?;
    }
    for x in &lanes {
        ctx.tx.remove(EntityKey::AutomationLane(x.id))?;
    }
    for x in &midi {
        ctx.tx.remove(EntityKey::MidiMapping(x.id))?;
    }
    for x in &mods {
        ctx.tx.remove(EntityKey::ModMapping(x.id))?;
    }
    for x in &modulators {
        ctx.tx.remove(EntityKey::Modulator(x.id))?;
    }
    ctx.tx.remove(EntityKey::Device(id))?;
    d.kind = e.kind.clone();
    if e.clear_sidechain {
        d.sidechain = None;
        d.sidechain_tap = None;
    }
    if let Some(name) = &e.rename {
        d.name = name.clone();
    }
    if let Some(params) = &e.params {
        d.params = params.clone();
    } else if e.map.is_some() {
        d.params = d
            .params
            .iter()
            .filter_map(|(k, v)| remap(*k).map(|t| (t, *v)))
            .collect();
    }
    ctx.tx.insert(Entity::Device(d))?;
    for x in modulators {
        ctx.tx.insert(Entity::Modulator(x))?;
    }
    let mut kept = BTreeSet::new();
    for mut l in lanes {
        // Parameter lanes follow the map; the device's own (on/off) lanes stay as they are.
        if let AutomationTarget::DeviceParam { param, .. } = l.target {
            let Some(t) = remap(param) else { continue };
            l.target = AutomationTarget::DeviceParam {
                device: id,
                param: t,
            };
        }
        kept.insert(l.id);
        ctx.tx.insert(Entity::AutomationLane(l))?;
    }
    for x in points {
        if kept.contains(&x.lane) {
            ctx.tx.insert(Entity::AutomationPoint(x))?;
        }
    }
    for mut m in mods {
        if m.device == id {
            let Some(t) = remap(m.param) else { continue };
            m.param = t;
        }
        ctx.tx.insert(Entity::ModMapping(m))?;
    }
    for mut m in midi {
        if let MidiMapTarget::Param {
            target: AutomationTarget::DeviceParam { param, .. },
        } = m.target
        {
            let Some(t) = remap(param) else { continue };
            m.target = MidiMapTarget::Param {
                target: AutomationTarget::DeviceParam {
                    device: id,
                    param: t,
                },
            };
        }
        ctx.tx.insert(Entity::MidiMapping(m))?;
    }
    Ok(())
}

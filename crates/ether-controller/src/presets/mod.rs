//! Device presets (v0.2, owned by the `presets` node; file format `ether_model::preset`,
//! protocol `ether_protocol::presets`, CONTRACTS.md §12.5).
//!
//! - **Factory** presets are embedded per built-in type (`ether_devices::factory_presets`,
//!   ids `"<device-key>/<slug>"`, read-only).
//! - **User** presets are files in the writable user library (`Library::user_root`):
//!   `Presets/<device-key>/<name>.etherpreset`, plugins under
//!   `Presets/plugins/<format>/<plugin id>/`. The user id is the path relative to
//!   `Presets/`. Hosts without a user library reply `Unsupported` to the writes and list
//!   factory presets only.
//! - `List` = factory then user, each sorted by name (case-insensitive), filtered by device
//!   type and text (name, tags, author).
//! - `Load` is one undo step (`edit_with`, in the caller's gesture or a fresh one that also
//!   covers the sample imports): built-ins get every descriptor param set (the preset's
//!   value snapped to its range/step, or the default when missing; unknown ids ignored)
//!   and, when the preset carries kind data (sampler, multisampler), the kind with its media
//!   remapped to project media (`samples` are matched by hash or imported from their library
//!   location first; unavailable ones are dropped with a warning). Plugins get the state blob
//!   (authoritative) and are reloaded from the document; the param mirror follows from the
//!   new instance (`plugins` module). A preset for another device type is `InvalidArgument`.
//! - `Save` serializes the device: all built-in params (plain values), kind data + sample
//!   references for sampler types (project copies are written to `Samples/` in the user
//!   library so the preset stays usable in other projects; library/external files keep
//!   their library location), plugin state read from the live instance.
//! - Rack devices (v0.3, `rack-presets`, CONTRACTS.md §13.9): `Save` also stores the rack's
//!   chains with their devices, its modulators and the mappings inside it (`Preset::rack`,
//!   `rack::snapshot`); `Load` of such a preset needs the command's `seed` and rebuilds
//!   the structure in the same undo step (`rack::rebuild`, ids `derive_id(seed, i)`).
//!   Version-1 rack presets (no `rack`) set macros and params only.
//! - `Save`/`Rename`/`Delete`/`SetMeta` are runtime (no undo) and emit
//!   `PresetEvent::Changed`. Names are unique per device type (case-insensitive): `Save`
//!   replaces an existing one only with `overwrite`, `Rename` refuses a taken name.

mod files;
mod rack;

use std::collections::BTreeMap;

use ether_core::protocol::media::{BrowseLocation, MediaCommand, MediaSource};
use ether_core::protocol::model::{
    BuiltinDevice, Device, DeviceChange, DeviceId, DeviceKind, GestureId, MediaId, MediaLocation,
    PRESET_EXTENSION, PRESETS_DIR, ParamId, Preset, PresetDevice, PresetSample, RackChainId,
    save_preset,
};
use ether_core::protocol::presets::{
    PresetCommand, PresetEvent, PresetInfo, PresetRef, PresetSource,
};
use ether_core::protocol::{ClientMessage, Command, Event, NotificationLevel, ReplyValue};

use crate::handlers::{event, no_project, notify, store_err};
use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, internal, invalid, not_found, unsupported};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

use files::{
    device_dir, factory_preset, factory_types, file_stem, info, library_path, matches_text,
    normalize_meta, same_device, user_presets,
};

/// User-library folder where `Save` copies project samples of sample-based presets.
const SAMPLES_DIR: &str = "Samples";

/// Media ids referenced by built-in kind data.
fn kind_media(kind: &BuiltinDevice) -> Vec<MediaId> {
    match kind {
        BuiltinDevice::Sampler { sample, .. } => sample.iter().copied().collect(),
        BuiltinDevice::MultiSampler { zones } => zones.iter().filter_map(|z| z.media).collect(),
        _ => Vec::new(),
    }
}

/// Kind data carried by presets (sample-based types only).
fn preset_kind(kind: &BuiltinDevice) -> Option<BuiltinDevice> {
    matches!(
        kind,
        BuiltinDevice::Sampler { .. } | BuiltinDevice::MultiSampler { .. }
    )
    .then(|| kind.clone())
}

/// `kind` with its media ids mapped through `map` (unmapped ids become `None`).
fn remap_kind(kind: &BuiltinDevice, map: &BTreeMap<MediaId, MediaId>) -> BuiltinDevice {
    let mut kind = kind.clone();
    match &mut kind {
        BuiltinDevice::Sampler { sample, .. } => {
            *sample = sample.and_then(|m| map.get(&m).copied())
        }
        BuiltinDevice::MultiSampler { zones } => {
            for z in zones {
                z.media = z.media.and_then(|m| map.get(&m).copied());
            }
        }
        _ => {}
    }
    kind
}

fn preset_device(d: &Device) -> PresetDevice {
    match &d.kind {
        DeviceKind::Builtin { device } => PresetDevice::Builtin {
            device: device.device_type(),
        },
        DeviceKind::Plugin { plugin } => PresetDevice::Plugin {
            format: plugin.format,
            plugin_id: plugin.plugin_id.clone(),
            name: plugin.name.clone(),
            vendor: plugin.vendor.clone(),
        },
    }
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    pub(crate) fn preset_command(
        &mut self,
        command: &PresetCommand,
        gesture: Option<GestureId>,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        match command {
            PresetCommand::List { device, text } => Ok(ReplyValue::Presets {
                presets: self.preset_list(device.as_ref(), text.as_deref()),
            }),
            PresetCommand::Load {
                device,
                preset,
                seed,
            } => {
                self.preset_load(*device, preset, *seed, gesture, now, out)?;
                Ok(ReplyValue::Unit)
            }
            PresetCommand::Save {
                device,
                name,
                meta,
                overwrite,
            } => {
                let preset = self.preset_save(*device, name, meta, *overwrite)?;
                changed(out);
                Ok(ReplyValue::Preset { preset })
            }
            PresetCommand::Rename { preset, name } => {
                let preset = self.preset_rename(preset, name)?;
                changed(out);
                Ok(ReplyValue::Preset { preset })
            }
            PresetCommand::Delete { preset } => {
                let root = self.preset_user_root()?;
                let path = user_path(preset)?;
                self.library.remove_file(&root, &path).map_err(store_err)?;
                changed(out);
                Ok(ReplyValue::Unit)
            }
            PresetCommand::SetMeta { preset, meta } => {
                let (root, path, mut p) = self.preset_read_user(preset)?;
                p.meta = normalize_meta(meta);
                self.preset_write(&root, &path, &p)?;
                changed(out);
                Ok(ReplyValue::Preset {
                    preset: info(PresetSource::User, preset.id.clone(), &p),
                })
            }
        }
    }

    fn preset_user_root(&self) -> CmdResult<String> {
        self.library
            .user_root()
            .ok_or_else(|| unsupported("this host has no writable user library for presets"))
    }

    fn preset_list(
        &mut self,
        device: Option<&PresetDevice>,
        text: Option<&str>,
    ) -> Vec<PresetInfo> {
        let by_name = |a: &PresetInfo, b: &PresetInfo| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.preset.id.cmp(&b.preset.id))
        };
        let mut factory: Vec<PresetInfo> = factory_types(device)
            .into_iter()
            .flat_map(|t| ether_devices::factory_presets(t).iter())
            .filter_map(|f| {
                let p = ether_core::protocol::model::load_preset(f.json).ok()?;
                Some(info(PresetSource::Factory, f.id.to_string(), &p))
            })
            .filter(|i| matches_text(&i.name, &i.meta, text))
            .collect();
        factory.sort_by(by_name);
        let mut user: Vec<PresetInfo> = match self.library.user_root() {
            Some(root) => {
                let dir = device.map(device_dir).unwrap_or_default();
                user_presets(&mut self.library, &root, &dir)
                    .into_iter()
                    .filter(|(_, p)| device.is_none_or(|d| same_device(d, &p.device)))
                    .filter(|(_, p)| matches_text(&p.name, &p.meta, text))
                    .map(|(id, p)| info(PresetSource::User, id, &p))
                    .collect()
            }
            None => Vec::new(),
        };
        user.sort_by(by_name);
        factory.extend(user);
        factory
    }

    /// A user preset's library root, path and parsed content.
    fn preset_read_user(&mut self, preset: &PresetRef) -> CmdResult<(String, String, Preset)> {
        let root = self.preset_user_root()?;
        let path = user_path(preset)?;
        let bytes = self.library.read(&root, &path).map_err(store_err)?;
        let json = std::str::from_utf8(&bytes).map_err(|_| invalid("preset is not UTF-8"))?;
        let p = ether_core::protocol::model::load_preset(json)
            .map_err(|e| invalid(format!("invalid preset {}: {e}", preset.id)))?;
        Ok((root, path, p))
    }

    fn preset_resolve(&mut self, preset: &PresetRef) -> CmdResult<Preset> {
        match preset.source {
            PresetSource::Factory => factory_preset(&preset.id)
                .ok_or_else(|| not_found(format!("factory preset {}", preset.id))),
            PresetSource::User => self.preset_read_user(preset).map(|(_, _, p)| p),
        }
    }

    fn preset_write(&mut self, root: &str, path: &str, p: &Preset) -> CmdResult<()> {
        let json = save_preset(p, &self.config.app_version).map_err(|e| internal(e.to_string()))?;
        self.library
            .write_file(root, path, json.as_bytes())
            .map_err(store_err)
    }

    /// Another user preset of the same device type named `name` (case-insensitive).
    fn preset_named(
        &mut self,
        root: &str,
        device: &PresetDevice,
        name: &str,
        except: Option<&str>,
    ) -> Option<String> {
        let name = name.to_lowercase();
        user_presets(&mut self.library, root, &device_dir(device))
            .into_iter()
            .find(|(id, p)| {
                Some(id.as_str()) != except
                    && same_device(&p.device, device)
                    && p.name.to_lowercase() == name
            })
            .map(|(id, _)| id)
    }

    /// A free id `<dir>/<stem>[ n].etherpreset` (never an existing file).
    fn preset_free_id(
        &mut self,
        root: &str,
        dir: &str,
        stem: &str,
        except: Option<&str>,
    ) -> String {
        let taken: Vec<String> = self
            .library
            .list_dir(root, &format!("{PRESETS_DIR}/{dir}"))
            .map(|l| {
                l.entries
                    .into_iter()
                    .map(|e| e.name.to_lowercase())
                    .collect()
            })
            .unwrap_or_default();
        let except = except.map(str::to_lowercase);
        (1..)
            .map(|n| {
                let file = if n == 1 {
                    format!("{stem}.{PRESET_EXTENSION}")
                } else {
                    format!("{stem} {n}.{PRESET_EXTENSION}")
                };
                if dir.is_empty() {
                    file
                } else {
                    format!("{dir}/{file}")
                }
            })
            .find(|id| {
                let lower = id.to_lowercase();
                let file = lower.rsplit('/').next().unwrap_or(&lower);
                except.as_deref() == Some(lower.as_str()) || !taken.iter().any(|t| t == file)
            })
            .expect("unbounded")
    }

    fn preset_save(
        &mut self,
        device: DeviceId,
        name: &str,
        meta: &ether_core::protocol::model::PresetMeta,
        overwrite: bool,
    ) -> CmdResult<PresetInfo> {
        let root = self.preset_user_root()?;
        let name = name.trim();
        if name.is_empty() {
            return Err(invalid("a preset needs a name"));
        }
        let doc = self.doc.as_ref().ok_or_else(no_project)?;
        let d = doc
            .project
            .devices
            .get(&device)
            .cloned()
            .ok_or_else(|| not_found(format!("device {device}")))?;
        let pid = doc.project.id;
        let pdev = preset_device(&d);
        let existing = self.preset_named(&root, &pdev, name, None);
        if existing.is_some() && !overwrite {
            return Err(invalid(format!("a preset named \"{name}\" already exists")));
        }
        let mut preset = Preset {
            name: name.to_string(),
            device: pdev.clone(),
            meta: normalize_meta(meta),
            params: BTreeMap::new(),
            kind: None,
            samples: Vec::new(),
            state: None,
            rack: None,
        };
        match &d.kind {
            DeviceKind::Builtin { device: kind } => {
                let desc = ether_devices::descriptor(kind.device_type());
                preset.params = desc
                    .params
                    .iter()
                    .map(|p| (p.id, d.params.get(&p.id).copied().unwrap_or(p.default)))
                    .collect();
                let mut media = Vec::new();
                if let Some(k) = preset_kind(kind) {
                    media = kind_media(&k);
                    preset.kind = Some(k);
                }
                // v0.3: a rack stores its chains, their devices and its modulation.
                if kind.device_type().is_rack() {
                    let project = &self.doc.as_ref().ok_or_else(no_project)?.project;
                    let (bridge, engine) = (&mut self.bridge, &self.engine);
                    let mut state = |id: DeviceId| {
                        engine
                            .node(id)
                            .and_then(|_| bridge.plugin_state(id).ok().flatten())
                    };
                    let (structure, rack_media) = rack::snapshot(project, device, &mut state);
                    media.extend(rack_media);
                    preset.rack = Some(structure);
                }
                if !media.is_empty() {
                    preset.samples = self.preset_sample_refs(&root, pid, &media)?;
                }
            }
            DeviceKind::Plugin { plugin } => {
                let live = if self.engine.node(device).is_some() {
                    self.bridge.plugin_state(device).ok().flatten()
                } else {
                    None
                };
                preset.state = live.or_else(|| plugin.state.clone());
                preset.params = d.params.clone();
            }
        }
        let dir = device_dir(&pdev);
        let id = match existing {
            Some(id) => id,
            None => self.preset_free_id(&root, &dir, &file_stem(name), None),
        };
        self.preset_write(&root, &format!("{PRESETS_DIR}/{id}"), &preset)?;
        Ok(info(PresetSource::User, id, &preset))
    }

    /// Library references for the samples of a preset being saved: library files and
    /// external references inside a library root keep their location; anything else is
    /// copied to the user library's `Samples/` (content-addressed by hash).
    fn preset_sample_refs(
        &mut self,
        root: &str,
        pid: ether_core::protocol::model::ProjectId,
        media: &[MediaId],
    ) -> CmdResult<Vec<PresetSample>> {
        let doc = self.doc.as_ref().ok_or_else(no_project)?;
        let refs: Vec<_> = media
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
        let mut out = Vec::new();
        for m in refs {
            if out.iter().any(|s: &PresetSample| s.media == m.id) {
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

    fn preset_rename(&mut self, preset: &PresetRef, name: &str) -> CmdResult<PresetInfo> {
        let name = name.trim();
        if name.is_empty() {
            return Err(invalid("a preset needs a name"));
        }
        let (root, path, mut p) = self.preset_read_user(preset)?;
        if self
            .preset_named(&root, &p.device, name, Some(&preset.id))
            .is_some()
        {
            return Err(invalid(format!("a preset named \"{name}\" already exists")));
        }
        p.name = name.to_string();
        let dir = preset
            .id
            .rsplit_once('/')
            .map_or(String::new(), |(d, _)| d.to_string());
        let id = self.preset_free_id(&root, &dir, &file_stem(name), Some(&preset.id));
        self.preset_write(&root, &path, &p)?;
        if id != preset.id {
            self.library
                .rename_file(&root, &path, &format!("{PRESETS_DIR}/{id}"))
                .map_err(store_err)?;
        }
        Ok(info(PresetSource::User, id, &p))
    }

    fn preset_load(
        &mut self,
        device: DeviceId,
        preset: &PresetRef,
        seed: Option<RackChainId>,
        gesture: Option<GestureId>,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<()> {
        let p = self.preset_resolve(preset)?;
        let doc = self.doc.as_ref().ok_or_else(no_project)?;
        let d = doc
            .project
            .devices
            .get(&device)
            .cloned()
            .ok_or_else(|| not_found(format!("device {device}")))?;
        if !same_device(&preset_device(&d), &p.device) {
            return Err(invalid(format!(
                "preset \"{}\" is for another device type",
                p.name
            )));
        }
        // A preset with a rack structure needs the seed its new ids derive from.
        let seed = match (&p.rack, seed) {
            (Some(_), None) => {
                return Err(invalid(
                    "loading a rack preset with chains needs a seed for the new ids",
                ));
            }
            (Some(_), seed) => seed,
            (None, _) => None,
        };
        if seed.is_some() && (d.chain.is_some() || d.pad.is_some()) {
            return Err(invalid("racks cannot be nested in rack chains"));
        }
        // One undo step: the caller's gesture, or ours around imports + the edit.
        let own = gesture.is_none();
        let gesture = gesture.unwrap_or_else(|| self.new_gesture());
        let result = self.preset_apply(&d, &p, seed, gesture, now, out);
        if own && let Some(doc) = self.doc.as_mut() {
            doc.history.end_gesture(gesture);
        }
        result
    }

    fn preset_apply(
        &mut self,
        d: &Device,
        p: &Preset,
        seed: Option<RackChainId>,
        gesture: GestureId,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<()> {
        let device = d.id;
        match &d.kind {
            DeviceKind::Builtin { device: current } => {
                let map = if p.samples.is_empty() {
                    BTreeMap::new()
                } else {
                    self.preset_import_samples(p, gesture, now, out)
                };
                let kind = p.kind.as_ref().map(|k| remap_kind(k, &map));
                let structure = p.rack.as_ref().zip(seed);
                let mut plugins = Vec::new();
                let desc = ether_devices::descriptor(current.device_type());
                let params: Vec<(ParamId, f64)> = desc
                    .params
                    .iter()
                    .map(|info| {
                        let v = p
                            .params
                            .get(&info.id)
                            .map_or(info.default, |v| info.snap(*v));
                        (info.id, v)
                    })
                    .filter(|(id, v)| d.params.get(id) != Some(v))
                    .collect();
                self.edit_with("Load Preset", Some(gesture), now, out, |ctx| {
                    if let Some((structure, seed)) = structure {
                        plugins = rack::rebuild(ctx, device, structure, seed, &map)?;
                    }
                    if let Some(kind) = kind
                        && &kind != current
                    {
                        ctx.set_device(
                            device,
                            DeviceChange::Kind(DeviceKind::Builtin { device: kind }),
                        )?;
                    }
                    for (param, value) in params {
                        ctx.set_device(
                            device,
                            DeviceChange::Param {
                                param,
                                value: Some(value),
                            },
                        )?;
                    }
                    Ok(())
                })?;
                if !plugins.is_empty() {
                    // Chain plugins: re-create the instances from their preset state.
                    for id in plugins {
                        self.engine.request_reload_from_doc(id);
                    }
                    self.publish_if_due(now, true, out);
                }
                Ok(())
            }
            DeviceKind::Plugin { plugin } => match &p.state {
                Some(state) => {
                    let mut plugin = plugin.clone();
                    plugin.state = Some(state.clone());
                    self.edit_with("Load Preset", Some(gesture), now, out, |ctx| {
                        ctx.set_device(device, DeviceChange::Plugin(plugin))
                    })?;
                    // The state blob is authoritative: re-create the instance from it.
                    self.engine.request_reload_from_doc(device);
                    self.publish_if_due(now, true, out);
                    Ok(())
                }
                // A state-less plugin preset: apply its param values.
                None => self.edit_with("Load Preset", Some(gesture), now, out, |ctx| {
                    for (param, value) in &p.params {
                        if value.is_finite() {
                            ctx.set_device(
                                device,
                                DeviceChange::Param {
                                    param: *param,
                                    value: Some(*value),
                                },
                            )?;
                        }
                    }
                    Ok(())
                }),
            },
        }
    }

    /// Project media for a preset's samples (preset media id → project media id): reuse a
    /// project media with the same hash, else import from the library location. Samples
    /// that can't be found are left out (warning).
    fn preset_import_samples(
        &mut self,
        p: &Preset,
        gesture: GestureId,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> BTreeMap<MediaId, MediaId> {
        let mut map = BTreeMap::new();
        for s in &p.samples {
            let existing = self.doc.as_ref().and_then(|doc| {
                let hash = s.hash.as_deref()?;
                doc.project
                    .media
                    .values()
                    .find(|m| m.hash.as_deref() == Some(hash))
                    .map(|m| m.id)
            });
            if let Some(id) = existing {
                map.insert(s.media, id);
                continue;
            }
            let id: MediaId = self.ids.next(now);
            let msg = ClientMessage {
                id: 0,
                gesture: Some(gesture),
                command: Command::Media(MediaCommand::Import {
                    id,
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
                    map.insert(s.media, id);
                }
                Err(e) => notify(
                    out,
                    NotificationLevel::Warning,
                    format!(
                        "preset \"{}\": sample {} is unavailable ({})",
                        p.name, s.path, e.message
                    ),
                ),
            }
        }
        map
    }
}

fn user_path(preset: &PresetRef) -> CmdResult<String> {
    if preset.source != PresetSource::User {
        return Err(invalid("factory presets are read-only"));
    }
    library_path(&preset.id).map_err(store_err)
}

fn changed(out: &mut dyn MessageSink) {
    event(
        out,
        Event::Preset {
            event: PresetEvent::Changed,
        },
    );
}

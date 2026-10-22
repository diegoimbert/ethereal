//! Replacing a device's implementation while keeping what refers to it, shared by
//! `Resolve::Replace` (owner requests: "swap plugins even if no error", then "it should
//! rather just be replace device and not limited to plugins": any device that isn't a rack,
//! built-in, plugin or missing, gets another device of the same role, built-in or plugin, in
//! one undo step "Replace Device") and `Resolve::Apply`'s missing devices (owner request
//! "missing devices": a placeholder of a device Ethereal could not create gets a plugin or
//! a built-in device).
//!
//! The switch itself is the resolver's ([`super::replace_plugin_device`]: the device is
//! re-created with the same id, so its position, sidechain source and enabled state stay,
//! and what refers to it is taken off and put back through a parameter map). This module
//! decides the map ([`plan`], pure) and prepares the edit ([`EtherController::prepare_replacement`]):
//!
//! - without `retain`: nothing maps (the new device starts at its defaults; parameter lanes
//!   and mappings go, the on/off lane and modulators stay);
//! - the same plugin, or another version of it in the same format (the resolver's
//!   [`super::plugins::can_keep_state`] rule), or the same built-in type: the state (a
//!   built-in's content: its sample, zones, impulse response, routing) is carried,
//!   parameter ids kept;
//! - both parameter lists known: by normalized name ([`match_by_name`]), values converted
//!   through their normalized range;
//! - the same plugin family in another format, names unknown: by id;
//! - else nothing can be matched.
//!
//! Parameter lists: the old plugin's from its live instance (or the names an import saved),
//! a missing device's from its placeholder; the new plugin's from a device already using
//! it, else a short-lived private instance ([`crate::EngineBridge::probe_plugin`], cached); a
//! built-in's from its descriptor.
//!
//! Roles: the old and new devices' categories must match (audio effect, instrument, MIDI
//! effect) when both are known (a plugin's only from a descriptor), and an instrument needs
//! a MIDI track. Racks and drum racks are never replaced nor replacements: their chains and
//! pads would go with them. The sidechain source stays when the new device has a sidechain
//! input (or it is unknown: a plugin that couldn't be probed), else it goes and is listed.

use std::collections::{BTreeMap, BTreeSet};

use ether_core::protocol::devices::{
    DeviceCategory, DeviceDescriptor, ParamInfo, ParamScale, ParamUnit,
};
use ether_core::protocol::model::{
    AutomationTarget, Base64Bytes, BuiltinDevice, BuiltinDeviceType, DeviceId, DeviceKind,
    MidiMapTarget, MissingParam, MissingRole, ParamId, PluginFormat, PluginInstance, TrackKind,
};
use ether_core::protocol::resolver::{ParamMatch, ReplaceReport, Replacement};

use super::normalize::{self, VendorMatch};
use super::plugins::vst2_equivalent;
use super::{PluginEdit, replace_plugin_device};
use crate::handlers::no_project;
use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, invalid};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// Values closer than this to a parameter's default don't count as set.
const DEFAULT_EPSILON: f64 = 1e-9;
/// Lost parameters listed by name before "and N more parameters".
const LOST_SHOWN: usize = 8;
/// Report lines (English, like the parameter names around them).
const LOST_PLUGIN_STATE: &str = "plugin state (presets, loaded samples)";
const LOST_CONTENT: &str = "device content (samples, impulse response, routing)";
const LOST_SIDECHAIN: &str = "sidechain input";

/// One side of a replacement.
pub struct Side<'a> {
    /// `None`: not a plugin (a built-in device, or a missing device).
    pub format: Option<PluginFormat>,
    pub id: &'a str,
    pub name: &'a str,
    pub vendor: &'a str,
    /// Parameter names (`None` = unknown).
    pub names: Option<&'a [(ParamId, String)]>,
}

/// How parameters move: `map: None` keeps them as they are.
#[derive(Debug, PartialEq)]
pub struct Plan {
    pub matched: ParamMatch,
    pub map: Option<Vec<(ParamId, ParamId)>>,
}

fn same_family(old: &Side<'_>, new: &Side<'_>) -> bool {
    normalize::compact(&normalize::family(old.name))
        == normalize::compact(&normalize::family(new.name))
}

/// Parameters matched by normalized name (`from` → `to`): the first parameter of each name
/// on the `to` side, one source per target. The one place both plugin replacement and
/// missing devices match parameters by name.
pub fn match_by_name(
    from: &[(ParamId, String)],
    to: &[(ParamId, String)],
) -> Vec<(ParamId, ParamId)> {
    let mut by_name: BTreeMap<String, ParamId> = BTreeMap::new();
    for (id, name) in to.iter().rev() {
        by_name.insert(normalize::param(name), *id);
    }
    let mut taken = BTreeSet::new();
    from.iter()
        .filter_map(|(id, name)| {
            let to = *by_name.get(&normalize::param(name))?;
            taken.insert(to).then_some((*id, to))
        })
        .collect()
}

/// The parameter map for replacing `old` with `new` (`used`: the old device's parameters
/// that carry something: a value, a lane or a mapping).
pub fn plan(old: &Side<'_>, new: &Side<'_>, used: &BTreeSet<ParamId>, retain: bool) -> Plan {
    if !retain {
        return Plan {
            matched: ParamMatch::Reset,
            map: Some(Vec::new()),
        };
    }
    let vendor = normalize::vendor_match(old.vendor, new.vendor);
    if old.format.is_some()
        && old.format == new.format
        && (old.id.eq_ignore_ascii_case(new.id)
            || (vendor == VendorMatch::Same && same_family(old, new)))
    {
        return Plan {
            matched: ParamMatch::SameIds,
            map: None,
        };
    }
    if let (Some(from), Some(to)) = (old.names, new.names)
        && !from.is_empty()
        && !to.is_empty()
    {
        return Plan {
            matched: ParamMatch::ByName,
            map: Some(match_by_name(from, to)),
        };
    }
    let family = match (old.format, new.format) {
        (Some(a), Some(b)) => {
            vendor != VendorMatch::Different
                && (same_family(old, new)
                    || matches!(
                        (vst2_equivalent(a, old.id), vst2_equivalent(b, new.id)),
                        (Some(x), Some(y)) if x == y
                    ))
        }
        _ => false,
    };
    if family {
        let known: Option<BTreeSet<ParamId>> = new.names.map(|n| n.iter().map(|p| p.0).collect());
        let map = used
            .iter()
            .filter(|p| known.as_ref().is_none_or(|k| k.contains(p)))
            .map(|p| (*p, *p))
            .collect();
        return Plan {
            matched: ParamMatch::ByIndex,
            map: Some(map),
        };
    }
    Plan {
        matched: ParamMatch::Unmatched,
        map: Some(Vec::new()),
    }
}

/// A value moved from `from` to `to` (through the normalized range when both are known).
fn convert(value: f64, from: Option<&ParamInfo>, to: Option<&ParamInfo>) -> f64 {
    match (from, to) {
        (Some(a), Some(b)) => b.snap(b.to_plain(a.to_normalized(value).clamp(0.0, 1.0))),
        _ => value,
    }
}

fn names_of(d: &DeviceDescriptor) -> Vec<(ParamId, String)> {
    d.params.iter().map(|p| (p.id, p.name.clone())).collect()
}

/// A missing device's parameter as a linear [`ParamInfo`] (its values convert through it).
fn missing_info(p: &MissingParam) -> ParamInfo {
    ParamInfo {
        id: p.id,
        name: p.name.clone(),
        group: None,
        unit: ParamUnit::None,
        min: p.min,
        max: p.max,
        default: p.min,
        scale: ParamScale::Linear,
        labels: None,
        automatable: true,
        hidden: false,
        step: None,
        remote: None,
    }
}

/// A replacement checked against the document, ready to apply ([`super::replace_plugin_device`]).
pub struct Prepared {
    pub report: ReplaceReport,
    pub edit: PluginEdit,
    /// The old plugin's live state (written into the document before the edit, so the undo
    /// step puts the device back as it was).
    pub live_state: Option<Base64Bytes>,
    /// For notifications: "Replaced <old> with <new>".
    pub old_name: String,
    pub new_name: String,
}

/// What the old device brings.
struct Old {
    names: Option<Vec<(ParamId, String)>>,
    info: BTreeMap<ParamId, ParamInfo>,
    /// Parameters with a value (not its default, when the default is known).
    valued: BTreeSet<ParamId>,
    format: Option<PluginFormat>,
    id: String,
    name: String,
    vendor: String,
    sandboxed: bool,
    state: Option<Base64Bytes>,
    live_state: Option<Base64Bytes>,
    /// The name the device got from it (renamed after the replacement when unchanged).
    default_name: String,
    /// Its role, when known (a plugin's only from its descriptor).
    category: Option<DeviceCategory>,
    /// A built-in: its type, and whether it carries content (a sample, zones, an impulse
    /// response, routing) a fresh one of that type doesn't.
    builtin: Option<(BuiltinDeviceType, bool)>,
}

impl<B, H, S, L, X> EtherController<B, H, S, L, X>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// The new plugin's descriptor: a device using it, the cache, else a probe.
    fn target_descriptor(&mut self, target: &PluginInstance) -> Option<DeviceDescriptor> {
        let key = (target.format, target.plugin_id.to_uppercase());
        let using = self.doc.as_ref().and_then(|doc| {
            doc.project.devices.values().find_map(|d| match &d.kind {
                DeviceKind::Plugin { plugin }
                    if plugin.format == target.format
                        && plugin.plugin_id.eq_ignore_ascii_case(&target.plugin_id) =>
                {
                    self.engine.descriptor(d)
                }
                _ => None,
            })
        });
        if let Some(d) = using {
            return Some(d);
        }
        if let Some(d) = self.resolver.probed.get(&key) {
            return Some(d.clone());
        }
        let d = self.bridge.probe_plugin(target)?;
        self.resolver.probed.insert(key, d.clone());
        Some(d)
    }

    /// Check replacing `device` (any device but a rack) with `to` and work out the parameter
    /// map, values and report. Changes nothing.
    pub fn prepare_replacement(
        &mut self,
        device: DeviceId,
        to: &Replacement,
        retain: bool,
    ) -> CmdResult<Prepared> {
        if let Replacement::Plugin { plugin } = to
            && plugin.plugin_id.trim().is_empty()
        {
            return Err(invalid("no plugin id"));
        }
        let doc = self.doc.as_ref().ok_or_else(no_project)?;
        let p = &doc.project;
        let d = p
            .devices
            .get(&device)
            .cloned()
            .ok_or_else(|| invalid(format!("device {device} does not exist")))?;
        // Parameters that carry something besides a value: lanes and mappings.
        let lane_params: Vec<ParamId> = p
            .automation_lanes
            .values()
            .filter_map(|l| match l.target {
                AutomationTarget::DeviceParam { device: x, param } if x == device => Some(param),
                _ => None,
            })
            .collect();
        let mut referenced: BTreeSet<ParamId> = lane_params.iter().copied().collect();
        referenced.extend(
            p.mod_mappings
                .values()
                .filter(|m| m.device == device)
                .map(|m| m.param),
        );
        referenced.extend(p.midi_mappings.values().filter_map(|m| match &m.target {
            MidiMapTarget::Param {
                target: AutomationTarget::DeviceParam { device: x, param },
            } if *x == device => Some(*param),
            _ => None,
        }));
        let track_kind = p.tracks.get(&d.track).map(|t| t.kind);

        let old = match &d.kind {
            DeviceKind::Plugin { plugin } => {
                let desc = self.engine.descriptor(&d);
                let live = self.engine.node(device).is_some();
                let category = desc.as_ref().map(|d| d.category);
                let names: Option<Vec<(ParamId, String)>> =
                    desc.as_ref().map(names_of).or_else(|| {
                        self.hints()
                            .and_then(|h| h.param_names.get(&device))
                            .map(|v| v.iter().map(|(i, n)| (ParamId(*i), n.clone())).collect())
                    });
                let info: BTreeMap<ParamId, ParamInfo> = desc
                    .map(|d| d.params.into_iter().map(|p| (p.id, p)).collect())
                    .unwrap_or_default();
                // Set values: those away from their default (all of them when defaults are
                // unknown).
                let valued = d
                    .params
                    .iter()
                    .filter_map(|(id, v)| match info.get(id) {
                        Some(i) if (v - i.default).abs() <= DEFAULT_EPSILON => None,
                        _ => Some(*id),
                    })
                    .collect();
                Old {
                    names,
                    info,
                    valued,
                    format: Some(plugin.format),
                    id: plugin.plugin_id.clone(),
                    name: plugin.name.clone(),
                    vendor: plugin.vendor.clone(),
                    sandboxed: plugin.sandboxed,
                    state: plugin.state.clone(),
                    live_state: if live {
                        self.bridge.plugin_state(device).ok().flatten()
                    } else {
                        None
                    },
                    default_name: plugin.name.clone(),
                    category,
                    builtin: None,
                }
            }
            DeviceKind::Missing { missing } => Old {
                names: Some(
                    missing
                        .params
                        .iter()
                        .map(|p| (p.id, p.name.clone()))
                        .collect(),
                ),
                info: missing
                    .params
                    .iter()
                    .map(|p| (p.id, missing_info(p)))
                    .collect(),
                valued: d.params.keys().copied().collect(),
                format: None,
                id: missing.source_id.clone(),
                name: missing.description.clone(),
                vendor: String::new(),
                sandboxed: false,
                state: None,
                live_state: None,
                default_name: missing.description.clone(),
                category: Some(match missing.role {
                    MissingRole::Instrument => DeviceCategory::Instrument,
                    MissingRole::AudioEffect => DeviceCategory::AudioEffect,
                    MissingRole::NoteEffect => DeviceCategory::NoteEffect,
                }),
                builtin: None,
            },
            DeviceKind::Builtin { device: b } => {
                if d.kind.is_any_rack() {
                    return Err(invalid(format!(
                        "\"{}\" is a rack: racks can't be replaced",
                        d.name
                    )));
                }
                let ty = b.device_type();
                let desc = crate::doc::builtin_descriptor(b);
                let valued = d
                    .params
                    .iter()
                    .filter_map(|(id, v)| match desc.params.iter().find(|p| p.id == *id) {
                        Some(i) if (v - i.default).abs() <= DEFAULT_EPSILON => None,
                        _ => Some(*id),
                    })
                    .collect();
                Old {
                    names: Some(names_of(&desc)),
                    valued,
                    format: None,
                    id: format!("{ty:?}"),
                    name: desc.name.clone(),
                    vendor: "Ethereal".into(),
                    sandboxed: false,
                    state: None,
                    live_state: None,
                    default_name: crate::doc::default_device_name(b),
                    category: Some(desc.category),
                    builtin: Some((ty, *b != BuiltinDevice::new(ty))),
                    info: desc.params.into_iter().map(|p| (p.id, p)).collect(),
                }
            }
        };
        let missing_source = matches!(d.kind, DeviceKind::Missing { .. });
        let mut used = referenced.clone();
        used.extend(old.valued.iter().copied());

        let old_side = Side {
            format: old.format,
            id: &old.id,
            name: &old.name,
            vendor: &old.vendor,
            names: old.names.as_deref(),
        };
        // The new device, its side and its descriptor (names matter only when matching by
        // name).
        // The same built-in type keeps its content (a sample, zones...) and parameter ids.
        let same_builtin = retain
            && matches!((old.builtin, to), (Some((a, _)), Replacement::Builtin { device: b }) if a == *b);
        let (mut kind, new_name, new_desc, new_format, new_id, new_vendor) = match to {
            Replacement::Plugin { plugin: to } => {
                let target = PluginInstance {
                    format: to.format,
                    plugin_id: to.plugin_id.clone(),
                    name: to.name.clone(),
                    vendor: to.vendor.clone(),
                    version: to.version.clone(),
                    // Another plugin is a new instance: the "sandboxed by default" list
                    // decides; the same plugin keeps the old instance's flag.
                    sandboxed: if old.format == Some(to.format)
                        && old.id.eq_ignore_ascii_case(&to.plugin_id)
                    {
                        old.sandboxed
                    } else {
                        self.bridge.sandbox_by_default(to.format, &to.plugin_id)
                    },
                    always_process: false,
                    state: None,
                };
                let pre = plan(
                    &old_side,
                    &Side {
                        format: Some(to.format),
                        id: &to.plugin_id,
                        name: &to.name,
                        vendor: &to.vendor,
                        names: None,
                    },
                    &used,
                    retain,
                );
                // Its descriptor: to match names, and to check its role against a known one.
                let desc = match pre.matched {
                    ParamMatch::SameIds => None,
                    ParamMatch::Reset if old.category.is_none() => None,
                    _ if old.names.is_none() && old.category.is_none() => None,
                    _ => self.target_descriptor(&target),
                };
                if let Some(desc) = &desc {
                    check_role(old.category, desc.category, track_kind, &d.name)?;
                }
                (
                    DeviceKind::Plugin { plugin: target },
                    to.name.clone(),
                    desc,
                    Some(to.format),
                    to.plugin_id.clone(),
                    to.vendor.clone(),
                )
            }
            Replacement::Builtin { device: ty } => {
                if ty.is_rack() || *ty == BuiltinDeviceType::DrumRack {
                    return Err(invalid(
                        "a device can't be replaced by a rack or a drum rack",
                    ));
                }
                let desc = ether_devices::descriptor(*ty);
                check_role(old.category, desc.category, track_kind, &d.name)?;
                (
                    DeviceKind::Builtin {
                        device: match &d.kind {
                            DeviceKind::Builtin { device } if same_builtin => device.clone(),
                            _ => BuiltinDevice::new(*ty),
                        },
                    },
                    crate::doc::default_device_name(&BuiltinDevice::new(*ty)),
                    Some(desc),
                    None,
                    format!("{ty:?}"),
                    "Ethereal".to_string(),
                )
            }
        };
        let new_names = new_desc.as_ref().map(names_of);
        let plan = if same_builtin {
            Plan {
                matched: ParamMatch::SameIds,
                map: None,
            }
        } else {
            plan(
                &old_side,
                &Side {
                    format: new_format,
                    id: &new_id,
                    name: &new_name,
                    vendor: &new_vendor,
                    names: new_names.as_deref(),
                },
                &used,
                retain,
            )
        };

        // The state: carried for the same plugin (the live one when it runs).
        let current_state = old.live_state.clone().or_else(|| old.state.clone());
        let state_kept = plan.matched == ParamMatch::SameIds;
        if state_kept && let DeviceKind::Plugin { plugin } = &mut kind {
            plugin.state = current_state.clone();
        }

        let new_info: BTreeMap<ParamId, &ParamInfo> = new_desc
            .as_ref()
            .map(|d| d.params.iter().map(|p| (p.id, p)).collect())
            .unwrap_or_default();
        let params: Option<BTreeMap<ParamId, f64>> = plan.map.as_ref().map(|map| {
            map.iter()
                .filter_map(|(from, to)| {
                    let v = d.params.get(from)?;
                    Some((
                        *to,
                        convert(*v, old.info.get(from), new_info.get(to).copied()),
                    ))
                })
                .collect()
        });

        // The sidechain: kept when the new device has an input for it (unknown counts as
        // yes); a plugin is probed for it only when there is one to keep.
        let clear_sidechain = d.sidechain.is_some()
            && plan.matched != ParamMatch::SameIds
            && match (&new_desc, &kind) {
                (Some(desc), _) => desc.sidechain_inputs == 0,
                (None, DeviceKind::Plugin { plugin }) => {
                    let probe = PluginInstance {
                        state: None,
                        ..plugin.clone()
                    };
                    self.target_descriptor(&probe)
                        .is_some_and(|desc| desc.sidechain_inputs == 0)
                }
                (None, _) => false,
            };

        // The report.
        let mapped = |param: &ParamId| match &plan.map {
            None => true,
            Some(m) => m.iter().any(|(f, _)| f == param),
        };
        let label = |param: &ParamId| {
            old.names
                .as_ref()
                .and_then(|n| n.iter().find(|(i, _)| i == param))
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| format!("parameter {}", param.0))
        };
        let lanes_removed = lane_params.iter().filter(|p| !mapped(p)).count() as u32;
        let (kept, mut lost) = if plan.matched == ParamMatch::Reset {
            (0, Vec::new())
        } else {
            let kept = used.iter().filter(|p| mapped(p)).count() as u32;
            // A missing device lists every value its source stored: only lanes and
            // mappings are named, the other values counted.
            let named: Vec<String> = used
                .iter()
                .filter(|p| !mapped(p) && (!missing_source || referenced.contains(p)))
                .map(label)
                .collect();
            (kept, named)
        };
        if lost.len() > LOST_SHOWN {
            let more = lost.len() - (LOST_SHOWN - 2);
            lost.truncate(LOST_SHOWN - 2);
            lost.push(format!("and {more} more parameters"));
        }
        if missing_source && plan.matched != ParamMatch::Reset {
            let values = used
                .iter()
                .filter(|p| !mapped(p) && !referenced.contains(p))
                .count();
            if values > 0 {
                lost.push(format!("values of {values} other parameters"));
            }
        }
        let had_state = old.live_state.is_some() || old.state.is_some();
        if !state_kept && plan.matched != ParamMatch::Reset && had_state {
            lost.insert(0, LOST_PLUGIN_STATE.into());
        }
        if !state_kept && plan.matched != ParamMatch::Reset && old.builtin.is_some_and(|b| b.1) {
            lost.insert(0, LOST_CONTENT.into());
        }
        if clear_sidechain {
            lost.push(LOST_SIDECHAIN.into());
        }
        let report = ReplaceReport {
            matched: plan.matched,
            state_kept,
            kept,
            lost,
            lanes_removed,
        };
        let edit = PluginEdit {
            device,
            rename: (d.name == old.default_name && !new_name.is_empty() && new_name != d.name)
                .then(|| new_name.clone()),
            kind,
            map: plan.map,
            params,
            clear_sidechain,
        };
        Ok(Prepared {
            report,
            edit,
            live_state: old.live_state,
            old_name: old.name,
            new_name,
        })
    }

    /// [`Self::keep_live_state`] with the state of `device`'s running instance, if any.
    pub fn keep_running_state(&mut self, device: DeviceId) {
        if self.engine.node(device).is_some() {
            let live = self.bridge.plugin_state(device).ok().flatten();
            self.keep_live_state(device, live);
        }
    }

    /// The old plugin's live state goes into the document first (like a save, not an edit),
    /// so the device the undo step puts back loads as it was.
    pub fn keep_live_state(&mut self, device: DeviceId, state: Option<Base64Bytes>) {
        if let Some(state) = state
            && let Some(DeviceKind::Plugin { plugin }) = self
                .doc
                .as_mut()
                .and_then(|doc| doc.project.devices.get_mut(&device))
                .map(|d| &mut d.kind)
        {
            plugin.state = Some(state);
        }
    }

    pub fn replace_device(
        &mut self,
        device: DeviceId,
        to: &Replacement,
        retain: bool,
        preview: bool,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplaceReport> {
        let prepared = self.prepare_replacement(device, to, retain)?;
        if preview {
            return Ok(prepared.report);
        }
        let Prepared {
            report,
            edit,
            live_state,
            old_name,
            new_name,
        } = prepared;
        self.keep_live_state(device, live_state);
        self.edit_with(
            crate::doc::undo("undo.resolve.replace"),
            None,
            now,
            out,
            |ctx| replace_plugin_device(ctx, edit),
        )?;
        // A new node from the document's state (never the old plugin's live state).
        self.engine.request_reload_from_doc(device);
        if !report.lost.is_empty() {
            crate::handlers::notify(
                out,
                ether_core::protocol::NotificationLevel::Warning,
                crate::text::msg("notice.resolver.replaced_lost")
                    .arg("old", old_name.as_str())
                    .arg("new", new_name.as_str())
                    .arg("lost", report.lost.join(", ")),
            );
        }
        Ok(report)
    }
}

/// A replacement has the old device's role (when both are known), and an instrument needs a
/// MIDI track.
fn check_role(
    old: Option<DeviceCategory>,
    category: DeviceCategory,
    track: Option<TrackKind>,
    name: &str,
) -> CmdResult<()> {
    if let Some(old) = old
        && old != category
    {
        return Err(invalid(format!(
            "\"{name}\" is {}: {} can't replace it",
            role_text(old),
            role_text(category)
        )));
    }
    if category == DeviceCategory::Instrument && track.is_some_and(|k| k != TrackKind::Midi) {
        return Err(invalid(format!(
            "\"{name}\" is on an audio track: an instrument can't replace it"
        )));
    }
    Ok(())
}

fn role_text(c: DeviceCategory) -> &'static str {
    match c {
        DeviceCategory::Instrument => "an instrument",
        DeviceCategory::AudioEffect => "an audio effect",
        DeviceCategory::NoteEffect => "a MIDI effect",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn side<'a>(
        format: PluginFormat,
        id: &'a str,
        name: &'a str,
        names: Option<&'a [(ParamId, String)]>,
    ) -> Side<'a> {
        Side {
            format: Some(format),
            id,
            name,
            vendor: "Acme",
            names,
        }
    }

    fn used(ids: &[u32]) -> BTreeSet<ParamId> {
        ids.iter().map(|i| ParamId(*i)).collect()
    }

    #[test]
    fn without_retain_nothing_maps() {
        let a = side(PluginFormat::Clap, "a", "Alpha", None);
        let p = plan(&a, &a, &used(&[1]), false);
        assert_eq!(p.matched, ParamMatch::Reset);
        assert_eq!(p.map, Some(vec![]));
    }

    #[test]
    fn same_plugin_or_version_keeps_ids() {
        let a = side(PluginFormat::Vst3, "AAA", "Kontakt 7", None);
        let b = side(PluginFormat::Vst3, "aaa", "Kontakt 7", None);
        assert_eq!(plan(&a, &b, &used(&[1]), true).map, None);
        let v8 = side(PluginFormat::Vst3, "BBB", "Kontakt 8", None);
        assert_eq!(
            plan(&a, &v8, &used(&[1]), true).matched,
            ParamMatch::SameIds
        );
    }

    #[test]
    fn other_plugins_map_by_normalized_name() {
        let from = vec![
            (ParamId(1), "Cutoff".to_string()),
            (ParamId(2), "Res-onance".to_string()),
            (ParamId(3), "Drive".to_string()),
        ];
        let to = vec![
            (ParamId(10), "RESONANCE".to_string()),
            (ParamId(11), "cutoff".to_string()),
            (ParamId(12), "Cutoff".to_string()),
        ];
        let a = side(PluginFormat::Clap, "a", "Alpha", Some(&from));
        let b = side(PluginFormat::Clap, "b", "Beta", Some(&to));
        let p = plan(&a, &b, &used(&[1, 2, 3]), true);
        assert_eq!(p.matched, ParamMatch::ByName);
        assert_eq!(
            p.map,
            Some(vec![(ParamId(1), ParamId(11)), (ParamId(2), ParamId(10))])
        );
    }

    #[test]
    fn same_family_without_names_maps_by_id() {
        let a = side(PluginFormat::Vst3, "AAA", "Diva", None);
        let b = side(PluginFormat::Clap, "com.u-he.diva", "Diva", None);
        let p = plan(&a, &b, &used(&[4, 5]), true);
        assert_eq!(p.matched, ParamMatch::ByIndex);
        assert_eq!(
            p.map,
            Some(vec![(ParamId(4), ParamId(4)), (ParamId(5), ParamId(5))])
        );
        // Unrelated, names unknown: nothing.
        let c = side(PluginFormat::Clap, "x", "Zebra", None);
        assert_eq!(
            plan(&a, &c, &used(&[4]), true).matched,
            ParamMatch::Unmatched
        );
    }

    #[test]
    fn missing_and_builtin_sides_never_keep_ids_or_match_by_family() {
        // A missing "Reverb" and the built-in Reverb: names only, never by id.
        let from = vec![
            (ParamId(0), "DryWet".to_string()),
            (ParamId(1), "Decay".to_string()),
        ];
        let to = vec![(ParamId(7), "Dry/Wet".to_string())];
        let missing = Side {
            format: None,
            id: "ableton:Reverb",
            name: "Reverb",
            vendor: "",
            names: Some(&from),
        };
        let builtin = Side {
            format: None,
            id: "Reverb",
            name: "Reverb",
            vendor: "Ethereal",
            names: Some(&to),
        };
        let p = plan(&missing, &builtin, &used(&[0, 1]), true);
        assert_eq!(p.matched, ParamMatch::ByName);
        assert_eq!(p.map, Some(vec![(ParamId(0), ParamId(7))]));
        let unnamed = Side {
            names: None,
            ..builtin
        };
        assert_eq!(
            plan(&missing, &unnamed, &used(&[0]), true).matched,
            ParamMatch::Unmatched
        );
    }

    #[test]
    fn missing_values_convert_through_their_range() {
        let from = missing_info(&MissingParam {
            id: ParamId(0),
            name: "Mix".into(),
            min: 0.0,
            max: 1.0,
        });
        let mut to = from.clone();
        to.max = 100.0;
        assert!((convert(0.25, Some(&from), Some(&to)) - 25.0).abs() < 1e-9);
    }
}

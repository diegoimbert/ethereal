//! `DeviceCommand`s (document part; `ListBuiltin`/`GetDescriptor` are queries handled by
//! the controller).

use std::collections::BTreeMap;

use ether_core::protocol::devices::{DeviceCategory, DeviceCommand, DeviceDescriptor, DeviceSpec};
use ether_core::protocol::model::*;

use super::{DocCtx, order_after, order_before};
use crate::tx::{CmdResult, invalid, not_found, unsupported};

pub fn builtin_descriptor(device: &BuiltinDevice) -> DeviceDescriptor {
    ether_devices::descriptor(device.device_type())
}

/// A fresh built-in device's name: its descriptor's, lowercase (owner request: default names
/// are lowercase). Every insert path uses it (track chain, rack chain, drum pad, grouping,
/// replacements); a plugin keeps its vendor's name.
pub fn default_device_name(device: &BuiltinDevice) -> String {
    crate::text::device_default_name(&builtin_descriptor(device).name)
}

/// A missing device's descriptor: the parameters its source gave, linear over their ranges
/// (no node: values and automation are kept for its replacement).
pub fn missing_descriptor(m: &MissingDevice) -> DeviceDescriptor {
    use ether_core::protocol::devices::{DeviceTypeRef, ParamInfo, ParamScale, ParamUnit};
    let (category, inputs, outputs, midi) = match m.role {
        MissingRole::Instrument => (DeviceCategory::Instrument, 0, 2, true),
        MissingRole::AudioEffect => (DeviceCategory::AudioEffect, 2, 2, false),
        MissingRole::NoteEffect => (DeviceCategory::NoteEffect, 0, 0, true),
    };
    DeviceDescriptor {
        device_type: DeviceTypeRef::Missing {
            source_id: m.source_id.clone(),
        },
        name: m.description.clone(),
        category,
        params: m
            .params
            .iter()
            .map(|p| ParamInfo {
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
            })
            .collect(),
        audio_inputs: inputs,
        audio_outputs: outputs,
        midi_input: midi,
        sidechain_inputs: 0,
        layout: None,
        implemented: true,
    }
}

fn chain(p: &Project, track: TrackId, except: Option<DeviceId>) -> Vec<(OrderKey, DeviceId)> {
    p.devices_of(track)
        .into_iter()
        .filter(|d| Some(d.id) != except)
        .map(|d| (d.order.clone(), d.id))
        .collect()
}

fn check_fits(track: &Track, category: Option<DeviceCategory>) -> CmdResult<()> {
    if category == Some(DeviceCategory::Instrument) && track.kind != TrackKind::Midi {
        return Err(invalid(format!(
            "instruments can only be inserted on MIDI tracks (track {} is {:?})",
            track.id, track.kind
        )));
    }
    if track.kind == TrackKind::Midi || category != Some(DeviceCategory::NoteEffect) {
        return Ok(());
    }
    Err(invalid("note effects can only be inserted on MIDI tracks"))
}

fn category_of(ctx: &mut DocCtx, d: &Device) -> Option<DeviceCategory> {
    ctx.host.descriptor(d.id, &d.kind).map(|desc| desc.category)
}

/// Clamp a plain value to a param's range (and snap enum params to a step).
fn clamp_param(desc: &DeviceDescriptor, param: ParamId, value: f64) -> CmdResult<f64> {
    if !value.is_finite() {
        return Err(invalid("param values must be finite"));
    }
    let Some(info) = desc.params.iter().find(|p| p.id == param) else {
        return Err(not_found(format!("param {}", param.0)));
    };
    let (lo, hi) = (info.min.min(info.max), info.min.max(info.max));
    let v = value.clamp(lo, hi);
    Ok(match &info.labels {
        Some(labels) if labels.len() > 1 => {
            let step = (info.max - info.min) / (labels.len() - 1) as f64;
            info.min + ((v - info.min) / step).round() * step
        }
        _ => v,
    })
}

/// Edits a rack's macro bank settings ([`RackMacros`], any rack type) as one `Kind` change;
/// no change when `f` leaves them as they are.
fn set_rack_macros(
    ctx: &mut DocCtx,
    device: DeviceId,
    f: impl FnOnce(&mut RackMacros),
) -> CmdResult<()> {
    let d = ctx.device(device)?;
    let DeviceKind::Builtin { device: b } = &d.kind else {
        return Err(invalid(format!("device {} is not a rack", d.id)));
    };
    let Some(mut macros) = b.rack_macros() else {
        return Err(invalid(format!("device {} is not a rack", d.id)));
    };
    f(&mut macros);
    let next = b.clone().with_rack_macros(macros);
    if &next == b {
        return Ok(());
    }
    ctx.set_device(
        d.id,
        DeviceChange::Kind(DeviceKind::Builtin { device: next }),
    )
}

pub(super) fn apply(ctx: &mut DocCtx, c: &DeviceCommand) -> CmdResult<()> {
    apply_inner(ctx, c)?;
    // v0.2 (`midi-fx`): MIDI effects precede the instrument.
    if let DeviceCommand::Insert { track, .. } | DeviceCommand::Move { track, .. } = c {
        crate::midi_fx::check_chain_order(ctx.p(), *track)?;
    }
    Ok(())
}

fn apply_inner(ctx: &mut DocCtx, c: &DeviceCommand) -> CmdResult<()> {
    match c {
        DeviceCommand::Insert {
            id,
            track,
            device,
            before,
        } => {
            if ctx.p().devices.contains_key(id) {
                return Ok(());
            }
            let t = ctx.track(*track)?;
            let order = order_before(&chain(ctx.p(), t.id, None), *before)?;
            let (kind, name, params) = match device {
                DeviceSpec::Builtin { device } => {
                    let desc = builtin_descriptor(device);
                    check_fits(&t, Some(desc.category))?;
                    let params: BTreeMap<ParamId, f64> =
                        desc.params.iter().map(|p| (p.id, p.default)).collect();
                    (
                        DeviceKind::Builtin {
                            device: device.clone(),
                        },
                        default_device_name(device),
                        params,
                    )
                }
                DeviceSpec::Plugin {
                    plugin_id,
                    sandboxed,
                    format,
                    name: listed,
                } => {
                    let plugin = PluginInstance {
                        format: format.unwrap_or(PluginFormat::Clap),
                        plugin_id: plugin_id.clone(),
                        name: listed.clone().unwrap_or_else(|| plugin_id.clone()),
                        vendor: String::new(),
                        version: String::new(),
                        sandboxed: ctx.host.new_instance_sandboxed(
                            *sandboxed,
                            format.unwrap_or(PluginFormat::Clap),
                            plugin_id,
                        ),
                        always_process: false,
                        state: None,
                    };
                    let desc = ctx.host.instantiate_plugin(*id, &plugin)?;
                    check_fits(&t, desc.as_ref().map(|d| d.category))?;
                    let name = desc
                        .as_ref()
                        .map(|d| d.name.clone())
                        .filter(|n| !n.is_empty())
                        .unwrap_or_else(|| plugin.name.clone());
                    let plugin = PluginInstance {
                        name: name.clone(),
                        ..plugin
                    };
                    (DeviceKind::Plugin { plugin }, name, BTreeMap::new())
                }
            };
            ctx.tx.insert(Entity::Device(Device {
                sidechain_tap: None,
                midi_sidechain: None,
                chain: None,
                id: *id,
                track: t.id,
                order,
                name,
                enabled: true,
                kind,
                params,
                sidechain: None,
                pad: None,
            }))?;
            if let DeviceSpec::Builtin { device } = device {
                crate::racks::add_first_chain(ctx, *id, device)?;
            }
            Ok(())
        }
        DeviceCommand::Remove { id } => {
            ctx.device(*id)?;
            ctx.delete_device(*id)
        }
        DeviceCommand::Move { id, track, before } => {
            let d = ctx.device(*id)?;
            if d.pad.is_some() {
                return Err(invalid(format!(
                    "device {} is on a drum pad: use DrumRack::MoveDevice",
                    d.id
                )));
            }
            if d.track != *track && !ctx.p().pads_of(d.id).is_empty() {
                // Moving a rack across tracks would strand its pad chains (each op is
                // checked on its own, so they can't move together). Unsupported by design.
                return Err(invalid(
                    "a drum rack with pads cannot move to another track (its pad chains live on its track); duplicate it there instead",
                ));
            }
            let t = ctx.track(*track)?;
            let category = category_of(ctx, &d);
            check_fits(&t, category)?;
            if *before == Some(d.id) {
                return Ok(());
            }
            let order = order_before(&chain(ctx.p(), t.id, Some(d.id)), *before)?;
            if d.track != t.id {
                ctx.set_device(d.id, DeviceChange::Track(t.id))?;
                // Clip envelopes of the device on its old track can't follow it.
                let dead: Vec<AutomationLaneId> = ctx
                    .p()
                    .automation_lanes
                    .values()
                    .filter(|l| {
                        l.target.device() == Some(d.id)
                            && matches!(l.owner, AutomationOwner::Clip { clip }
                                if ctx.p().clips.get(&clip).is_some_and(|c| c.track != t.id))
                    })
                    .map(|l| l.id)
                    .collect();
                if !dead.is_empty() {
                    for l in dead {
                        ctx.delete_lane(l)?;
                    }
                    ctx.warnings.push(
                        crate::text::msg("notice.device.envelopes_removed")
                            .arg("device", d.name.as_str())
                            .into(),
                    );
                }
            }
            ctx.set_device(d.id, DeviceChange::Order(order))
        }
        DeviceCommand::Duplicate { id, new_id, track } => {
            if ctx.p().devices.contains_key(new_id) {
                return Ok(());
            }
            let d = ctx.device(*id)?;
            if let Some(to) = track.filter(|t| *t != d.track) {
                return duplicate_to_track(ctx, &d, *new_id, to);
            }
            let siblings: Vec<(OrderKey, DeviceId)> = match d.pad {
                // Pad devices are duplicated within their pad chain.
                Some(pad) => ctx
                    .p()
                    .pad_devices_of(pad)
                    .into_iter()
                    .map(|d| (d.order.clone(), d.id))
                    .collect(),
                // v0.2: rack-chain devices within their chain.
                None if d.chain.is_some() => ctx
                    .p()
                    .chain_devices_of(d.chain.expect("checked"))
                    .into_iter()
                    .map(|d| (d.order.clone(), d.id))
                    .collect(),
                None => chain(ctx.p(), d.track, None),
            };
            let order = order_after(&siblings, d.id)?;
            let mut copy = d.clone();
            copy.id = *new_id;
            copy.order = order;
            if let DeviceKind::Plugin { plugin } = &mut copy.kind
                && let Some(state) = ctx.host.plugin_state(d.id)
            {
                plugin.state = Some(state);
            }
            ctx.tx.insert(Entity::Device(copy))?;
            // A drum rack is copied with its pads and their chains.
            let mut ids = BTreeMap::from([(d.id, *new_id)]);
            ctx.copy_rack_pads(d.id, *new_id, d.track, &mut ids)?;
            // v0.2: every copied rack (this one, or racks on the copied
            // pads) with its chains; modulators and their mappings inside the copy.
            crate::racks::copy_for_track(ctx, &mut ids)
        }
        DeviceCommand::Rename { id, name } => {
            ctx.device(*id)?;
            ctx.set_device(*id, DeviceChange::Name(name.clone()))
        }
        DeviceCommand::SetEnabled { id, enabled } => {
            ctx.device(*id)?;
            ctx.set_device(*id, DeviceChange::Enabled(*enabled))
        }
        DeviceCommand::SetParam {
            device,
            param,
            value,
        } => {
            let d = ctx.device(*device)?;
            let value = match ctx.host.descriptor(d.id, &d.kind) {
                Some(desc) => clamp_param(&desc, *param, *value)?,
                // Plugin not instantiated (missing): keep the mirror value as given.
                None if value.is_finite() => *value,
                None => return Err(invalid("param values must be finite")),
            };
            ctx.set_device(
                d.id,
                DeviceChange::Param {
                    param: *param,
                    value: Some(value),
                },
            )
        }
        DeviceCommand::ResetParam { device, param } => {
            let d = ctx.device(*device)?;
            // Reset = store the default explicitly (built-ins) or drop the mirror entry.
            let default = ctx.host.descriptor(d.id, &d.kind).and_then(|desc| {
                desc.params
                    .iter()
                    .find(|p| p.id == *param)
                    .map(|p| p.default)
            });
            if matches!(d.kind, DeviceKind::Builtin { .. }) && default.is_none() {
                return Err(not_found(format!("param {}", param.0)));
            }
            ctx.set_device(
                d.id,
                DeviceChange::Param {
                    param: *param,
                    value: default,
                },
            )
        }
        // v0.2 (`multisampler`).
        DeviceCommand::SetZones { device, zones } => {
            crate::multisampler::set_zones(ctx, *device, zones)
        }
        // `fx-modulation`: the Volume LFO's drawn cycle (one `Kind` change, one undo step).
        DeviceCommand::SetLfoCurve { device, curve } => {
            let d = ctx.device(*device)?;
            if !matches!(
                d.kind,
                DeviceKind::Builtin {
                    device: BuiltinDevice::VolumeLfo { .. }
                }
            ) {
                return Err(invalid(format!("device {} is not a volume lfo", d.id)));
            }
            ctx.set_device(
                d.id,
                DeviceChange::Kind(DeviceKind::Builtin {
                    device: BuiltinDevice::VolumeLfo {
                        curve: curve.sanitized(),
                    },
                }),
            )
        }
        // `shaper`: one lane's drawn cycle (one `Kind` change, one undo step).
        DeviceCommand::SetShaperCurve {
            device,
            lane,
            curve,
        } => {
            let d = ctx.device(*device)?;
            let DeviceKind::Builtin {
                device: BuiltinDevice::Shaper { curves },
            } = &d.kind
            else {
                return Err(invalid(format!("device {} is not a shaper", d.id)));
            };
            let curves = curves.with(*lane, curve);
            ctx.set_device(
                d.id,
                DeviceChange::Kind(DeviceKind::Builtin {
                    device: BuiltinDevice::Shaper { curves },
                }),
            )
        }
        // Racks: macro names and the visible macro count (one `Kind` change, one undo step).
        DeviceCommand::RenameMacro {
            device,
            index,
            name,
        } => {
            if *index >= ether_model::RACK_MACROS {
                return Err(invalid(format!(
                    "macro index must be < {}",
                    ether_model::RACK_MACROS
                )));
            }
            set_rack_macros(ctx, *device, |m| m.set_name(*index, name))
        }
        DeviceCommand::SetMacroCount { device, count } => {
            set_rack_macros(ctx, *device, |m| m.visible = *count)
        }
        DeviceCommand::SetSample { device, media } => {
            let d = ctx.device(*device)?;
            if !matches!(
                d.kind,
                DeviceKind::Builtin {
                    device: BuiltinDevice::Sampler { .. }
                }
            ) {
                return Err(invalid(format!("device {} is not a sampler", d.id)));
            }
            if let Some(m) = media
                && !ctx.p().media.contains_key(m)
            {
                return Err(not_found(format!("media {m}")));
            }
            let DeviceKind::Builtin {
                device: BuiltinDevice::Sampler { slices, .. },
            } = d.kind
            else {
                unreachable!("checked above");
            };
            ctx.set_device(
                d.id,
                DeviceChange::Kind(DeviceKind::Builtin {
                    device: BuiltinDevice::Sampler {
                        sample: *media,
                        slices,
                    },
                }),
            )
        }
        DeviceCommand::SetSidechain {
            device,
            source,
            tap,
        } => crate::sidechain::set_sidechain(ctx, *device, *source, *tap),
        DeviceCommand::SetMidiSidechain { device, source } => {
            crate::midi_sidechain::set_midi_sidechain(ctx, *device, *source)
        }
        // v0.3 (`fx-space`).
        DeviceCommand::SetIr { device, ir } => crate::fx_space::set_ir(ctx, *device, ir.as_ref()),
        DeviceCommand::ListBuiltin
        | DeviceCommand::GetDescriptor { .. }
        | DeviceCommand::ListFactoryIrs => Err(unsupported("not a document command")),
    }
}

/// `Device::Duplicate { track }` onto another track: the copy goes at the end of that
/// track's chain (the caller then moves it into place), its rack chains or drum pads with it.
fn duplicate_to_track(
    ctx: &mut DocCtx,
    d: &Device,
    new_id: DeviceId,
    to: TrackId,
) -> CmdResult<()> {
    let t = ctx.track(to)?;
    check_fits(&t, category_of(ctx, d))?;
    let order = order_before(&chain(ctx.p(), t.id, None), None)?;
    let mut copy = d.clone();
    copy.id = new_id;
    copy.track = t.id;
    copy.pad = None;
    copy.chain = None;
    copy.order = order;
    if let DeviceKind::Plugin { plugin } = &mut copy.kind
        && let Some(state) = ctx.host.plugin_state(d.id)
    {
        plugin.state = Some(state);
    }
    ctx.tx.insert(Entity::Device(copy))?;
    let mut ids = BTreeMap::from([(d.id, new_id)]);
    ctx.copy_rack_pads(d.id, new_id, t.id, &mut ids)?;
    crate::racks::copy_for_track(ctx, &mut ids)
}

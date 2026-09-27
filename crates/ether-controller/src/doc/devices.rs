//! `DeviceCommand`s (document part; `ListBuiltin`/`GetDescriptor` are queries handled by
//! the controller).

use std::collections::BTreeMap;

use ether_core::protocol::devices::{DeviceCategory, DeviceCommand, DeviceDescriptor, DeviceSpec};
use ether_core::protocol::model::*;

use super::{DocCtx, order_after, order_before};
use crate::tx::{CmdResult, invalid, not_found, unsupported};

pub(crate) fn builtin_descriptor(device: &BuiltinDevice) -> DeviceDescriptor {
    ether_devices::descriptor(device.device_type())
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

pub(super) fn apply(ctx: &mut DocCtx, c: &DeviceCommand) -> CmdResult<()> {
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
                        desc.name,
                        params,
                    )
                }
                DeviceSpec::Plugin {
                    plugin_id,
                    sandboxed,
                    format,
                } => {
                    let plugin = PluginInstance {
                        format: format.unwrap_or(PluginFormat::Clap),
                        plugin_id: plugin_id.clone(),
                        name: plugin_id.clone(),
                        vendor: String::new(),
                        version: String::new(),
                        sandboxed: sandboxed.unwrap_or(false),
                        state: None,
                    };
                    let desc = ctx.host.instantiate_plugin(*id, &plugin)?;
                    check_fits(&t, desc.as_ref().map(|d| d.category))?;
                    let name = desc
                        .as_ref()
                        .map(|d| d.name.clone())
                        .filter(|n| !n.is_empty())
                        .unwrap_or_else(|| plugin_id.clone());
                    let plugin = PluginInstance {
                        name: name.clone(),
                        ..plugin
                    };
                    (DeviceKind::Plugin { plugin }, name, BTreeMap::new())
                }
            };
            ctx.tx.insert(Entity::Device(Device {
                id: *id,
                track: t.id,
                order,
                name,
                enabled: true,
                kind,
                params,
                sidechain: None,
                pad: None,
            }))
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
                        matches!(l.target, AutomationTarget::DeviceParam { device, .. } if device == d.id)
                            && matches!(l.owner, AutomationOwner::Clip { clip }
                                if ctx.p().clips.get(&clip).is_some_and(|c| c.track != t.id))
                    })
                    .map(|l| l.id)
                    .collect();
                if !dead.is_empty() {
                    for l in dead {
                        ctx.delete_lane(l)?;
                    }
                    ctx.warnings.push(format!(
                        "\"{}\": clip envelopes on its previous track were removed",
                        d.name
                    ));
                }
            }
            ctx.set_device(d.id, DeviceChange::Order(order))
        }
        DeviceCommand::Duplicate { id, new_id } => {
            if ctx.p().devices.contains_key(new_id) {
                return Ok(());
            }
            let d = ctx.device(*id)?;
            let siblings: Vec<(OrderKey, DeviceId)> = match d.pad {
                // Pad devices are duplicated within their pad chain.
                Some(pad) => ctx
                    .p()
                    .pad_devices_of(pad)
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
            ctx.copy_rack_pads(d.id, *new_id, d.track, &mut Default::default())
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
        DeviceCommand::SetSidechain { device, source } => {
            crate::sidechain::set_sidechain(ctx, *device, *source)
        }
        DeviceCommand::ListBuiltin | DeviceCommand::GetDescriptor { .. } => {
            Err(unsupported("not a document command"))
        }
    }
}

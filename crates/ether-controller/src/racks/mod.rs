//! Racks, macros and modulation (v0.2; model
//! `ether_model::{rack, modulation}`, protocol `ether_protocol::racks`, CONTRACTS.md §12.6).
//!
//! - [`rack_command`], [`modulation_command`]: document commands (dispatched from
//!   `doc::apply`), one undo step each; creates are idempotent (client-chosen ids).
//!   `Modulation::ListModulatorKinds` is answered in `handlers.rs`
//!   (`ether_devices::modulators::all`).
//! - [`chain_racks_desc`], [`modulation_desc`]: called by `compile.rs` per track
//!   (`TrackDesc::{chain_racks, modulation}`), see [`compile`].
//! - Modulator param drags and depth drags are document edits sent with a gesture; the
//!   graph is republished (at most once per controller tick) and the engine carries the
//!   modulators' running state over (`ether_core::modulation`).
//! - Cascades on delete are in `doc/mod.rs`. [`copy_for_track`] copies the copied racks'
//!   chains (with their devices) and every device's modulators and mappings for
//!   `Device::Duplicate` and `Track::Duplicate`.
//!
//! Chain content rules (Ableton): audio effect rack chains take audio effects only (audio
//! effect racks included), MIDI effect rack chains MIDI effects only (MIDI effect racks
//! included), instrument rack chains anything, a drum rack included (as the chain's
//! instrument; its pads are edited with `DrumRack` commands, as on a track chain). Racks nest
//! up to `MAX_RACK_DEPTH` levels (every rack on the way down counts, drum racks included) and
//! never inside themselves; no drum rack ever sits under a drum pad (the model enforces all
//! three). A drum pad's chain may hold chain racks (inserted with `DrumRack::InsertDevice`).
//! Moving a device first drops the modulation mappings its new place puts out of scope, for
//! the device and everything nested inside it.

mod compile;

use std::collections::BTreeMap;

use ether_core::protocol::devices::{DeviceCategory, DeviceSpec};
use ether_core::protocol::model::*;
use ether_core::protocol::racks::{ModulationCommand, RackCommand};

pub use compile::{chain_racks_desc, modulation_desc};

use crate::doc::{DocCtx, builtin_descriptor, default_device_name, order_before};
use crate::tx::{CmdResult, invalid, not_found};
use crate::{MAX_VOLUME_DB, SILENCE_DB};

/// A new chain of `rack` with default mix and full zones.
fn new_chain(id: RackChainId, rack: DeviceId, order: OrderKey, name: String) -> RackChain {
    RackChain {
        id,
        rack,
        order,
        name,
        color: None,
        volume: Decibels::UNITY,
        pan: Pan(0.0),
        mute: false,
        solo: false,
        keys: Zone::FULL,
        velocities: Zone::FULL,
        select: Zone::FULL,
    }
}

/// A freshly inserted audio / MIDI effect rack (`device`, just inserted as `rack`) starts with
/// one empty chain, "chain 1" (an empty rack lets nothing through, an empty chain passes
/// everything: `ether_core::rack_chains`), in the same undo step. Its id is derived from the
/// rack's (`derive_id(rack, 0)`), so every client computes the same one. Instrument racks
/// start empty (their chains come with an instrument); duplicates, presets and loaded
/// documents keep exactly their chains (they never go through the insert commands).
pub fn add_first_chain(ctx: &mut DocCtx, rack: DeviceId, device: &BuiltinDevice) -> CmdResult<()> {
    if !matches!(
        device.device_type(),
        BuiltinDeviceType::AudioEffectRack | BuiltinDeviceType::MidiEffectRack
    ) {
        return Ok(());
    }
    let id: RackChainId = derive_id(rack, 0);
    if ctx.p().rack_chains.contains_key(&id) {
        return Ok(());
    }
    let order = order_before::<RackChainId>(&[], None)?;
    ctx.tx.insert(Entity::RackChain(new_chain(
        id,
        rack,
        order,
        "chain 1".into(),
    )))
}

/// Whether `d` is a chain rack (instrument / audio effect / MIDI effect rack).
pub fn is_chain_rack(d: &Device) -> bool {
    matches!(&d.kind, DeviceKind::Builtin { device } if device.device_type().is_rack())
}

fn rack_type(d: &Device) -> Option<BuiltinDeviceType> {
    match &d.kind {
        DeviceKind::Builtin { device } if device.device_type().is_rack() => {
            Some(device.device_type())
        }
        _ => None,
    }
}

/// A chain rack, on a track chain, a drum pad or nested in a rack chain.
fn rack(ctx: &DocCtx, id: DeviceId) -> CmdResult<Device> {
    let d = ctx.device(id)?;
    if !is_chain_rack(&d) {
        return Err(invalid(format!("device {id} is not a rack")));
    }
    Ok(d)
}

/// Whether a device of `kind` (`device`: an existing one being moved) may go onto a chain of
/// rack `host`: drum racks only on instrument rack chains and never under a drum pad, no
/// racks deeper than `MAX_RACK_DEPTH` (every rack on the way down counts, drum racks
/// included), never inside itself.
fn check_nestable(
    ctx: &DocCtx,
    kind: &DeviceKind,
    device: Option<DeviceId>,
    host: DeviceId,
) -> CmdResult<()> {
    let p = ctx.p();
    let drums = kind.is_drum_rack() || device.is_some_and(|id| p.has_nested_drum_rack(id));
    if kind.is_drum_rack()
        && !matches!(
            p.devices.get(&host).map(|h| &h.kind),
            Some(DeviceKind::Builtin {
                device: BuiltinDevice::InstrumentRack
            })
        )
    {
        return Err(invalid("drum racks only go on instrument rack chains"));
    }
    if drums && p.is_under_drum_pad(host) {
        return Err(invalid("drum racks cannot be nested in drum pads"));
    }
    if !kind.is_any_rack() {
        return Ok(());
    }
    let path = p.device_path(host);
    if let Some(id) = device
        && path.as_ref().is_none_or(|path| path.contains(&id))
    {
        return Err(invalid("a rack cannot go inside itself"));
    }
    // `host` is at level `path.len()`; the device one below, with what it holds.
    let above = path.map_or(MAX_RACK_DEPTH, |r| r.len());
    let below = device.map_or(0, |id| p.nested_rack_height(id));
    if above + 1 + below > MAX_RACK_DEPTH {
        return Err(invalid(format!("racks nest at most {MAX_RACK_DEPTH} deep")));
    }
    Ok(())
}

fn chain(ctx: &DocCtx, id: RackChainId) -> CmdResult<RackChain> {
    ctx.p()
        .rack_chains
        .get(&id)
        .cloned()
        .ok_or_else(|| not_found(format!("rack chain {id}")))
}

fn set_chain(ctx: &mut DocCtx, id: RackChainId, change: RackChainChange) -> CmdResult<()> {
    ctx.tx.update(EntityUpdate::RackChain { id, change })
}

fn modulator(ctx: &DocCtx, id: ModulatorId) -> CmdResult<Modulator> {
    ctx.p()
        .modulators
        .get(&id)
        .cloned()
        .ok_or_else(|| not_found(format!("modulator {id}")))
}

fn mapping(ctx: &DocCtx, id: ModMappingId) -> CmdResult<ModMapping> {
    ctx.p()
        .mod_mappings
        .get(&id)
        .cloned()
        .ok_or_else(|| not_found(format!("modulation mapping {id}")))
}

fn chain_siblings(
    p: &Project,
    rack: DeviceId,
    except: Option<RackChainId>,
) -> Vec<(OrderKey, RackChainId)> {
    p.chains_of(rack)
        .into_iter()
        .filter(|c| Some(c.id) != except)
        .map(|c| (c.order.clone(), c.id))
        .collect()
}

fn chain_device_siblings(
    p: &Project,
    chain: RackChainId,
    except: Option<DeviceId>,
) -> Vec<(OrderKey, DeviceId)> {
    p.chain_devices_of(chain)
        .into_iter()
        .filter(|d| Some(d.id) != except)
        .map(|d| (d.order.clone(), d.id))
        .collect()
}

fn track_siblings(p: &Project, track: TrackId, except: DeviceId) -> Vec<(OrderKey, DeviceId)> {
    p.devices_of(track)
        .into_iter()
        .filter(|d| d.id != except)
        .map(|d| (d.order.clone(), d.id))
        .collect()
}

/// Whether a device of `category` may sit on a chain of a `rack` rack.
fn check_chain_fit(rack: BuiltinDeviceType, category: Option<DeviceCategory>) -> CmdResult<()> {
    let ok = match (rack, category) {
        (_, None) => true,
        (BuiltinDeviceType::AudioEffectRack, Some(c)) => c == DeviceCategory::AudioEffect,
        (BuiltinDeviceType::MidiEffectRack, Some(c)) => c == DeviceCategory::NoteEffect,
        _ => true,
    };
    if ok {
        Ok(())
    } else {
        Err(invalid(match rack {
            BuiltinDeviceType::AudioEffectRack => "audio effect racks only hold audio effects",
            _ => "MIDI effect racks only hold MIDI effects",
        }))
    }
}

fn default_params(device: &BuiltinDevice) -> BTreeMap<ParamId, f64> {
    builtin_descriptor(device)
        .params
        .iter()
        .map(|p| (p.id, p.default))
        .collect()
}

fn check_zone(what: &str, z: Zone) -> CmdResult<()> {
    if z.lo > z.hi || z.hi > 127 {
        return Err(invalid(format!("{what} zone must be lo <= hi <= 127")));
    }
    Ok(())
}

/// Remove the modulation mappings that a move of `device` onto `new_chain` (`None`: the
/// track chain) would put out of scope, for the device and every device nested inside it,
/// with a warning. Runs before the move: undoing replays the ops backwards, so each
/// mapping comes back once its device is in scope again.
fn drop_out_of_scope_mappings(
    ctx: &mut DocCtx,
    device: DeviceId,
    new_chain: Option<RackChainId>,
) -> CmdResult<()> {
    let p = ctx.p();
    // The racks the device lands inside: the new chain's rack and the racks around it.
    let mut outside: Vec<DeviceId> = Vec::new();
    if let Some(rack) = new_chain
        .and_then(|c| p.rack_chains.get(&c))
        .map(|c| c.rack)
    {
        outside.push(rack);
        outside.extend(p.enclosing_racks(rack).unwrap_or_default());
    }
    let dead: Vec<ModMappingId> = p
        .mod_mappings
        .values()
        .filter(|m| m.device == device || p.is_inside(m.device, device))
        .filter(|m| {
            let source_host = match m.source {
                ModSource::Modulator { modulator } => {
                    p.modulators.get(&modulator).map(|x| x.device)
                }
                ModSource::Macro { rack, .. } => Some(rack),
            };
            let Some(host) = source_host else {
                return true;
            };
            // In scope: sourced from the target itself (its own modulators), from a rack
            // between it and the moved device (they move together), or from a rack the
            // moved device lands inside.
            let within = m.device != device
                && p.enclosing_racks(m.device)
                    .unwrap_or_default()
                    .into_iter()
                    .take_while(|&r| r != device)
                    .chain([device])
                    .any(|r| r == host);
            host != m.device && !within && !outside.contains(&host)
        })
        .map(|m| m.id)
        .collect();
    if !dead.is_empty() {
        let name = ctx.device(device)?.name;
        for m in dead {
            ctx.tx.remove(EntityKey::ModMapping(m))?;
        }
        ctx.warnings.push(
            crate::text::msg("notice.rack.outside_modulation_removed")
                .arg("device", name)
                .into(),
        );
    }
    Ok(())
}

/// Put track-chain device `d` onto `chain` with `order` (modulators must be gone already).
/// A sidechain keyed from a chain of its own track is cut (its rack might no longer come
/// before it); other sidechains stay.
fn onto_chain(ctx: &mut DocCtx, d: &Device, chain: RackChainId, order: OrderKey) -> CmdResult<()> {
    if crate::sidechain::depends_on_position(d) {
        crate::sidechain::cut(ctx, d)?;
        ctx.warnings.push(
            crate::text::msg("notice.device.sidechain_removed")
                .arg("device", d.name.as_str())
                .into(),
        );
    }
    drop_out_of_scope_mappings(ctx, d.id, Some(chain))?;
    if d.chain != Some(chain) {
        ctx.set_device(d.id, DeviceChange::Chain(Some(chain)))?;
    }
    ctx.set_device(d.id, DeviceChange::Order(order))
}

pub fn rack_command(ctx: &mut DocCtx, command: &RackCommand) -> CmdResult<()> {
    match command {
        RackCommand::AddChain {
            id,
            rack: rack_id,
            name,
            before,
        } => {
            if ctx.p().rack_chains.contains_key(id) {
                return Ok(());
            }
            let r = rack(ctx, *rack_id)?;
            let order = order_before(&chain_siblings(ctx.p(), r.id, None), *before)?;
            let n = ctx.p().chains_of(r.id).len() + 1;
            ctx.tx.insert(Entity::RackChain(new_chain(
                *id,
                r.id,
                order,
                name.clone().unwrap_or_else(|| {
                    crate::text::default_name(&crate::text::msg("name.rack.chain").arg("n", n))
                }),
            )))
        }
        RackCommand::RemoveChain { id } => {
            chain(ctx, *id)?;
            ctx.delete_rack_chain(*id)
        }
        RackCommand::RenameChain { id, name } => {
            chain(ctx, *id)?;
            set_chain(ctx, *id, RackChainChange::Name(name.clone()))
        }
        RackCommand::SetChainColor { id, color } => {
            chain(ctx, *id)?;
            set_chain(ctx, *id, RackChainChange::Color(*color))
        }
        RackCommand::MoveChain { id, before } => {
            let c = chain(ctx, *id)?;
            if *before == Some(c.id) {
                return Ok(());
            }
            let order = order_before(&chain_siblings(ctx.p(), c.rack, Some(c.id)), *before)?;
            set_chain(ctx, c.id, RackChainChange::Order(order))
        }
        RackCommand::CopyChain {
            id,
            new_id,
            rack,
            before,
            cut,
        } => copy_chain_command(ctx, *id, *new_id, *rack, *before, *cut),
        RackCommand::SetChainMix {
            id,
            volume,
            pan,
            mute,
            solo,
        } => {
            let c = chain(ctx, *id)?;
            if let Some(v) = volume {
                if !v.0.is_finite() {
                    return Err(invalid("volume must be finite"));
                }
                let v = Decibels(v.0.clamp(SILENCE_DB, MAX_VOLUME_DB));
                if v != c.volume {
                    set_chain(ctx, c.id, RackChainChange::Volume(v))?;
                }
            }
            if let Some(p) = pan {
                if !p.0.is_finite() {
                    return Err(invalid("pan must be finite"));
                }
                let p = Pan(p.0.clamp(-1.0, 1.0));
                if p != c.pan {
                    set_chain(ctx, c.id, RackChainChange::Pan(p))?;
                }
            }
            if let Some(m) = mute
                && *m != c.mute
            {
                set_chain(ctx, c.id, RackChainChange::Mute(*m))?;
            }
            if let Some(s) = solo
                && *s != c.solo
            {
                set_chain(ctx, c.id, RackChainChange::Solo(*s))?;
            }
            Ok(())
        }
        RackCommand::SetChainZones {
            id,
            keys,
            velocities,
            select,
        } => {
            let c = chain(ctx, *id)?;
            if let Some(z) = keys {
                check_zone("key", *z)?;
                if *z != c.keys {
                    set_chain(ctx, c.id, RackChainChange::Keys(*z))?;
                }
            }
            if let Some(z) = velocities {
                check_zone("velocity", *z)?;
                if *z != c.velocities {
                    set_chain(ctx, c.id, RackChainChange::Velocities(*z))?;
                }
            }
            if let Some(z) = select {
                check_zone("selector", *z)?;
                if *z != c.select {
                    set_chain(ctx, c.id, RackChainChange::Select(*z))?;
                }
            }
            Ok(())
        }
        RackCommand::InsertDevice {
            id,
            chain: chain_id,
            device,
            before,
        } => {
            if ctx.p().devices.contains_key(id) {
                return Ok(());
            }
            let c = chain(ctx, *chain_id)?;
            let r = rack(ctx, c.rack)?;
            let ty = rack_type(&r).expect("checked");
            let order = order_before(&chain_device_siblings(ctx.p(), c.id, None), *before)?;
            let (kind, name, params) = match device {
                DeviceSpec::Builtin { device } => {
                    let kind = DeviceKind::Builtin {
                        device: device.clone(),
                    };
                    check_nestable(ctx, &kind, None, r.id)?;
                    let desc = builtin_descriptor(device);
                    check_chain_fit(ty, Some(desc.category))?;
                    (kind, default_device_name(device), default_params(device))
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
                    check_chain_fit(ty, desc.as_ref().map(|d| d.category))?;
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
                id: *id,
                track: r.track,
                order,
                name,
                enabled: true,
                kind,
                params,
                sidechain: None,
                pad: None,
                chain: Some(c.id),
            }))?;
            if let DeviceSpec::Builtin { device } = device {
                add_first_chain(ctx, *id, device)?;
            }
            Ok(())
        }
        RackCommand::MoveDevice {
            id,
            chain: to,
            before,
        } => {
            let d = ctx.device(*id)?;
            if *before == Some(d.id) {
                return Ok(());
            }
            if d.pad.is_some() {
                return Err(invalid(format!(
                    "device {} is on a drum pad: use DrumRack::MoveDevice",
                    d.id
                )));
            }
            match to {
                Some(chain_id) => {
                    let c = chain(ctx, *chain_id)?;
                    let r = rack(ctx, c.rack)?;
                    if r.track != d.track {
                        return Err(invalid("a chain device must be on the rack's track"));
                    }
                    check_nestable(ctx, &d.kind, Some(d.id), r.id)?;
                    let category = ctx.host.descriptor(d.id, &d.kind).map(|x| x.category);
                    check_chain_fit(rack_type(&r).expect("checked"), category)?;
                    if !ctx.p().modulators_of(d.id).is_empty() {
                        return Err(invalid(
                            "devices with modulators stay on the track chain (put the modulators on the rack)",
                        ));
                    }
                    let order =
                        order_before(&chain_device_siblings(ctx.p(), c.id, Some(d.id)), *before)?;
                    onto_chain(ctx, &d, c.id, order)
                }
                None => {
                    let order = order_before(&track_siblings(ctx.p(), d.track, d.id), *before)?;
                    if d.chain.is_some() {
                        drop_out_of_scope_mappings(ctx, d.id, None)?;
                        ctx.set_device(d.id, DeviceChange::Chain(None))?;
                    }
                    ctx.set_device(d.id, DeviceChange::Order(order))?;
                    crate::midi_fx::check_chain_order(ctx.p(), d.track)
                }
            }
        }
        RackCommand::Group {
            rack: rack_id,
            rack_type,
            chain: chain_id,
            devices,
        } => group(ctx, *rack_id, *rack_type, *chain_id, devices),
    }
}

/// Rack presets (CONTRACTS.md §13.9): append a device of `kind` to the end
/// of `chain_id` with the content rules of `Rack::InsertDevice` (no nested racks, chain
/// fit). Built-ins get their default params, plugins are instantiated with `kind`'s state.
/// Returns the inserted device.
pub fn append_chain_device(
    ctx: &mut DocCtx,
    id: DeviceId,
    chain_id: RackChainId,
    kind: DeviceKind,
) -> CmdResult<Device> {
    let c = chain(ctx, chain_id)?;
    let r = rack(ctx, c.rack)?;
    let ty = rack_type(&r).expect("checked");
    check_nestable(ctx, &kind, None, r.id)?;
    let order = order_before(&chain_device_siblings(ctx.p(), c.id, None), None)?;
    let (name, params) = match &kind {
        DeviceKind::Missing { .. } => {
            return Err(invalid("a missing device can't be added to a rack chain"));
        }
        DeviceKind::Builtin { device } => {
            let desc = builtin_descriptor(device);
            check_chain_fit(ty, Some(desc.category))?;
            (default_device_name(device), default_params(device))
        }
        DeviceKind::Plugin { plugin } => {
            let desc = ctx.host.instantiate_plugin(id, plugin)?;
            check_chain_fit(ty, desc.as_ref().map(|d| d.category))?;
            let name = desc
                .as_ref()
                .map(|d| d.name.clone())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| plugin.name.clone());
            (name, BTreeMap::new())
        }
    };
    let d = Device {
        sidechain_tap: None,
        id,
        track: r.track,
        order,
        name,
        enabled: true,
        kind,
        params,
        sidechain: None,
        pad: None,
        chain: Some(c.id),
    };
    ctx.tx.insert(Entity::Device(d.clone()))?;
    Ok(d)
}

/// `Rack::Group`: consecutive track-chain devices into a new rack with one chain. The
/// devices' modulators move to the rack (new ids derived from the rack id, in the order
/// modulators then mappings, each sorted by id) so their mappings keep working.
fn group(
    ctx: &mut DocCtx,
    rack_id: DeviceId,
    rack_type: BuiltinDeviceType,
    chain_id: RackChainId,
    ids: &[DeviceId],
) -> CmdResult<()> {
    if ctx.p().devices.contains_key(&rack_id) {
        return Ok(());
    }
    if !rack_type.is_rack() {
        return Err(invalid(format!("{rack_type:?} is not a rack type")));
    }
    if ids.is_empty() {
        return Err(invalid("select at least one device to group"));
    }
    if ctx.p().rack_chains.contains_key(&chain_id) {
        return Err(invalid(format!("rack chain {chain_id} already exists")));
    }
    let devices: Vec<Device> = ids
        .iter()
        .map(|id| ctx.device(*id))
        .collect::<CmdResult<_>>()?;
    let track = devices[0].track;
    if devices
        .iter()
        .any(|d| d.track != track || d.pad.is_some() || d.chain.is_some())
    {
        return Err(invalid("grouped devices must be on one track chain"));
    }
    // Consecutive on the chain.
    let chain_order: Vec<DeviceId> = ctx.p().devices_of(track).iter().map(|d| d.id).collect();
    let mut idx: Vec<usize> = devices
        .iter()
        .map(|d| {
            chain_order
                .iter()
                .position(|x| *x == d.id)
                .expect("on the chain")
        })
        .collect();
    idx.sort_unstable();
    idx.dedup();
    if idx.len() != devices.len() || idx.windows(2).any(|w| w[1] != w[0] + 1) {
        return Err(invalid("grouped devices must be consecutive and distinct"));
    }
    let mut devices: Vec<Device> = idx
        .iter()
        .map(|&i| ctx.device(chain_order[i]))
        .collect::<CmdResult<_>>()?;
    for d in &devices {
        // The new rack is level 1: grouped racks go one level down with what they hold. A
        // drum rack groups into an instrument rack only.
        if d.kind.is_drum_rack() && rack_type != BuiltinDeviceType::InstrumentRack {
            return Err(invalid("drum racks only go on instrument rack chains"));
        }
        if d.kind.is_any_rack() && 2 + ctx.p().nested_rack_height(d.id) > MAX_RACK_DEPTH {
            return Err(invalid(format!("racks nest at most {MAX_RACK_DEPTH} deep")));
        }
        let category = ctx.host.descriptor(d.id, &d.kind).map(|x| x.category);
        check_chain_fit(rack_type, category)?;
    }
    let kind = BuiltinDevice::new(rack_type);
    let desc = builtin_descriptor(&kind);
    let t = ctx.track(track)?;
    if desc.category == DeviceCategory::Instrument && t.kind != TrackKind::Midi {
        return Err(invalid("instrument racks can only be on MIDI tracks"));
    }
    let first_order = devices[0].order.clone();
    ctx.tx.insert(Entity::Device(Device {
        sidechain_tap: None,
        id: rack_id,
        track,
        order: first_order,
        name: default_device_name(&kind),
        enabled: true,
        kind: DeviceKind::Builtin {
            device: kind.clone(),
        },
        params: default_params(&kind),
        sidechain: None,
        pad: None,
        chain: None,
    }))?;
    ctx.tx.insert(Entity::RackChain(RackChain {
        id: chain_id,
        rack: rack_id,
        order: order_before::<RackChainId>(&[], None)?,
        name: devices[0].name.clone(),
        color: None,
        volume: Decibels::UNITY,
        pan: Pan(0.0),
        mute: false,
        solo: false,
        keys: Zone::FULL,
        velocities: Zone::FULL,
        select: Zone::FULL,
    }))?;
    // Re-host the devices' modulators on the rack.
    let grouped: Vec<DeviceId> = devices.iter().map(|d| d.id).collect();
    let mut old_mods: Vec<Modulator> = ctx
        .p()
        .modulators
        .values()
        .filter(|m| grouped.contains(&m.device))
        .cloned()
        .collect();
    old_mods.sort_by_key(|m| m.id);
    let mut old_maps: Vec<ModMapping> = ctx
        .p()
        .mod_mappings
        .values()
        .filter(|m| {
            matches!(m.source, ModSource::Modulator { modulator }
                if old_mods.iter().any(|x| x.id == modulator))
        })
        .cloned()
        .collect();
    old_maps.sort_by_key(|m| m.id);
    for m in &old_maps {
        ctx.tx.remove(EntityKey::ModMapping(m.id))?;
    }
    let mut new_ids: BTreeMap<ModulatorId, ModulatorId> = BTreeMap::new();
    let mut next = 0u32;
    let mut last_order: Option<OrderKey> = None;
    for m in &old_mods {
        ctx.tx.remove(EntityKey::Modulator(m.id))?;
    }
    for m in &old_mods {
        let id: ModulatorId = derive_id(rack_id, next);
        next += 1;
        new_ids.insert(m.id, id);
        let order = OrderKey::between(last_order.as_ref(), None);
        last_order = Some(order.clone());
        ctx.tx.insert(Entity::Modulator(Modulator {
            id,
            device: rack_id,
            order,
            ..m.clone()
        }))?;
    }
    for d in &mut devices {
        let order = d.order.clone();
        let d0 = d.clone();
        onto_chain(ctx, &d0, chain_id, order)?;
    }
    for m in &old_maps {
        let ModSource::Modulator { modulator } = m.source else {
            continue;
        };
        let id: ModMappingId = derive_id(rack_id, next);
        next += 1;
        ctx.tx.insert(Entity::ModMapping(ModMapping {
            id,
            source: ModSource::Modulator {
                modulator: new_ids[&modulator],
            },
            ..m.clone()
        }))?;
    }
    Ok(())
}

/// Track / device duplication: `devices` maps the original devices (a track's, or a device
/// with its drum pads' devices) to their copies (already inserted). Copies every copied rack's chains with their devices (added to `devices`),
/// then the modulators and mappings among all copied devices.
pub fn copy_for_track(
    ctx: &mut DocCtx,
    devices: &mut BTreeMap<DeviceId, DeviceId>,
) -> CmdResult<()> {
    let racks: Vec<(DeviceId, DeviceId)> = devices
        .iter()
        .filter(|(src, _)| ctx.p().devices.get(src).is_some_and(is_chain_rack))
        .map(|(a, b)| (*a, *b))
        .collect();
    for (src, dst) in racks {
        copy_chains(ctx, src, dst, devices)?;
    }
    copy_modulation(ctx, devices)
}

fn copy_chains(
    ctx: &mut DocCtx,
    src: DeviceId,
    dst: DeviceId,
    devices: &mut BTreeMap<DeviceId, DeviceId>,
) -> CmdResult<()> {
    let chains: Vec<RackChain> = ctx.p().chains_of(src).into_iter().cloned().collect();
    for c in chains {
        let id = ctx.new_id();
        copy_chain(ctx, &c, id, dst, c.order.clone(), devices)?;
    }
    Ok(())
}

/// Copy chain `c` (its settings and devices: nested racks with their chains, nested drum
/// racks with their pads) as `id` onto rack `dst` with `order`. `devices` receives old → new
/// device ids. A copied device's sidechain that can't hold at its new place (its own
/// track's post-fader on another track, or a chain tap of its own track) is cut, with a
/// notice.
fn copy_chain(
    ctx: &mut DocCtx,
    c: &RackChain,
    id: RackChainId,
    dst: DeviceId,
    order: OrderKey,
    devices: &mut BTreeMap<DeviceId, DeviceId>,
) -> CmdResult<()> {
    let track = ctx.device(dst)?.track;
    let mut nc = c.clone();
    nc.id = id;
    nc.rack = dst;
    nc.order = order;
    ctx.tx.insert(Entity::RackChain(nc))?;
    let chain_devices: Vec<Device> = ctx
        .p()
        .chain_devices_of(c.id)
        .into_iter()
        .cloned()
        .collect();
    for d in chain_devices {
        let mut nd = d.clone();
        nd.id = ctx.new_id();
        nd.track = track;
        nd.chain = Some(id);
        if let DeviceKind::Plugin { plugin } = &mut nd.kind
            && let Some(state) = ctx.host.plugin_state(d.id)
        {
            plugin.state = Some(state);
        }
        if nd.sidechain == Some(track)
            && (d.track != track || nd.sidechain_tap.is_some_and(|t| t.chain.is_some()))
        {
            nd.sidechain = None;
            nd.sidechain_tap = None;
            ctx.warnings.push(
                crate::text::msg("notice.device.sidechain_removed")
                    .arg("device", d.name.as_str())
                    .into(),
            );
        }
        let copy = nd.id;
        devices.insert(d.id, copy);
        ctx.tx.insert(Entity::Device(nd))?;
        // A nested rack: its chains too (depth bounded by the model).
        if is_chain_rack(&d) {
            copy_chains(ctx, d.id, copy, devices)?;
        }
        // A nested drum rack: its pads with their chains, and the chains of the racks
        // on them.
        if d.kind.is_drum_rack() {
            let mut pad_devices = BTreeMap::new();
            ctx.copy_rack_pads(d.id, copy, track, &mut pad_devices)?;
            for (&src, &dst) in &pad_devices {
                devices.insert(src, dst);
                if ctx.p().devices.get(&src).is_some_and(is_chain_rack) {
                    copy_chains(ctx, src, dst, devices)?;
                }
            }
        }
    }
    Ok(())
}

/// `Rack::CopyChain`: chain `id` copied as `new_id` onto `rack_id` (any rack, any track)
/// before `before`; `cut` then removes the source (a move between racks, one undo step).
///
/// - Content rules: every device of the chain must fit the target rack (`check_chain_fit`),
///   racks in it must nest there (`check_nestable`: depth, drum racks, never inside itself).
/// - Modulation: mappings between copied devices follow the copies (a macro of a rack in the
///   chain keeps driving its copy). A mapping from outside the chain is kept when its source
///   (a rack's macro, a track-chain rack's modulator) still surrounds the new place (the
///   target rack or a rack enclosing it); others are dropped, with a notice. So a duplicate in
///   the same rack keeps its macro mappings; a chain moved to another rack loses the old
///   rack's (macros are not remapped: macro N of another rack means something else).
/// - A cut on the same track carries the automation of its devices (track lanes and clip
///   envelopes, retargeted to the new devices); to another track it is dropped, like a device
///   envelope that can't follow its device.
fn copy_chain_command(
    ctx: &mut DocCtx,
    id: RackChainId,
    new_id: RackChainId,
    rack_id: DeviceId,
    before: Option<RackChainId>,
    cut: bool,
) -> CmdResult<()> {
    if ctx.p().rack_chains.contains_key(&new_id) {
        return Ok(());
    }
    let c = chain(ctx, id)?;
    let r = rack(ctx, rack_id)?;
    let ty = rack_type(&r).expect("checked");
    let top: Vec<Device> = ctx
        .p()
        .chain_devices_of(c.id)
        .into_iter()
        .cloned()
        .collect();
    for d in &top {
        check_nestable(ctx, &d.kind, Some(d.id), r.id)?;
        let category = ctx.host.descriptor(d.id, &d.kind).map(|x| x.category);
        check_chain_fit(ty, category)?;
    }
    if before.is_some_and(|b| ctx.p().rack_chains.get(&b).is_none_or(|b| b.rack != r.id)) {
        return Err(invalid("`before` must be a chain of the target rack"));
    }
    let order = order_before(&chain_siblings(ctx.p(), r.id, None), before)?;
    let mut ids = BTreeMap::new();
    copy_chain(ctx, &c, new_id, r.id, order, &mut ids)?;
    copy_modulation(ctx, &ids)?;
    // Mappings from outside the chain whose source still surrounds the new place.
    let p = ctx.p();
    let mut around = vec![r.id];
    around.extend(p.enclosing_racks(r.id).unwrap_or_default());
    let outside: Vec<ModMapping> = p
        .mod_mappings
        .values()
        .filter(|m| {
            ids.contains_key(&m.device)
                && match m.source {
                    ModSource::Macro { rack, .. } => !ids.contains_key(&rack),
                    ModSource::Modulator { modulator } => p
                        .modulators
                        .get(&modulator)
                        .is_none_or(|x| !ids.contains_key(&x.device)),
                }
        })
        .cloned()
        .collect();
    let mut dropped = false;
    for m in outside {
        let host = match m.source {
            ModSource::Macro { rack, .. } => Some(rack),
            ModSource::Modulator { modulator } => {
                ctx.p().modulators.get(&modulator).map(|x| x.device)
            }
        };
        if host.is_some_and(|h| around.contains(&h)) {
            let mid = ctx.new_id();
            ctx.tx.insert(Entity::ModMapping(ModMapping {
                id: mid,
                device: ids[&m.device],
                ..m
            }))?;
        } else {
            dropped = true;
        }
    }
    if dropped {
        ctx.warnings.push(
            crate::text::msg("notice.rack.chain_modulation_removed")
                .arg("chain", c.name.as_str())
                .into(),
        );
    }
    if !cut {
        return Ok(());
    }
    let from_track = ctx.device(c.rack)?.track;
    if from_track == r.track {
        carry_automation(ctx, &ids)?;
    }
    ctx.delete_rack_chain(c.id)
}

/// Copies the automation lanes (with their points) of the devices in `ids` (old → new) onto
/// the new devices, for a chain moved within its track.
fn carry_automation(ctx: &mut DocCtx, ids: &BTreeMap<DeviceId, DeviceId>) -> CmdResult<()> {
    let mut lanes: Vec<AutomationLane> = ctx
        .p()
        .automation_lanes
        .values()
        .filter(|l| l.target.device().is_some_and(|d| ids.contains_key(&d)))
        .cloned()
        .collect();
    lanes.sort_by_key(|l| l.id);
    for l in lanes {
        let target = match l.target {
            AutomationTarget::DeviceParam { device, param } => AutomationTarget::DeviceParam {
                device: ids[&device],
                param,
            },
            AutomationTarget::DeviceEnabled { device } => AutomationTarget::DeviceEnabled {
                device: ids[&device],
            },
            other => other,
        };
        let lane = ctx.new_id();
        let mut points: Vec<AutomationPoint> = ctx
            .p()
            .automation_points
            .values()
            .filter(|p| p.lane == l.id)
            .cloned()
            .collect();
        points.sort_by_key(|p| p.id);
        ctx.tx.insert(Entity::AutomationLane(AutomationLane {
            id: lane,
            target,
            ..l
        }))?;
        for pt in points {
            let pid = ctx.new_id();
            ctx.tx.insert(Entity::AutomationPoint(AutomationPoint {
                id: pid,
                lane,
                ..pt
            }))?;
        }
    }
    Ok(())
}

/// Copy the modulators of the devices in `devices` (old → new) and the mappings between
/// them (source and target both copied) onto the copies.
pub fn copy_modulation(ctx: &mut DocCtx, devices: &BTreeMap<DeviceId, DeviceId>) -> CmdResult<()> {
    let mut mods: Vec<Modulator> = ctx
        .p()
        .modulators
        .values()
        .filter(|m| devices.contains_key(&m.device))
        .cloned()
        .collect();
    mods.sort_by_key(|m| m.id);
    let mut new_mods: BTreeMap<ModulatorId, ModulatorId> = BTreeMap::new();
    for m in mods {
        let mut nm = m.clone();
        nm.id = ctx.new_id();
        nm.device = devices[&m.device];
        new_mods.insert(m.id, nm.id);
        ctx.tx.insert(Entity::Modulator(nm))?;
    }
    let mut maps: Vec<ModMapping> = ctx
        .p()
        .mod_mappings
        .values()
        .filter(|m| devices.contains_key(&m.device))
        .cloned()
        .collect();
    maps.sort_by_key(|m| m.id);
    for m in maps {
        let source = match m.source {
            ModSource::Modulator { modulator } => match new_mods.get(&modulator) {
                Some(n) => ModSource::Modulator { modulator: *n },
                None => continue,
            },
            ModSource::Macro { rack, index } => match devices.get(&rack) {
                Some(r) => ModSource::Macro { rack: *r, index },
                None => continue,
            },
        };
        let id = ctx.new_id();
        ctx.tx.insert(Entity::ModMapping(ModMapping {
            id,
            source,
            device: devices[&m.device],
            ..m
        }))?;
    }
    Ok(())
}

// ─── Modulation ─────────────────────────────────────────────────────────────────────────

fn modulator_param_info(
    kind: ModulatorKind,
    param: ParamId,
) -> CmdResult<ether_core::protocol::devices::ParamInfo> {
    ether_devices::modulators::descriptor(kind)
        .params
        .into_iter()
        .find(|p| p.id == param)
        .ok_or_else(|| not_found(format!("modulator param {}", param.0)))
}

pub fn modulation_command(ctx: &mut DocCtx, command: &ModulationCommand) -> CmdResult<()> {
    match command {
        ModulationCommand::AddModulator {
            id,
            device,
            kind,
            name,
        } => {
            if ctx.p().modulators.contains_key(id) {
                return Ok(());
            }
            let d = ctx.device(*device)?;
            if d.pad.is_some() || d.chain.is_some() {
                return Err(invalid(
                    "modulators live on track-chain devices (put them on the rack)",
                ));
            }
            let desc = ether_devices::modulators::descriptor(*kind);
            let siblings: Vec<(OrderKey, ModulatorId)> = ctx
                .p()
                .modulators_of(d.id)
                .into_iter()
                .map(|m| (m.order.clone(), m.id))
                .collect();
            let order = order_before(&siblings, None)?;
            let name = name.clone().unwrap_or_else(|| {
                let same = ctx
                    .p()
                    .modulators_of(d.id)
                    .iter()
                    .filter(|m| m.kind == *kind)
                    .count();
                // lowercase like a device's default name ("lfo", "lfo 2"), in the names' language
                let base = crate::text::device_default_name(&desc.name);
                if same == 0 {
                    base
                } else {
                    format!("{base} {}", same + 1)
                }
            });
            ctx.tx.insert(Entity::Modulator(Modulator {
                id: *id,
                device: d.id,
                order,
                name,
                kind: *kind,
                params: desc.params.iter().map(|p| (p.id, p.default)).collect(),
                sidechain: None,
            }))
        }
        ModulationCommand::RemoveModulator { id } => {
            modulator(ctx, *id)?;
            let maps: Vec<ModMappingId> = ctx
                .p()
                .mod_mappings
                .values()
                .filter(|m| m.source == ModSource::Modulator { modulator: *id })
                .map(|m| m.id)
                .collect();
            for m in maps {
                ctx.tx.remove(EntityKey::ModMapping(m))?;
            }
            ctx.tx.remove(EntityKey::Modulator(*id))
        }
        ModulationCommand::RenameModulator { id, name } => {
            modulator(ctx, *id)?;
            ctx.tx.update(EntityUpdate::Modulator {
                id: *id,
                change: ModulatorChange::Name(name.clone()),
            })
        }
        ModulationCommand::SetModulatorParam {
            modulator: id,
            param,
            value,
        } => {
            let m = modulator(ctx, *id)?;
            if !value.is_finite() {
                return Err(invalid("param values must be finite"));
            }
            let info = modulator_param_info(m.kind, *param)?;
            let v = info.snap(value.clamp(info.min.min(info.max), info.min.max(info.max)));
            if m.params.get(param) == Some(&v) {
                return Ok(());
            }
            ctx.tx.update(EntityUpdate::Modulator {
                id: *id,
                change: ModulatorChange::Param {
                    param: *param,
                    value: Some(v),
                },
            })
        }
        ModulationCommand::ResetModulatorParam {
            modulator: id,
            param,
        } => {
            let m = modulator(ctx, *id)?;
            let info = modulator_param_info(m.kind, *param)?;
            ctx.tx.update(EntityUpdate::Modulator {
                id: *id,
                change: ModulatorChange::Param {
                    param: *param,
                    value: Some(info.default),
                },
            })
        }
        ModulationCommand::Map {
            id,
            source,
            device,
            param,
            depth,
        } => {
            if ctx.p().mod_mappings.contains_key(id) {
                return Ok(());
            }
            if !depth.is_finite() {
                return Err(invalid("depth must be finite"));
            }
            let d = ctx.device(*device)?;
            // The device's on/off switch is a target of every device (not a param).
            if *param != DEVICE_ENABLED_PARAM
                && let Some(desc) = ctx.host.descriptor(d.id, &d.kind)
            {
                let Some(info) = desc.params.iter().find(|p| p.id == *param) else {
                    return Err(not_found(format!("param {}", param.0)));
                };
                if !info.automatable {
                    return Err(invalid(format!("\"{}\" cannot be modulated", info.name)));
                }
            }
            match source {
                ModSource::Modulator { modulator: m } => {
                    modulator(ctx, *m)?;
                }
                ModSource::Macro { rack: r, .. } => {
                    // A chain rack or a drum rack (both have the 8 macros).
                    if !ctx.device(*r)?.kind.is_any_rack() {
                        return Err(invalid(format!("device {r} is not a rack")));
                    }
                }
            }
            if ctx
                .p()
                .mod_mappings
                .values()
                .any(|m| m.source == *source && m.device == *device && m.param == *param)
            {
                return Err(invalid("this source already modulates that parameter"));
            }
            ctx.tx.insert(Entity::ModMapping(ModMapping {
                id: *id,
                source: *source,
                device: *device,
                param: *param,
                depth: depth.clamp(-1.0, 1.0),
            }))
        }
        ModulationCommand::SetDepth { id, depth } => {
            let m = mapping(ctx, *id)?;
            if !depth.is_finite() {
                return Err(invalid("depth must be finite"));
            }
            let depth = depth.clamp(-1.0, 1.0);
            if m.depth == depth {
                return Ok(());
            }
            ctx.tx.update(EntityUpdate::ModMapping {
                id: *id,
                change: ModMappingChange::Depth(depth),
            })
        }
        ModulationCommand::Unmap { id } => {
            mapping(ctx, *id)?;
            ctx.tx.remove(EntityKey::ModMapping(*id))
        }
        ModulationCommand::SetSidechain {
            modulator: id,
            source,
        } => {
            let m = modulator(ctx, *id)?;
            if m.sidechain == *source {
                return Ok(());
            }
            if m.kind != ModulatorKind::EnvelopeFollower {
                return Err(invalid("only envelope followers take a sidechain"));
            }
            if let Some(src) = source {
                let t = ctx.track(*src)?;
                if t.kind == TrackKind::Master {
                    return Err(invalid("the master track cannot be a sidechain source"));
                }
                if ctx.device(m.device)?.track == *src {
                    return Err(invalid("a modulator cannot follow its own track"));
                }
            }
            ctx.tx.update(EntityUpdate::Modulator {
                id: *id,
                change: ModulatorChange::Sidechain(*source),
            })
        }
        ModulationCommand::ListModulatorKinds => Err(crate::tx::unsupported(
            "not a document command (answered by the controller)",
        )),
    }
}

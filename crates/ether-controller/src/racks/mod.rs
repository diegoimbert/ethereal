//! Racks, macros and modulation (v0.2, owned by the `racks-modulation` node; model
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
//! - Cascades on delete are in `doc/mod.rs`. [`copy_rack`] copies a rack's chains (with
//!   their devices) and every device's modulators and mappings for `Device::Duplicate`.
//!
//! Chain content rules (Ableton): audio effect rack chains take audio effects only, MIDI
//! effect rack chains MIDI effects only, instrument rack chains anything but racks. No
//! nesting in v0.2 (the model also enforces it).

mod compile;

use std::collections::BTreeMap;

use ether_core::protocol::devices::{DeviceCategory, DeviceSpec};
use ether_core::protocol::model::*;
use ether_core::protocol::racks::{ModulationCommand, RackCommand};

pub(crate) use compile::{chain_racks_desc, modulation_desc};

use crate::doc::{DocCtx, builtin_descriptor, order_before};
use crate::tx::{CmdResult, invalid, not_found};
use crate::{MAX_VOLUME_DB, SILENCE_DB};

/// Whether `d` is a chain rack (instrument / audio effect / MIDI effect rack).
pub(crate) fn is_chain_rack(d: &Device) -> bool {
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

fn rack(ctx: &DocCtx, id: DeviceId) -> CmdResult<Device> {
    let d = ctx.device(id)?;
    if !is_chain_rack(&d) || d.pad.is_some() || d.chain.is_some() {
        return Err(invalid(format!("device {id} is not a rack")));
    }
    Ok(d)
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

fn check_not_rack(kind: &DeviceKind) -> CmdResult<()> {
    match kind {
        DeviceKind::Builtin { device }
            if device.device_type().is_rack() || matches!(device, BuiltinDevice::DrumRack) =>
        {
            Err(invalid("racks cannot be nested in rack chains"))
        }
        _ => Ok(()),
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

/// Remove the modulation mappings targeting `device` that a move would put out of scope
/// (sources outside the device's new place), with a warning.
fn drop_out_of_scope_mappings(
    ctx: &mut DocCtx,
    device: DeviceId,
    new_chain: Option<RackChainId>,
) -> CmdResult<()> {
    let rack_of_new = new_chain
        .and_then(|c| ctx.p().rack_chains.get(&c))
        .map(|c| c.rack);
    let p = ctx.p();
    let dead: Vec<ModMappingId> = p
        .mod_mappings
        .values()
        .filter(|m| m.device == device)
        .filter(|m| {
            let source_host = match m.source {
                ModSource::Modulator { modulator } => {
                    p.modulators.get(&modulator).map(|x| x.device)
                }
                ModSource::Macro { rack, .. } => Some(rack),
            };
            // In scope: sourced from the device itself (its own modulators) or from the rack
            // whose chain it lands on.
            source_host != Some(device) && source_host != rack_of_new
        })
        .map(|m| m.id)
        .collect();
    if !dead.is_empty() {
        let name = ctx.device(device)?.name;
        for m in dead {
            ctx.tx.remove(EntityKey::ModMapping(m))?;
        }
        ctx.warnings.push(format!(
            "\"{name}\": modulation from outside its rack was removed"
        ));
    }
    Ok(())
}

/// Put track-chain device `d` onto `chain` with `order` (clearing what a chain device can't
/// have: its sidechain; modulators must be gone already).
fn onto_chain(ctx: &mut DocCtx, d: &Device, chain: RackChainId, order: OrderKey) -> CmdResult<()> {
    if d.sidechain.is_some() {
        ctx.set_device(d.id, DeviceChange::Sidechain(None))?;
        ctx.warnings
            .push(format!("\"{}\": its sidechain was removed", d.name));
    }
    drop_out_of_scope_mappings(ctx, d.id, Some(chain))?;
    if d.chain != Some(chain) {
        ctx.set_device(d.id, DeviceChange::Chain(Some(chain)))?;
    }
    ctx.set_device(d.id, DeviceChange::Order(order))
}

pub(crate) fn rack_command(ctx: &mut DocCtx, command: &RackCommand) -> CmdResult<()> {
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
            ctx.tx.insert(Entity::RackChain(RackChain {
                id: *id,
                rack: r.id,
                order,
                name: name.clone().unwrap_or_else(|| format!("Chain {n}")),
                color: None,
                volume: Decibels::UNITY,
                pan: Pan(0.0),
                mute: false,
                solo: false,
                keys: Zone::FULL,
                velocities: Zone::FULL,
                select: Zone::FULL,
            }))
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
                    check_not_rack(&kind)?;
                    let desc = builtin_descriptor(device);
                    check_chain_fit(ty, Some(desc.category))?;
                    (kind, desc.name, default_params(device))
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
                    check_chain_fit(ty, desc.as_ref().map(|d| d.category))?;
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
                track: r.track,
                order,
                name,
                enabled: true,
                kind,
                params,
                sidechain: None,
                pad: None,
                chain: Some(c.id),
            }))
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
                    check_not_rack(&d.kind)?;
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
        check_not_rack(&d.kind)?;
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
        id: rack_id,
        track,
        order: first_order,
        name: desc.name.clone(),
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

/// Copy rack `src`'s chains (with their devices) onto `dst`, and the modulators of `src`
/// and its chain devices with their mappings (sources and targets remapped to the copies).
/// Used by `Device::Duplicate` after the rack device itself was copied.
pub(crate) fn copy_rack(ctx: &mut DocCtx, src: DeviceId, dst: DeviceId) -> CmdResult<()> {
    let mut devices: BTreeMap<DeviceId, DeviceId> = BTreeMap::from([(src, dst)]);
    copy_chains(ctx, src, dst, &mut devices)?;
    copy_modulation(ctx, &devices)
}

/// Track duplication: `devices` maps the original track's devices to their copies (already
/// inserted). Copies every copied rack's chains with their devices (added to `devices`),
/// then the modulators and mappings among all copied devices. (Called from
/// `doc/tracks.rs` once the BCR wiring lands.)
#[allow(dead_code)]
pub(crate) fn copy_for_track(
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
    let track = ctx.device(dst)?.track;
    let chains: Vec<RackChain> = ctx.p().chains_of(src).into_iter().cloned().collect();
    for c in chains {
        let mut nc = c.clone();
        nc.id = ctx.new_id();
        nc.rack = dst;
        let new_chain = nc.id;
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
            nd.chain = Some(new_chain);
            if let DeviceKind::Plugin { plugin } = &mut nd.kind
                && let Some(state) = ctx.host.plugin_state(d.id)
            {
                plugin.state = Some(state);
            }
            devices.insert(d.id, nd.id);
            ctx.tx.insert(Entity::Device(nd))?;
        }
    }
    Ok(())
}

/// Copy the modulators of the devices in `devices` (old → new) and the mappings between
/// them (source and target both copied) onto the copies.
pub(crate) fn copy_modulation(
    ctx: &mut DocCtx,
    devices: &BTreeMap<DeviceId, DeviceId>,
) -> CmdResult<()> {
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

pub(crate) fn modulation_command(ctx: &mut DocCtx, command: &ModulationCommand) -> CmdResult<()> {
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
                if same == 0 {
                    desc.name.clone()
                } else {
                    format!("{} {}", desc.name, same + 1)
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
            if let Some(desc) = ctx.host.descriptor(d.id, &d.kind) {
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
                    rack(ctx, *r)?;
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

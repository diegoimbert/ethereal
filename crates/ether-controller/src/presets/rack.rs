//! Rack presets (v0.3, `rack-presets`; file format `ether_model::preset::PresetRack`,
//! CONTRACTS.md §13.9).
//!
//! - [`snapshot`] (`Save` of a rack device): the rack's chains in order with their devices
//!   (built-ins: every descriptor param + kind data for sample-based types; plugins: the
//!   live state blob and the param mirror), the rack's modulators in order, and the
//!   mappings whose source is one of the rack's macros or modulators and whose target is the
//!   rack itself or one of its chain devices. Mappings that leave the rack are not stored.
//! - [`rebuild`] (`Load` of a preset with `rack`, inside the load's one undo step): removes
//!   the rack's chains (with their devices, cascading their automation and mappings), the
//!   rack's modulators and every mapping sourced from the rack's macros or modulators, then
//!   creates the preset's structure through the racks code with ids `derive_id(seed, i)`:
//!   chains first, then each chain's devices in chain order, then modulators, then mappings
//!   (an index is consumed even when its entity is skipped, so replays stay aligned).
//!   A mapping the target no longer accepts (param gone, not automatable) is skipped with a
//!   warning; content the rack refuses (an instrument on an audio effect rack chain) fails
//!   the whole load.

use std::collections::BTreeMap;

use ether_core::protocol::model::{
    Base64Bytes, BuiltinDevice, Device, DeviceChange, DeviceId, DeviceKind, MediaId, ModMappingId,
    ModSource, ModulatorId, ParamId, PluginInstance, PresetChain, PresetChainDevice, PresetDevice,
    PresetModMapping, PresetModSource, PresetModTarget, PresetModulator, PresetRack, Project,
    RackChainId, derive_id,
};
use ether_core::protocol::racks::{ModulationCommand, RackCommand};

use crate::doc::DocCtx;
use crate::racks::{append_chain_device, modulation_command, rack_command};
use crate::tx::CmdResult;

use super::{kind_media, preset_device, preset_kind, remap_kind};

/// Plain values of every descriptor param of a built-in (missing = default).
fn builtin_params(d: &Device, kind: &BuiltinDevice) -> BTreeMap<ParamId, f64> {
    ether_devices::descriptor(kind.device_type())
        .params
        .iter()
        .map(|p| (p.id, d.params.get(&p.id).copied().unwrap_or(p.default)))
        .collect()
}

/// The structure of rack `rack` for a preset, and the media its chain devices reference.
/// `plugin_state` reads a plugin device's live state (falls back to the document's).
pub(super) fn snapshot(
    p: &Project,
    rack: DeviceId,
    plugin_state: &mut dyn FnMut(DeviceId) -> Option<Base64Bytes>,
) -> (PresetRack, Vec<MediaId>) {
    let mut media = Vec::new();
    let mut where_is: BTreeMap<DeviceId, (u32, u32)> = BTreeMap::new();
    let chains: Vec<PresetChain> = p
        .chains_of(rack)
        .into_iter()
        .enumerate()
        .map(|(ci, c)| PresetChain {
            name: c.name.clone(),
            color: c.color,
            volume: c.volume,
            pan: c.pan,
            mute: c.mute,
            solo: c.solo,
            keys: c.keys,
            velocities: c.velocities,
            select: c.select,
            devices: p
                .chain_devices_of(c.id)
                .into_iter()
                .enumerate()
                .map(|(di, d)| {
                    where_is.insert(d.id, (ci as u32, di as u32));
                    let mut out = PresetChainDevice {
                        name: d.name.clone(),
                        enabled: d.enabled,
                        device: preset_device(d),
                        params: BTreeMap::new(),
                        kind: None,
                        state: None,
                    };
                    match &d.kind {
                        DeviceKind::Builtin { device } => {
                            out.params = builtin_params(d, device);
                            if let Some(k) = preset_kind(device) {
                                media.extend(kind_media(&k));
                                out.kind = Some(k);
                            }
                        }
                        DeviceKind::Plugin { plugin } => {
                            out.state = plugin_state(d.id).or_else(|| plugin.state.clone());
                            out.params = d.params.clone();
                        }
                    }
                    out
                })
                .collect(),
        })
        .collect();
    let mods = p.modulators_of(rack);
    let modulators: Vec<PresetModulator> = mods
        .iter()
        .map(|m| PresetModulator {
            name: m.name.clone(),
            kind: m.kind,
            params: m.params.clone(),
        })
        .collect();
    let mut mappings: Vec<PresetModMapping> = p
        .mod_mappings
        .values()
        .filter_map(|m| {
            let source = match m.source {
                ModSource::Macro { rack: r, index } if r == rack => {
                    PresetModSource::Macro { index }
                }
                ModSource::Modulator { modulator } => PresetModSource::Modulator {
                    index: mods.iter().position(|x| x.id == modulator)? as u32,
                },
                ModSource::Macro { .. } => return None,
            };
            let target = if m.device == rack {
                PresetModTarget::Rack
            } else {
                let (chain, device) = *where_is.get(&m.device)?;
                PresetModTarget::ChainDevice { chain, device }
            };
            Some(PresetModMapping {
                source,
                target,
                param: m.param,
                depth: m.depth,
            })
        })
        .collect();
    let target_key = |t: &PresetModTarget| match *t {
        PresetModTarget::Rack => (0, 0, 0),
        PresetModTarget::ChainDevice { chain, device } => (1, chain, device),
    };
    let source_key = |s: &PresetModSource| match *s {
        PresetModSource::Macro { index } => (0, u32::from(index)),
        PresetModSource::Modulator { index } => (1, index),
    };
    mappings.sort_by_key(|m| (target_key(&m.target), m.param, source_key(&m.source)));
    media.sort();
    media.dedup();
    (
        PresetRack {
            chains,
            modulators,
            mappings,
        },
        media,
    )
}

/// Replace rack `rack`'s structure with `preset` (see the module docs). `media` maps the
/// preset's media ids to project media. Returns the plugin devices created (to reload
/// from their state once the edit is applied).
pub(super) fn rebuild(
    ctx: &mut DocCtx,
    rack: DeviceId,
    preset: &PresetRack,
    seed: RackChainId,
    media: &BTreeMap<MediaId, MediaId>,
) -> CmdResult<Vec<DeviceId>> {
    clear(ctx, rack)?;
    let mut next = 0u32;
    let mut mint = || {
        let i = next;
        next += 1;
        i
    };
    let chain_ids: Vec<RackChainId> = preset
        .chains
        .iter()
        .map(|_| derive_id(seed, mint()))
        .collect();
    for (c, id) in preset.chains.iter().zip(&chain_ids) {
        rack_command(
            ctx,
            &RackCommand::AddChain {
                id: *id,
                rack,
                name: Some(c.name.clone()),
                before: None,
            },
        )?;
        if c.color.is_some() {
            rack_command(
                ctx,
                &RackCommand::SetChainColor {
                    id: *id,
                    color: c.color,
                },
            )?;
        }
        rack_command(
            ctx,
            &RackCommand::SetChainMix {
                id: *id,
                volume: Some(c.volume),
                pan: Some(c.pan),
                mute: Some(c.mute),
                solo: Some(c.solo),
            },
        )?;
        rack_command(
            ctx,
            &RackCommand::SetChainZones {
                id: *id,
                keys: Some(c.keys),
                velocities: Some(c.velocities),
                select: Some(c.select),
            },
        )?;
    }
    let mut plugins = Vec::new();
    let mut device_ids: Vec<Vec<DeviceId>> = Vec::new();
    for (c, chain_id) in preset.chains.iter().zip(&chain_ids) {
        let mut ids = Vec::new();
        for pd in &c.devices {
            let id: DeviceId = derive_id(seed, mint());
            insert_device(ctx, id, *chain_id, pd, media)?;
            if matches!(pd.device, PresetDevice::Plugin { .. }) {
                plugins.push(id);
            }
            ids.push(id);
        }
        device_ids.push(ids);
    }
    let mut mod_ids: Vec<ModulatorId> = Vec::new();
    for m in &preset.modulators {
        let id: ModulatorId = derive_id(seed, mint());
        modulation_command(
            ctx,
            &ModulationCommand::AddModulator {
                id,
                device: rack,
                kind: m.kind,
                name: Some(m.name.clone()),
            },
        )?;
        let desc = ether_devices::modulators::descriptor(m.kind);
        for (param, value) in &m.params {
            if value.is_finite() && desc.params.iter().any(|p| p.id == *param) {
                modulation_command(
                    ctx,
                    &ModulationCommand::SetModulatorParam {
                        modulator: id,
                        param: *param,
                        value: *value,
                    },
                )?;
            }
        }
        mod_ids.push(id);
    }
    for m in &preset.mappings {
        let id: ModMappingId = derive_id(seed, mint());
        let source = match m.source {
            PresetModSource::Macro { index } => ModSource::Macro { rack, index },
            PresetModSource::Modulator { index } => match mod_ids.get(index as usize) {
                Some(modulator) => ModSource::Modulator {
                    modulator: *modulator,
                },
                None => continue,
            },
        };
        let device = match m.target {
            PresetModTarget::Rack => rack,
            PresetModTarget::ChainDevice { chain, device } => {
                match device_ids
                    .get(chain as usize)
                    .and_then(|c| c.get(device as usize))
                {
                    Some(d) => *d,
                    None => continue,
                }
            }
        };
        let map = ModulationCommand::Map {
            id,
            source,
            device,
            param: m.param,
            depth: m.depth,
        };
        if let Err(e) = modulation_command(ctx, &map) {
            ctx.warnings
                .push(format!("a modulation mapping was skipped: {}", e.message));
        }
    }
    Ok(plugins)
}

/// Remove the rack's chains (with their devices), its modulators and the mappings sourced
/// from its macros or modulators.
fn clear(ctx: &mut DocCtx, rack: DeviceId) -> CmdResult<()> {
    let chains: Vec<RackChainId> = ctx.p().chains_of(rack).iter().map(|c| c.id).collect();
    for c in chains {
        ctx.delete_rack_chain(c)?;
    }
    let p = ctx.p();
    let mappings: Vec<ModMappingId> = p
        .mod_mappings
        .values()
        .filter(|m| match m.source {
            ModSource::Macro { rack: r, .. } => r == rack,
            ModSource::Modulator { modulator } => p
                .modulators
                .get(&modulator)
                .is_some_and(|x| x.device == rack),
        })
        .map(|m| m.id)
        .collect();
    for m in mappings {
        modulation_command(ctx, &ModulationCommand::Unmap { id: m })?;
    }
    let mods: Vec<ModulatorId> = ctx.p().modulators_of(rack).iter().map(|m| m.id).collect();
    for m in mods {
        modulation_command(ctx, &ModulationCommand::RemoveModulator { id: m })?;
    }
    Ok(())
}

/// Create one chain device of a rack preset at the end of `chain`.
fn insert_device(
    ctx: &mut DocCtx,
    id: DeviceId,
    chain: RackChainId,
    pd: &PresetChainDevice,
    media: &BTreeMap<MediaId, MediaId>,
) -> CmdResult<()> {
    let kind = match &pd.device {
        PresetDevice::Builtin { device } => {
            let kind = pd
                .kind
                .as_ref()
                .filter(|k| k.device_type() == *device)
                .map(|k| remap_kind(k, media))
                .unwrap_or_else(|| BuiltinDevice::new(*device));
            DeviceKind::Builtin { device: kind }
        }
        PresetDevice::Plugin {
            format,
            plugin_id,
            name,
            vendor,
        } => DeviceKind::Plugin {
            plugin: PluginInstance {
                format: *format,
                plugin_id: plugin_id.clone(),
                name: name.clone(),
                vendor: vendor.clone(),
                version: String::new(),
                sandboxed: false,
                state: pd.state.clone(),
            },
        },
    };
    let d = append_chain_device(ctx, id, chain, kind)?;
    if !pd.name.trim().is_empty() && pd.name != d.name {
        ctx.set_device(id, DeviceChange::Name(pd.name.clone()))?;
    }
    if !pd.enabled {
        ctx.set_device(id, DeviceChange::Enabled(false))?;
    }
    let params: Vec<(ParamId, f64)> = match &d.kind {
        DeviceKind::Builtin { device } => ether_devices::descriptor(device.device_type())
            .params
            .iter()
            .map(|info| {
                let v = pd
                    .params
                    .get(&info.id)
                    .filter(|v| v.is_finite())
                    .map_or(info.default, |v| info.snap(*v));
                (info.id, v)
            })
            .filter(|(id, v)| d.params.get(id) != Some(v))
            .collect(),
        // The state blob is authoritative; a state-less plugin preset sets its param mirror.
        DeviceKind::Plugin { .. } if pd.state.is_some() => Vec::new(),
        DeviceKind::Plugin { .. } => pd
            .params
            .iter()
            .filter(|(_, v)| v.is_finite())
            .map(|(id, v)| (*id, *v))
            .collect(),
    };
    for (param, value) in params {
        ctx.set_device(
            id,
            DeviceChange::Param {
                param,
                value: Some(value),
            },
        )?;
    }
    Ok(())
}

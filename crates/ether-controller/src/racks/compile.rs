//! Document → engine descs of racks and modulation (`TrackDesc::{chain_racks, modulation}`).
//!
//! - Chain racks: every rack on the track chain with an engine node (even without chains, so
//!   MIDI effect racks merge/forward consistently), its chains in order with their devices
//!   (devices without a node are skipped), linear chain gain, solo folded into `mute`, zones
//!   and the document chain-selector value.
//! - Modulation: the modulators of the track-chain devices (all kind params, defaults filled
//!   in), every mapping whose target device is on the track (sources resolved to modulator
//!   indices / rack nodes; unknown params or nodes skipped) with the target's normalized
//!   document value as the base, and the macro values of the racks used as sources.

use ether_core::graph::{ChainEntry, ParamMapping};
use ether_core::modulation::{
    MACROS, MacroDesc, ModMappingDesc, ModSourceDesc, ModulationDesc, ModulatorDesc,
};
use ether_core::protocol::devices::{DeviceDescriptor, ParamInfo};
use ether_core::protocol::model::*;
use ether_core::rack_chains::{ChainRackDesc, ChainRackKind, RackChainDesc};

use crate::compile::CompileContext;

fn rack_kind(d: &Device) -> Option<ChainRackKind> {
    match &d.kind {
        DeviceKind::Builtin { device } => match device.device_type() {
            BuiltinDeviceType::InstrumentRack => Some(ChainRackKind::Instrument),
            BuiltinDeviceType::AudioEffectRack => Some(ChainRackKind::AudioEffect),
            BuiltinDeviceType::MidiEffectRack => Some(ChainRackKind::MidiEffect),
            _ => None,
        },
        DeviceKind::Plugin { .. } => None,
    }
}

/// Plain document value of `param` (default when unset).
fn doc_value(d: &Device, info: &ParamInfo) -> f64 {
    d.params.get(&info.id).copied().unwrap_or(info.default)
}

/// Normalized → plain mapping of a param (stepped: labels or a plain step).
pub(crate) fn param_mapping(info: &ParamInfo) -> ParamMapping {
    let steps = match (&info.labels, info.step) {
        (Some(l), _) if l.len() > 1 => Some(l.len() as u32),
        (_, Some(s)) if s > 0.0 => {
            let n = ((info.max - info.min).abs() / s).round() as u32 + 1;
            (n > 1).then_some(n)
        }
        _ => None,
    };
    ParamMapping {
        min: info.min,
        max: info.max,
        scale: info.scale,
        steps,
    }
}

/// `TrackDesc::chain_racks` of `track`.
pub(crate) fn chain_racks_desc(
    p: &Project,
    track: TrackId,
    ctx: &CompileContext,
) -> Vec<ChainRackDesc> {
    p.devices_of(track)
        .into_iter()
        .filter_map(|r| {
            let kind = rack_kind(r)?;
            let rack = (ctx.nodes)(r.id)?;
            let chains = p.chains_of(r.id);
            let any_solo = chains.iter().any(|c| c.solo);
            let selector = r
                .params
                .get(&ether_core::protocol::model::RACK_SELECTOR_PARAM)
                .copied()
                .unwrap_or(0.0)
                .round()
                .clamp(0.0, 127.0) as u8;
            Some(ChainRackDesc {
                rack,
                kind,
                selector,
                chains: chains
                    .into_iter()
                    .map(|c| RackChainDesc {
                        id: c.id,
                        chain: p
                            .chain_devices_of(c.id)
                            .into_iter()
                            .filter_map(|d| {
                                (ctx.nodes)(d.id).map(|node| ChainEntry {
                                    node,
                                    enabled: d.enabled,
                                    sidechain: None,
                                })
                            })
                            .collect(),
                        volume: c.volume.to_linear(),
                        pan: c.pan.0,
                        mute: c.mute || (any_solo && !c.solo),
                        keys: (c.keys.lo, c.keys.hi),
                        velocities: (c.velocities.lo, c.velocities.hi),
                        select: (c.select.lo, c.select.hi),
                    })
                    .collect(),
            })
        })
        .collect()
}

/// `TrackDesc::modulation` of `track`.
pub(crate) fn modulation_desc(p: &Project, track: TrackId, ctx: &CompileContext) -> ModulationDesc {
    let mut out = ModulationDesc::default();
    if p.modulators.is_empty() && p.mod_mappings.is_empty() {
        return out;
    }
    // Modulators of the track-chain devices.
    let mut index: Vec<ModulatorId> = Vec::new();
    for d in p.devices_of(track) {
        let Some(host) = (ctx.nodes)(d.id) else {
            continue;
        };
        for m in p.modulators_of(d.id) {
            let desc = ether_devices::modulators::descriptor(m.kind);
            out.modulators.push(ModulatorDesc {
                id: m.id,
                host,
                kind: m.kind,
                params: desc
                    .params
                    .iter()
                    .map(|i| (i.id, m.params.get(&i.id).copied().unwrap_or(i.default)))
                    .collect(),
                sidechain: m.sidechain,
            });
            index.push(m.id);
        }
    }
    // Mappings targeting devices on this track.
    let mut racks: Vec<DeviceId> = Vec::new();
    let mut maps: Vec<&ModMapping> = p
        .mod_mappings
        .values()
        .filter(|m| p.devices.get(&m.device).is_some_and(|d| d.track == track))
        .collect();
    maps.sort_by_key(|m| (m.device, m.param, m.id));
    let mut descs: Vec<(DeviceId, Option<DeviceDescriptor>)> = Vec::new();
    for m in maps {
        let Some(target) = p.devices.get(&m.device) else {
            continue;
        };
        let Some(node) = (ctx.nodes)(target.id) else {
            continue;
        };
        let source = match m.source {
            ModSource::Modulator { modulator } => {
                match index.iter().position(|x| *x == modulator) {
                    Some(i) => ModSourceDesc::Modulator(i as u32),
                    None => continue,
                }
            }
            ModSource::Macro { rack, index } => {
                let Some(r) = (ctx.nodes)(rack) else {
                    continue;
                };
                if !racks.contains(&rack) {
                    racks.push(rack);
                }
                ModSourceDesc::Macro { rack: r, index }
            }
        };
        let desc = match descs.iter().find(|(d, _)| *d == target.id) {
            Some((_, d)) => d.clone(),
            None => {
                let d = (ctx.descriptors)(target);
                descs.push((target.id, d.clone()));
                d
            }
        };
        let Some(info) = desc
            .as_ref()
            .and_then(|d| d.params.iter().find(|i| i.id == m.param))
        else {
            continue;
        };
        out.mappings.push(ModMappingDesc {
            source,
            node,
            param: m.param,
            depth: m.depth,
            mapping: param_mapping(info),
            base: info.to_normalized(doc_value(target, info)),
        });
    }
    for rack in racks {
        let (Some(r), Some(node)) = (p.devices.get(&rack), (ctx.nodes)(rack)) else {
            continue;
        };
        let mut values = [0.0; MACROS];
        for (i, v) in values.iter_mut().enumerate() {
            *v = r
                .params
                .get(&ParamId(i as u32))
                .copied()
                .unwrap_or(0.0)
                .clamp(0.0, 1.0);
        }
        out.macros.push(MacroDesc { rack: node, values });
    }
    out
}

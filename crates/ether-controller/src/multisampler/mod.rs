//! Multisampler zones (v0.2, owned by the `multisampler` node; model
//! `ether_model::multisampler`, CONTRACTS.md §12.4).
//!
//! - [`set_zones`]: `Device::SetZones` (document command from `doc/devices.rs`): one
//!   `DeviceChange::Kind` op (one undo step), then an in-place node update
//!   (`EngineBridge::update_builtin` with the resolved zone set, see [`updatable_in_place`];
//!   hosts without it re-create the node).
//! - Zone media load like the sampler's sample: multisamplers whose zones use newly loaded
//!   media are rebuilt (`handlers.rs::tick_impl`, next to samplers). Zone media may be
//!   external references (`media-references`); missing media play silent until relinked.

use ether_core::protocol::model::{
    BuiltinDevice, DeviceChange, DeviceId, DeviceKind, MAX_ZONES, MediaId, SampleZone,
};

use crate::doc::DocCtx;
use crate::tx::{CmdResult, invalid, not_found};

pub(crate) fn set_zones(ctx: &mut DocCtx, device: DeviceId, zones: &[SampleZone]) -> CmdResult<()> {
    let d = ctx.device(device)?;
    if !matches!(
        d.kind,
        DeviceKind::Builtin {
            device: BuiltinDevice::MultiSampler { .. }
        }
    ) {
        return Err(invalid(format!("device {} is not a multisampler", d.id)));
    }
    if zones.len() > MAX_ZONES {
        return Err(invalid(format!("at most {MAX_ZONES} zones")));
    }
    for m in zones.iter().filter_map(|z| z.media) {
        if !ctx.p().media.contains_key(&m) {
            return Err(not_found(format!("media {m}")));
        }
    }
    ctx.set_device(
        d.id,
        DeviceChange::Kind(DeviceKind::Builtin {
            device: BuiltinDevice::MultiSampler {
                zones: zones.to_vec(),
            },
        }),
    )
}

/// Whether a live node built from `old` takes `new` in place (`engine.rs`, next to the
/// sampler slices): only the zones of a multisampler changed.
pub(crate) fn updatable_in_place(old: &BuiltinDevice, new: &BuiltinDevice) -> bool {
    ether_devices::multisampler::updatable_in_place(old, new)
}

/// Whether `kind` is a multisampler with a zone on one of `media` (rebuilt on media load).
pub(crate) fn uses_media(kind: &DeviceKind, media: &[MediaId]) -> bool {
    matches!(kind, DeviceKind::Builtin {
        device: BuiltinDevice::MultiSampler { zones }
    } if zones.iter().any(|z| z.media.is_some_and(|m| media.contains(&m))))
}

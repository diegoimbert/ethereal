//! MIDI effects: chain ordering and scale data (v0.2, owned by the `midi-fx` node;
//! CONTRACTS.md §12.4.4).
//!
//! - [`check_chain_order`]: called by `doc/devices.rs` after `Device::{Insert, Move}`: MIDI
//!   effects (`DeviceCategory::NoteEffect`) must precede the first instrument of the track's
//!   chain and of each of its rack chains; `InvalidArgument` otherwise.
//! - Scale data ([`scale_for`]): the resolved `MusicalScale` a Scale Quantize / Random node
//!   gets through `Node::set_data` when it is created and whenever the track/project scale
//!   (or its `Scale` source param) changes.

use ether_core::protocol::devices::DeviceCategory;
use ether_core::protocol::model::{
    BuiltinDeviceType, Device, DeviceKind, MusicalScale, Project, ScaleKind, TrackId, TrackScale,
};

use crate::tx::{CmdResult, invalid};

/// Category of a built-in device (plugins: unknown here, never constrained).
fn category(d: &Device) -> Option<DeviceCategory> {
    match &d.kind {
        DeviceKind::Builtin { device } => {
            Some(ether_devices::descriptor(device.device_type()).category)
        }
        DeviceKind::Plugin { .. } => None,
    }
}

fn check(chain: &[&Device]) -> CmdResult<()> {
    let mut instrument: Option<&Device> = None;
    for d in chain {
        match category(d) {
            Some(DeviceCategory::Instrument) if instrument.is_none() => instrument = Some(d),
            Some(DeviceCategory::NoteEffect) => {
                if let Some(i) = instrument {
                    return Err(invalid(format!(
                        "MIDI effect \"{}\" must come before the instrument \"{}\"",
                        d.name, i.name
                    )));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Check the MIDI-effect ordering rule on `track`'s chain (and its rack chains).
pub(crate) fn check_chain_order(p: &Project, track: TrackId) -> CmdResult<()> {
    let top = p.devices_of(track);
    check(&top)?;
    for rack in &top {
        for chain in p.chains_of(rack.id) {
            check(&p.chain_devices_of(chain.id))?;
        }
    }
    Ok(())
}

/// Scale source param values of Scale Quantize (`Track` / `Project` / `Custom`).
const SOURCE_PROJECT: f64 = 1.0;

/// Whether `ty` takes the resolved scale through `Node::set_data`.
pub(crate) fn wants_scale(ty: BuiltinDeviceType) -> bool {
    matches!(
        ty,
        BuiltinDeviceType::ScaleQuantize | BuiltinDeviceType::Randomizer
    )
}

/// The scale pushed to `device`'s node: the track's resolved scale (a `FollowProject`
/// track uses the project scale, a `Chromatic` one chromatic), or the project scale when a
/// Scale Quantize's `Scale` source is `Project`. `None` for devices that take no scale.
pub(crate) fn scale_for(p: &Project, device: &Device) -> Option<MusicalScale> {
    let DeviceKind::Builtin { device: kind } = &device.kind else {
        return None;
    };
    let ty = kind.device_type();
    if !wants_scale(ty) {
        return None;
    }
    let source = device
        .params
        .get(&ether_devices::midi_fx::scale_quantize::SOURCE)
        .copied()
        .unwrap_or(0.0);
    if ty == BuiltinDeviceType::ScaleQuantize && source.round() == SOURCE_PROJECT {
        return Some(p.scale);
    }
    Some(match p.tracks.get(&device.track).map(|t| t.scale) {
        Some(TrackScale::Custom { scale }) => scale,
        Some(TrackScale::Chromatic) => MusicalScale {
            root: p.scale.root,
            kind: ScaleKind::Chromatic,
        },
        _ => p.scale,
    })
}

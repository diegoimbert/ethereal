//! Sidechain routing (roadmap v2; see CONTRACTS.md §11.10).
//!
//! `DeviceCommand::SetSidechain` is a document command ([`set_sidechain`], dispatched from
//! `doc/devices.rs`): `DeviceChange::Sidechain` / `DeviceChange::SidechainTap` ops in one
//! transaction, so one undo step. `compile.rs` copies `Device::sidechain` into
//! `ChainEntry::sidechain` and resolves the tap ([`crate::compile`]'s sidechain points);
//! the engine side (ordering, taps, PDC alignment) is `ether-core/src/sidechain.rs` and the
//! devices read the signal in `Node::process_sidechain`. Deleting the source track, chain or
//! pad cuts the sidechain (`doc/mod.rs`).
//!
//! Validation (on top of the model's, which rejects routing cycles and the same-track points
//! a device can't hear, `ether_model::SidechainTap`, with `InvalidArgument`):
//! - the source must be an audio/MIDI/group or return track (not master: it is always
//!   rendered last, so it can never feed a device);
//! - the device must have a sidechain input (`DeviceDescriptor::sidechain_inputs > 0`).
//!
//! Devices take a sidechain at any depth (track chain, drum pads, rack chains). Clearing
//! (`source: None`) is always allowed; setting the current value is a no-op.

use ether_core::protocol::devices::DeviceCategory;
use ether_core::protocol::model::{
    Device, DeviceChange, DeviceId, InputTap, Project, SidechainChain, SidechainTap, TrackId,
    TrackKind,
};
use ether_core::{ScPoint, SidechainTapDesc};

use crate::doc::DocCtx;
use crate::tx::{CmdResult, invalid};

pub fn set_sidechain(
    ctx: &mut DocCtx,
    device: DeviceId,
    source: Option<TrackId>,
    tap: Option<SidechainTap>,
) -> CmdResult<()> {
    let d = ctx.device(device)?;
    // The post-fader tap is stored as `None` (the original meaning).
    let tap = match source {
        None => None,
        Some(_) => tap.filter(|t| *t != SidechainTap::POST_FADER),
    };
    let old_tap = d.sidechain_tap.filter(|t| *t != SidechainTap::POST_FADER);
    if d.sidechain == source && old_tap == tap {
        return Ok(());
    }
    if let Some(src) = source {
        let track = ctx.track(src)?;
        if track.kind == TrackKind::Master {
            return Err(invalid("the master track cannot be a sidechain source"));
        }
        let inputs = ctx
            .host
            .descriptor(d.id, &d.kind)
            .map_or(0, |desc| desc.sidechain_inputs);
        if inputs == 0 {
            return Err(invalid(format!("device {} has no sidechain input", d.id)));
        }
    }
    if d.sidechain == source || d.sidechain_tap == tap {
        // One field changes: the end state is the only new state.
        if d.sidechain != source {
            return ctx.set_device(d.id, DeviceChange::Sidechain(source));
        }
        return ctx.set_device(d.id, DeviceChange::SidechainTap(tap));
    }
    // Both change: go through "no sidechain" (the tap isn't checked without a source), so
    // every intermediate state — and every state of the undo — is valid.
    if d.sidechain.is_some() {
        ctx.set_device(d.id, DeviceChange::Sidechain(None))?;
    }
    ctx.set_device(d.id, DeviceChange::SidechainTap(tap))?;
    if source.is_some() {
        ctx.set_device(d.id, DeviceChange::Sidechain(source))?;
    }
    Ok(())
}

/// Clears device `d`'s sidechain (source, then tap).
pub fn cut(ctx: &mut DocCtx, d: &Device) -> CmdResult<()> {
    if d.sidechain.is_some() {
        ctx.set_device(d.id, DeviceChange::Sidechain(None))?;
    }
    if d.sidechain_tap.is_some() {
        ctx.set_device(d.id, DeviceChange::SidechainTap(None))?;
    }
    Ok(())
}

/// Whether device `d`'s sidechain may stop being valid when `d` moves within its track
/// (into or out of a rack chain / drum pad): only a tap of a chain of its own track can
/// (its rack must stay before the device); other sources don't depend on the position.
pub fn depends_on_position(d: &Device) -> bool {
    d.sidechain == Some(d.track) && d.sidechain_tap.is_some_and(|t| t.chain.is_some())
}

/// `TrackDesc::sidechain_taps` of `track`: the engine point of every sidechained device on it
/// (any depth) that doesn't tap its source's post-fader output (devices without a node are
/// skipped, like in the chains).
pub fn taps_desc(
    p: &Project,
    track: TrackId,
    ctx: &crate::compile::CompileContext,
) -> Vec<SidechainTapDesc> {
    let mut out: Vec<SidechainTapDesc> = p
        .devices
        .values()
        .filter(|d| d.track == track)
        .filter_map(|d| {
            let src = d.sidechain?;
            let tap = d.sidechain_tap?;
            let point = engine_point(p, d, src, tap, ctx)?;
            let node = (ctx.nodes)(d.id)?;
            Some(SidechainTapDesc { node, point })
        })
        .collect();
    out.sort_by_key(|t| t.node);
    out
}

/// The engine point of `tap` on `source` for `consumer` (`None` = the post-fader default). `PreFx` is the
/// output of the first instrument of the track / chain (racks count), else its input
/// (`ether_model::SidechainTap`).
fn engine_point(
    p: &Project,
    consumer: &Device,
    source: TrackId,
    tap: SidechainTap,
    ctx: &crate::compile::CompileContext,
) -> Option<ScPoint> {
    let instrument = |devices: Vec<&Device>| {
        devices
            .into_iter()
            .find(|d| {
                (ctx.descriptors)(d).is_some_and(|desc| desc.category == DeviceCategory::Instrument)
            })
            .and_then(|d| (ctx.nodes)(d.id))
    };
    Some(match tap.chain {
        None => match tap.point {
            InputTap::PreFx => match instrument(p.devices_of(source)) {
                Some(node) => ScPoint::After(node),
                None => ScPoint::Track(InputTap::PreFx),
            },
            InputTap::PostFx => ScPoint::Track(InputTap::PostFx),
            InputTap::PostFader => return None,
        },
        Some(chain) => {
            let (id, devices) = match chain {
                SidechainChain::Rack { chain } => (chain.0.0, p.chain_devices_of(chain)),
                SidechainChain::Pad { pad } => (pad.0.0, p.pad_devices_of(pad)),
            };
            match tap.point {
                InputTap::PreFx => match instrument(devices) {
                    Some(node) => ScPoint::After(node),
                    None => ScPoint::ChainIn(id),
                },
                // Another chain of a rack holding the consumer: its output before the rack's
                // alignment (it runs first; the aligned output comes after every chain).
                InputTap::PostFx
                    if p.sibling_chain_edge(consumer)
                        .is_some_and(|e| p.is_inside(consumer.id, e.0) && e.1.0.0 == id) =>
                {
                    ScPoint::ChainRaw(id)
                }
                InputTap::PostFx => ScPoint::ChainOut(id),
                InputTap::PostFader => ScPoint::ChainMix(id),
            }
        }
    })
}

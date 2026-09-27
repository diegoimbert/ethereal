//! Sidechain routing (roadmap v2, owned by the `sidechain` node; see `docs/ROADMAP.md` and
//! CONTRACTS.md §11.10).
//!
//! `DeviceCommand::SetSidechain` is a document command ([`set_sidechain`], dispatched from
//! `doc/devices.rs`): one `DeviceChange::Sidechain` op, so one undo step. `compile.rs`
//! copies `Device::sidechain` into `ChainEntry::sidechain`; the engine side (ordering, tap,
//! PDC alignment) is `ether-core/src/sidechain.rs` and the devices read the signal in
//! `Node::process_sidechain`. Deleting the source track cuts the sidechain (`doc/mod.rs`).
//!
//! Validation (on top of the model's, which rejects routing cycles and the device's own
//! track with `InvalidArgument`):
//! - the source must be an audio/MIDI/group or return track (not master: it is always
//!   rendered last, so it can never feed a device);
//! - the device must have a sidechain input (`DeviceDescriptor::sidechain_inputs > 0`);
//! - pad-chain devices (inside a drum rack) can't take a sidechain.
//!
//! Clearing (`source: None`) is always allowed; setting the current value is a no-op.

use ether_core::protocol::model::{DeviceChange, DeviceId, TrackId, TrackKind};

use crate::doc::DocCtx;
use crate::tx::{CmdResult, invalid};

pub(crate) fn set_sidechain(
    ctx: &mut DocCtx,
    device: DeviceId,
    source: Option<TrackId>,
) -> CmdResult<()> {
    let d = ctx.device(device)?;
    if d.sidechain == source {
        return Ok(());
    }
    if let Some(src) = source {
        let track = ctx.track(src)?;
        if track.kind == TrackKind::Master {
            return Err(invalid("the master track cannot be a sidechain source"));
        }
        if d.pad.is_some() {
            return Err(invalid(format!(
                "device {} is in a drum-rack pad chain and cannot take a sidechain",
                d.id
            )));
        }
        let inputs = ctx
            .host
            .descriptor(d.id, &d.kind)
            .map_or(0, |desc| desc.sidechain_inputs);
        if inputs == 0 {
            return Err(invalid(format!(
                "device {} has no sidechain input",
                d.id
            )));
        }
    }
    ctx.set_device(d.id, DeviceChange::Sidechain(source))
}

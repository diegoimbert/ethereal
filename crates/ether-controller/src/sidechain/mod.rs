//! Sidechain routing (roadmap v2, owned by the `sidechain` node; see `docs/ROADMAP.md` and
//! CONTRACTS.md §11.10).
//!
//! `DeviceCommand::SetSidechain` is a document command ([`set_sidechain`], dispatched from
//! `doc/devices.rs`). `compile.rs` already copies `Device::sidechain` into
//! `ChainEntry::sidechain`; the engine side (ordering, PDC, `Node::process_sidechain`) is
//! in `ether-core` (`graph.rs`/`mixer.rs`, shared touches).

use ether_core::protocol::model::{DeviceId, TrackId};

use crate::doc::DocCtx;
use crate::tx::{CmdResult, unsupported};

pub(crate) fn set_sidechain(
    ctx: &mut DocCtx,
    device: DeviceId,
    source: Option<TrackId>,
) -> CmdResult<()> {
    let _ = (ctx, device, source);
    Err(unsupported(
        "sidechain is not implemented yet (sidechain node)",
    ))
}

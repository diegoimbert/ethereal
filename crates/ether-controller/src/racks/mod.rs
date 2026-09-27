//! Racks, macros and modulation (v0.2, owned by the `racks-modulation` node; model
//! `ether_model::{rack, modulation}`, protocol `ether_protocol::racks`, CONTRACTS.md §12.6).
//!
//! - [`rack_command`], [`modulation_command`]: document commands (dispatched from
//!   `doc::apply`). `Modulation::ListModulatorKinds` is answered in `handlers.rs`
//!   (`ether_devices::modulators::all`). Placeholder: `Unsupported`.
//! - [`chain_racks_desc`], [`modulation_desc`]: called by `compile.rs` per track
//!   (`TrackDesc::{chain_racks, modulation}`). Placeholder: empty. Rack-chain devices get
//!   engine nodes already (every document device does) but are not in any desc yet.
//! - Live modulator param drags: `ParamTarget::Modulator` through `EngineBridge::set_param`
//!   (add the fast path in `doc::devices`-style param handling of this module).
//! - Cascades are done (`doc/mod.rs`): deleting a device removes mappings targeting it or
//!   sourced from it (its modulators, its macros), its modulators, and a rack's chains with
//!   their devices.

use ether_core::modulation::ModulationDesc;
use ether_core::protocol::model::{Project, TrackId};
use ether_core::protocol::racks::{ModulationCommand, RackCommand};
use ether_core::rack_chains::ChainRackDesc;

use crate::compile::CompileContext;
use crate::doc::DocCtx;
use crate::tx::{CmdResult, unsupported};

pub(crate) fn rack_command(ctx: &mut DocCtx, command: &RackCommand) -> CmdResult<()> {
    let _ = (ctx, command);
    Err(unsupported(
        "racks are not implemented yet (racks-modulation)",
    ))
}

pub(crate) fn modulation_command(ctx: &mut DocCtx, command: &ModulationCommand) -> CmdResult<()> {
    let _ = (ctx, command);
    Err(unsupported(
        "modulation is not implemented yet (racks-modulation)",
    ))
}

/// `TrackDesc::chain_racks` of `track`.
pub(crate) fn chain_racks_desc(
    p: &Project,
    track: TrackId,
    ctx: &CompileContext,
) -> Vec<ChainRackDesc> {
    let _ = (p, track, ctx);
    Vec::new()
}

/// `TrackDesc::modulation` of `track`.
pub(crate) fn modulation_desc(p: &Project, track: TrackId, ctx: &CompileContext) -> ModulationDesc {
    let _ = (p, track, ctx);
    ModulationDesc::default()
}

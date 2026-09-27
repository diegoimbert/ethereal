//! Drum racks and sampler slicing (roadmap v2, owned by the `drum-rack` node; see
//! `docs/ROADMAP.md`, `ether_model::drum_rack` and `ether_protocol::drum_rack`).
//!
//! - `DrumRackCommand` / `SliceCommand` are document commands ([`rack_command`],
//!   [`slice_command`], from `doc::apply`). Deleting a rack must delete its pads and their
//!   devices first (`doc::DocCtx::delete_device` → a cascade hook here).
//! - [`racks_desc`] is called by `compile.rs` for every track: it compiles the track's
//!   racks into `TrackDesc::racks` (pad chains → node keys).

use ether_core::graph::RackDesc;
use ether_core::protocol::drum_rack::{DrumRackCommand, SliceCommand};
use ether_core::protocol::model::{Project, TrackId};

use crate::compile::CompileContext;
use crate::doc::DocCtx;
use crate::tx::{CmdResult, unsupported};

pub(crate) fn rack_command(ctx: &mut DocCtx, c: &DrumRackCommand) -> CmdResult<()> {
    let _ = (ctx, c);
    Err(unsupported("drum racks are not implemented yet (drum-rack node)"))
}

pub(crate) fn slice_command(ctx: &mut DocCtx, c: &SliceCommand) -> CmdResult<()> {
    let _ = (ctx, c);
    Err(unsupported("slicing is not implemented yet (drum-rack node)"))
}

/// Pad chains of the racks on `track`'s chain. Empty while unimplemented.
pub(crate) fn racks_desc(p: &Project, track: TrackId, ctx: &CompileContext) -> Vec<RackDesc> {
    let _ = (p, track, ctx);
    Vec::new()
}

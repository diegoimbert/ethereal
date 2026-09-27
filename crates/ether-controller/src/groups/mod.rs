//! Groups, buses, track input taps and VCAs (v0.2, owned by the `groups-buses` node,
//! priority 1; CONTRACTS.md §12.10).
//!
//! - [`track_command`]: `Track::{GroupSelected, Ungroup, SetVca}` (document commands,
//!   dispatched from `doc/tracks.rs`). Placeholder: `Unsupported`.
//! - [`vca_descs`]: `RenderGraphDesc::vcas` (`compile.rs` already keeps VCA tracks out of
//!   `tracks` and copies `Track::vca` / `TrackInput::Track` into `TrackDesc::{vca,
//!   input_tap}`). Placeholder: none. Engine: `ether_core::{vca, bus_tap}`.
//! - Solo rules (CONTRACTS.md §12.10) are compiled in `ether-core/src/graph.rs` (solo_ok);
//!   the controller folds VCA solo into `TrackDesc::solo` here.
//! - Deleting a VCA unassigns its tracks (`doc/mod.rs`, done).

use ether_core::protocol::model::Project;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::vca::VcaDesc;

use crate::compile::CompileContext;
use crate::doc::DocCtx;
use crate::tx::{CmdResult, unsupported};

pub(crate) fn track_command(ctx: &mut DocCtx, command: &TrackCommand) -> CmdResult<()> {
    let _ = (ctx, command);
    Err(unsupported(
        "grouping and VCAs are not implemented yet (groups-buses)",
    ))
}

/// `RenderGraphDesc::vcas`.
pub(crate) fn vca_descs(p: &Project, ctx: &CompileContext) -> Vec<VcaDesc> {
    let _ = (p, ctx);
    Vec::new()
}

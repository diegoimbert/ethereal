//! Takes and comping (v0.2, owned by the `comping` node; model `ether_model::take`,
//! protocol `ether_protocol::takes`, CONTRACTS.md §12.2).
//!
//! - [`take_command`]: every `TakeCommand` (document commands, dispatched from
//!   `doc::apply`; one undo step each; `SetComp` is the swipe gesture). Placeholder:
//!   `Unsupported`.
//! - [`comp_clips`]: called by `compile.rs` for every audio/MIDI track: the clip pieces
//!   the comp regions select from the take lanes (trimmed to each region, boundary
//!   crossfades as fades), merged with the main-lane clips. Take-lane clips are never
//!   compiled directly (`compile.rs` skips `Clip::lane.is_some()`). Placeholder: none.
//! - Recording (shared touch in `recording/`): loop/punch passes become take lanes + a comp
//!   region per pass.
//! - Cascades are done (`doc/mod.rs`): deleting a track removes its comp regions, lane
//!   clips and lanes.

use ether_core::graph::ClipDesc;
use ether_core::protocol::model::{Project, TrackId};
use ether_core::protocol::takes::TakeCommand;

use crate::compile::CompileContext;
use crate::doc::DocCtx;
use crate::tx::{CmdResult, unsupported};

pub(crate) fn take_command(ctx: &mut DocCtx, command: &TakeCommand) -> CmdResult<()> {
    let _ = (ctx, command);
    Err(unsupported(
        "takes and comping are not implemented yet (comping)",
    ))
}

/// Clip descs of `track`'s comp (see the module docs), sorted by start.
pub(crate) fn comp_clips(p: &Project, ctx: &CompileContext, track: TrackId) -> Vec<ClipDesc> {
    let _ = (p, ctx, track);
    Vec::new()
}

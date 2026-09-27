//! Clip editing: fade curves, reverse, crossfades and arrangement markers (roadmap v2,
//! owned by the `clip-editing` node; see `docs/ROADMAP.md`).
//!
//! Dispatched from `doc::apply` (`MarkerCommand`) and `doc/clips.rs` (the v2
//! `ClipCommand` variants). Overlap/crossfade rules: `ether_model::clip` module docs; the
//! existing overlap trimming (`doc/clips.rs`) must keep crossfade overlaps.

use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::markers::MarkerCommand;

use crate::doc::DocCtx;
use crate::tx::{CmdResult, unsupported};

/// `ClipCommand::{SetFadeCurves, SetReversed, Crossfade}`.
pub(crate) fn clip_command(ctx: &mut DocCtx, c: &ClipCommand) -> CmdResult<()> {
    let _ = (ctx, c);
    Err(unsupported("not implemented yet (clip-editing node)"))
}

pub(crate) fn marker_command(ctx: &mut DocCtx, c: &MarkerCommand) -> CmdResult<()> {
    let _ = (ctx, c);
    Err(unsupported("markers are not implemented yet (clip-editing node)"))
}

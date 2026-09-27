//! Groove: humanize, quantize swing and the project playback swing (roadmap v2, owned by
//! the `groove` node; see `docs/ROADMAP.md` and `ether_protocol::groove`).
//!
//! - `GrooveCommand` is a document command ([`apply`], from `doc::apply`).
//! - `NoteCommand::Quantize { swing }` is handled in `doc/notes.rs` (shared touch).
//! - [`swing_notes`] is called by `compile.rs` for every MIDI clip: it applies the project
//!   swing (`ProjectSettings::swing`/`swing_grid`) to the compiled notes.

use ether_core::graph::NoteDesc;
use ether_core::protocol::groove::GrooveCommand;
use ether_core::protocol::model::ProjectSettings;

use crate::doc::DocCtx;
use crate::tx::{CmdResult, unsupported};

pub(crate) fn apply(ctx: &mut DocCtx, c: &GrooveCommand) -> CmdResult<()> {
    let _ = (ctx, c);
    Err(unsupported("groove is not implemented yet (groove node)"))
}

/// Apply the project playback swing to a clip's notes (content-relative beats; `offset`
/// = the clip's content offset, so the grid is aligned to the arrangement when the clip
/// starts on a grid line). Must keep `notes` sorted by start. No-op while unimplemented.
pub(crate) fn swing_notes(settings: &ProjectSettings, offset: f64, notes: &mut [NoteDesc]) {
    let _ = (settings, offset, notes);
}

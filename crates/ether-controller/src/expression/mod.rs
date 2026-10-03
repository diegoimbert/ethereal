//! MIDI expression (v0.3, owned by the `midi-expression` node; model
//! `ether_model::expression`, protocol `ether_protocol::expression`, engine
//! `ether_core::expression`; CONTRACTS.md §13.2).
//!
//! - [`expression_command`]: every `ExpressionCommand` (document commands dispatched from
//!   `doc::apply`, one undo step each; `SetTrackMpe` is delegated to [`crate::mpe`]).
//! - [`track_expression`]: compile hook (`compile.rs`): a MIDI track's clip lanes and note
//!   expressions → `TrackDesc::expression` (note indices in the clip's sorted note list;
//!   `mpe` from `Track::mpe`).
//! - Recording (shared touch in `recording/` and the native writer): CC / pitch bend /
//!   channel and poly pressure played while recording become lanes and note expressions of
//!   the recorded clip (thinned to at most one point per 5 ms per curve, keeping extremes).
//! - Cascades are done (`doc/mod.rs`, `doc/notes.rs`): deleting a clip removes its lanes,
//!   deleting a note its expressions; `DocCtx::copy_clip` copies both. Other note copies
//!   (`Note::Duplicate`, split, consolidate, flatten, paste) are this node's to extend.
//!
//! Until the node lands: commands reply `Unsupported` (tests/roadmap_v4.rs) and nothing is
//! compiled (v0.2 behaviour).

use ether_core::expression::TrackExpressionDesc;
use ether_core::protocol::expression::ExpressionCommand;
use ether_core::protocol::model::{Project, Track};

use crate::doc::DocCtx;
use crate::tx::{CmdResult, unsupported};

/// Apply one `ExpressionCommand`.
pub(crate) fn expression_command(ctx: &mut DocCtx, c: &ExpressionCommand) -> CmdResult<()> {
    match c {
        ExpressionCommand::SetTrackMpe { track, mpe } => {
            crate::mpe::set_track_mpe(ctx, *track, *mpe)
        }
        other => Err(unsupported(format!(
            "{} is not implemented yet (midi-expression)",
            crate::doc::label_of(&ether_core::protocol::Command::Expression(other.clone()))
        ))),
    }
}

/// Compile hook: the expression of `track`'s compiled clips. Placeholder: empty.
pub(crate) fn track_expression(project: &Project, track: &Track) -> TrackExpressionDesc {
    let _ = (project, track);
    TrackExpressionDesc::default()
}

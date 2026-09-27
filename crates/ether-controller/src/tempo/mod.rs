//! Tempo map editing and metronome settings (roadmap v2, owned by the `tempo-metronome`
//! node; see `docs/ROADMAP.md` and `ether_protocol::tempo`).
//!
//! `TempoCommand` is a document command (dispatched from `doc::apply`). The click itself
//! is rendered by `ether_core::metronome`; [`metronome_desc`] compiles the settings into
//! the render graph.

use ether_core::graph::MetronomeDesc;
use ether_core::protocol::model::ProjectSettings;
use ether_core::protocol::tempo::TempoCommand;

use crate::doc::DocCtx;
use crate::tx::{CmdResult, unsupported};

pub(crate) fn apply(ctx: &mut DocCtx, c: &TempoCommand) -> CmdResult<()> {
    let _ = (ctx, c);
    Err(unsupported(
        "tempo map editing is not implemented yet (tempo-metronome node)",
    ))
}

/// Click settings for the render graph.
pub(crate) fn metronome_desc(s: &ProjectSettings) -> MetronomeDesc {
    MetronomeDesc {
        volume: s.metronome_volume.to_linear(),
        accent: s.metronome_accent,
        sound: s.metronome_sound,
    }
}

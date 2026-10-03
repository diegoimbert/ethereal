//! MPE (v0.3, owned by the `mpe` node; builds on `midi-expression`; CONTRACTS.md §13.3).
//!
//! - [`set_track_mpe`]: `Expression::SetTrackMpe` (document command, `TrackChange::Mpe`;
//!   MIDI tracks only).
//! - MPE input (shared touch in `recording/` and the native MIDI input path): on a track with
//!   `Track::mpe`, each member channel's note gets the channel's pitch bend (scaled by
//!   `note_pitch_range`), channel pressure and CC 74 as its `Pitch` / `Pressure` / `Timbre`
//!   note expressions when recording, and live (monitoring) as `EventKind::NoteExpression`.
//! - Output: the plugin hosts translate `EventKind::NoteExpression` (CLAP note expressions,
//!   VST3 note expression, else MPE MIDI with `TrackExpressionDesc::mpe`), and the Poly Synth
//!   responds to per-note pitch/pressure/timbre (shared touches in `ether-clap`, `ether-vst3`,
//!   `ether-au`, `ether-devices/src/poly_synth/`).
//!
//! Until the node lands: `SetTrackMpe` replies `Unsupported` (tests/roadmap_v4.rs).

use ether_core::protocol::model::{MpeSettings, TrackId};

use crate::doc::DocCtx;
use crate::tx::{CmdResult, unsupported};

/// `Expression::SetTrackMpe`.
pub(crate) fn set_track_mpe(
    ctx: &mut DocCtx,
    track: TrackId,
    mpe: Option<MpeSettings>,
) -> CmdResult<()> {
    let _ = (ctx, track, mpe);
    Err(unsupported("MPE is not implemented yet (mpe)"))
}

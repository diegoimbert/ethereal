//! MPE (v0.3, owned by the `mpe` node; builds on `midi-expression`; CONTRACTS.md §13.3).
//!
//! - [`set_track_mpe`]: `Expression::SetTrackMpe` (document command, `TrackChange::Mpe`;
//!   MIDI tracks only, settings checked by `ether_model::check_mpe`; idempotent).
//! - MPE input while recording ([`record`], called from `recording/`): on a track with
//!   `Track::mpe`, each member channel's note gets the channel's pitch bend (scaled by
//!   `note_pitch_range`), channel pressure and CC 74 as its `Pitch` / `Pressure` / `Timbre`
//!   note expressions; the master channel's messages become clip lanes as on any track.
//! - Live input (monitoring) is translated in the engine (`ether_core::expression::mpe`,
//!   `ExpressionRt::live_input`): member-channel messages reach the instrument as
//!   `EventKind::NoteExpression`.
//! - Output: the plugin hosts translate `EventKind::NoteExpression` (CLAP note expressions,
//!   VST3 note expression, else MPE MIDI through `ether_core::expression::mpe::MpeOut`), and
//!   the Poly Synth responds to per-note pitch/pressure/timbre.

pub(crate) mod record;

use ether_core::protocol::model::{MpeSettings, TrackChange, TrackId, TrackKind, check_mpe};

use crate::doc::DocCtx;
use crate::tx::{CmdResult, invalid};

/// `Expression::SetTrackMpe`.
pub(crate) fn set_track_mpe(
    ctx: &mut DocCtx,
    track: TrackId,
    mpe: Option<MpeSettings>,
) -> CmdResult<()> {
    let t = ctx.track(track)?;
    if t.kind != TrackKind::Midi {
        return Err(invalid("MPE is for MIDI tracks"));
    }
    if let Some(m) = &mpe {
        check_mpe(m).map_err(invalid)?;
    }
    if t.mpe == mpe {
        return Ok(());
    }
    ctx.set_track(track, TrackChange::Mpe(mpe))
}

//! Groove (roadmap v2, `groove` node): humanize and the project swing. Quantize itself
//! (with `strength` and `swing`) is `NoteCommand::Quantize`. All undoable.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{Beats, ClipId, NoteId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum GrooveCommand {
    /// Randomize note starts by up to ±`timing` beats and velocities by up to ±`velocity`
    /// (0..=1), deterministically from `seed` (same seed + same notes = same result, so
    /// retries are idempotent and the UI can preview). `notes: None` = all notes of the
    /// clip. Starts are clamped to `>= 0`, velocities to 0..=1.
    Humanize {
        clip: ClipId,
        notes: Option<Vec<NoteId>>,
        timing: Beats,
        velocity: f32,
        seed: u32,
    },
    /// Project playback swing (`ProjectSettings::swing`/`swing_grid`, see there).
    /// `amount` is clamped to 0..=1; `grid` must be `> 0`.
    SetSwing { amount: f32, grid: Beats },
}

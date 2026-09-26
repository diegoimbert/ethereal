//! MIDI notes.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{ClipId, NoteId};
use crate::value::Beats;

/// A note in a MIDI clip. Stored flat in `Project::notes` (normalized), keyed by id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Note {
    pub id: NoteId,
    pub clip: ClipId,
    /// MIDI key 0..=127.
    pub pitch: u8,
    /// 0.0..=1.0.
    pub velocity: f32,
    /// 0.0..=1.0.
    pub release_velocity: f32,
    /// Content-relative start.
    pub start: Beats,
    pub duration: Beats,
    pub muted: bool,
}

//! MIDI note editing (piano roll).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{Beats, ClipId, NoteId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum NoteCommand {
    Add {
        clip: ClipId,
        notes: Vec<NoteSpec>,
    },
    Remove {
        ids: Vec<NoteId>,
    },
    /// Partial edits; `None` fields are unchanged. Typical drag: many edits, one gesture.
    Edit {
        edits: Vec<NoteEdit>,
    },
    /// Quantize starts (and optionally ends) to `grid` with `strength` 0..=1.
    /// `notes: None` = all notes of the clip.
    Quantize {
        clip: ClipId,
        notes: Option<Vec<NoteId>>,
        grid: Beats,
        strength: f32,
        ends: bool,
    },
    /// Copy notes shifted by `offset` beats and `transpose` semitones, with new ids.
    Duplicate {
        copies: Vec<NoteCopy>,
        offset: Beats,
        transpose: i8,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct NoteSpec {
    pub id: NoteId,
    pub pitch: u8,
    pub velocity: f32,
    pub start: Beats,
    pub duration: Beats,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct NoteEdit {
    pub id: NoteId,
    pub pitch: Option<u8>,
    pub velocity: Option<f32>,
    pub start: Option<Beats>,
    pub duration: Option<Beats>,
    pub muted: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct NoteCopy {
    pub from: NoteId,
    pub new_id: NoteId,
}

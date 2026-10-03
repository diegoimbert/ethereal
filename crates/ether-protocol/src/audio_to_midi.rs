//! Audio to MIDI (v0.3, `audio-to-midi`; CONTRACTS.md §13.5): convert an audio clip to a new
//! MIDI track, offline, as a job with progress (like freeze/export).
//!
//! `Start` replies `Unit` at once; detection runs from the controller tick (bounded work per
//! tick, on the decoded media) and reports `AudioToMidiEvent::{Progress, Done, Failed,
//! Cancelled}`. The document changes once, at `Done`: a new MIDI track `track` right below
//! the clip's track, with the clip `clip` at the source clip's position and length, the
//! detected notes (`derive_id(seed_notes, i)` in (start, pitch) order) and, with
//! `instrument`, a default instrument with that device id (`PolySynth` for melody/harmony, an
//! empty `DrumRack` for drums). One undo step. One job at a time per
//! controller (another `Start` replies `InvalidState`).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{ClipId, DeviceId, NoteId, Seconds, TrackId};

/// Client-chosen job id (like `RenderJobId`).
pub type AudioToMidiJobId = String;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub enum AudioToMidiMode {
    /// Monophonic pitch tracking (vocals, bass, leads).
    #[default]
    Melody,
    /// Polyphonic pitches (piano, guitar chords; simple).
    Harmony,
    /// Onsets classified into kick / snare / hi-hat bands.
    Drums,
}

/// Detection settings (all have neutral defaults).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct AudioToMidiOptions {
    /// 0..=1: higher finds more (quieter) notes.
    pub sensitivity: f32,
    /// Notes shorter than this are dropped.
    pub min_duration: Seconds,
    /// Lowest/highest MIDI key kept (melody/harmony).
    pub min_pitch: u8,
    pub max_pitch: u8,
    /// Drum-mode keys (General MIDI defaults: 36, 38, 42).
    pub kick_key: u8,
    pub snare_key: u8,
    pub hihat_key: u8,
}

impl Default for AudioToMidiOptions {
    fn default() -> Self {
        Self {
            sensitivity: 0.5,
            min_duration: Seconds(0.05),
            min_pitch: 21,
            max_pitch: 108,
            kick_key: 36,
            snare_key: 38,
            hihat_key: 42,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum AudioToMidiCommand {
    /// Convert the audio clip `clip` (warped clips: detected on the source, placed through
    /// the clip's warp map).
    Start {
        job: AudioToMidiJobId,
        clip: ClipId,
        mode: AudioToMidiMode,
        options: AudioToMidiOptions,
        track: TrackId,
        new_clip: ClipId,
        seed_notes: NoteId,
        instrument: Option<DeviceId>,
    },
    Cancel {
        job: AudioToMidiJobId,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum AudioToMidiEvent {
    Progress {
        job: AudioToMidiJobId,
        /// 0..=1.
        progress: f32,
    },
    /// The document edit was applied (its patch precedes this event).
    Done {
        job: AudioToMidiJobId,
        track: TrackId,
        clip: ClipId,
        notes: u32,
    },
    Failed {
        job: AudioToMidiJobId,
        message: String,
    },
    Cancelled {
        job: AudioToMidiJobId,
    },
}

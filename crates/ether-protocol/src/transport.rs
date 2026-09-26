//! Transport: play/stop/locate, loop, tempo, metronome.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{BeatRange, Beats, Seconds, TimeSignature};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum TransportCommand {
    Play,
    /// Stop; a second `Stop` while stopped returns the playhead to the start position.
    Stop,
    TogglePlay,
    /// Move the playhead (while playing: jump, quantized to the next block).
    Locate {
        position: Beats,
    },
    /// Undoable (project setting).
    SetLoopEnabled {
        enabled: bool,
    },
    /// Undoable (project setting).
    SetLoopRegion {
        region: BeatRange,
    },
    /// Sets the tempo of the tempo point in effect at the playhead (undoable). With a single
    /// tempo point this is "the project tempo".
    SetTempo {
        bpm: f64,
    },
    /// Sets the time signature in effect at the playhead (undoable).
    SetTimeSignature {
        signature: TimeSignature,
    },
    SetMetronome {
        enabled: bool,
    },
    /// Tap tempo; the controller averages recent taps.
    TapTempo,
}

/// Emitted as `Event::Transport` whenever any field changes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct TransportState {
    pub playing: bool,
    pub recording: bool,
    pub loop_enabled: bool,
    pub loop_region: BeatRange,
    /// Tempo at the playhead.
    pub bpm: f64,
    pub time_signature: TimeSignature,
    pub metronome: bool,
    /// Where `Stop` returns to.
    pub start_position: Beats,
}

/// High-rate playhead stream (~30-60 Hz), delivered via `EngineTransport.subscribePlayhead`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PlayheadUpdate {
    pub position: Beats,
    pub seconds: Seconds,
    pub playing: bool,
    pub bpm: f64,
}

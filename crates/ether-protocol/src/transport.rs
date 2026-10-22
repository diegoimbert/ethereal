//! Transport: play/stop/locate, loop, tempo, metronome.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{BeatRange, Beats, Seconds, TimeSignature};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum TransportCommand {
    /// Play from where the playhead is (a stop leaves it where it stopped; `Locate` or a
    /// second `Stop` moves it).
    Play,
    /// Stop; a second `Stop` while stopped returns the playhead to the start position.
    Stop,
    TogglePlay,
    /// Move the playhead (while playing: jump, quantized to the next block). While stopped
    /// it also sets the start position (where a second `Stop` returns); while playing it
    /// doesn't.
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
    /// The open project is still loading (plugins instantiating, media decoding): the
    /// engine's audio is suspended, and play, stop and record are refused with
    /// `error.project.loading` until it is ready. The project can be explored and edited
    /// meanwhile; edits are heard once the load ends. Absent once loaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub loading: Option<LoadProgress>,
}

/// How far the open project's load is ([`TransportState::loading`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct LoadProgress {
    /// Plugin devices instantiated (or failed) of `plugins_total`.
    pub plugins_loaded: u32,
    pub plugins_total: u32,
    /// Media files decoded (or found missing) of `media_total`.
    pub media_loaded: u32,
    pub media_total: u32,
}

/// High-rate playhead stream (~30-60 Hz), delivered via `EngineTransport.subscribePlayhead`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PlayheadUpdate {
    pub position: Beats,
    pub seconds: Seconds,
    pub playing: bool,
    pub bpm: f64,
}

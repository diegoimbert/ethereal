//! Transport state seen by nodes, transport controls sent to the engine, playhead readback.

use ether_protocol::model::{BeatRange, Beats, TimeSignature};
use serde::{Deserialize, Serialize};

/// Musical/time position at the start of the current (sub-)block. Linear within the call
/// (the scheduler splits at tempo segments and loop points).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransportInfo {
    pub playing: bool,
    pub recording: bool,
    /// Engine-time sample counter since start (monotonic, never loops).
    pub sample_time: u64,
    /// Song position in beats at block start (loops with the loop region).
    pub position: f64,
    /// Song position in seconds at block start.
    pub seconds: f64,
    /// Tempo at block start and beats advanced per sample in this sub-block.
    pub bpm: f64,
    pub beats_per_sample: f64,
    pub time_signature: TimeSignature,
    /// Beat position of the start of the current bar.
    pub bar_start: f64,
    pub loop_active: bool,
    pub loop_start: f64,
    pub loop_end: f64,
}

impl TransportInfo {
    pub const STOPPED: Self = Self {
        playing: false,
        recording: false,
        sample_time: 0,
        position: 0.0,
        seconds: 0.0,
        bpm: 120.0,
        beats_per_sample: 0.0,
        time_signature: TimeSignature {
            numerator: 4,
            denominator: 4,
        },
        bar_start: 0.0,
        loop_active: false,
        loop_start: 0.0,
        loop_end: 0.0,
    };
}

/// Controls sent from the controller (`EngineHandle::transport`). Tempo, loop region and
/// metronome come from the published graph (they are document state); these are the
/// non-document transport actions.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum TransportControl {
    Play,
    Stop,
    /// Jump; applied at the next block boundary. Nodes get `AllNotesOff` + `reset`.
    Locate {
        position: Beats,
    },
    SetRecording {
        enabled: bool,
    },
    /// Override the loop without a republish (e.g. while dragging the loop brace).
    SetLoop {
        enabled: bool,
        region: BeatRange,
    },
}

/// Latest playhead, published by the audio thread once per block.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PlayheadState {
    pub playing: bool,
    pub recording: bool,
    pub position: Beats,
    pub seconds: f64,
    pub bpm: f64,
    pub sample_time: u64,
}

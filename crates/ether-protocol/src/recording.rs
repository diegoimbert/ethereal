//! Recording: arm, monitoring, inputs, record transport.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{ClipId, MonitorMode, TrackId, TrackInput};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum RecordingCommand {
    /// Record-arm (NOT undoable, not stored in the document; runtime state of the
    /// controller, reported via `RecordingEvent::ArmChanged`). `exclusive`: disarm others.
    Arm {
        track: TrackId,
        armed: bool,
        exclusive: bool,
    },
    SetMonitor {
        track: TrackId,
        monitor: MonitorMode,
    },
    SetInput {
        track: TrackId,
        input: TrackInput,
    },
    /// Global arrangement record button: starts playback + recording on armed tracks.
    SetRecording {
        enabled: bool,
    },
    /// Punch in/out: record only inside the loop region (runtime, not undoable; reported
    /// via `RecordingEvent::PunchChanged`).
    SetPunch {
        enabled: bool,
    },
    /// Undoable project setting.
    SetCountIn {
        bars: u32,
    },
    /// Replies `Inputs`.
    ListInputs,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct InputList {
    pub audio: Vec<AudioInputChannel>,
    pub midi: Vec<MidiPort>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct AudioInputChannel {
    pub index: u16,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MidiPort {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum RecordingEvent {
    Started {
        tracks: Vec<TrackId>,
    },
    /// Recorded clips are ordinary clips (already delivered via patches); this lists them.
    Stopped {
        clips: Vec<ClipId>,
    },
    InputsChanged {
        inputs: InputList,
    },
    /// The full set of currently armed tracks (sent on change and on connect).
    ArmChanged {
        armed: Vec<TrackId>,
    },
    PunchChanged {
        enabled: bool,
    },
    /// Live view of the takes being recorded (`live-record`): only what is new since the
    /// previous `Progress`, emitted by the controller while recording (~20 Hz). Runtime only:
    /// never part of the document or of collab; the real clips arrive with `Stopped`.
    Progress {
        audio: Vec<LiveAudioChunk>,
        midi: Vec<LiveMidiNote>,
    },
}

/// New waveform peaks of one audio take. Peaks are min/max over `frames_per_peak` input
/// frames, all channels merged; peak `i` of the take covers frames
/// `[i * frames_per_peak, (i + 1) * frames_per_peak)` from the take start.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct LiveAudioChunk {
    pub track: TrackId,
    /// Take number within this recording session (a new take starts on each loop wrap).
    pub take: u32,
    /// Timeline position (beats) of the take's first frame, latency-compensated exactly like
    /// the committed clip.
    pub start: f64,
    pub sample_rate: u32,
    pub frames_per_peak: u32,
    /// Index of `min[0]` / `max[0]` within the take.
    pub first_peak: u64,
    pub min: Vec<f32>,
    pub max: Vec<f32>,
}

/// A MIDI note played while recording, at its latency-compensated timeline position.
/// Sent when it starts (`length: None`) and again when it ends (`length: Some`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct LiveMidiNote {
    pub track: TrackId,
    pub pitch: u8,
    pub velocity: u8,
    pub start: f64,
    pub length: Option<f64>,
}

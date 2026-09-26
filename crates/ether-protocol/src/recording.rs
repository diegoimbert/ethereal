//! Recording: arm, monitoring, inputs, record transport.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{ClipId, MonitorMode, TrackId, TrackInput};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum RecordingCommand {
    /// `exclusive`: disarm all other tracks.
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
}

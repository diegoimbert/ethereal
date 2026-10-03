//! External hardware devices (v0.3, `external-instrument`; CONTRACTS.md §13.7): the External
//! Instrument and External Audio Effect built-ins (`BuiltinDevice::{ExternalInstrument,
//! ExternalAudioEffect}`, routing in `ether_model::external`).
//!
//! `SetRouting` is a document command (undoable). `ListPorts` and `MeasureLatency` are
//! runtime: hosts without hardware I/O (web) reply `Unsupported`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{DeviceId, ExternalRouting};
use crate::recording::{AudioInputChannel, MidiPort};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ExternalCommand {
    /// Replace an external device's routing (ports and channels). Undoable.
    SetRouting {
        device: DeviceId,
        routing: ExternalRouting,
    },
    /// Replies `HardwarePorts`.
    ListPorts,
    /// Measure the device's round trip: a click goes out (audio send, or a short note on the
    /// MIDI out) and its arrival on the return is timed. Replies `Unit` at once; the result
    /// arrives as `ExternalEvent::LatencyMeasured` (and sets the device's `LATENCY` param as
    /// one undoable edit) or `MeasureFailed` (nothing came back within 2 s).
    MeasureLatency { device: DeviceId },
}

/// Hardware the engine can reach (reply to `ListPorts`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct HardwarePorts {
    pub midi_outputs: Vec<MidiPort>,
    pub audio_inputs: Vec<AudioInputChannel>,
    /// Output channels of the audio device (same shape as inputs: index + name).
    pub audio_outputs: Vec<AudioInputChannel>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ExternalEvent {
    LatencyMeasured {
        device: DeviceId,
        latency_ms: f64,
    },
    MeasureFailed {
        device: DeviceId,
        message: String,
    },
    /// Hardware ports appeared or disappeared (devices whose ports resolved again resume).
    PortsChanged {
        ports: HardwarePorts,
    },
}

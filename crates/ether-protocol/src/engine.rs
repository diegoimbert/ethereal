//! Audio engine / device configuration and status.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum EngineCommand {
    /// Replies `AudioDevices`. `all`: every device the driver lists (ALSA lists each PCM
    /// of `aplay -L`; by default only the sound server and the hardware devices).
    ListAudioDevices {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        all: Option<bool>,
    },
    /// (Re)start the audio backend with this configuration. Replies `Status`.
    SetAudioConfig { config: AudioConfig },
    /// Replies `Status`.
    GetStatus,
}

/// Audio backend selection. `backend: "null"` (or env `ETHER_AUDIO=null`) runs the engine
/// on a timer thread with no device: used by tests, CI and parallel dev instances.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct AudioConfig {
    /// `"cpal"`, `"null"` (native) or `"webaudio"` (web). `None` = host default.
    pub backend: Option<String>,
    /// Backend-specific host API (CoreAudio, WASAPI, ASIO, ALSA, JACK, ...).
    pub host: Option<String>,
    pub output_device: Option<String>,
    pub input_device: Option<String>,
    pub sample_rate: Option<u32>,
    pub buffer_size: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct AudioDeviceList {
    pub backends: Vec<String>,
    pub hosts: Vec<String>,
    pub outputs: Vec<AudioDeviceInfo>,
    pub inputs: Vec<AudioDeviceInfo>,
    pub current: AudioConfig,
    /// Whether `buffer_size` is honoured by the current driver. `false`: the system sets
    /// the device buffer (WASAPI shared mode: one Windows period per callback), so the
    /// settings show the running size instead of offering a choice.
    pub buffer_size_settable: bool,
    /// Devices left out of `outputs` and `inputs` (listed with `all`).
    #[serde(default)]
    pub hidden_devices: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct AudioDeviceInfo {
    pub name: String,
    pub channels: u16,
    pub sample_rates: Vec<u32>,
    pub is_default: bool,
    /// Device buffer sizes (frames) a stream can ask for, `[min, max]`; `None` = unknown
    /// or unconstrained. Requests outside it are clamped to it.
    pub buffer_size_range: Option<[u32; 2]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct EngineStatus {
    pub running: bool,
    pub backend: String,
    pub sample_rate: u32,
    pub buffer_size: u32,
    /// Output latency in samples, as reported by the backend.
    pub output_latency: u32,
    pub input_latency: u32,
    /// Audio dropouts since start.
    pub xruns: u32,
    /// Dev instance id (`ETHER_INSTANCE`), for diagnostics.
    pub instance: String,
    /// The audio device is closed because the local engine is off ("Play with: no engine"
    /// or a participant's engine): other applications can use it, and
    /// it opens again (with the saved settings) when the engine comes back.
    #[serde(default)]
    pub device_released: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum EngineEvent {
    Status {
        status: EngineStatus,
    },
    Xrun {
        count: u32,
    },
    /// The engine's controller thread has been stuck on one step for `stalled_ms` (sent
    /// when that crosses the watchdog threshold, and again if `reason` changes). `reason`
    /// is a sentence fragment ("Analog Lab V is busy (opening its editor)"), `None` when
    /// unknown. Commands queue meanwhile; audio keeps playing. May arrive late: on desktop,
    /// events reach the UI through the main thread, which may be what is stuck.
    Unresponsive {
        stalled_ms: u32,
        reason: Option<String>,
    },
    /// The stall reported by `Unresponsive` is over; it lasted about `stalled_ms`.
    Responsive {
        stalled_ms: u32,
        reason: Option<String>,
    },
}

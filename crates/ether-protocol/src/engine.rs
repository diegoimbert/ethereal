//! Audio engine / device configuration and status.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum EngineCommand {
    /// Replies `AudioDevices`.
    ListAudioDevices,
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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct AudioDeviceInfo {
    pub name: String,
    pub channels: u16,
    pub sample_rates: Vec<u32>,
    pub is_default: bool,
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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum EngineEvent {
    Status { status: EngineStatus },
    Xrun { count: u32 },
}

//! Level meters (high-rate stream, delivered via `EngineTransport.subscribeMeters`).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::TrackId;

/// One frame of meter readings (~30 Hz). Only tracks whose level changed noticeably need to
/// be included; missing tracks keep their last value and decay in the UI.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MeterFrame {
    pub tracks: Vec<TrackMeter>,
    /// Engine DSP load 0..=1 (time spent / block duration).
    pub cpu_load: f32,
}

/// Post-fader levels as *linear* amplitude (1.0 = 0 dBFS), max-held since the last frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct TrackMeter {
    pub track: TrackId,
    /// `[left, right]` (mono tracks duplicate).
    pub peak: [f32; 2],
    pub rms: [f32; 2],
    /// Sample >= 1.0 since the last frame.
    pub clipped: bool,
}

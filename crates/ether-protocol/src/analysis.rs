//! Device → UI analysis channel (v0.2, frozen by contracts-3; CONTRACTS.md §12.4.3).
//!
//! Devices such as the spectrum analyzer and tuner (`fx-analysis`), gain-reduction meters
//! (`fx-dynamics`) and the modulated-value readback of `racks-modulation` push small frames
//! from the audio thread (bounded, throttled, allocation-free: `ether_core::analysis`). The
//! controller converts them to `Event::Analysis` **only for watched devices**: the UI sends
//! `Watch` when a device panel becomes visible and `Unwatch` when it goes away (watches are
//! per connection and dropped on disconnect/project change). Rate: at most
//! `ANALYSIS_MAX_HZ` frames per second per device and kind; frames are latest-wins (the UI
//! must not expect every frame).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{DeviceId, ParamId};

/// Upper bound on frames per second per (device, kind).
pub const ANALYSIS_MAX_HZ: u32 = 30;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum AnalysisCommand {
    /// Start receiving `Event::Analysis` for `device` (idempotent).
    Watch { device: DeviceId },
    /// Stop (idempotent).
    Unwatch { device: DeviceId },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum AnalysisEvent {
    Frame {
        device: DeviceId,
        data: AnalysisData,
    },
}

/// Payload kinds (append-only).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum AnalysisData {
    /// Magnitude spectrum: `bins_db[i]` at frequency `min_hz · (max_hz/min_hz)^(i/(n-1))`
    /// (log-spaced), in dBFS, smoothed by the device. At most 256 bins.
    Spectrum {
        min_hz: f32,
        max_hz: f32,
        bins_db: Vec<f32>,
        /// v0.2 (`graphical-eq`): input (`Pre`) or output (`Post`, the default) of the
        /// device. Analyzers send `Post`.
        #[serde(default)]
        stage: SpectrumStage,
    },
    /// Pitch detection. `hz: None` = no stable pitch (silence/noise).
    Tuner {
        hz: Option<f32>,
        /// Nearest MIDI note and the deviation from it in cents (-50..=50), relative to the
        /// device's reference pitch.
        note: Option<u8>,
        cents: f32,
        /// 0..=1.
        confidence: f32,
        level_db: f32,
    },
    /// Generic meters (e.g. per-band gain reduction in dB, gate state). Meaning is defined by
    /// the device's layout widget that shows them.
    Levels { values: Vec<f32> },
    /// Modulated param values of the device (`racks-modulation` readback for depth rings):
    /// normalized `base` and `effective` values of every modulated param.
    Modulation { values: Vec<ModulatedValue> },
}

/// Which side of the device a spectrum was measured on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum SpectrumStage {
    Pre,
    #[default]
    Post,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ModulatedValue {
    pub param: ParamId,
    /// Normalized base (automation or document value).
    pub base: f32,
    /// Normalized effective value after modulation (clamped).
    pub value: f32,
}

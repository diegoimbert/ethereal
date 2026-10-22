//! Warping of audio clips.
//!
//! When `WarpSettings::enabled`, the clip's content timeline (beats) is mapped to source
//! time (seconds) piecewise-linearly through its warp markers, and the audio is
//! time-stretched to follow the project tempo. With fewer than two markers the mapping is
//! derived from `source_bpm`. When disabled, the clip plays at the source's native speed:
//! content beats are converted to seconds at the tempo in effect at the clip start.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{ClipId, WarpMarkerId};
use crate::value::{Beats, Seconds};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct WarpSettings {
    pub enabled: bool,
    pub mode: WarpMode,
    /// Detected or user-set tempo of the source material.
    pub source_bpm: Option<f64>,
    /// Formant handling of [`WarpMode::ComplexPlus`] (the other modes ignore it). Written
    /// only when not the defaults, which documents without it read.
    #[serde(default, skip_serializing_if = "WarpFormants::is_default")]
    #[ts(as = "Option<WarpFormants>", optional)]
    pub formants: WarpFormants,
}

impl Default for WarpSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: WarpMode::Complex,
            source_bpm: None,
            formants: WarpFormants::DEFAULT,
        }
    }
}

/// Serialized: append new modes at the end (never reorder or rename).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum WarpMode {
    /// Resample (tempo changes pitch).
    Repitch,
    /// Signalsmith Stretch.
    #[default]
    Complex,
    /// Complex+: ethereal's multi-resolution stretcher (transients kept, formants preserved
    /// when transposing; `docs/WARP.md`). Live's Complex Pro imports as this.
    ComplexPlus,
}

impl WarpMode {
    /// Keeps the pitch (a stretcher plays it; transpose is a pitch shift, not a speed
    /// change).
    pub fn stretches(self) -> bool {
        !matches!(self, Self::Repitch)
    }
}

/// Complex+ formant controls (Live's Complex Pro "Formants" and "Envelope", plus a shift).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(default)]
pub struct WarpFormants {
    /// How much of the spectral envelope stays in place when transposing, `0..=1` (shown as
    /// 0–100 %): 1 keeps voices natural, 0 lets the formants move with the pitch.
    pub amount: f32,
    /// Formant shift (semitones, [`WarpFormants::SHIFT_MAX`] either way), independent of the
    /// transposition.
    pub shift: f32,
    /// Envelope detail, [`WarpFormants::ENVELOPE_MIN`]..=[`WarpFormants::ENVELOPE_MAX`]
    /// (Live's scale): lower for high voices, higher for low ones.
    pub envelope: f32,
}

impl WarpFormants {
    pub const SHIFT_MAX: f32 = 24.0;
    pub const ENVELOPE_MIN: f32 = 8.0;
    pub const ENVELOPE_MAX: f32 = 256.0;
    /// Formants kept, unshifted, Live's default envelope.
    pub const DEFAULT: Self = Self {
        amount: 1.0,
        shift: 0.0,
        envelope: 128.0,
    };

    pub fn is_default(&self) -> bool {
        *self == Self::DEFAULT
    }

    /// The settings with every value in range (`None` if one isn't a number).
    pub fn clamped(self) -> Option<Self> {
        let ok = self.amount.is_finite() && self.shift.is_finite() && self.envelope.is_finite();
        ok.then(|| Self {
            amount: self.amount.clamp(0.0, 1.0),
            shift: self.shift.clamp(-Self::SHIFT_MAX, Self::SHIFT_MAX),
            envelope: self.envelope.clamp(Self::ENVELOPE_MIN, Self::ENVELOPE_MAX),
        })
    }
}

impl Default for WarpFormants {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Pins content beat `beat` of clip `clip` to source time `source`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct WarpMarker {
    pub id: WarpMarkerId,
    pub clip: ClipId,
    pub beat: Beats,
    pub source: Seconds,
}

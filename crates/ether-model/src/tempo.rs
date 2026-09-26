//! Tempo and time-signature changes.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{TempoPointId, TimeSignatureId};
use crate::value::{Beats, Seconds, TimeSignature};

/// A tempo change at `time`. There is always a point at beat 0.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct TempoPoint {
    pub id: TempoPointId,
    pub time: Beats,
    pub bpm: f64,
    /// Shape of the segment from this point to the next.
    pub curve: TempoCurve,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum TempoCurve {
    /// Constant until the next point.
    #[default]
    Step,
    /// Linear ramp in BPM (over beats) to the next point.
    Linear,
}

/// A time-signature change. `time` must fall on a bar line of the previous signature.
/// There is always one at beat 0.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct TimeSignaturePoint {
    pub id: TimeSignatureId,
    pub time: Beats,
    pub signature: TimeSignature,
}

/// Sorted, read-only view of the tempo/time-signature points with conversions. Built from a
/// project by `Project::tempo_map()`. (The engine compiles its own RT-friendly tempo map in
/// `ether-core`; both must agree, and share test vectors.)
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct TempoMap {
    pub tempo: Vec<TempoPoint>,
    pub signatures: Vec<TimeSignaturePoint>,
}

/// Bar/beat position for display (1-based like every DAW: bar 1 beat 1 = beat 0).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct BarBeat {
    pub bar: i32,
    pub beat: u32,
    /// Fraction of the beat, 0..1.
    pub fraction: f64,
}

impl TempoMap {
    pub fn beats_to_seconds(&self, beats: Beats) -> Seconds {
        let _ = beats;
        todo!("model node")
    }

    pub fn seconds_to_beats(&self, seconds: Seconds) -> Beats {
        let _ = seconds;
        todo!("model node")
    }

    pub fn bpm_at(&self, beats: Beats) -> f64 {
        let _ = beats;
        todo!("model node")
    }

    pub fn signature_at(&self, beats: Beats) -> TimeSignature {
        let _ = beats;
        todo!("model node")
    }

    pub fn bar_beat(&self, beats: Beats) -> BarBeat {
        let _ = beats;
        todo!("model node")
    }
}

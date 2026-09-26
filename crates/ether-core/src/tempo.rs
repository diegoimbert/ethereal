//! RT-friendly compiled tempo map (piecewise constant / linear-ramp segments).
//!
//! Built off-thread from the document's tempo + time-signature points; queried on the audio
//! thread with O(log n) lookups and no allocation. Must agree with
//! `ether_model::TempoMap` (shared test vectors).

use ether_protocol::model::{TempoCurve, TimeSignature};
use serde::{Deserialize, Serialize};

/// One tempo point as given to the compiler.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TempoPointDesc {
    pub beat: f64,
    pub bpm: f64,
    pub curve: TempoCurve,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimeSignatureDesc {
    pub beat: f64,
    pub signature: TimeSignature,
}

/// Compiled tempo map. Internals are owned by the `core` node.
#[derive(Clone, Debug, Default)]
pub struct TempoMapRt {
    _private: (),
}

impl TempoMapRt {
    /// Non-RT. `points` must contain one at beat 0 (the compiler inserts 120 BPM if not).
    pub fn compile(points: &[TempoPointDesc], signatures: &[TimeSignatureDesc]) -> Self {
        let _ = (points, signatures);
        todo!("core node")
    }

    pub fn beats_to_seconds(&self, beats: f64) -> f64 {
        let _ = beats;
        todo!("core node")
    }

    pub fn seconds_to_beats(&self, seconds: f64) -> f64 {
        let _ = seconds;
        todo!("core node")
    }

    pub fn bpm_at(&self, beats: f64) -> f64 {
        let _ = beats;
        todo!("core node")
    }

    /// Beat of the next tempo-segment boundary strictly after `beats` (block splitting).
    pub fn next_boundary(&self, beats: f64) -> Option<f64> {
        let _ = beats;
        todo!("core node")
    }

    pub fn signature_at(&self, beats: f64) -> (TimeSignature, f64) {
        let _ = beats;
        todo!("core node: returns (signature, beat of the current bar start)")
    }
}

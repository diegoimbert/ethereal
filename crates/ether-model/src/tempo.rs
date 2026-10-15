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

/// A time-signature change, anywhere on the timeline. A change inside a bar ends that bar
/// early (a partial bar); a new bar in the new signature starts at the change (see
/// `TempoMap::bar_beat`).
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

/// Tempo of an empty tempo map.
pub const DEFAULT_BPM: f64 = 120.0;
/// A segment whose start and end BPM differ by at most this is treated as constant.
pub const RAMP_EPSILON_BPM: f64 = 1e-9;

/// Seconds per beat at `bpm`.
fn spb(bpm: f64) -> f64 {
    60.0 / bpm
}

// ---------------------------------------------------------------------------------------
// Per-segment tempo math: the single source of truth, shared with `ether-core`'s RT tempo
// map and mirrored by the UI. Pure, allocation-free, no panics.
//
// A segment starts at a tempo point with `start_bpm`. For a `Linear` point followed by
// another point, it ramps BPM linearly *over beats* to `end_bpm` across `length` beats.
// For a `Step` point, or the last point, pass `end_bpm == start_bpm` (constant tempo; any
// `length`, including `f64::INFINITY`). Offsets are measured from the segment start and
// are only meaningful within `[0, length]`.
//
// Ramp: bpm(x) = b0 + (b1 - b0)·x/L, seconds(x) = ∫ 60/bpm = 60·L/(b1-b0)·ln(bpm(x)/b0).
// Test vectors: crates/ether-model/tests/tempo_vectors.json.
// ---------------------------------------------------------------------------------------

/// `true` if the segment ramps (otherwise it is constant at `start_bpm`).
pub fn segment_is_ramp(start_bpm: f64, end_bpm: f64, length: f64) -> bool {
    length.is_finite() && length > Beats::EPSILON && (end_bpm - start_bpm).abs() > RAMP_EPSILON_BPM
}

/// Seconds elapsed `beats` into a segment.
pub fn segment_beats_to_seconds(start_bpm: f64, end_bpm: f64, length: f64, beats: f64) -> f64 {
    if segment_is_ramp(start_bpm, end_bpm, length) {
        let slope = (end_bpm - start_bpm) / length; // bpm per beat
        60.0 / slope * (beats * slope / start_bpm).ln_1p()
    } else {
        beats * spb(start_bpm)
    }
}

/// Beats elapsed `seconds` into a segment (inverse of [`segment_beats_to_seconds`]).
pub fn segment_seconds_to_beats(start_bpm: f64, end_bpm: f64, length: f64, seconds: f64) -> f64 {
    if segment_is_ramp(start_bpm, end_bpm, length) {
        let slope = (end_bpm - start_bpm) / length;
        start_bpm / slope * (seconds * slope / 60.0).exp_m1()
    } else {
        seconds / spb(start_bpm)
    }
}

/// Instantaneous BPM `beats` into a segment (clamped to `[0, length]` for ramps).
pub fn segment_bpm_at(start_bpm: f64, end_bpm: f64, length: f64, beats: f64) -> f64 {
    if segment_is_ramp(start_bpm, end_bpm, length) {
        start_bpm + (end_bpm - start_bpm) * beats.clamp(0.0, length) / length
    } else {
        start_bpm
    }
}

/// Semantics (shared with `ether-core`'s RT tempo map; see the per-segment functions above):
/// - seconds are measured from beat 0;
/// - a `Step` segment is constant; a `Linear` segment ramps to the next point's BPM;
/// - before the first point and after the last one the tempo is constant;
/// - an empty map means 120 BPM, 4/4.
impl TempoMap {
    /// Index of the tempo segment containing `beats` (`None` if the map is empty).
    fn segment(&self, beats: Beats) -> Option<usize> {
        if self.tempo.is_empty() {
            return None;
        }
        let i = self
            .tempo
            .partition_point(|p| p.time.0 <= beats.0 + Beats::EPSILON);
        Some(i.saturating_sub(1))
    }

    /// `(start_bpm, end_bpm, length)` of segment `i`, in the per-segment functions' terms.
    fn segment_params(&self, i: usize) -> (f64, f64, f64) {
        let p = &self.tempo[i];
        match self.tempo.get(i + 1) {
            Some(next) if p.curve == TempoCurve::Linear => {
                (p.bpm, next.bpm, next.time.0 - p.time.0)
            }
            Some(next) => (p.bpm, p.bpm, next.time.0 - p.time.0),
            None => (p.bpm, p.bpm, f64::INFINITY),
        }
    }

    /// Seconds from segment `i`'s start to `d` beats into it.
    fn seconds_in_segment(&self, i: usize, d: f64) -> f64 {
        let (b0, b1, len) = self.segment_params(i);
        segment_beats_to_seconds(b0, b1, len, d)
    }

    /// Beats from segment `i`'s start after `s` seconds into it.
    fn beats_in_segment(&self, i: usize, s: f64) -> f64 {
        let (b0, b1, len) = self.segment_params(i);
        segment_seconds_to_beats(b0, b1, len, s)
    }

    /// Seconds from the first tempo point to `beats` (negative before it).
    fn seconds_from_first(&self, beats: f64) -> f64 {
        let first = &self.tempo[0];
        if beats <= first.time.0 {
            return (beats - first.time.0) * spb(first.bpm);
        }
        let mut acc = 0.0;
        for i in 0..self.tempo.len() {
            let start = self.tempo[i].time.0;
            let end = self.tempo.get(i + 1).map_or(f64::INFINITY, |n| n.time.0);
            if beats <= end {
                return acc + self.seconds_in_segment(i, beats - start);
            }
            acc += self.seconds_in_segment(i, end - start);
        }
        acc
    }

    pub fn beats_to_seconds(&self, beats: Beats) -> Seconds {
        if self.tempo.is_empty() {
            return Seconds(beats.0 * spb(DEFAULT_BPM));
        }
        Seconds(self.seconds_from_first(beats.0) - self.seconds_from_first(0.0))
    }

    /// Inverse of [`Self::beats_to_seconds`].
    pub fn seconds_to_beats(&self, seconds: Seconds) -> Beats {
        if self.tempo.is_empty() {
            return Beats(seconds.0 / spb(DEFAULT_BPM));
        }
        let first = &self.tempo[0];
        let abs = seconds.0 + self.seconds_from_first(0.0);
        if abs <= 0.0 {
            return Beats(first.time.0 + abs / spb(first.bpm));
        }
        let mut acc = 0.0;
        for i in 0..self.tempo.len() {
            let start = self.tempo[i].time.0;
            let seg = self.tempo.get(i + 1).map_or(f64::INFINITY, |n| {
                self.seconds_in_segment(i, n.time.0 - start)
            });
            if abs <= acc + seg {
                return Beats(start + self.beats_in_segment(i, abs - acc));
            }
            acc += seg;
        }
        unreachable!("the last tempo segment is unbounded")
    }

    /// Instantaneous tempo at `beats` (interpolated inside linear segments).
    pub fn bpm_at(&self, beats: Beats) -> f64 {
        let Some(i) = self.segment(beats) else {
            return DEFAULT_BPM;
        };
        let (b0, b1, len) = self.segment_params(i);
        segment_bpm_at(b0, b1, len, beats.0 - self.tempo[i].time.0)
    }

    /// Signature in effect at `beats` (the first one before the first change).
    pub fn signature_at(&self, beats: Beats) -> TimeSignature {
        let i = self
            .signatures
            .partition_point(|p| p.time.0 <= beats.0 + Beats::EPSILON);
        self.signatures
            .get(i.saturating_sub(1))
            .map_or_else(TimeSignature::default, |p| p.signature)
    }

    /// Bar/beat display position. Bar 1 beat 1 = beat 0; beats count in denominator units
    /// (6/8 has 6 eighth-note beats per bar). A signature change that is not on a bar line
    /// ends the previous bar early (the partial bar counts as one bar). Negative positions
    /// extrapolate the first signature (bar 0, -1, ...).
    pub fn bar_beat(&self, beats: Beats) -> BarBeat {
        let per_bar = |s: TimeSignature| f64::from(s.numerator) * 4.0 / f64::from(s.denominator);
        let mut sig = self
            .signatures
            .first()
            .map_or_else(TimeSignature::default, |p| p.signature);
        let mut bars_before: i64 = 0;
        let mut seg_start = 0.0;
        for next in self.signatures.iter().skip(1) {
            if next.time.0 <= seg_start + Beats::EPSILON {
                sig = next.signature;
                continue;
            }
            if beats.0 + Beats::EPSILON < next.time.0 {
                break;
            }
            let bpb = per_bar(sig);
            bars_before +=
                (Beats(next.time.0 - seg_start).ceil_to(Beats(bpb)).0 / bpb).round() as i64;
            seg_start = next.time.0;
            sig = next.signature;
        }
        let bpb = per_bar(sig);
        let unit = 4.0 / f64::from(sig.denominator);
        let rel = beats.0 - seg_start;
        let n = ((rel + Beats::EPSILON) / bpb).floor();
        let rem = (rel - n * bpb).max(0.0);
        let idx = ((rem + Beats::EPSILON) / unit)
            .floor()
            .min(f64::from(sig.numerator.max(1) - 1));
        let fraction = ((rem - idx * unit) / unit).clamp(0.0, 1.0);
        BarBeat {
            bar: (bars_before + n as i64 + 1) as i32,
            beat: idx as u32 + 1,
            fraction: if fraction < 1e-6 || fraction >= 1.0 {
                0.0
            } else {
                fraction
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{TempoPointId, TimeSignatureId};

    fn tp(time: f64, bpm: f64, curve: TempoCurve) -> TempoPoint {
        TempoPoint {
            id: TempoPointId::NIL,
            time: Beats(time),
            bpm,
            curve,
        }
    }

    fn ts(time: f64, n: u8, d: u8) -> TimeSignaturePoint {
        TimeSignaturePoint {
            id: TimeSignatureId::NIL,
            time: Beats(time),
            signature: TimeSignature {
                numerator: n,
                denominator: d,
            },
        }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn constant_and_step_tempo() {
        let m = TempoMap {
            tempo: vec![
                tp(0.0, 120.0, TempoCurve::Step),
                tp(8.0, 60.0, TempoCurve::Step),
            ],
            signatures: vec![],
        };
        assert!(close(m.beats_to_seconds(Beats(4.0)).0, 2.0));
        assert!(close(m.beats_to_seconds(Beats(8.0)).0, 4.0));
        assert!(close(m.beats_to_seconds(Beats(10.0)).0, 6.0));
        assert!(close(m.beats_to_seconds(Beats(-2.0)).0, -1.0));
        assert!(close(m.seconds_to_beats(Seconds(6.0)).0, 10.0));
        assert!(close(m.seconds_to_beats(Seconds(-1.0)).0, -2.0));
        assert_eq!(m.bpm_at(Beats(7.9)), 120.0);
        assert_eq!(m.bpm_at(Beats(8.0)), 60.0);
    }

    #[test]
    fn linear_ramp_roundtrips() {
        let m = TempoMap {
            tempo: vec![
                tp(0.0, 60.0, TempoCurve::Linear),
                tp(4.0, 120.0, TempoCurve::Step),
            ],
            signatures: vec![],
        };
        assert!(close(m.bpm_at(Beats(2.0)), 90.0));
        // ∫0..4 60/(60+15x) dx = 4 ln 2.
        assert!(close(m.beats_to_seconds(Beats(4.0)).0, 4.0 * 2f64.ln()));
        assert!(close(
            m.beats_to_seconds(Beats(6.0)).0,
            4.0 * 2f64.ln() + 1.0
        ));
        for b in [-1.0, 0.0, 0.5, 2.0, 3.99, 4.0, 7.25, 100.0] {
            let s = m.beats_to_seconds(Beats(b));
            assert!(close(m.seconds_to_beats(s).0, b), "{b}");
        }
    }

    #[test]
    fn empty_map_defaults_to_120() {
        let m = TempoMap {
            tempo: vec![],
            signatures: vec![],
        };
        assert_eq!(m.beats_to_seconds(Beats(2.0)), Seconds(1.0));
        assert_eq!(m.seconds_to_beats(Seconds(1.0)), Beats(2.0));
        assert_eq!(m.signature_at(Beats(3.0)), TimeSignature::default());
        assert_eq!(m.bpm_at(Beats(3.0)), 120.0);
    }

    #[test]
    fn bar_beat_display() {
        let m = TempoMap {
            tempo: vec![],
            signatures: vec![ts(0.0, 4, 4), ts(8.0, 6, 8), ts(12.0, 3, 4)],
        };
        let bb = |b: f64| {
            let r = m.bar_beat(Beats(b));
            (r.bar, r.beat, r.fraction)
        };
        assert_eq!(bb(0.0), (1, 1, 0.0));
        assert_eq!(bb(5.0), (2, 2, 0.0));
        assert_eq!(bb(5.5), (2, 2, 0.5));
        assert_eq!(bb(3.999_999_9), (2, 1, 0.0));
        // 6/8 from beat 8 (bar 3): eighth-note beats, 3 quarter notes per bar.
        assert_eq!(bb(8.0), (3, 1, 0.0));
        assert_eq!(bb(8.5), (3, 2, 0.0));
        assert_eq!(bb(11.0), (4, 1, 0.0));
        // 3/4 from beat 12 (bar 5).
        assert_eq!(bb(12.0), (5, 1, 0.0));
        assert_eq!(bb(16.0), (6, 2, 0.0));
        assert_eq!(bb(-4.0), (0, 1, 0.0));
        assert_eq!(m.signature_at(Beats(9.0)).numerator, 6);
        assert_eq!(m.signature_at(Beats(1.0)).numerator, 4);
    }
}

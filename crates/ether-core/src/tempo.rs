//! RT-friendly compiled tempo map (piecewise constant / linear-ramp segments).
//!
//! Built off-thread from the document's tempo + time-signature points; queried on the audio
//! thread with O(log n) lookups and no allocation. Must agree with
//! `ether_model::TempoMap` (shared test vectors).
//!
//! # Math
//! Within a segment starting at beat `B0` (seconds `S0`, tempo `T0` BPM) the tempo is
//! `T(b) = T0 + k·(b − B0)` (`k = 0` for `Step`, `k = ΔT/Δbeats` for a `Linear` ramp towards
//! the next point: a linear ramp in BPM *over beats*). Time is `dt/db = 60 / T(b)`, so
//! - `k = 0`: `t = S0 + 60·(b − B0)/T0`
//! - `k ≠ 0`: `t = S0 + (60/k)·ln(T(b)/T0)`, inverted as `T = T0·exp(k·(t − S0)/60)`,
//!   `b = B0 + (T − T0)/k`.
//!
//! Before the first point the first tempo extends backwards (negative beats); the last
//! point's tempo holds forever (a `Linear` curve on the last point is treated as `Step`).

use ether_protocol::model::{TempoCurve, TimeSignature};
use serde::{Deserialize, Serialize};

/// Default tempo when the desc has no tempo points.
pub const DEFAULT_BPM: f64 = 120.0;
/// Tempos are clamped to this range to keep the math finite.
pub const MIN_BPM: f64 = 1.0;
pub const MAX_BPM: f64 = 10_000.0;

/// Tempo differences below this are treated as constant tempo.
const RAMP_EPSILON_BPM: f64 = 1e-9;
/// Boundary comparisons tolerance (beats), matches `Beats::EPSILON`.
const BEAT_EPS: f64 = 1e-6;

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

// Per-segment math. Offsets are relative to the segment start; `end_bpm == start_bpm` for
// constant segments (`length` may be infinite). Same signatures as
// `ether_model::tempo::segment_*`, which become the single source of truth once available.

#[inline]
fn segment_is_ramp(start_bpm: f64, end_bpm: f64, length: f64) -> bool {
    (end_bpm - start_bpm).abs() > RAMP_EPSILON_BPM && length.is_finite() && length > 0.0
}

#[inline]
fn segment_bpm_at(start_bpm: f64, end_bpm: f64, length: f64, beats: f64) -> f64 {
    if segment_is_ramp(start_bpm, end_bpm, length) {
        start_bpm + (end_bpm - start_bpm) * beats / length
    } else {
        start_bpm
    }
}

#[inline]
fn segment_beats_to_seconds(start_bpm: f64, end_bpm: f64, length: f64, beats: f64) -> f64 {
    if segment_is_ramp(start_bpm, end_bpm, length) {
        let k = (end_bpm - start_bpm) / length;
        60.0 / k * (segment_bpm_at(start_bpm, end_bpm, length, beats) / start_bpm).ln()
    } else {
        60.0 * beats / start_bpm
    }
}

#[inline]
fn segment_seconds_to_beats(start_bpm: f64, end_bpm: f64, length: f64, seconds: f64) -> f64 {
    if segment_is_ramp(start_bpm, end_bpm, length) {
        let k = (end_bpm - start_bpm) / length;
        let t = start_bpm * (k * seconds / 60.0).exp();
        (t - start_bpm) / k
    } else {
        seconds * start_bpm / 60.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Segment {
    beat: f64,
    seconds: f64,
    bpm: f64,
    /// Tempo reached at `beat + length` (== `bpm` for constant segments).
    end_bpm: f64,
    /// Beats until the next point (infinite for the last one).
    length: f64,
}

impl Segment {
    #[inline]
    fn bpm_at(&self, beats: f64) -> f64 {
        segment_bpm_at(self.bpm, self.end_bpm, self.length, beats - self.beat)
    }

    #[inline]
    fn seconds_at(&self, beats: f64) -> f64 {
        self.seconds + segment_beats_to_seconds(self.bpm, self.end_bpm, self.length, beats - self.beat)
    }

    #[inline]
    fn beats_at(&self, seconds: f64) -> f64 {
        self.beat
            + segment_seconds_to_beats(self.bpm, self.end_bpm, self.length, seconds - self.seconds)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct SigSegment {
    beat: f64,
    signature: TimeSignature,
    /// Bar length in quarter-note beats.
    bar: f64,
}

/// Compiled tempo map.
#[derive(Clone, Debug, PartialEq)]
pub struct TempoMapRt {
    segments: Vec<Segment>,
    signatures: Vec<SigSegment>,
    /// Sorted beats where either tempo segment or signature changes (excluding the first).
    boundaries: Vec<f64>,
}

impl Default for TempoMapRt {
    fn default() -> Self {
        Self::compile(&[], &[])
    }
}

fn bar_len(sig: TimeSignature) -> f64 {
    let num = sig.numerator.max(1) as f64;
    let den = sig.denominator.max(1) as f64;
    num * 4.0 / den
}

impl TempoMapRt {
    /// Non-RT. `points` should contain one at beat 0 (120 BPM is used when empty; otherwise
    /// the first tempo holds before the first point).
    pub fn compile(points: &[TempoPointDesc], signatures: &[TimeSignatureDesc]) -> Self {
        let mut pts: Vec<TempoPointDesc> = points
            .iter()
            .copied()
            .filter(|p| p.beat.is_finite() && p.bpm.is_finite())
            .collect();
        pts.sort_by(|a, b| a.beat.total_cmp(&b.beat));
        // Later points at (nearly) the same beat win.
        pts.dedup_by(|later, earlier| {
            if (later.beat - earlier.beat).abs() <= BEAT_EPS {
                *earlier = *later;
                true
            } else {
                false
            }
        });
        if pts.is_empty() {
            pts.push(TempoPointDesc {
                beat: 0.0,
                bpm: DEFAULT_BPM,
                curve: TempoCurve::Step,
            });
        }

        let mut segments = Vec::with_capacity(pts.len());
        for (i, p) in pts.iter().enumerate() {
            let bpm = p.bpm.clamp(MIN_BPM, MAX_BPM);
            let (end_bpm, length) = match (p.curve, pts.get(i + 1)) {
                (TempoCurve::Linear, Some(next)) => {
                    (next.bpm.clamp(MIN_BPM, MAX_BPM), next.beat - p.beat)
                }
                (_, Some(next)) => (bpm, next.beat - p.beat),
                (_, None) => (bpm, f64::INFINITY),
            };
            // Seconds count from beat 0; the first tempo holds before the first point.
            let seconds = match segments.last() {
                Some(prev) => Segment::seconds_at(prev, p.beat),
                None => 60.0 * p.beat / bpm,
            };
            segments.push(Segment {
                beat: p.beat,
                seconds,
                bpm,
                end_bpm,
                length,
            });
        }

        let mut sigs: Vec<TimeSignatureDesc> = signatures
            .iter()
            .copied()
            .filter(|s| s.beat.is_finite())
            .collect();
        sigs.sort_by(|a, b| a.beat.total_cmp(&b.beat));
        sigs.dedup_by(|later, earlier| {
            if (later.beat - earlier.beat).abs() <= BEAT_EPS {
                *earlier = *later;
                true
            } else {
                false
            }
        });
        if sigs.first().is_none_or(|s| s.beat > BEAT_EPS) {
            sigs.insert(
                0,
                TimeSignatureDesc {
                    beat: 0.0,
                    signature: TimeSignature::default(),
                },
            );
        }
        let signatures: Vec<SigSegment> = sigs
            .iter()
            .map(|s| SigSegment {
                beat: s.beat,
                signature: s.signature,
                bar: bar_len(s.signature),
            })
            .collect();

        let mut boundaries: Vec<f64> = segments
            .iter()
            .skip(1)
            .map(|s| s.beat)
            .chain(signatures.iter().skip(1).map(|s| s.beat))
            .collect();
        boundaries.sort_by(f64::total_cmp);
        boundaries.dedup_by(|a, b| (*a - *b).abs() <= BEAT_EPS);

        Self {
            segments,
            signatures,
            boundaries,
        }
    }

    #[inline]
    fn segment_for_beats(&self, beats: f64) -> &Segment {
        let i = self.segments.partition_point(|s| s.beat <= beats);
        &self.segments[i.saturating_sub(1)]
    }

    #[inline]
    fn segment_for_seconds(&self, seconds: f64) -> &Segment {
        let i = self.segments.partition_point(|s| s.seconds <= seconds);
        &self.segments[i.saturating_sub(1)]
    }

    pub fn beats_to_seconds(&self, beats: f64) -> f64 {
        let seg = self.segment_for_beats(beats);
        if beats < seg.beat {
            // Before the first point: constant first tempo.
            return seg.seconds + 60.0 * (beats - seg.beat) / seg.bpm;
        }
        seg.seconds_at(beats)
    }

    pub fn seconds_to_beats(&self, seconds: f64) -> f64 {
        let seg = self.segment_for_seconds(seconds);
        if seconds < seg.seconds {
            return seg.beat + (seconds - seg.seconds) * seg.bpm / 60.0;
        }
        seg.beats_at(seconds)
    }

    pub fn bpm_at(&self, beats: f64) -> f64 {
        let seg = self.segment_for_beats(beats);
        if beats < seg.beat {
            seg.bpm
        } else {
            seg.bpm_at(beats)
        }
    }

    /// Beat of the next tempo-segment (or time-signature) boundary strictly after `beats`
    /// (block splitting). Boundaries within `1e-6` beats of `beats` are skipped.
    pub fn next_boundary(&self, beats: f64) -> Option<f64> {
        let i = self.boundaries.partition_point(|b| *b <= beats + BEAT_EPS);
        self.boundaries.get(i).copied()
    }

    /// `(signature, beat of the current bar start)`. Bars restart at every signature change.
    pub fn signature_at(&self, beats: f64) -> (TimeSignature, f64) {
        let i = self
            .signatures
            .partition_point(|s| s.beat <= beats + BEAT_EPS);
        let sig = &self.signatures[i.saturating_sub(1)];
        let rel = (beats - sig.beat) / sig.bar;
        // Absorb float error: a position within epsilon of a bar line is on it.
        let bars = (rel + BEAT_EPS / sig.bar).floor();
        (sig.signature, sig.beat + bars * sig.bar)
    }

    /// Number of tempo segments (diagnostics/tests).
    pub fn segment_count(&self) -> usize {
        self.segments.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn pt(beat: f64, bpm: f64, curve: TempoCurve) -> TempoPointDesc {
        TempoPointDesc { beat, bpm, curve }
    }

    fn sig(beat: f64, n: u8, d: u8) -> TimeSignatureDesc {
        TimeSignatureDesc {
            beat,
            signature: TimeSignature {
                numerator: n,
                denominator: d,
            },
        }
    }

    #[test]
    fn default_is_120() {
        let m = TempoMapRt::compile(&[], &[]);
        assert!(close(m.beats_to_seconds(4.0), 2.0));
        assert!(close(m.seconds_to_beats(1.0), 2.0));
        assert!(close(m.bpm_at(100.0), 120.0));
        assert_eq!(m.next_boundary(0.0), None);
        assert!(close(m.beats_to_seconds(-2.0), -1.0));
    }

    #[test]
    fn step_changes() {
        let m = TempoMapRt::compile(
            &[
                pt(0.0, 120.0, TempoCurve::Step),
                pt(4.0, 60.0, TempoCurve::Step),
            ],
            &[],
        );
        assert!(close(m.beats_to_seconds(4.0), 2.0));
        assert!(close(m.beats_to_seconds(6.0), 4.0));
        assert!(close(m.seconds_to_beats(4.0), 6.0));
        assert!(close(m.seconds_to_beats(1.0), 2.0));
        assert_eq!(m.next_boundary(0.0), Some(4.0));
        assert_eq!(m.next_boundary(4.0), None);
        assert!(close(m.bpm_at(3.99), 120.0));
        assert!(close(m.bpm_at(4.0), 60.0));
    }

    #[test]
    fn linear_ramp() {
        // 60 -> 120 BPM over 4 beats: slope 15 BPM/beat.
        let m = TempoMapRt::compile(
            &[
                pt(0.0, 60.0, TempoCurve::Linear),
                pt(4.0, 120.0, TempoCurve::Step),
            ],
            &[],
        );
        let expected = 60.0 / 15.0 * 2f64.ln(); // 4·ln 2 ≈ 2.7726 s
        assert!(close(m.beats_to_seconds(4.0), expected));
        assert!(close(m.bpm_at(2.0), 90.0));
        assert!(close(m.beats_to_seconds(6.0), expected + 1.0));
        for b in [0.0, 0.5, 1.0, 2.5, 3.999, 4.0, 7.25, -1.0] {
            let s = m.beats_to_seconds(b);
            assert!((m.seconds_to_beats(s) - b).abs() < 1e-9, "roundtrip {b}");
        }
    }

    #[test]
    fn first_tempo_holds_before_first_point() {
        let m = TempoMapRt::compile(&[pt(8.0, 60.0, TempoCurve::Step)], &[]);
        assert_eq!(m.segment_count(), 1);
        assert!(close(m.beats_to_seconds(8.0), 8.0));
        assert!(close(m.beats_to_seconds(9.0), 9.0));
        assert!(close(m.seconds_to_beats(2.0), 2.0));
    }

    #[test]
    fn signatures_and_bars() {
        let m = TempoMapRt::compile(&[], &[sig(0.0, 4, 4), sig(8.0, 3, 4), sig(14.0, 6, 8)]);
        let (s, bar) = m.signature_at(5.0);
        assert_eq!(s.numerator, 4);
        assert!(close(bar, 4.0));
        let (s, bar) = m.signature_at(12.5);
        assert_eq!(s.numerator, 3);
        assert!(close(bar, 11.0));
        let (s, bar) = m.signature_at(17.5);
        assert_eq!((s.numerator, s.denominator), (6, 8));
        assert!(close(bar, 17.0));
        // Float error just below a bar line is absorbed.
        let (_, bar) = m.signature_at(4.0 - 1e-9);
        assert!(close(bar, 4.0));
        assert_eq!(m.next_boundary(0.0), Some(8.0));
        assert_eq!(m.next_boundary(8.0), Some(14.0));
    }
}

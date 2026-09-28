//! Automation breakpoints under time edits (pure functions over one lane's points).
//!
//! A lane's value is piecewise: before the first point it is the first point's value, after
//! the last one the last point's value, in between each point's `curve` shapes the segment
//! to the next one (`ether_core::automation::evaluate`). Every edit here keeps the curve
//! outside the edited range exactly as it was, by adding breakpoints at the range edges
//! where needed:
//! - [`delete_time`]: the value up to `a` and from `b` on are kept; the range is removed
//!   (a jump at `a` if the two sides differ);
//! - [`insert_time`]: the curve before `at` is kept, the curve from `at` on moves right, and
//!   the inserted gap holds the value at `at`;
//! - [`slice`] / [`overwrite`]: copy a range (with its edge values) and write it over a range.
//!
//! Two points at the same time form a jump; which comes first is decided by id order
//! (`Project::points_of` sorts by time, then id), so [`super::edit::rewrite_lane`] gives
//! such groups ascending ids in list order.
//!
//! Tension curves split at an edge are approximated (the kept part keeps its tension);
//! linear and step segments are exact.

use ether_core::protocol::model::{AutomationPointId, CurveShape};

/// Same-time tolerance (beats).
pub(super) const TIME_EPS: f64 = 1e-9;
/// Values closer than this are equal (normalized 0..1).
const VALUE_EPS: f64 = 1e-9;

/// One breakpoint; `id: None` = a new point.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Pt {
    pub id: Option<AutomationPointId>,
    pub time: f64,
    pub value: f64,
    pub curve: CurveShape,
}

impl Pt {
    pub fn new(time: f64, value: f64, curve: CurveShape) -> Self {
        Self {
            id: None,
            time,
            value,
            curve,
        }
    }
}

fn shape(curve: CurveShape, x: f64) -> f64 {
    match curve {
        CurveShape::Linear => x,
        CurveShape::Step => 0.0,
        CurveShape::Curve { tension } => {
            let t = (tension as f64).clamp(-1.0, 1.0);
            x.clamp(0.0, 1.0).powf(4f64.powf(t))
        }
    }
}

fn segment(p0: &Pt, p1: &Pt, time: f64) -> f64 {
    let span = p1.time - p0.time;
    if span <= 0.0 {
        return p1.value;
    }
    p0.value + (p1.value - p0.value) * shape(p0.curve, (time - p0.time) / span)
}

/// Value at `time` (right limit: a point exactly at `time` counts). `pts` sorted.
pub(super) fn value_at(pts: &[Pt], time: f64) -> Option<f64> {
    let first = pts.first()?;
    if time < first.time {
        return Some(first.value);
    }
    // First point strictly after `time`.
    let i = pts.partition_point(|p| p.time <= time);
    if i >= pts.len() {
        return Some(pts[pts.len() - 1].value);
    }
    Some(segment(&pts[i - 1], &pts[i], time))
}

/// Value just before `time` (left limit: points exactly at `time` don't count yet).
pub(super) fn value_before(pts: &[Pt], time: f64) -> Option<f64> {
    let first = pts.first()?;
    if time <= first.time {
        return Some(first.value);
    }
    // First point at or after `time`.
    let i = pts.partition_point(|p| p.time < time);
    if i >= pts.len() {
        return Some(pts[pts.len() - 1].value);
    }
    Some(segment(&pts[i - 1], &pts[i], time))
}

/// Curve of the segment running through `time` (the last point at or before it).
fn curve_at(pts: &[Pt], time: f64) -> CurveShape {
    let i = pts.partition_point(|p| p.time <= time);
    i.checked_sub(1)
        .map_or(CurveShape::Linear, |i| pts[i].curve)
}

fn shifted(p: &Pt, delta: f64) -> Pt {
    Pt {
        time: p.time + delta,
        ..p.clone()
    }
}

/// Remove `[a, b]` and shift the points after `b` left by `b - a`.
pub(super) fn delete_time(pts: &[Pt], a: f64, b: f64) -> Vec<Pt> {
    let len = b - a;
    let has_in = pts
        .iter()
        .any(|p| p.time >= a - TIME_EPS && p.time <= b + TIME_EPS);
    let has_before = pts.iter().any(|p| p.time < a - TIME_EPS);
    let has_after = pts.iter().any(|p| p.time > b + TIME_EPS);
    if !has_in && !(has_before && has_after) {
        // The value is constant across the range: only shift.
        return pts
            .iter()
            .map(|p| {
                if p.time > b {
                    shifted(p, -len)
                } else {
                    p.clone()
                }
            })
            .collect();
    }
    let (Some(left), Some(right)) = (value_before(pts, a), value_at(pts, b)) else {
        return Vec::new();
    };
    let right_curve = curve_at(pts, b);
    let mut out: Vec<Pt> = pts
        .iter()
        .filter(|p| p.time < a - TIME_EPS)
        .cloned()
        .collect();
    if (left - right).abs() <= VALUE_EPS {
        out.push(Pt::new(a, left, right_curve));
    } else {
        out.push(Pt::new(a, left, CurveShape::Linear));
        out.push(Pt::new(a, right, right_curve));
    }
    out.extend(
        pts.iter()
            .filter(|p| p.time > b + TIME_EPS)
            .map(|p| shifted(p, -len)),
    );
    out
}

/// Shift the points at or after `at` right by `len`; the gap holds the value at `at`.
pub(super) fn insert_time(pts: &[Pt], at: f64, len: f64) -> Vec<Pt> {
    let has_before = pts.iter().any(|p| p.time < at - TIME_EPS);
    let has_after = pts.iter().any(|p| p.time >= at - TIME_EPS);
    let moved = |p: &Pt| {
        if p.time >= at - TIME_EPS {
            shifted(p, len)
        } else {
            p.clone()
        }
    };
    if !(has_before && has_after) {
        return pts.iter().map(moved).collect();
    }
    let (Some(left), Some(right)) = (value_before(pts, at), value_at(pts, at)) else {
        return Vec::new();
    };
    let exact = pts.iter().any(|p| (p.time - at).abs() <= TIME_EPS);
    let right_curve = curve_at(pts, at);
    let mut out: Vec<Pt> = pts
        .iter()
        .filter(|p| p.time < at - TIME_EPS)
        .cloned()
        .collect();
    out.push(Pt::new(at, left, CurveShape::Step));
    if !exact {
        out.push(Pt::new(at + len, right, right_curve));
    }
    out.extend(pts.iter().filter(|p| p.time >= at - TIME_EPS).map(moved));
    out
}

/// The points of `[a, b]` relative to `a`, with the edge values at `0` and `b - a`
/// (`None` when the lane has no points).
pub(super) fn slice(pts: &[Pt], a: f64, b: f64) -> Option<Vec<Pt>> {
    let start = value_at(pts, a)?;
    let end = value_before(pts, b)?;
    let mut out = vec![Pt::new(0.0, start, curve_at(pts, a))];
    out.extend(
        pts.iter()
            .filter(|p| p.time > a + TIME_EPS && p.time < b - TIME_EPS)
            .map(|p| Pt {
                id: None,
                time: p.time - a,
                ..p.clone()
            }),
    );
    out.push(Pt::new(b - a, end, CurveShape::Linear));
    Some(out)
}

/// Replace `[at, at + len]` of `pts` with `piece` (relative times, from [`slice`]); the
/// curve outside the range is kept.
pub(super) fn overwrite(pts: &[Pt], at: f64, piece: &[Pt], len: f64) -> Vec<Pt> {
    let end = at + len;
    let has_before = pts.iter().any(|p| p.time < at - TIME_EPS);
    let has_after = pts.iter().any(|p| p.time > end + TIME_EPS);
    let mut out: Vec<Pt> = pts
        .iter()
        .filter(|p| p.time < at - TIME_EPS)
        .cloned()
        .collect();
    if has_before && let Some(left) = value_before(pts, at) {
        out.push(Pt::new(at, left, CurveShape::Linear));
    }
    out.extend(piece.iter().map(|p| Pt {
        id: None,
        time: p.time + at,
        ..p.clone()
    }));
    if has_after && let Some(right) = value_at(pts, end) {
        out.push(Pt::new(end, right, curve_at(pts, end)));
    }
    out.extend(pts.iter().filter(|p| p.time > end + TIME_EPS).cloned());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lane(pts: &[(f64, f64)]) -> Vec<Pt> {
        pts.iter()
            .map(|&(t, v)| Pt::new(t, v, CurveShape::Linear))
            .collect()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn values_left_and_right() {
        let pts = vec![
            Pt::new(0.0, 0.0, CurveShape::Linear),
            Pt::new(4.0, 1.0, CurveShape::Step),
            Pt::new(8.0, 0.5, CurveShape::Linear),
        ];
        assert!(close(value_at(&pts, 2.0).unwrap(), 0.5));
        assert!(close(value_before(&pts, 4.0).unwrap(), 1.0));
        assert!(close(value_at(&pts, 6.0).unwrap(), 1.0));
        assert!(close(value_before(&pts, 8.0).unwrap(), 1.0));
        assert!(close(value_at(&pts, 8.0).unwrap(), 0.5));
        assert!(close(value_at(&pts, 20.0).unwrap(), 0.5));
        assert!(close(value_at(&pts, -1.0).unwrap(), 0.0));
        assert_eq!(value_at(&[], 1.0), None);
    }

    #[test]
    fn delete_keeps_both_sides() {
        // Ramp 0→1 over 0..8, delete 2..6: 0..2 unchanged, then a jump to 0.75 and 0.75→1
        // over 2..4.
        let out = delete_time(&lane(&[(0.0, 0.0), (8.0, 1.0)]), 2.0, 6.0);
        assert_eq!(out.len(), 4);
        for (t, v) in [(1.0, 0.125), (3.0, 0.875), (4.0, 1.0), (10.0, 1.0)] {
            assert!(close(value_at(&out, t).unwrap(), v), "{t}: {out:?}");
        }
        assert!(close(value_before(&out, 2.0).unwrap(), 0.25));
        assert!(close(value_at(&out, 2.0).unwrap(), 0.75));
    }

    #[test]
    fn delete_far_from_points_only_shifts() {
        let pts = lane(&[(0.0, 0.2), (1.0, 0.4)]);
        assert_eq!(delete_time(&pts, 4.0, 8.0), pts);
        let out = delete_time(&lane(&[(10.0, 0.2)]), 4.0, 8.0);
        assert_eq!(out, lane(&[(6.0, 0.2)]));
    }

    #[test]
    fn insert_holds_value() {
        let out = insert_time(&lane(&[(0.0, 0.0), (8.0, 1.0)]), 4.0, 4.0);
        for (t, v) in [
            (2.0, 0.25),
            (4.0, 0.5),
            (6.0, 0.5),
            (8.0, 0.5),
            (10.0, 0.75),
            (12.0, 1.0),
        ] {
            assert!(close(value_at(&out, t).unwrap(), v), "{t}: {out:?}");
        }
        // A point exactly at the insertion moves with the material.
        let out = insert_time(&lane(&[(0.0, 0.0), (4.0, 1.0)]), 4.0, 2.0);
        assert!(close(value_at(&out, 5.0).unwrap(), 1.0));
        assert!(close(value_at(&out, 6.0).unwrap(), 1.0));
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn slice_then_overwrite_round_trips() {
        let src = lane(&[(0.0, 0.0), (8.0, 1.0)]);
        let piece = slice(&src, 2.0, 6.0).unwrap();
        assert_eq!(piece.len(), 2);
        assert!(close(piece[0].value, 0.25) && close(piece[1].value, 0.75));
        // Pasting a slice over the same range leaves the curve unchanged.
        let out = overwrite(&src, 2.0, &piece, 4.0);
        for t in [0.0, 1.0, 2.0, 3.0, 5.0, 6.0, 7.0, 9.0] {
            assert!(
                close(value_at(&out, t).unwrap(), value_at(&src, t).unwrap()),
                "{t}"
            );
        }
        // Onto a flat lane: the pasted ramp, then back to the lane's value.
        let flat = lane(&[(0.0, 0.5), (20.0, 0.5)]);
        let out = overwrite(&flat, 10.0, &piece, 4.0);
        assert!(close(value_at(&out, 9.0).unwrap(), 0.5));
        assert!(close(value_at(&out, 12.0).unwrap(), 0.5));
        assert!(close(value_at(&out, 10.0).unwrap(), 0.25));
        assert!(close(value_before(&out, 14.0).unwrap(), 0.75));
        assert!(close(value_at(&out, 15.0).unwrap(), 0.5));
    }
}

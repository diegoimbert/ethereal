//! `NoteCommand::Reverse`: mirror a MIDI clip's content (or some of its notes) in time.
//!
//! - **Whole clip** (`notes: None`): the region is the clip's loop region when looping,
//!   else the content span it plays (`max(offset, 0) .. offset + length`). Every note
//!   lying entirely inside the region is mirrored (`start' = r0 + r1 - end`; duration,
//!   pitch, velocity, release velocity and mute are kept) together with its per-note
//!   expressions (mirrored inside the note). Notes crossing a region edge or lying outside
//!   it are **left as they are** (mirroring them would land before the region or before
//!   content time 0). The clip's expression lanes (CC, pitch bend, pressure, ...) and
//!   clip envelopes are mirrored over the same region.
//! - **Selection** (`notes: Some(ids)`): the span is `min start .. max end` of the
//!   selected notes of the clip; only those notes (and their per-note expressions) are
//!   mirrored inside it. Lanes, envelopes and unselected notes stay.
//!
//! Reversing twice restores the original (notes: up to float rounding; curves: see
//! [`reverse_points`]). One transaction, so one undo step; the result is plain entity
//! updates, so collab peers replay them like any other edit.

use ether_core::protocol::model::*;

use super::DocCtx;
use crate::tx::CmdResult;

const EPS: f64 = Beats::EPSILON;

/// One breakpoint, curve value widened to f64 (expression values are f32).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pt {
    pub t: f64,
    pub v: f64,
    pub curve: CurveShape,
}

fn mirror_shape(c: CurveShape) -> CurveShape {
    match c {
        // `x^(4^tension)` reversed has no exact form; the opposite tension is the closest
        // (and an involution, so a double reverse restores it).
        CurveShape::Curve { tension } => CurveShape::Curve { tension: -tension },
        other => other,
    }
}

fn value_at(pts: &[Pt], t: f64) -> (f64, CurveShape) {
    let i = pts.partition_point(|p| p.t <= t);
    if i == 0 {
        return (pts[0].v, pts[0].curve);
    }
    if i >= pts.len() {
        return (pts[pts.len() - 1].v, CurveShape::Step);
    }
    let (a, b) = (pts[i - 1], pts[i]);
    let shape = match a.curve {
        CurveShape::Curve { .. } => CurveShape::Linear,
        s => s,
    };
    let x = if b.t > a.t {
        (t - a.t) / (b.t - a.t)
    } else {
        1.0
    };
    let v = match a.curve {
        CurveShape::Step => a.v,
        CurveShape::Linear => a.v + (b.v - a.v) * x,
        CurveShape::Curve { tension } => {
            a.v + (b.v - a.v) * ether_core::automation::curve_fraction(x, tension)
        }
    };
    (v, shape)
}

/// `pts` (sorted by time) with the signal inside `[r0, r1]` mirrored in time; points
/// outside stay. Segment shapes follow their segment (`Step` holds the value that was held
/// before, `Curve` flips its tension). Where the region cuts a segment or an edge is
/// covered by the first/last point's hold, an edge point is added so the mirror is exact,
/// and dropped again when it is redundant (a double reverse gives the input back).
pub fn reverse_points(pts: &[Pt], r0: f64, r1: f64) -> Vec<Pt> {
    let left: Vec<Pt> = pts.iter().copied().filter(|p| p.t < r0).collect();
    let inner: Vec<Pt> = pts
        .iter()
        .copied()
        .filter(|p| p.t >= r0 && p.t <= r1)
        .collect();
    let right: Vec<Pt> = pts.iter().copied().filter(|p| p.t > r1).collect();
    if inner.is_empty() || r1 <= r0 {
        return pts.to_vec();
    }
    let trail = inner[inner.len() - 1].curve;
    // The virtual chain over exactly [r0, r1], with the shape leaving each point.
    let mut v: Vec<Pt> = Vec::with_capacity(inner.len() + 2);
    let mut front = false;
    let mut back = false;
    if inner[0].t > r0 {
        front = true;
        let (val, shape) = if left.is_empty() {
            (inner[0].v, CurveShape::Step)
        } else {
            value_at(pts, r0)
        };
        v.push(Pt {
            t: r0,
            v: val,
            curve: shape,
        });
    }
    v.extend(inner.iter().copied());
    let mut shapes: Vec<CurveShape> = v.iter().map(|p| p.curve).collect();
    if inner[inner.len() - 1].t < r1 {
        back = true;
        let last = inner[inner.len() - 1];
        let (val, shape) = if right.is_empty() {
            (last.v, CurveShape::Step)
        } else {
            let (val, _) = value_at(pts, r1);
            (
                val,
                match trail {
                    CurveShape::Curve { .. } => CurveShape::Linear,
                    s => s,
                },
            )
        };
        let n = shapes.len();
        shapes[n - 1] = shape;
        v.push(Pt {
            t: r1,
            v: val,
            curve: CurveShape::Step,
        });
        shapes.push(CurveShape::Step);
    }
    let n = v.len() - 1;
    let m = |t: f64| r0 + r1 - t;
    let mut rev: Vec<Pt> = Vec::with_capacity(v.len());
    for k in (0..n).rev() {
        let s = shapes[k];
        rev.push(Pt {
            t: m(v[k + 1].t),
            v: if matches!(s, CurveShape::Step) {
                v[k].v
            } else {
                v[k + 1].v
            },
            curve: mirror_shape(s),
        });
    }
    rev.push(Pt {
        t: m(v[0].t),
        v: v[0].v,
        curve: trail,
    });
    let flat = |a: f64, b: f64| (a - b).abs() <= 1e-9;
    // Drop the edge points added above when they changed nothing.
    let has = |t: f64| pts.iter().any(|p| p.t == t);
    if (back || !has(r0)) && left.is_empty() && rev.len() > 1 && flat(rev[0].v, rev[1].v) {
        rev.remove(0);
    }
    if (front || !has(r1))
        && right.is_empty()
        && rev.len() > 1
        && flat(rev[rev.len() - 1].v, rev[rev.len() - 2].v)
    {
        rev.pop();
    }
    if right.is_empty()
        && let Some(last) = rev.last_mut()
    {
        last.curve = trail;
    }
    let mut out = left;
    out.extend(rev);
    out.extend(right);
    out
}

fn to_pts(points: &[ExpressionPoint]) -> Vec<Pt> {
    points
        .iter()
        .map(|p| Pt {
            t: p.time.0,
            v: p.value as f64,
            curve: p.curve,
        })
        .collect()
}

fn from_pts(points: &[Pt]) -> Vec<ExpressionPoint> {
    points
        .iter()
        .map(|p| ExpressionPoint {
            time: Beats(p.t.max(0.0)),
            value: p.v as f32,
            curve: p.curve,
        })
        .collect()
}

/// The span of a clip's content that gets mirrored.
fn clip_region(cl: &Clip) -> (f64, f64) {
    if cl.looping.enabled {
        (cl.looping.start.0, cl.looping.end.0)
    } else {
        (cl.offset.0.max(0.0), cl.offset.0 + cl.length.0)
    }
}

fn reverse_note_expressions(ctx: &mut DocCtx, note: &Note) -> CmdResult<()> {
    let exprs: Vec<NoteExpression> = ctx
        .p()
        .note_expressions_of(note.id)
        .into_iter()
        .cloned()
        .collect();
    for e in exprs {
        let pts = reverse_points(&to_pts(&e.points), 0.0, note.duration.0);
        let points = from_pts(&pts);
        if points != e.points && check_points(&points, e.kind.range()).is_ok() {
            ctx.tx.update(EntityUpdate::NoteExpression {
                id: e.id,
                change: NoteExpressionChange::Points(points),
            })?;
        }
    }
    Ok(())
}

fn reverse_lanes(ctx: &mut DocCtx, clip: ClipId, r0: f64, r1: f64) -> CmdResult<()> {
    let lanes: Vec<ExpressionLane> = ctx
        .p()
        .expression_lanes_of(clip)
        .into_iter()
        .cloned()
        .collect();
    for l in lanes {
        let pts = reverse_points(&to_pts(&l.points), r0, r1);
        let points = from_pts(&pts);
        if points != l.points && check_points(&points, l.kind.range()).is_ok() {
            ctx.tx.update(EntityUpdate::ExpressionLane {
                id: l.id,
                change: ExpressionLaneChange::Points(points),
            })?;
        }
    }
    Ok(())
}

pub fn reverse_envelopes(ctx: &mut DocCtx, clip: ClipId, r0: f64, r1: f64) -> CmdResult<()> {
    for lane in ctx.p().lane_ids_owned_by(AutomationOwner::Clip { clip }) {
        let old: Vec<AutomationPoint> = ctx.p().points_of(lane).into_iter().cloned().collect();
        let all: Vec<Pt> = old
            .iter()
            .map(|p| Pt {
                t: p.time.0,
                v: p.value,
                curve: p.curve,
            })
            .collect();
        let new = reverse_points(&all, r0, r1);
        if new == all {
            continue;
        }
        if new.len() == old.len() {
            for (o, n) in old.iter().zip(&new) {
                if !o.time.approx_eq(Beats(n.t)) {
                    ctx.tx.update(EntityUpdate::AutomationPoint {
                        id: o.id,
                        change: AutomationPointChange::Time(Beats(n.t)),
                    })?;
                }
                if o.value != n.v {
                    ctx.tx.update(EntityUpdate::AutomationPoint {
                        id: o.id,
                        change: AutomationPointChange::Value(n.v),
                    })?;
                }
                if o.curve != n.curve {
                    ctx.tx.update(EntityUpdate::AutomationPoint {
                        id: o.id,
                        change: AutomationPointChange::Curve(n.curve),
                    })?;
                }
            }
        } else {
            for o in &old {
                ctx.tx.remove(EntityKey::AutomationPoint(o.id))?;
            }
            for n in new {
                let id: AutomationPointId = ctx.new_id();
                ctx.tx.insert(Entity::AutomationPoint(AutomationPoint {
                    id,
                    lane,
                    time: Beats(n.t),
                    value: n.v,
                    curve: n.curve,
                }))?;
            }
        }
    }
    Ok(())
}

pub(super) fn reverse(ctx: &mut DocCtx, clip: ClipId, notes: Option<&[NoteId]>) -> CmdResult<()> {
    let cl = ctx.clip(clip)?;
    if !matches!(cl.content, ClipContent::Midi) {
        return Err(crate::tx::invalid(format!(
            "clip {clip} is not a MIDI clip"
        )));
    }
    let mut targets: Vec<Note> = ctx.p().notes_of(clip).into_iter().cloned().collect();
    let (r0, r1) = match notes {
        None => clip_region(&cl),
        Some(ids) => {
            targets.retain(|n| ids.contains(&n.id));
            let r0 = targets
                .iter()
                .map(|n| n.start.0)
                .fold(f64::INFINITY, f64::min);
            let r1 = targets
                .iter()
                .map(|n| n.start.0 + n.duration.0)
                .fold(f64::NEG_INFINITY, f64::max);
            (r0, r1)
        }
    };
    // Empty selection or an empty/degenerate region: nothing to do.
    if !(r0.is_finite() && r1.is_finite() && r1 > r0) {
        return Ok(());
    }
    for n in targets {
        let end = n.start.0 + n.duration.0;
        if n.start.0 < r0 - EPS || end > r1 + EPS {
            continue;
        }
        let start = (r0 + r1 - end).max(0.0);
        if !Beats(start).approx_eq(n.start) {
            ctx.tx.update(EntityUpdate::Note {
                id: n.id,
                change: NoteChange::Start(Beats(start)),
            })?;
        }
        reverse_note_expressions(ctx, &n)?;
    }
    if notes.is_none() {
        reverse_lanes(ctx, clip, r0, r1)?;
        reverse_envelopes(ctx, clip, r0, r1)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(t: f64, v: f64, curve: CurveShape) -> Pt {
        Pt { t, v, curve }
    }

    #[test]
    fn double_reverse_is_identity() {
        use CurveShape::*;
        let cases = [
            vec![p(1.0, 0.2, Linear), p(3.0, 0.9, Linear)],
            vec![p(1.0, 0.2, Step), p(2.0, 0.5, Step), p(3.0, 0.9, Linear)],
            vec![
                p(0.0, 0.1, Curve { tension: 0.5 }),
                p(2.0, 0.7, Linear),
                p(4.0, 0.3, Step),
            ],
            vec![p(1.0, 0.4, Linear)],
        ];
        for c in cases {
            let once = reverse_points(&c, 0.0, 4.0);
            let twice = reverse_points(&once, 0.0, 4.0);
            assert_eq!(twice, c, "once = {once:?}");
        }
    }

    #[test]
    fn linear_ramp_mirrors() {
        let c = vec![
            p(0.0, 0.0, CurveShape::Linear),
            p(4.0, 1.0, CurveShape::Linear),
        ];
        let r = reverse_points(&c, 0.0, 4.0);
        assert_eq!(r[0].v, 1.0);
        assert_eq!(r[1].v, 0.0);
        assert_eq!(r[1].t, 4.0);
    }

    #[test]
    fn step_holds_mirror() {
        use CurveShape::Step;
        // 0.2 on [1,2), 0.5 on [2,4]; mirrored over 0..4: 0.5 on [0,2), 0.2 on [2,3], then 0.2.
        let c = vec![p(1.0, 0.2, Step), p(2.0, 0.5, Step)];
        let r = reverse_points(&c, 0.0, 4.0);
        let at = |t: f64| value_at(&r, t).0;
        assert_eq!(at(0.5), 0.5);
        assert_eq!(at(1.9), 0.5);
        assert_eq!(at(2.5), 0.2);
        assert_eq!(at(3.9), 0.2);
    }
}

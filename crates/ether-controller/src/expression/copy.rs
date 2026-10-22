//! Expression follows copied notes and clips (the paths outside `DocCtx::copy_clip`).
//!
//! Callers choose the new ids (`ctx.new_id()` for interactive edits, `derive_id` where the
//! edit derives every id from a seed so collab peers agree).

use ether_core::protocol::model::*;

use crate::doc::DocCtx;
use crate::expression::compile::evaluate_points;
use crate::tx::CmdResult;

/// Copy note `from`'s expressions onto note `to` (`id(k)` = the id of the k-th copy).
pub fn copy_note_expressions(
    ctx: &mut DocCtx,
    from: NoteId,
    to: NoteId,
    mut id: impl FnMut(&mut DocCtx, usize) -> NoteExpressionId,
) -> CmdResult<()> {
    let exprs: Vec<NoteExpression> = ctx
        .p()
        .note_expressions_of(from)
        .into_iter()
        .cloned()
        .collect();
    for (k, mut e) in exprs.into_iter().enumerate() {
        e.id = id(ctx, k);
        e.note = to;
        if ctx.p().note_expressions.contains_key(&e.id) {
            continue;
        }
        ctx.tx.insert(Entity::NoteExpression(e))?;
    }
    Ok(())
}

/// Copy clip `from`'s lanes onto MIDI clip `to` (same content times).
pub fn copy_lanes(
    ctx: &mut DocCtx,
    from: ClipId,
    to: ClipId,
    mut id: impl FnMut(&mut DocCtx, usize) -> ExpressionLaneId,
) -> CmdResult<()> {
    let lanes: Vec<ExpressionLane> = ctx
        .p()
        .expression_lanes_of(from)
        .into_iter()
        .cloned()
        .collect();
    for (k, mut l) in lanes.into_iter().enumerate() {
        l.id = id(ctx, k);
        l.clip = to;
        if ctx.p().expression_lanes.contains_key(&l.id) {
            continue;
        }
        ctx.tx.insert(Entity::ExpressionLane(l))?;
    }
    Ok(())
}

/// A stretch of a clip's content laid out somewhere else: content `[c0, c1)` lands at
/// `at` (in the destination's time).
#[derive(Clone, Copy, Debug)]
pub struct Piece {
    pub clip: ClipId,
    pub at: f64,
    pub c0: f64,
    pub c1: f64,
}

/// Lay `points`' stretch `[c0, c1)` at `at` into `curve`: its value at `c0`, its inner
/// points and its value at the end (so a later piece or the end of the curve jumps exactly
/// there). Later placements win where they overlap; a point already at the start (the end
/// of the previous piece) stays, so the curve jumps there.
fn place(curve: &mut Vec<ExpressionPoint>, points: &[ExpressionPoint], at: f64, c0: f64, c1: f64) {
    let Some(first) = evaluate_points(points, c0) else {
        return;
    };
    let to = |c: f64| Beats((at + (c - c0)).max(0.0));
    let mut pts = vec![ExpressionPoint {
        time: to(c0),
        value: first.0,
        curve: first.1,
    }];
    for q in points {
        if q.time.0 > c0 && q.time.0 < c1 {
            pts.push(ExpressionPoint {
                time: to(q.time.0),
                ..*q
            });
        }
    }
    if let Some(last) = evaluate_points(points, c1 - 1e-9) {
        pts.push(ExpressionPoint {
            time: to(c1),
            value: last.0,
            curve: CurveShape::Step,
        });
    }
    let (start, end) = (to(c0).0, to(c1).0);
    curve.retain(|q| q.time.0 <= start || q.time.0 > end);
    let i = curve.partition_point(|q| q.time.0 <= start);
    curve.splice(i..i, pts);
}

/// A played note joined across loop wraps (CONTRACTS.md §2, Clips, "loop join"): from
/// `at` beats after its start it plays note `note` (which starts `lead` beats before the
/// loop start), restarted every `period` beats (the loop length).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoopJoin {
    pub at: f64,
    pub note: NoteId,
    pub lead: f64,
    pub period: f64,
}

/// The note expressions of a consolidated note joined across loop wraps ([`LoopJoin`]),
/// `len` beats long:
/// `first` (its first note's expressions) until the wrap, then the loop start note's from
/// `lead` beats into it, restarted on each later wrap: what the engine sends for the
/// joined voice. Ids and notes are placeholders (the caller sets them).
pub fn join_note_expressions(
    p: &Project,
    first: &[NoteExpression],
    source: NoteId,
    join: LoopJoin,
    len: f64,
) -> Vec<NoteExpression> {
    fn curve(
        out: &mut Vec<(NoteExpressionKind, Vec<ExpressionPoint>)>,
        kind: NoteExpressionKind,
    ) -> &mut Vec<ExpressionPoint> {
        let i = match out.iter().position(|(k, _)| *k == kind) {
            Some(i) => i,
            None => {
                out.push((kind, Vec::new()));
                out.len() - 1
            }
        };
        &mut out[i].1
    }
    let partner: Vec<&NoteExpression> = p.note_expressions_of(join.note);
    let mut out = Vec::new();
    for e in first {
        place(
            curve(&mut out, e.kind),
            &e.points,
            0.0,
            0.0,
            join.at.min(len),
        );
    }
    let mut w = join.at;
    while w < len - 1e-9 && join.period > 0.0 {
        let span = join.period.min(len - w);
        for e in &partner {
            place(
                curve(&mut out, e.kind),
                &e.points,
                w,
                join.lead,
                join.lead + span,
            );
        }
        w += join.period;
    }
    out.into_iter()
        .enumerate()
        .map(|(k, (kind, mut points))| {
            points.truncate(MAX_EXPRESSION_POINTS);
            NoteExpression {
                id: ether_model::derive_id(source, 0x4a00 + k as u32),
                note: source,
                kind,
                points,
            }
        })
        .collect()
}

/// The lanes of the clips in `pieces`, laid out one after the other in destination time
/// (consolidate: looped and offset clips unrolled). Each piece contributes its value at
/// `c0`, its inner points and its value at the end (so a later piece or the end of the
/// curve jumps exactly there); pieces of later clips overwrite overlaps. Curves are capped
/// at [`MAX_EXPRESSION_POINTS`] (the rest is dropped).
pub fn unroll_lanes(p: &Project, pieces: &[Piece]) -> Vec<(ExpressionKind, Vec<ExpressionPoint>)> {
    let mut out: Vec<(ExpressionKind, Vec<ExpressionPoint>)> = Vec::new();
    for piece in pieces {
        if piece.c1 <= piece.c0 {
            continue;
        }
        for lane in p.expression_lanes_of(piece.clip) {
            let curve = match out.iter_mut().find(|(k, _)| *k == lane.kind) {
                Some((_, c)) => c,
                None => {
                    out.push((lane.kind, Vec::new()));
                    &mut out.last_mut().unwrap().1
                }
            };
            place(curve, &lane.points, piece.at, piece.c0, piece.c1);
        }
    }
    for (_, c) in &mut out {
        c.truncate(MAX_EXPRESSION_POINTS);
    }
    out.sort_by_key(|(k, _)| *k);
    out
}

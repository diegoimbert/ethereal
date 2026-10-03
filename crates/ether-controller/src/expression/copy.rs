//! Expression follows copied notes and clips (the paths outside `DocCtx::copy_clip`).
//!
//! Callers choose the new ids (`ctx.new_id()` for interactive edits, `derive_id` where the
//! edit derives every id from a seed so collab peers agree).

use ether_core::protocol::model::*;

use crate::doc::DocCtx;
use crate::expression::compile::evaluate_points;
use crate::tx::CmdResult;

/// Copy note `from`'s expressions onto note `to` (`id(k)` = the id of the k-th copy).
pub(crate) fn copy_note_expressions(
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
pub(crate) fn copy_lanes(
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
pub(crate) struct Piece {
    pub clip: ClipId,
    pub at: f64,
    pub c0: f64,
    pub c1: f64,
}

/// The lanes of the clips in `pieces`, laid out one after the other in destination time
/// (consolidate: looped and offset clips unrolled). Each piece contributes its value at
/// `c0`, its inner points and its value at the end (so a later piece or the end of the
/// curve jumps exactly there); pieces of later clips overwrite overlaps. Curves are capped
/// at [`MAX_EXPRESSION_POINTS`] (the rest is dropped).
pub(crate) fn unroll_lanes(p: &Project, pieces: &[Piece]) -> Vec<(ExpressionKind, Vec<ExpressionPoint>)> {
    let mut out: Vec<(ExpressionKind, Vec<ExpressionPoint>)> = Vec::new();
    for piece in pieces {
        if piece.c1 <= piece.c0 {
            continue;
        }
        for lane in p.expression_lanes_of(piece.clip) {
            let Some(first) = evaluate_points(&lane.points, piece.c0) else {
                continue;
            };
            let at = |c: f64| Beats((piece.at + (c - piece.c0)).max(0.0));
            let mut pts = vec![ExpressionPoint {
                time: at(piece.c0),
                value: first.0,
                curve: first.1,
            }];
            for q in &lane.points {
                if q.time.0 > piece.c0 && q.time.0 < piece.c1 {
                    pts.push(ExpressionPoint {
                        time: at(q.time.0),
                        ..*q
                    });
                }
            }
            if let Some(last) = evaluate_points(&lane.points, piece.c1 - 1e-9) {
                pts.push(ExpressionPoint {
                    time: at(piece.c1),
                    value: last.0,
                    curve: CurveShape::Step,
                });
            }
            let start = at(piece.c0).0;
            let end = at(piece.c1).0;
            let curve = match out.iter_mut().find(|(k, _)| *k == lane.kind) {
                Some((_, c)) => c,
                None => {
                    out.push((lane.kind, Vec::new()));
                    &mut out.last_mut().unwrap().1
                }
            };
            // Later pieces win where they overlap; a point already at `start` (the end of
            // the previous piece) stays, so the curve jumps there.
            curve.retain(|q| q.time.0 <= start || q.time.0 > end);
            let i = curve.partition_point(|q| q.time.0 <= start);
            curve.splice(i..i, pts);
        }
    }
    for (_, c) in &mut out {
        c.truncate(MAX_EXPRESSION_POINTS);
    }
    out.sort_by_key(|(k, _)| *k);
    out
}

//! `ExpressionCommand`s (document commands, one undo step each; idempotent).

use ether_core::protocol::expression::ExpressionCommand;
use ether_core::protocol::model::*;

use crate::doc::DocCtx;
use crate::tx::{CmdResult, invalid, not_found};

/// Apply one `ExpressionCommand`.
pub(crate) fn expression_command(ctx: &mut DocCtx, c: &ExpressionCommand) -> CmdResult<()> {
    match c {
        ExpressionCommand::SetTrackMpe { track, mpe } => {
            crate::mpe::set_track_mpe(ctx, *track, *mpe)
        }
        ExpressionCommand::CreateLane { id, clip, kind } => create_lane(ctx, *id, *clip, *kind),
        ExpressionCommand::RemoveLane { id } => {
            lane(ctx, *id)?;
            ctx.tx.remove(EntityKey::ExpressionLane(*id))
        }
        ExpressionCommand::SetPoints { lane: id, points } => {
            let l = lane(ctx, *id)?;
            set_lane_points(ctx, &l, points.clone())
        }
        ExpressionCommand::ReplaceRange {
            lane: id,
            start,
            end,
            points,
        } => {
            let l = lane(ctx, *id)?;
            let merged = replace_range(&l.points, *start, *end, points)?;
            set_lane_points(ctx, &l, merged)
        }
        ExpressionCommand::SetNoteExpression {
            id,
            note,
            kind,
            points,
        } => set_note_expression(ctx, *id, *note, *kind, points),
        ExpressionCommand::ClearNoteExpressions { notes, kind } => {
            for note in notes {
                let ids: Vec<NoteExpressionId> = ctx
                    .p()
                    .note_expressions_of(*note)
                    .into_iter()
                    .filter(|e| kind.is_none_or(|k| e.kind == k))
                    .map(|e| e.id)
                    .collect();
                for id in ids {
                    ctx.tx.remove(EntityKey::NoteExpression(id))?;
                }
            }
            Ok(())
        }
    }
}

fn lane(ctx: &DocCtx, id: ExpressionLaneId) -> CmdResult<ExpressionLane> {
    ctx.p()
        .expression_lanes
        .get(&id)
        .cloned()
        .ok_or_else(|| not_found(format!("expression lane {id}")))
}

fn create_lane(
    ctx: &mut DocCtx,
    id: ExpressionLaneId,
    clip: ClipId,
    kind: ExpressionKind,
) -> CmdResult<()> {
    if ctx.p().expression_lanes.contains_key(&id) {
        return Ok(());
    }
    let cl = ctx.clip(clip)?;
    if !matches!(cl.content, ClipContent::Midi) {
        return Err(invalid(format!("clip {clip} is not a MIDI clip")));
    }
    if let ExpressionKind::Cc { controller } = kind
        && controller > MAX_EXPRESSION_CC
    {
        return Err(invalid(format!(
            "CC {controller} can't be an expression lane (0..={MAX_EXPRESSION_CC})"
        )));
    }
    if ctx
        .p()
        .expression_lanes
        .values()
        .any(|l| l.clip == clip && l.kind == kind)
    {
        return Ok(());
    }
    ctx.tx.insert(Entity::ExpressionLane(ExpressionLane {
        id,
        clip,
        kind,
        points: Vec::new(),
    }))
}

fn set_lane_points(
    ctx: &mut DocCtx,
    l: &ExpressionLane,
    points: Vec<ExpressionPoint>,
) -> CmdResult<()> {
    check_points(&points, l.kind.range()).map_err(invalid)?;
    if points == l.points {
        return Ok(());
    }
    ctx.tx.update(EntityUpdate::ExpressionLane {
        id: l.id,
        change: ExpressionLaneChange::Points(points),
    })
}

/// `old` with the points of `[start, end)` replaced by `points` (which must lie inside).
pub(crate) fn replace_range(
    old: &[ExpressionPoint],
    start: Beats,
    end: Beats,
    points: &[ExpressionPoint],
) -> CmdResult<Vec<ExpressionPoint>> {
    let (s, e) = (start.0, end.0);
    if !(s.is_finite() && e.is_finite() && s >= 0.0 && s <= e) {
        return Err(invalid(format!("invalid expression range {s}..{e}")));
    }
    if points.iter().any(|p| !(p.time.0 >= s && p.time.0 < e)) {
        return Err(invalid(format!(
            "replacement points must lie in the range {s}..{e}"
        )));
    }
    let mut out = Vec::with_capacity(old.len() + points.len());
    out.extend(old.iter().filter(|p| p.time.0 < s));
    out.extend_from_slice(points);
    out.extend(old.iter().filter(|p| p.time.0 >= e));
    Ok(out)
}

fn set_note_expression(
    ctx: &mut DocCtx,
    id: NoteExpressionId,
    note: NoteId,
    kind: NoteExpressionKind,
    points: &[ExpressionPoint],
) -> CmdResult<()> {
    if !ctx.p().notes.contains_key(&note) {
        return Err(not_found(format!("note {note}")));
    }
    check_points(points, kind.range()).map_err(invalid)?;
    let existing = ctx
        .p()
        .note_expressions_of(note)
        .into_iter()
        .find(|e| e.kind == kind)
        .cloned();
    match existing {
        Some(e) if points.is_empty() => ctx.tx.remove(EntityKey::NoteExpression(e.id)),
        Some(e) if e.points == points => Ok(()),
        Some(e) => ctx.tx.update(EntityUpdate::NoteExpression {
            id: e.id,
            change: NoteExpressionChange::Points(points.to_vec()),
        }),
        None if points.is_empty() => Ok(()),
        // A retried create whose curve was since removed or moved: nothing to do.
        None if ctx.p().note_expressions.contains_key(&id) => Ok(()),
        None => ctx.tx.insert(Entity::NoteExpression(NoteExpression {
            id,
            note,
            kind,
            points: points.to_vec(),
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(t: f64, v: f32) -> ExpressionPoint {
        ExpressionPoint {
            time: Beats(t),
            value: v,
            curve: CurveShape::Linear,
        }
    }

    #[test]
    fn replace_range_keeps_the_outside() {
        let old = [p(0.0, 0.1), p(1.0, 0.2), p(2.0, 0.3), p(3.0, 0.4)];
        let got = replace_range(&old, Beats(1.0), Beats(3.0), &[p(1.5, 0.9)]).unwrap();
        assert_eq!(got, vec![p(0.0, 0.1), p(1.5, 0.9), p(3.0, 0.4)]);
        // Erase.
        let got = replace_range(&old, Beats(0.0), Beats(2.5), &[]).unwrap();
        assert_eq!(got, vec![p(3.0, 0.4)]);
        // Points must be inside, the range ordered.
        assert!(replace_range(&old, Beats(1.0), Beats(2.0), &[p(2.0, 0.0)]).is_err());
        assert!(replace_range(&old, Beats(2.0), Beats(1.0), &[]).is_err());
        assert!(replace_range(&old, Beats(f64::NAN), Beats(1.0), &[]).is_err());
    }
}

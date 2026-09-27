//! `NoteCommand`s.

use ether_core::protocol::model::*;
use ether_core::protocol::notes::NoteCommand;

use super::DocCtx;
use crate::tx::{CmdResult, invalid, not_found};

const DEFAULT_RELEASE_VELOCITY: f32 = 0.5;

fn note(ctx: &DocCtx, id: NoteId) -> CmdResult<Note> {
    ctx.p()
        .notes
        .get(&id)
        .cloned()
        .ok_or_else(|| not_found(format!("note {id}")))
}

fn set(ctx: &mut DocCtx, id: NoteId, change: NoteChange) -> CmdResult<()> {
    ctx.tx.update(EntityUpdate::Note { id, change })
}

fn check_pitch(p: u8) -> CmdResult<()> {
    if p > 127 {
        return Err(invalid(format!("invalid pitch {p}")));
    }
    Ok(())
}

fn check_beats(what: &str, b: Beats, strictly_positive: bool) -> CmdResult<()> {
    let ok = b.0.is_finite()
        && if strictly_positive {
            b.0 > 0.0
        } else {
            b.0 >= 0.0
        };
    if !ok {
        return Err(invalid(format!("invalid note {what} {}", b.0)));
    }
    Ok(())
}

fn velocity(v: f32) -> CmdResult<f32> {
    if !v.is_finite() {
        return Err(invalid("velocity must be finite"));
    }
    Ok(v.clamp(0.0, 1.0))
}

pub(super) fn apply(ctx: &mut DocCtx, c: &NoteCommand) -> CmdResult<()> {
    match c {
        NoteCommand::Add { clip, notes } => {
            let cl = ctx.clip(*clip)?;
            if !matches!(cl.content, ClipContent::Midi) {
                return Err(invalid(format!("clip {} is not a MIDI clip", cl.id)));
            }
            for n in notes {
                if ctx.p().notes.contains_key(&n.id) {
                    continue;
                }
                check_pitch(n.pitch)?;
                check_beats("start", n.start, false)?;
                check_beats("duration", n.duration, true)?;
                ctx.tx.insert(Entity::Note(Note {
                    id: n.id,
                    clip: cl.id,
                    pitch: n.pitch,
                    velocity: velocity(n.velocity)?,
                    release_velocity: DEFAULT_RELEASE_VELOCITY,
                    start: n.start,
                    duration: n.duration,
                    muted: false,
                }))?;
            }
            Ok(())
        }
        NoteCommand::Remove { ids } => {
            for id in ids {
                note(ctx, *id)?;
                ctx.tx.remove(EntityKey::Note(*id))?;
            }
            Ok(())
        }
        NoteCommand::Edit { edits } => {
            for e in edits {
                let n = note(ctx, e.id)?;
                if let Some(p) = e.pitch {
                    check_pitch(p)?;
                    if p != n.pitch {
                        set(ctx, n.id, NoteChange::Pitch(p))?;
                    }
                }
                if let Some(v) = e.velocity {
                    set(ctx, n.id, NoteChange::Velocity(velocity(v)?))?;
                }
                if let Some(s) = e.start {
                    if !s.0.is_finite() {
                        return Err(invalid("note start must be finite"));
                    }
                    set(ctx, n.id, NoteChange::Start(Beats(s.0.max(0.0))))?;
                }
                if let Some(d) = e.duration {
                    check_beats("duration", d, true)?;
                    set(ctx, n.id, NoteChange::Duration(d))?;
                }
                if let Some(m) = e.muted {
                    set(ctx, n.id, NoteChange::Muted(m))?;
                }
            }
            Ok(())
        }
        NoteCommand::Quantize {
            clip,
            notes,
            grid,
            strength,
            ends,
            swing,
        } => {
            ctx.clip(*clip)?;
            if !(grid.0.is_finite() && grid.0 > 0.0) {
                return Err(invalid("grid must be > 0"));
            }
            let strength = if strength.is_finite() {
                strength.clamp(0.0, 1.0) as f64
            } else {
                1.0
            };
            // Target = nearest grid line, delayed by the quantize swing on odd lines.
            let target = |t: f64| {
                let s = Beats(t).snap(*grid).0;
                s + crate::groove::swing_delay(s, grid.0, *swing)
            };
            let snap = |t: f64| t + (target(t) - t) * strength;
            let targets: Vec<Note> = ctx
                .p()
                .notes_of(*clip)
                .into_iter()
                .filter(|n| notes.as_ref().is_none_or(|ids| ids.contains(&n.id)))
                .cloned()
                .collect();
            for n in targets {
                let start = snap(n.start.0).max(0.0);
                let end = if *ends {
                    snap(n.start.0 + n.duration.0)
                } else {
                    start + n.duration.0
                };
                let duration = (end - start).max(grid.0 * 0.25);
                if !Beats(start).approx_eq(n.start) {
                    set(ctx, n.id, NoteChange::Start(Beats(start)))?;
                }
                if !Beats(duration).approx_eq(n.duration) {
                    set(ctx, n.id, NoteChange::Duration(Beats(duration)))?;
                }
            }
            Ok(())
        }
        NoteCommand::Duplicate {
            copies,
            offset,
            transpose,
        } => {
            if !offset.0.is_finite() {
                return Err(invalid("offset must be finite"));
            }
            for cp in copies {
                if ctx.p().notes.contains_key(&cp.new_id) {
                    continue;
                }
                let mut n = note(ctx, cp.from)?;
                n.id = cp.new_id;
                n.start = Beats((n.start.0 + offset.0).max(0.0));
                n.pitch = (n.pitch as i16 + *transpose as i16).clamp(0, 127) as u8;
                ctx.tx.insert(Entity::Note(n))?;
            }
            Ok(())
        }
    }
}

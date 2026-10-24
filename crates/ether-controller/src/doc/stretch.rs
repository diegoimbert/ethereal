//! `ClipCommand::Stretch`: give a clip new bounds and time-scale its content to fill them
//! (the arrangement's ctrl+shift edge drag), so it plays faster or slower.
//!
//! The factor is `k = new length / old length`, and the clip's whole content timeline
//! scales by it about content 0: the offset (a leading gap too), the loop region, and
//! - **MIDI**: every note's start and duration, its per-note expressions, the clip's
//!   expression lanes;
//! - **audio**: the warp markers' beats (their source seconds stay), so the same audio
//!   spans `k` times as many beats. A clip without its own mapping (unwarped, or warped
//!   from `source_bpm` with fewer than two markers) first gets two markers that pin the
//!   mapping it plays with, and an unwarped clip gets warped (with its mode). The fades
//!   scale too;
//! - both: the clip envelopes (automation lanes owned by the clip).
//!
//! The clip then takes the new start and length, and overlaps on its lane resolve like a
//! resize. One transaction, so one undo step.

use std::collections::BTreeSet;

use ether_core::protocol::model::*;

use super::DocCtx;
use super::clips::{check_start, clip_start, resolve_overlaps, start_change};
use crate::tx::{CmdResult, invalid};
use crate::warp::ContentMap;

/// Most a single stretch can speed up or slow down a clip.
pub const MAX_STRETCH: f64 = 1024.0;

pub(super) fn stretch(ctx: &mut DocCtx, id: ClipId, start: Beats, length: Beats) -> CmdResult<()> {
    let cl = ctx.clip(id)?;
    let start = check_start(start)?;
    if !(length.0.is_finite() && length.0 > 0.0) {
        return Err(invalid("clip length must be > 0"));
    }
    let k = length.0 / cl.length.0;
    if !(k.is_finite() && (1.0 / MAX_STRETCH..=MAX_STRETCH).contains(&k)) {
        return Err(invalid("stretch factor out of range"));
    }
    if (k - 1.0).abs() > 1e-12 {
        scale_content(ctx, &cl, k)?;
    }
    if clip_start(&cl) != start {
        ctx.set_clip(id, start_change(start))?;
    }
    ctx.set_clip(id, ClipChange::Length(length))?;
    resolve_overlaps(ctx, id, &BTreeSet::new())
}

fn scale_content(ctx: &mut DocCtx, cl: &Clip, k: f64) -> CmdResult<()> {
    let id = cl.id;
    ctx.set_clip(id, ClipChange::Offset(Beats(cl.offset.0 * k)))?;
    let lp = cl.looping;
    ctx.set_clip(
        id,
        ClipChange::Loop(ClipLoop {
            start: Beats(lp.start.0 * k),
            end: Beats(lp.end.0 * k),
            ..lp
        }),
    )?;
    match &cl.content {
        ClipContent::Midi => scale_midi(ctx, id, k)?,
        ClipContent::Audio(a) => scale_audio(ctx, cl, a, k)?,
    }
    scale_envelopes(ctx, id, k)
}

fn scaled(points: &[ExpressionPoint], k: f64) -> Vec<ExpressionPoint> {
    points
        .iter()
        .map(|p| ExpressionPoint {
            time: Beats(p.time.0 * k),
            ..*p
        })
        .collect()
}

fn scale_midi(ctx: &mut DocCtx, clip: ClipId, k: f64) -> CmdResult<()> {
    let notes: Vec<Note> = ctx.p().notes_of(clip).into_iter().cloned().collect();
    for n in &notes {
        ctx.tx.update(EntityUpdate::Note {
            id: n.id,
            change: NoteChange::Start(Beats(n.start.0 * k)),
        })?;
        ctx.tx.update(EntityUpdate::Note {
            id: n.id,
            change: NoteChange::Duration(Beats(n.duration.0 * k)),
        })?;
        let exprs: Vec<NoteExpression> = ctx
            .p()
            .note_expressions_of(n.id)
            .into_iter()
            .cloned()
            .collect();
        for e in exprs {
            ctx.tx.update(EntityUpdate::NoteExpression {
                id: e.id,
                change: NoteExpressionChange::Points(scaled(&e.points, k)),
            })?;
        }
    }
    let lanes: Vec<ExpressionLane> = ctx
        .p()
        .expression_lanes_of(clip)
        .into_iter()
        .cloned()
        .collect();
    for l in lanes {
        ctx.tx.update(EntityUpdate::ExpressionLane {
            id: l.id,
            change: ExpressionLaneChange::Points(scaled(&l.points, k)),
        })?;
    }
    Ok(())
}

fn scale_audio(ctx: &mut DocCtx, cl: &Clip, a: &AudioContent, k: f64) -> CmdResult<()> {
    let id = cl.id;
    let own_map = a.warp.enabled
        && crate::warp::warp_desc(ctx.p(), cl, a).is_some()
        && ctx.p().warp_markers_of(id).len() >= 2;
    if !own_map {
        // Pin the mapping the clip plays with now (it is linear: two points define it).
        let before = ContentMap::of(ctx.p(), cl, a);
        let old: Vec<WarpMarkerId> = ctx.p().warp_markers_of(id).iter().map(|m| m.id).collect();
        for m in old {
            ctx.tx.remove(EntityKey::WarpMarker(m))?;
        }
        let b0 = cl.offset.0.max(0.0);
        let b1 = b0 + cl.length.0.max(1.0);
        for beat in [b0, b1] {
            let marker: WarpMarkerId = ctx.new_id();
            ctx.tx.insert(Entity::WarpMarker(WarpMarker {
                id: marker,
                clip: id,
                beat: Beats(beat),
                source: Seconds(before.forward(beat).max(0.0)),
            }))?;
        }
        if !a.warp.enabled {
            let ref_bpm = ctx.p().tempo_map().bpm_at(clip_start(cl));
            let warp = WarpSettings {
                enabled: true,
                source_bpm: a.warp.source_bpm.or(Some(ref_bpm)),
                ..a.warp
            };
            ctx.set_clip(id, ClipChange::Warp(warp))?;
        }
    }
    let markers: Vec<WarpMarker> = ctx.p().warp_markers_of(id).into_iter().cloned().collect();
    for m in markers {
        ctx.tx.update(EntityUpdate::WarpMarker {
            id: m.id,
            change: WarpMarkerChange::Beat(Beats(m.beat.0 * k)),
        })?;
    }
    let len = cl.length.0 * k;
    ctx.set_clip(id, ClipChange::FadeIn(Beats((a.fade_in.0 * k).min(len))))?;
    ctx.set_clip(id, ClipChange::FadeOut(Beats((a.fade_out.0 * k).min(len))))
}

fn scale_envelopes(ctx: &mut DocCtx, clip: ClipId, k: f64) -> CmdResult<()> {
    for lane in ctx.p().lane_ids_owned_by(AutomationOwner::Clip { clip }) {
        let points: Vec<AutomationPoint> = ctx.p().points_of(lane).into_iter().cloned().collect();
        for p in points {
            ctx.tx.update(EntityUpdate::AutomationPoint {
                id: p.id,
                change: AutomationPointChange::Time(Beats(p.time.0 * k)),
            })?;
        }
    }
    Ok(())
}

//! Clip editing: fade curves, reverse, crossfades and arrangement markers (roadmap v2,
//! owned by the `clip-editing` node; see `docs/ROADMAP.md`).
//!
//! Dispatched from `doc::apply` (`MarkerCommand`) and `doc/clips.rs` (the v2
//! `ClipCommand` variants). Overlap/crossfade rules: `ether_model::clip` module docs; the
//! overlap trimming in `doc/clips.rs` keeps crossfade overlaps ([`is_crossfade`]).
//!
//! Every command is one document transaction, so one undo step.

use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::markers::MarkerCommand;
use ether_core::protocol::model::*;

use crate::doc::{DocCtx, clip_start};
use crate::tx::{CmdResult, invalid, invalid_state, not_found};

const EPS: f64 = Beats::EPSILON;

fn audio(c: &Clip) -> CmdResult<&AudioContent> {
    match &c.content {
        ClipContent::Audio(a) => Ok(a),
        ClipContent::Midi => Err(invalid(format!("clip {} is not an audio clip", c.id))),
    }
}

fn check_curve(c: &FadeCurve) -> CmdResult<()> {
    if let FadeCurve::Curve { tension } = c
        && !(tension.is_finite() && (-1.0..=1.0).contains(tension))
    {
        return Err(invalid("curve tension must be in -1..=1"));
    }
    Ok(())
}

/// `true` if `a` and `b` (same track) overlap as a crossfade: the earlier one ends inside
/// the later one and the overlap is covered by both the earlier clip's fade-out and the
/// later clip's fade-in (`ether_model::clip` docs). Audio clips only.
pub(crate) fn is_crossfade(a: &Clip, b: &Clip) -> bool {
    let (first, second) = if clip_start(a).0 <= clip_start(b).0 {
        (a, b)
    } else {
        (b, a)
    };
    let (Ok(fa), Ok(fb)) = (audio(first), audio(second)) else {
        return false;
    };
    let first_end = clip_start(first).0 + first.length.0;
    let second_end = clip_start(second).0 + second.length.0;
    let overlap = first_end - clip_start(second).0;
    overlap > 0.0
        && first_end < second_end + EPS
        && clip_start(first).0 < clip_start(second).0 - EPS
        && overlap <= fa.fade_out.0 + EPS
        && overlap <= fb.fade_in.0 + EPS
}

/// Media seconds played at content beat `c` of an audio clip (the engine's mapping:
/// `ether_core::sched`/`warp`, on the reversed timeline for reversed clips; the reference
/// tempo is the tempo at the clip start). Monotonic in `c`.
fn source_seconds(p: &Project, clip: &Clip, a: &AudioContent, c: f64) -> f64 {
    let ref_bpm = p.tempo_map().bpm_at(clip_start(clip)).max(1e-6);
    let warp = crate::warp::warp_desc(p, clip, a);
    let map = |c: f64| match &warp {
        Some(w) if w.markers.len() >= 2 => {
            let m = &w.markers;
            let i = m.partition_point(|&(b, _)| b <= c).clamp(1, m.len() - 1);
            let (b0, s0) = m[i - 1];
            let (b1, s1) = m[i];
            if b1 - b0 <= 0.0 {
                s0
            } else {
                s0 + (c - b0) * (s1 - s0) / (b1 - b0)
            }
        }
        _ => c * 60.0 / ref_bpm,
    };
    let s = map(c);
    let repitch = warp.as_ref().is_none_or(|w| w.mode == WarpMode::Repitch);
    if !repitch || a.transpose == 0.0 {
        return s;
    }
    let s0 = map(clip.offset.0);
    s0 + (s - s0) * (a.transpose as f64 / 12.0).exp2()
}

/// Largest `x` in `0..=want` with `ok(x)` (assumes `ok` holds at 0 and is monotonic).
fn max_ok(want: f64, ok: impl Fn(f64) -> bool) -> f64 {
    if want <= 0.0 || ok(want) {
        return want.max(0.0);
    }
    let (mut lo, mut hi) = (0.0, want);
    for _ in 0..50 {
        let mid = (lo + hi) / 2.0;
        if ok(mid) { lo = mid } else { hi = mid }
    }
    lo
}

/// How far (beats) clip `c` can extend its end, up to `want`: unlimited when looping, else
/// until the media runs out.
fn tail_room(p: &Project, c: &Clip, want: f64) -> f64 {
    let Ok(a) = audio(c) else { return 0.0 };
    if c.looping.enabled {
        return want;
    }
    let Some(m) = p.media.get(&a.media) else {
        return 0.0;
    };
    let dur = m.frames as f64 / m.sample_rate.max(1) as f64;
    let end = c.offset.0 + c.length.0;
    max_ok(want, |x| source_seconds(p, c, a, end + x) <= dur + 1e-9)
}

/// How far (beats) clip `c` can extend its start earlier, up to `want`: the offset can't go
/// below 0 nor before the media start.
fn head_room(p: &Project, c: &Clip, want: f64) -> f64 {
    let Ok(a) = audio(c) else { return 0.0 };
    let want = want.min(c.offset.0);
    if c.looping.enabled {
        return want.max(0.0);
    }
    max_ok(want, |x| source_seconds(p, c, a, c.offset.0 - x) >= -1e-9)
}

/// `ClipCommand::{SetFadeCurves, SetReversed, Crossfade}`.
pub(crate) fn clip_command(ctx: &mut DocCtx, c: &ClipCommand) -> CmdResult<()> {
    match c {
        ClipCommand::SetFadeCurves {
            id,
            fade_in,
            fade_out,
        } => {
            audio(&ctx.clip(*id)?)?;
            if let Some(f) = fade_in {
                check_curve(f)?;
                ctx.set_clip(*id, ClipChange::FadeInCurve(*f))?;
            }
            if let Some(f) = fade_out {
                check_curve(f)?;
                ctx.set_clip(*id, ClipChange::FadeOutCurve(*f))?;
            }
            Ok(())
        }
        ClipCommand::SetReversed { id, reversed } => {
            audio(&ctx.clip(*id)?)?;
            // Offset, loop and warp markers are on the reversed timeline: toggling doesn't
            // move them (`AudioContent::reversed`).
            ctx.set_clip(*id, ClipChange::Reversed(*reversed))
        }
        ClipCommand::Crossfade {
            first,
            second,
            length,
            curve,
        } => crossfade(ctx, *first, *second, length.0, *curve),
        _ => Err(invalid("not a clip-editing command")),
    }
}

/// Crossfade `first` into `second` over `length` beats centred on their boundary (the
/// middle of their current overlap, or where they touch), extending each clip into the
/// other as far as its source allows (the other side makes up what one side can't).
fn crossfade(
    ctx: &mut DocCtx,
    first: ClipId,
    second: ClipId,
    length: f64,
    curve: FadeCurve,
) -> CmdResult<()> {
    check_curve(&curve)?;
    if !(length.is_finite() && length > 0.0) {
        return Err(invalid("crossfade length must be > 0"));
    }
    if first == second {
        return Err(invalid("cannot crossfade a clip with itself"));
    }
    let a = ctx.clip(first)?;
    let b = ctx.clip(second)?;
    audio(&a)?;
    audio(&b)?;
    if a.track != b.track {
        return Err(invalid("crossfaded clips must be on the same track"));
    }
    let (a_start, b_start) = (clip_start(&a).0, clip_start(&b).0);
    let (a_end, b_end) = (a_start + a.length.0, b_start + b.length.0);
    if !(a_start < b_start - EPS && a_end >= b_start - EPS && a_end < b_end + EPS) {
        return Err(invalid(
            "the first clip must start before the second and end where it starts or inside it",
        ));
    }
    let centre = (b_start + a_end.min(b_end)) / 2.0;
    // Can't extend past the other clip's far edge.
    let half = length / 2.0;
    let p = ctx.p();
    // Wanted moves: A's end to centre + half, B's start to centre - half.
    let want_a = (centre + half - a_end).min(b_end - a_end - EPS).max(0.0);
    let want_b = (b_start - (centre - half))
        .min(b_start - a_start - EPS)
        .max(0.0);
    let mut ext_a = tail_room(p, &a, want_a);
    let mut ext_b = head_room(p, &b, want_b);
    // One side short: the other makes up the difference where it can.
    let missing = (want_a - ext_a) + (want_b - ext_b);
    if missing > EPS {
        let more_a = tail_room(p, &a, (ext_a + missing).min(b_end - a_end - EPS)) - ext_a;
        ext_a += more_a.max(0.0);
        let rest = missing - more_a.max(0.0);
        if rest > EPS {
            let more_b = head_room(p, &b, (ext_b + rest).min(b_start - a_start - EPS)) - ext_b;
            ext_b += more_b.max(0.0);
        }
    }
    // A shrink when the current overlap is longer than the crossfade.
    let shrink_a = (a_end - (centre + half)).max(0.0);
    let shrink_b = ((centre - half) - b_start).max(0.0);
    let new_a_end = a_end + ext_a - shrink_a;
    let new_b_start = b_start - ext_b + shrink_b;
    let overlap = new_a_end - new_b_start;
    if overlap <= EPS {
        return Err(invalid_state(
            "there is no source material left to crossfade these clips",
        ));
    }
    if (new_a_end - a_end).abs() > EPS {
        ctx.set_clip(a.id, ClipChange::Length(Beats(new_a_end - a_start)))?;
    }
    if (new_b_start - b_start).abs() > EPS {
        let delta = b_start - new_b_start;
        ctx.set_clip(b.id, ClipChange::Length(Beats(b.length.0 + delta)))?;
        ctx.set_clip(
            b.id,
            ClipChange::Offset(Beats((b.offset.0 - delta).max(0.0))),
        )?;
        // Same op as `doc/clips.rs::start_change` (not re-exported from `doc`).
        ctx.set_clip(b.id, ClipChange::Start(Beats(new_b_start)))?;
    }
    let fade = Beats(overlap);
    ctx.set_clip(a.id, ClipChange::FadeOut(fade))?;
    ctx.set_clip(a.id, ClipChange::FadeOutCurve(curve))?;
    ctx.set_clip(b.id, ClipChange::FadeIn(fade))?;
    ctx.set_clip(b.id, ClipChange::FadeInCurve(curve))?;
    // Fades never exceed the clip length (`SetFades` clamps the same way).
    let a_len = new_a_end - a_start;
    if audio(&ctx.clip(a.id)?)?.fade_in.0 > a_len - overlap {
        ctx.set_clip(a.id, ClipChange::FadeIn(Beats((a_len - overlap).max(0.0))))?;
    }
    let b_len = b_end - new_b_start;
    if audio(&ctx.clip(b.id)?)?.fade_out.0 > b_len - overlap {
        ctx.set_clip(b.id, ClipChange::FadeOut(Beats((b_len - overlap).max(0.0))))?;
    }
    Ok(())
}

fn check_position(position: Beats) -> CmdResult<Beats> {
    if !(position.0.is_finite() && position.0 >= 0.0) {
        return Err(invalid("marker position must be >= 0"));
    }
    Ok(position)
}

fn marker(ctx: &DocCtx, id: MarkerId) -> CmdResult<Marker> {
    ctx.p()
        .markers
        .get(&id)
        .cloned()
        .ok_or_else(|| not_found(format!("marker {id}")))
}

fn set_marker(ctx: &mut DocCtx, id: MarkerId, change: MarkerChange) -> CmdResult<()> {
    ctx.tx.update(EntityUpdate::Marker { id, change })
}

pub(crate) fn marker_command(ctx: &mut DocCtx, c: &MarkerCommand) -> CmdResult<()> {
    match c {
        MarkerCommand::Add {
            id,
            position,
            name,
            color,
        } => {
            if ctx.p().markers.contains_key(id) {
                return Ok(());
            }
            let position = check_position(*position)?;
            let name = name
                .clone()
                .unwrap_or_else(|| format!("Marker {}", ctx.p().markers.len() + 1));
            ctx.tx.insert(Entity::Marker(Marker {
                id: *id,
                position,
                name,
                color: *color,
            }))
        }
        MarkerCommand::Move { id, position } => {
            marker(ctx, *id)?;
            let position = check_position(*position)?;
            set_marker(ctx, *id, MarkerChange::Position(position))
        }
        MarkerCommand::Rename { id, name } => {
            marker(ctx, *id)?;
            set_marker(ctx, *id, MarkerChange::Name(name.clone()))
        }
        MarkerCommand::SetColor { id, color } => {
            marker(ctx, *id)?;
            set_marker(ctx, *id, MarkerChange::Color(*color))
        }
        MarkerCommand::Remove { ids } => {
            for id in ids {
                marker(ctx, *id)?;
                ctx.tx.remove(EntityKey::Marker(*id))?;
            }
            Ok(())
        }
    }
}

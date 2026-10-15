//! `ClipCommand::SetTranspose`: transposing an audio clip resizes it on the arrangement
//! when the transpose changes its playback speed (base-144).
//!
//! An unwarped clip, or a warped clip in `Repitch` mode, plays its source at
//! `2^(transpose/12)` times the speed around its pivot (`max(offset, 0)`, the engine's
//! `warp::repitch_source_seconds`). Changing the transpose then keeps the same audio in the
//! clip and changes its duration instead, like a tape: every content position `c` past the
//! pivot moves to the content position `c'` that reads the same source second at the new
//! rate. That gives the new clip end (so the new `length`, `length / rate` for an unwarped
//! clip), and the new loop region (it keeps covering the same audio, so a looped clip keeps
//! its number of repetitions). Content before 0 (a leading gap) is silence and keeps its
//! length. Pitch-preserving modes (`Complex`) keep the duration: only the pitch changes.
//!
//! A clip that gets longer never grows over a later clip of its lane: its new end is
//! trimmed at the start of the next clip (a clip that already overlapped the next one, e.g.
//! a crossfade, keeps its old end). Fades are clamped to the new length. The transpose, the
//! length, the loop region and the fades change in one transaction (one undo step).

use ether_core::protocol::model::*;

use crate::doc::{DocCtx, clip_start};
use crate::tx::{CmdResult, invalid};
use crate::warp::ContentMap;

const EPS: f64 = Beats::EPSILON;

/// `true` if transposing clip `c` changes its playback speed (unwarped or `Repitch`; the
/// engine's rule, see `clip_editing::source_seconds`).
fn speed_changes(p: &Project, c: &Clip, a: &AudioContent) -> bool {
    crate::warp::warp_desc(p, c, a).is_none_or(|w| w.mode == WarpMode::Repitch)
}

/// Start of the first clip after `c` on its lane (same track and take lane).
fn next_start(p: &Project, c: &Clip) -> Option<f64> {
    let s = clip_start(c).0;
    p.clips
        .values()
        .filter(|o| o.id != c.id && o.track == c.track && o.lane == c.lane)
        .map(|o| clip_start(o).0)
        .filter(|&os| os > s + EPS)
        .min_by(f64::total_cmp)
}

pub(crate) fn set_transpose(ctx: &mut DocCtx, id: ClipId, semitones: f32) -> CmdResult<()> {
    let cl = ctx.clip(id)?;
    let ClipContent::Audio(a) = &cl.content else {
        return Err(invalid(format!("clip {id} is not an audio clip")));
    };
    if !semitones.is_finite() {
        return Err(invalid("transpose must be finite"));
    }
    let new = semitones.clamp(-48.0, 48.0);
    if new == a.transpose {
        return Ok(());
    }
    ctx.set_clip(id, ClipChange::Transpose(new))?;
    if !speed_changes(ctx.p(), &cl, a) {
        return Ok(());
    }
    let map = ContentMap::of(ctx.p(), &cl, a);
    // Source seconds past the pivot scale by old rate / new rate at the new transpose.
    let k = ((a.transpose - new) as f64 / 12.0).exp2();
    let pivot = cl.offset.0.max(0.0);
    let sp = map.forward(pivot);
    let remap = |c: f64| {
        if c < 0.0 {
            c
        } else {
            map.inverse(sp + (map.forward(c) - sp) * k)
        }
    };

    let old_len = cl.length.0;
    let mut len = remap(cl.offset.0 + old_len) - cl.offset.0;
    if !len.is_finite() {
        return Ok(());
    }
    if len > old_len
        && let Some(next) = next_start(ctx.p(), &cl)
    {
        len = len.min((next - clip_start(&cl).0).max(old_len));
    }
    let len = len.max(EPS * 10.0);
    if (len - old_len).abs() > 1e-12 {
        ctx.set_clip(id, ClipChange::Length(Beats(len)))?;
    }

    let lp = cl.looping;
    let (ls, le) = (remap(lp.start.0).max(0.0), remap(lp.end.0));
    if ls.is_finite() && le.is_finite() && le > ls {
        let looping = ClipLoop {
            start: Beats(ls),
            end: Beats(le),
            ..lp
        };
        if looping != lp {
            ctx.set_clip(id, ClipChange::Loop(looping))?;
        }
    }

    if a.fade_in.0 > len {
        ctx.set_clip(id, ClipChange::FadeIn(Beats(len)))?;
    }
    if a.fade_out.0 > len {
        ctx.set_clip(id, ClipChange::FadeOut(Beats(len)))?;
    }
    Ok(())
}

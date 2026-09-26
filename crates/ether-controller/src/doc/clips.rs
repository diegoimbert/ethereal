//! `ClipCommand`s.
//!
//! Clip positions are read and written ONLY through [`clip_start`], [`arrangement_start`]
//! and [`start_change`] (base-3 flattens `ClipLocation` into `Clip.start`; these three are
//! the only places to adapt). Session clips are not supported by this controller.

use std::collections::BTreeSet;

use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::model::*;

use super::DocCtx;
use crate::tx::{CmdResult, invalid, not_found, unsupported};

const EPS: f64 = Beats::EPSILON;
const MIN_GAIN_DB: f32 = -144.0;
const MAX_GAIN_DB: f32 = 24.0;

/// Timeline start of an arrangement clip (`None` for legacy session clips, which the
/// controller ignores).
pub(crate) fn clip_start(c: &Clip) -> Option<Beats> {
    match c.location {
        ClipLocation::Arrangement { start } => Some(start),
        ClipLocation::Session { .. } => None,
    }
}

/// Timeline start requested by a command.
pub(crate) fn arrangement_start(location: &ClipLocation) -> CmdResult<Beats> {
    match location {
        ClipLocation::Arrangement { start } => {
            if !(start.0.is_finite() && start.0 >= 0.0) {
                return Err(invalid("clip start must be >= 0"));
            }
            Ok(*start)
        }
        ClipLocation::Session { .. } => Err(unsupported("session clips are not supported")),
    }
}

/// The op field that moves a clip to `start`.
pub(crate) fn start_change(start: Beats) -> ClipChange {
    ClipChange::Location(ClipLocation::Arrangement { start })
}

fn set_start(c: &mut Clip, start: Beats) {
    c.location = ClipLocation::Arrangement { start };
}

fn check_fits(track: &Track, content: &ClipContent) -> CmdResult<()> {
    let ok = matches!(
        (content, track.kind),
        (ClipContent::Midi, TrackKind::Midi) | (ClipContent::Audio(_), TrackKind::Audio)
    );
    if !ok {
        return Err(invalid(format!(
            "this clip cannot go on {:?} track {}",
            track.kind, track.id
        )));
    }
    Ok(())
}

fn audio(c: &Clip) -> CmdResult<&AudioContent> {
    match &c.content {
        ClipContent::Audio(a) => Ok(a),
        ClipContent::Midi => Err(invalid(format!("clip {} is not an audio clip", c.id))),
    }
}

fn check_length(length: Beats) -> CmdResult<()> {
    if !(length.0.is_finite() && length.0 > 0.0) {
        return Err(invalid("clip length must be > 0"));
    }
    Ok(())
}

/// Make room for arrangement clip `keep` on its track, Ableton-style: clips it fully covers
/// are deleted, partially covered clips are trimmed, a clip that contains it is split.
fn resolve_overlaps(ctx: &mut DocCtx, keep: ClipId, ignore: &BTreeSet<ClipId>) -> CmdResult<()> {
    let Some(k) = ctx.p().clips.get(&keep).cloned() else {
        return Ok(());
    };
    let Some(s) = clip_start(&k).map(|b| b.0) else {
        return Ok(());
    };
    let e = s + k.length.0;
    let others: Vec<Clip> = ctx
        .p()
        .clips
        .values()
        .filter(|o| o.id != keep && o.track == k.track && !ignore.contains(&o.id))
        .cloned()
        .collect();
    for o in others {
        let Some(os) = clip_start(&o).map(|b| b.0) else {
            continue;
        };
        let oe = os + o.length.0;
        if oe <= s + EPS || os >= e - EPS {
            continue;
        }
        if os >= s - EPS && oe <= e + EPS {
            ctx.delete_clip(o.id)?;
        } else if os < s && oe > e {
            let right: ClipId = ctx.new_id();
            ctx.copy_clip(&o, right, |c| {
                set_start(c, Beats(e));
                c.length = Beats(oe - e);
                c.offset = Beats(o.offset.0 + (e - os));
            })?;
            ctx.set_clip(o.id, ClipChange::Length(Beats(s - os)))?;
        } else if os < s {
            ctx.set_clip(o.id, ClipChange::Length(Beats(s - os)))?;
        } else {
            ctx.set_clip(o.id, ClipChange::Length(Beats(oe - e)))?;
            ctx.set_clip(o.id, ClipChange::Offset(Beats(o.offset.0 + (e - os))))?;
            ctx.set_clip(o.id, start_change(Beats(e)))?;
        }
    }
    Ok(())
}

fn new_clip(id: ClipId, track: TrackId, start: Beats, length: Beats, name: String, content: ClipContent) -> Clip {
    Clip {
        id,
        track,
        location: ClipLocation::Arrangement { start },
        name,
        color: None,
        muted: false,
        length,
        offset: Beats::ZERO,
        looping: ClipLoop {
            enabled: false,
            start: Beats::ZERO,
            end: length,
        },
        launch: LaunchSettings::default(),
        content,
    }
}

fn strip_extension(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 => name[..i].to_string(),
        _ => name.to_string(),
    }
}

pub(super) fn apply(ctx: &mut DocCtx, c: &ClipCommand) -> CmdResult<()> {
    match c {
        ClipCommand::CreateMidi {
            id,
            track,
            location,
            length,
            name,
        } => {
            if ctx.p().clips.contains_key(id) {
                return Ok(());
            }
            let t = ctx.track(*track)?;
            check_fits(&t, &ClipContent::Midi)?;
            let start = arrangement_start(location)?;
            check_length(*length)?;
            let clip = new_clip(*id, t.id, start, *length, name.clone().unwrap_or_default(), ClipContent::Midi);
            ctx.tx.insert(Entity::Clip(clip))?;
            resolve_overlaps(ctx, *id, &BTreeSet::new())
        }
        ClipCommand::CreateAudio {
            id,
            track,
            location,
            media,
        } => {
            if ctx.p().clips.contains_key(id) {
                return Ok(());
            }
            let t = ctx.track(*track)?;
            let m = ctx
                .p()
                .media
                .get(media)
                .cloned()
                .ok_or_else(|| not_found(format!("media {media}")))?;
            let start = arrangement_start(location)?;
            let bpm = ctx.p().tempo_map().bpm_at(start);
            let seconds = m.frames as f64 / m.sample_rate.max(1) as f64;
            let length = Beats((seconds * bpm / 60.0).max(Beats::EPSILON * 10.0));
            let content = ClipContent::Audio(AudioContent {
                media: m.id,
                gain: Decibels::UNITY,
                transpose: 0.0,
                fade_in: Beats::ZERO,
                fade_out: Beats::ZERO,
                warp: WarpSettings {
                    enabled: true,
                    mode: WarpMode::Complex,
                    source_bpm: Some(bpm),
                },
            });
            check_fits(&t, &content)?;
            let clip = new_clip(*id, t.id, start, length, strip_extension(&m.name), content);
            ctx.tx.insert(Entity::Clip(clip))?;
            resolve_overlaps(ctx, *id, &BTreeSet::new())
        }
        ClipCommand::Delete { ids } => {
            for id in ids {
                ctx.clip(*id)?;
                ctx.delete_clip(*id)?;
            }
            Ok(())
        }
        ClipCommand::Move { moves } => {
            let moving: BTreeSet<ClipId> = moves.iter().map(|m| m.id).collect();
            for m in moves {
                let cl = ctx.clip(m.id)?;
                let t = ctx.track(m.track)?;
                check_fits(&t, &cl.content)?;
                let start = arrangement_start(&m.location)?;
                if cl.track != t.id {
                    ctx.set_clip(cl.id, ClipChange::Track(t.id))?;
                }
                if !clip_start(&cl).is_some_and(|s| s == start) {
                    ctx.set_clip(cl.id, start_change(start))?;
                }
            }
            for m in moves {
                resolve_overlaps(ctx, m.id, &moving)?;
            }
            Ok(())
        }
        ClipCommand::SetBounds {
            id,
            location,
            length,
            offset,
        } => {
            let cl = ctx.clip(*id)?;
            check_length(*length)?;
            if !(offset.0.is_finite() && offset.0 >= 0.0) {
                return Err(invalid("clip offset must be >= 0"));
            }
            let start = arrangement_start(location)?;
            if !clip_start(&cl).is_some_and(|s| s == start) {
                ctx.set_clip(cl.id, start_change(start))?;
            }
            ctx.set_clip(cl.id, ClipChange::Length(*length))?;
            ctx.set_clip(cl.id, ClipChange::Offset(*offset))?;
            resolve_overlaps(ctx, cl.id, &BTreeSet::new())
        }
        ClipCommand::Split { id, at, new_id } => {
            if ctx.p().clips.contains_key(new_id) {
                return Ok(());
            }
            let cl = ctx.clip(*id)?;
            let start = clip_start(&cl).ok_or_else(|| unsupported("session clips are not supported"))?;
            let (s, e) = (start.0, start.0 + cl.length.0);
            if !(at.0 > s + EPS && at.0 < e - EPS) {
                return Err(invalid("split point outside the clip"));
            }
            ctx.copy_clip(&cl, *new_id, |c| {
                set_start(c, *at);
                c.length = Beats(e - at.0);
                c.offset = Beats(cl.offset.0 + (at.0 - s));
            })?;
            ctx.set_clip(cl.id, ClipChange::Length(Beats(at.0 - s)))
        }
        ClipCommand::Duplicate {
            id,
            new_id,
            location,
        } => {
            if ctx.p().clips.contains_key(new_id) {
                return Ok(());
            }
            let cl = ctx.clip(*id)?;
            let start = match location {
                Some(l) => arrangement_start(l)?,
                None => {
                    let s = clip_start(&cl).ok_or_else(|| unsupported("session clips are not supported"))?;
                    Beats(s.0 + cl.length.0)
                }
            };
            ctx.copy_clip(&cl, *new_id, |c| set_start(c, start))?;
            resolve_overlaps(ctx, *new_id, &BTreeSet::new())
        }
        ClipCommand::Rename { id, name } => {
            ctx.clip(*id)?;
            ctx.set_clip(*id, ClipChange::Name(name.clone()))
        }
        ClipCommand::SetColor { id, color } => {
            ctx.clip(*id)?;
            ctx.set_clip(*id, ClipChange::Color(*color))
        }
        ClipCommand::SetMuted { ids, muted } => {
            for id in ids {
                ctx.clip(*id)?;
                ctx.set_clip(*id, ClipChange::Muted(*muted))?;
            }
            Ok(())
        }
        ClipCommand::SetLoop { id, looping } => {
            ctx.clip(*id)?;
            if !(looping.start.0 >= 0.0 && looping.end.0 > looping.start.0) {
                return Err(invalid("invalid loop region"));
            }
            ctx.set_clip(*id, ClipChange::Loop(*looping))
        }
        ClipCommand::SetLaunch { id, launch } => {
            ctx.clip(*id)?;
            ctx.set_clip(*id, ClipChange::Launch(*launch))
        }
        ClipCommand::SetGain { id, gain } => {
            audio(&ctx.clip(*id)?)?;
            if !gain.0.is_finite() {
                return Err(invalid("gain must be finite"));
            }
            ctx.set_clip(*id, ClipChange::Gain(Decibels(gain.0.clamp(MIN_GAIN_DB, MAX_GAIN_DB))))
        }
        ClipCommand::SetTranspose { id, semitones } => {
            audio(&ctx.clip(*id)?)?;
            if !semitones.is_finite() {
                return Err(invalid("transpose must be finite"));
            }
            ctx.set_clip(*id, ClipChange::Transpose(semitones.clamp(-48.0, 48.0)))
        }
        ClipCommand::SetFades {
            id,
            fade_in,
            fade_out,
        } => {
            let cl = ctx.clip(*id)?;
            audio(&cl)?;
            let max = cl.length.0;
            let clamp = |b: Beats| {
                if b.0.is_finite() {
                    Ok(Beats(b.0.clamp(0.0, max)))
                } else {
                    Err(invalid("fade lengths must be finite"))
                }
            };
            let (fi, fo) = (clamp(*fade_in)?, clamp(*fade_out)?);
            ctx.set_clip(*id, ClipChange::FadeIn(fi))?;
            ctx.set_clip(*id, ClipChange::FadeOut(fo))
        }
    }
}

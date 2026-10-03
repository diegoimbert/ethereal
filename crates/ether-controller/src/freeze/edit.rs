//! Document edits of freeze, flatten, bounce and consolidate (each runs inside one
//! `edit_with` transaction: one undo step).

use ether_core::protocol::Command;
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::freeze::BounceTarget;
use ether_core::protocol::mixer::MixerCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteEdit, NoteSpec};
use ether_core::protocol::tracks::TrackCommand;
use ether_model::derive_id;

use crate::doc::{self, DocCtx, clip_start};
use crate::tx::{CmdResult, invalid, invalid_state, not_found};

/// Most warp markers a placed render gets (tempo ramps are followed in quarter beats).
const MAX_MARKERS: usize = 4096;
/// Step of the markers inside a tempo ramp.
const RAMP_STEP: f64 = 0.25;

/// Content beats (relative to the clip start) of the markers that pin a render starting at
/// song beat `start` and lasting `length` beats to the tempo map.
pub(crate) fn marker_beats(p: &Project, start: Beats, length: f64) -> Vec<f64> {
    let map = p.tempo_map();
    let end = start.0 + length;
    let mut beats = vec![0.0];
    for (i, pt) in map.tempo.iter().enumerate() {
        let next = map.tempo.get(i + 1).map(|n| n.time.0);
        if pt.time.0 > start.0 && pt.time.0 < end {
            beats.push(pt.time.0 - start.0);
        }
        // Inside a ramp: follow it in small steps.
        if pt.curve == TempoCurve::Linear
            && let Some(n) = next
            && (pt.bpm - map.tempo[i + 1].bpm).abs() > 1e-9
        {
            let (a, b) = (pt.time.0.max(start.0), n.min(end));
            let mut x = (a / RAMP_STEP).floor() * RAMP_STEP + RAMP_STEP;
            while x < b && beats.len() < MAX_MARKERS - 1 {
                beats.push(x - start.0);
                x += RAMP_STEP;
            }
        }
    }
    beats.push(length);
    beats.sort_by(f64::total_cmp);
    beats.dedup_by(|b, a| (*b - *a).abs() < 1e-6);
    beats
}

/// Create audio clip `id` of `media` (a render whose frame 0 sounds at song beat `start`) on
/// `track`, making room for it (overlapped clips are trimmed or removed), and warp it to the
/// tempo map (markers `derive_id(id, i)`, rate 1) so it plays exactly the render.
pub(crate) fn place_render(
    ctx: &mut DocCtx,
    id: ClipId,
    track: TrackId,
    start: Beats,
    media: &MediaRef,
) -> CmdResult<()> {
    let map = ctx.p().tempo_map();
    let s0 = map.beats_to_seconds(start).0;
    let seconds = media.frames as f64 / media.sample_rate.max(1) as f64;
    let length = (map.seconds_to_beats(Seconds(s0 + seconds)).0 - start.0).max(1e-6);
    doc::apply(
        ctx,
        &Command::Clip(ClipCommand::CreateAudio {
            id,
            track,
            start,
            media: media.id,
        }),
    )?;
    doc::apply(
        ctx,
        &Command::Clip(ClipCommand::SetBounds {
            id,
            start,
            length: Beats(length),
            offset: Beats::ZERO,
        }),
    )?;
    let beats = marker_beats(ctx.p(), start, length);
    for (i, b) in beats.into_iter().enumerate() {
        let source = (map.beats_to_seconds(Beats(start.0 + b)).0 - s0).max(0.0);
        ctx.tx.insert(Entity::WarpMarker(WarpMarker {
            id: derive_id(id, i as u32),
            clip: id,
            beat: Beats(b),
            source: Seconds(source),
        }))?;
    }
    ctx.set_clip(
        id,
        ClipChange::Warp(WarpSettings {
            enabled: true,
            mode: WarpMode::Repitch,
            source_bpm: Some(map.bpm_at(start)),
        }),
    )
}

/// Keep the live state of the track's plugins in the document (their nodes are destroyed
/// while frozen; unfreezing re-creates them from it).
fn keep_plugin_states(ctx: &mut DocCtx, track: TrackId) -> CmdResult<()> {
    let plugins: Vec<(DeviceId, PluginInstance)> = ctx
        .p()
        .devices
        .values()
        .filter(|d| d.track == track)
        .filter_map(|d| match &d.kind {
            DeviceKind::Plugin { plugin } => Some((d.id, plugin.clone())),
            DeviceKind::Builtin { .. } => None,
        })
        .collect();
    for (id, mut plugin) in plugins {
        if let Some(state) = ctx.host.plugin_state(id)
            && plugin.state.as_ref() != Some(&state)
        {
            plugin.state = Some(state);
            ctx.set_device(id, DeviceChange::Plugin(plugin))?;
        }
    }
    Ok(())
}

fn live_track(ctx: &DocCtx, id: TrackId) -> CmdResult<Track> {
    ctx.p()
        .tracks
        .get(&id)
        .cloned()
        .ok_or_else(|| invalid_state("the track was deleted during the render"))
}

/// `Freeze` done: the media and the freeze (one undo step).
pub(crate) fn finish_freeze(ctx: &mut DocCtx, track: TrackId, media: MediaRef) -> CmdResult<()> {
    let t = live_track(ctx, track)?;
    if t.freeze.is_some() {
        return Err(invalid_state("the track is already frozen"));
    }
    let id = media.id;
    ctx.tx.insert(Entity::Media(media))?;
    keep_plugin_states(ctx, track)?;
    ctx.set_track(
        track,
        TrackChange::Freeze(Some(TrackFreeze {
            media: id,
            start: Seconds(0.0),
        })),
    )
}

/// Tracks right after `t` among its siblings (for "below").
fn next_sibling(p: &Project, t: &Track) -> Option<TrackId> {
    let mut sibs: Vec<&Track> = p.tracks.values().filter(|x| x.parent == t.parent).collect();
    sibs.sort_by(|a, b| a.order.cmp(&b.order).then(a.id.cmp(&b.id)));
    let i = sibs.iter().position(|x| x.id == t.id)?;
    sibs.get(i + 1).map(|x| x.id)
}

/// Delete everything a flatten replaces: clips (all lanes), comp regions, take lanes and
/// devices of `track`.
fn clear_content(ctx: &mut DocCtx, track: TrackId) -> CmdResult<()> {
    let clips: Vec<ClipId> = ctx
        .p()
        .clips
        .values()
        .filter(|c| c.track == track)
        .map(|c| c.id)
        .collect();
    for c in clips {
        if ctx.p().clips.contains_key(&c) {
            ctx.delete_clip(c)?;
        }
    }
    let regions: Vec<CompRegionId> = ctx
        .p()
        .comp_regions
        .values()
        .filter(|r| r.track == track)
        .map(|r| r.id)
        .collect();
    for r in regions {
        ctx.tx.remove(EntityKey::CompRegion(r))?;
    }
    let lanes: Vec<TakeLaneId> = ctx
        .p()
        .take_lanes
        .values()
        .filter(|l| l.track == track)
        .map(|l| l.id)
        .collect();
    for l in lanes {
        ctx.tx.remove(EntityKey::TakeLane(l))?;
    }
    // Top-level devices first (they cascade to pads and rack chains), then any rest.
    let mut devices: Vec<(bool, DeviceId)> = ctx
        .p()
        .devices
        .values()
        .filter(|d| d.track == track)
        .map(|d| (d.pad.is_some() || d.chain.is_some(), d.id))
        .collect();
    devices.sort();
    for (_, d) in devices {
        if ctx.p().devices.contains_key(&d) {
            ctx.delete_device(d)?;
        }
    }
    Ok(())
}

/// `Flatten`: make the freeze permanent (see `FreezeCommand::Flatten`).
pub(crate) fn flatten(
    ctx: &mut DocCtx,
    track: TrackId,
    clip: ClipId,
    new_track: TrackId,
) -> CmdResult<()> {
    let t = ctx.track(track)?;
    let Some(f) = t.freeze.clone() else {
        return Err(invalid_state(format!("track \"{}\" is not frozen", t.name)));
    };
    let media = ctx
        .p()
        .media
        .get(&f.media)
        .cloned()
        .ok_or_else(|| not_found(format!("media {}", f.media)))?;
    let start = ctx.p().tempo_map().seconds_to_beats(f.start);
    let start = Beats(start.0.max(0.0));
    match t.kind {
        TrackKind::Audio => {
            ctx.set_track(track, TrackChange::Freeze(None))?;
            clear_content(ctx, track)?;
            place_render(ctx, clip, track, start, &media)
        }
        TrackKind::Midi => {
            if ctx.p().tracks.contains_key(&new_track) {
                return Err(invalid(format!("track {new_track} already exists")));
            }
            doc::apply(
                ctx,
                &Command::Track(TrackCommand::Create {
                    id: new_track,
                    kind: TrackKind::Audio,
                    name: Some(t.name.clone()),
                    color: Some(t.color),
                    parent: t.parent,
                    before: Some(t.id),
                }),
            )?;
            for change in [
                TrackChange::Volume(t.mixer.volume),
                TrackChange::Pan(t.mixer.pan),
                TrackChange::Mute(t.mixer.mute),
                TrackChange::Solo(t.mixer.solo),
                TrackChange::Output(t.output.clone()),
            ] {
                ctx.set_track(new_track, change)?;
            }
            if t.vca.is_some() {
                ctx.set_track(new_track, TrackChange::Vca(t.vca))?;
            }
            let mut sends: Vec<TrackSend> = ctx
                .p()
                .sends
                .values()
                .filter(|s| s.from == track)
                .cloned()
                .collect();
            sends.sort_by_key(|s| s.id);
            for (i, s) in sends.into_iter().enumerate() {
                doc::apply(
                    ctx,
                    &Command::Mixer(MixerCommand::CreateSend {
                        id: derive_id(new_track, i as u32),
                        from: new_track,
                        to: s.to,
                        level: s.level,
                        pre_fader: s.pre_fader,
                    }),
                )?;
            }
            place_render(ctx, clip, new_track, start, &media)?;
            doc::apply(ctx, &Command::Track(TrackCommand::Delete { id: track }))?;
            Ok(())
        }
        _ => Err(invalid("only audio and MIDI tracks can be frozen")),
    }
}

/// `Bounce` done.
pub(crate) fn finish_bounce(
    ctx: &mut DocCtx,
    track: TrackId,
    start: Beats,
    target: &BounceTarget,
    media: MediaRef,
) -> CmdResult<()> {
    let t = live_track(ctx, track)?;
    ctx.tx.insert(Entity::Media(media.clone()))?;
    match target {
        BounceTarget::NewTrack {
            track: new_track,
            clip,
        } => {
            let before = next_sibling(ctx.p(), &t);
            doc::apply(
                ctx,
                &Command::Track(TrackCommand::Create {
                    id: *new_track,
                    kind: TrackKind::Audio,
                    name: Some(format!("{} Bounce", t.name)),
                    color: Some(t.color),
                    parent: t.parent,
                    before,
                }),
            )?;
            if t.output != TrackOutput::Default {
                ctx.set_track(*new_track, TrackChange::Output(t.output.clone()))?;
            }
            place_render(ctx, *clip, *new_track, start, &media)?;
            if !t.mixer.mute {
                ctx.set_track(track, TrackChange::Mute(true))?;
            }
            Ok(())
        }
        BounceTarget::InPlace { clip } => place_render(ctx, *clip, track, start, &media),
    }
}

/// A note as it sounds in song time.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PlayedNote {
    pub start: f64,
    pub duration: f64,
    pub pitch: u8,
    pub velocity: f32,
    pub muted: bool,
    /// v0.3 (`midi-expression`): the clip note it plays (its expressions follow it).
    pub source: NoteId,
}

/// The linear pieces of a clip in song time: `(song start, content start, content end)`
/// (the loop unrolled).
fn clip_pieces(c: &Clip) -> Vec<(f64, f64, f64)> {
    let cs = clip_start(c).0;
    let ce = cs + c.length.0;
    let mut pieces = Vec::new();
    let (ls, le) = (c.looping.start.0, c.looping.end.0);
    if c.looping.enabled && le > ls + 1e-9 {
        let mut song = cs;
        let mut content = c.offset.0;
        while song < ce - 1e-9 && pieces.len() < 100_000 {
            let piece_end = if content < le { le } else { f64::INFINITY };
            let len = (piece_end - content).min(ce - song);
            pieces.push((song, content, content + len));
            song += len;
            content = ls;
        }
    } else {
        pieces.push((cs, c.offset.0, c.offset.0 + c.length.0));
    }
    pieces
}

/// The unmuted main-lane MIDI clips of `track` overlapping `[start, end)`.
fn played_clips(p: &Project, track: TrackId, start: f64, end: f64) -> Vec<&Clip> {
    p.clips
        .values()
        .filter(|c| {
            c.track == track
                && c.lane.is_none()
                && !c.muted
                && matches!(c.content, ClipContent::Midi)
                && clip_start(c).0 + c.length.0 > start
                && clip_start(c).0 < end
        })
        .collect()
}

/// Notes of the unmuted main-lane MIDI clips of `track` sounding from `[start, end)`
/// (clip offset and loop applied; notes cut at the clip/loop end and at `end`), sorted by
/// (start, pitch).
pub(crate) fn played_notes(p: &Project, track: TrackId, start: f64, end: f64) -> Vec<PlayedNote> {
    let mut out = Vec::new();
    for c in played_clips(p, track, start, end) {
        let notes = p.notes_of(c.id);
        let pieces = clip_pieces(c);
        for (song, c0, c1) in pieces {
            for n in &notes {
                if n.start.0 < c0 - 1e-9 || n.start.0 >= c1 - 1e-9 {
                    continue;
                }
                let at = song + (n.start.0 - c0);
                if at < start - 1e-9 || at >= end - 1e-9 {
                    continue;
                }
                let dur = n.duration.0.min(c1 - n.start.0).min(end - at);
                if dur <= 1e-9 {
                    continue;
                }
                out.push(PlayedNote {
                    start: at,
                    duration: dur,
                    pitch: n.pitch,
                    velocity: n.velocity,
                    muted: n.muted,
                    source: n.id,
                });
            }
        }
    }
    out.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
    out
}

/// Consolidate the MIDI clips of `track` over `[start, end)` into clip `clip` (notes
/// `derive_id(seed_notes, next..)`). Tracks without clips in the range are left alone.
pub(crate) fn consolidate_midi(
    ctx: &mut DocCtx,
    track: TrackId,
    start: Beats,
    end: Beats,
    clip: ClipId,
    seed_notes: ClipId,
    next: &mut u32,
) -> CmdResult<()> {
    let overlaps = ctx.p().clips.values().any(|c| {
        c.track == track
            && c.lane.is_none()
            && clip_start(c).0 < end.0
            && clip_start(c).0 + c.length.0 > start.0
    });
    if !overlaps {
        return Ok(());
    }
    let notes = played_notes(ctx.p(), track, start.0, end.0);
    // v0.3 (`midi-expression`): read the expression before the new clip replaces the
    // sources.
    let note_exprs: Vec<Vec<NoteExpression>> = notes
        .iter()
        .map(|n| {
            ctx.p()
                .note_expressions_of(n.source)
                .into_iter()
                .cloned()
                .collect()
        })
        .collect();
    let mut pieces: Vec<crate::expression::copy::Piece> =
        played_clips(ctx.p(), track, start.0, end.0)
            .into_iter()
            .flat_map(|c| {
                clip_pieces(c)
                    .into_iter()
                    .filter_map(move |(song, c0, c1)| {
                        // The piece clamped to `[start, end)`.
                        let s0 = song.max(start.0);
                        let s1 = (song + (c1 - c0)).min(end.0);
                        (s1 > s0).then_some(crate::expression::copy::Piece {
                            clip: c.id,
                            at: s0 - start.0,
                            c0: c0 + (s0 - song),
                            c1: c0 + (s1 - song),
                        })
                    })
            })
            .collect();
    pieces.sort_by(|a, b| a.at.total_cmp(&b.at));
    let lanes = crate::expression::copy::unroll_lanes(ctx.p(), &pieces);
    let name = ctx.track(track)?.name;
    doc::apply(
        ctx,
        &Command::Clip(ClipCommand::CreateMidi {
            id: clip,
            track,
            start,
            length: Beats(end.0 - start.0),
            name: Some(name),
        }),
    )?;
    let mut specs = Vec::with_capacity(notes.len());
    let mut specs_ids: Vec<NoteId> = Vec::with_capacity(notes.len());
    let mut muted = Vec::new();
    for n in &notes {
        let id: NoteId = derive_id(seed_notes, *next);
        *next += 1;
        specs_ids.push(id);
        specs.push(NoteSpec {
            id,
            pitch: n.pitch,
            velocity: n.velocity,
            start: Beats(n.start - start.0),
            duration: Beats(n.duration),
        });
        if n.muted {
            muted.push(NoteEdit {
                id,
                pitch: None,
                velocity: None,
                start: None,
                duration: None,
                muted: Some(true),
            });
        }
    }
    if !specs.is_empty() {
        doc::apply(ctx, &Command::Note(NoteCommand::Add { clip, notes: specs }))?;
    }
    if !muted.is_empty() {
        doc::apply(ctx, &Command::Note(NoteCommand::Edit { edits: muted }))?;
    }
    // v0.3 (`midi-expression`): note expressions follow their notes, lanes are unrolled
    // into the new clip's content time (ids derived from the new clip / note).
    for (exprs, &to) in note_exprs.into_iter().zip(&specs_ids) {
        for (k, e) in exprs.into_iter().enumerate() {
            ctx.tx.insert(Entity::NoteExpression(NoteExpression {
                id: derive_id(to, k as u32),
                note: to,
                ..e
            }))?;
        }
    }
    for (k, (kind, points)) in lanes.into_iter().enumerate() {
        ctx.tx.insert(Entity::ExpressionLane(ExpressionLane {
            id: derive_id(clip, 0x4558_0000 + k as u32),
            clip,
            kind,
            points,
        }))?;
    }
    Ok(())
}

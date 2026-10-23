//! Document edits of freeze, flatten, bounce and consolidate (each runs inside one
//! `edit_with` transaction: one undo step).

use ether_core::protocol::Command;
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::devices::DeviceCommand;
use ether_core::protocol::freeze::BounceTarget;
use ether_core::protocol::mixer::MixerCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteEdit, NoteSpec};
use ether_core::protocol::tracks::TrackCommand;
use ether_model::derive_id;

use crate::doc::{self, DocCtx, clip_start};
use crate::expression::copy::LoopJoin;
use crate::tx::{CmdResult, invalid, invalid_state, not_found};

/// Most warp markers a placed render gets (tempo ramps are followed in quarter beats).
const MAX_MARKERS: usize = 4096;
/// Step of the markers inside a tempo ramp.
const RAMP_STEP: f64 = 0.25;

/// Content beats (relative to the clip start) of the markers that pin a render starting at
/// song beat `start` and lasting `length` beats to the tempo map.
pub fn marker_beats(p: &Project, start: Beats, length: f64) -> Vec<f64> {
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
pub fn place_render(
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
            formants: Default::default(),
        }),
    )
}

/// Keep the live state of the track's plugins in the document (their nodes are destroyed
/// while frozen; unfreezing re-creates them from it). Written outside the history first, so
/// undoing the freeze keeps it instead of restoring the last saved state; the step
/// still carries it (a no-op here) so peers in a session get it with the freeze.
fn keep_plugin_states(ctx: &mut DocCtx, track: TrackId) -> CmdResult<()> {
    // A group freeze also silences every descendant (their nodes are destroyed too).
    let mut tracks = ctx.p().descendants(track);
    tracks.push(track);
    let plugins: Vec<DeviceId> = ctx
        .p()
        .devices
        .values()
        .filter(|d| tracks.contains(&d.track) && matches!(d.kind, DeviceKind::Plugin { .. }))
        .map(|d| d.id)
        .collect();
    for id in plugins {
        if ctx.keep_running_plugin_state(id).is_some()
            && let DeviceKind::Plugin { plugin } = ctx.device(id)?.kind
        {
            ctx.set_device(id, DeviceChange::Plugin(plugin))?;
        }
    }
    Ok(())
}

fn live_track(ctx: &DocCtx, id: TrackId) -> CmdResult<Track> {
    ctx.p().tracks.get(&id).cloned().ok_or_else(|| {
        crate::tx::keyed_err(
            ether_core::protocol::ErrorCode::InvalidState,
            crate::text::msg("error.freeze.track_deleted"),
        )
    })
}

/// `Freeze` done: the media and the freeze (one undo step).
pub fn finish_freeze(
    ctx: &mut DocCtx,
    track: TrackId,
    up_to: Option<DeviceId>,
    media: MediaRef,
) -> CmdResult<()> {
    let t = live_track(ctx, track)?;
    if t.freeze.is_some() {
        return Err(crate::tx::keyed_err(
            ether_core::protocol::ErrorCode::InvalidState,
            crate::text::msg("error.freeze.already_frozen"),
        ));
    }
    let id = media.id;
    ctx.tx.insert(Entity::Media(media))?;
    keep_plugin_states(ctx, track)?;
    ctx.set_track(
        track,
        TrackChange::Freeze(Some(TrackFreeze {
            media: id,
            start: Seconds(0.0),
            up_to,
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
/// the frozen devices of `track` (a partial freeze's live tail stays).
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
        .filter(|d| d.track == track && ctx.p().device_frozen(d.id))
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

/// Flatten of a partial freeze: the live tail of `from` (in chain order) moves to the end of
/// `to`'s chain, so the flattened track still sounds the same.
fn move_live_tail(ctx: &mut DocCtx, from: TrackId, to: TrackId) -> CmdResult<()> {
    let tail: Vec<DeviceId> = ctx
        .p()
        .devices_of(from)
        .into_iter()
        .filter(|d| !ctx.p().device_frozen(d.id))
        .map(|d| d.id)
        .collect();
    for id in tail {
        doc::apply(
            ctx,
            &Command::Device(DeviceCommand::Move {
                id,
                track: to,
                before: None,
            }),
        )?;
    }
    Ok(())
}

/// `Flatten`: make the freeze permanent (see `FreezeCommand::Flatten`).
pub fn flatten(
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
            // While still frozen: a partial freeze's live tail stays.
            clear_content(ctx, track)?;
            ctx.set_track(track, TrackChange::Freeze(None))?;
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
            move_live_tail(ctx, track, new_track)?;
            doc::apply(ctx, &Command::Track(TrackCommand::Delete { id: track }))?;
            Ok(())
        }
        TrackKind::Group => flatten_group(ctx, &t, &media, start, clip, new_track),
        _ => Err(invalid("only audio, MIDI and group tracks can be frozen")),
    }
}

/// Flatten Group: the group (and everything inside it) becomes one audio track `new_track`
/// in its place, holding a single clip of the frozen render. The new track takes the group's
/// name, colour, parent, mixer, output and VCA, and copies its sends (ids
/// `derive_id(new_track, i)`). The group, its descendants and what hangs off them are
/// deleted (one undo step); tracks outside routed into the group, or listening to a
/// descendant's sidechain, are reset by the delete (to their default routing / no sidechain).
/// The group's own fader/pan automation lanes are not carried over.
fn flatten_group(
    ctx: &mut DocCtx,
    t: &Track,
    media: &MediaRef,
    start: Beats,
    clip: ClipId,
    new_track: TrackId,
) -> CmdResult<()> {
    if ctx.p().tracks.contains_key(&new_track) {
        return Err(invalid(format!("track {new_track} already exists")));
    }
    let (name, color, parent) = (t.name.clone(), t.color, t.parent);
    doc::apply(
        ctx,
        &Command::Track(TrackCommand::Create {
            id: new_track,
            kind: TrackKind::Audio,
            name: Some(name),
            color: Some(color),
            parent,
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
        .filter(|s| s.from == t.id)
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
    place_render(ctx, clip, new_track, start, media)?;
    move_live_tail(ctx, t.id, new_track)?;
    doc::apply(ctx, &Command::Track(TrackCommand::Delete { id: t.id }))?;
    Ok(())
}

/// `Bounce` done.
pub fn finish_bounce(
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
pub struct PlayedNote {
    pub start: f64,
    pub duration: f64,
    pub pitch: u8,
    pub velocity: f32,
    pub muted: bool,
    /// v0.3: the clip note it plays (its expressions follow it).
    pub source: NoteId,
    /// Held across a loop wrap into the loop's start note (`Clip::loop_join`).
    pub join: Option<LoopJoin>,
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
/// (clip offset and loop applied; notes cut at the clip/loop end and at `end`, a note held
/// across a loop wrap joined to the loop's start note like in the engine), sorted by
/// (start, pitch).
pub fn played_notes(p: &Project, track: TrackId, start: f64, end: f64) -> Vec<PlayedNote> {
    let mut out = Vec::new();
    for c in played_clips(p, track, start, end) {
        let notes = p.notes_of(c.id);
        let pieces = clip_pieces(c);
        let looping = c.played_loop();
        for (song, c0, c1) in pieces {
            for n in &notes {
                if n.start.0 < c0 - 1e-9 || n.start.0 >= c1 - 1e-9 {
                    continue;
                }
                let at = song + (n.start.0 - c0);
                if at < start - 1e-9 || at >= end - 1e-9 {
                    continue;
                }
                let mut dur = n.duration.0.min(c1 - n.start.0);
                let mut join = None;
                if let Some((ls, le)) = looping
                    && !n.muted
                    && (c1 - le).abs() < 1e-9
                    && n.start.0 + n.duration.0 > le + LOOP_JOIN_EPS
                {
                    let wrap = song + (c1 - c0);
                    let joined =
                        c.loop_join(notes.iter().copied(), n.pitch, wrap - clip_start(c).0);
                    if let Some((partner, after)) = joined {
                        dur += after;
                        join = Some(LoopJoin {
                            at: wrap - at,
                            note: partner.id,
                            lead: ls - partner.start.0,
                            period: le - ls,
                        });
                    }
                }
                let dur = dur.min(end - at);
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
                    join: join.filter(|j| j.at < dur - 1e-9),
                });
            }
        }
    }
    out.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
    out
}

/// Consolidate the MIDI clips of `track` over `[start, end)` into clip `clip` (notes
/// `derive_id(seed_notes, next..)`). Tracks without clips in the range are left alone.
pub fn consolidate_midi(
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
    // v0.3: read the expression before the new clip replaces the
    // sources.
    let note_exprs: Vec<Vec<NoteExpression>> = notes
        .iter()
        .map(|n| {
            let exprs: Vec<NoteExpression> = ctx
                .p()
                .note_expressions_of(n.source)
                .into_iter()
                .cloned()
                .collect();
            match n.join {
                // A joined note: its first note's curves until the wrap, then the loop
                // start note's (like the engine plays it).
                Some(j) => crate::expression::copy::join_note_expressions(
                    ctx.p(),
                    &exprs,
                    n.source,
                    j,
                    n.duration,
                ),
                None => exprs,
            }
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
    doc::apply(
        ctx,
        &Command::Clip(ClipCommand::CreateMidi {
            id: clip,
            track,
            start,
            length: Beats(end.0 - start.0),
            // Unnamed: it reads as its track (and follows a rename).
            name: None,
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
    // v0.3: note expressions follow their notes, lanes are unrolled
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ether_core::protocol::devices::DeviceDescriptor;

    use super::*;
    use crate::doc::DocHost;
    use crate::tx::Tx;

    /// The running plugin reports this state.
    struct Live(Base64Bytes);

    impl DocHost for Live {
        fn descriptor(&mut self, _: DeviceId, _: &DeviceKind) -> Option<DeviceDescriptor> {
            None
        }
        fn plugin_state(&mut self, _: DeviceId) -> Option<Base64Bytes> {
            Some(self.0.clone())
        }
        fn instantiate_plugin(
            &mut self,
            _: DeviceId,
            _: &PluginInstance,
        ) -> CmdResult<Option<DeviceDescriptor>> {
            Ok(None)
        }
    }

    fn state(p: &Project, d: DeviceId) -> Option<Base64Bytes> {
        match &p.devices[&d].kind {
            DeviceKind::Plugin { plugin } => plugin.state.clone(),
            other => panic!("{other:?}"),
        }
    }

    /// Undoing a freeze keeps what the plugin was running when it was frozen (the
    /// freeze step used to record the capture, so its undo put the last saved state back).
    #[test]
    fn undoing_a_freeze_keeps_the_running_plugin_state() {
        let mut ids = IdGen::new(7);
        let mut project = Project::new(&mut ids, 0);
        let track = Track {
            freeze: None,
            vca: Default::default(),
            id: ids.next(0),
            kind: TrackKind::Audio,
            name: "Audio".into(),
            color: Color(0x33_6699),
            order: OrderKey::between(None, None),
            parent: None,
            mixer: TrackMixer::default(),
            input: TrackInput::None,
            output: TrackOutput::Default,
            monitor: MonitorMode::default(),
            scale: Default::default(),
            mpe: None,
        };
        let device = Device {
            id: ids.next(0),
            track: track.id,
            order: OrderKey::between(None, None),
            name: "Verb".into(),
            enabled: true,
            kind: DeviceKind::Plugin {
                plugin: PluginInstance {
                    format: PluginFormat::Clap,
                    plugin_id: "com.test.Verb".into(),
                    name: "Verb".into(),
                    vendor: String::new(),
                    version: String::new(),
                    sandboxed: false,
                    always_process: false,
                    state: Some(Base64Bytes(b"saved".to_vec())),
                },
            },
            params: BTreeMap::new(),
            sidechain: None,
            sidechain_tap: None,
            midi_sidechain: None,
            pad: None,
            chain: None,
        };
        let media = MediaRef {
            location: Default::default(),
            id: ids.next(0),
            name: "freeze.wav".into(),
            file: "media/freeze.wav".into(),
            sample_rate: 48_000,
            channels: 2,
            frames: 48_000,
            hash: None,
        };
        let (t, d) = (track.id, device.id);
        for e in [Entity::Track(track), Entity::Device(device)] {
            project.apply(&Op::Insert { entity: e }).unwrap();
        }
        let live = Base64Bytes(b"running".to_vec());
        let mut host = Live(live.clone());
        let mut ctx = DocCtx {
            tx: Tx::new(&mut project),
            ids: &mut ids,
            now: 1,
            position: Beats::ZERO,
            host: &mut host,
            warnings: Vec::new(),
        };
        finish_freeze(&mut ctx, t, None, media).unwrap();
        let ops = ctx.tx.finish();
        // The step still carries the state (peers in a session get it with the freeze).
        assert!(ops.iter().any(|op| matches!(
            op,
            Op::Update {
                update: EntityUpdate::Device {
                    change: DeviceChange::Plugin(_),
                    ..
                }
            }
        )));
        let mut history = History::new(10);
        history
            .commit_with_inverse(
                &mut project,
                Transaction {
                    key: None,
                    label: "Freeze".into(),
                    ops,
                },
                None,
            )
            .unwrap();
        assert!(project.tracks[&t].freeze.is_some());
        assert_eq!(state(&project, d), Some(live.clone()));

        history.undo(&mut project).unwrap();
        assert!(project.tracks[&t].freeze.is_none());
        assert_eq!(
            state(&project, d),
            Some(live.clone()),
            "not the saved state"
        );
        history.redo(&mut project).unwrap();
        assert_eq!(state(&project, d), Some(live));
    }
}

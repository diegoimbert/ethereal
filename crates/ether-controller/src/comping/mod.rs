//! Takes and comping (v0.2, owned by the `comping` node; model `ether_model::take`,
//! protocol `ether_protocol::takes`, CONTRACTS.md §12.2).
//!
//! - [`take_command`]: every `TakeCommand` (document commands, dispatched from
//!   `doc::apply`; one undo step each; `SetComp` is the swipe gesture).
//! - [`comp_pieces`]: the clip pieces a track's comp plays (each region's lane clips trimmed
//!   to the region, boundary crossfades as equal-power fades). One function for playback
//!   ([`comp_clips`], called by `compile.rs`) and `Flatten`, so a flattened comp plays
//!   exactly like the comp did. Take-lane clips are never compiled directly (`compile.rs`
//!   skips `Clip::lane.is_some()`).
//! - [`apply_audition`]: runtime take audition (`Take::Audition`, like drum pad solo: no
//!   ops), applied after compile by `engine.rs`.
//! - Recording (shared touch in `recording/`): loop/punch passes become take lanes + a comp
//!   region per pass ([`begin_takes`], [`add_take`]).
//! - Cascades are done (`doc/mod.rs`): deleting a track removes its comp regions, lane
//!   clips and lanes.
//!
//! # Crossfades
//! Where region `A` ends exactly where region `B` starts, both pieces extend into the other
//! region (source material permitting) so they overlap over `B.crossfade` seconds centred
//! on the boundary, with equal-power fades over the overlap. Outer edges of a run of regions
//! get a fade of `crossfade / 2` inside the region. Where a lane clip starts or ends inside
//! its region, that edge keeps the clip's own fade. MIDI tracks have no crossfades: pieces
//! are cut exactly at region edges (notes starting before a piece don't sound, notes are cut
//! at its end: the engine's clip-window rules).

use std::collections::{BTreeMap, BTreeSet};

use ether_core::RenderGraphDesc;
use ether_core::graph::ClipDesc;
use ether_core::protocol::model::*;
use ether_core::protocol::takes::TakeCommand;

use crate::compile::CompileContext;
use crate::doc::{DocCtx, order_before};
use crate::tx::{CmdResult, invalid, not_found};

const EPS: f64 = Beats::EPSILON;

/// One piece of a track's comp: `[start, end)` of take clip `clip`, played from content
/// position `offset`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CompPiece {
    pub region: CompRegionId,
    /// Index of the piece within its region (timeline order).
    pub index: u32,
    pub clip: ClipId,
    pub start: f64,
    pub end: f64,
    pub offset: f64,
    /// Comp fades (beats, equal power); `None` = the clip's own fade (the clip edge is inside
    /// the region). Always `None` on MIDI tracks.
    pub fade_in: Option<f64>,
    pub fade_out: Option<f64>,
}

/// Content position of `clip` at `d` beats after its start (clip loop unrolled like the
/// engine's `sched::for_each_piece`).
fn content_at(clip: &Clip, d: f64) -> f64 {
    let pos = clip.offset.0 + d;
    let (ls, le) = (clip.looping.start.0, clip.looping.end.0);
    if !clip.looping.enabled || le - ls <= EPS || clip.offset.0 >= le || pos < le {
        return pos;
    }
    ls + (pos - le).rem_euclid(le - ls)
}

/// Beats covered by `seconds` before (`left`) and after (`right`) timeline position `t`.
fn halves(map: &TempoMap, t: f64, seconds: f64) -> (f64, f64) {
    let s = map.beats_to_seconds(Beats(t)).0;
    let left = t - map.seconds_to_beats(Seconds((s - seconds).max(0.0))).0;
    let right = map.seconds_to_beats(Seconds(s + seconds)).0 - t;
    (left.max(0.0), right.max(0.0))
}

/// The clip pieces `track`'s comp plays, in timeline order (see the module docs).
pub(crate) fn comp_pieces(p: &Project, track: TrackId) -> Vec<CompPiece> {
    let Some(t) = p.tracks.get(&track) else {
        return Vec::new();
    };
    let audio = t.kind == TrackKind::Audio;
    let regions = p.comp_of(track);
    if regions.is_empty() {
        return Vec::new();
    }
    let map = p.tempo_map();
    let mut out = Vec::new();
    for (i, r) in regions.iter().enumerate() {
        let prev = i
            .checked_sub(1)
            .map(|j| regions[j])
            .filter(|q| (q.end.0 - r.start.0).abs() <= EPS);
        let next = regions
            .get(i + 1)
            .copied()
            .filter(|q| (q.start.0 - r.end.0).abs() <= EPS);
        // (extension outside the region, where the comp fade ends/starts inside it)
        let (ext_l, fade_in_end, ext_r, fade_out_start) = if audio {
            let (ext_l, fin_end) = match prev {
                Some(_) => {
                    let (l, rr) = halves(&map, r.start.0, r.crossfade.0 / 2.0);
                    (l, r.start.0 + rr)
                }
                None => {
                    let (_, rr) = halves(&map, r.start.0, r.crossfade.0 / 2.0);
                    (0.0, r.start.0 + rr)
                }
            };
            let (ext_r, fout_start) = match next {
                Some(q) => {
                    let (l, rr) = halves(&map, r.end.0, q.crossfade.0 / 2.0);
                    (rr, r.end.0 - l)
                }
                None => {
                    let (l, _) = halves(&map, r.end.0, r.crossfade.0 / 2.0);
                    (0.0, r.end.0 - l)
                }
            };
            (ext_l, fin_end, ext_r, fout_start)
        } else {
            (0.0, r.start.0, 0.0, r.end.0)
        };
        let mut index = 0;
        for c in p.lane_clips_of(r.lane) {
            if c.track != track {
                continue;
            }
            let (cs, ce) = (c.start.0, c.start.0 + c.length.0);
            if ce <= r.start.0 + EPS || cs >= r.end.0 - EPS {
                continue;
            }
            let start = cs.max(r.start.0 - ext_l);
            let end = ce.min(r.end.0 + ext_r);
            if end - start <= EPS {
                continue;
            }
            let len = end - start;
            let mut fade_in =
                (audio && cs <= r.start.0 + EPS).then(|| (fade_in_end - start).max(0.0));
            let mut fade_out =
                (audio && ce >= r.end.0 - EPS).then(|| (end - fade_out_start).max(0.0));
            let total = fade_in.unwrap_or(0.0) + fade_out.unwrap_or(0.0);
            if total > len {
                let k = len / total;
                fade_in = fade_in.map(|f| f * k);
                fade_out = fade_out.map(|f| f * k);
            }
            out.push(CompPiece {
                region: r.id,
                index,
                clip: c.id,
                start,
                end,
                offset: content_at(c, start - cs),
                fade_in,
                fade_out,
            });
            index += 1;
        }
    }
    out.sort_by(|a, b| {
        a.start
            .total_cmp(&b.start)
            .then(a.region.cmp(&b.region))
            .then(a.index.cmp(&b.index))
    });
    out
}

/// `clip` reshaped to `piece` (same id: notes, warp markers and envelopes stay attached).
fn piece_clip(clip: &Clip, piece: &CompPiece) -> Clip {
    let mut c = clip.clone();
    c.start = Beats(piece.start);
    c.length = Beats(piece.end - piece.start);
    c.offset = Beats(piece.offset);
    c.lane = None;
    if let ClipContent::Audio(a) = &mut c.content {
        if let Some(f) = piece.fade_in {
            a.fade_in = Beats(f);
            a.fade_in_curve = FadeCurve::EqualPower;
        }
        if let Some(f) = piece.fade_out {
            a.fade_out = Beats(f);
            a.fade_out_curve = FadeCurve::EqualPower;
        }
        // The clip's own fades never exceed the piece.
        let len = c.length.0;
        a.fade_in = Beats(a.fade_in.0.min(len));
        a.fade_out = Beats(a.fade_out.0.min(len - a.fade_in.0));
    }
    c
}

/// Engine id of a comp piece: unique and stable across recompiles (warp stretchers are
/// keyed by clip id).
fn piece_id(piece: &CompPiece) -> ClipId {
    derive_id(piece.region, piece.index)
}

/// Clip descs of `track`'s comp (see the module docs), sorted by start.
pub(crate) fn comp_clips(p: &Project, ctx: &CompileContext, track: TrackId) -> Vec<ClipDesc> {
    comp_pieces(p, track)
        .iter()
        .filter_map(|piece| {
            let clip = p.clips.get(&piece.clip)?;
            let shaped = piece_clip(clip, piece);
            let mut desc = crate::compile::clip_desc(p, ctx, &shaped, shaped.start);
            desc.id = piece_id(piece);
            Some(desc)
        })
        .collect()
}

/// Take audition (runtime, `Take::Audition`): each auditioned track plays its lane's clips
/// instead of its comp (main-lane clips keep playing). Entries whose lane or track is gone
/// are dropped. Clip envelopes on device params don't follow while auditioning (no device
/// nodes here); volume/pan/send envelopes do.
pub(crate) fn apply_audition(
    desc: &mut RenderGraphDesc,
    project: Option<&Project>,
    audition: &mut BTreeMap<TrackId, TakeLaneId>,
) {
    let Some(p) = project else {
        audition.clear();
        return;
    };
    audition.retain(|t, l| p.take_lanes.get(l).is_some_and(|lane| lane.track == *t));
    if audition.is_empty() {
        return;
    }
    let ctx = CompileContext {
        nodes: &|_| None,
        descriptors: &crate::compile::builtin_descriptors,
        armed: &|_| false,
        version: desc.version,
    };
    for (track, lane) in audition.iter() {
        let Some(t) = desc.tracks.iter_mut().find(|t| t.id == *track) else {
            continue;
        };
        if t.frozen.is_some() {
            continue;
        }
        let comp: BTreeSet<ClipId> = comp_pieces(p, *track).iter().map(piece_id).collect();
        t.clips.retain(|c| !comp.contains(&c.id));
        for c in p.lane_clips_of(*lane) {
            t.clips.push(crate::compile::clip_desc(p, &ctx, c, c.start));
        }
        t.clips
            .sort_by(|a, b| a.start.total_cmp(&b.start).then(a.id.cmp(&b.id)));
    }
}

// ─── Commands ───────────────────────────────────────────────────────────────────────────

fn lane(ctx: &DocCtx, id: TakeLaneId) -> CmdResult<TakeLane> {
    ctx.p()
        .take_lanes
        .get(&id)
        .cloned()
        .ok_or_else(|| not_found(format!("take lane {id}")))
}

fn check_range(start: Beats, end: Beats) -> CmdResult<()> {
    if !(start.0.is_finite() && end.0.is_finite() && start.0 >= 0.0 && end.0 > start.0 + EPS) {
        return Err(invalid("comp range must satisfy 0 <= start < end"));
    }
    Ok(())
}

/// Default name of a new lane of `track`: "Take N", N one past the highest existing number.
pub(crate) fn next_take_name(p: &Project, track: TrackId) -> String {
    let lanes = p.lanes_of(track);
    let n = lanes
        .iter()
        .filter_map(|l| l.name.strip_prefix("Take ")?.trim().parse::<u32>().ok())
        .max()
        .unwrap_or(0)
        .max(lanes.len() as u32);
    format!("Take {}", n + 1)
}

/// Create a lane (idempotent on `id`).
pub(crate) fn create_lane(
    ctx: &mut DocCtx,
    id: TakeLaneId,
    track: TrackId,
    name: Option<String>,
    before: Option<TakeLaneId>,
) -> CmdResult<()> {
    if ctx.p().take_lanes.contains_key(&id) {
        return Ok(());
    }
    let t = ctx.track(track)?;
    if !matches!(t.kind, TrackKind::Audio | TrackKind::Midi) {
        return Err(invalid("take lanes belong to audio or MIDI tracks"));
    }
    let siblings: Vec<(OrderKey, TakeLaneId)> = ctx
        .p()
        .lanes_of(track)
        .iter()
        .map(|l| (l.order.clone(), l.id))
        .collect();
    let order = order_before(&siblings, before)?;
    let name = match name {
        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
        _ => next_take_name(ctx.p(), track),
    };
    ctx.tx.insert(Entity::TakeLane(TakeLane {
        id,
        track,
        order,
        name,
        color: None,
    }))
}

/// Remove the comp of `track` in `[start, end)`: regions are trimmed, split (the right part
/// gets `split_id`) or removed.
fn clear_range(
    ctx: &mut DocCtx,
    track: TrackId,
    start: f64,
    end: f64,
    split_id: CompRegionId,
) -> CmdResult<()> {
    let regions: Vec<CompRegion> = ctx.p().comp_of(track).into_iter().cloned().collect();
    for r in regions {
        let (rs, re) = (r.start.0, r.end.0);
        if re <= start + EPS || rs >= end - EPS {
            continue;
        }
        let keep_left = rs < start - EPS;
        let keep_right = re > end + EPS;
        match (keep_left, keep_right) {
            (false, false) => ctx.tx.remove(EntityKey::CompRegion(r.id))?,
            (true, false) => set_range(ctx, r.id, rs, start)?,
            (false, true) => set_range(ctx, r.id, end, re)?,
            (true, true) => {
                set_range(ctx, r.id, rs, start)?;
                ctx.tx.insert(Entity::CompRegion(CompRegion {
                    id: split_id,
                    start: Beats(end),
                    end: Beats(re),
                    ..r
                }))?;
            }
        }
    }
    Ok(())
}

fn set_range(ctx: &mut DocCtx, id: CompRegionId, start: f64, end: f64) -> CmdResult<()> {
    ctx.tx.update(EntityUpdate::CompRegion {
        id,
        change: CompRegionChange::Range(BeatRange {
            start: Beats(start),
            end: Beats(end),
        }),
    })
}

/// `Take::SetComp`: `[start, end)` of `track` plays from `lane` (idempotent on `id`).
pub(crate) fn set_comp(
    ctx: &mut DocCtx,
    id: CompRegionId,
    split_id: CompRegionId,
    track: TrackId,
    lane_id: TakeLaneId,
    start: Beats,
    end: Beats,
) -> CmdResult<()> {
    if ctx.p().comp_regions.contains_key(&id) {
        return Ok(());
    }
    check_range(start, end)?;
    let l = lane(ctx, lane_id)?;
    if l.track != track {
        return Err(invalid(format!(
            "take lane {lane_id} is not on track {track}"
        )));
    }
    clear_range(ctx, track, start.0, end.0, split_id)?;
    ctx.tx.insert(Entity::CompRegion(CompRegion {
        id,
        track,
        lane: lane_id,
        start,
        end,
        crossfade: DEFAULT_COMP_CROSSFADE,
    }))
}

/// Move the parts of `track`'s main-lane clips inside `[start, end)` onto `lane` (clips
/// crossing an edge are split; the outside parts stay). Returns whether anything moved.
pub(crate) fn move_main_range_to_lane(
    ctx: &mut DocCtx,
    track: TrackId,
    start: f64,
    end: f64,
    lane: TakeLaneId,
) -> CmdResult<bool> {
    let clips: Vec<Clip> = ctx
        .p()
        .arrangement_clips_of(track)
        .into_iter()
        .filter(|c| c.start.0 + c.length.0 > start + EPS && c.start.0 < end - EPS)
        .cloned()
        .collect();
    let same = |t: &AutomationTarget| Some(*t);
    for c in &clips {
        let (cs, ce) = (c.start.0, c.start.0 + c.length.0);
        let inside_start = cs.max(start);
        let inside_end = ce.min(end);
        if cs >= start - EPS && ce <= end + EPS {
            ctx.set_clip(c.id, ClipChange::Lane(Some(lane)))?;
            continue;
        }
        // The inside part becomes a take clip.
        let inside: ClipId = ctx.new_id();
        ctx.copy_clip(
            c,
            inside,
            |n| {
                n.start = Beats(inside_start);
                n.length = Beats(inside_end - inside_start);
                n.offset = Beats(content_at(c, inside_start - cs));
                n.lane = Some(lane);
                trim_fades(n, inside_start > cs + EPS, inside_end < ce - EPS);
            },
            &same,
        )?;
        if ce > end + EPS {
            // Right outside part.
            let right: ClipId = ctx.new_id();
            ctx.copy_clip(
                c,
                right,
                |n| {
                    n.start = Beats(end);
                    n.length = Beats(ce - end);
                    n.offset = Beats(content_at(c, end - cs));
                    trim_fades(n, true, false);
                },
                &same,
            )?;
        }
        if cs < start - EPS {
            ctx.set_clip(c.id, ClipChange::Length(Beats(start - cs)))?;
            if let ClipContent::Audio(a) = &c.content {
                ctx.set_clip(c.id, ClipChange::FadeOut(Beats::ZERO))?;
                if a.fade_in.0 > start - cs {
                    ctx.set_clip(c.id, ClipChange::FadeIn(Beats(start - cs)))?;
                }
            }
        } else {
            // Nothing left of the original on the main lane.
            ctx.delete_clip(c.id)?;
        }
    }
    Ok(!clips.is_empty())
}

// ─── Recording (called from `recording/`) ────────────────────────────────────────────────

/// `true` if `[start, end)` of `track` has main-lane clips or comp regions: a recording
/// there becomes takes.
pub(crate) fn range_is_used(p: &Project, track: TrackId, start: f64, end: f64) -> bool {
    let hits = |s: f64, e: f64| e > start + EPS && s < end - EPS;
    p.arrangement_clips_of(track)
        .iter()
        .any(|c| hits(c.start.0, c.start.0 + c.length.0))
        || p.comp_of(track).iter().any(|r| hits(r.start.0, r.end.0))
}

/// Before recording takes over `[start, end)` of `track`: main-lane material there moves
/// onto a new lane (the first take), selected over the range.
pub(crate) fn begin_takes(ctx: &mut DocCtx, track: TrackId, start: f64, end: f64) -> CmdResult<()> {
    let used = ctx
        .p()
        .arrangement_clips_of(track)
        .iter()
        .any(|c| c.start.0 + c.length.0 > start + EPS && c.start.0 < end - EPS);
    if !used {
        return Ok(());
    }
    let lane: TakeLaneId = ctx.new_id();
    create_lane(ctx, lane, track, None, None)?;
    move_main_range_to_lane(ctx, track, start, end, lane)?;
    let (id, split): (CompRegionId, CompRegionId) = (ctx.new_id(), ctx.new_id());
    set_comp(
        ctx,
        id,
        split,
        track,
        lane,
        Beats(start.max(0.0)),
        Beats(end),
    )
}

/// A recorded pass (`clip`, just created on the main lane): onto a new lane of its track,
/// selected over the clip's range.
pub(crate) fn add_take(ctx: &mut DocCtx, clip: ClipId) -> CmdResult<()> {
    let c = ctx.clip(clip)?;
    let lane: TakeLaneId = ctx.new_id();
    create_lane(ctx, lane, c.track, None, None)?;
    ctx.set_clip(clip, ClipChange::Lane(Some(lane)))?;
    let (id, split): (CompRegionId, CompRegionId) = (ctx.new_id(), ctx.new_id());
    set_comp(
        ctx,
        id,
        split,
        c.track,
        lane,
        c.start,
        Beats(c.start.0 + c.length.0),
    )
}

/// A cut edge has no fade (the neighbour continues the material); kept fades fit the clip.
fn trim_fades(c: &mut Clip, cut_start: bool, cut_end: bool) {
    let len = c.length.0;
    if let ClipContent::Audio(a) = &mut c.content {
        if cut_start {
            a.fade_in = Beats::ZERO;
        }
        if cut_end {
            a.fade_out = Beats::ZERO;
        }
        a.fade_in = Beats(a.fade_in.0.min(len));
        a.fade_out = Beats(a.fade_out.0.min(len - a.fade_in.0));
    }
}

/// `Take::Flatten` (see `ether_protocol::takes`).
fn flatten(
    ctx: &mut DocCtx,
    track: TrackId,
    seed: ClipId,
    seed_notes: ClipId,
    keep_lanes: bool,
) -> CmdResult<()> {
    let t = ctx.track(track)?;
    if !matches!(t.kind, TrackKind::Audio | TrackKind::Midi) {
        return Err(invalid("only audio and MIDI tracks have takes"));
    }
    let pieces = comp_pieces(ctx.p(), track);
    let mut note_index = 0u32;
    for (i, piece) in pieces.iter().enumerate() {
        let src = ctx.clip(piece.clip)?;
        let id: ClipId = derive_id(seed, i as u32);
        if ctx.p().clips.contains_key(&id) {
            return Err(invalid(format!("clip {id} already exists")));
        }
        let shaped = piece_clip(&src, piece);
        ctx.tx.insert(Entity::Clip(Clip { id, ..shaped }))?;
        // Notes whose start plays in the piece (all of them for a looping clip).
        let (c0, c1) = (piece.offset, piece.offset + (piece.end - piece.start));
        let notes: Vec<Note> = ctx
            .p()
            .notes_of(src.id)
            .into_iter()
            .filter(|n| src.looping.enabled || (n.start.0 >= c0 - EPS && n.start.0 < c1 - EPS))
            .cloned()
            .collect();
        for n in notes {
            let nid: NoteId = derive_id(seed_notes, note_index);
            note_index += 1;
            ctx.tx.insert(Entity::Note(Note {
                id: nid,
                clip: id,
                ..n
            }))?;
        }
        // Warp markers and clip envelopes, with ids derived from the new clip's.
        let markers: Vec<WarpMarker> = ctx
            .p()
            .warp_markers_of(src.id)
            .into_iter()
            .cloned()
            .collect();
        for (k, m) in markers.into_iter().enumerate() {
            ctx.tx.insert(Entity::WarpMarker(WarpMarker {
                id: derive_id(id, k as u32),
                clip: id,
                ..m
            }))?;
        }
        let lanes: Vec<AutomationLane> = ctx
            .p()
            .automation_lanes
            .values()
            .filter(|l| matches!(l.owner, AutomationOwner::Clip { clip } if clip == src.id))
            .cloned()
            .collect();
        for (k, l) in lanes.into_iter().enumerate() {
            let lane_id: AutomationLaneId = derive_id(id, 10_000 + k as u32);
            let points: Vec<AutomationPoint> =
                ctx.p().points_of(l.id).into_iter().cloned().collect();
            ctx.tx.insert(Entity::AutomationLane(AutomationLane {
                id: lane_id,
                owner: AutomationOwner::Clip { clip: id },
                ..l
            }))?;
            for (j, pt) in points.into_iter().enumerate() {
                ctx.tx.insert(Entity::AutomationPoint(AutomationPoint {
                    id: derive_id(lane_id, j as u32),
                    lane: lane_id,
                    ..pt
                }))?;
            }
        }
    }
    let regions: Vec<CompRegionId> = ctx.p().comp_of(track).iter().map(|r| r.id).collect();
    for r in regions {
        ctx.tx.remove(EntityKey::CompRegion(r))?;
    }
    if !keep_lanes {
        let lanes: Vec<TakeLaneId> = ctx.p().lanes_of(track).iter().map(|l| l.id).collect();
        for l in lanes {
            remove_lane(ctx, l)?;
        }
    }
    Ok(())
}

fn remove_lane(ctx: &mut DocCtx, id: TakeLaneId) -> CmdResult<()> {
    let regions: Vec<CompRegionId> = ctx
        .p()
        .comp_regions
        .values()
        .filter(|r| r.lane == id)
        .map(|r| r.id)
        .collect();
    for r in regions {
        ctx.tx.remove(EntityKey::CompRegion(r))?;
    }
    let clips: Vec<ClipId> = ctx.p().lane_clips_of(id).iter().map(|c| c.id).collect();
    for c in clips {
        ctx.delete_clip(c)?;
    }
    ctx.tx.remove(EntityKey::TakeLane(id))
}

pub(crate) fn take_command(ctx: &mut DocCtx, command: &TakeCommand) -> CmdResult<()> {
    match command {
        TakeCommand::CreateLane {
            id,
            track,
            name,
            before,
        } => create_lane(ctx, *id, *track, name.clone(), *before),
        TakeCommand::RemoveLane { id } => {
            lane(ctx, *id)?;
            remove_lane(ctx, *id)
        }
        TakeCommand::RenameLane { id, name } => {
            lane(ctx, *id)?;
            let name = name.trim();
            if name.is_empty() {
                return Err(invalid("a take lane needs a name"));
            }
            ctx.tx.update(EntityUpdate::TakeLane {
                id: *id,
                change: TakeLaneChange::Name(name.to_string()),
            })
        }
        TakeCommand::SetLaneColor { id, color } => {
            lane(ctx, *id)?;
            ctx.tx.update(EntityUpdate::TakeLane {
                id: *id,
                change: TakeLaneChange::Color(*color),
            })
        }
        TakeCommand::MoveLane { id, before } => {
            let l = lane(ctx, *id)?;
            if *before == Some(*id) {
                return Ok(());
            }
            let siblings: Vec<(OrderKey, TakeLaneId)> = ctx
                .p()
                .lanes_of(l.track)
                .iter()
                .filter(|s| s.id != *id)
                .map(|s| (s.order.clone(), s.id))
                .collect();
            let order = order_before(&siblings, *before)?;
            ctx.tx.update(EntityUpdate::TakeLane {
                id: *id,
                change: TakeLaneChange::Order(order),
            })
        }
        TakeCommand::MoveToLane { clips, lane: to } => {
            if let Some(to) = to {
                lane(ctx, *to)?;
            }
            for c in clips {
                let clip = ctx.clip(*c)?;
                if clip.lane != *to {
                    ctx.set_clip(*c, ClipChange::Lane(*to))?;
                }
            }
            Ok(())
        }
        TakeCommand::SetComp {
            id,
            split_id,
            track,
            lane,
            start,
            end,
        } => set_comp(ctx, *id, *split_id, *track, *lane, *start, *end),
        TakeCommand::ClearComp {
            track,
            start,
            end,
            split_id,
        } => {
            ctx.track(*track)?;
            check_range(*start, *end)?;
            clear_range(ctx, *track, start.0, end.0, *split_id)
        }
        TakeCommand::SetCrossfade { region, crossfade } => {
            if !ctx.p().comp_regions.contains_key(region) {
                return Err(not_found(format!("comp region {region}")));
            }
            if !(crossfade.0.is_finite() && (0.0..=MAX_COMP_CROSSFADE).contains(&crossfade.0)) {
                return Err(invalid(format!(
                    "comp crossfade must be 0..={MAX_COMP_CROSSFADE} s"
                )));
            }
            ctx.tx.update(EntityUpdate::CompRegion {
                id: *region,
                change: CompRegionChange::Crossfade(*crossfade),
            })
        }
        TakeCommand::Flatten {
            track,
            seed,
            seed_notes,
            keep_lanes,
        } => flatten(ctx, *track, *seed, *seed_notes, *keep_lanes),
        TakeCommand::Audition { track, lane: l } => {
            ctx.track(*track)?;
            if let Some(l) = l
                && lane(ctx, *l)?.track != *track
            {
                return Err(invalid(format!("take lane {l} is not on track {track}")));
            }
            ctx.host.set_audition(*track, *l);
            Ok(())
        }
    }
}

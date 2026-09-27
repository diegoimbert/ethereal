//! Document edits of one track's time line (clips, take clips, comp regions, track
//! automation) and of the song's global time line (markers, tempo, time signatures, loop).
//!
//! New entities get their ids from [`Ids`] (`derive_id(seed, i)`), allocated in a fixed
//! order (tracks in display order, then clips by start/id, then regions, then lanes), so a
//! replay of the same command on the same document mints the same ids.

use ether_core::protocol::model::*;

use super::points::{self, Pt, TIME_EPS};
use crate::doc::DocCtx;
use crate::tx::CmdResult;

pub(super) const EPS: f64 = Beats::EPSILON;

/// Ids of the entities a command creates: `derive_id(seed, 0)`, `derive_id(seed, 1)`, ...
pub(super) struct Ids {
    seed: ClipId,
    next: u32,
}

impl Ids {
    pub fn new(seed: ClipId) -> Self {
        Self { seed, next: 0 }
    }

    pub fn next<I: Id>(&mut self) -> I {
        let id = derive_id(self.seed, self.next);
        self.next += 1;
        id
    }
}

// ─── Clips ──────────────────────────────────────────────────────────────────────────────

fn end_of(c: &Clip) -> f64 {
    c.start.0 + c.length.0
}

/// Every clip of a track (main lane and take lanes), by start then id.
pub(super) fn track_clips(p: &Project, track: TrackId) -> Vec<Clip> {
    let mut v: Vec<Clip> = p
        .clips
        .values()
        .filter(|c| c.track == track)
        .cloned()
        .collect();
    v.sort_by(|a, b| a.start.0.total_cmp(&b.start.0).then(a.id.cmp(&b.id)));
    v
}

/// A clip with its children (notes, warp markers, clip envelopes and their points).
#[derive(Clone, Debug)]
pub(super) struct ClipBundle {
    pub clip: Clip,
    pub notes: Vec<Note>,
    pub warp_markers: Vec<WarpMarker>,
    pub envelopes: Vec<(AutomationLane, Vec<AutomationPoint>)>,
}

pub(super) fn bundle_of(p: &Project, c: &Clip) -> ClipBundle {
    let mut envelopes: Vec<(AutomationLane, Vec<AutomationPoint>)> = p
        .automation_lanes
        .values()
        .filter(|l| matches!(l.owner, AutomationOwner::Clip { clip } if clip == c.id))
        .map(|l| (l.clone(), p.points_of(l.id).into_iter().cloned().collect()))
        .collect();
    envelopes.sort_by_key(|(l, _)| l.id);
    ClipBundle {
        clip: c.clone(),
        notes: p.notes_of(c.id).into_iter().cloned().collect(),
        warp_markers: p.warp_markers_of(c.id).into_iter().cloned().collect(),
        envelopes,
    }
}

/// `c` restricted to the timeline range `[s, e)` (inside the clip): the content keeps
/// playing at the same song positions; fades are kept only on the clip's own edges.
pub(super) fn piece(c: &Clip, s: f64, e: f64) -> Clip {
    let mut out = c.clone();
    let len = e - s;
    out.offset = Beats(c.offset.0 + (s - c.start.0));
    out.start = Beats(s);
    out.length = Beats(len);
    if let ClipContent::Audio(a) = &mut out.content {
        a.fade_in = if (s - c.start.0).abs() <= EPS {
            Beats(a.fade_in.0.min(len))
        } else {
            Beats::ZERO
        };
        a.fade_out = if (e - end_of(c)).abs() <= EPS {
            Beats(a.fade_out.0.min(len))
        } else {
            Beats::ZERO
        };
    }
    out
}

/// Insert a copy of `b` as clip `id` (`b.clip` already has its final position, track and
/// lane); children get new ids. Envelopes whose target `retarget` drops are left out.
pub(super) fn insert_bundle(
    ctx: &mut DocCtx,
    ids: &mut Ids,
    b: &ClipBundle,
    id: ClipId,
    retarget: &dyn Fn(&Project, &AutomationTarget) -> Option<AutomationTarget>,
) -> CmdResult<()> {
    let mut clip = b.clip.clone();
    clip.id = id;
    ctx.tx.insert(Entity::Clip(clip))?;
    for n in &b.notes {
        let mut n = n.clone();
        n.id = ids.next();
        n.clip = id;
        ctx.tx.insert(Entity::Note(n))?;
    }
    for m in &b.warp_markers {
        let mut m = m.clone();
        m.id = ids.next();
        m.clip = id;
        ctx.tx.insert(Entity::WarpMarker(m))?;
    }
    for (lane, pts) in &b.envelopes {
        let Some(target) = retarget(ctx.p(), &lane.target) else {
            continue;
        };
        let lane_id: AutomationLaneId = ids.next();
        ctx.tx.insert(Entity::AutomationLane(AutomationLane {
            id: lane_id,
            owner: AutomationOwner::Clip { clip: id },
            target,
            enabled: lane.enabled,
        }))?;
        for p in pts {
            let mut p = p.clone();
            p.id = ids.next();
            p.lane = lane_id;
            ctx.tx.insert(Entity::AutomationPoint(p))?;
        }
    }
    Ok(())
}

fn same_target(p: &Project, t: &AutomationTarget) -> Option<AutomationTarget> {
    target_exists(p, t).then_some(*t)
}

fn target_exists(p: &Project, t: &AutomationTarget) -> bool {
    match *t {
        AutomationTarget::TrackVolume { track } | AutomationTarget::TrackPan { track } => {
            p.tracks.contains_key(&track)
        }
        AutomationTarget::SendLevel { send } => p.sends.contains_key(&send),
        AutomationTarget::DeviceParam { device, .. } => p.devices.contains_key(&device),
    }
}

/// An automation target of track `from` moved to track `to` (like a clip moved between
/// tracks: volume/pan follow, sends map to `to`'s send to the same return, devices only if
/// they are on `to`). `None` = `to` has no such target.
pub(super) fn retarget(
    p: &Project,
    from: TrackId,
    to: TrackId,
    t: &AutomationTarget,
) -> Option<AutomationTarget> {
    let out = match *t {
        AutomationTarget::TrackVolume { track } if track == from => {
            AutomationTarget::TrackVolume { track: to }
        }
        AutomationTarget::TrackPan { track } if track == from => {
            AutomationTarget::TrackPan { track: to }
        }
        AutomationTarget::SendLevel { send } if from != to => {
            let ret = p.sends.get(&send)?.to;
            let s = p.sends.values().find(|s| s.from == to && s.to == ret)?;
            AutomationTarget::SendLevel { send: s.id }
        }
        AutomationTarget::DeviceParam { device, .. } if from != to => {
            p.devices.get(&device).filter(|d| d.track == to)?;
            *t
        }
        other => other,
    };
    target_exists(p, &out).then_some(out)
}

/// Split every clip of `track` (main and take lanes) crossing `at`; the right parts are
/// new clips. Fades at the cut are dropped (the outer fades stay).
pub(super) fn split(ctx: &mut DocCtx, ids: &mut Ids, track: TrackId, at: f64) -> CmdResult<()> {
    for c in track_clips(ctx.p(), track) {
        let (s, e) = (c.start.0, end_of(&c));
        if !(at > s + EPS && at < e - EPS) {
            continue;
        }
        let mut right = bundle_of(ctx.p(), &c);
        right.clip = piece(&c, at, e);
        let id: ClipId = ids.next();
        insert_bundle(ctx, ids, &right, id, &same_target)?;
        let left = piece(&c, s, at);
        ctx.set_clip(c.id, ClipChange::Length(left.length))?;
        if let (ClipContent::Audio(old), ClipContent::Audio(new)) = (&c.content, &left.content) {
            if old.fade_in != new.fade_in {
                ctx.set_clip(c.id, ClipChange::FadeIn(new.fade_in))?;
            }
            if old.fade_out != new.fade_out {
                ctx.set_clip(c.id, ClipChange::FadeOut(new.fade_out))?;
            }
        }
    }
    Ok(())
}

/// Split at both edges and delete the clips inside `[a, b]`.
fn clear_clips(ctx: &mut DocCtx, ids: &mut Ids, track: TrackId, a: f64, b: f64) -> CmdResult<()> {
    split(ctx, ids, track, a)?;
    split(ctx, ids, track, b)?;
    for c in track_clips(ctx.p(), track) {
        if c.start.0 >= a - EPS && end_of(&c) <= b + EPS {
            ctx.delete_clip(c.id)?;
        }
    }
    Ok(())
}

fn shift_clips(ctx: &mut DocCtx, track: TrackId, from: f64, delta: f64) -> CmdResult<()> {
    for c in track_clips(ctx.p(), track) {
        if c.start.0 >= from - EPS {
            ctx.set_clip(c.id, ClipChange::Start(Beats((c.start.0 + delta).max(0.0))))?;
        }
    }
    Ok(())
}

// ─── Comp regions ───────────────────────────────────────────────────────────────────────

fn track_regions(p: &Project, track: TrackId) -> Vec<CompRegion> {
    let mut v: Vec<CompRegion> = p
        .comp_regions
        .values()
        .filter(|r| r.track == track)
        .cloned()
        .collect();
    v.sort_by(|a, b| a.start.0.total_cmp(&b.start.0).then(a.id.cmp(&b.id)));
    v
}

fn set_range(ctx: &mut DocCtx, r: &CompRegion, s: f64, e: f64) -> CmdResult<()> {
    if (r.start.0 - s).abs() <= TIME_EPS && (r.end.0 - e).abs() <= TIME_EPS {
        return Ok(());
    }
    ctx.tx.update(EntityUpdate::CompRegion {
        id: r.id,
        change: CompRegionChange::Range(BeatRange {
            start: Beats(s),
            end: Beats(e),
        }),
    })
}

/// Regions after [`delete_time`]: cut out `[a, b)`, the rest shifted left. Updates run
/// left to right so regions never overlap on the way (model invariant).
fn delete_time_regions(ctx: &mut DocCtx, track: TrackId, a: f64, b: f64) -> CmdResult<()> {
    let len = b - a;
    let mut moves = Vec::new();
    for r in track_regions(ctx.p(), track) {
        let (s, e) = (r.start.0, r.end.0);
        if e <= a + EPS {
            continue;
        }
        let (ns, ne) = if s >= b - EPS {
            (s - len, e - len)
        } else {
            (s.min(a), if e <= b { a } else { e - len })
        };
        if ne - ns <= EPS {
            ctx.tx.remove(EntityKey::CompRegion(r.id))?;
        } else {
            moves.push((r, ns, ne));
        }
    }
    moves.sort_by(|x, y| x.1.total_cmp(&y.1));
    for (r, s, e) in moves {
        set_range(ctx, &r, s, e)?;
    }
    Ok(())
}

/// Regions after [`insert_time`]: the ones from `at` on move right (right to left); a
/// region crossing `at` is split, its right part is a new region.
fn insert_time_regions(
    ctx: &mut DocCtx,
    ids: &mut Ids,
    track: TrackId,
    at: f64,
    len: f64,
) -> CmdResult<()> {
    let mut regions = track_regions(ctx.p(), track);
    regions.reverse();
    for r in regions {
        let (s, e) = (r.start.0, r.end.0);
        if s >= at - EPS {
            set_range(ctx, &r, s + len, e + len)?;
        } else if e > at + EPS {
            set_range(ctx, &r, s, at)?;
            ctx.tx.insert(Entity::CompRegion(CompRegion {
                id: ids.next(),
                start: Beats(at + len),
                end: Beats(e + len),
                ..r
            }))?;
        }
    }
    Ok(())
}

/// Remove `[a, b)` from the regions without shifting (paste over a range).
fn clear_regions(ctx: &mut DocCtx, ids: &mut Ids, track: TrackId, a: f64, b: f64) -> CmdResult<()> {
    let mut rights = Vec::new();
    for r in track_regions(ctx.p(), track) {
        let (s, e) = (r.start.0, r.end.0);
        if e <= a + EPS || s >= b - EPS {
            continue;
        }
        let keep_left = s < a - EPS;
        let keep_right = e > b + EPS;
        match (keep_left, keep_right) {
            (false, false) => ctx.tx.remove(EntityKey::CompRegion(r.id))?,
            (true, false) => set_range(ctx, &r, s, a)?,
            (false, true) => set_range(ctx, &r, b, e)?,
            (true, true) => {
                set_range(ctx, &r, s, a)?;
                rights.push(CompRegion {
                    id: ids.next(),
                    start: Beats(b),
                    end: Beats(e),
                    ..r
                });
            }
        }
    }
    for r in rights {
        ctx.tx.insert(Entity::CompRegion(r))?;
    }
    Ok(())
}

// ─── Track automation ───────────────────────────────────────────────────────────────────

/// Arrangement automation lanes of a track, by id.
pub(super) fn track_lanes(p: &Project, track: TrackId) -> Vec<AutomationLane> {
    p.automation_lanes
        .values()
        .filter(|l| matches!(l.owner, AutomationOwner::Track { track: t } if t == track))
        .cloned()
        .collect()
}

pub(super) fn lane_points(p: &Project, lane: AutomationLaneId) -> Vec<Pt> {
    p.points_of(lane)
        .into_iter()
        .map(|p| Pt {
            id: Some(p.id),
            time: p.time.0,
            value: p.value,
            curve: p.curve,
        })
        .collect()
}

/// Make lane `lane`'s points equal `new` (in order). Kept ids are updated in place; within
/// a group of same-time points ids must ascend in list order (it decides the jump
/// direction), so such groups get fresh ids unless theirs already do.
pub(super) fn rewrite_lane(
    ctx: &mut DocCtx,
    ids: &mut Ids,
    lane: AutomationLaneId,
    mut new: Vec<Pt>,
) -> CmdResult<()> {
    let old = lane_points(ctx.p(), lane);
    // Same-time groups.
    let mut i = 0;
    while i < new.len() {
        let mut j = i + 1;
        while j < new.len() && (new[j].time - new[i].time).abs() <= TIME_EPS {
            j += 1;
        }
        if j - i > 1 {
            let ascending = new[i..j].windows(2).all(|w| match (w[0].id, w[1].id) {
                (Some(x), Some(y)) => x < y,
                _ => false,
            });
            if !ascending {
                let mut fresh: Vec<AutomationPointId> = (i..j).map(|_| ids.next()).collect();
                fresh.sort();
                for (p, id) in new[i..j].iter_mut().zip(fresh) {
                    p.id = Some(id);
                }
            }
        }
        i = j;
    }
    for p in &mut new {
        if p.id.is_none() {
            p.id = Some(ids.next());
        }
    }
    let kept: std::collections::BTreeSet<AutomationPointId> =
        new.iter().filter_map(|p| p.id).collect();
    for o in &old {
        if let Some(id) = o.id
            && !kept.contains(&id)
        {
            ctx.tx.remove(EntityKey::AutomationPoint(id))?;
        }
    }
    for p in new {
        let id = p.id.expect("assigned above");
        match old.iter().find(|o| o.id == Some(id)) {
            Some(o) => {
                let mut up = |change| ctx.tx.update(EntityUpdate::AutomationPoint { id, change });
                if (o.time - p.time).abs() > TIME_EPS {
                    up(AutomationPointChange::Time(Beats(p.time.max(0.0))))?;
                }
                if o.value != p.value {
                    up(AutomationPointChange::Value(p.value))?;
                }
                if o.curve != p.curve {
                    up(AutomationPointChange::Curve(p.curve))?;
                }
            }
            None => ctx.tx.insert(Entity::AutomationPoint(AutomationPoint {
                id,
                lane,
                time: Beats(p.time.max(0.0)),
                value: p.value,
                curve: p.curve,
            }))?,
        }
    }
    Ok(())
}

fn edit_lanes(
    ctx: &mut DocCtx,
    ids: &mut Ids,
    track: TrackId,
    f: impl Fn(&[Pt]) -> Vec<Pt>,
) -> CmdResult<()> {
    for lane in track_lanes(ctx.p(), track) {
        let pts = lane_points(ctx.p(), lane.id);
        if pts.is_empty() {
            continue;
        }
        let new = f(&pts);
        if new != pts {
            rewrite_lane(ctx, ids, lane.id, new)?;
        }
    }
    Ok(())
}

// ─── Per-track time edits ───────────────────────────────────────────────────────────────

/// Remove `[a, b)` from a track and shift what follows left by `b - a`.
pub(super) fn delete_time(
    ctx: &mut DocCtx,
    ids: &mut Ids,
    track: TrackId,
    a: f64,
    b: f64,
) -> CmdResult<()> {
    clear_clips(ctx, ids, track, a, b)?;
    shift_clips(ctx, track, b, a - b)?;
    delete_time_regions(ctx, track, a, b)?;
    edit_lanes(ctx, ids, track, |pts| points::delete_time(pts, a, b))
}

/// Insert `len` beats at `at` on a track (clips crossing `at` are split).
pub(super) fn insert_time(
    ctx: &mut DocCtx,
    ids: &mut Ids,
    track: TrackId,
    at: f64,
    len: f64,
) -> CmdResult<()> {
    split(ctx, ids, track, at)?;
    shift_clips(ctx, track, at, len)?;
    insert_time_regions(ctx, ids, track, at, len)?;
    edit_lanes(ctx, ids, track, |pts| points::insert_time(pts, at, len))
}

/// Empty `[a, b)` of a track's clips and comp regions (no shift; paste over a range).
pub(super) fn clear_range(
    ctx: &mut DocCtx,
    ids: &mut Ids,
    track: TrackId,
    a: f64,
    b: f64,
) -> CmdResult<()> {
    clear_clips(ctx, ids, track, a, b)?;
    clear_regions(ctx, ids, track, a, b)
}

// ─── Global time line ───────────────────────────────────────────────────────────────────

/// Markers, tempo points, time signatures and the loop region after deleting `[a, b)`.
/// Points inside the range go (never the ones at beat 0); the tempo and signature in effect
/// at `b` continue at `a`.
pub(super) fn global_delete(ctx: &mut DocCtx, ids: &mut Ids, a: f64, b: f64) -> CmdResult<()> {
    let len = b - a;
    let map = ctx.p().tempo_map();
    let (bpm_b, sig_b) = (map.bpm_at(Beats(b)), map.signature_at(Beats(b)));
    let markers: Vec<Marker> = ctx.p().markers.values().cloned().collect();
    for m in markers {
        let t = m.position.0;
        // A marker right at `a` stays (it marks where the material after the cut starts).
        if t > a + EPS && t < b - EPS {
            ctx.tx.remove(EntityKey::Marker(m.id))?;
        } else if t >= b - EPS {
            ctx.tx.update(EntityUpdate::Marker {
                id: m.id,
                change: MarkerChange::Position(Beats((t - len).max(0.0))),
            })?;
        }
    }
    for p in map.tempo.clone() {
        let t = p.time.0;
        if t <= EPS {
            continue;
        }
        if t >= a - EPS && t < b - EPS {
            ctx.tx.remove(EntityKey::TempoPoint(p.id))?;
        } else if t >= b - EPS {
            ctx.tx.update(EntityUpdate::TempoPoint {
                id: p.id,
                change: TempoPointChange::Time(Beats(t - len)),
            })?;
        }
    }
    let now = ctx.p().tempo_map();
    if (now.bpm_at(Beats(a)) - bpm_b).abs() > 1e-9 {
        match now.tempo.iter().find(|p| (p.time.0 - a).abs() <= EPS) {
            Some(p) => ctx.tx.update(EntityUpdate::TempoPoint {
                id: p.id,
                change: TempoPointChange::Bpm(bpm_b),
            })?,
            None => ctx.tx.insert(Entity::TempoPoint(TempoPoint {
                id: ids.next(),
                time: Beats(a),
                bpm: bpm_b,
                curve: TempoCurve::Step,
            }))?,
        }
    }
    for s in map.signatures.clone() {
        let t = s.time.0;
        if t <= EPS {
            continue;
        }
        if t >= a - EPS && t < b - EPS {
            ctx.tx.remove(EntityKey::TimeSignature(s.id))?;
        } else if t >= b - EPS {
            ctx.tx.update(EntityUpdate::TimeSignature {
                id: s.id,
                change: TimeSignatureChange::Time(Beats(t - len)),
            })?;
        }
    }
    let now = ctx.p().tempo_map();
    if now.signature_at(Beats(a)) != sig_b {
        match now.signatures.iter().find(|s| (s.time.0 - a).abs() <= EPS) {
            Some(s) => ctx.tx.update(EntityUpdate::TimeSignature {
                id: s.id,
                change: TimeSignatureChange::Signature(sig_b),
            })?,
            None => ctx.tx.insert(Entity::TimeSignature(TimeSignaturePoint {
                id: ids.next(),
                time: Beats(a),
                signature: sig_b,
            }))?,
        }
    }
    let map_pos = |x: f64| {
        if x < a {
            x
        } else if x < b {
            a
        } else {
            x - len
        }
    };
    set_loop(ctx, map_pos)
}

/// Markers, tempo points, time signatures and the loop region after inserting `len` beats
/// at `at` (everything at or after `at` moves, except the points at beat 0).
pub(super) fn global_insert(ctx: &mut DocCtx, at: f64, len: f64) -> CmdResult<()> {
    let markers: Vec<Marker> = ctx.p().markers.values().cloned().collect();
    for m in markers {
        if m.position.0 >= at - EPS {
            ctx.tx.update(EntityUpdate::Marker {
                id: m.id,
                change: MarkerChange::Position(Beats(m.position.0 + len)),
            })?;
        }
    }
    let map = ctx.p().tempo_map();
    for p in map.tempo {
        if p.time.0 > EPS && p.time.0 >= at - EPS {
            ctx.tx.update(EntityUpdate::TempoPoint {
                id: p.id,
                change: TempoPointChange::Time(Beats(p.time.0 + len)),
            })?;
        }
    }
    for s in map.signatures {
        if s.time.0 > EPS && s.time.0 >= at - EPS {
            ctx.tx.update(EntityUpdate::TimeSignature {
                id: s.id,
                change: TimeSignatureChange::Time(Beats(s.time.0 + len)),
            })?;
        }
    }
    set_loop(ctx, |x| if x < at { x } else { x + len })
}

fn set_loop(ctx: &mut DocCtx, f: impl Fn(f64) -> f64) -> CmdResult<()> {
    let r = ctx.p().settings.loop_region;
    let (s, e) = (f(r.start.0), f(r.end.0));
    if e - s <= EPS || ((s - r.start.0).abs() <= EPS && (e - r.end.0).abs() <= EPS) {
        return Ok(());
    }
    ctx.tx.settings(SettingsChange::LoopRegion(BeatRange {
        start: Beats(s),
        end: Beats(e),
    }))
}

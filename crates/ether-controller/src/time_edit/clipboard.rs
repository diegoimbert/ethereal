//! The time clipboard: a range of tracks' material, relative to the range start.
//!
//! Controller runtime state (not in the document, not undoable). It holds copies, not
//! references, so later edits (or deleting the source) don't change what pastes.

use ether_core::protocol::model::*;

use super::edit::{self, ClipBundle, EPS, Ids};
use super::points::{self, Pt};
use crate::doc::DocCtx;
use crate::tx::CmdResult;

/// Copied material of one track.
#[derive(Clone, Debug)]
pub(crate) struct TrackCopy {
    pub track: TrackId,
    pub kind: TrackKind,
    /// Clip pieces inside the range (main and take lanes), starts relative to the range.
    pub clips: Vec<ClipBundle>,
    /// Comp regions inside the range, relative.
    pub regions: Vec<CompRegion>,
    /// Track automation lanes with points: their target, enabled flag and the range's
    /// points (relative, with the edge values at 0 and `length`).
    pub lanes: Vec<(AutomationTarget, bool, Vec<Pt>)>,
}

#[derive(Clone, Debug)]
pub(crate) struct TimeClipboard {
    pub length: f64,
    pub tracks: Vec<TrackCopy>,
}

/// A copied piece of a (non-looping) MIDI clip keeps only the notes that play in it (start
/// inside its window): a section copy carries the section, not the whole source clip's
/// notes hidden outside the piece (owner report, `section-edit`). A looping clip replays
/// its loop, so it keeps every note.
fn keep_section_notes(b: &mut ClipBundle) {
    let c = &b.clip;
    if !matches!(c.content, ClipContent::Midi) || c.looping.enabled {
        return;
    }
    let (from, to) = (c.offset.0, c.offset.0 + c.length.0);
    b.notes
        .retain(|n| n.start.0 >= from - EPS && n.start.0 < to - EPS);
}

/// Copy `[a, b)` of `tracks` (display order).
pub(super) fn copy(p: &Project, tracks: &[TrackId], a: f64, b: f64) -> TimeClipboard {
    let mut out = Vec::new();
    for &t in tracks {
        let Some(track) = p.tracks.get(&t) else {
            continue;
        };
        let clips = edit::track_clips(p, t)
            .into_iter()
            .filter(|c| c.start.0 < b - EPS && c.start.0 + c.length.0 > a + EPS)
            .map(|c| {
                let mut bundle = edit::bundle_of(p, &c);
                let s = c.start.0.max(a);
                let e = (c.start.0 + c.length.0).min(b);
                bundle.clip = edit::piece(&c, s, e);
                bundle.clip.start = Beats(s - a);
                keep_section_notes(&mut bundle);
                bundle
            })
            .collect();
        let mut regions: Vec<CompRegion> = p
            .comp_regions
            .values()
            .filter(|r| r.track == t && r.start.0 < b - EPS && r.end.0 > a + EPS)
            .map(|r| CompRegion {
                start: Beats(r.start.0.max(a) - a),
                end: Beats(r.end.0.min(b) - a),
                ..r.clone()
            })
            .collect();
        regions.sort_by(|x, y| x.start.0.total_cmp(&y.start.0));
        let mut lanes = Vec::new();
        let mut track_lanes = edit::track_lanes(p, t);
        track_lanes.sort_by_key(|l| l.id);
        for lane in track_lanes {
            if let Some(pts) = points::slice(&edit::lane_points(p, lane.id), a, b) {
                lanes.push((lane.target, lane.enabled, pts));
            }
        }
        out.push(TrackCopy {
            track: t,
            kind: track.kind,
            clips,
            regions,
            lanes,
        });
    }
    TimeClipboard {
        length: b - a,
        tracks: out,
    }
}

/// Paste `cb` at `at`: clipboard track `pairs[i].0` onto track `pairs[i].1`. `insert`
/// shifts the destination tracks' material after `at` right first; otherwise the range is
/// overwritten (clips and comp regions in it are replaced, pasted automation lanes are
/// written over it).
pub(super) fn paste(
    ctx: &mut DocCtx,
    ids: &mut Ids,
    cb: &TimeClipboard,
    at: f64,
    pairs: &[(usize, TrackId)],
    insert: bool,
) -> CmdResult<()> {
    let len = cb.length;
    for &(_, dest) in pairs {
        if insert {
            edit::insert_time(ctx, ids, dest, at, len)?;
        } else {
            edit::clear_range(ctx, ids, dest, at, at + len)?;
        }
    }
    for &(i, dest) in pairs {
        let src = &cb.tracks[i];
        let from = src.track;
        let lane_ok = |p: &Project, lane: TakeLaneId| {
            from == dest && p.take_lanes.get(&lane).is_some_and(|l| l.track == dest)
        };
        for b in &src.clips {
            // Take clips only go back to their own lane (other tracks don't have it).
            if b.clip.lane.is_some_and(|l| !lane_ok(ctx.p(), l)) {
                continue;
            }
            let mut b = b.clone();
            b.clip.start = Beats(b.clip.start.0 + at);
            b.clip.track = dest;
            let id: ClipId = ids.next();
            edit::insert_bundle(ctx, ids, &b, id, &|p, t| edit::retarget(p, from, dest, t))?;
        }
        for r in &src.regions {
            if !lane_ok(ctx.p(), r.lane) {
                continue;
            }
            ctx.tx.insert(Entity::CompRegion(CompRegion {
                id: ids.next(),
                track: dest,
                start: Beats(r.start.0 + at),
                end: Beats(r.end.0 + at),
                ..r.clone()
            }))?;
        }
        for (target, enabled, piece) in &src.lanes {
            let Some(target) = edit::retarget(ctx.p(), from, dest, target) else {
                continue;
            };
            let existing = edit::track_lanes(ctx.p(), dest)
                .into_iter()
                .filter(|l| l.target == target)
                .min_by_key(|l| l.id);
            let lane = match existing {
                Some(l) => l.id,
                None => {
                    let id: AutomationLaneId = ids.next();
                    ctx.tx.insert(Entity::AutomationLane(AutomationLane {
                        id,
                        owner: AutomationOwner::Track { track: dest },
                        target,
                        enabled: *enabled,
                    }))?;
                    id
                }
            };
            let pts = edit::lane_points(ctx.p(), lane);
            let new = points::overwrite(&pts, at, piece, len);
            edit::rewrite_lane(ctx, ids, lane, new)?;
        }
    }
    Ok(())
}

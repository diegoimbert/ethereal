//! Parent → children indexes of a [`Project`] (derived data).
//!
//! The document is normalized: children point to their parent by id, so "the notes of this
//! clip" is a scan of the whole note table. The index keeps, for the big tables, the
//! `(parent, child)` pairs in ordered sets, so the children of one parent are a range
//! query. It is never serialized, ignored by `==`, and kept in sync by every mutation that
//! goes through the model ([`Project::apply`], undo/redo, patch mirroring
//! (`upsert_unchecked` / `remove_unchecked`)).
//!
//! Code that mutates the public tables directly bypasses it. Inserting or removing rows
//! that way is detected (the index sizes no longer match the tables) and the index is
//! simply not used until [`Project::ensure_index`] rebuilds it (`apply` does that first).
//! Changing a parent reference in place (`p.clips.get_mut(..).track = ..`) is not
//! detected: such code must call [`Project::invalidate_index`].

use std::collections::BTreeSet;
use std::ops::RangeInclusive;

use crate::automation::AutomationOwner;
use crate::entity::{Entity, EntityKey};
use crate::ids::*;
use crate::project::Project;

/// The parent link of one indexed entity (`None` for unindexed kinds).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Link {
    None,
    Note(ClipId, NoteId),
    Clip(TrackId, ClipId),
    WarpMarker(ClipId, WarpMarkerId),
    TrackLane(TrackId, AutomationLaneId),
    ClipLane(ClipId, AutomationLaneId),
    Point(AutomationLaneId, AutomationPointId),
    ExpressionLane(ClipId, ExpressionLaneId),
    NoteExpression(NoteId, NoteExpressionId),
}

impl Link {
    pub(crate) fn of(e: &Entity) -> Link {
        match e {
            Entity::Note(n) => Link::Note(n.clip, n.id),
            Entity::Clip(c) => Link::Clip(c.track, c.id),
            Entity::WarpMarker(m) => Link::WarpMarker(m.clip, m.id),
            Entity::AutomationLane(l) => match l.owner {
                AutomationOwner::Track { track } => Link::TrackLane(track, l.id),
                AutomationOwner::Clip { clip } => Link::ClipLane(clip, l.id),
            },
            Entity::AutomationPoint(p) => Link::Point(p.lane, p.id),
            Entity::ExpressionLane(l) => Link::ExpressionLane(l.clip, l.id),
            Entity::NoteExpression(e) => Link::NoteExpression(e.note, e.id),
            _ => Link::None,
        }
    }

    /// The link of the entity `key` as currently stored in `p` (`None` if absent or not
    /// indexed).
    pub(crate) fn current(p: &Project, key: EntityKey) -> Link {
        match key {
            EntityKey::Note(id) => p
                .notes
                .get(&id)
                .map_or(Link::None, |n| Link::Note(n.clip, id)),
            EntityKey::Clip(id) => p
                .clips
                .get(&id)
                .map_or(Link::None, |c| Link::Clip(c.track, id)),
            EntityKey::WarpMarker(id) => p
                .warp_markers
                .get(&id)
                .map_or(Link::None, |m| Link::WarpMarker(m.clip, id)),
            EntityKey::AutomationLane(id) => {
                p.automation_lanes
                    .get(&id)
                    .map_or(Link::None, |l| match l.owner {
                        AutomationOwner::Track { track } => Link::TrackLane(track, id),
                        AutomationOwner::Clip { clip } => Link::ClipLane(clip, id),
                    })
            }
            EntityKey::AutomationPoint(id) => p
                .automation_points
                .get(&id)
                .map_or(Link::None, |pt| Link::Point(pt.lane, id)),
            EntityKey::ExpressionLane(id) => p
                .expression_lanes
                .get(&id)
                .map_or(Link::None, |l| Link::ExpressionLane(l.clip, id)),
            EntityKey::NoteExpression(id) => p
                .note_expressions
                .get(&id)
                .map_or(Link::None, |e| Link::NoteExpression(e.note, id)),
            _ => Link::None,
        }
    }
}

/// See the module docs.
#[derive(Clone, Default)]
pub(crate) struct ProjectIndex {
    built: bool,
    notes: BTreeSet<(ClipId, NoteId)>,
    clips: BTreeSet<(TrackId, ClipId)>,
    warp_markers: BTreeSet<(ClipId, WarpMarkerId)>,
    track_lanes: BTreeSet<(TrackId, AutomationLaneId)>,
    clip_lanes: BTreeSet<(ClipId, AutomationLaneId)>,
    points: BTreeSet<(AutomationLaneId, AutomationPointId)>,
    expression_lanes: BTreeSet<(ClipId, ExpressionLaneId)>,
    note_expressions: BTreeSet<(NoteId, NoteExpressionId)>,
}

/// Derived data: two projects with the same tables are equal whatever their indexes.
impl PartialEq for ProjectIndex {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for ProjectIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProjectIndex { .. }")
    }
}

/// All children of `parent` in a `(parent, child)` set.
fn range<P: Id, C: Id>(parent: P) -> RangeInclusive<(P, C)> {
    (parent, C::from_ulid(Ulid(0)))..=(parent, C::from_ulid(Ulid(u128::MAX)))
}

fn children<P: Id, C: Id>(
    set: &BTreeSet<(P, C)>,
    parent: P,
) -> impl DoubleEndedIterator<Item = C> + '_ {
    set.range(range::<P, C>(parent)).map(|(_, c)| *c)
}

impl ProjectIndex {
    pub(crate) fn build(p: &Project) -> Self {
        let mut ix = ProjectIndex {
            built: true,
            notes: p.notes.values().map(|n| (n.clip, n.id)).collect(),
            clips: p.clips.values().map(|c| (c.track, c.id)).collect(),
            warp_markers: p.warp_markers.values().map(|m| (m.clip, m.id)).collect(),
            points: p
                .automation_points
                .values()
                .map(|pt| (pt.lane, pt.id))
                .collect(),
            expression_lanes: p
                .expression_lanes
                .values()
                .map(|l| (l.clip, l.id))
                .collect(),
            note_expressions: p
                .note_expressions
                .values()
                .map(|e| (e.note, e.id))
                .collect(),
            ..Default::default()
        };
        for l in p.automation_lanes.values() {
            ix.add(match l.owner {
                AutomationOwner::Track { track } => Link::TrackLane(track, l.id),
                AutomationOwner::Clip { clip } => Link::ClipLane(clip, l.id),
            });
        }
        ix
    }

    /// Built, and every indexed table has as many rows as the index (rows inserted or
    /// removed behind the model's back make it unusable until rebuilt).
    pub(crate) fn valid_for(&self, p: &Project) -> bool {
        self.built
            && self.notes.len() == p.notes.len()
            && self.clips.len() == p.clips.len()
            && self.warp_markers.len() == p.warp_markers.len()
            && self.track_lanes.len() + self.clip_lanes.len() == p.automation_lanes.len()
            && self.points.len() == p.automation_points.len()
            && self.expression_lanes.len() == p.expression_lanes.len()
            && self.note_expressions.len() == p.note_expressions.len()
    }

    pub(crate) fn invalidate(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn add(&mut self, link: Link) {
        match link {
            Link::None => {}
            Link::Note(p, c) => {
                self.notes.insert((p, c));
            }
            Link::Clip(p, c) => {
                self.clips.insert((p, c));
            }
            Link::WarpMarker(p, c) => {
                self.warp_markers.insert((p, c));
            }
            Link::TrackLane(p, c) => {
                self.track_lanes.insert((p, c));
            }
            Link::ClipLane(p, c) => {
                self.clip_lanes.insert((p, c));
            }
            Link::Point(p, c) => {
                self.points.insert((p, c));
            }
            Link::ExpressionLane(p, c) => {
                self.expression_lanes.insert((p, c));
            }
            Link::NoteExpression(p, c) => {
                self.note_expressions.insert((p, c));
            }
        }
    }

    pub(crate) fn remove(&mut self, link: Link) {
        match link {
            Link::None => {}
            Link::Note(p, c) => {
                self.notes.remove(&(p, c));
            }
            Link::Clip(p, c) => {
                self.clips.remove(&(p, c));
            }
            Link::WarpMarker(p, c) => {
                self.warp_markers.remove(&(p, c));
            }
            Link::TrackLane(p, c) => {
                self.track_lanes.remove(&(p, c));
            }
            Link::ClipLane(p, c) => {
                self.clip_lanes.remove(&(p, c));
            }
            Link::Point(p, c) => {
                self.points.remove(&(p, c));
            }
            Link::ExpressionLane(p, c) => {
                self.expression_lanes.remove(&(p, c));
            }
            Link::NoteExpression(p, c) => {
                self.note_expressions.remove(&(p, c));
            }
        }
    }

    /// `old` → `new` (either may be [`Link::None`]).
    pub(crate) fn relink(&mut self, old: Link, new: Link) {
        if old != new {
            self.remove(old);
            self.add(new);
        }
    }

    /// Same pairs as a fresh build (tests).
    pub(crate) fn same_as(&self, other: &Self) -> bool {
        self.built == other.built
            && self.notes == other.notes
            && self.clips == other.clips
            && self.warp_markers == other.warp_markers
            && self.track_lanes == other.track_lanes
            && self.clip_lanes == other.clip_lanes
            && self.points == other.points
            && self.expression_lanes == other.expression_lanes
            && self.note_expressions == other.note_expressions
    }

    pub(crate) fn notes_of(&self, clip: ClipId) -> impl DoubleEndedIterator<Item = NoteId> + '_ {
        children(&self.notes, clip)
    }

    pub(crate) fn clips_of(&self, track: TrackId) -> impl DoubleEndedIterator<Item = ClipId> + '_ {
        children(&self.clips, track)
    }

    pub(crate) fn warp_markers_of(
        &self,
        clip: ClipId,
    ) -> impl DoubleEndedIterator<Item = WarpMarkerId> + '_ {
        children(&self.warp_markers, clip)
    }

    pub(crate) fn track_lanes_of(
        &self,
        track: TrackId,
    ) -> impl DoubleEndedIterator<Item = AutomationLaneId> + '_ {
        children(&self.track_lanes, track)
    }

    pub(crate) fn clip_lanes_of(
        &self,
        clip: ClipId,
    ) -> impl DoubleEndedIterator<Item = AutomationLaneId> + '_ {
        children(&self.clip_lanes, clip)
    }

    pub(crate) fn points_of(
        &self,
        lane: AutomationLaneId,
    ) -> impl DoubleEndedIterator<Item = AutomationPointId> + '_ {
        children(&self.points, lane)
    }

    pub(crate) fn expression_lanes_of(
        &self,
        clip: ClipId,
    ) -> impl DoubleEndedIterator<Item = ExpressionLaneId> + '_ {
        children(&self.expression_lanes, clip)
    }

    pub(crate) fn note_expressions_of(
        &self,
        note: NoteId,
    ) -> impl DoubleEndedIterator<Item = NoteExpressionId> + '_ {
        children(&self.note_expressions, note)
    }
}

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
//!
//! A second, reverse index answers "who references this entity" ([`Refs`]): what a track,
//! device, send, drum pad, rack chain, modulator or take lane is referenced by (routing,
//! sidechains, chains, mappings, automation targets...). It makes the removal check
//! ([`Project::first_dependent`]) and the device rules independent of the project's size.
//! The same rule applies to it: a reference changed in place (`device.chain = None`,
//! `track.output = ..`) needs [`Project::invalidate_index`]. Its answers are re-checked
//! against the tables, so a stale entry is never reported as a dependent.

use std::collections::{BTreeSet, HashMap};
use std::ops::RangeInclusive;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::automation::{AutomationOwner, AutomationTarget};
use crate::device::SidechainChain;
use crate::entity::{Entity, EntityKey};
use crate::ids::*;
use crate::midi_map::MidiMapTarget;
use crate::modulation::ModSource;
use crate::project::Project;
use crate::track::{TrackInput, TrackOutput};
use crate::*;

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

/// The most keys one entity references in the dependents index (a device: its track,
/// sidechain, pad, chain and sidechain tap).
const MAX_REFS: usize = 5;

/// The entities one entity references, for the dependents index (`(target, from)` pairs).
/// Only the references [`Project::first_dependent`] treats as dependencies of the target
/// kinds it answers from this index (tracks, devices, sends, drum pads, rack chains,
/// modulators, take lanes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Refs {
    from: EntityKey,
    to: [Option<EntityKey>; MAX_REFS],
}

impl Refs {
    fn new(from: EntityKey) -> Self {
        Refs {
            from,
            to: [None; MAX_REFS],
        }
    }

    fn push(&mut self, key: Option<EntityKey>) {
        let Some(key) = key else { return };
        if self.to.contains(&Some(key)) {
            return;
        }
        let slot = self
            .to
            .iter_mut()
            .find(|s| s.is_none())
            .expect("an entity references at most MAX_REFS keys");
        *slot = Some(key);
    }

    fn targets(&self) -> impl Iterator<Item = EntityKey> + '_ {
        self.to.iter().flatten().copied()
    }

    pub(crate) fn from(&self) -> EntityKey {
        self.from
    }

    /// `from` references `key`.
    pub(crate) fn contains(&self, key: EntityKey) -> bool {
        self.to.contains(&Some(key))
    }

    pub(crate) fn track(t: &crate::track::Track) -> Self {
        let mut r = Refs::new(EntityKey::Track(t.id));
        r.push(t.parent.map(EntityKey::Track));
        if let TrackInput::Track { track, .. } = t.input {
            r.push(Some(EntityKey::Track(track)));
        }
        r.push(t.vca.map(EntityKey::Track));
        if let TrackOutput::Track { track } = t.output {
            r.push(Some(EntityKey::Track(track)));
        }
        r
    }

    pub(crate) fn clip(c: &crate::clip::Clip) -> Self {
        let mut r = Refs::new(EntityKey::Clip(c.id));
        r.push(Some(EntityKey::Track(c.track)));
        r.push(c.lane.map(EntityKey::TakeLane));
        r
    }

    pub(crate) fn device(d: &crate::device::Device) -> Self {
        let mut r = Refs::new(EntityKey::Device(d.id));
        r.push(Some(EntityKey::Track(d.track)));
        r.push(d.sidechain.map(EntityKey::Track));
        r.push(d.pad.map(EntityKey::DrumPad));
        r.push(d.chain.map(EntityKey::RackChain));
        r.push(d.sidechain_tap.and_then(|t| t.chain).map(|c| match c {
            SidechainChain::Rack { chain } => EntityKey::RackChain(chain),
            SidechainChain::Pad { pad } => EntityKey::DrumPad(pad),
        }));
        r
    }

    pub(crate) fn send(s: &TrackSend) -> Self {
        let mut r = Refs::new(EntityKey::Send(s.id));
        r.push(Some(EntityKey::Track(s.from)));
        r.push(Some(EntityKey::Track(s.to)));
        r
    }

    /// The track, device or send an automation target belongs to.
    fn target(t: &AutomationTarget) -> Option<EntityKey> {
        match *t {
            AutomationTarget::TrackVolume { track } | AutomationTarget::TrackPan { track } => {
                Some(EntityKey::Track(track))
            }
            AutomationTarget::SendLevel { send } => Some(EntityKey::Send(send)),
            _ => t.device().map(EntityKey::Device),
        }
    }

    pub(crate) fn lane(l: &crate::automation::AutomationLane) -> Self {
        let mut r = Refs::new(EntityKey::AutomationLane(l.id));
        if let AutomationOwner::Track { track } = l.owner {
            r.push(Some(EntityKey::Track(track)));
        }
        r.push(Self::target(&l.target));
        r
    }

    pub(crate) fn midi_mapping(m: &crate::midi_map::MidiMapping) -> Self {
        let mut r = Refs::new(EntityKey::MidiMapping(m.id));
        r.push(match &m.target {
            MidiMapTarget::Param { target } => Self::target(target),
            MidiMapTarget::TrackMute { track }
            | MidiMapTarget::TrackSolo { track }
            | MidiMapTarget::TrackArm { track } => Some(EntityKey::Track(*track)),
            MidiMapTarget::Transport { .. } => None,
        });
        r
    }

    pub(crate) fn drum_pad(p: &crate::drum_rack::DrumPad) -> Self {
        let mut r = Refs::new(EntityKey::DrumPad(p.id));
        r.push(Some(EntityKey::Device(p.rack)));
        r
    }

    pub(crate) fn take_lane(l: &crate::take::TakeLane) -> Self {
        let mut r = Refs::new(EntityKey::TakeLane(l.id));
        r.push(Some(EntityKey::Track(l.track)));
        r
    }

    pub(crate) fn comp_region(c: &crate::take::CompRegion) -> Self {
        let mut r = Refs::new(EntityKey::CompRegion(c.id));
        r.push(Some(EntityKey::Track(c.track)));
        r.push(Some(EntityKey::TakeLane(c.lane)));
        r
    }

    pub(crate) fn rack_chain(c: &crate::rack::RackChain) -> Self {
        let mut r = Refs::new(EntityKey::RackChain(c.id));
        r.push(Some(EntityKey::Device(c.rack)));
        r
    }

    pub(crate) fn modulator(m: &crate::modulation::Modulator) -> Self {
        let mut r = Refs::new(EntityKey::Modulator(m.id));
        r.push(Some(EntityKey::Device(m.device)));
        r.push(m.sidechain.map(EntityKey::Track));
        r
    }

    pub(crate) fn mod_mapping(m: &crate::modulation::ModMapping) -> Self {
        let mut r = Refs::new(EntityKey::ModMapping(m.id));
        r.push(Some(EntityKey::Device(m.device)));
        r.push(Some(match m.source {
            ModSource::Modulator { modulator } => EntityKey::Modulator(modulator),
            ModSource::Macro { rack, .. } => EntityKey::Device(rack),
        }));
        r
    }

    /// The references of `e` (`None` for kinds the index doesn't cover).
    pub(crate) fn of(e: &Entity) -> Option<Refs> {
        Some(match e {
            Entity::Track(t) => Self::track(t),
            Entity::Clip(c) => Self::clip(c),
            Entity::Device(d) => Self::device(d),
            Entity::Send(s) => Self::send(s),
            Entity::AutomationLane(l) => Self::lane(l),
            Entity::MidiMapping(m) => Self::midi_mapping(m),
            Entity::DrumPad(p) => Self::drum_pad(p),
            Entity::TakeLane(l) => Self::take_lane(l),
            Entity::CompRegion(c) => Self::comp_region(c),
            Entity::RackChain(c) => Self::rack_chain(c),
            Entity::Modulator(m) => Self::modulator(m),
            Entity::ModMapping(m) => Self::mod_mapping(m),
            _ => return None,
        })
    }

    /// The references of the entity `key` as currently stored in `p` (`None` if absent or
    /// not covered).
    pub(crate) fn current(p: &Project, key: EntityKey) -> Option<Refs> {
        match key {
            EntityKey::Track(id) => p.tracks.get(&id).map(Self::track),
            EntityKey::Clip(id) => p.clips.get(&id).map(Self::clip),
            EntityKey::Device(id) => p.devices.get(&id).map(Self::device),
            EntityKey::Send(id) => p.sends.get(&id).map(Self::send),
            EntityKey::AutomationLane(id) => p.automation_lanes.get(&id).map(Self::lane),
            EntityKey::MidiMapping(id) => p.midi_mappings.get(&id).map(Self::midi_mapping),
            EntityKey::DrumPad(id) => p.drum_pads.get(&id).map(Self::drum_pad),
            EntityKey::TakeLane(id) => p.take_lanes.get(&id).map(Self::take_lane),
            EntityKey::CompRegion(id) => p.comp_regions.get(&id).map(Self::comp_region),
            EntityKey::RackChain(id) => p.rack_chains.get(&id).map(Self::rack_chain),
            EntityKey::Modulator(id) => p.modulators.get(&id).map(Self::modulator),
            EntityKey::ModMapping(id) => p.mod_mappings.get(&id).map(Self::mod_mapping),
            _ => None,
        }
    }

    /// The references of every row of the tables the index covers.
    pub(crate) fn all(p: &Project) -> impl Iterator<Item = Refs> + '_ {
        (p.tracks.values().map(Refs::track))
            .chain(p.clips.values().map(Refs::clip))
            .chain(p.devices.values().map(Refs::device))
            .chain(p.sends.values().map(Refs::send))
            .chain(p.automation_lanes.values().map(Refs::lane))
            .chain(p.midi_mappings.values().map(Refs::midi_mapping))
            .chain(p.drum_pads.values().map(Refs::drum_pad))
            .chain(p.take_lanes.values().map(Refs::take_lane))
            .chain(p.comp_regions.values().map(Refs::comp_region))
            .chain(p.rack_chains.values().map(Refs::rack_chain))
            .chain(p.modulators.values().map(Refs::modulator))
            .chain(p.mod_mappings.values().map(Refs::mod_mapping))
    }

    /// Rows of the tables the dependents index covers.
    fn rows(p: &Project) -> usize {
        p.tracks.len()
            + p.clips.len()
            + p.devices.len()
            + p.sends.len()
            + p.automation_lanes.len()
            + p.midi_mappings.len()
            + p.drum_pads.len()
            + p.take_lanes.len()
            + p.comp_regions.len()
            + p.rack_chains.len()
            + p.modulators.len()
            + p.mod_mappings.len()
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
    /// `(target, from)`: `from` references `target` ([`Refs`]).
    refs: BTreeSet<(EntityKey, EntityKey)>,
    /// Rows behind `refs` (one per entity of a covered kind, with or without references).
    ref_rows: usize,
    /// Unique per build ([`next_revision`]): revisions of different builds never match.
    base: u64,
    /// Per clip: the revision of its last note change since the build (absent: none).
    note_revs: HashMap<ClipId, u64>,
}

/// Process-wide revision counter: every build and every note change takes a fresh value,
/// so equal revisions mean the same notes, across clones of a project too.
fn next_revision() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
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
            base: next_revision(),
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
        for r in Refs::all(p) {
            ix.add_refs(Some(r));
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
            && self.ref_rows == Refs::rows(p)
    }

    pub(crate) fn invalidate(&mut self) {
        *self = Self::default();
    }

    /// A note of `link`'s clip changed (inserted, removed, edited or moved).
    fn touch(&mut self, link: Link) {
        if let Link::Note(clip, _) = link {
            self.note_revs.insert(clip, next_revision());
        }
    }

    /// `(build, revision)` of `clip`'s notes: equal values mean the same notes (the
    /// controller's compiled-notes cache).
    pub(crate) fn notes_revision(&self, clip: ClipId) -> (u64, u64) {
        (self.base, self.note_revs.get(&clip).copied().unwrap_or(0))
    }

    pub(crate) fn add(&mut self, link: Link) {
        self.touch(link);
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
        self.touch(link);
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
        } else {
            // Edited in place (a note's start, pitch, mute...).
            self.touch(new);
        }
    }

    pub(crate) fn add_refs(&mut self, refs: Option<Refs>) {
        if let Some(r) = refs {
            self.ref_rows += 1;
            for t in r.targets() {
                self.refs.insert((t, r.from));
            }
        }
    }

    pub(crate) fn remove_refs(&mut self, refs: Option<Refs>) {
        if let Some(r) = refs {
            self.ref_rows -= 1;
            for t in r.targets() {
                self.refs.remove(&(t, r.from));
            }
        }
    }

    pub(crate) fn relink_refs(&mut self, old: Option<Refs>, new: Option<Refs>) {
        if old != new {
            self.remove_refs(old);
            self.add_refs(new);
        }
    }

    /// The entities that reference `key` according to the index (re-check them: see the
    /// module docs), in key order.
    pub(crate) fn referrers(&self, key: EntityKey) -> impl Iterator<Item = EntityKey> + '_ {
        let min = EntityKey::Track(TrackId::from_ulid(Ulid(0)));
        self.refs
            .range((key, min)..)
            .take_while(move |(t, _)| *t == key)
            .map(|(_, from)| *from)
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
            && self.refs == other.refs
            && self.ref_rows == other.ref_rows
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

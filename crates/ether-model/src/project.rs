//! The project document: normalized entity tables + project settings.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::automation::{AutomationLane, AutomationPoint};
use crate::clip::Clip;
use crate::device::{Device, SidechainChain};
use crate::entity::{Entity, EntityKey};
use crate::error::ModelError;
use crate::ids::*;
use crate::index::ProjectIndex;
use crate::media::MediaRef;
use crate::mixer::TrackSend;
use crate::note::Note;
use crate::op::Op;
use crate::tempo::{TempoMap, TempoPoint, TimeSignaturePoint};
use crate::track::Track;
use crate::value::BeatRange;
use crate::warp::WarpMarker;
use crate::*;

/// The whole document.
///
/// **Normalized**: every entity lives in exactly one flat table keyed by its ID and points to
/// its parent by ID (`Clip::track`, `Note::clip`, `Device::track`, ...). Sibling order uses
/// [`crate::value::OrderKey`]. This maps 1:1 onto a CRDT map-of-maps (Loro/Yrs) later, makes
/// every op touch O(1) entities, and lets the UI mirror apply patches as plain upserts.
///
/// Invariants (enforced by `apply`, checked by `validate`):
/// - exactly one `Master` track; a tempo point and a time signature at beat 0;
/// - every parent reference resolves; notes only in MIDI clips; sends target `Return`
///   tracks; no routing cycles.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Project {
    pub id: ProjectId,
    pub settings: ProjectSettings,
    pub tracks: BTreeMap<TrackId, Track>,
    pub clips: BTreeMap<ClipId, Clip>,
    pub notes: BTreeMap<NoteId, Note>,
    pub devices: BTreeMap<DeviceId, Device>,
    pub sends: BTreeMap<SendId, TrackSend>,
    pub automation_lanes: BTreeMap<AutomationLaneId, AutomationLane>,
    pub automation_points: BTreeMap<AutomationPointId, AutomationPoint>,
    pub tempo_points: BTreeMap<TempoPointId, TempoPoint>,
    pub time_signatures: BTreeMap<TimeSignatureId, TimeSignaturePoint>,
    pub warp_markers: BTreeMap<WarpMarkerId, WarpMarker>,
    pub media: BTreeMap<MediaId, MediaRef>,
    /// Arrangement markers (roadmap v2; pre-release `.ether` v3).
    pub markers: BTreeMap<MarkerId, Marker>,
    /// MIDI controller mappings (roadmap v2, `midi-learn`; pre-release `.ether` v3).
    pub midi_mappings: BTreeMap<MidiMappingId, MidiMapping>,
    /// Drum rack pads (roadmap v2, `drum-rack`; pre-release `.ether` v3).
    pub drum_pads: BTreeMap<DrumPadId, DrumPad>,
    // --- v0.2 (pre-release `.ether` v4) ---
    /// Take lanes (`comping`).
    #[serde(default)]
    pub take_lanes: BTreeMap<TakeLaneId, TakeLane>,
    /// Comp regions (`comping`).
    #[serde(default)]
    pub comp_regions: BTreeMap<CompRegionId, CompRegion>,
    /// Rack chains.
    #[serde(default)]
    pub rack_chains: BTreeMap<RackChainId, RackChain>,
    /// Modulators inside devices.
    #[serde(default)]
    pub modulators: BTreeMap<ModulatorId, Modulator>,
    /// Modulation mappings.
    #[serde(default)]
    pub mod_mappings: BTreeMap<ModMappingId, ModMapping>,
    /// Chat journal (pre-release `.ether` v4).
    #[serde(default)]
    pub chat: BTreeMap<ChatMessageId, ChatMessage>,
    /// Notes pinned on the arrangement (pre-release `.ether` v4).
    #[serde(default)]
    pub pinned_notes: BTreeMap<PinnedNoteId, PinnedNote>,
    // --- v0.3 (pre-release `.ether` v5) ---
    /// Clip expression lanes: CC, pitch bend, channel pressure.
    #[serde(default)]
    pub expression_lanes: BTreeMap<ExpressionLaneId, ExpressionLane>,
    /// Per-note expressions: pitch, pressure, timbre (`mpe`).
    #[serde(default)]
    pub note_expressions: BTreeMap<NoteExpressionId, NoteExpression>,
    /// Parent → children indexes (derived; never serialized, ignored by `==`; see
    /// [`Self::ensure_index`]).
    #[serde(skip)]
    #[ts(skip)]
    pub(crate) index: ProjectIndex,
}

/// Project-wide singleton settings (a single LWW register per field).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ProjectSettings {
    pub name: String,
    pub loop_enabled: bool,
    pub loop_region: BeatRange,
    pub metronome: bool,
    /// Count-in before recording, in bars (0 = off).
    pub count_in_bars: u32,
    // --- roadmap v2 (pre-release `.ether` v3) ---
    /// Metronome click level (`tempo-metronome`). -144 dB = silent.
    pub metronome_volume: Decibels,
    /// Accent the first beat of each bar (`tempo-metronome`).
    pub metronome_accent: bool,
    pub metronome_sound: MetronomeSound,
    /// Project swing, 0..=1 (`groove`): a *playback* groove applied to every MIDI clip's
    /// notes when compiling the render graph (non-destructive; notes are not moved). Notes
    /// starting on the odd positions of `swing_grid` (within epsilon) are delayed by
    /// `swing · swing_grid / 3` beats, so 1 = full triplet feel. 0 = straight.
    pub swing: f32,
    /// Swing grid in beats (0.5 = eighths, 0.25 = sixteenths). `> 0`.
    pub swing_grid: Beats,
    /// Project musical scale (piano-roll guide only; never restricts notes). Older files
    /// without it load as chromatic.
    #[serde(default)]
    pub scale: MusicalScale,
    /// The project this one was forked from (`Project::Duplicate` / `SaveAs`): its id and
    /// title when the copy was made. Only set when the copy is made (no op changes it).
    /// Older files and originals have none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub forked_from: Option<ForkOrigin>,
}

/// Where a forked project came from ([`ProjectSettings::forked_from`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ForkOrigin {
    /// The original project (it may since have been deleted).
    pub project: ProjectId,
    /// Its title when the fork was made.
    pub name: String,
    /// When the fork was made, Unix epoch milliseconds.
    pub created_ms: f64,
}

/// Metronome click sound (`tempo-metronome`; the engine synthesizes these, no samples).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub enum MetronomeSound {
    /// Short sine blip (high on accents).
    #[default]
    Classic,
    /// Woodblock-like click.
    Wood,
    /// Square-wave beep.
    Beep,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            name: "Untitled".into(),
            loop_enabled: false,
            loop_region: BeatRange {
                start: Beats::ZERO,
                end: Beats(16.0),
            },
            metronome: false,
            count_in_bars: 0,
            metronome_volume: Decibels(-6.0),
            metronome_accent: true,
            metronome_sound: MetronomeSound::Classic,
            swing: 0.0,
            swing_grid: Beats(0.25),
            scale: MusicalScale::default(),
            forked_from: None,
        }
    }
}

pub(crate) fn by_order<'a, I: Ord + Copy>(
    a: (&'a OrderKey, I),
    b: (&'a OrderKey, I),
) -> std::cmp::Ordering {
    a.cmp(&b)
}

impl Project {
    /// A new empty project: master track, tempo 120 at beat 0, 4/4 at beat 0, default
    /// settings. IDs come from `ids` at time `now_ms` (deterministic for a given generator state).
    pub fn new(ids: &mut IdGen, now_ms: u64) -> Self {
        let id = ids.next_project_id(now_ms);
        let master = Track {
            vca: Default::default(),
            id: ids.next(now_ms),
            kind: TrackKind::Master,
            name: "master".into(),
            color: Color(0x9a_9a9a),
            order: OrderKey::between(None, None),
            parent: None,
            mixer: TrackMixer::default(),
            input: TrackInput::None,
            output: TrackOutput::Default,
            monitor: MonitorMode::default(),
            scale: Default::default(),
            freeze: None,
            mpe: None,
        };
        let tempo = TempoPoint {
            id: ids.next(now_ms),
            time: Beats::ZERO,
            bpm: 120.0,
            curve: TempoCurve::Step,
        };
        let sig = TimeSignaturePoint {
            id: ids.next(now_ms),
            time: Beats::ZERO,
            signature: TimeSignature::default(),
        };
        Self {
            id,
            settings: ProjectSettings::default(),
            tracks: BTreeMap::from([(master.id, master)]),
            clips: BTreeMap::new(),
            notes: BTreeMap::new(),
            devices: BTreeMap::new(),
            sends: BTreeMap::new(),
            automation_lanes: BTreeMap::new(),
            automation_points: BTreeMap::new(),
            tempo_points: BTreeMap::from([(tempo.id, tempo)]),
            time_signatures: BTreeMap::from([(sig.id, sig)]),
            warp_markers: BTreeMap::new(),
            media: BTreeMap::new(),
            markers: BTreeMap::new(),
            midi_mappings: BTreeMap::new(),
            drum_pads: BTreeMap::new(),
            take_lanes: BTreeMap::new(),
            comp_regions: BTreeMap::new(),
            rack_chains: BTreeMap::new(),
            modulators: BTreeMap::new(),
            mod_mappings: BTreeMap::new(),
            chat: BTreeMap::new(),
            pinned_notes: BTreeMap::new(),
            expression_lanes: BTreeMap::new(),
            note_expressions: BTreeMap::new(),
            index: ProjectIndex::default(),
        }
        .indexed()
    }

    fn indexed(mut self) -> Self {
        self.ensure_index();
        self
    }

    /// Build the parent → children indexes if they are missing or stale (after a load, or
    /// after rows were inserted or removed directly in the public tables). `apply` calls it;
    /// read-only accessors fall back to table scans while the index is stale, so this is
    /// only about speed, never about correctness.
    pub fn ensure_index(&mut self) {
        if !self.index.valid_for(self) {
            self.index = ProjectIndex::build(self);
        }
    }

    /// Drop the indexes (rebuilt by the next [`Self::ensure_index`] or `apply`). Code that
    /// changes a reference in place in the public tables (`clip.track`, `note.clip`,
    /// `point.lane`, a lane's owner or target, a device's track, chain, pad or sidechain, a
    /// track's parent, output, input or VCA, ...) must call this.
    pub fn invalidate_index(&mut self) {
        self.index.invalidate();
    }

    /// Remove every device and plugin chain, and everything that only exists for them: rack
    /// chains, drum pads, modulators and their mappings, automation of device parameters
    /// (lanes and points), MIDI mappings to them. Tracks, clips, notes, media, mixer and
    /// routing stay. For "Hide devices": the static project copy and
    /// view, and what a live listener receives.
    pub fn strip_devices(&mut self) {
        self.devices.clear();
        self.rack_chains.clear();
        self.drum_pads.clear();
        self.modulators.clear();
        self.mod_mappings.clear();
        let lanes: Vec<_> = self
            .automation_lanes
            .values()
            .filter(|l| l.target.device().is_some())
            .map(|l| l.id)
            .collect();
        for l in &lanes {
            self.automation_lanes.remove(l);
        }
        self.automation_points
            .retain(|_, pt| !lanes.contains(&pt.lane));
        self.midi_mappings.retain(|_, m| {
            !matches!(&m.target, crate::midi_map::MidiMapTarget::Param { target } if target.device().is_some())
        });
        self.invalidate_index();
    }

    /// The indexes, if usable.
    pub(crate) fn index(&self) -> Option<&ProjectIndex> {
        self.index.valid_for(self).then_some(&self.index)
    }

    /// The indexes are built and match a fresh build (tests and debug checks).
    #[doc(hidden)]
    pub fn index_is_consistent(&self) -> bool {
        self.index.valid_for(self) && self.index.same_as(&ProjectIndex::build(self))
    }

    /// Apply one op; returns its inverse (applying the inverse restores the previous state).
    /// On error the project is unchanged.
    ///
    /// Errors: `NotFound` / `AlreadyExists` (structure), `HasChildren` (removing an entity
    /// that something still references: children, lanes targeting it, ...),
    /// `DanglingReference`, `Invariant` (master, routing cycles, tempo and
    /// signature at 0, ...), `InvalidValue` (out-of-range values, audio fields on MIDI clips).
    pub fn apply(&mut self, op: &Op) -> Result<Op, ModelError> {
        self.apply_checked(op)
    }

    /// Apply ops atomically (all or nothing); returns the inverses in *reverse* order, ready
    /// to be applied as an undo. Every intermediate state must be valid (insert parents
    /// before children, remove children before parents).
    pub fn apply_all(&mut self, ops: &[Op]) -> Result<Vec<Op>, ModelError> {
        self.apply_all_checked(ops)
    }

    /// Apply one op with only its own checks (as each op of [`Self::apply_all`]); returns its
    /// inverse. The document-wide invariants it may break (routing cycles, sidechains, tempo
    /// at 0, ...) are left to [`Self::check_deferred`], once per kind of table recorded in
    /// `kinds`, so a long run of structural ops doesn't re-check the whole project after
    /// each one. The caller must run that check before the document is used, and undo the
    /// ops (their inverses, last first) if it fails. On error the project is unchanged.
    pub fn apply_deferred(
        &mut self,
        op: &Op,
        kinds: &mut Vec<EntityKey>,
    ) -> Result<Op, ModelError> {
        self.apply_deferred_checked(op, kinds)
    }

    /// The document-wide checks left by [`Self::apply_deferred`].
    pub fn check_deferred(&self, kinds: &[EntityKey]) -> Result<(), ModelError> {
        self.check_deferred_globals(kinds)
    }

    /// Check all invariants (used after load/migration and in property tests). Exactly the
    /// checks `apply` enforces, over the whole document.
    pub fn validate(&self) -> Result<(), ModelError> {
        self.validate_all()
    }

    /// Look up any entity by key (cloned).
    pub fn get(&self, key: EntityKey) -> Option<Entity> {
        self.get_entity(key)
    }

    /// All entities, parents before children (useful for full-state patches and CRDT export).
    ///
    /// Order: media, tracks (by nesting depth), tempo points, time signatures, markers, take
    /// lanes, track-chain devices, drum pads, rack chains, pad-chain and rack-chain devices,
    /// modulators, sends, clips, comp regions, notes, warp markers, automation lanes,
    /// automation points, MIDI mappings, modulation mappings, chat messages (by `seq`),
    /// pinned notes, expression lanes, note expressions.
    pub fn entities(&self) -> Vec<Entity> {
        let depth = |t: &Track| {
            let mut d = 0;
            let mut cur = t.parent;
            while let Some(p) = cur {
                d += 1;
                if d > self.tracks.len() {
                    break;
                }
                cur = self.tracks.get(&p).and_then(|p| p.parent);
            }
            d
        };
        let mut tracks: Vec<&Track> = self.tracks.values().collect();
        tracks.sort_by_key(|t| (depth(t), t.id));
        let mut out = Vec::new();
        out.extend(self.media.values().cloned().map(Entity::Media));
        out.extend(tracks.into_iter().cloned().map(Entity::Track));
        out.extend(self.tempo_points.values().cloned().map(Entity::TempoPoint));
        out.extend(
            self.time_signatures
                .values()
                .cloned()
                .map(Entity::TimeSignature),
        );
        out.extend(self.markers.values().cloned().map(Entity::Marker));
        out.extend(self.take_lanes.values().cloned().map(Entity::TakeLane));
        // Racks (track-chain devices) before their pads/chains, pads/chains before their
        // devices.
        let nested = |d: &&Device| d.pad.is_some() || d.chain.is_some();
        out.extend(
            self.devices
                .values()
                .filter(|d| !nested(d))
                .cloned()
                .map(Entity::Device),
        );
        out.extend(self.drum_pads.values().cloned().map(Entity::DrumPad));
        out.extend(self.rack_chains.values().cloned().map(Entity::RackChain));
        out.extend(
            self.devices
                .values()
                .filter(nested)
                .cloned()
                .map(Entity::Device),
        );
        out.extend(self.modulators.values().cloned().map(Entity::Modulator));
        out.extend(self.sends.values().cloned().map(Entity::Send));
        out.extend(self.clips.values().cloned().map(Entity::Clip));
        out.extend(self.comp_regions.values().cloned().map(Entity::CompRegion));
        out.extend(self.notes.values().cloned().map(Entity::Note));
        out.extend(self.warp_markers.values().cloned().map(Entity::WarpMarker));
        out.extend(
            self.automation_lanes
                .values()
                .cloned()
                .map(Entity::AutomationLane),
        );
        out.extend(
            self.automation_points
                .values()
                .cloned()
                .map(Entity::AutomationPoint),
        );
        out.extend(
            self.midi_mappings
                .values()
                .cloned()
                .map(Entity::MidiMapping),
        );
        out.extend(self.mod_mappings.values().cloned().map(Entity::ModMapping));
        out.extend(
            self.chat_ordered()
                .into_iter()
                .cloned()
                .map(Entity::ChatMessage),
        );
        out.extend(self.pinned_notes.values().cloned().map(Entity::PinnedNote));
        out.extend(
            self.expression_lanes
                .values()
                .cloned()
                .map(Entity::ExpressionLane),
        );
        out.extend(
            self.note_expressions
                .values()
                .cloned()
                .map(Entity::NoteExpression),
        );
        out
    }

    /// # Panics
    /// If the project has no master track (never the case for a validated project).
    pub fn master_track(&self) -> &Track {
        self.tracks
            .values()
            .find(|t| t.kind == TrackKind::Master)
            .expect("a project always has a master track")
    }

    /// Top-level tracks sorted by `order` (children via [`Self::child_tracks`]).
    pub fn tracks_ordered(&self) -> Vec<&Track> {
        let mut v: Vec<&Track> = self
            .tracks
            .values()
            .filter(|t| t.parent.is_none())
            .collect();
        v.sort_by(|a, b| by_order((&a.order, a.id), (&b.order, b.id)));
        v
    }

    /// Every track in display order, depth first (a group, then its children), from one pass
    /// over the tracks (a [`Self::child_tracks`] call per track is quadratic). Tracks no walk
    /// reaches (impossible in a valid project) come last.
    pub fn tracks_depth_first(&self) -> Vec<&Track> {
        let mut children: BTreeMap<Option<TrackId>, Vec<&Track>> = BTreeMap::new();
        for t in self.tracks.values() {
            children.entry(t.parent).or_default().push(t);
        }
        for v in children.values_mut() {
            v.sort_by(|a, b| by_order((&a.order, a.id), (&b.order, b.id)));
        }
        let mut out = Vec::with_capacity(self.tracks.len());
        let mut seen: BTreeSet<TrackId> = BTreeSet::new();
        let mut stack: Vec<&Track> = children
            .get(&None)
            .map(|v| v.iter().rev().copied().collect())
            .unwrap_or_default();
        while let Some(t) = stack.pop() {
            if !seen.insert(t.id) {
                continue;
            }
            out.push(t);
            if let Some(kids) = children.get(&Some(t.id)) {
                stack.extend(kids.iter().rev().copied());
            }
        }
        if out.len() != self.tracks.len() {
            out.extend(self.tracks.values().filter(|t| !seen.contains(&t.id)));
        }
        out
    }

    /// Direct children of a group track, sorted by `order`.
    pub fn child_tracks(&self, group: TrackId) -> Vec<&Track> {
        let mut v = self.rows_referencing(
            EntityKey::Track(group),
            &self.tracks,
            |k| match k {
                EntityKey::Track(id) => Some(id),
                _ => None,
            },
            |t| t.parent == Some(group),
        );
        v.sort_by(|a, b| by_order((&a.order, a.id), (&b.order, b.id)));
        v
    }

    /// The group that makes `track` inert: the nearest **ancestor** group that is frozen
    /// (Freeze Group). `None` when no ancestor is frozen. The frozen group itself is not
    /// inert (it plays its render); its descendants are (not processed, locked).
    pub fn frozen_ancestor(&self, track: TrackId) -> Option<TrackId> {
        let mut cur = self.tracks.get(&track)?.parent;
        // Bounded by the track count: a (corrupt) parent cycle cannot loop forever.
        for _ in 0..=self.tracks.len() {
            let g = self.tracks.get(&cur?)?;
            if g.freeze.is_some() {
                return Some(g.id);
            }
            cur = g.parent;
        }
        None
    }

    /// The track whose freeze covers `track`: `track` itself when frozen, else its nearest
    /// frozen ancestor group.
    pub fn frozen_by(&self, track: TrackId) -> Option<TrackId> {
        match self.tracks.get(&track) {
            Some(t) if t.freeze.is_some() => Some(t.id),
            Some(_) => self.frozen_ancestor(track),
            None => None,
        }
    }

    /// How many devices of `track`'s chain ([`Self::devices_of`]) are frozen: `None` when the
    /// track is not covered by a freeze; the whole chain when it is frozen without a freeze
    /// point or sits inside a frozen group; else the devices up to and including
    /// `TrackFreeze::up_to` (partial freeze; the rest is the live tail).
    pub fn frozen_chain_len(&self, track: TrackId) -> Option<usize> {
        let by = self.frozen_by(track)?;
        let chain = self.devices_of(track);
        if by != track {
            return Some(chain.len());
        }
        let up_to = self.tracks.get(&track)?.freeze.as_ref()?.up_to;
        Some(
            up_to
                .and_then(|d| chain.iter().position(|x| x.id == d))
                .map_or(chain.len(), |i| i + 1),
        )
    }

    /// The track-chain device that `device` sits under: itself when it is on a track chain,
    /// else the rack (drum rack or chain rack, nested racks followed up) holding it.
    pub fn chain_root(&self, device: DeviceId) -> Option<&Device> {
        let mut d = self.devices.get(&device)?;
        // Bounded by the device count: a (corrupt) nesting cycle cannot loop forever.
        for _ in 0..=self.devices.len() {
            let rack = match (d.pad, d.chain) {
                (Some(pad), _) => self.drum_pads.get(&pad)?.rack,
                (None, Some(chain)) => self.rack_chains.get(&chain)?.rack,
                (None, None) => return Some(d),
            };
            d = self.devices.get(&rack)?;
        }
        None
    }

    /// Whether `device` is frozen: on a track covered by a freeze, at or before the freeze
    /// point (nested devices follow their rack). Devices of the live tail are not.
    pub fn device_frozen(&self, device: DeviceId) -> bool {
        let Some(root) = self.chain_root(device) else {
            return false;
        };
        let Some(n) = self.frozen_chain_len(root.track) else {
            return false;
        };
        self.devices_of(root.track)
            .iter()
            .position(|d| d.id == root.id)
            .is_none_or(|i| i < n)
    }

    /// Every frozen device ([`Self::device_frozen`]); empty when nothing is frozen.
    pub fn frozen_devices(&self) -> std::collections::BTreeSet<DeviceId> {
        let mut out = std::collections::BTreeSet::new();
        if !self.tracks.values().any(|t| t.freeze.is_some()) {
            return out;
        }
        let mut roots: BTreeMap<TrackId, std::collections::BTreeSet<DeviceId>> = BTreeMap::new();
        for d in self.devices.values() {
            let Some(root) = self.chain_root(d.id) else {
                continue;
            };
            let frozen = roots.entry(root.track).or_insert_with(|| {
                let n = self.frozen_chain_len(root.track).unwrap_or(0);
                self.devices_of(root.track)
                    .into_iter()
                    .take(n)
                    .map(|d| d.id)
                    .collect()
            });
            if frozen.contains(&root.id) {
                out.insert(d.id);
            }
        }
        out
    }

    /// All descendants of `group` (children, their children, ...), in no particular order.
    pub fn descendants(&self, group: TrackId) -> Vec<TrackId> {
        let mut out = Vec::new();
        let mut stack = vec![group];
        while let Some(g) = stack.pop() {
            for t in self.tracks.values().filter(|t| t.parent == Some(g)) {
                if out.len() <= self.tracks.len() {
                    out.push(t.id);
                    stack.push(t.id);
                }
            }
        }
        out
    }

    /// The rows of `table` that reference `key` and pass `keep`, in id order: from the
    /// dependents index when it is usable (see [`Self::referencing`]), else a scan. `id`
    /// picks `table`'s keys out of the referrers.
    fn rows_referencing<'a, I: Ord, T>(
        &'a self,
        key: EntityKey,
        table: &'a BTreeMap<I, T>,
        id: impl Fn(EntityKey) -> Option<I>,
        keep: impl Fn(&T) -> bool,
    ) -> Vec<&'a T> {
        match self.referrers(key) {
            Some(refs) => refs
                .filter_map(id)
                .filter_map(|i| table.get(&i))
                .filter(|t| keep(t))
                .collect(),
            None => table.values().filter(|t| keep(t)).collect(),
        }
    }

    /// The device chain of a track, sorted by `order`. Devices on drum pads
    /// (`Device::pad`) and rack chains (`Device::chain`) are not part of it: see
    /// [`Self::pad_devices_of`], [`Self::chain_devices_of`].
    pub fn devices_of(&self, track: TrackId) -> Vec<&Device> {
        let mut v = self.rows_referencing(
            EntityKey::Track(track),
            &self.devices,
            |k| match k {
                EntityKey::Device(id) => Some(id),
                _ => None,
            },
            |d| d.track == track && d.pad.is_none() && d.chain.is_none(),
        );
        v.sort_by(|a, b| by_order((&a.order, a.id), (&b.order, b.id)));
        v
    }

    /// The device chain of a drum pad, sorted by `order`.
    pub fn pad_devices_of(&self, pad: DrumPadId) -> Vec<&Device> {
        let mut v = self.rows_referencing(
            EntityKey::DrumPad(pad),
            &self.devices,
            |k| match k {
                EntityKey::Device(id) => Some(id),
                _ => None,
            },
            |d| d.pad == Some(pad),
        );
        v.sort_by(|a, b| by_order((&a.order, a.id), (&b.order, b.id)));
        v
    }

    /// The device chain of a rack chain, sorted by `order` (v0.2).
    pub fn chain_devices_of(&self, chain: RackChainId) -> Vec<&Device> {
        let mut v = self.rows_referencing(
            EntityKey::RackChain(chain),
            &self.devices,
            |k| match k {
                EntityKey::Device(id) => Some(id),
                _ => None,
            },
            |d| d.chain == Some(chain),
        );
        v.sort_by(|a, b| by_order((&a.order, a.id), (&b.order, b.id)));
        v
    }

    /// The racks enclosing `device`, innermost first: the rack of each rack chain and the
    /// drum rack of each drum pad on the way up (empty on a track chain). `None` when the
    /// links loop or run deeper than [`MAX_RACK_DEPTH`] (+1, so the model can report a
    /// too-deep nesting) or dangle.
    pub fn enclosing_racks(&self, device: DeviceId) -> Option<Vec<DeviceId>> {
        let mut out = Vec::new();
        let mut d = self.devices.get(&device)?;
        loop {
            let rack = if let Some(chain) = d.chain {
                self.rack_chains.get(&chain)?.rack
            } else if let Some(pad) = d.pad {
                self.drum_pads.get(&pad)?.rack
            } else {
                break;
            };
            if rack == device || out.contains(&rack) || out.len() > MAX_RACK_DEPTH {
                return None;
            }
            out.push(rack);
            d = self.devices.get(&rack)?;
        }
        Some(out)
    }

    /// The devices from the track chain down to `device` (inclusive, outermost first): each
    /// one is a rack whose chain (or drum pad) holds the next. `None` when the links dangle,
    /// loop or run deeper than [`MAX_RACK_DEPTH`] + 1.
    pub fn device_path(&self, device: DeviceId) -> Option<Vec<DeviceId>> {
        let mut out = vec![device];
        let mut d = self.devices.get(&device)?;
        loop {
            let parent = if let Some(chain) = d.chain {
                self.rack_chains.get(&chain)?.rack
            } else if let Some(pad) = d.pad {
                self.drum_pads.get(&pad)?.rack
            } else {
                break;
            };
            if out.contains(&parent) || out.len() > MAX_RACK_DEPTH + 1 {
                return None;
            }
            out.push(parent);
            d = self.devices.get(&parent)?;
        }
        out.reverse();
        Some(out)
    }

    /// Whether `device` sits inside `rack` (on one of its chains or pads, at any depth).
    pub fn is_inside(&self, device: DeviceId, rack: DeviceId) -> bool {
        self.enclosing_racks(device)
            .is_some_and(|r| r.contains(&rack))
    }

    /// Levels of racks nested inside `rack` below it (0 = none), capped at
    /// [`MAX_RACK_DEPTH`] + 1. Every rack counts, drum racks included: below a chain rack its
    /// chains' devices, below a drum rack its pads' devices.
    pub fn nested_rack_height(&self, rack: DeviceId) -> usize {
        fn go(p: &Project, rack: DeviceId, budget: usize) -> usize {
            if budget == 0 {
                return 0;
            }
            p.rack_children(rack)
                .filter(|d| d.kind.is_any_rack())
                .map(|d| 1 + go(p, d.id, budget - 1))
                .max()
                .unwrap_or(0)
        }
        go(self, rack, MAX_RACK_DEPTH + 1)
    }

    /// The devices directly inside `rack`: on its chains (chain rack) or its pads (drum rack).
    /// Indexed (its chains and pads, then their devices) when the index is usable: the
    /// device rules ask this for every device nested in a rack. A device may come twice.
    fn rack_children(&self, rack: DeviceId) -> Box<dyn Iterator<Item = &Device> + '_> {
        let inside = move |d: &&Device| {
            d.chain
                .and_then(|c| self.rack_chains.get(&c))
                .is_some_and(|c| c.rack == rack)
                || d.pad
                    .and_then(|p| self.drum_pads.get(&p))
                    .is_some_and(|p| p.rack == rack)
        };
        match self.referrers(EntityKey::Device(rack)) {
            Some(refs) => Box::new(
                refs.filter(|k| matches!(k, EntityKey::RackChain(_) | EntityKey::DrumPad(_)))
                    .flat_map(|k| self.referrers(k).into_iter().flatten())
                    .filter_map(|k| match k {
                        EntityKey::Device(id) => self.devices.get(&id),
                        _ => None,
                    })
                    .filter(inside),
            ),
            None => Box::new(self.devices.values().filter(inside)),
        }
    }

    /// Whether a drum rack sits anywhere inside `rack` (on its chains or pads, at any depth,
    /// up to [`MAX_RACK_DEPTH`] + 1 levels).
    pub fn has_nested_drum_rack(&self, rack: DeviceId) -> bool {
        fn go(p: &Project, rack: DeviceId, budget: usize) -> bool {
            budget > 0
                && p.rack_children(rack).any(|d| {
                    d.kind.is_drum_rack() || (d.kind.is_any_rack() && go(p, d.id, budget - 1))
                })
        }
        go(self, rack, MAX_RACK_DEPTH + 1)
    }

    /// Whether `device` sits under a drum pad (on a pad's chain, at any depth): a drum rack
    /// is among the racks enclosing it ([`Self::device_path`]).
    pub fn is_under_drum_pad(&self, device: DeviceId) -> bool {
        self.device_path(device).is_some_and(|path| {
            path[..path.len() - 1]
                .iter()
                .any(|r| self.devices.get(r).is_some_and(|r| r.kind.is_drum_rack()))
        })
    }

    /// Chains of a rack device, sorted by `order` (v0.2).
    pub fn chains_of(&self, rack: DeviceId) -> Vec<&RackChain> {
        let mut v = self.rows_referencing(
            EntityKey::Device(rack),
            &self.rack_chains,
            |k| match k {
                EntityKey::RackChain(id) => Some(id),
                _ => None,
            },
            |c| c.rack == rack,
        );
        v.sort_by(|a, b| by_order((&a.order, a.id), (&b.order, b.id)));
        v
    }

    /// The order a same-track sidechain sets between two chains of one rack: device `d` keys
    /// rack chain `from` (or a point inside it) from inside another chain `to` of the same
    /// rack (any depth), so `from` must run first. `(rack, from, to)`; `None` for any other
    /// sidechain (another track, the track's own points, a rack placed before, drum pads).
    /// See `SidechainTap` for the rules.
    pub fn sibling_chain_edge(&self, d: &Device) -> Option<(DeviceId, RackChainId, RackChainId)> {
        if d.sidechain? != d.track {
            return None;
        }
        let SidechainChain::Rack { chain } = d.sidechain_tap?.chain? else {
            return None;
        };
        let rack = self.rack_chains.get(&chain)?.rack;
        let rp = self.device_path(rack)?;
        let dp = self.device_path(d.id)?;
        match rp.iter().zip(&dp).position(|(a, b)| a != b) {
            // The tapped chain's rack holds the device: the chain of that rack holding it.
            None => {
                let to = self.devices.get(dp.get(rp.len())?)?.chain?;
                (to != chain).then_some((rack, chain, to))
            }
            // A rack nested in another chain of a common rack: those two chains.
            Some(i) => {
                let (p, q) = (self.devices.get(&rp[i])?, self.devices.get(&dp[i])?);
                let (Some(a), Some(b)) = (p.chain, q.chain) else {
                    return None;
                };
                let r = self.rack_chains.get(&a)?.rack;
                (a != b && self.rack_chains.get(&b)?.rack == r).then_some((r, a, b))
            }
        }
    }

    /// The order the engine runs `rack`'s chains in: a chain keyed by another one of the same
    /// rack ([`Self::sibling_chain_edge`]) runs before it, otherwise the chains' own order
    /// (the mix is a sum: the order changes nothing else). `None` when they key each other in
    /// a loop (the model rejects it).
    pub fn chain_order(&self, rack: DeviceId) -> Option<Vec<RackChainId>> {
        let mut left: Vec<RackChainId> = self.chains_of(rack).iter().map(|c| c.id).collect();
        let track = self.devices.get(&rack)?.track;
        let mut edges: Vec<(RackChainId, RackChainId)> = self
            .devices
            .values()
            .filter(|d| d.track == track && d.sidechain == Some(track))
            .filter_map(|d| self.sibling_chain_edge(d))
            .filter(|e| e.0 == rack)
            .map(|e| (e.1, e.2))
            .collect();
        edges.sort();
        edges.dedup();
        let mut out = Vec::with_capacity(left.len());
        while !left.is_empty() {
            let next = left.iter().position(|c| {
                !edges
                    .iter()
                    .any(|(from, to)| to == c && left.contains(from))
            })?;
            out.push(left.remove(next));
        }
        Some(out)
    }

    /// Modulators inside a device, sorted by `order` (v0.2).
    pub fn modulators_of(&self, device: DeviceId) -> Vec<&Modulator> {
        let mut v = self.rows_referencing(
            EntityKey::Device(device),
            &self.modulators,
            |k| match k {
                EntityKey::Modulator(id) => Some(id),
                _ => None,
            },
            |m| m.device == device,
        );
        v.sort_by(|a, b| by_order((&a.order, a.id), (&b.order, b.id)));
        v
    }

    /// Modulation mappings targeting a device's params, sorted by (param, id) (v0.2).
    pub fn mappings_to(&self, device: DeviceId) -> Vec<&ModMapping> {
        let mut v = self.rows_referencing(
            EntityKey::Device(device),
            &self.mod_mappings,
            |k| match k {
                EntityKey::ModMapping(id) => Some(id),
                _ => None,
            },
            |m| m.device == device,
        );
        v.sort_by(|a, b| a.param.cmp(&b.param).then(a.id.cmp(&b.id)));
        v
    }

    /// Take lanes of a track, sorted by `order` (v0.2).
    pub fn lanes_of(&self, track: TrackId) -> Vec<&TakeLane> {
        let mut v = self.rows_referencing(
            EntityKey::Track(track),
            &self.take_lanes,
            |k| match k {
                EntityKey::TakeLane(id) => Some(id),
                _ => None,
            },
            |l| l.track == track,
        );
        v.sort_by(|a, b| by_order((&a.order, a.id), (&b.order, b.id)));
        v
    }

    /// Clips of a take lane sorted by start (v0.2).
    pub fn lane_clips_of(&self, lane: TakeLaneId) -> Vec<&Clip> {
        let mut v = self.rows_referencing(
            EntityKey::TakeLane(lane),
            &self.clips,
            |k| match k {
                EntityKey::Clip(id) => Some(id),
                _ => None,
            },
            |c| c.lane == Some(lane),
        );
        v.sort_by(|a, b| a.start.0.total_cmp(&b.start.0).then(a.id.cmp(&b.id)));
        v
    }

    /// Comp regions of a track sorted by start (v0.2).
    pub fn comp_of(&self, track: TrackId) -> Vec<&CompRegion> {
        let mut v = self.rows_referencing(
            EntityKey::Track(track),
            &self.comp_regions,
            |k| match k {
                EntityKey::CompRegion(id) => Some(id),
                _ => None,
            },
            |r| r.track == track,
        );
        v.sort_by(|a, b| a.start.0.total_cmp(&b.start.0).then(a.id.cmp(&b.id)));
        v
    }

    /// Expression lanes of a MIDI clip, sorted by (kind, id) (v0.3).
    pub fn expression_lanes_of(&self, clip: ClipId) -> Vec<&ExpressionLane> {
        let mut v: Vec<&ExpressionLane> = match self.index() {
            Some(ix) => ix
                .expression_lanes_of(clip)
                .filter_map(|id| self.expression_lanes.get(&id))
                .collect(),
            None => self
                .expression_lanes
                .values()
                .filter(|l| l.clip == clip)
                .collect(),
        };
        v.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.id.cmp(&b.id)));
        v
    }

    /// Expressions of a note, sorted by (kind, id) (v0.3).
    pub fn note_expressions_of(&self, note: NoteId) -> Vec<&NoteExpression> {
        let mut v: Vec<&NoteExpression> = match self.index() {
            Some(ix) => ix
                .note_expressions_of(note)
                .filter_map(|id| self.note_expressions.get(&id))
                .collect(),
            None => self
                .note_expressions
                .values()
                .filter(|e| e.note == note)
                .collect(),
        };
        v.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.id.cmp(&b.id)));
        v
    }

    /// Pads of a drum rack device, sorted by note.
    pub fn pads_of(&self, rack: DeviceId) -> Vec<&DrumPad> {
        let mut v = self.rows_referencing(
            EntityKey::Device(rack),
            &self.drum_pads,
            |k| match k {
                EntityKey::DrumPad(id) => Some(id),
                _ => None,
            },
            |p| p.rack == rack,
        );
        v.sort_by(|a, b| a.note.cmp(&b.note).then(a.id.cmp(&b.id)));
        v
    }

    /// Arrangement markers sorted by position (ties by id).
    pub fn markers_sorted(&self) -> Vec<&Marker> {
        let mut v: Vec<&Marker> = self.markers.values().collect();
        v.sort_by(|a, b| a.position.0.total_cmp(&b.position.0).then(a.id.cmp(&b.id)));
        v
    }

    /// Main-lane clips of a track sorted by start (take-lane clips, v0.2, are listed by
    /// [`Self::lane_clips_of`]).
    pub fn arrangement_clips_of(&self, track: TrackId) -> Vec<&Clip> {
        let mut v: Vec<&Clip> = match self.index() {
            Some(ix) => ix
                .clips_of(track)
                .filter_map(|id| self.clips.get(&id))
                .filter(|c| c.lane.is_none())
                .collect(),
            None => self
                .clips
                .values()
                .filter(|c| c.track == track && c.lane.is_none())
                .collect(),
        };
        v.sort_by(|a, b| a.start.0.total_cmp(&b.start.0).then(a.id.cmp(&b.id)));
        v
    }

    /// Notes of a clip sorted by start, then pitch.
    /// A token of `clip`'s notes: equal tokens mean the same notes (no note of the clip was
    /// inserted, removed, edited or moved in between, in this project or a clone of it).
    /// `None` while the index is unusable (rows changed behind the model's back).
    pub fn notes_revision(&self, clip: ClipId) -> Option<(u64, u64)> {
        self.index().map(|ix| ix.notes_revision(clip))
    }

    pub fn notes_of(&self, clip: ClipId) -> Vec<&Note> {
        let mut v: Vec<&Note> = match self.index() {
            Some(ix) => ix
                .notes_of(clip)
                .filter_map(|id| self.notes.get(&id))
                .collect(),
            None => self.notes.values().filter(|n| n.clip == clip).collect(),
        };
        v.sort_by(|a, b| {
            a.start
                .0
                .total_cmp(&b.start.0)
                .then(a.pitch.cmp(&b.pitch))
                .then(a.id.cmp(&b.id))
        });
        v
    }

    /// Points of a lane sorted by time.
    pub fn points_of(&self, lane: AutomationLaneId) -> Vec<&AutomationPoint> {
        let mut v: Vec<&AutomationPoint> = match self.index() {
            Some(ix) => ix
                .points_of(lane)
                .filter_map(|id| self.automation_points.get(&id))
                .collect(),
            None => self
                .automation_points
                .values()
                .filter(|p| p.lane == lane)
                .collect(),
        };
        v.sort_by(|a, b| a.time.0.total_cmp(&b.time.0).then(a.id.cmp(&b.id)));
        v
    }

    /// Warp markers of a clip sorted by beat.
    pub fn warp_markers_of(&self, clip: ClipId) -> Vec<&WarpMarker> {
        let mut v: Vec<&WarpMarker> = match self.index() {
            Some(ix) => ix
                .warp_markers_of(clip)
                .filter_map(|id| self.warp_markers.get(&id))
                .collect(),
            None => self
                .warp_markers
                .values()
                .filter(|m| m.clip == clip)
                .collect(),
        };
        v.sort_by(|a, b| a.beat.0.total_cmp(&b.beat.0).then(a.id.cmp(&b.id)));
        v
    }

    /// Ids of all clips on a track (main and take lanes), in id order.
    pub fn clip_ids_of(&self, track: TrackId) -> Vec<ClipId> {
        match self.index() {
            Some(ix) => ix.clips_of(track).collect(),
            None => self
                .clips
                .values()
                .filter(|c| c.track == track)
                .map(|c| c.id)
                .collect(),
        }
    }

    /// Ids of a clip's notes, in id order.
    pub fn note_ids_of(&self, clip: ClipId) -> Vec<NoteId> {
        match self.index() {
            Some(ix) => ix.notes_of(clip).collect(),
            None => self
                .notes
                .values()
                .filter(|n| n.clip == clip)
                .map(|n| n.id)
                .collect(),
        }
    }

    /// Ids of a clip's warp markers, in id order.
    pub fn warp_marker_ids_of(&self, clip: ClipId) -> Vec<WarpMarkerId> {
        match self.index() {
            Some(ix) => ix.warp_markers_of(clip).collect(),
            None => self
                .warp_markers
                .values()
                .filter(|m| m.clip == clip)
                .map(|m| m.id)
                .collect(),
        }
    }

    /// Ids of the automation lanes owned by `owner` (a track's arrangement lanes or a
    /// clip's envelopes), in id order.
    pub fn lane_ids_owned_by(&self, owner: AutomationOwner) -> Vec<AutomationLaneId> {
        match (self.index(), owner) {
            (Some(ix), AutomationOwner::Track { track }) => ix.track_lanes_of(track).collect(),
            (Some(ix), AutomationOwner::Clip { clip }) => ix.clip_lanes_of(clip).collect(),
            (None, _) => self
                .automation_lanes
                .values()
                .filter(|l| l.owner == owner)
                .map(|l| l.id)
                .collect(),
        }
    }

    /// Ids of a lane's points, in id order.
    pub fn point_ids_of(&self, lane: AutomationLaneId) -> Vec<AutomationPointId> {
        match self.index() {
            Some(ix) => ix.points_of(lane).collect(),
            None => self
                .automation_points
                .values()
                .filter(|p| p.lane == lane)
                .map(|p| p.id)
                .collect(),
        }
    }

    /// Ids of a clip's expression lanes, in id order (v0.3).
    pub fn expression_lane_ids_of(&self, clip: ClipId) -> Vec<ExpressionLaneId> {
        match self.index() {
            Some(ix) => ix.expression_lanes_of(clip).collect(),
            None => self
                .expression_lanes
                .values()
                .filter(|l| l.clip == clip)
                .map(|l| l.id)
                .collect(),
        }
    }

    /// Ids of a note's expressions, in id order (v0.3).
    pub fn note_expression_ids_of(&self, note: NoteId) -> Vec<NoteExpressionId> {
        match self.index() {
            Some(ix) => ix.note_expressions_of(note).collect(),
            None => self
                .note_expressions
                .values()
                .filter(|e| e.note == note)
                .map(|e| e.id)
                .collect(),
        }
    }

    /// Tempo and signature points sorted by time (ties by id).
    pub fn tempo_map(&self) -> TempoMap {
        let mut tempo: Vec<TempoPoint> = self.tempo_points.values().cloned().collect();
        tempo.sort_by(|a, b| a.time.0.total_cmp(&b.time.0).then(a.id.cmp(&b.id)));
        let mut signatures: Vec<TimeSignaturePoint> =
            self.time_signatures.values().cloned().collect();
        signatures.sort_by(|a, b| a.time.0.total_cmp(&b.time.0).then(a.id.cmp(&b.id)));
        TempoMap { tempo, signatures }
    }
}

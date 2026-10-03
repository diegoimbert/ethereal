//! The project document: normalized entity tables + project settings.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::automation::{AutomationLane, AutomationPoint};
use crate::clip::Clip;
use crate::device::Device;
use crate::entity::{Entity, EntityKey};
use crate::error::ModelError;
use crate::ids::*;
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
    /// Arrangement markers (roadmap v2, `clip-editing`; `.ether` v3).
    pub markers: BTreeMap<MarkerId, Marker>,
    /// MIDI controller mappings (roadmap v2, `midi-learn`; `.ether` v3).
    pub midi_mappings: BTreeMap<MidiMappingId, MidiMapping>,
    /// Drum rack pads (roadmap v2, `drum-rack`; `.ether` v3).
    pub drum_pads: BTreeMap<DrumPadId, DrumPad>,
    // --- v0.2 (`.ether` v4, contracts-3) ---
    /// Take lanes (`comping`).
    #[serde(default)]
    pub take_lanes: BTreeMap<TakeLaneId, TakeLane>,
    /// Comp regions (`comping`).
    #[serde(default)]
    pub comp_regions: BTreeMap<CompRegionId, CompRegion>,
    /// Rack chains (`racks-modulation`).
    #[serde(default)]
    pub rack_chains: BTreeMap<RackChainId, RackChain>,
    /// Modulators inside devices (`racks-modulation`).
    #[serde(default)]
    pub modulators: BTreeMap<ModulatorId, Modulator>,
    /// Modulation mappings (`racks-modulation`).
    #[serde(default)]
    pub mod_mappings: BTreeMap<ModMappingId, ModMapping>,
    /// Chat journal (base-62, `collab-social`; `.ether` v4).
    #[serde(default)]
    pub chat: BTreeMap<ChatMessageId, ChatMessage>,
    /// Notes pinned on the arrangement (base-62, `collab-social`; `.ether` v4).
    #[serde(default)]
    pub pinned_notes: BTreeMap<PinnedNoteId, PinnedNote>,
    // --- v0.3 (`.ether` v5, contracts-4) ---
    /// Clip expression lanes: CC, pitch bend, channel pressure (`midi-expression`).
    #[serde(default)]
    pub expression_lanes: BTreeMap<ExpressionLaneId, ExpressionLane>,
    /// Per-note expressions: pitch, pressure, timbre (`midi-expression`, `mpe`).
    #[serde(default)]
    pub note_expressions: BTreeMap<NoteExpressionId, NoteExpression>,
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
    // --- roadmap v2 (`.ether` v3) ---
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
        }
    }
}

fn by_order<'a, I: Ord + Copy>(a: (&'a OrderKey, I), b: (&'a OrderKey, I)) -> std::cmp::Ordering {
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
            name: "Master".into(),
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
        }
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

    /// Direct children of a group track, sorted by `order`.
    pub fn child_tracks(&self, group: TrackId) -> Vec<&Track> {
        let mut v: Vec<&Track> = self
            .tracks
            .values()
            .filter(|t| t.parent == Some(group))
            .collect();
        v.sort_by(|a, b| by_order((&a.order, a.id), (&b.order, b.id)));
        v
    }

    /// The device chain of a track, sorted by `order`. Devices on drum pads
    /// (`Device::pad`) and rack chains (`Device::chain`) are not part of it: see
    /// [`Self::pad_devices_of`], [`Self::chain_devices_of`].
    pub fn devices_of(&self, track: TrackId) -> Vec<&Device> {
        let mut v: Vec<&Device> = self
            .devices
            .values()
            .filter(|d| d.track == track && d.pad.is_none() && d.chain.is_none())
            .collect();
        v.sort_by(|a, b| by_order((&a.order, a.id), (&b.order, b.id)));
        v
    }

    /// The device chain of a drum pad, sorted by `order`.
    pub fn pad_devices_of(&self, pad: DrumPadId) -> Vec<&Device> {
        let mut v: Vec<&Device> = self
            .devices
            .values()
            .filter(|d| d.pad == Some(pad))
            .collect();
        v.sort_by(|a, b| by_order((&a.order, a.id), (&b.order, b.id)));
        v
    }

    /// The device chain of a rack chain, sorted by `order` (v0.2).
    pub fn chain_devices_of(&self, chain: RackChainId) -> Vec<&Device> {
        let mut v: Vec<&Device> = self
            .devices
            .values()
            .filter(|d| d.chain == Some(chain))
            .collect();
        v.sort_by(|a, b| by_order((&a.order, a.id), (&b.order, b.id)));
        v
    }

    /// Chains of a rack device, sorted by `order` (v0.2).
    pub fn chains_of(&self, rack: DeviceId) -> Vec<&RackChain> {
        let mut v: Vec<&RackChain> = self
            .rack_chains
            .values()
            .filter(|c| c.rack == rack)
            .collect();
        v.sort_by(|a, b| by_order((&a.order, a.id), (&b.order, b.id)));
        v
    }

    /// Modulators inside a device, sorted by `order` (v0.2).
    pub fn modulators_of(&self, device: DeviceId) -> Vec<&Modulator> {
        let mut v: Vec<&Modulator> = self
            .modulators
            .values()
            .filter(|m| m.device == device)
            .collect();
        v.sort_by(|a, b| by_order((&a.order, a.id), (&b.order, b.id)));
        v
    }

    /// Modulation mappings targeting a device's params, sorted by (param, id) (v0.2).
    pub fn mappings_to(&self, device: DeviceId) -> Vec<&ModMapping> {
        let mut v: Vec<&ModMapping> = self
            .mod_mappings
            .values()
            .filter(|m| m.device == device)
            .collect();
        v.sort_by(|a, b| a.param.cmp(&b.param).then(a.id.cmp(&b.id)));
        v
    }

    /// Take lanes of a track, sorted by `order` (v0.2).
    pub fn lanes_of(&self, track: TrackId) -> Vec<&TakeLane> {
        let mut v: Vec<&TakeLane> = self
            .take_lanes
            .values()
            .filter(|l| l.track == track)
            .collect();
        v.sort_by(|a, b| by_order((&a.order, a.id), (&b.order, b.id)));
        v
    }

    /// Clips of a take lane sorted by start (v0.2).
    pub fn lane_clips_of(&self, lane: TakeLaneId) -> Vec<&Clip> {
        let mut v: Vec<&Clip> = self
            .clips
            .values()
            .filter(|c| c.lane == Some(lane))
            .collect();
        v.sort_by(|a, b| a.start.0.total_cmp(&b.start.0).then(a.id.cmp(&b.id)));
        v
    }

    /// Comp regions of a track sorted by start (v0.2).
    pub fn comp_of(&self, track: TrackId) -> Vec<&CompRegion> {
        let mut v: Vec<&CompRegion> = self
            .comp_regions
            .values()
            .filter(|r| r.track == track)
            .collect();
        v.sort_by(|a, b| a.start.0.total_cmp(&b.start.0).then(a.id.cmp(&b.id)));
        v
    }

    /// Expression lanes of a MIDI clip, sorted by (kind, id) (v0.3).
    pub fn expression_lanes_of(&self, clip: ClipId) -> Vec<&ExpressionLane> {
        let mut v: Vec<&ExpressionLane> = self
            .expression_lanes
            .values()
            .filter(|l| l.clip == clip)
            .collect();
        v.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.id.cmp(&b.id)));
        v
    }

    /// Expressions of a note, sorted by (kind, id) (v0.3).
    pub fn note_expressions_of(&self, note: NoteId) -> Vec<&NoteExpression> {
        let mut v: Vec<&NoteExpression> = self
            .note_expressions
            .values()
            .filter(|e| e.note == note)
            .collect();
        v.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.id.cmp(&b.id)));
        v
    }

    /// Pads of a drum rack device, sorted by note.
    pub fn pads_of(&self, rack: DeviceId) -> Vec<&DrumPad> {
        let mut v: Vec<&DrumPad> = self.drum_pads.values().filter(|p| p.rack == rack).collect();
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
        let mut v: Vec<&Clip> = self
            .clips
            .values()
            .filter(|c| c.track == track && c.lane.is_none())
            .collect();
        v.sort_by(|a, b| a.start.0.total_cmp(&b.start.0).then(a.id.cmp(&b.id)));
        v
    }

    /// Notes of a clip sorted by start, then pitch.
    pub fn notes_of(&self, clip: ClipId) -> Vec<&Note> {
        let mut v: Vec<&Note> = self.notes.values().filter(|n| n.clip == clip).collect();
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
        let mut v: Vec<&AutomationPoint> = self
            .automation_points
            .values()
            .filter(|p| p.lane == lane)
            .collect();
        v.sort_by(|a, b| a.time.0.total_cmp(&b.time.0).then(a.id.cmp(&b.id)));
        v
    }

    /// Warp markers of a clip sorted by beat.
    pub fn warp_markers_of(&self, clip: ClipId) -> Vec<&WarpMarker> {
        let mut v: Vec<&WarpMarker> = self
            .warp_markers
            .values()
            .filter(|m| m.clip == clip)
            .collect();
        v.sort_by(|a, b| a.beat.0.total_cmp(&b.beat.0).then(a.id.cmp(&b.id)));
        v
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

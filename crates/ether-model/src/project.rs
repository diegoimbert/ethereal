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
    /// Order: media, tracks (by nesting depth), tempo points, time signatures, markers,
    /// track-chain devices, drum pads, pad-chain devices, sends, clips, notes, warp markers,
    /// automation lanes, automation points, MIDI mappings.
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
        // Racks (track-chain devices) before their pads, pads before their devices.
        out.extend(
            self.devices
                .values()
                .filter(|d| d.pad.is_none())
                .cloned()
                .map(Entity::Device),
        );
        out.extend(self.drum_pads.values().cloned().map(Entity::DrumPad));
        out.extend(
            self.devices
                .values()
                .filter(|d| d.pad.is_some())
                .cloned()
                .map(Entity::Device),
        );
        out.extend(self.sends.values().cloned().map(Entity::Send));
        out.extend(self.clips.values().cloned().map(Entity::Clip));
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
    /// (`Device::pad`) are not part of it: see [`Self::pad_devices_of`].
    pub fn devices_of(&self, track: TrackId) -> Vec<&Device> {
        let mut v: Vec<&Device> = self
            .devices
            .values()
            .filter(|d| d.track == track && d.pad.is_none())
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

    /// Clips of a track sorted by start.
    pub fn arrangement_clips_of(&self, track: TrackId) -> Vec<&Clip> {
        let mut v: Vec<&Clip> = self.clips.values().filter(|c| c.track == track).collect();
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

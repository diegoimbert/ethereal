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
use crate::session::{ClipSlot, Scene};
use crate::tempo::{TempoMap, TempoPoint, TimeSignaturePoint};
use crate::track::Track;
use crate::value::{BeatRange, Quantization};
use crate::warp::WarpMarker;

/// The whole document.
///
/// **Normalized**: every entity lives in exactly one flat table keyed by its ID and points to
/// its parent by ID (`Clip::track`, `Note::clip`, `Device::track`, ...). Sibling order uses
/// [`crate::value::OrderKey`]. This maps 1:1 onto a CRDT map-of-maps (Loro/Yrs) later, makes
/// every op touch O(1) entities, and lets the UI mirror apply patches as plain upserts.
///
/// Invariants (enforced by `apply`, checked by `validate`):
/// - exactly one `Master` track; a tempo point and a time signature at beat 0;
/// - every parent reference resolves; notes only in MIDI clips; at most one clip per
///   `(track, scene)` session slot; sends target `Return` tracks; no routing cycles.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Project {
    pub id: ProjectId,
    pub settings: ProjectSettings,
    pub tracks: BTreeMap<TrackId, Track>,
    pub clips: BTreeMap<ClipId, Clip>,
    pub notes: BTreeMap<NoteId, Note>,
    pub devices: BTreeMap<DeviceId, Device>,
    pub sends: BTreeMap<SendId, TrackSend>,
    pub scenes: BTreeMap<SceneId, Scene>,
    pub automation_lanes: BTreeMap<AutomationLaneId, AutomationLane>,
    pub automation_points: BTreeMap<AutomationPointId, AutomationPoint>,
    pub tempo_points: BTreeMap<TempoPointId, TempoPoint>,
    pub time_signatures: BTreeMap<TimeSignatureId, TimeSignaturePoint>,
    pub warp_markers: BTreeMap<WarpMarkerId, WarpMarker>,
    pub media: BTreeMap<MediaId, MediaRef>,
}

/// Project-wide singleton settings (a single LWW register per field).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ProjectSettings {
    pub name: String,
    pub loop_enabled: bool,
    pub loop_region: BeatRange,
    pub metronome: bool,
    /// Global clip launch quantization.
    pub launch_quantization: Quantization,
    /// Count-in before recording, in bars (0 = off).
    pub count_in_bars: u32,
}

impl Project {
    /// A new empty project: master track, tempo 120 at beat 0, 4/4 at beat 0, a handful of
    /// empty scenes, default settings. IDs come from `ids` at time `now_ms`.
    pub fn new(ids: &mut IdGen, now_ms: u64) -> Self {
        let _ = (ids, now_ms);
        todo!("model node")
    }

    /// Apply one op; returns its inverse (applying the inverse restores the previous state).
    /// On error the project is unchanged.
    pub fn apply(&mut self, op: &Op) -> Result<Op, ModelError> {
        let _ = op;
        todo!("model node")
    }

    /// Apply ops atomically (all or nothing); returns the inverses in *reverse* order, ready
    /// to be applied as an undo.
    pub fn apply_all(&mut self, ops: &[Op]) -> Result<Vec<Op>, ModelError> {
        let _ = ops;
        todo!("model node")
    }

    /// Check all invariants (used after load/migration and in property tests).
    pub fn validate(&self) -> Result<(), ModelError> {
        todo!("model node")
    }

    /// Look up any entity by key (cloned).
    pub fn get(&self, key: EntityKey) -> Option<Entity> {
        let _ = key;
        todo!("model node")
    }

    /// All entities, parents before children (useful for full-state patches and CRDT export).
    pub fn entities(&self) -> Vec<Entity> {
        todo!("model node")
    }

    pub fn master_track(&self) -> &Track {
        todo!("model node")
    }

    /// Top-level tracks sorted by `order` (children via [`Self::child_tracks`]).
    pub fn tracks_ordered(&self) -> Vec<&Track> {
        todo!("model node")
    }

    pub fn child_tracks(&self, group: TrackId) -> Vec<&Track> {
        let _ = group;
        todo!("model node")
    }

    /// The device chain of a track, sorted by `order`.
    pub fn devices_of(&self, track: TrackId) -> Vec<&Device> {
        let _ = track;
        todo!("model node")
    }

    /// Arrangement clips of a track sorted by start.
    pub fn arrangement_clips_of(&self, track: TrackId) -> Vec<&Clip> {
        let _ = track;
        todo!("model node")
    }

    pub fn notes_of(&self, clip: ClipId) -> Vec<&Note> {
        let _ = clip;
        todo!("model node")
    }

    pub fn points_of(&self, lane: AutomationLaneId) -> Vec<&AutomationPoint> {
        let _ = lane;
        todo!("model node")
    }

    pub fn warp_markers_of(&self, clip: ClipId) -> Vec<&WarpMarker> {
        let _ = clip;
        todo!("model node")
    }

    pub fn scenes_ordered(&self) -> Vec<&Scene> {
        todo!("model node")
    }

    pub fn clip_slot(&self, track: TrackId, scene: SceneId) -> ClipSlot {
        let _ = (track, scene);
        todo!("model node")
    }

    pub fn tempo_map(&self) -> TempoMap {
        todo!("model node")
    }
}

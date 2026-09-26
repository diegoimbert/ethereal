//! Uniform views over all document entities, used by ops and patches.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::automation::{AutomationLane, AutomationPoint};
use crate::clip::Clip;
use crate::device::Device;
use crate::ids::*;
use crate::media::MediaRef;
use crate::mixer::TrackSend;
use crate::note::Note;
use crate::session::Scene;
use crate::tempo::{TempoPoint, TimeSignaturePoint};
use crate::track::Track;
use crate::warp::WarpMarker;

/// Any document entity (one row of one of the `Project` tables).
///
/// JSON: `{ "type": "Track", "value": { ...Track } }` so the UI can store `value` as-is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "value")]
pub enum Entity {
    Track(Track),
    Clip(Clip),
    Note(Note),
    Device(Device),
    Send(TrackSend),
    Scene(Scene),
    AutomationLane(AutomationLane),
    AutomationPoint(AutomationPoint),
    TempoPoint(TempoPoint),
    TimeSignature(TimeSignaturePoint),
    WarpMarker(WarpMarker),
    Media(MediaRef),
}

/// The key of any entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(tag = "type", content = "id")]
pub enum EntityKey {
    Track(TrackId),
    Clip(ClipId),
    Note(NoteId),
    Device(DeviceId),
    Send(SendId),
    Scene(SceneId),
    AutomationLane(AutomationLaneId),
    AutomationPoint(AutomationPointId),
    TempoPoint(TempoPointId),
    TimeSignature(TimeSignatureId),
    WarpMarker(WarpMarkerId),
    Media(MediaId),
}

impl Entity {
    pub fn key(&self) -> EntityKey {
        match self {
            Self::Track(e) => EntityKey::Track(e.id),
            Self::Clip(e) => EntityKey::Clip(e.id),
            Self::Note(e) => EntityKey::Note(e.id),
            Self::Device(e) => EntityKey::Device(e.id),
            Self::Send(e) => EntityKey::Send(e.id),
            Self::Scene(e) => EntityKey::Scene(e.id),
            Self::AutomationLane(e) => EntityKey::AutomationLane(e.id),
            Self::AutomationPoint(e) => EntityKey::AutomationPoint(e.id),
            Self::TempoPoint(e) => EntityKey::TempoPoint(e.id),
            Self::TimeSignature(e) => EntityKey::TimeSignature(e.id),
            Self::WarpMarker(e) => EntityKey::WarpMarker(e.id),
            Self::Media(e) => EntityKey::Media(e.id),
        }
    }
}

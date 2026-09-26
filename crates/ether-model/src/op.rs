//! Ops: the only way the document changes.
//!
//! Design rules (CRDT-friendly):
//! - Ops address entities by ID only; never by index or position.
//! - `Insert`/`Remove` are whole-entity; `Update` changes **one field** of one entity (a
//!   last-writer-wins register per field), so concurrent edits of different fields never
//!   conflict and every op has a trivial inverse (same variant, old value).
//! - `Remove` requires the entity to have no children; the controller emits child removals
//!   first (so every op stays O(1) and invertible without hidden cascades).
//! - Ordering changes are `Order(OrderKey)` field updates (fractional indexing).
//!
//! Undo = applying inverses. The same op stream is what a future collab layer will sync.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::automation::CurveShape;
use crate::clip::{ClipLocation, ClipLoop, LaunchSettings};
use crate::device::{DeviceKind, PluginInstance};
use crate::entity::{Entity, EntityKey};
use crate::ids::*;
use crate::tempo::TempoCurve;
use crate::track::{MonitorMode, TrackInput, TrackOutput};
use crate::value::*;
use crate::warp::WarpSettings;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "op")]
pub enum Op {
    Insert { entity: Entity },
    Remove { key: EntityKey },
    Update { update: EntityUpdate },
    Settings { change: SettingsChange },
}

/// A single-field change of one entity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum EntityUpdate {
    Track {
        id: TrackId,
        change: TrackChange,
    },
    Clip {
        id: ClipId,
        change: ClipChange,
    },
    Note {
        id: NoteId,
        change: NoteChange,
    },
    Device {
        id: DeviceId,
        change: DeviceChange,
    },
    Send {
        id: SendId,
        change: SendChange,
    },
    Scene {
        id: SceneId,
        change: SceneChange,
    },
    AutomationLane {
        id: AutomationLaneId,
        change: AutomationLaneChange,
    },
    AutomationPoint {
        id: AutomationPointId,
        change: AutomationPointChange,
    },
    TempoPoint {
        id: TempoPointId,
        change: TempoPointChange,
    },
    TimeSignature {
        id: TimeSignatureId,
        change: TimeSignatureChange,
    },
    WarpMarker {
        id: WarpMarkerId,
        change: WarpMarkerChange,
    },
    Media {
        id: MediaId,
        change: MediaChange,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "field", content = "value")]
pub enum SettingsChange {
    Name(String),
    LoopEnabled(bool),
    LoopRegion(BeatRange),
    Metronome(bool),
    LaunchQuantization(Quantization),
    CountInBars(u32),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "field", content = "value")]
pub enum TrackChange {
    Name(String),
    Color(Color),
    Order(OrderKey),
    Parent(Option<TrackId>),
    Volume(Decibels),
    Pan(Pan),
    Mute(bool),
    Solo(bool),
    Input(TrackInput),
    Output(TrackOutput),
    Monitor(MonitorMode),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "field", content = "value")]
pub enum ClipChange {
    /// Move to another track (same kind).
    Track(TrackId),
    Location(ClipLocation),
    Name(String),
    Color(Option<Color>),
    Muted(bool),
    Length(Beats),
    Offset(Beats),
    Loop(ClipLoop),
    Launch(LaunchSettings),
    // Audio-only fields (error on MIDI clips):
    Gain(Decibels),
    Transpose(f32),
    FadeIn(Beats),
    FadeOut(Beats),
    Warp(WarpSettings),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "field", content = "value")]
pub enum NoteChange {
    Pitch(u8),
    Velocity(f32),
    ReleaseVelocity(f32),
    Start(Beats),
    Duration(Beats),
    Muted(bool),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "field", content = "value")]
pub enum DeviceChange {
    Name(String),
    Enabled(bool),
    /// Move to another track's chain (combine with `Order`).
    Track(TrackId),
    Order(OrderKey),
    /// Set a parameter's plain value; `value: None` resets to default (removes the entry).
    Param {
        param: ParamId,
        value: Option<f64>,
    },
    /// Replace device-kind data (e.g. sampler sample). Must keep the same device type.
    Kind(DeviceKind),
    /// Replace plugin metadata/state/sandbox flag (plugins only).
    Plugin(PluginInstance),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "field", content = "value")]
pub enum SendChange {
    Level(Decibels),
    PreFader(bool),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "field", content = "value")]
pub enum SceneChange {
    Name(String),
    Color(Option<Color>),
    Order(OrderKey),
    Tempo(Option<f64>),
    TimeSignature(Option<TimeSignature>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "field", content = "value")]
pub enum AutomationLaneChange {
    Enabled(bool),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "field", content = "value")]
pub enum AutomationPointChange {
    Time(Beats),
    Value(f64),
    Curve(CurveShape),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "field", content = "value")]
pub enum TempoPointChange {
    Time(Beats),
    Bpm(f64),
    Curve(TempoCurve),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "field", content = "value")]
pub enum TimeSignatureChange {
    Time(Beats),
    Signature(TimeSignature),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "field", content = "value")]
pub enum WarpMarkerChange {
    Beat(Beats),
    Source(Seconds),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "field", content = "value")]
pub enum MediaChange {
    Name(String),
}

/// A labeled group of ops applied atomically = one undo step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Transaction {
    pub label: String,
    pub ops: Vec<Op>,
}

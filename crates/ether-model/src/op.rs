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
use crate::clip::{ClipLoop, FadeCurve};
use crate::device::{DeviceKind, PluginInstance};
use crate::entity::{Entity, EntityKey};
use crate::ids::*;
use crate::midi_map::{MidiMapMode, MidiMapTarget, MidiSource};
use crate::project::MetronomeSound;
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
    Marker {
        id: MarkerId,
        change: MarkerChange,
    },
    MidiMapping {
        id: MidiMappingId,
        change: MidiMappingChange,
    },
    DrumPad {
        id: DrumPadId,
        change: DrumPadChange,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "field", content = "value")]
pub enum SettingsChange {
    Name(String),
    LoopEnabled(bool),
    LoopRegion(BeatRange),
    Metronome(bool),
    CountInBars(u32),
    // --- roadmap v2 ---
    MetronomeVolume(Decibels),
    MetronomeAccent(bool),
    MetronomeSound(MetronomeSound),
    Swing(f32),
    SwingGrid(Beats),
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
    /// Arrangement position.
    Start(Beats),
    Name(String),
    Color(Option<Color>),
    Muted(bool),
    Length(Beats),
    Offset(Beats),
    Loop(ClipLoop),
    // Audio-only fields (error on MIDI clips):
    Gain(Decibels),
    Transpose(f32),
    FadeIn(Beats),
    FadeOut(Beats),
    Warp(WarpSettings),
    // Roadmap v2 (audio-only):
    FadeInCurve(FadeCurve),
    FadeOutCurve(FadeCurve),
    Reversed(bool),
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
    /// Roadmap v2: sidechain source track.
    Sidechain(Option<TrackId>),
    /// Roadmap v2: move between the track chain (`None`) and a drum pad chain (combine with
    /// `Order`; the pad must be on a rack of the same track).
    Pad(Option<DrumPadId>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "field", content = "value")]
pub enum SendChange {
    Level(Decibels),
    PreFader(bool),
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "field", content = "value")]
pub enum MarkerChange {
    Position(Beats),
    Name(String),
    Color(Option<Color>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "field", content = "value")]
pub enum MidiMappingChange {
    Source(MidiSource),
    Target(MidiMapTarget),
    Min(f64),
    Max(f64),
    Mode(MidiMapMode),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "field", content = "value")]
pub enum DrumPadChange {
    Note(u8),
    Name(String),
    Color(Option<Color>),
    ChokeGroup(Option<u8>),
    Volume(Decibels),
    Pan(Pan),
    Mute(bool),
}

/// A labeled group of ops applied atomically = one undo step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Transaction {
    pub label: String,
    pub ops: Vec<Op>,
}

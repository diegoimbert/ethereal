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
use crate::media::MediaLocation;
use crate::midi_map::{MidiMapMode, MidiMapTarget, MidiSource};
use crate::modulation::ModSource;
use crate::project::MetronomeSound;
use crate::rack::Zone;
use crate::social::NotePosition;
use crate::tempo::TempoCurve;
use crate::track::{MonitorMode, TrackFreeze, TrackInput, TrackOutput};
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
    // --- v0.2 ---
    TakeLane {
        id: TakeLaneId,
        change: TakeLaneChange,
    },
    CompRegion {
        id: CompRegionId,
        change: CompRegionChange,
    },
    RackChain {
        id: RackChainId,
        change: RackChainChange,
    },
    Modulator {
        id: ModulatorId,
        change: ModulatorChange,
    },
    ModMapping {
        id: ModMappingId,
        change: ModMappingChange,
    },
    /// base-62 (`collab-social`). Chat messages have no updates (sent messages are final).
    PinnedNote {
        id: PinnedNoteId,
        change: PinnedNoteChange,
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
    /// Project musical scale (piano-roll guide).
    Scale(crate::scale::MusicalScale),
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
    /// Track musical scale (MIDI tracks only).
    Scale(crate::scale::TrackScale),
    /// v0.2 (`freeze-bounce`): freeze state.
    Freeze(Option<TrackFreeze>),
    /// v0.2 (`groups-buses`): VCA assignment.
    Vca(Option<TrackId>),
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
    /// v0.2 (`comping`): move between the main lane (`None`) and a take lane of the same track.
    Lane(Option<TakeLaneId>),
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
    /// v0.2 (`racks-modulation`): move between the track chain (`None`) and a rack chain
    /// (combine with `Order`; the chain's rack must be on the same track).
    Chain(Option<RackChainId>),
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
    /// v0.2 (`media-references`): relink / collect.
    Location(MediaLocation),
    /// v0.2 (`media-references`): content hash after a relink to different content.
    Hash(Option<String>),
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
pub enum PinnedNoteChange {
    Position(NotePosition),
    Text(String),
    Resolved(bool),
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "field", content = "value")]
pub enum TakeLaneChange {
    Order(OrderKey),
    Name(String),
    Color(Option<Color>),
}

/// Comp regions are edited as a whole range (start and end together keep the no-overlap
/// invariant checkable per op).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "field", content = "value")]
pub enum CompRegionChange {
    Range(BeatRange),
    Lane(TakeLaneId),
    Crossfade(Seconds),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "field", content = "value")]
pub enum RackChainChange {
    Order(OrderKey),
    Name(String),
    Color(Option<Color>),
    Volume(Decibels),
    Pan(Pan),
    Mute(bool),
    Solo(bool),
    Keys(Zone),
    Velocities(Zone),
    Select(Zone),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "field", content = "value")]
pub enum ModulatorChange {
    Order(OrderKey),
    Name(String),
    /// Set a param's plain value; `None` resets it.
    Param {
        param: ParamId,
        value: Option<f64>,
    },
    /// Envelope-follower sidechain source.
    Sidechain(Option<TrackId>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "field", content = "value")]
pub enum ModMappingChange {
    Depth(f64),
    Source(ModSource),
}

/// A labeled group of ops applied atomically = one undo step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Transaction {
    pub label: String,
    pub ops: Vec<Op>,
}

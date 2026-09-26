//! What the engine renders: [`RenderGraphDesc`] (plain data, built by the controller from
//! the model) and [`RenderSnapshot`] (compiled by the engine off the audio thread).
//!
//! The desc is `Serialize`/`Deserialize` so the web host can ship it from the controller
//! Worker to the AudioWorklet. It references stateful nodes by [`NodeKey`] and audio by
//! `MediaId` (sources registered with `EngineHandle::add_source`), never by pointer.

use ether_protocol::model::{
    AutomationTarget, ClipId, CurveShape, LaunchSettings, MediaId, ParamId, SceneId, SendId,
    TrackId, TrackKind, WarpMode,
};
use serde::{Deserialize, Serialize};

use crate::config::EngineConfig;
use crate::node::NodeKey;
use crate::tempo::{TempoMapRt, TempoPointDesc, TimeSignatureDesc};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RenderGraphDesc {
    /// Increments with every publish (diagnostics, ordering).
    pub version: u64,
    pub tempo: Vec<TempoPointDesc>,
    pub signatures: Vec<TimeSignatureDesc>,
    pub loop_enabled: bool,
    pub loop_start: f64,
    pub loop_end: f64,
    pub metronome: bool,
    /// All tracks incl. groups, returns and master. Order is irrelevant: the compiler
    /// topologically sorts by routing (outputs, sends, resampling inputs) and rejects cycles.
    pub tracks: Vec<TrackDesc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrackDesc {
    pub id: TrackId,
    pub kind: TrackKind,
    /// Device chain in processing order (nodes previously added with `add_node`).
    /// Disabled devices are listed with `enabled: false` (bypassed, latency still counted).
    pub chain: Vec<ChainEntry>,
    /// Fully resolved destination bus: a group track, a return, or master. The controller
    /// resolves `TrackOutput::Default` to the parent group (if any) else master.
    /// `None` = not routed (master: the hardware output).
    pub output: Option<TrackId>,
    /// Enclosing group (tree parent), for group mute/solo propagation. Group tracks
    /// (`kind == Group`) are buses: their input is the sum of every track whose `output`
    /// is the group, then their own device chain + fader, then their `output`.
    pub group: Option<TrackId>,
    pub sends: Vec<SendDesc>,
    /// Linear gain, pan -1..=1. Live changes arrive via the param queue; these are the
    /// initial values when this snapshot is swapped in.
    pub volume: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    /// Hardware input channels monitored/recorded by this track, if any.
    pub audio_input: Option<(u16, u16)>,
    /// Effective monitoring (controller resolves Auto/In/Off + its runtime arm state).
    pub monitor: bool,
    /// Runtime record-arm state (from the controller, not the document).
    pub armed: bool,
    /// Arrangement clips, sorted by start.
    pub clips: Vec<ClipDesc>,
    /// Session clips of this track (used by `session`).
    pub session_clips: Vec<SessionClipDesc>,
    /// Arrangement automation of this track and its devices/sends.
    pub automation: Vec<AutomationDesc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChainEntry {
    pub node: NodeKey,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SendDesc {
    pub id: SendId,
    pub to: TrackId,
    /// Linear gain.
    pub level: f32,
    pub pre_fader: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipDesc {
    pub id: ClipId,
    /// Timeline start (arrangement) in beats; ignored for session clips.
    pub start: f64,
    pub length: f64,
    pub offset: f64,
    /// Content-relative loop `[start, end)` when looping.
    pub looping: Option<(f64, f64)>,
    pub muted: bool,
    pub content: ClipContentDesc,
    /// Clip envelopes (times content-relative).
    pub envelopes: Vec<AutomationDesc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ClipContentDesc {
    Midi {
        /// Sorted by start.
        notes: Vec<NoteDesc>,
    },
    Audio {
        media: MediaId,
        /// Linear gain.
        gain: f32,
        transpose: f32,
        fade_in: f64,
        fade_out: f64,
        warp: Option<WarpDesc>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct NoteDesc {
    /// Content-relative beats.
    pub start: f64,
    pub duration: f64,
    pub key: u8,
    pub velocity: f32,
    pub release_velocity: f32,
}

/// Warping of an audio clip: content beats → source seconds, piecewise linear.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WarpDesc {
    pub mode: WarpMode,
    /// `(content beat, source seconds)`, sorted, at least two.
    pub markers: Vec<(f64, f64)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionClipDesc {
    pub scene: SceneId,
    pub clip: ClipDesc,
    pub launch: LaunchSettings,
}

/// An automation curve resolved to an engine target.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AutomationDesc {
    /// Document-level target (for diagnostics / UI "automation active" state).
    pub target: AutomationTarget,
    pub resolved: ResolvedTarget,
    /// `(time beats, normalized value, curve)` sorted by time.
    pub points: Vec<(f64, f64, CurveShape)>,
    /// Normalized → plain mapping of the target param.
    pub mapping: ParamMapping,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ResolvedTarget {
    TrackVolume,
    TrackPan,
    Send { send: SendId },
    Node { node: NodeKey, param: ParamId },
}

/// RT-evaluable normalized→plain mapping (copied from `ParamInfo` by the controller).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParamMapping {
    pub min: f64,
    pub max: f64,
    pub scale: ether_protocol::devices::ParamScale,
    /// Number of discrete steps, if stepped.
    pub steps: Option<u32>,
}

/// Compiled, immutable render state. Built by [`compile`] off the audio thread, sent to the
/// audio thread boxed, swapped at a block boundary, and returned to the GC for dropping.
/// Internals (processing order, PDC delays, per-track event schedules) belong to the
/// `core` node.
#[non_exhaustive]
pub struct RenderSnapshot {
    pub desc: RenderGraphDesc,
    pub tempo: TempoMapRt,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CompileError {
    #[error("routing cycle through track {0:?}")]
    Cycle(TrackId),
    #[error("unknown node {0:?}")]
    UnknownNode(NodeKey),
    #[error("unknown track {0:?}")]
    UnknownTrack(TrackId),
    #[error("unknown media {0:?}")]
    UnknownMedia(MediaId),
    #[error("graph exceeds engine capacity: {0}")]
    Capacity(String),
}

/// Non-RT. Validate + compile a desc: topological order, PDC, event schedules.
/// Called by `EngineHandle::publish`.
pub fn compile(
    desc: RenderGraphDesc,
    config: &EngineConfig,
) -> Result<RenderSnapshot, CompileError> {
    let _ = (desc, config);
    todo!("core node")
}

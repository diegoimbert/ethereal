//! What the engine renders: [`RenderGraphDesc`] (plain data, built by the controller from
//! the model) and [`RenderSnapshot`] (compiled by the engine off the audio thread).
//!
//! The desc is `Serialize`/`Deserialize` so the web host can ship it from the controller
//! Worker to the AudioWorklet. It references stateful nodes by [`NodeKey`] and audio by
//! `MediaId` (sources registered with `EngineHandle::add_source`), never by pointer.

use ether_protocol::model::{
    AutomationTarget, ClipId, CurveShape, DrumPadId, FadeCurve, InputTap, MediaId, MetronomeSound,
    ParamId, SendId, TrackId, TrackKind, WarpMode,
};
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::config::EngineConfig;
use crate::delay::DelayLine;
use crate::event::EventBuffer;
use crate::mixer::{
    BusInput, ChainRt, MAX_ACTIVE_NOTES, MIX_RAMP_MS, MeterAccum, SendRt, TrackRt, stereo,
};
use crate::node::NodeKey;
use crate::param::Smoother;
use crate::tempo::{TempoMapRt, TempoPointDesc, TimeSignatureDesc};
pub use crate::warp::RateIntegral;

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
    /// Roadmap v2 (`tempo-metronome`): how the click sounds when `metronome` is on.
    #[serde(default)]
    pub click: MetronomeDesc,
    /// All tracks incl. groups, returns and master. Order is irrelevant: the compiler
    /// topologically sorts by routing (outputs, sends, resampling inputs) and rejects cycles.
    pub tracks: Vec<TrackDesc>,
    /// v0.2 (`groups-buses`): VCA faders (`TrackKind::Vca` tracks are never in `tracks`).
    #[serde(default)]
    pub vcas: Vec<crate::vca::VcaDesc>,
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
    /// Hardware input channels monitored/recorded by this track, if any, as
    /// `(first, count)` (`TrackInput::Audio`: count 1 = mono, 2 = stereo).
    pub audio_input: Option<(u16, u16)>,
    /// Live MIDI this track monitors and records (`TrackInput::Midi`: port and channel;
    /// default: everything). Only read on MIDI tracks.
    #[serde(default)]
    pub midi_input: crate::recording::MidiInputFilter,
    /// Effective monitoring (controller resolves Auto/In/Off + its runtime arm state).
    pub monitor: bool,
    /// Runtime record-arm state (from the controller, not the document).
    pub armed: bool,
    /// The track's clips, sorted by start.
    pub clips: Vec<ClipDesc>,
    /// Arrangement automation of this track and its devices/sends.
    pub automation: Vec<AutomationDesc>,
    /// Roadmap v2 (`drum-rack`): pad chains of the drum racks in `chain` (one entry per
    /// rack device, keyed by the rack's node). Empty for tracks without racks.
    #[serde(default)]
    pub racks: Vec<RackDesc>,
    // --- v0.2 (contracts-3; each field's module owns its semantics) ---
    /// Frozen track (`freeze-bounce`, [`crate::freeze`]): `clips` are empty, and so is
    /// `chain` unless the freeze is partial (then it holds the live tail, which processes
    /// the render).
    #[serde(default)]
    pub frozen: Option<crate::freeze::FrozenDesc>,
    /// Rack chains of the rack devices in `chain` (`racks-modulation`,
    /// [`crate::rack_chains`]).
    #[serde(default)]
    pub chain_racks: Vec<crate::rack_chains::ChainRackDesc>,
    /// Modulators and mappings of this track's devices (`racks-modulation`,
    /// [`crate::modulation`]).
    #[serde(default)]
    pub modulation: crate::modulation::ModulationDesc,
    /// Input from another track (`groups-buses`, [`crate::bus_tap`]).
    #[serde(default)]
    pub input_tap: Option<crate::bus_tap::InputTapDesc>,
    /// VCA assignment (`groups-buses`, [`crate::vca`]): an id in `RenderGraphDesc::vcas`.
    #[serde(default)]
    pub vca: Option<TrackId>,
    // --- v0.3 (contracts-4; each field's module owns its semantics) ---
    /// MIDI expression lanes, note expressions and MPE (`midi-expression`, `mpe`,
    /// [`crate::expression`]). Empty = none (v0.2 behaviour).
    #[serde(default)]
    pub expression: crate::expression::TrackExpressionDesc,
    /// External instrument/effect hardware I/O of this track's chain (`external-instrument`,
    /// [`crate::hw_io`]).
    #[serde(default)]
    pub hw_io: Vec<crate::hw_io::HwIoDesc>,
    // --- v0.4 ---
    /// Tap points of sidechained entries (any depth, `crate::sidechain`). A sidechained entry
    /// not listed taps its source's post-fader output.
    #[serde(default)]
    pub sidechain_taps: Vec<crate::sidechain::SidechainTapDesc>,
}

/// Click settings (roadmap v2, `tempo-metronome`). The click is rendered by
/// [`crate::metronome`] straight into the hardware output after master (not metered, never
/// part of exports), on every beat while playing when `RenderGraphDesc::metronome` is on,
/// and during a recording count-in regardless of it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetronomeDesc {
    /// Linear gain.
    pub volume: f32,
    /// Accent (higher/louder click) on the first beat of each bar.
    pub accent: bool,
    pub sound: MetronomeSound,
    /// Recording count-in: while playing and recording with the position before this beat,
    /// the click sounds even if the metronome is off. Set by the controller for the
    /// duration of a record session's pre-roll (`recording` computes it:
    /// `EtherController::recording` knows the record start), `None` otherwise.
    #[serde(default)]
    pub count_in_end: Option<f64>,
}

impl Default for MetronomeDesc {
    fn default() -> Self {
        Self {
            volume: 0.5,
            accent: true,
            sound: MetronomeSound::Classic,
            count_in_end: None,
        }
    }
}

/// A drum rack's pads (roadmap v2, `drum-rack`). When processing the chain entry whose
/// node is `rack`, the engine routes the incoming note events to the pad chains by key
/// (transposed to `ether_model::PAD_PLAY_NOTE`), applies choke groups, runs each pad chain
/// (same rules as a track chain: bypass, latency, param events), mixes them through the
/// pad's volume/pan/mute into the rack node's input, then runs the rack node itself. PDC
/// inside a rack: every pad chain is delayed to the longest pad chain's latency, and the
/// rack node reports that as part of its chain position (the controller adds it when
/// computing `NodeInfo`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RackDesc {
    pub rack: NodeKey,
    pub pads: Vec<PadDesc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PadDesc {
    pub pad: DrumPadId,
    /// Incoming key that triggers the pad.
    pub note: u8,
    pub choke_group: Option<u8>,
    pub chain: Vec<ChainEntry>,
    /// Linear gain; pan -1..=1.
    pub volume: f32,
    pub pan: f32,
    pub mute: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChainEntry {
    pub node: NodeKey,
    pub enabled: bool,
    /// Roadmap v2 (`sidechain`): the track whose signal feeds this node's sidechain input
    /// (`Node::process_sidechain`), at the point listed in `TrackDesc::sidechain_taps`
    /// (post-fader by default). Any depth (v0.4: pad and rack chain entries too). The
    /// compiler orders another `sidechain` track before this one (a routing edge) and aligns
    /// it for PDC (CONTRACTS.md §11.10, `crate::sidechain`).
    #[serde(default)]
    pub sidechain: Option<TrackId>,
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
    /// Timeline start in beats (`Clip.start`).
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

// Audio is the large variant (two envelopes inline). Boxing them would add an allocation per
// envelope to every decode on the web, which the codec keeps to one per `Vec`
// (`tests/codec_alloc.rs`); the descs live in `Vec`s built off the audio thread.
#[allow(clippy::large_enum_variant)]
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
        /// Roadmap v2 (`clip-editing`): fade shapes (`crate::fades::fade_gain`) and reverse
        /// playback (see `ether_model::AudioContent::reversed`).
        #[serde(default)]
        fade_in_curve: FadeCurve,
        #[serde(default)]
        fade_out_curve: FadeCurve,
        #[serde(default)]
        reversed: bool,
        /// Clip gain envelope (`AutomationTarget::ClipGain`): dB added to `gain` at each
        /// content beat. Applied with the fades ([`crate::sched::ClipEnvelope`]).
        #[serde(default)]
        gain_envelope: Option<ClipParamEnvelope>,
        /// Clip transposition envelope (`AutomationTarget::ClipTranspose`): semitones added
        /// to `transpose` at each content beat.
        #[serde(default)]
        transpose_envelope: Option<TransposeEnvelope>,
    },
}

/// An envelope of an audio clip's own parameter: normalized points at content beats and
/// the normalized → plain mapping (dB for gain, semitones for transposition).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipParamEnvelope {
    /// `(content beat, normalized value, curve)`, sorted, at least one.
    pub points: Vec<(f64, f64, CurveShape)>,
    pub mapping: ParamMapping,
}

impl ClipParamEnvelope {
    /// Plain value at content beat `c`.
    #[inline]
    pub fn plain_at(&self, c: f64) -> f64 {
        crate::automation::evaluate(&self.points, c)
            .map_or(0.0, |v| crate::automation::to_plain(&self.mapping, v))
    }
}

/// A transposition envelope plus what resampled playback needs to follow it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransposeEnvelope {
    pub env: ClipParamEnvelope,
    /// Resampled (Repitch / unwarped) playback: the source position integrates the
    /// envelope's rate ([`crate::warp::RateIntegral`]).
    pub rate: RateIntegral,
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
    Send {
        send: SendId,
    },
    Node {
        node: NodeKey,
        param: ParamId,
    },
    /// A device's on/off switch (normalized 0/1, on at >= 0.5): overrides the chain entry's
    /// `enabled` while automated (`ChainRt::auto_enabled`).
    NodeEnabled {
        node: NodeKey,
    },
}

impl ResolvedTarget {
    /// The device node this target drives (device params and the device switch).
    pub fn node_mut(&mut self) -> Option<&mut NodeKey> {
        match self {
            Self::Node { node, .. } | Self::NodeEnabled { node } => Some(node),
            _ => None,
        }
    }

    /// The device node this target drives.
    pub fn node(&self) -> Option<NodeKey> {
        match *self {
            Self::Node { node, .. } | Self::NodeEnabled { node } => Some(node),
            _ => None,
        }
    }
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
///
/// The routing is immutable; the per-track running state inside (smoothers, delay lines,
/// sounding notes, meters) is owned by the audio thread while the snapshot is current and
/// carried over to the next snapshot on swap.
#[non_exhaustive]
pub struct RenderSnapshot {
    pub desc: RenderGraphDesc,
    pub tempo: TempoMapRt,
    pub(crate) rt: SnapshotRt,
}

/// One level of the processing DAG (`crate::parallel`): `SnapshotRt::level_order[start..end]`,
/// the first `pinned` of which must run on the audio thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Level {
    pub start: usize,
    pub end: usize,
    pub pinned: usize,
}

pub(crate) struct SnapshotRt {
    /// Track indices (into `desc.tracks` / `tracks`) in processing order.
    pub order: Vec<usize>,
    /// Track indices grouped by level (`levels`), pinned tracks first within a level, then
    /// processing order.
    pub level_order: Vec<usize>,
    pub levels: Vec<Level>,
    pub tracks: Vec<TrackRt>,
    /// Sorted lookups (binary search on the audio thread).
    pub track_index: Vec<(TrackId, usize)>,
    pub send_index: Vec<(SendId, usize, usize)>,
    pub node_index: Vec<(NodeKey, usize, usize)>,
    /// Total latency from the timeline to the hardware output (samples).
    pub latency: u32,
    /// Drum-rack pad-chain nodes and their track index, sorted (live params, automation
    /// and latency refresh reach them through `TrackRt::racks`).
    pub pad_index: Vec<(NodeKey, usize)>,
    /// v0.2: rack-chain nodes and their track index, sorted (`crate::rack_chains`).
    pub rack_chain_index: Vec<(NodeKey, usize)>,
    /// v0.2: VCA faders (`crate::vca`).
    pub vcas: crate::vca::VcaRt,
}

impl RenderSnapshot {
    /// Processing order (track ids), sources before the buses they feed.
    pub fn processing_order(&self) -> impl Iterator<Item = TrackId> + '_ {
        self.rt.order.iter().map(|&i| self.rt.tracks[i].id)
    }

    /// Latency (samples) of the signal at `track`'s output, relative to the timeline.
    pub fn track_latency(&self, track: TrackId) -> Option<u32> {
        self.rt
            .lookup_track(track)
            .map(|i| self.rt.tracks[i].out_latency)
    }

    /// PDC delay (samples) inserted between `track`'s output and its destination bus.
    pub fn output_compensation(&self, track: TrackId) -> Option<u32> {
        self.rt
            .lookup_track(track)
            .map(|i| self.rt.tracks[i].output_delay.delay() as u32)
    }

    /// PDC delay (samples) inserted on a send.
    pub fn send_compensation(&self, send: SendId) -> Option<u32> {
        let (t, s) = self.rt.lookup_send(send)?;
        Some(self.rt.tracks[t].sends[s].delay.delay() as u32)
    }

    /// Total output latency of the graph (samples): the master bus' output latency.
    pub fn latency(&self) -> u32 {
        self.rt.latency
    }
}

impl SnapshotRt {
    pub(crate) fn lookup_track(&self, track: TrackId) -> Option<usize> {
        self.track_index
            .binary_search_by(|e| e.0.cmp(&track))
            .ok()
            .map(|i| self.track_index[i].1)
    }

    pub(crate) fn lookup_send(&self, send: SendId) -> Option<(usize, usize)> {
        self.send_index
            .binary_search_by(|e| e.0.cmp(&send))
            .ok()
            .map(|i| (self.send_index[i].1, self.send_index[i].2))
    }

    /// Track of a drum-rack pad-chain node.
    pub(crate) fn lookup_pad_node(&self, node: NodeKey) -> Option<usize> {
        self.pad_index
            .binary_search_by(|e| e.0.cmp(&node))
            .ok()
            .map(|i| self.pad_index[i].1)
    }

    pub(crate) fn lookup_node(&self, node: NodeKey) -> Option<(usize, usize)> {
        self.node_index
            .binary_search_by(|e| e.0.cmp(&node))
            .ok()
            .map(|i| (self.node_index[i].1, self.node_index[i].2))
    }

    pub(crate) fn lookup_rack_chain_node(&self, node: NodeKey) -> Option<usize> {
        self.rack_chain_index
            .binary_search_by(|e| e.0.cmp(&node))
            .ok()
            .map(|i| self.rack_chain_index[i].1)
    }

    /// RT. Recompute every track's mute/solo gate target (mute propagates from groups).
    pub(crate) fn update_gates(&mut self) {
        let n = self.tracks.len();
        for i in 0..n {
            // A muted VCA mutes its tracks (`crate::vca`, v0.2).
            let mut muted = self.tracks[i].vca.muted;
            let mut cur = Some(i);
            let mut depth = 0;
            while let Some(t) = cur {
                if self.tracks[t].mute {
                    muted = true;
                    break;
                }
                cur = self.tracks[t].parent;
                depth += 1;
                if depth > n {
                    break;
                }
            }
            let open = !muted && self.tracks[i].solo_ok;
            let target = if open { 1.0 } else { 0.0 };
            let gate = &mut self.tracks[i].gate;
            if gate.current() != target && !(gate.is_smoothing() && gate.target() == target) {
                gate.set_target(target);
            }
        }
    }
}

/// What the compiler needs to know about an inserted node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeInfo {
    pub latency: u32,
    pub channels: (u16, u16),
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

/// Longest PDC delay the compiler accepts (seconds of audio). Also the most latency a
/// single node may report ([`max_latency_samples`]): anything longer could never be
/// compensated anyway.
pub const MAX_PDC_SECONDS: u32 = 10;

/// [`MAX_PDC_SECONDS`] in samples at `sample_rate`: the cap on a node's reported latency.
/// Plugin hosts clamp what plugins report to it (a plugin answering `-1` / `u32::MAX` must
/// not size multi-GB delay lines), and [`compile_with`] clamps every node's latency to it.
pub fn max_latency_samples(sample_rate: u32) -> u32 {
    MAX_PDC_SECONDS.saturating_mul(sample_rate)
}

/// Non-RT. Validate + compile a desc: topological order, PDC, event schedules.
/// Called by `EngineHandle::publish`.
///
/// This standalone form knows nothing about inserted nodes: every node is assumed to be
/// stereo with zero latency and node keys are not validated. The engine handle uses
/// [`compile_with`] with its real node table.
pub fn compile(
    desc: RenderGraphDesc,
    config: &EngineConfig,
) -> Result<RenderSnapshot, CompileError> {
    compile_with(desc, config, &|_| {
        Some(NodeInfo {
            latency: 0,
            channels: (2, 2),
        })
    })
}

/// Non-RT. [`compile`] with node information (`None` = unknown node → error).
pub fn compile_with(
    desc: RenderGraphDesc,
    config: &EngineConfig,
    node_info: &dyn Fn(NodeKey) -> Option<NodeInfo>,
) -> Result<RenderSnapshot, CompileError> {
    // A node's latency is capped at the PDC limit, so a bogus report (a plugin answering
    // `u32::MAX`) can neither wrap the latency sums below nor slip past the PDC check.
    let max_latency = max_latency_samples(config.sample_rate);
    let clamped = |key: NodeKey| {
        node_info(key).map(|mut info| {
            info.latency = info.latency.min(max_latency);
            info
        })
    };
    let node_info: &dyn Fn(NodeKey) -> Option<NodeInfo> = &clamped;
    let n = desc.tracks.len();
    let sr = config.sample_rate as f32;
    let frames = config.max_block_size;

    // --- ids ---
    let mut track_index: Vec<(TrackId, usize)> = desc
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| (t.id, i))
        .collect();
    track_index.sort();
    if let Some(w) = track_index.windows(2).find(|w| w[0].0 == w[1].0) {
        return Err(CompileError::Capacity(format!(
            "duplicate track {:?}",
            w[0].0
        )));
    }
    let find = |id: TrackId| -> Result<usize, CompileError> {
        track_index
            .binary_search_by(|e| e.0.cmp(&id))
            .map(|i| track_index[i].1)
            .map_err(|_| CompileError::UnknownTrack(id))
    };

    // --- edges (src -> dst) ---
    let mut outputs = vec![None; n];
    let mut parents = vec![None; n];
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, t) in desc.tracks.iter().enumerate() {
        if let Some(o) = t.output {
            let o = find(o)?;
            if o == i {
                return Err(CompileError::Cycle(t.id));
            }
            outputs[i] = Some(o);
            succ[i].push(o);
        }
        if let Some(g) = t.group {
            parents[i] = Some(find(g)?);
        }
        for s in &t.sends {
            let to = find(s.to)?;
            if to == i {
                return Err(CompileError::Cycle(t.id));
            }
            succ[i].push(to);
        }
    }

    // Sidechain edges (source → consumer) only constrain the processing order; they are
    // not bus connections (`crate::sidechain` handles their PDC).
    let track_of = |id: TrackId| find(id).ok();
    let mut order_succ = succ.clone();
    // v0.4: sidechained entries at any depth (`crate::sidechain`).
    let sc_consumers: Vec<Vec<crate::sidechain::Consumer>> = desc
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| crate::sidechain::consumers(i, t, &track_of))
        .collect();
    for (i, cs) in sc_consumers.iter().enumerate() {
        let mut sources: Vec<usize> = cs.iter().map(|c| c.source).filter(|&s| s != i).collect();
        sources.sort_unstable();
        sources.dedup();
        for s in sources {
            order_succ[s].push(i);
        }
    }
    // v0.2: envelope-follower sidechains (`crate::modulation`): the source goes first and
    // keeps its tap.
    let mod_sc_sources: Vec<Vec<usize>> = desc
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let mut v: Vec<usize> = t
                .modulation
                .modulators
                .iter()
                .filter_map(|m| m.sidechain.and_then(track_of))
                .filter(|&s| s != i)
                .collect();
            v.sort_unstable();
            v.dedup();
            v
        })
        .collect();
    for (i, sources) in mod_sc_sources.iter().enumerate() {
        for &s in sources {
            order_succ[s].push(i);
        }
    }
    // v0.2: track input from another track (`crate::bus_tap`): the source goes first.
    let tap_source: Vec<Option<usize>> = desc
        .tracks
        .iter()
        .map(|t| t.input_tap.and_then(|tap| track_of(tap.track)))
        .collect();
    for (i, s) in tap_source.iter().enumerate() {
        if let Some(s) = *s {
            if s == i {
                return Err(CompileError::Cycle(desc.tracks[i].id));
            }
            order_succ[s].push(i);
        }
    }

    // --- topological sort (Kahn; ties broken by desc order, so it is deterministic) ---
    let mut indeg = vec![0usize; n];
    for s in &order_succ {
        for &d in s {
            indeg[d] += 1;
        }
    }
    let mut ready: BTreeSet<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
    let mut order = Vec::with_capacity(n);
    while let Some(i) = ready.pop_first() {
        order.push(i);
        for &d in &order_succ[i] {
            indeg[d] -= 1;
            if indeg[d] == 0 {
                ready.insert(d);
            }
        }
    }
    if order.len() != n {
        let stuck = (0..n).find(|&i| indeg[i] > 0).unwrap_or(0);
        return Err(CompileError::Cycle(desc.tracks[stuck].id));
    }

    // --- levels (`crate::parallel`) ---
    // Level = 1 + max level of every source (outputs, sends, sidechains): tracks of one
    // level never depend on each other.
    let mut level = vec![0usize; n];
    for &i in &order {
        for &d in &order_succ[i] {
            level[d] = level[d].max(level[i] + 1);
        }
    }
    // No track is pinned to the audio thread any more: Complex-warped clips play through
    // their own track's stretcher voices (`crate::warp`, perf audit #8). The pinned-jobs
    // path of the executor stays for hosts' future needs.
    let pinned = vec![false; n];
    let mut pos = vec![0usize; n];
    for (p, &i) in order.iter().enumerate() {
        pos[i] = p;
    }
    let mut level_order = order.clone();
    level_order.sort_by_key(|&i| (level[i], !pinned[i], pos[i]));
    let mut levels: Vec<Level> = Vec::new();
    for (k, &i) in level_order.iter().enumerate() {
        match levels.last_mut() {
            Some(l) if level[level_order[l.start]] == level[i] => {
                l.end = k + 1;
                l.pinned += pinned[i] as usize;
            }
            _ => levels.push(Level {
                start: k,
                end: k + 1,
                pinned: pinned[i] as usize,
            }),
        }
    }

    // --- bus inputs, in the sequential summation order (`crate::parallel`) ---
    let mut bus_inputs: Vec<Vec<BusInput>> = vec![Vec::new(); n];
    for &c in &order {
        for pass_pre in [true, false] {
            for (k, s) in desc.tracks[c].sends.iter().enumerate() {
                if s.pre_fader == pass_pre {
                    bus_inputs[find(s.to)?].push(BusInput::Send(c, k));
                }
            }
        }
        if let Some(o) = outputs[c] {
            bus_inputs[o].push(BusInput::Output(c));
        }
    }

    // --- nodes ---
    let mut node_index = Vec::new();
    let mut pad_index = Vec::new();
    let mut rack_chain_index = Vec::new();
    let mut chain_info: Vec<Vec<NodeInfo>> = Vec::with_capacity(n);
    for (i, t) in desc.tracks.iter().enumerate() {
        let mut infos = Vec::with_capacity(t.chain.len());
        for (k, e) in t.chain.iter().enumerate() {
            let mut info = node_info(e.node).ok_or(CompileError::UnknownNode(e.node))?;
            // A drum rack's entry also carries its longest pad chain (pads run before it).
            info.latency = info.latency.saturating_add(crate::drum_rack::pad_latency(
                &t.racks,
                &t.chain_racks,
                e.node,
                node_info,
            ));
            // v0.2: a rack's entry carries its longest chain (`crate::rack_chains`).
            info.latency = info
                .latency
                .saturating_add(crate::rack_chains::chain_latency(
                    &t.chain_racks,
                    &t.racks,
                    e.node,
                    node_info,
                ));
            infos.push(info);
            node_index.push((e.node, i, k));
        }
        for pad_node in t
            .racks
            .iter()
            .flat_map(|r| &r.pads)
            .flat_map(|p| &p.chain)
            .map(|e| e.node)
        {
            // Unknown pad nodes stay allowed (silent: `pad_latency` counts 0). A stale key
            // there may share its slot with a live node of another track; the jobs'
            // `engine::NodeTable::get` only reads the slot's generation for it.
            pad_index.push((pad_node, i));
        }
        for chain_node in t
            .chain_racks
            .iter()
            .flat_map(|r| &r.chains)
            .flat_map(|c| &c.chain)
            .map(|e| e.node)
        {
            rack_chain_index.push((chain_node, i));
        }
        chain_info.push(infos);
    }
    node_index.sort();
    pad_index.sort();
    rack_chain_index.sort();
    {
        // A key is used once (parallel jobs get `&mut` to their live nodes through
        // `engine::NodeTable`; a slot has one live generation, so live keys never collide).
        let mut all: Vec<NodeKey> = node_index
            .iter()
            .map(|e| e.0)
            .chain(pad_index.iter().map(|e| e.0))
            .chain(rack_chain_index.iter().map(|e| e.0))
            .collect();
        all.sort();
        if let Some(w) = all.windows(2).find(|w| w[0] == w[1]) {
            return Err(CompileError::Capacity(format!(
                "node {:?} used more than once",
                w[0]
            )));
        }
    }

    // --- PDC ---
    // A track's output latency is its input latency plus its whole chain's latency
    // (bypassed devices included: they are replaced by an equal delay, so a track's
    // latency never depends on bypass state). A bus's input latency is the max over its
    // inputs; every connection into it is delayed by the difference.
    let mut chain_lat: Vec<u32> = chain_info
        .iter()
        .map(|c| c.iter().fold(0u32, |a, i| a.saturating_add(i.latency)))
        .collect();
    let mut in_lat = vec![0u32; n];
    let mut out_lat = vec![0u32; n];
    // Sidechain PDC (`crate::sidechain::plan`): main-signal delays before sidechained
    // entries count into the track's chain latency (pad / rack chain entries: into their
    // rack entry's latency).
    let mut sc_plans: Vec<crate::sidechain::TrackPlan> = vec![Default::default(); n];
    // v0.2 (`crate::bus_tap`): a track input tap counts as an input of the consumer.
    let mut tap_delay = vec![0u32; n];
    for &i in &order {
        if let (Some(s), Some(tap)) = (tap_source[i], desc.tracks[i].input_tap) {
            let tap_lat = match tap.point {
                InputTap::PreFx => in_lat[s],
                InputTap::PostFx | InputTap::PostFader => out_lat[s],
            };
            in_lat[i] = in_lat[i].max(tap_lat);
            tap_delay[i] = in_lat[i] - tap_lat;
        }
        let plan = {
            let planned = &sc_plans;
            crate::sidechain::plan(
                i,
                &desc.tracks[i],
                node_info,
                &sc_consumers[i],
                in_lat[i],
                &|s, point| planned[s].point_latency(point),
            )
        };
        // Rack entries carry their chains' latency, with the delays planned inside them.
        for (info, &l) in chain_info[i].iter_mut().zip(&plan.top_latency) {
            info.latency = l;
        }
        chain_lat[i] = chain_info[i]
            .iter()
            .fold(0u32, |a, c| a.saturating_add(c.latency));
        out_lat[i] = in_lat[i]
            .saturating_add(chain_lat[i])
            .saturating_add(plan.top_main);
        sc_plans[i] = plan;
        for &d in &succ[i] {
            in_lat[d] = in_lat[d].max(out_lat[i]);
        }
    }
    let max_delay = max_latency;
    let check = |d: u32| -> Result<usize, CompileError> {
        if d > max_delay {
            Err(CompileError::Capacity(format!(
                "plugin delay compensation of {:.1} s, the limit is {MAX_PDC_SECONDS} s",
                f64::from(d) / f64::from(config.sample_rate)
            )))
        } else {
            Ok(d as usize)
        }
    };

    // --- solo (CONTRACTS.md §12.10, `groups-buses`) ---
    // While anything is soloed, a track stays audible if it is soloed, inside a soloed
    // group, a group/bus on the output path of a soloed track (solo in place through the
    // bus chain, explicit outputs included), a return, or the master. VCA solo is folded
    // into `TrackDesc::solo` by the controller.
    let any_solo = desc.tracks.iter().any(|t| t.solo);
    let parents_ref = &parents;
    let ancestors = |i: usize| {
        let mut cur = parents_ref[i];
        let mut guard = 0;
        std::iter::from_fn(move || {
            let p = cur?;
            guard += 1;
            if guard > n {
                return None;
            }
            cur = parents_ref[p];
            Some(p)
        })
    };
    let mut solo_ok = vec![!any_solo; n];
    if any_solo {
        for (i, t) in desc.tracks.iter().enumerate() {
            if matches!(t.kind, TrackKind::Master | TrackKind::Return) {
                solo_ok[i] = true;
            }
            if t.solo {
                solo_ok[i] = true;
                for p in ancestors(i) {
                    solo_ok[p] = true;
                }
                // Every bus the soloed track's output reaches.
                let mut cur = outputs[i];
                let mut guard = 0;
                while let Some(o) = cur {
                    solo_ok[o] = true;
                    cur = outputs[o];
                    guard += 1;
                    if guard > n {
                        break;
                    }
                }
            }
            if ancestors(i).any(|p| desc.tracks[p].solo) {
                solo_ok[i] = true;
            }
        }
    }

    // --- sidechains: each consumer gets its own taps (jobs never share them) ---
    let mut sc_points: Vec<Vec<crate::sidechain::ScPoint>> = vec![Vec::new(); n];
    for p in sc_plans.iter().flat_map(|p| &p.entries) {
        check(p.main_delay)?;
        check(p.sc_delay)?;
        sc_points[p.source].push(p.point);
    }
    // Envelope followers tap the post-fader output (`TrackRt::tap`).
    let mut tapped = vec![false; n];
    for &s in mod_sc_sources.iter().flatten() {
        tapped[s] = true;
    }
    // v0.2: tap points each track must provide (`crate::bus_tap`).
    let mut tap_points: Vec<Vec<InputTap>> = vec![Vec::new(); n];
    for (i, s) in tap_source.iter().enumerate() {
        if let (Some(s), Some(tap)) = (*s, desc.tracks[i].input_tap) {
            tap_points[s].push(tap.point);
        }
    }

    // --- runtime state ---
    let mut tracks = Vec::with_capacity(n);
    let mut send_index = Vec::new();
    for (i, t) in desc.tracks.iter().enumerate() {
        let sidechain = crate::sidechain::Taps::compile(i, &sc_plans[i], config);
        let planned = &sc_plans[i];
        let has_sc = |node: NodeKey| {
            planned
                .entries
                .binary_search_by_key(&node, |e| e.node)
                .is_ok()
        };
        // Pad / rack chain nodes: their planned main-signal delays count as their latency.
        let rack_info = |node: NodeKey| {
            node_info(node).map(|mut info| {
                info.latency = info.latency.saturating_add(planned.extra_latency(node));
                info
            })
        };
        let chain = t
            .chain
            .iter()
            .zip(&chain_info[i])
            .map(|(e, info)| {
                ChainRt::new(
                    e.node,
                    e.enabled,
                    info.channels,
                    has_sc(e.node),
                    config.max_events_per_block,
                )
            })
            .collect();
        let bypass: u32 = t
            .chain
            .iter()
            .zip(&chain_info[i])
            .filter(|(e, _)| !e.enabled)
            .map(|(_, info)| info.latency)
            .sum();
        let mut sends = Vec::with_capacity(t.sends.len());
        for (k, s) in t.sends.iter().enumerate() {
            let target = find(s.to)?;
            send_index.push((s.id, i, k));
            sends.push(SendRt {
                id: s.id,
                target,
                pre_fader: s.pre_fader,
                level: Smoother::new(s.level.max(0.0), MIX_RAMP_MS, sr),
                delay: DelayLine::new(check(in_lat[target] - out_lat[i])?),
                buf: stereo(frames),
            });
        }
        let output_delay = match outputs[i] {
            Some(o) => DelayLine::new(check(in_lat[o] - out_lat[i])?),
            None => DelayLine::new(0),
        };
        tracks.push(TrackRt {
            id: t.id,
            parent: parents[i],
            to_hardware: t.output.is_none() && t.kind == TrackKind::Master,
            chain,
            sends,
            bypass_delay: DelayLine::new(check(bypass)?),
            output_delay,
            volume: Smoother::new(t.volume.max(0.0), MIX_RAMP_MS, sr),
            pan: Smoother::new(t.pan.clamp(-1.0, 1.0), MIX_RAMP_MS, sr),
            gate: Smoother::new(0.0, MIX_RAMP_MS, sr),
            mute: t.mute,
            solo_ok: solo_ok[i],
            audio_input: t.audio_input,
            monitor: t.monitor,
            live: crate::recording::LiveNotes::new(),
            a: stereo(frames),
            b: stereo(frames),
            out_events: EventBuffer::with_capacity(config.max_events_per_block),
            notes: Vec::with_capacity(MAX_ACTIVE_NOTES),
            auto_last: vec![f64::NAN; t.automation.len()],
            env_last: vec![f64::NAN; t.clips.iter().map(|c| c.envelopes.len()).sum()],
            env_base: t
                .clips
                .iter()
                .scan(0, |acc, c| {
                    let base = *acc;
                    *acc += c.envelopes.len();
                    Some(base)
                })
                .collect(),
            clip_ends: crate::automation_rt::clip_ends(&t.clips),
            auto_window: (0, 0),
            awake: true,
            auto_dirty: false,
            meter: MeterAccum::default(),
            out_latency: out_lat[i],
            racks: crate::drum_rack::RacksRt::compile(
                &t.racks,
                &t.chain_racks,
                &rack_info,
                &has_sc,
                config,
            ),
            inputs: std::mem::take(&mut bus_inputs[i]),
            sidechain,
            sc_points: crate::sidechain::Points::compile(&sc_points[i], config),
            tap: tapped[i].then(|| stereo(frames)),
            next_note_id: 0,
            src_scratch: if t.kind == TrackKind::Audio {
                vec![0.0; frames * 8 + 64]
            } else {
                Vec::new()
            },
            warp_scratch: t
                .clips
                .iter()
                .any(|c| crate::warp::complex_warp(c).is_some())
                .then(|| crate::warp::WarpScratch::new(f64::from(config.sample_rate), frames)),
            overflow: false,
            underruns: 0,
            chain_racks: crate::rack_chains::ChainRacksRt::compile(
                &t.chain_racks,
                &t.racks,
                &rack_info,
                &has_sc,
                config,
            ),
            modulation: crate::modulation::ModulationRt::compile(
                &t.modulation,
                mod_sc_sources[i].clone(),
                crate::modulation::SidechainLatency {
                    sources: mod_sc_sources[i]
                        .iter()
                        .map(|&s| (desc.tracks[s].id, out_lat[s]))
                        .collect(),
                    host_in: t
                        .chain
                        .iter()
                        .zip(&chain_info[i])
                        .scan(in_lat[i], |acc, (e, info)| {
                            let at = *acc;
                            *acc = acc.saturating_add(info.latency);
                            Some((e.node, at))
                        })
                        .collect(),
                },
                config,
            ),
            input_tap: crate::bus_tap::InputTapRt::compile(
                tap_source[i],
                t.input_tap.map(|tap| tap.point),
                tap_delay[i],
                config,
            ),
            taps: crate::bus_tap::TapBuffers::compile(&tap_points[i], config),
            vca: crate::vca::TrackVcaRt::compile(t.vca, &desc.vcas, config),
            expression: crate::expression::ExpressionRt::compile(&t.expression),
        });
    }
    send_index.sort();

    let latency = tracks
        .iter()
        .filter(|t| t.to_hardware)
        .map(|t| t.out_latency)
        .max()
        .unwrap_or(0);

    let mut rt = SnapshotRt {
        pad_index,
        rack_chain_index,
        vcas: crate::vca::VcaRt::compile(&desc.vcas, &desc.tracks, config),
        order,
        level_order,
        levels,
        tracks,
        track_index,
        send_index,
        node_index,
        latency,
    };
    // Initial gate values (no ramp for a fresh graph; `inherit` ramps on swaps).
    rt.update_gates();
    for t in &mut rt.tracks {
        let g = t.gate.target();
        t.gate.set_immediate(g);
    }

    let tempo = TempoMapRt::compile(&desc.tempo, &desc.signatures);
    Ok(RenderSnapshot { desc, tempo, rt })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ether_protocol::model::Ulid;

    fn t(id: u128, kind: TrackKind, output: Option<u128>) -> TrackDesc {
        TrackDesc {
            modulation: Default::default(),
            vca: Default::default(),
            chain_racks: Default::default(),
            frozen: Default::default(),
            input_tap: Default::default(),
            id: TrackId(Ulid(id)),
            kind,
            chain: vec![],
            output: output.map(|o| TrackId(Ulid(o))),
            group: None,
            sends: vec![],
            volume: 1.0,
            pan: 0.0,
            mute: false,
            solo: false,
            audio_input: None,
            midi_input: Default::default(),
            monitor: false,
            armed: false,
            clips: vec![],
            automation: vec![],
            racks: vec![],
            expression: Default::default(),
            hw_io: Vec::new(),
            sidechain_taps: Vec::new(),
        }
    }

    fn key(i: u32) -> NodeKey {
        NodeKey {
            index: i,
            generation: 1,
        }
    }

    /// Levels follow outputs, sends and sidechains; each level lists pinned tracks first;
    /// bus inputs keep the sequential summation order.
    #[test]
    fn levels_partition_the_dag() {
        let mut t1 = t(10, TrackKind::Midi, Some(2));
        t1.sends.push(SendDesc {
            id: SendId(Ulid(1)),
            to: TrackId(Ulid(3)),
            level: 1.0,
            pre_fader: false,
        });
        let mut t2 = t(11, TrackKind::Audio, Some(1));
        t2.chain.push(ChainEntry {
            node: key(0),
            enabled: true,
            sidechain: Some(TrackId(Ulid(10))),
        });
        t2.clips.push(ClipDesc {
            id: ClipId(Ulid(1)),
            start: 0.0,
            length: 1.0,
            offset: 0.0,
            looping: None,
            muted: false,
            content: ClipContentDesc::Audio {
                media: MediaId(Ulid(1)),
                gain: 1.0,
                transpose: 0.0,
                fade_in: 0.0,
                fade_out: 0.0,
                warp: Some(WarpDesc {
                    mode: WarpMode::Complex,
                    markers: vec![(0.0, 0.0), (1.0, 1.0)],
                }),
                fade_in_curve: FadeCurve::default(),
                fade_out_curve: FadeCurve::default(),
                reversed: false,
                gain_envelope: None,
                transpose_envelope: None,
            },
            envelopes: vec![],
        });
        let t3 = t(12, TrackKind::Midi, Some(2));
        let desc = RenderGraphDesc {
            tracks: vec![
                t(1, TrackKind::Master, None),
                t(2, TrackKind::Group, Some(1)),
                t(3, TrackKind::Return, Some(1)),
                t1,
                t2,
                t3,
            ],
            ..Default::default()
        };
        let snap = compile(desc, &EngineConfig::default()).unwrap();
        let rt = &snap.rt;
        let level_of = |ti: usize| {
            rt.levels
                .iter()
                .position(|l| rt.level_order[l.start..l.end].contains(&ti))
                .unwrap()
        };
        // t1, t3 | group, return, t2 (sidechained by t1; Complex clips, not pinned) | master
        assert_eq!(rt.levels.len(), 3);
        assert_eq!(level_of(3), 0);
        assert_eq!(level_of(5), 0);
        assert_eq!(level_of(4), 1);
        assert_eq!(level_of(1), 1);
        assert_eq!(level_of(2), 1);
        assert_eq!(level_of(0), 2);
        assert!(rt.levels.iter().all(|l| l.pinned == 0));
        assert!(rt.tracks[4].warp_scratch.is_some());
        for (ti, track) in rt.tracks.iter().enumerate() {
            for input in &track.inputs {
                let (BusInput::Output(c) | BusInput::Send(c, _)) = *input;
                assert!(level_of(c) < level_of(ti));
            }
            for s in track.sidechain.sources() {
                assert!(level_of(s) < level_of(ti));
            }
        }
        assert_eq!(rt.tracks[4].sidechain.sources(), vec![3]);
        let post_fader = crate::sidechain::ScPoint::POST_FADER;
        assert!(
            rt.tracks[3].sc_points.get(post_fader).is_some()
                && rt.tracks[5].sc_points.get(post_fader).is_none()
        );
        // Group inputs in processing order (t1 before t3); the return gets t1's send.
        assert_eq!(
            rt.tracks[1].inputs,
            vec![BusInput::Output(3), BusInput::Output(5)]
        );
        assert_eq!(rt.tracks[2].inputs, vec![BusInput::Send(3, 0)]);
        // Master: in processing order (t1, return, t2, t3, group, master).
        assert_eq!(rt.order, vec![3, 2, 4, 5, 1, 0]);
        assert_eq!(
            rt.tracks[0].inputs,
            vec![
                BusInput::Output(2),
                BusInput::Output(4),
                BusInput::Output(1)
            ]
        );
    }

    fn chain_of(node_keys: &[u32]) -> Vec<ChainEntry> {
        node_keys
            .iter()
            .map(|&i| ChainEntry {
                node: key(i),
                enabled: true,
                sidechain: None,
            })
            .collect()
    }

    /// Issue #569: a node reporting `u32::MAX` latency is clamped to the PDC limit (no
    /// wrapping sum, no debug-overflow panic, no multi-GB delay line).
    #[test]
    fn absurd_node_latency_is_clamped_to_the_pdc_limit() {
        let config = EngineConfig::default();
        let cap = max_latency_samples(config.sample_rate);
        let mut audio = t(10, TrackKind::Audio, Some(1));
        audio.chain = chain_of(&[0]);
        let desc = RenderGraphDesc {
            tracks: vec![
                t(1, TrackKind::Master, None),
                audio,
                t(11, TrackKind::Audio, Some(1)),
            ],
            ..Default::default()
        };
        let info = |_: NodeKey| {
            Some(NodeInfo {
                latency: u32::MAX,
                channels: (2, 2),
            })
        };
        let snap = compile_with(desc, &config, &info).unwrap();
        assert_eq!(snap.latency(), cap);
        assert_eq!(snap.track_latency(TrackId(Ulid(10))), Some(cap));
    }

    /// Max-latency nodes in series exceed the limit: compensating the parallel dry track is
    /// a capacity error, not a wrapped (and silently accepted) sum.
    #[test]
    fn huge_latencies_in_series_are_a_capacity_error() {
        let mut audio = t(10, TrackKind::Audio, Some(1));
        audio.chain = chain_of(&[0, 1, 2]);
        let desc = RenderGraphDesc {
            tracks: vec![
                t(1, TrackKind::Master, None),
                audio,
                t(11, TrackKind::Audio, Some(1)),
            ],
            ..Default::default()
        };
        let info = |_: NodeKey| {
            Some(NodeInfo {
                latency: u32::MAX - 1,
                channels: (2, 2),
            })
        };
        let err = compile_with(desc, &EngineConfig::default(), &info)
            .err()
            .unwrap();
        assert!(matches!(err, CompileError::Capacity(_)), "{err:?}");
    }
}

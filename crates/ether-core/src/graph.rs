//! What the engine renders: [`RenderGraphDesc`] (plain data, built by the controller from
//! the model) and [`RenderSnapshot`] (compiled by the engine off the audio thread).
//!
//! The desc is `Serialize`/`Deserialize` so the web host can ship it from the controller
//! Worker to the AudioWorklet. It references stateful nodes by [`NodeKey`] and audio by
//! `MediaId` (sources registered with `EngineHandle::add_source`), never by pointer.

use ether_protocol::model::{
    AutomationTarget, ClipId, CurveShape, MediaId, ParamId, SendId, TrackId, TrackKind, WarpMode,
};
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::config::EngineConfig;
use crate::delay::DelayLine;
use crate::event::EventBuffer;
use crate::mixer::{
    ChainRt, MAX_ACTIVE_NOTES, MAX_PENDING_EVENTS, MIX_RAMP_MS, MeterAccum, SendRt, Stereo,
    TrackRt, stereo,
};
use crate::node::NodeKey;
use crate::param::Smoother;
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
    /// Hardware input channels monitored/recorded by this track, if any, as `(first, count)`
    /// (count 1 = mono, 2 = stereo; see `crate::recording::input_channels`).
    pub audio_input: Option<(u16, u16)>,
    /// Effective monitoring (controller resolves Auto/In/Off + its runtime arm state).
    pub monitor: bool,
    /// Runtime record-arm state (from the controller, not the document).
    pub armed: bool,
    /// The track's clips, sorted by start.
    pub clips: Vec<ClipDesc>,
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

pub(crate) struct SnapshotRt {
    /// Track indices (into `desc.tracks` / `tracks`) in processing order.
    pub order: Vec<usize>,
    pub tracks: Vec<TrackRt>,
    /// Input bus of every track (sum of routed tracks and sends).
    pub buses: Vec<Stereo>,
    /// Sorted lookups (binary search on the audio thread).
    pub track_index: Vec<(TrackId, usize)>,
    pub send_index: Vec<(SendId, usize, usize)>,
    pub node_index: Vec<(NodeKey, usize, usize)>,
    /// Total latency from the timeline to the hardware output (samples).
    pub latency: u32,
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

    pub(crate) fn lookup_node(&self, node: NodeKey) -> Option<(usize, usize)> {
        self.node_index
            .binary_search_by(|e| e.0.cmp(&node))
            .ok()
            .map(|i| (self.node_index[i].1, self.node_index[i].2))
    }

    /// RT. Recompute every track's mute/solo gate target (mute propagates from groups).
    pub(crate) fn update_gates(&mut self) {
        let n = self.tracks.len();
        for i in 0..n {
            let mut muted = false;
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

/// Longest PDC delay the compiler accepts (seconds of audio).
const MAX_PDC_SECONDS: u32 = 4;

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

    // --- topological sort (Kahn; ties broken by desc order, so it is deterministic) ---
    let mut indeg = vec![0usize; n];
    for s in &succ {
        for &d in s {
            indeg[d] += 1;
        }
    }
    let mut ready: BTreeSet<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
    let mut order = Vec::with_capacity(n);
    while let Some(i) = ready.pop_first() {
        order.push(i);
        for &d in &succ[i] {
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

    // --- nodes ---
    let mut node_index = Vec::new();
    let mut chain_info: Vec<Vec<NodeInfo>> = Vec::with_capacity(n);
    for (i, t) in desc.tracks.iter().enumerate() {
        let mut infos = Vec::with_capacity(t.chain.len());
        for (k, e) in t.chain.iter().enumerate() {
            let info = node_info(e.node).ok_or(CompileError::UnknownNode(e.node))?;
            infos.push(info);
            node_index.push((e.node, i, k));
        }
        chain_info.push(infos);
    }
    node_index.sort();
    if let Some(w) = node_index.windows(2).find(|w| w[0].0 == w[1].0) {
        return Err(CompileError::Capacity(format!(
            "node {:?} used more than once",
            w[0].0
        )));
    }

    // --- PDC ---
    // A track's output latency is its input latency plus its whole chain's latency
    // (bypassed devices included: they are replaced by an equal delay, so a track's
    // latency never depends on bypass state). A bus's input latency is the max over its
    // inputs; every connection into it is delayed by the difference.
    let chain_lat: Vec<u32> = chain_info
        .iter()
        .map(|c| c.iter().map(|i| i.latency).sum())
        .collect();
    let mut in_lat = vec![0u32; n];
    let mut out_lat = vec![0u32; n];
    for &i in &order {
        out_lat[i] = in_lat[i] + chain_lat[i];
        for &d in &succ[i] {
            in_lat[d] = in_lat[d].max(out_lat[i]);
        }
    }
    let max_delay = MAX_PDC_SECONDS * config.sample_rate;
    let check = |d: u32| -> Result<usize, CompileError> {
        if d > max_delay {
            Err(CompileError::Capacity(format!("PDC delay of {d} samples")))
        } else {
            Ok(d as usize)
        }
    };

    // --- solo ---
    // While anything is soloed, a track stays audible if it is soloed, inside a soloed
    // group, a group on the path of a soloed track, a return, or the master.
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
            }
            if ancestors(i).any(|p| desc.tracks[p].solo) {
                solo_ok[i] = true;
            }
        }
    }

    // --- runtime state ---
    let mut tracks = Vec::with_capacity(n);
    let mut send_index = Vec::new();
    for (i, t) in desc.tracks.iter().enumerate() {
        let chain = t
            .chain
            .iter()
            .zip(&chain_info[i])
            .map(|(e, info)| ChainRt {
                key: e.node,
                enabled: e.enabled,
                channels: info.channels,
                events: EventBuffer::with_capacity(config.max_events_per_block),
                pending: EventBuffer::with_capacity(MAX_PENDING_EVENTS),
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
            });
        }
        let output_delay = match outputs[i] {
            Some(o) => DelayLine::new(check(in_lat[o] - out_lat[i])?),
            None => DelayLine::new(0),
        };
        tracks.push(TrackRt {
            id: t.id,
            output: outputs[i],
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
            a: stereo(frames),
            b: stereo(frames),
            scratch: stereo(frames),
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
            auto_dirty: false,
            meter: MeterAccum::default(),
            out_latency: out_lat[i],
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
        order,
        tracks,
        buses: (0..n).map(|_| stereo(frames)).collect(),
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

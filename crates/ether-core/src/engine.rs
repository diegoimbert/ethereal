//! The engine and its control/GC halves.
//!
//! # Threading
//! - [`EngineHandle`] (controller thread) prepares nodes, compiles snapshots and pushes
//!   them, with parameter changes and transport controls, through SPSC rings.
//! - [`Engine`] (audio thread) drains those rings at the start of every block, renders,
//!   and pushes meters/diagnostics back plus everything it retires (old snapshots,
//!   removed nodes, replaced sources) to the [`GarbageCollector`] ring. It never frees.
//! - The playhead is published through a seqlock of atomics (no ring, always the latest).
//!
//! # Rendering one block
//! 1. Drain controls (node inserts/removals, snapshot swap, sources, transport) and
//!    parameter changes.
//! 2. Split the block at loop ends and tempo/time-signature boundaries so transport
//!    information is linear within each sub-block.
//! 3. Per sub-block, process tracks level by level of the routing DAG, one job per track
//!    (in parallel through the host's [`crate::parallel::ParallelExecutor`], see there):
//!    input bus gathered from the finished sources (+ monitored hardware input) → clips
//!    (audio rendered, MIDI notes scheduled sample-accurately) → automation → device chain →
//!    pre-fader sends → fader/pan/mute gate → post-fader sends → meter → PDC-aligned output
//!    in the track's own buffers. Then master goes to the hardware.
//! 4. Publish playhead; every ~33 ms push meter readings.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};

use ether_protocol::meters::TrackMeter;
use ether_protocol::model::{InputTap, MediaId, TrackId, TrackKind};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::automation_rt::{self, apply_automation};
use crate::buffer::AudioBuffers;
use crate::config::{EngineConfig, PrepareConfig};
use crate::event::{EventKind, ProcessEvent};
use crate::graph::{
    ClipContentDesc, CompileError, NodeInfo, RenderGraphDesc, RenderSnapshot, SnapshotRt,
    compile_with,
};
use crate::media::AudioSource;
use crate::meter::EngineOutputs;
use crate::mixer::{BusInput, TrackRt, apply_fader, scale};
use crate::node::{Node, NodeKey, ProcessContext};
use crate::param::{ParamChange, ParamTarget};
use crate::sched::{self, NoteSink, Timing};
use crate::transport::{PlayheadState, TransportControl, TransportInfo};

/// Registered audio sources (media) the engine can hold.
pub const MAX_SOURCES: usize = 4096;
/// Beat tolerance for loop/boundary decisions.
const EPS: f64 = 1e-9;

/// The three halves returned by [`create`].
pub struct EngineParts {
    pub engine: Engine,
    pub handle: EngineHandle,
    pub gc: GarbageCollector,
}

enum Control {
    AddNode {
        key: NodeKey,
        node: Box<dyn Node>,
    },
    RemoveNode {
        key: NodeKey,
    },
    NodeData {
        key: NodeKey,
        data: crate::node::NodeData,
    },
    Swap(Box<RenderSnapshot>),
    AddSource {
        media: MediaId,
        source: Arc<dyn AudioSource>,
    },
    RemoveSource {
        media: MediaId,
    },
    Transport(TransportControl),
    Preview(crate::preview::PreviewControl),
    StreamTap(Option<Box<crate::stream_tap::StreamTapWriter>>),
    /// v0.2: start/stop collecting a node's analysis frames (`crate::analysis`).
    AnalysisWatch {
        key: NodeKey,
        on: bool,
    },
}

enum Garbage {
    Snapshot(#[allow(dead_code)] Box<RenderSnapshot>),
    Node(#[allow(dead_code)] Box<dyn Node>),
    Data(#[allow(dead_code)] crate::node::NodeData),
    Source(#[allow(dead_code)] Arc<dyn AudioSource>),
    StreamTap(#[allow(dead_code)] Box<crate::stream_tap::StreamTapWriter>),
}

enum Output {
    Meter(TrackMeter),
    Overflow,
    Underruns(u32),
    PreviewEnded(u64),
}

/// Single-writer seqlock holding the latest playhead.
#[derive(Default)]
struct SharedPlayhead {
    seq: AtomicU64,
    flags: AtomicU32,
    position: AtomicU64,
    seconds: AtomicU64,
    bpm: AtomicU64,
    sample_time: AtomicU64,
}

impl SharedPlayhead {
    fn write(&self, p: &PlayheadState) {
        let s = self.seq.load(Ordering::Relaxed);
        self.seq.store(s.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);
        self.flags.store(
            p.playing as u32 | ((p.recording as u32) << 1),
            Ordering::Relaxed,
        );
        self.position
            .store(p.position.0.to_bits(), Ordering::Relaxed);
        self.seconds.store(p.seconds.to_bits(), Ordering::Relaxed);
        self.bpm.store(p.bpm.to_bits(), Ordering::Relaxed);
        self.sample_time.store(p.sample_time, Ordering::Relaxed);
        self.seq.store(s.wrapping_add(2), Ordering::Release);
    }

    fn read(&self) -> PlayheadState {
        loop {
            let s1 = self.seq.load(Ordering::Acquire);
            if s1 & 1 == 1 {
                std::hint::spin_loop();
                continue;
            }
            let flags = self.flags.load(Ordering::Relaxed);
            let p = PlayheadState {
                playing: flags & 1 != 0,
                recording: flags & 2 != 0,
                position: ether_protocol::model::Beats(f64::from_bits(
                    self.position.load(Ordering::Relaxed),
                )),
                seconds: f64::from_bits(self.seconds.load(Ordering::Relaxed)),
                bpm: f64::from_bits(self.bpm.load(Ordering::Relaxed)),
                sample_time: self.sample_time.load(Ordering::Relaxed),
            };
            fence(Ordering::Acquire);
            if self.seq.load(Ordering::Relaxed) == s1 {
                return p;
            }
        }
    }
}

pub(crate) struct NodeSlot {
    pub(crate) generation: u32,
    pub(crate) node: Option<Box<dyn Node>>,
}

impl std::fmt::Debug for NodeSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeSlot")
            .field("generation", &self.generation)
            .field("live", &self.node.is_some())
            .finish()
    }
}

impl NodeSlot {
    /// RT. The live node behind `key`, if the slot still holds that generation (also used
    /// by `crate::drum_rack`, which processes pad-chain nodes).
    pub(crate) fn get(slots: &mut [NodeSlot], key: NodeKey) -> Option<&mut Box<dyn Node>> {
        let slot = slots.get_mut(key.index as usize)?;
        if slot.generation != key.generation {
            return None;
        }
        slot.node.as_mut()
    }
}

#[derive(Default)]
struct TransportRt {
    playing: bool,
    recording: bool,
    /// Song position in beats (start of the next sub-block).
    position: f64,
    sample_time: u64,
    /// `SetLoop` override until the next snapshot swap.
    loop_override: Option<(bool, f64, f64)>,
    /// Note-offs for every sounding clip note at the next sub-block.
    release_notes: bool,
    /// `AllNotesOff` to every node at the next sub-block.
    all_notes_off: bool,
    /// `Node::reset` on every node before the next sub-block.
    reset_nodes: bool,
    /// Note chasing on the next played sub-block: start the clip notes already sounding at
    /// the position (after Play, Locate or a loop jump).
    chase_notes: bool,
    /// The timeline jumps at the next sub-block (Play from stopped, Locate, loop wrap);
    /// reported to the stream tap ([`crate::stream_tap::StreamBlock::jump`]), then cleared.
    jump: bool,
    /// Exact tempo-ramp integration (`sched::Exact`, CONTRACTS.md §12.7): `(beat, seconds,
    /// sample_time)` of the last timeline jump while playing a ramped tempo map. Every
    /// position since is the tempo map's closed form of the sample clock. `None` = re-anchor
    /// at `position` (after Play, Stop, Locate, a loop wrap, a tempo/signature boundary, which
    /// restarts the timeline on a whole sample as in v0.1, or a new tempo map).
    anchor: Option<(f64, f64, u64)>,
}

/// Create an engine. Non-RT (allocates every ring and table up front).
pub fn create(config: EngineConfig) -> EngineParts {
    let (control_tx, control_rx) = RingBuffer::new(config.control_queue_capacity.max(1));
    let (param_tx, param_rx) = RingBuffer::new(config.param_queue_capacity.max(1));
    let (out_tx, out_rx) = RingBuffer::new(config.output_queue_capacity.max(1));
    // Every control message retires at most one object; leave headroom for sources.
    let (gc_tx, gc_rx) =
        RingBuffer::new((config.control_queue_capacity + config.max_nodes).max(16) * 2);
    let node_latency: Arc<[AtomicU32]> = (0..config.max_nodes)
        .map(|_| AtomicU32::new(0))
        .collect::<Vec<_>>()
        .into();
    let playhead = Arc::new(SharedPlayhead::default());
    let (recording_rt, recording_io) = crate::recording::channel(&config);
    let (warp_handle, warp_rt) = crate::warp::channel(&config);
    let snapshot =
        compile_with(RenderGraphDesc::default(), &config, &|_| None).expect("empty graph compiles");
    let tempo_bpm = snapshot.tempo.bpm_at(0.0);
    let (analysis_rt, analysis_rx) = crate::analysis::AnalysisRt::new(config.sample_rate);
    playhead.write(&PlayheadState {
        bpm: tempo_bpm,
        ..Default::default()
    });

    let engine = Engine {
        nodes: (0..config.max_nodes)
            .map(|_| NodeSlot {
                generation: 0,
                node: None,
            })
            .collect(),
        node_latency: node_latency.clone(),
        sources: Vec::with_capacity(MAX_SOURCES),
        snapshot: Box::new(snapshot),
        control: control_rx,
        params: param_rx,
        garbage: gc_tx,
        out: out_tx,
        playhead: playhead.clone(),
        transport: TransportRt::default(),
        meter_interval: (config.sample_rate as usize / 30).max(1),
        meter_elapsed: 0,
        overflow: false,
        underruns: 0,
        leaked: 0,
        recording: recording_rt,
        metronome: crate::metronome::Metronome::new(config.sample_rate as f32),
        preview: crate::preview::PreviewVoice::new(config.max_block_size),
        stream_tap: Default::default(),
        executor: Box::new(crate::parallel::SequentialExecutor),
        warp: warp_rt,
        analysis: analysis_rt,
        beat_table: vec![0.0; config.max_block_size + 1],
        config: config.clone(),
    };
    let handle = EngineHandle {
        control: control_tx,
        params: param_tx,
        out: out_rx,
        slots: (0..config.max_nodes)
            .map(|_| HandleSlot {
                generation: 0,
                live: false,
                channels: (2, 2),
            })
            .collect(),
        free: (0..config.max_nodes as u32).rev().collect(),
        node_latency,
        playhead,
        recording: Some(recording_io),
        latency: 0,
        warp: warp_handle,
        analysis: analysis_rx,
        config,
    };
    EngineParts {
        engine,
        handle,
        gc: GarbageCollector { rx: gc_rx },
    }
}

/// Audio-thread half. Owns the node table and the current snapshot.
pub struct Engine {
    config: EngineConfig,
    nodes: Vec<NodeSlot>,
    node_latency: Arc<[AtomicU32]>,
    /// Sorted by media id.
    sources: Vec<(MediaId, Arc<dyn AudioSource>)>,
    snapshot: Box<RenderSnapshot>,
    control: Consumer<Control>,
    params: Consumer<ParamChange>,
    garbage: Producer<Garbage>,
    out: Producer<Output>,
    playhead: Arc<SharedPlayhead>,
    transport: TransportRt,
    meter_interval: usize,
    meter_elapsed: usize,
    overflow: bool,
    underruns: u32,
    /// Objects that could not be handed to the GC (ring full) and were leaked instead of
    /// being freed on the audio thread.
    leaked: u64,
    /// Recording hooks: input capture, live MIDI in/out ([`crate::recording`]).
    pub(crate) recording: crate::recording::RecordingRt,
    /// Click generator ([`crate::metronome`], roadmap v2).
    metronome: crate::metronome::Metronome,
    /// Browser preview voice ([`crate::preview`]).
    preview: crate::preview::PreviewVoice,
    /// "Listen on <peer>" tap ([`crate::stream_tap`], base-53): after master + metronome,
    /// before the preview voice.
    stream_tap: crate::stream_tap::StreamTap,
    /// Parallel track processing ([`crate::parallel`], roadmap v2; `multicore` node).
    /// Sequential until a host injects one with [`Engine::set_executor`].
    executor: Box<dyn crate::parallel::ParallelExecutor>,
    pub(crate) warp: crate::warp::WarpRt,
    /// Device → UI analysis frames ([`crate::analysis`], v0.2).
    analysis: crate::analysis::AnalysisRt,
    /// Exact beats of the current sub-block's samples on ramped tempo maps
    /// (`sched::Exact::beats`, `max_block_size + 1` entries).
    beat_table: Vec<f64>,
}

impl Engine {
    /// **RT.** Render one block. `inputs`/`outputs` are planar, each `frames` long
    /// (`frames <= max_block_size`; channel counts as configured, extra channels ignored).
    ///
    /// Per block: drain control ring (node inserts/removals, snapshot swap, sources,
    /// transport), drain param ring, render (splitting at loop/tempo boundaries), write
    /// meters/playhead to the output ring, push retired objects to the GC ring. Never
    /// allocates, locks or blocks; bounded by graph size.
    pub fn process(&mut self, inputs: &[&[f32]], outputs: &mut [&mut [f32]], frames: usize) {
        // Zero everything the host asked for; only `max_block_size` frames are rendered
        // (hosts with larger buffers call `process` repeatedly), the rest stays silent.
        for out in outputs.iter_mut() {
            let n = frames.min(out.len());
            out[..n].fill(0.0);
        }
        let frames = frames.min(self.config.max_block_size);
        self.drain_control();
        self.warp.drain();
        self.drain_params();
        self.update_latencies();

        let mut done = 0;
        while done < frames {
            done += self.render_sub(inputs, outputs, done, frames - done);
        }

        // --- v0.2 analysis channel (`crate::analysis`): after every track job ---
        if self.analysis.due(frames) {
            // Modulation readback first, so device frames filling the ring can't starve it.
            crate::modulation::readback(&mut self.snapshot.rt.tracks, &mut self.analysis);
            self.analysis.collect_all(&mut self.nodes);
        }

        self.publish_playhead();
        self.meter_elapsed += frames;
        if self.meter_elapsed >= self.meter_interval {
            self.meter_elapsed = 0;
            self.push_meters();
        }
        self.report();
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// Non-RT (call before the audio thread starts processing, or while it is stopped):
    /// the executor used to process independent tracks in parallel (roadmap v2,
    /// `multicore`; see [`crate::parallel`]). Ignored on wasm32 (single-threaded).
    pub fn set_executor(&mut self, executor: Box<dyn crate::parallel::ParallelExecutor>) {
        if cfg!(target_arch = "wasm32") {
            return;
        }
        self.executor = executor;
    }

    /// Objects leaked because the GC ring was full (should stay 0).
    pub fn leaked(&self) -> u64 {
        self.leaked
    }

    fn retire(&mut self, g: Garbage) {
        if let Err(rtrb::PushError::Full(g)) = self.garbage.push(g) {
            // Never free on the audio thread.
            std::mem::forget(g);
            self.leaked += 1;
        }
    }

    fn drain_control(&mut self) {
        while let Ok(msg) = self.control.pop() {
            match msg {
                Control::AddNode { key, node } => {
                    let i = key.index as usize;
                    if i >= self.nodes.len() {
                        self.retire(Garbage::Node(node));
                        continue;
                    }
                    self.node_latency[i].store(node.latency(), Ordering::Relaxed);
                    self.analysis.on_add(key, node.as_ref());
                    let slot = &mut self.nodes[i];
                    slot.generation = key.generation;
                    if let Some(old) = slot.node.replace(node) {
                        self.retire(Garbage::Node(old));
                    }
                }
                Control::RemoveNode { key } => {
                    self.analysis.on_remove(key);
                    let old = self.nodes.get_mut(key.index as usize).and_then(|slot| {
                        if slot.generation == key.generation {
                            slot.node.take()
                        } else {
                            None
                        }
                    });
                    if let Some(old) = old {
                        self.retire(Garbage::Node(old));
                    }
                }
                Control::NodeData { key, data } => {
                    let rejected = match NodeSlot::get(&mut self.nodes, key) {
                        Some(node) => node.set_data(data),
                        None => Some(data),
                    };
                    if let Some(old) = rejected {
                        self.retire(Garbage::Data(old));
                    }
                }
                Control::Swap(mut new) => {
                    {
                        let old = &mut self.snapshot.rt;
                        let fresh = &mut new.rt;
                        for t in fresh.tracks.iter_mut() {
                            if let Some(oi) = old.lookup_track(t.id) {
                                t.inherit(&mut old.tracks[oi]);
                            }
                        }
                        fresh.vcas.inherit(&mut old.vcas);
                    }
                    self.transport.loop_override = None;
                    if new.tempo != self.snapshot.tempo {
                        // Re-anchor exact tempo integration on the new map (`sched::Exact`).
                        self.transport.anchor = None;
                    }
                    let old = std::mem::replace(&mut self.snapshot, new);
                    self.retire(Garbage::Snapshot(old));
                }
                Control::AddSource { media, source } => {
                    match self.sources.binary_search_by(|e| e.0.cmp(&media)) {
                        Ok(i) => {
                            let old = std::mem::replace(&mut self.sources[i].1, source);
                            self.retire(Garbage::Source(old));
                        }
                        Err(i) => {
                            if self.sources.len() < self.sources.capacity() {
                                self.sources.insert(i, (media, source));
                            } else {
                                self.retire(Garbage::Source(source));
                            }
                        }
                    }
                }
                Control::RemoveSource { media } => {
                    if let Ok(i) = self.sources.binary_search_by(|e| e.0.cmp(&media)) {
                        let (_, old) = self.sources.remove(i);
                        self.retire(Garbage::Source(old));
                    }
                }
                Control::Transport(t) => self.apply_transport(t),
                Control::Preview(c) => {
                    if let Some(old) = self.preview.control(c) {
                        self.retire(Garbage::Source(old));
                    }
                }
                Control::AnalysisWatch { key, on } => self.analysis.watch(key, on),
                Control::StreamTap(w) => {
                    if let Some(old) = self.stream_tap.set(w) {
                        self.retire(Garbage::StreamTap(old));
                    }
                }
            }
        }
    }

    fn apply_transport(&mut self, control: TransportControl) {
        let t = &mut self.transport;
        if matches!(
            control,
            TransportControl::Play | TransportControl::Stop | TransportControl::Locate { .. }
        ) {
            t.anchor = None;
        }
        match control {
            TransportControl::Play => {
                if !t.playing {
                    t.chase_notes = true;
                    t.jump = true;
                }
                t.playing = true;
            }
            TransportControl::Stop => {
                if t.playing {
                    t.release_notes = true;
                    t.all_notes_off = true;
                }
                t.playing = false;
            }
            TransportControl::Locate { position } => {
                t.position = position.0;
                t.release_notes = true;
                t.all_notes_off = true;
                t.reset_nodes = true;
                t.chase_notes = true;
                t.jump = true;
            }
            TransportControl::SetRecording { enabled } => t.recording = enabled,
            TransportControl::SetLoop { enabled, region } => {
                t.loop_override = Some((enabled, region.start.0, region.end.0));
            }
        }
    }

    fn drain_params(&mut self) {
        let rt = &mut self.snapshot.rt;
        let mut gates_dirty = false;
        while let Ok(change) = self.params.pop() {
            let v = change.value;
            match change.target {
                ParamTarget::TrackVolume { track } => {
                    if let Some(i) = rt.lookup_track(track) {
                        rt.tracks[i].volume.set_target(v.max(0.0) as f32);
                    } else if let Some(dirty) =
                        rt.vcas
                            .set_param(track, None, Some(v.max(0.0) as f32), &mut rt.tracks)
                    {
                        // A VCA fader (`crate::vca`).
                        gates_dirty |= dirty;
                    }
                }
                ParamTarget::TrackPan { track } => {
                    if let Some(i) = rt.lookup_track(track) {
                        rt.tracks[i].pan.set_target(v.clamp(-1.0, 1.0) as f32);
                    }
                }
                ParamTarget::TrackMute { track } => {
                    if let Some(i) = rt.lookup_track(track) {
                        rt.tracks[i].mute = v >= 0.5;
                        gates_dirty = true;
                    } else if let Some(dirty) =
                        rt.vcas
                            .set_param(track, Some(v >= 0.5), None, &mut rt.tracks)
                    {
                        gates_dirty |= dirty;
                    }
                }
                ParamTarget::SendLevel { send } => {
                    if let Some((t, s)) = rt.lookup_send(send) {
                        rt.tracks[t].sends[s].level.set_target(v.max(0.0) as f32);
                    }
                }
                ParamTarget::Modulator { modulator, param } => {
                    // Live modulator param (`crate::modulation`).
                    for t in rt.tracks.iter_mut() {
                        if t.modulation.set_param(modulator, param, v) {
                            break;
                        }
                    }
                }
                ParamTarget::Node { node, param } => {
                    let event = ProcessEvent {
                        offset: 0,
                        kind: EventKind::Param { param, value: v },
                    };
                    // Modulated params and macros: the change sets the base
                    // (`crate::modulation`).
                    let owner = rt
                        .lookup_node(node)
                        .map(|(t, _)| t)
                        .or_else(|| rt.lookup_pad_node(node))
                        .or_else(|| rt.lookup_rack_chain_node(node));
                    if let Some(t) = owner
                        && rt.tracks[t].modulation.intercept(node, param, v)
                    {
                        rt.tracks[t].auto_dirty = true;
                        continue;
                    }
                    if let Some((t, k)) = rt.lookup_node(node) {
                        rt.tracks[t].auto_dirty = true;
                        if !rt.tracks[t].chain[k].pending.push(event) {
                            self.overflow = true;
                        }
                    } else if let Some(t) = rt.lookup_pad_node(node) {
                        // A device on a drum pad (`crate::drum_rack`).
                        rt.tracks[t].auto_dirty = true;
                        if let Some(pending) = rt.tracks[t].racks.pending_mut(node)
                            && !pending.push(event)
                        {
                            self.overflow = true;
                        }
                    } else if let Some(t) = rt.lookup_rack_chain_node(node) {
                        // A device on a rack chain (`crate::rack_chains`).
                        rt.tracks[t].auto_dirty = true;
                        if let Some(pending) = rt.tracks[t].chain_racks.pending_mut(node)
                            && !pending.push(event)
                        {
                            self.overflow = true;
                        }
                    }
                }
            }
        }
        if gates_dirty {
            rt.update_gates();
        }
    }

    fn update_latencies(&mut self) {
        let rt = &self.snapshot.rt;
        let keys = rt
            .node_index
            .iter()
            .map(|e| e.0)
            .chain(rt.pad_index.iter().map(|e| e.0))
            .chain(rt.rack_chain_index.iter().map(|e| e.0));
        for key in keys {
            let i = key.index as usize;
            if let Some(slot) = self.nodes.get(i)
                && slot.generation == key.generation
                && let Some(node) = &slot.node
            {
                self.node_latency[i].store(node.latency(), Ordering::Relaxed);
            }
        }
    }

    fn loop_region(&self) -> (bool, f64, f64) {
        let (on, s, e) = self.transport.loop_override.unwrap_or((
            self.snapshot.desc.loop_enabled,
            self.snapshot.desc.loop_start,
            self.snapshot.desc.loop_end,
        ));
        (on && e - s > 1e-6, s, e)
    }

    /// Render up to `max` frames starting at output offset `off`; returns frames rendered.
    fn render_sub(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        off: usize,
        max: usize,
    ) -> usize {
        let (loop_on, loop_start, loop_end) = self.loop_region();
        let sr = self.config.sample_rate as f64;
        let Engine {
            nodes,
            sources,
            snapshot,
            transport,
            overflow,
            underruns,
            recording,
            warp,
            metronome,
            preview,
            stream_tap,
            executor,
            beat_table,
            ..
        } = self;
        let RenderSnapshot { desc, tempo, rt } = &mut **snapshot;
        let SnapshotRt {
            order,
            level_order,
            levels,
            tracks,
            latency: graph_latency,
            vcas,
            ..
        } = rt;

        // --- sub-block extent ---
        let playing = transport.playing;
        let b0 = transport.position;
        // Ramped tempo maps are integrated exactly from the sample clock (`sched::Exact`,
        // CONTRACTS.md §12.7); others keep the v0.1 math, bit for bit.
        let anchor = if playing && tempo.has_ramps() {
            let sample_time = transport.sample_time;
            Some(
                *transport
                    .anchor
                    .get_or_insert_with(|| (b0, tempo.beats_to_seconds(b0), sample_time)),
            )
        } else {
            transport.anchor = None;
            None
        };
        let since = anchor.map_or(0, |(_, _, at)| transport.sample_time - at);
        let s0 = match anchor {
            Some((_, seconds, _)) if since > 0 => seconds + since as f64 / sr,
            Some((_, seconds, _)) => seconds,
            None => tempo.beats_to_seconds(b0),
        };
        let mut n = max;
        let mut b1 = None;
        let mut hit_loop = false;
        if playing {
            let mut limit = None;
            let mut limit_is_loop = false;
            if loop_on && b0 < loop_end - EPS {
                limit = Some(loop_end);
                limit_is_loop = true;
            }
            if let Some(nb) = tempo.next_boundary(b0)
                && limit.is_none_or(|l| nb < l - EPS)
            {
                limit = Some(nb);
                limit_is_loop = false;
            }
            if let Some(l) = limit {
                let f = ((tempo.beats_to_seconds(l) - s0) * sr - 1e-7)
                    .ceil()
                    .max(1.0) as usize;
                if f <= n {
                    n = f;
                    b1 = Some(l);
                    hit_loop = limit_is_loop;
                }
            }
        }
        // A loop end or tempo/signature boundary ends the sub-block on the first sample at
        // or after it, and the timeline restarts exactly there (v0.1: every boundary lands
        // on a whole sample; exact integration re-anchors on it).
        let split = b1.is_some();
        let b1 = match (playing, b1, anchor) {
            (false, _, _) => b0,
            (true, Some(b), _) => b,
            (true, None, Some((beat, seconds, _))) => {
                sched::exact_beat(tempo, sr, (beat, seconds), since, n as f64)
            }
            (true, None, None) => tempo.seconds_to_beats(s0 + n as f64 / sr),
        };
        let exact = anchor.map(|(beat, seconds, _)| {
            let beats = &mut beat_table[..=n];
            beats[0] = b0;
            for (o, b) in beats.iter_mut().enumerate().skip(1) {
                *b = sched::exact_beat(tempo, sr, (beat, seconds), since, o as f64);
            }
            sched::Exact {
                beat,
                seconds,
                since,
                beats,
            }
        });
        let (signature, bar_start) = tempo.signature_at(b0);
        let info = TransportInfo {
            playing,
            recording: transport.recording,
            sample_time: transport.sample_time,
            position: b0,
            seconds: s0,
            bpm: tempo.bpm_at(b0),
            beats_per_sample: (b1 - b0) / n as f64,
            time_signature: signature,
            bar_start,
            loop_active: loop_on,
            loop_start,
            loop_end,
        };
        let timing = Timing {
            tempo,
            b0,
            b1,
            s0,
            sample_rate: sr,
            frames: n,
            sample_time: transport.sample_time,
            wraps: hit_loop,
            exact,
        };
        recording.process(&info, n, &desc.tracks, tracks);
        // VCA gains and automation for this sub-block (`crate::vca`), before the jobs.
        vcas.update(tracks, &timing, playing);

        // --- tracks, level by level (`crate::parallel`) ---
        // Stopped with nothing live (no monitored input): only tails ring out, so keep the
        // workers parked and run the jobs here, in level order (same result, bit for bit).
        let parallel = playing || tracks.iter().any(|t| t.monitor);
        let ctx = JobCtx {
            tracks: tracks.as_mut_ptr(),
            n_tracks: tracks.len(),
            nodes: nodes.as_mut_ptr(),
            n_nodes: nodes.len(),
            warp: &mut *warp,
            descs: &desc.tracks,
            sources,
            inputs,
            off,
            n,
            sr,
            flags: JobFlags {
                playing,
                release_notes: transport.release_notes,
                all_notes_off: transport.all_notes_off,
                reset_nodes: transport.reset_nodes,
                chase_notes: transport.chase_notes,
            },
            info: &info,
            timing: &timing,
        };
        for level in levels.iter() {
            let idx = &level_order[level.start..level.end];
            // SAFETY (`crate::parallel`, "Unsafe sharing"): job `j` touches track `idx[j]`
            // (distinct within a level), its nodes (each node in one chain) and finished
            // tracks of earlier levels; the executor runs each `j` once, pinned jobs on this
            // thread, and returns after all of them finished.
            let job = |j: usize| unsafe { ctx.run(idx[j]) };
            if idx.len() == 1 || !parallel {
                for j in 0..idx.len() {
                    job(j);
                }
            } else {
                executor.execute_pinned(idx.len(), level.pinned, &job);
            }
        }
        // --- recording capture: hardware input + aligned input taps (`tap-recording`) ---
        recording.capture(&info, inputs, off, n, &desc.tracks, tracks);

        // --- collect job flags; master to the hardware (fixed order) ---
        for &ti in order.iter() {
            let track = &mut tracks[ti];
            *overflow |= std::mem::take(&mut track.overflow);
            *underruns += std::mem::take(&mut track.underruns);
            if track.to_hardware {
                for (ch, out) in outputs.iter_mut().take(2).enumerate() {
                    let end = (off + n).min(out.len());
                    if off >= end {
                        continue;
                    }
                    for (d, s) in out[off..end].iter_mut().zip(&track.a[ch][..n]) {
                        *d += s;
                    }
                }
            }
        }
        // --- metronome (after master reached the hardware outputs; not metered) ---
        if transport.reset_nodes {
            metronome.reset();
        }
        metronome.render(
            &desc.click,
            desc.metronome,
            &info,
            tempo,
            *graph_latency,
            off,
            n,
            outputs,
        );
        // --- stream tap (base-53): master + metronome/count-in, never the preview ---
        stream_tap.write(&info, *graph_latency, transport.jump, off, n, outputs);
        transport.jump = false;
        // --- browser preview (after master; not metered, transport-independent) ---
        preview.render(off, n, outputs);

        // --- advance ---
        transport.release_notes = false;
        transport.all_notes_off = false;
        transport.reset_nodes = false;
        transport.sample_time += n as u64;
        if playing {
            transport.chase_notes = false;
            if hit_loop {
                transport.position = loop_start;
                transport.anchor = None;
                transport.release_notes = true;
                transport.chase_notes = true;
                transport.jump = true;
            } else {
                transport.position = b1;
                if split {
                    transport.anchor = None;
                }
            }
        }
        n
    }

    fn publish_playhead(&mut self) {
        let tempo = &self.snapshot.tempo;
        let pos = self.transport.position;
        self.playhead.write(&PlayheadState {
            playing: self.transport.playing,
            recording: self.transport.recording,
            position: ether_protocol::model::Beats(pos),
            seconds: tempo.beats_to_seconds(pos),
            bpm: tempo.bpm_at(pos),
            sample_time: self.transport.sample_time,
        });
    }

    fn push_meters(&mut self) {
        for t in self.snapshot.rt.tracks.iter_mut() {
            if self.out.slots() == 0 {
                break;
            }
            let m = std::mem::take(&mut t.meter);
            let _ = self.out.push(Output::Meter(TrackMeter {
                track: t.id,
                peak: m.peak,
                rms: m.rms(),
                clipped: m.clipped,
            }));
        }
    }

    fn report(&mut self) {
        if let Some(old) = self.preview.take_retired() {
            self.retire(Garbage::Source(old));
        }
        if let Some(id) = self.preview.ended
            && self.out.push(Output::PreviewEnded(id)).is_ok()
        {
            self.preview.ended = None;
        }
        if self.overflow && self.out.push(Output::Overflow).is_ok() {
            self.overflow = false;
        }
        if self.underruns > 0 && self.out.push(Output::Underruns(self.underruns)).is_ok() {
            self.underruns = 0;
        }
    }
}

/// RT. Keyed access to the engine's node table from one track job (its chain and drum-rack
/// pad chains, `crate::drum_rack`). Every access goes through [`NodeTable::get`], which
/// yields one slot at a time, so no job ever holds a `&mut` over the whole table.
pub(crate) struct NodeTable<'a> {
    ptr: *mut NodeSlot,
    len: usize,
    _slots: std::marker::PhantomData<&'a mut [NodeSlot]>,
}

impl<'a> NodeTable<'a> {
    /// A table with exclusive access to `slots`.
    #[allow(dead_code)]
    pub(crate) fn new(slots: &'a mut [NodeSlot]) -> Self {
        Self {
            len: slots.len(),
            ptr: slots.as_mut_ptr(),
            _slots: std::marker::PhantomData,
        }
    }

    /// One of several tables over the same slots, one per concurrent track job.
    ///
    /// # Safety
    /// `ptr..ptr+len` must be valid `NodeSlot`s for `'a`, not accessed through any other
    /// path for `'a` except other tables made by this function, and every table must only
    /// be asked for keys of *its own* track (its chain and pad chains). That holds for
    /// track jobs (`crate::parallel`, "Unsafe sharing"):
    /// - a *live* key (generation equal to its slot's) is unique in a snapshot: the compiler
    ///   rejects a key used twice, and a slot has one generation, so two live keys never
    ///   share a slot; only live keys reach a `&mut` (to the slot's `node` field);
    /// - a *stale* key (a pad node removed and its slot reused by another track: the
    ///   compiler tolerates unknown pad nodes) only *reads* the slot's `generation`, which
    ///   nothing writes while a level runs, and yields `None`;
    /// - the audio thread doesn't touch the table while a level runs.
    ///
    /// So [`NodeTable::get`] never creates two live `&mut` to one node, nor a `&mut` to a
    /// slot another job uses.
    pub(crate) unsafe fn shared(ptr: *mut NodeSlot, len: usize) -> Self {
        Self {
            ptr,
            len,
            _slots: std::marker::PhantomData,
        }
    }

    /// RT. The live node behind `key`, if the slot still holds that generation.
    pub(crate) fn get(&mut self, key: NodeKey) -> Option<&mut Box<dyn Node>> {
        let i = key.index as usize;
        if i >= self.len {
            return None;
        }
        // Never a reference to the whole slot: read the generation through the raw pointer
        // (nobody writes it while jobs run) and borrow only the `node` field, and only when
        // the key is live. A stale key of another track's slot thus never forms a `&mut`.
        let slot = self.ptr.wrapping_add(i);
        // SAFETY: `i < len`: a valid `NodeSlot`; `generation` is only written on the audio
        // thread between blocks (`drain_control`), never during a level.
        if unsafe { std::ptr::addr_of!((*slot).generation).read() } != key.generation {
            return None;
        }
        // SAFETY: the key is live, so by the `shared` contract (or `new`'s exclusivity) this
        // slot's node belongs to this table's track only; the borrow of `self` keeps it to
        // one node at a time per table.
        unsafe { (*std::ptr::addr_of_mut!((*slot).node)).as_mut() }
    }
}

/// Transport flags of the current sub-block (read-only for the jobs).
#[derive(Clone, Copy)]
struct JobFlags {
    playing: bool,
    release_notes: bool,
    all_notes_off: bool,
    reset_nodes: bool,
    chase_notes: bool,
}

/// What a track job needs: raw pointers to the per-track/per-node state it gets exclusive
/// access to (`crate::parallel`, "Unsafe sharing"), plus read-only sub-block context.
struct JobCtx<'a> {
    tracks: *mut TrackRt,
    n_tracks: usize,
    nodes: *mut NodeSlot,
    n_nodes: usize,
    /// Only dereferenced by pinned jobs (on the audio thread, one at a time).
    warp: *mut crate::warp::WarpRt,
    descs: &'a [crate::graph::TrackDesc],
    sources: &'a [(MediaId, Arc<dyn AudioSource>)],
    inputs: &'a [&'a [f32]],
    off: usize,
    n: usize,
    sr: f64,
    flags: JobFlags,
    info: &'a TransportInfo,
    timing: &'a Timing<'a>,
}

// SAFETY: the shared fields are `Sync` (checked below); the raw pointers are only
// dereferenced by `JobCtx::run` under the level contract: one job per track per level,
// each node reached by one job, finished tracks only read, `warp` only on the audio thread.
// The pointees are `Send` (`TrackRt`, `NodeSlot` via `Node: Send`), so handing them to a
// worker thread is fine.
unsafe impl Sync for JobCtx<'_> {}

const _: () = {
    const fn sync<T: Sync + ?Sized>() {}
    const fn send<T: Send + ?Sized>() {}
    sync::<crate::graph::TrackDesc>();
    sync::<(MediaId, Arc<dyn AudioSource>)>();
    sync::<TransportInfo>();
    sync::<Timing<'static>>();
    send::<TrackRt>();
    send::<NodeSlot>();
};

impl JobCtx<'_> {
    /// RT. Process track `ti` for the sub-block (see [`Engine::process`]).
    ///
    /// # Safety
    /// Called at most once per track per level, never concurrently for one track; every
    /// track in `tracks[ti].inputs` / `sc_sources` finished in an earlier level (the level
    /// partition guarantees it); the tracks of the running level are touched by their own
    /// jobs only; if `tracks[ti].pinned`, this runs on the audio thread (`execute_pinned`).
    unsafe fn run(&self, ti: usize) {
        debug_assert!(ti < self.n_tracks);
        // SAFETY: exclusive by the contract above.
        let track = unsafe { &mut *self.tracks.add(ti) };
        // SAFETY: this job only asks for the keys of its own chain and pad chains.
        let mut nodes = unsafe { NodeTable::shared(self.nodes, self.n_nodes) };
        let n = self.n;

        // --- input bus: gather the finished sources in the fixed order ---
        {
            let TrackRt {
                a,
                inputs,
                sidechain,
                sc_sources,
                ..
            } = &mut *track;
            a[0][..n].fill(0.0);
            a[1][..n].fill(0.0);
            for &input in inputs.iter() {
                let (c, sel) = match input {
                    BusInput::Output(c) => (c, None),
                    BusInput::Send(c, k) => (c, Some(k)),
                };
                debug_assert!(c != ti && c < self.n_tracks);
                // SAFETY: `c != ti` finished in an earlier level; nothing writes it now.
                let src = unsafe { &*self.tracks.add(c) };
                let buf = match sel {
                    None => &src.a,
                    Some(k) => &src.sends[k].buf,
                };
                for ch in 0..2 {
                    for (d, s) in a[ch][..n].iter_mut().zip(&buf[ch][..n]) {
                        *d += s;
                    }
                }
            }
            // Sidechain sources' taps (finished too) into this track's own taps.
            for &s in sc_sources.iter() {
                // SAFETY: as above (sidechain sources are ordered before their consumer).
                let src = unsafe { &*self.tracks.add(s) };
                if let Some(tap) = &src.tap {
                    sidechain.write(s, tap, n);
                }
            }
        }
        // Envelope-follower sidechains (`crate::modulation`, v0.2): sources finished earlier.
        for i in 0..track.modulation.sidechain_sources().len() {
            let s = track.modulation.sidechain_sources()[i];
            if s == ti {
                continue;
            }
            // SAFETY: as above (sidechain sources are ordered before their consumer).
            let src = unsafe { &*self.tracks.add(s) };
            if let Some(tap) = &src.tap {
                track.modulation.write_sidechain(s, tap, n);
            }
        }
        // Track input from another track (`crate::bus_tap`, v0.2): its source finished in an
        // earlier level.
        if let Some(s) = track.input_tap.source
            && s != ti
        {
            // SAFETY: as above (tap sources are ordered before their consumer).
            let src = unsafe { &*self.tracks.add(s) };
            track.input_tap.gather(&src.taps, n);
        }

        let warp = if track.pinned {
            // SAFETY: pinned jobs run on the audio thread, one after the other.
            Some(unsafe { &mut *self.warp })
        } else {
            None
        };
        self.render_track(ti, track, &mut nodes, warp);
    }

    /// RT. The body of a track job, after its input bus was gathered into `track.a`.
    fn render_track(
        &self,
        ti: usize,
        track: &mut TrackRt,
        nodes: &mut NodeTable<'_>,
        mut warp: Option<&mut crate::warp::WarpRt>,
    ) {
        let n = self.n;
        let off = self.off;
        let sr = self.sr;
        let flags = self.flags;
        let playing = flags.playing;
        let timing = self.timing;
        let tempo = timing.tempo;
        let (b0, b1) = (timing.b0, timing.b1);
        let info = self.info;
        let tdesc = &self.descs[ti];
        let TrackRt {
            chain,
            a,
            b,
            out_events,
            notes,
            racks,
            sidechain: rt_sidechain,
            next_note_id,
            src_scratch,
            overflow,
            underruns,
            chain_racks,
            modulation,
            input_tap,
            taps,
            vca,
            ..
        } = track;

        // --- monitored hardware input ---
        if track.monitor
            && let Some((l, r)) = crate::recording::input_channels(track.audio_input)
        {
            for (ch, hw) in [(0usize, l), (1, r)] {
                if let Some(input) = self.inputs.get(hw as usize) {
                    let src = &input[off.min(input.len())..(off + n).min(input.len())];
                    for (d, s) in a[ch].iter_mut().zip(src) {
                        *d += s;
                    }
                }
            }
        }

        // --- track input from another track (`crate::bus_tap`, v0.2) ---
        input_tap.mix_into(a, n, track.monitor);

        // --- events: pending live params ---
        racks.begin_block(flags.all_notes_off);
        chain_racks.begin_block(flags.all_notes_off);
        for c in chain.iter_mut() {
            c.events.clear();
            for e in c.pending.as_slice() {
                c.events.push(*e);
            }
            c.pending.clear();
            if flags.all_notes_off {
                c.events.push(ProcessEvent {
                    offset: 0,
                    kind: EventKind::AllNotesOff,
                });
            }
        }

        // --- clips ---
        if let Some(first) = chain.first_mut() {
            let mut sink = NoteSink {
                events: &mut first.events,
                notes,
                next_note_id,
            };
            if flags.release_notes {
                sink.release_all(0);
            }
            if playing {
                let end = tdesc.clips.partition_point(|c| c.start < b1);
                for clip in &tdesc.clips[..end] {
                    if matches!(clip.content, ClipContentDesc::Midi { .. }) {
                        if flags.chase_notes {
                            sched::chase_notes(clip, timing, &mut sink);
                        }
                        sched::schedule_notes(clip, timing, &mut sink);
                    }
                }
                sink.end_notes(timing);
            }
        } else if flags.release_notes {
            notes.clear();
        }
        // Frozen track (`crate::freeze`, v0.2): the render replaces clips and chain (the
        // controller compiles neither).
        if playing
            && let Some(frozen) = &tdesc.frozen
            && !crate::freeze::render_frozen(frozen, self.sources, info, sr, a, n)
        {
            *underruns += 1;
        }
        if playing && tdesc.kind == TrackKind::Audio {
            let end = tdesc.clips.partition_point(|c| c.start < b1);
            for clip in &tdesc.clips[..end] {
                let ClipContentDesc::Audio { media, .. } = &clip.content else {
                    continue;
                };
                if clip.start + clip.length <= b0 {
                    continue;
                }
                let Ok(si) = self.sources.binary_search_by(|e| e.0.cmp(media)) else {
                    continue;
                };
                let ref_bpm = tempo.bpm_at(clip.start);
                let [al, ar] = &mut *a;
                let out = [&mut al[..n], &mut ar[..n]];
                let source = &*self.sources[si].1;
                // Unpinned tracks have no Complex-warped clip, for which `WarpRt::render`
                // is exactly `sched::render_audio`.
                let ok = match warp.as_deref_mut() {
                    Some(w) => w.render(clip, source, ref_bpm, timing, out, src_scratch),
                    None => sched::render_audio(clip, source, ref_bpm, timing, out, src_scratch),
                };
                if !ok {
                    *underruns += 1;
                }
            }
        }

        taps.write(InputTap::PreFx, a, n);

        // --- automation (`crate::automation_rt`: sample-accurate, CONTRACTS.md §12.7) ---
        // Enabled lanes/envelopes always drive their target (precedence per event, see
        // docs/CONTRACTS.md §4 "Automation precedence"): node params are re-sent after a
        // live change or a locate; mixer targets are ramped in the fader stage below.
        if track.auto_dirty || flags.reset_nodes {
            track.auto_last.fill(f64::NAN);
            track.env_last.fill(f64::NAN);
            track.auto_dirty = false;
        }
        let mixer_auto = apply_automation(
            tdesc,
            timing,
            playing,
            flags.chase_notes,
            &mut track.auto_last,
            &mut track.env_last,
            &track.env_base,
            &mut track.volume,
            &mut track.pan,
            &mut track.sends,
            chain,
            racks,
            chain_racks,
            modulation,
        );
        // --- modulation (`crate::modulation`, v0.2): base + Σ depth · source ---
        modulation.render(timing, info, chain, racks, chain_racks);

        // --- device chain ---
        for k in 0..chain.len() {
            let (head, tail) = chain.split_at_mut(k + 1);
            let entry = &mut head[k];
            entry.events.sort();
            *overflow |= entry.events.overflowed();
            let key = entry.key;
            // Sidechain PDC: delay the main signal before this entry if planned.
            rt_sidechain.align_main(ti, k, a, n);
            // Drum rack: pad chains feed the rack node's input (`crate::drum_rack`).
            if entry.enabled && !racks.is_empty() && racks.is_rack(key) {
                *overflow |= racks.run_pads(
                    key,
                    nodes,
                    entry.events.as_slice(),
                    info,
                    sr as f32,
                    a,
                    n,
                    flags.reset_nodes,
                );
            }
            // Rack chains (`crate::rack_chains`, v0.2): chains feed the rack node's input.
            if entry.enabled && !chain_racks.is_empty() && chain_racks.is_rack(key) {
                *overflow |= chain_racks.run(
                    key,
                    nodes,
                    entry.events.as_slice(),
                    out_events,
                    info,
                    sr as f32,
                    a,
                    n,
                    flags.reset_nodes,
                );
            }
            modulation.pre_node(k, key, entry.events.as_slice(), a, n);
            let Some(node) = nodes.get(key) else {
                continue;
            };
            if flags.reset_nodes {
                node.reset();
            }
            if !entry.enabled {
                // Bypassed = MIDI thru (v0.2 `midi-fx`): its notes reach the next device
                // (its own params don't).
                if let Some(next) = tail.first_mut() {
                    for e in entry.events.as_slice() {
                        if !matches!(e.kind, EventKind::Param { .. }) {
                            next.events.push(*e);
                        }
                    }
                }
                continue;
            }
            out_events.clear();
            let (n_in, n_out) = (
                (entry.channels.0 as usize).min(2),
                (entry.channels.1 as usize).min(2),
            );
            {
                let [al, ar] = &*a;
                let [bl, br] = &mut *b;
                let ins: [&[f32]; 2] = [&al[..n], &ar[..n]];
                let mut outs: [&mut [f32]; 2] = [&mut bl[..n], &mut br[..n]];
                let mut ctx = ProcessContext {
                    sample_rate: sr as f32,
                    frames: n,
                    transport: info,
                    events: entry.events.as_slice(),
                    out_events,
                };
                let mut buffers = AudioBuffers {
                    inputs: &ins[..n_in],
                    outputs: &mut outs[..n_out],
                };
                // Sidechain (`crate::sidechain`): a tapped source's aligned signal.
                match entry
                    .sidechain
                    .and_then(|src| rt_sidechain.read(ti, k, src, n))
                {
                    Some(sc) => node.process_sidechain(&mut ctx, &mut buffers, &sc),
                    None => node.process(&mut ctx, &mut buffers),
                };
            }
            *overflow |= out_events.overflowed();
            if !chain_racks.is_empty() {
                chain_racks.finish(key, out_events);
            }
            match n_out {
                0 => {}
                1 => {
                    let [bl, br] = &mut *b;
                    br[..n].copy_from_slice(&bl[..n]);
                    std::mem::swap(a, b);
                }
                _ => std::mem::swap(a, b),
            }
            // Note/MIDI output feeds the next device.
            if let Some(next) = tail.first_mut() {
                for e in out_events.as_slice() {
                    next.events.push(*e);
                }
            }
        }
        {
            let [al, ar] = &mut *a;
            track.bypass_delay.process(&mut al[..n], &mut ar[..n]);
        }
        taps.write(InputTap::PostFx, a, n);

        // --- sends (pre), fader, sends (post) ---
        // Each send's signal goes to its own buffer, gathered by the destination's job.
        let gate = track.gate.current();
        for pass_pre in [true, false] {
            if !pass_pre {
                // Automated volume/pan: ramped per sample (`crate::automation_rt`).
                if !automation_rt::fader(
                    tdesc,
                    timing,
                    mixer_auto,
                    a,
                    &mut track.volume,
                    &mut track.pan,
                    &mut track.gate,
                ) {
                    apply_fader(a, &mut track.volume, &mut track.pan, &mut track.gate, n);
                }
                // VCA gain (`crate::vca`, v0.2), then the post-fader tap.
                vca.apply(a, n);
                taps.write(InputTap::PostFader, a, n);
            }
            for send in track.sends.iter_mut().filter(|s| s.pre_fader == pass_pre) {
                let [sl, sr_] = &mut send.buf;
                sl[..n].copy_from_slice(&a[0][..n]);
                sr_[..n].copy_from_slice(&a[1][..n]);
                if pass_pre && gate != 1.0 {
                    for s in sl[..n].iter_mut().chain(sr_[..n].iter_mut()) {
                        *s *= gate;
                    }
                }
                send.delay.process(&mut sl[..n], &mut sr_[..n]);
                // Automated send level: ramped per sample (`crate::automation_rt`).
                if !automation_rt::send_level(tdesc, timing, mixer_auto, send) {
                    let [sl, sr_] = &mut send.buf;
                    scale(&mut sl[..n], &mut send.level, false);
                    scale(&mut sr_[..n], &mut send.level, true);
                }
            }
        }

        // --- meter + output ---
        track.meter.add(&a[0][..n], &a[1][..n]);
        // Sidechain tap: post-fader, before the PDC output delay (latency = out_lat).
        if let Some(tap) = &mut track.tap {
            tap[0][..n].copy_from_slice(&a[0][..n]);
            tap[1][..n].copy_from_slice(&a[1][..n]);
        }
        // The output stays in `a`: gathered by the destination bus's job, or summed into
        // the hardware output by the audio thread (master).
        let [al, ar] = &mut *a;
        track.output_delay.process(&mut al[..n], &mut ar[..n]);
    }
}

/// Controller-thread half. All methods are non-blocking; `Err(EngineError::QueueFull)`
/// means the audio thread isn't draining (stalled or not started) and the caller should
/// retry later.
pub struct EngineHandle {
    config: EngineConfig,
    control: Producer<Control>,
    params: Producer<ParamChange>,
    out: Consumer<Output>,
    slots: Vec<HandleSlot>,
    free: Vec<u32>,
    node_latency: Arc<[AtomicU32]>,
    playhead: Arc<SharedPlayhead>,
    pub(crate) recording: Option<crate::recording::RecordingIo>,
    pub(crate) warp: crate::warp::WarpHandle,
    /// Output latency of the last published graph (samples).
    latency: u32,
    /// Analysis frames from the audio thread ([`crate::analysis`], v0.2).
    analysis: Consumer<crate::analysis::AnalysisFrame>,
}

struct HandleSlot {
    generation: u32,
    live: bool,
    channels: (u16, u16),
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EngineError {
    #[error("engine queue full")]
    QueueFull,
    #[error("node table full")]
    TooManyNodes,
    #[error("stale or unknown node key {0:?}")]
    UnknownNode(NodeKey),
    #[error(transparent)]
    Compile(#[from] CompileError),
}

impl EngineHandle {
    fn send(&mut self, msg: Control) -> Result<(), EngineError> {
        self.control.push(msg).map_err(|_| EngineError::QueueFull)
    }

    fn key_live(&self, key: NodeKey) -> bool {
        self.slots
            .get(key.index as usize)
            .is_some_and(|s| s.live && s.generation == key.generation)
    }

    /// Prepare `node` (calls `Node::prepare` here, off the audio thread) and send it to the
    /// engine. The key is valid immediately for use in a `RenderGraphDesc`.
    pub fn add_node(&mut self, mut node: Box<dyn Node>) -> Result<NodeKey, EngineError> {
        let Some(index) = self.free.pop() else {
            return Err(EngineError::TooManyNodes);
        };
        node.prepare(&PrepareConfig {
            sample_rate: self.config.sample_rate as f32,
            max_block_size: self.config.max_block_size,
            max_events_per_block: self.config.max_events_per_block,
        });
        let slot = &mut self.slots[index as usize];
        let key = NodeKey {
            index,
            generation: slot.generation.wrapping_add(1),
        };
        let channels = node.channels();
        self.node_latency[index as usize].store(node.latency(), Ordering::Relaxed);
        if self.control.push(Control::AddNode { key, node }).is_err() {
            self.free.push(index);
            return Err(EngineError::QueueFull);
        }
        let slot = &mut self.slots[index as usize];
        slot.generation = key.generation;
        slot.live = true;
        slot.channels = channels;
        Ok(key)
    }

    /// Remove a node; it is returned through the GC ring and dropped there. Publish a graph
    /// that no longer references it first (or in the same batch).
    pub fn remove_node(&mut self, key: NodeKey) -> Result<(), EngineError> {
        if !self.key_live(key) {
            return Err(EngineError::UnknownNode(key));
        }
        self.send(Control::RemoveNode { key })?;
        self.slots[key.index as usize].live = false;
        self.free.push(key.index);
        Ok(())
    }

    /// Hand non-parameter data to a live node in place (roadmap v2; e.g. new sampler slice
    /// markers, so an edit doesn't re-create the node and cut sounding notes). Delivered at
    /// the next block via [`Node::set_data`]; whatever the node returns (its previous data,
    /// or `data` itself if it doesn't take it) is dropped on the GC thread.
    pub fn set_node_data(
        &mut self,
        key: NodeKey,
        data: crate::node::NodeData,
    ) -> Result<(), EngineError> {
        if !self.key_live(key) {
            return Err(EngineError::UnknownNode(key));
        }
        self.send(Control::NodeData { key, data })
    }

    /// Register (or replace) the audio source for `media` (used by audio clips/samplers).
    pub fn add_source(
        &mut self,
        media: MediaId,
        source: Arc<dyn AudioSource>,
    ) -> Result<(), EngineError> {
        self.send(Control::AddSource { media, source })
    }

    pub fn remove_source(&mut self, media: MediaId) -> Result<(), EngineError> {
        self.send(Control::RemoveSource { media })
    }

    /// Compile `desc` (here, non-RT) and send the snapshot for an atomic swap at the next
    /// block boundary. Returns compile errors without touching the running snapshot.
    ///
    /// Node latencies are read from what the audio thread last observed (PDC); when a
    /// node's latency changes the controller re-publishes. Clips whose media has no
    /// registered source play silence (no error), so media can load asynchronously.
    pub fn publish(&mut self, desc: RenderGraphDesc) -> Result<(), EngineError> {
        self.warp.sync(&desc);
        let snapshot = {
            let slots = &self.slots;
            let lat = &self.node_latency;
            compile_with(desc, &self.config, &|key: NodeKey| {
                let s = slots.get(key.index as usize)?;
                (s.live && s.generation == key.generation).then(|| NodeInfo {
                    latency: lat[key.index as usize].load(Ordering::Relaxed),
                    channels: s.channels,
                })
            })?
        };
        let latency = snapshot.latency();
        self.send(Control::Swap(Box::new(snapshot)))?;
        self.latency = latency;
        Ok(())
    }

    /// Start or stop the browser preview voice ([`crate::preview`]). Non-blocking.
    pub fn preview(&mut self, control: crate::preview::PreviewControl) -> Result<(), EngineError> {
        self.send(Control::Preview(control))
    }

    /// Install (`Some`) or remove (`None`) the "listen on <peer>" stream tap
    /// ([`crate::stream_tap`]; base-53). Non-blocking; the previous writer is retired to the
    /// GC.
    pub fn set_stream_tap(
        &mut self,
        writer: Option<crate::stream_tap::StreamTapWriter>,
    ) -> Result<(), EngineError> {
        self.send(Control::StreamTap(writer.map(Box::new)))
    }

    /// Total output latency (samples, PDC included) of the last successfully published
    /// graph: timeline position `p` reaches the hardware `latency()` samples later.
    pub fn latency(&self) -> u32 {
        self.latency
    }

    /// Live parameter change (fader, knob). Lock-free; applied next block.
    pub fn set_param(&mut self, change: ParamChange) -> Result<(), EngineError> {
        self.params.push(change).map_err(|_| EngineError::QueueFull)
    }

    pub fn transport(&mut self, control: TransportControl) -> Result<(), EngineError> {
        self.send(Control::Transport(control))
    }

    /// Drain engine outputs (meters, playhead, diagnostics) into
    /// `out` (cleared first). Call at UI rate (~30-60 Hz).
    pub fn poll(&mut self, out: &mut EngineOutputs) {
        out.clear();
        while let Ok(msg) = self.out.pop() {
            match msg {
                Output::Meter(m) => merge_meter(&mut out.meters, m),
                Output::Overflow => out.event_overflow = true,
                Output::Underruns(n) => out.underruns += n,
                Output::PreviewEnded(id) => out.preview_ended = Some(id),
            }
        }
        out.playhead = Some(self.playhead());
    }

    /// Drain the analysis frames pushed since the last call ([`crate::analysis`], v0.2),
    /// oldest first. Non-blocking.
    pub fn poll_analysis(&mut self, mut f: impl FnMut(&crate::analysis::AnalysisFrame)) {
        while let Ok(frame) = self.analysis.pop() {
            f(&frame);
        }
    }

    /// Start/stop collecting `node`'s analysis frames ([`crate::analysis`], v0.2; driven by
    /// the controller's watches). Non-blocking.
    pub fn watch_analysis(&mut self, node: NodeKey, on: bool) -> Result<(), EngineError> {
        self.send(Control::AnalysisWatch { key: node, on })
    }

    /// Latest playhead published by the audio thread.
    pub fn playhead(&self) -> PlayheadState {
        self.playhead.read()
    }

    /// Latency (samples) last reported by a node, as used for PDC at the next publish.
    pub fn node_latency(&self, key: NodeKey) -> Option<u32> {
        self.key_live(key)
            .then(|| self.node_latency[key.index as usize].load(Ordering::Relaxed))
    }
}

fn merge_meter(meters: &mut Vec<TrackMeter>, m: TrackMeter) {
    let track: TrackId = m.track;
    match meters.iter_mut().find(|e| e.track == track) {
        Some(e) => {
            for ch in 0..2 {
                e.peak[ch] = e.peak[ch].max(m.peak[ch]);
                e.rms[ch] = e.rms[ch].max(m.rms[ch]);
            }
            e.clipped |= m.clipped;
        }
        None => meters.push(m),
    }
}

/// Receives objects retired by the audio thread (old snapshots, removed nodes, replaced
/// sources) and drops them off the audio thread. `Send`: hosts usually move it to a
/// low-priority GC thread and call [`GarbageCollector::collect`] every ~50 ms.
pub struct GarbageCollector {
    rx: Consumer<Garbage>,
}

impl GarbageCollector {
    /// Drop everything retired so far; returns the number of objects dropped.
    pub fn collect(&mut self) -> usize {
        let mut n = 0;
        while let Ok(g) = self.rx.pop() {
            drop(g);
            n += 1;
        }
        n
    }
}

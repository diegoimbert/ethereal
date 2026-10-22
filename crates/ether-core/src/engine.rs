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
//! 3. Per sub-block, process the tracks as a job graph (one job per track, each started
//!    as soon as its own sources finished, critical path first; in parallel through the
//!    host's [`crate::parallel::ParallelExecutor`], see there):
//!    input bus gathered from the finished sources (+ monitored hardware input) → clips
//!    (audio rendered, MIDI notes scheduled sample-accurately) → automation → device chain →
//!    pre-fader sends → fader/pan → sidechain tap → mute/solo gate → post-fader sends →
//!    meter → PDC-aligned output in the track's own buffers. Then master goes to the
//!    hardware. With anticipative processing ([`crate::anticipation`]) the tracks not
//!    listened to live render the next block in the same pass, and the real-time side
//!    renders the sub-blocks they planned one block earlier.
//! 4. Publish playhead; every ~33 ms push meter readings.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering, fence};

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
use crate::mixer::{BusInput, TrackRt, apply_fader, apply_gate, scale};
use crate::node::{Node, NodeKey, ProcessContext, ProcessStatus};
use crate::parallel::{Fork, SerialFork};
use crate::param::{ParamChange, ParamTarget};
use crate::sched::{self, NoteSink, Timing};
use crate::sidechain::{ScIo, ScPoint};
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
    /// With the snapshot's hardware I/O state (`crate::hw_io`, `external-instrument`).
    Swap(Box<RenderSnapshot>, Option<Box<crate::hw_io::HwIoRt>>),
    AddSource {
        media: MediaId,
        source: Arc<dyn AudioSource>,
    },
    RemoveSource {
        media: MediaId,
    },
    Transport(TransportControl),
    Preview(crate::preview::PreviewControl),
    /// A short MIDI message for one track's instrument (piano-roll note preview,
    /// [`crate::recording`]).
    PreviewNote {
        track: TrackId,
        data: [u8; 3],
    },
    StreamTap(Option<Box<crate::stream_tap::StreamTapWriter>>),
    /// The current snapshot's job graphs, re-ranked with measured job times (snapshot `seq`
    /// only): the full one and the anticipation one (`crate::anticipation::AnticRt::graph`).
    Jobs {
        seq: u64,
        jobs: Box<crate::parallel::JobGraph>,
        ahead: Option<Box<crate::parallel::JobGraph>>,
    },
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
    HwIo(#[allow(dead_code)] Box<crate::hw_io::HwIoRt>),
    Jobs(#[allow(dead_code)] Box<crate::parallel::JobGraph>),
}

enum Output {
    Meter(TrackMeter),
    /// A track's smoothed job time (ns per 1024 frames, `TrackRt::job_ns`).
    JobCost(TrackId, u32),
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
    /// Silence-skipping state of the node in this slot.
    pub(crate) rt: NodeRt,
}

/// Per-node silence-skipping state, kept with the node across snapshots.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NodeRt {
    /// The node's last `process` returned [`crate::ProcessStatus::Silent`]: while its
    /// input stays silent and no event arrives, the engine skips it (silent output).
    pub(crate) asleep: bool,
    /// Its analysis frames are watched (`crate::analysis`): always process it, so the UI
    /// sees the spectrum/levels of the silence too.
    pub(crate) watched: bool,
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
    /// A locate (playing or not): automation is re-sent at the next sub-block.
    relocated: bool,
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
    let suspend = Arc::new(AtomicBool::new(false));
    let anticipate = AnticipationSwitch(Arc::new(AtomicBool::new(true)));
    let (recording_rt, recording_io) = crate::recording::channel(&config);
    let (warp_handle, warp_rt) = crate::warp::channel(&config);
    let snapshot =
        compile_with(RenderGraphDesc::default(), &config, &|_| None).expect("empty graph compiles");
    let tempo_bpm = snapshot.tempo.bpm_at(0.0);
    let (analysis_rt, analysis_rx) = crate::analysis::AnalysisRt::new(config.sample_rate);
    let (hw_io_rt, hw_io_handle) = crate::hw_io::channel(config.sample_rate);
    playhead.write(&PlayheadState {
        bpm: tempo_bpm,
        ..Default::default()
    });

    let engine = Engine {
        nodes: (0..config.max_nodes)
            .map(|_| NodeSlot {
                generation: 0,
                node: None,
                rt: NodeRt::default(),
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
        cost_interval: (config.sample_rate as usize / 2).max(1),
        cost_elapsed: 0,
        overflow: false,
        underruns: 0,
        leaked: 0,
        recording: recording_rt,
        metronome: crate::metronome::Metronome::new(config.sample_rate as f32),
        preview: crate::preview::PreviewVoice::new(config.max_block_size, config.sample_rate),
        stream_tap: Default::default(),
        capture: Vec::new(),
        executor: Box::new(crate::parallel::SequentialExecutor),
        warp: warp_rt,
        analysis: analysis_rt,
        hw_io: hw_io_rt,
        beat_table: vec![0.0; config.max_block_size + 1],
        beat_table_ahead: vec![0.0; config.max_block_size + 1],
        silence_skipping: true,
        anticipate: anticipate.clone(),
        queue: crate::anticipation::PlanQueue::new(),
        suspend: suspend.clone(),
        suspended: false,
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
        hw_io: hw_io_handle,
        suspend,
        anticipate,
        job_costs: JobCosts::default(),
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
    /// Job times are reported every `cost_interval` frames (`Output::JobCost`).
    cost_interval: usize,
    cost_elapsed: usize,
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
    /// "Listen on <peer>" tap ([`crate::stream_tap`]): after master + metronome,
    /// before the preview voice.
    stream_tap: crate::stream_tap::StreamTap,
    /// Parallel track processing ([`crate::parallel`], roadmap v2; `multicore`).
    /// Sequential until a host injects one with [`Engine::set_executor`].
    executor: Box<dyn crate::parallel::ParallelExecutor>,
    pub(crate) warp: crate::warp::WarpRt,
    /// Device → UI analysis frames ([`crate::analysis`], v0.2).
    analysis: crate::analysis::AnalysisRt,
    /// External devices' hardware sends/returns and MIDI out ([`crate::hw_io`], v0.3).
    hw_io: crate::hw_io::HwIo,
    /// Exact beats of the current sub-block's samples on ramped tempo maps
    /// (`sched::Exact::beats`, `max_block_size + 1` entries).
    beat_table: Vec<f64>,
    /// Skip asleep nodes on silent input (on by default).
    silence_skipping: bool,
    /// Anticipative processing switched on ([`AnticipationSwitch`]; needs
    /// `EngineConfig::anticipative` and workers, `crate::anticipation`): read once per block.
    anticipate: AnticipationSwitch,
    /// The sub-blocks the anticipated tracks rendered ahead of the output.
    queue: crate::anticipation::PlanQueue,
    /// Exact beats of the anticipated side's sub-block (like `beat_table`).
    beat_table_ahead: Vec<f64>,
    /// Set by [`EngineHandle::set_suspended`] (a project is loading): read once per block.
    suspend: Arc<AtomicBool>,
    /// The last block was suspended (the next unsuspended one resumes cleanly).
    suspended: bool,
    /// Offline export stems: the tapped tracks copied out after every block.
    capture: Vec<CaptureSlot>,
}

/// One track's post-fader (pre-gate) signal of the last [`Engine::process`] block, for
/// offline export stems.
pub(crate) struct CaptureSlot {
    pub id: TrackId,
    /// Planar signal of the block (`frames` long).
    pub buf: [Vec<f32>; 2],
    /// Latency of the tap (samples): the signal is that late relative to the timeline.
    /// `None` until the track is in the rendered graph.
    pub latency: Option<u32>,
}

impl Engine {
    /// Non-RT (offline renderer). Copy these tracks' tap (see
    /// [`EngineHandle::publish_capturing`]) into [`CaptureSlot`]s after every sub-block.
    pub(crate) fn set_capture(&mut self, tracks: &[TrackId]) {
        let n = self.config.max_block_size;
        self.capture = tracks
            .iter()
            .map(|&id| CaptureSlot {
                id,
                buf: [vec![0.0; n], vec![0.0; n]],
                latency: None,
            })
            .collect();
    }

    pub(crate) fn captured(&self) -> &[CaptureSlot] {
        &self.capture
    }

    /// **RT.** Render one block. `inputs`/`outputs` are planar, each `frames` long
    /// (`frames <= max_block_size`; channel counts as configured, extra channels ignored).
    ///
    /// Per block: drain control ring (node inserts/removals, snapshot swap, sources,
    /// transport), drain param ring, render (splitting at loop/tempo boundaries), write
    /// meters/playhead to the output ring, push retired objects to the GC ring. Never
    /// allocates, locks or blocks; bounded by graph size.
    pub fn process(&mut self, inputs: &[&[f32]], outputs: &mut [&mut [f32]], frames: usize) {
        self.process_with_hint(inputs, outputs, frames, frames);
    }

    /// **RT.** [`Engine::process`], where `next` is the length of the block that will follow
    /// (hosts that split a device buffer into several blocks know it): with anticipative
    /// processing (`crate::anticipation`) the anticipated tracks render that much ahead.
    /// `process` assumes the next block has the same length.
    pub fn process_with_hint(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        frames: usize,
        next: usize,
    ) {
        // Zero everything the host asked for; only `max_block_size` frames are rendered
        // (hosts with larger buffers call `process` repeatedly), the rest stays silent.
        for out in outputs.iter_mut() {
            let n = frames.min(out.len());
            out[..n].fill(0.0);
        }
        let frames = frames.min(self.config.max_block_size);
        // Read before draining: everything sent before the flag changed is applied below.
        let suspended = self.suspend.load(Ordering::Acquire);
        self.drain_control();
        self.warp.drain();
        self.drain_params();
        self.update_latencies();

        if suspended {
            self.resync();
            self.process_suspended(frames);
            return;
        }
        if self.suspended {
            self.resume();
        }

        let max = self.config.max_block_size;
        self.render_block(inputs, outputs, frames, next.clamp(1, max));

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
        self.cost_elapsed += frames;
        if self.cost_elapsed >= self.cost_interval {
            self.cost_elapsed = 0;
            self.push_job_costs();
        }
        self.report();
    }

    /// **RT.** A suspended block ([`EngineHandle::set_suspended`], while a project loads):
    /// the graph is not processed and the output stays silent. The control and param
    /// rings were drained (graph swaps, nodes and media still arrive), and so is what
    /// would otherwise pile up for the resume: live MIDI and preview notes are dropped.
    /// The sample clock keeps counting, so hosts' wall-clock mappings stay valid; the
    /// transport does not move.
    fn process_suspended(&mut self, frames: usize) {
        self.suspended = true;
        self.transport.sample_time += frames as u64;
        self.recording.discard_input(self.transport.sample_time);
        self.publish_playhead();
        self.report();
    }

    /// **RT.** The first block after a suspension: start from a clean state instead of
    /// whatever the partial graphs left. Every node is reset and gets all-notes-off,
    /// automation is re-sent and notes are chased at the position, the exact-tempo anchor
    /// is dropped, and mixer smoothers jump to their targets (no fade from stale values).
    fn resume(&mut self) {
        self.suspended = false;
        let t = &mut self.transport;
        t.reset_nodes = true;
        t.all_notes_off = true;
        t.release_notes = true;
        t.relocated = true;
        t.chase_notes = true;
        t.jump = true;
        t.anchor = None;
        for track in self.snapshot.rt.tracks.iter_mut() {
            track.settle();
        }
        self.meter_elapsed = 0;
    }

    /// The last block was suspended ([`EngineHandle::set_suspended`]).
    pub fn is_suspended(&self) -> bool {
        self.suspended
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// Engine sample time of the next block (hosts map it to wall time, e.g. to schedule
    /// hardware MIDI out, `crate::hw_io`).
    pub fn sample_time(&self) -> u64 {
        self.heard().3
    }

    /// The output's transport (playing, recording, position, sample time): the frontier
    /// transport, or the oldest sub-block rendered ahead (`crate::anticipation`).
    fn heard(&self) -> (bool, bool, f64, u64) {
        match self.queue.front() {
            Some(p) => (p.playing, p.recording, p.b0, p.sample_time),
            None => {
                let t = &self.transport;
                (t.playing, t.recording, t.position, t.sample_time)
            }
        }
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

    /// Non-RT (like [`Engine::set_executor`]): skip nodes that reported
    /// [`ProcessStatus::Silent`] while their input stays silent and no event reaches them
    /// (on by default). Off: every node is processed every block (tests
    /// compare both).
    pub fn set_silence_skipping(&mut self, on: bool) {
        self.silence_skipping = on;
    }

    /// Non-RT (allocates; tests and diagnostics): the tracks in job rank order, most
    /// urgent first (`crate::parallel`).
    #[doc(hidden)]
    pub fn job_order(&self) -> Vec<TrackId> {
        let rt = &self.snapshot.rt;
        rt.jobs.order().map(|i| rt.tracks[i].id).collect()
    }

    /// Frames the anticipated tracks rendered ahead of the output (`crate::anticipation`;
    /// tests and diagnostics).
    #[doc(hidden)]
    pub fn frames_ahead(&self) -> usize {
        self.queue.frames
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
                    slot.rt = NodeRt::default();
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
                Control::Swap(mut new, hw) => {
                    if let Some(old) = self.hw_io.swap(hw) {
                        self.retire(Garbage::HwIo(old));
                    }
                    {
                        let old = &mut self.snapshot.rt;
                        let fresh = &mut new.rt;
                        for (i, t) in fresh.tracks.iter_mut().enumerate() {
                            if let Some(oi) = old.lookup_track(t.id) {
                                t.inherit(&mut old.tracks[oi]);
                                // What it rendered ahead (`crate::anticipation`).
                                if let (Some(new), Some(was)) =
                                    (fresh.antic.as_mut(), old.antic.as_ref())
                                {
                                    new.ahead[i] = was.ahead[oi];
                                    new.front[i].inherit(&was.front[oi]);
                                }
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
                Control::PreviewNote { track, data } => self.recording.queue_preview(track, data),
                Control::Jobs {
                    seq,
                    mut jobs,
                    ahead,
                } => {
                    // Swapped through the box: the old graph leaves in it (no allocation).
                    if seq == self.snapshot.seq && jobs.len() == self.snapshot.rt.jobs.len() {
                        std::mem::swap(&mut self.snapshot.rt.jobs, &mut *jobs);
                    }
                    self.retire(Garbage::Jobs(jobs));
                    if let Some(mut ahead) = ahead {
                        if seq == self.snapshot.seq
                            && let Some(an) = self.snapshot.rt.antic.as_mut()
                            && ahead.len() == an.graph.len()
                        {
                            std::mem::swap(&mut an.graph, &mut *ahead);
                        }
                        self.retire(Garbage::Jobs(ahead));
                    }
                }

                Control::AnalysisWatch { key, on } => {
                    self.analysis.watch(key, on);
                    if let Some(slot) = self.nodes.get_mut(key.index as usize)
                        && slot.generation == key.generation
                    {
                        slot.rt.watched = on;
                    }
                }
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
                } else {
                    // Stop while stopped: a panic. Every node restarts (tails cut), on top
                    // of the notes released below.
                    t.reset_nodes = true;
                }
                // Stop while stopped still releases every node's notes (a panic for
                // anything left hanging).
                t.all_notes_off = true;
                t.playing = false;
            }
            TransportControl::Locate { position } => {
                t.position = position.0;
                t.release_notes = true;
                t.all_notes_off = true;
                // A jump while playing restarts the devices; stopped (e.g. Space: stop, then
                // back to where playback started) their tails ring out: a reset there cuts
                // reverbs and snaps a ducking compressor back up (a click).
                t.reset_nodes |= t.playing;
                t.relocated = true;
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

    /// Render up to `max` frames starting at output offset `off`; returns frames rendered.
    fn render_sub(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        off: usize,
        max: usize,
    ) -> usize {
        let plan = self.plan(max);
        self.run_pass(Some((&plan, off)), None, Pass::Aligned, inputs, outputs);
        plan.n
    }

    /// RT. The next sub-block of the frontier transport ([`plan_sub`]).
    fn plan(&mut self, max: usize) -> SubPlan {
        plan_sub(
            &mut self.transport,
            &self.snapshot.desc,
            &self.snapshot.tempo,
            self.config.sample_rate as f64,
            max,
        )
    }

    /// RT. Render one host block of `frames`, `hint` = the next one's expected length
    /// (anticipative processing, `crate::anticipation`; without it, sub-block by sub-block).
    fn render_block(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        frames: usize,
        hint: usize,
    ) {
        use crate::anticipation::split;
        let want = self.anticipate.is_on()
            && self.executor.workers() > 0
            && self.snapshot.rt.antic.as_ref().is_some_and(|a| a.any);
        if self.queue.frames == 0 && !want || self.snapshot.rt.antic.is_none() {
            if self.queue.frames > 0 {
                self.resync();
            }
            let mut done = 0;
            while done < frames {
                done += self.render_sub(inputs, outputs, done, frames - done);
            }
            return;
        }
        // --- what each track does in this block ---
        let mut q = self.queue.frames;
        let both = {
            let an = self.snapshot.rt.antic.as_mut().expect("checked above");
            let decide = |an: &mut crate::anticipation::AnticRt, free: bool| {
                for i in 0..an.eff.len() {
                    an.eff[i] = if free {
                        want && an.target[i]
                    } else {
                        an.ahead[i]
                    };
                }
            };
            // Changes wait while more than this block is queued, unless the graph forces
            // them (an ahead track now reads one that is not).
            decide(an, q <= frames);
            if q > frames && !an.closed(&an.eff) {
                self.resync();
                q = 0;
                let an = self.snapshot.rt.antic.as_mut().expect("checked above");
                decide(an, true);
            }
            let an = self.snapshot.rt.antic.as_ref().expect("checked above");
            (0..an.eff.len()).any(|i| an.eff[i] && !an.ahead[i])
        };
        let tempo_sr = self.config.sample_rate as f64;
        let mut off = 0;
        if want && q == frames && !both {
            // Steady state: the queued block (real-time side) and the next one (ahead) at
            // the same time.
            let mut left = hint;
            while off < frames {
                let Some(now) = self.queue.pop() else { break };
                let ahead = (left > 0 && !self.queue.is_full()).then(|| self.plan(left));
                self.run_pass(
                    Some((&now, off)),
                    ahead.as_ref(),
                    Pass::Overlap,
                    inputs,
                    outputs,
                );
                self.consume_front(now.n);
                if let Some(a) = ahead {
                    left -= a.n;
                    self.queue.push(a);
                }
                off += now.n;
            }
            while left > 0 && !self.queue.is_full() {
                let a = self.plan(left);
                self.run_pass(None, Some(&a), Pass::Ahead, inputs, outputs);
                left -= a.n;
                self.queue.push(a);
            }
        } else {
            // 1. The queued frames (real-time side; the ahead tracks rendered them already).
            let consume = q.min(frames);
            while off < consume {
                let Some(&head) = self.queue.front() else {
                    break;
                };
                let now = if head.n > consume - off {
                    let (first, rest) = split(&head, consume - off, &self.snapshot.tempo, tempo_sr);
                    self.queue.replace_front(rest);
                    first
                } else {
                    self.queue.pop();
                    head
                };
                self.run_pass(Some((&now, off)), None, Pass::Front, inputs, outputs);
                self.consume_front(now.n);
                off += now.n;
            }
            // 2. Past them, everything in real time.
            while off < frames {
                let plan = self.plan(frames - off);
                self.run_pass(Some((&plan, off)), None, Pass::Aligned, inputs, outputs);
                off += plan.n;
            }
            // 3. The tracks ending ahead render the next block.
            let any = self
                .snapshot
                .rt
                .antic
                .as_ref()
                .is_some_and(|a| a.eff.iter().any(|&e| e));
            if any {
                let mut left = hint.saturating_sub(self.queue.frames);
                while left > 0 && !self.queue.is_full() {
                    let a = self.plan(left);
                    self.run_pass(None, Some(&a), Pass::Ahead, inputs, outputs);
                    left -= a.n;
                    self.queue.push(a);
                }
            }
        }
        // --- held output for the next block ---
        let queued = self.queue.frames;
        if let Some(an) = self.snapshot.rt.antic.as_mut() {
            for i in 0..an.eff.len() {
                let (front, back) = (&mut an.front[i], &mut an.back[i]);
                if back.len() > 0 {
                    front.take_from(back);
                }
                an.ahead[i] = an.eff[i] && queued > 0;
                if !an.ahead[i] {
                    front.clear();
                }
            }
        }
    }

    /// RT. The real-time side rendered `n` more frames: drop them from the held output.
    fn consume_front(&mut self, n: usize) {
        if let Some(an) = self.snapshot.rt.antic.as_mut() {
            for (f, &ahead) in an.front.iter_mut().zip(&an.ahead) {
                if ahead {
                    f.consume(n);
                }
            }
        }
    }

    /// RT. Drop what was rendered ahead: every track renders in real time from the next
    /// pass. The sample clock goes back to the output's; the timeline does not (ahead tracks
    /// skip the dropped frames).
    fn resync(&mut self) {
        if let Some(p) = self.queue.front() {
            self.transport.sample_time = p.sample_time;
            self.transport.anchor = None;
            self.transport.jump = true;
        }
        self.queue.clear();
        if let Some(an) = self.snapshot.rt.antic.as_mut() {
            an.ahead.fill(false);
            for h in an.front.iter_mut().chain(an.back.iter_mut()) {
                h.clear();
            }
        }
    }

    /// RT. One pass over the job graph: the real-time side renders `now` (at output offset
    /// `.1`), the anticipated side `ahead`, each track as `pass` says
    /// (`crate::anticipation`).
    fn run_pass(
        &mut self,
        now: Option<(&SubPlan, usize)>,
        ahead: Option<&SubPlan>,
        pass: Pass,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
    ) {
        use crate::anticipation::Lane;
        let sr = self.config.sample_rate as f64;
        let Engine {
            nodes,
            sources,
            snapshot,
            overflow,
            underruns,
            recording,
            warp,
            metronome,
            preview,
            stream_tap,
            executor,
            beat_table,
            beat_table_ahead,
            hw_io,
            silence_skipping,
            capture,
            ..
        } = self;
        let RenderSnapshot {
            desc, tempo, rt, ..
        } = &mut **snapshot;
        let SnapshotRt {
            order,
            jobs,
            tracks,
            latency: graph_latency,
            vcas,
            antic,
            ..
        } = rt;
        if let Some(an) = antic.as_mut() {
            for i in 0..an.lane.len() {
                let (was, eff) = (an.ahead[i], an.eff[i]);
                an.lane[i] = match pass {
                    Pass::Aligned => Lane::Now,
                    Pass::Front if was => Lane::Idle,
                    Pass::Front => Lane::Now,
                    Pass::Overlap if was && eff => Lane::Ahead,
                    Pass::Overlap if was => Lane::Idle,
                    Pass::Overlap => Lane::Now,
                    Pass::Ahead if eff => Lane::Ahead,
                    Pass::Ahead => Lane::Idle,
                };
            }
        }
        let lanes = antic.as_ref().map(|a| &a.lane[..]);
        let on = |l: Lane| move |i: usize| lanes.is_none_or(|v| v[i] == l);
        let flags = |plan: &SubPlan| JobFlags {
            playing: plan.playing,
            release_notes: plan.release_notes,
            all_notes_off: plan.all_notes_off,
            reset_nodes: plan.reset_nodes,
            relocated: plan.relocated,
            chase_notes: plan.chase_notes,
            skip_silence: *silence_skipping,
        };
        let now_t = now.map(|(plan, _)| plan.timing(tempo, sr, beat_table));
        let ahead_t = ahead.map(|plan| plan.timing(tempo, sr, beat_table_ahead));
        if let (Some((plan, off)), Some((info, timing))) = (now, &now_t) {
            recording.process(info, plan.n, &desc.tracks, tracks);
            // VCA gains and automation for this sub-block (`crate::vca`), before the jobs.
            vcas.update(tracks, timing, plan.playing, &on(Lane::Now));
            // Hardware returns of external devices (`crate::hw_io`), before the jobs.
            hw_io.gather_returns(inputs, off, plan.n, plan.sample_time);
        }
        if let (Some(plan), Some((_, timing))) = (ahead, &ahead_t) {
            vcas.update(tracks, timing, plan.playing, &on(Lane::Ahead));
        }

        // --- tracks, by dependency (`crate::parallel`) ---
        // Stopped, the jobs still run devices and plugins at full cost (tails, auditioned
        // instruments, plugins that never sleep): dispatch them like while playing.
        // Only when the previous sub-block did no real work anywhere (every node
        // asleep, nothing monitored: `TrackRt::awake`) keep the workers parked and run the
        // jobs here, in rank order. Same result either way, bit for bit.
        let playing = now.is_some_and(|(p, _)| p.playing) || ahead.is_some_and(|p| p.playing);
        let parallel = playing || tracks.iter().any(|t| t.awake || t.monitor);
        let ctx = JobCtx {
            tracks: tracks.as_mut_ptr(),
            n_tracks: tracks.len(),
            nodes: nodes.as_mut_ptr(),
            n_nodes: nodes.len(),
            warp: warp.table(),
            descs: &desc.tracks,
            sources,
            sr,
            now: now
                .zip(now_t.as_ref())
                .map(|((plan, off), (info, timing))| LaneCtx {
                    inputs,
                    off,
                    n: plan.n,
                    flags: flags(plan),
                    info,
                    timing,
                }),
            ahead: ahead
                .zip(ahead_t.as_ref())
                .map(|(plan, (info, timing))| LaneCtx {
                    inputs: &[],
                    off: 0,
                    n: plan.n,
                    flags: flags(plan),
                    info,
                    timing,
                }),
            antic: antic.as_mut().map(|a| AnticCtx {
                lane: a.lane.as_ptr(),
                ahead: a.ahead.as_ptr(),
                front: a.front.as_ptr(),
                back: a.back.as_mut_ptr(),
                front_reads: matches!(pass, Pass::Front | Pass::Overlap),
            }),
            hw: hw_io.table(),
        };
        // Overlapping passes run without the edges from ahead to real-time tracks (those
        // read the held output).
        let jobs = match antic.as_ref() {
            Some(a) if matches!(pass, Pass::Overlap | Pass::Ahead) => &a.graph,
            _ => &*jobs,
        };
        // SAFETY (`crate::parallel`, "Unsafe sharing"): job `ti` touches track `ti` (one job
        // per track), its nodes (each node in one chain), its own held output (`back`), and
        // its sources' tracks, which the graph finished before it, or their `front`, which
        // no job writes; the executor runs each job once, after its sources, pinned jobs on
        // this thread, and returns after all of them finished.
        let job = |ti: usize, fork: &dyn Fork| unsafe { ctx.run(ti, fork) };
        if jobs.len() == 1 && parallel && executor.workers() > 0 {
            // One track (or a pure chain): no other job to overlap, but its rack chains may
            // still fork.
            executor.execute_graph_forking(jobs, &job);
        } else if jobs.len() == 1 || !parallel {
            for ti in jobs.order() {
                job(ti, &SerialFork);
            }
        } else {
            executor.execute_graph_forking(jobs, &job);
        }
        let Some(((plan, off), (info, _))) = now.zip(now_t.as_ref()) else {
            for track in tracks.iter_mut() {
                *overflow |= std::mem::take(&mut track.overflow);
                *underruns += std::mem::take(&mut track.underruns);
            }
            return;
        };
        let info = *info;
        let n = plan.n;
        // --- offline export stems: post-fader, pre-gate taps of the requested tracks ---
        for slot in capture.iter_mut() {
            let found = tracks.iter().find(|t| t.id == slot.id);
            slot.latency = found.map(|t| t.out_latency);
            if let Some(tap) = found.and_then(|t| t.tap.as_ref()) {
                for (d, s) in slot.buf.iter_mut().zip(tap) {
                    d[off..off + n].copy_from_slice(&s[..n]);
                }
            } else {
                for d in &mut slot.buf {
                    d[off..off + n].fill(0.0);
                }
            }
        }
        // --- recording capture: hardware input + aligned input taps ---
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
        // --- external devices' hardware sends and MIDI (`crate::hw_io`), after master ---
        hw_io.write_sends(outputs, off, n, plan.sample_time);
        // --- metronome (after master reached the hardware outputs; not metered) ---
        if plan.reset_nodes {
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
        // --- stream tap: master + metronome/count-in, never the preview ---
        stream_tap.write(&info, *graph_latency, plan.jump, off, n, outputs);
        // --- browser preview (after master; not metered, transport-independent) ---
        preview.render(off, n, outputs);
    }

    fn publish_playhead(&mut self) {
        let (playing, recording, pos, sample_time) = self.heard();
        let tempo = &self.snapshot.tempo;
        self.playhead.write(&PlayheadState {
            playing,
            recording,
            position: ether_protocol::model::Beats(pos),
            seconds: tempo.beats_to_seconds(pos),
            bpm: tempo.bpm_at(pos),
            sample_time,
        });
    }

    /// RT. Report the measured job times (none on wasm, where nothing measures them).
    fn push_job_costs(&mut self) {
        for t in self.snapshot.rt.tracks.iter() {
            if t.job_ns <= 0.0 {
                continue;
            }
            if self.out.slots() == 0 {
                break;
            }
            let _ = self
                .out
                .push(Output::JobCost(t.id, t.job_ns.min(u32::MAX as f32) as u32));
        }
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

/// How the tracks render in one pass of [`Engine::run_pass`] (`crate::anticipation`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pass {
    /// Every track renders the output's sub-block.
    Aligned,
    /// The real-time side renders the output's sub-block, reading the held output of the
    /// tracks rendered ahead; those do nothing.
    Front,
    /// [`Pass::Front`], and the tracks staying ahead render the next block meanwhile.
    Overlap,
    /// Only the tracks ending ahead render, the next block.
    Ahead,
}

/// One sub-block of the timeline as the transport decided it ([`plan_sub`]): what rendering
/// it needs from the transport, so the decision and the render are separate steps.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SubPlan {
    /// Frames.
    pub n: usize,
    pub playing: bool,
    pub recording: bool,
    /// Engine sample clock at the start.
    pub sample_time: u64,
    /// Start/end in beats (`b1` exclusive) and seconds at `b0`.
    pub b0: f64,
    pub b1: f64,
    pub s0: f64,
    /// Ends at the loop end: the timeline jumps back after it (`Timing::wraps`).
    pub wraps: bool,
    /// Exact tempo integration (`sched::Exact`): anchor beat and seconds, samples since it.
    pub exact: Option<(f64, f64, u64)>,
    pub loop_on: bool,
    pub loop_start: f64,
    pub loop_end: f64,
    /// The transport's one-shot flags for this sub-block (see `TransportRt`).
    pub release_notes: bool,
    pub all_notes_off: bool,
    pub reset_nodes: bool,
    pub relocated: bool,
    pub chase_notes: bool,
    pub jump: bool,
}

impl SubPlan {
    /// RT. The sub-block's transport info and timing; `beats` (at least `n + 1` entries)
    /// holds the exact per-sample beats on ramped tempo maps.
    fn timing<'a>(
        &self,
        tempo: &'a crate::tempo::TempoMapRt,
        sr: f64,
        beats: &'a mut [f64],
    ) -> (TransportInfo, Timing<'a>) {
        let (n, b0, b1) = (self.n, self.b0, self.b1);
        let exact = self.exact.map(|(beat, seconds, since)| {
            let beats = &mut beats[..=n];
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
        let (signature, bar_start, bar_number) = tempo.bar_at(b0);
        let (bpm, end_bpm) = tempo.bpm_span(b0, b1);
        let info = TransportInfo {
            playing: self.playing,
            recording: self.recording,
            sample_time: self.sample_time,
            position: b0,
            seconds: self.s0,
            bpm,
            beats_per_sample: (b1 - b0) / n as f64,
            tempo_inc: (end_bpm - bpm) / n as f64,
            time_signature: signature,
            bar_start,
            bar_number: bar_number.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            loop_active: self.loop_on,
            loop_start: self.loop_start,
            loop_end: self.loop_end,
        };
        let timing = Timing {
            tempo,
            b0,
            b1,
            s0: self.s0,
            sample_rate: sr,
            frames: n,
            sample_time: self.sample_time,
            wraps: self.wraps,
            exact,
        };
        (info, timing)
    }
}

/// The loop region in effect (`SetLoop` override, else the graph's), `on` only when it is
/// longer than a hair.
fn loop_region(t: &TransportRt, desc: &RenderGraphDesc) -> (bool, f64, f64) {
    let (on, s, e) = t
        .loop_override
        .unwrap_or((desc.loop_enabled, desc.loop_start, desc.loop_end));
    (on && e - s > 1e-6, s, e)
}

/// RT. Decide the next sub-block (at most `max` frames) and advance the transport past it.
///
/// A sub-block ends early on the first sample at or after a loop end or a tempo/signature
/// boundary, and the timeline restarts exactly there (v0.1: every boundary lands on a whole
/// sample; exact integration re-anchors on it).
fn plan_sub(
    t: &mut TransportRt,
    desc: &RenderGraphDesc,
    tempo: &crate::tempo::TempoMapRt,
    sr: f64,
    max: usize,
) -> SubPlan {
    let (loop_on, loop_start, loop_end) = loop_region(t, desc);
    let playing = t.playing;
    let b0 = t.position;
    // Ramped tempo maps are integrated exactly from the sample clock (`sched::Exact`,
    // CONTRACTS.md §12.7); others keep the v0.1 math, bit for bit.
    let anchor = if playing && tempo.has_ramps() {
        let sample_time = t.sample_time;
        Some(
            *t.anchor
                .get_or_insert_with(|| (b0, tempo.beats_to_seconds(b0), sample_time)),
        )
    } else {
        t.anchor = None;
        None
    };
    let since = anchor.map_or(0, |(_, _, at)| t.sample_time - at);
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
    let split = b1.is_some();
    let b1 = match (playing, b1, anchor) {
        (false, _, _) => b0,
        (true, Some(b), _) => b,
        (true, None, Some((beat, seconds, _))) => {
            sched::exact_beat(tempo, sr, (beat, seconds), since, n as f64)
        }
        (true, None, None) => tempo.seconds_to_beats(s0 + n as f64 / sr),
    };
    let plan = SubPlan {
        n,
        playing,
        recording: t.recording,
        sample_time: t.sample_time,
        b0,
        b1,
        s0,
        wraps: hit_loop,
        exact: anchor.map(|(beat, seconds, _)| (beat, seconds, since)),
        loop_on,
        loop_start,
        loop_end,
        release_notes: t.release_notes,
        all_notes_off: t.all_notes_off,
        reset_nodes: t.reset_nodes,
        relocated: t.relocated,
        chase_notes: t.chase_notes,
        jump: t.jump,
    };

    // --- advance ---
    t.jump = false;
    t.release_notes = false;
    t.all_notes_off = false;
    t.reset_nodes = false;
    t.relocated = false;
    t.sample_time += n as u64;
    if playing {
        t.chase_notes = false;
        if hit_loop {
            t.position = loop_start;
            t.anchor = None;
            t.release_notes = true;
            t.chase_notes = true;
            t.jump = true;
        } else {
            t.position = b1;
            if split {
                t.anchor = None;
            }
        }
    }
    plan
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
    ///   nothing writes while the job graph runs, and yields `None`;
    /// - the audio thread doesn't touch the table while the job graph runs.
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
        // thread between blocks (`drain_control`), never while the job graph runs.
        if unsafe { std::ptr::addr_of!((*slot).generation).read() } != key.generation {
            return None;
        }
        // SAFETY: the key is live, so by the `shared` contract (or `new`'s exclusivity) this
        // slot's node belongs to this table's track only; the borrow of `self` keeps it to
        // one node at a time per table.
        unsafe { (*std::ptr::addr_of_mut!((*slot).node)).as_mut() }
    }

    /// RT. [`NodeTable::get`] with the node's silence-skipping state (same slot, same
    /// exclusivity: `rt` belongs to the node's owner like `node`).
    pub(crate) fn get_rt(&mut self, key: NodeKey) -> Option<(&mut Box<dyn Node>, &mut NodeRt)> {
        let i = key.index as usize;
        if i >= self.len {
            return None;
        }
        let slot = self.ptr.wrapping_add(i);
        // SAFETY: as in `get`.
        if unsafe { std::ptr::addr_of!((*slot).generation).read() } != key.generation {
            return None;
        }
        // SAFETY: as in `get`; `node` and `rt` are disjoint fields of this live slot.
        let rt = unsafe { &mut *std::ptr::addr_of_mut!((*slot).rt) };
        let node = unsafe { (*std::ptr::addr_of_mut!((*slot).node)).as_mut() }?;
        Some((node, rt))
    }
}

/// A [`NodeTable`]'s slots, handed to sub-jobs of its job (`crate::rack_chains`), which make
/// their own tables over them ([`SharedNodes::table`]).
#[derive(Clone, Copy)]
pub(crate) struct SharedNodes<'a> {
    ptr: *mut NodeSlot,
    len: usize,
    _slots: std::marker::PhantomData<&'a mut [NodeSlot]>,
}

// SAFETY: only dereferenced through tables made by `table`, under its contract (each sub-job
// asks for its own nodes only); nodes are `Send`.
unsafe impl Sync for SharedNodes<'_> {}

impl<'a> NodeTable<'a> {
    /// RT. The slots, for sub-jobs of this job while it waits for them (`Fork::join`).
    pub(crate) fn share(&mut self) -> SharedNodes<'_> {
        SharedNodes {
            ptr: self.ptr,
            len: self.len,
            _slots: std::marker::PhantomData,
        }
    }
}

impl<'a> SharedNodes<'a> {
    /// A table for one sub-job.
    ///
    /// # Safety
    /// As [`NodeTable::shared`]: the sub-job only asks for keys no other sub-job (and not the
    /// waiting job) uses meanwhile.
    pub(crate) unsafe fn table(&self) -> NodeTable<'a> {
        // SAFETY: forwarded to the caller.
        unsafe { NodeTable::shared(self.ptr, self.len) }
    }
}

/// Transport flags of the current sub-block (read-only for the jobs).
#[derive(Clone, Copy)]
struct JobFlags {
    playing: bool,
    release_notes: bool,
    all_notes_off: bool,
    reset_nodes: bool,
    relocated: bool,
    chase_notes: bool,
    /// Skip asleep nodes on silent input (`Engine::set_silence_skipping`).
    skip_silence: bool,
}

/// One side's sub-block as the jobs of a pass see it (`crate::anticipation::Lane`).
struct LaneCtx<'a> {
    /// Hardware input (the real-time side only; empty ahead).
    inputs: &'a [&'a [f32]],
    /// Offset of the sub-block in the host's block (hardware input).
    off: usize,
    n: usize,
    flags: JobFlags,
    info: &'a TransportInfo,
    timing: &'a Timing<'a>,
}

/// Anticipation state a pass's jobs read (`crate::anticipation`), as raw pointers into
/// `SnapshotRt::antic`, one entry per track.
#[derive(Clone, Copy)]
struct AnticCtx {
    lane: *const crate::anticipation::Lane,
    /// Rendered ahead (its readers take `front`).
    ahead: *const bool,
    front: *const crate::anticipation::Held,
    /// Written by the track's own job only.
    back: *mut crate::anticipation::Held,
    /// Real-time jobs read ahead sources from `front` in this pass.
    front_reads: bool,
}

/// What a track job needs: raw pointers to the per-track/per-node state it gets exclusive
/// access to (`crate::parallel`, "Unsafe sharing"), plus read-only sub-block context.
struct JobCtx<'a> {
    tracks: *mut TrackRt,
    n_tracks: usize,
    nodes: *mut NodeSlot,
    n_nodes: usize,
    /// Stretcher voices: each job takes its own track's (`WarpTable::track_voices`).
    warp: crate::warp::WarpTable,
    descs: &'a [crate::graph::TrackDesc],
    sources: &'a [(MediaId, Arc<dyn AudioSource>)],
    sr: f64,
    /// The output's sub-block, and the anticipated one (`crate::anticipation`).
    now: Option<LaneCtx<'a>>,
    ahead: Option<LaneCtx<'a>>,
    /// `None`: every job renders `now`.
    antic: Option<AnticCtx>,
    /// External devices' entries (`crate::hw_io`): each reached by its node's job only.
    hw: crate::hw_io::HwIoTable,
}

// SAFETY: the shared fields are `Sync` (checked below); the raw pointers are only
// dereferenced by `JobCtx::run` under the graph contract: one job per track per sub-block,
// each node reached by one job, finished source tracks only read, each track's voices by
// its job.
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
    /// Called at most once per track per sub-block, never concurrently for one track, and
    /// only after every track in `tracks[ti].inputs` / its sidechain, modulator and tap
    /// sources finished (the job graph's edges); running tracks are touched by their own
    /// jobs only.
    unsafe fn run(&self, ti: usize, fork: &dyn Fork) {
        debug_assert!(ti < self.n_tracks);
        use crate::anticipation::Lane;
        // SAFETY: `lane` has one entry per track and is only written between passes.
        let lane = self.antic.map_or(Lane::Now, |a| unsafe { *a.lane.add(ti) });
        let lc = match lane {
            Lane::Idle => None,
            Lane::Now => self.now.as_ref(),
            Lane::Ahead => self.ahead.as_ref(),
        };
        let Some(lc) = lc else {
            return;
        };
        // The held output of `c` when this job reads it there (a real-time job, `c` rendered
        // ahead: its own buffers hold the next buffer already, or will).
        let held = |c: usize| -> Option<&crate::anticipation::Held> {
            let a = self.antic?;
            // SAFETY: one entry per track; `ahead` and `front` only change between passes.
            (lane == Lane::Now && a.front_reads && unsafe { *a.ahead.add(c) })
                .then(|| unsafe { &*a.front.add(c) })
        };
        // The job's wall time, for the scheduler's priorities (`TrackRt::job_ns`). Native
        // only: a vDSO clock read at both ends (wasm runs everything in order anyway).
        #[cfg(not(target_arch = "wasm32"))]
        let started = std::time::Instant::now();
        // SAFETY: exclusive by the contract above.
        let track = unsafe { &mut *self.tracks.add(ti) };
        // SAFETY: this job only asks for the keys of its own chain and pad chains.
        let mut nodes = unsafe { NodeTable::shared(self.nodes, self.n_nodes) };
        let n = lc.n;

        // --- input bus: gather the finished sources in the fixed order ---
        {
            let TrackRt {
                a,
                inputs,
                sidechain,
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
                let buf = if let Some(h) = held(c) {
                    match sel {
                        None => h.output(),
                        Some(k) => match h.send(k) {
                            Some(b) => b,
                            None => continue,
                        },
                    }
                } else {
                    // SAFETY: `c != ti` is a source of this job: finished; nothing writes
                    // it now.
                    let src = unsafe { &*self.tracks.add(c) };
                    match sel {
                        None => &src.a,
                        Some(k) => &src.sends[k].buf,
                    }
                };
                for ch in 0..2 {
                    for (d, s) in a[ch][..n].iter_mut().zip(&buf[ch][..n]) {
                        *d += s;
                    }
                }
            }
            // Other tracks' sidechain taps (finished too) into this track's own entries.
            if !sidechain.is_empty() {
                let tracks = self.tracks;
                let n_tracks = self.n_tracks;
                sidechain.gather(n, |s, point| {
                    if s == ti || s >= n_tracks {
                        return None;
                    }
                    if let Some(h) = held(s) {
                        return h.sc_point(point);
                    }
                    // SAFETY: as above (sidechain sources are ordered before their consumer,
                    // `s != ti`).
                    let src = unsafe { &*tracks.add(s) };
                    src.sc_points.get(point)
                });
            }
        }
        // Envelope-follower sidechains (`crate::modulation`, v0.2): sources finished earlier.
        for i in 0..track.modulation.sidechain_sources().len() {
            let s = track.modulation.sidechain_sources()[i];
            if s == ti {
                continue;
            }
            let tap = match held(s) {
                Some(h) => h.tap(),
                // SAFETY: as above (sidechain sources are ordered before their consumer).
                None => unsafe { (*self.tracks.add(s)).tap.as_ref() },
            };
            if let Some(tap) = tap {
                track.modulation.write_sidechain(s, tap, n);
            }
        }
        // Track input from another track (`crate::bus_tap`, v0.2): its source finished before
        // this job (a graph edge).
        if let Some(s) = track.input_tap.source
            && s != ti
        {
            match held(s) {
                Some(h) => track.input_tap.gather(h.taps(), n),
                None => {
                    // SAFETY: as above (tap sources are ordered before their consumer).
                    let src = unsafe { &*self.tracks.add(s) };
                    track.input_tap.gather(&src.taps, n);
                }
            }
        }

        // SAFETY: this job's own track (ids are unique in a snapshot); the voice table only
        // changes between blocks.
        let voices = unsafe { self.warp.track_voices(track.id) };
        self.render_track(lc, ti, track, &mut nodes, voices, fork);
        // Ahead: what other tracks read from this one, held for the real-time side.
        if lane == Lane::Ahead
            && let Some(a) = self.antic
        {
            // SAFETY: `back[ti]` is touched by this job only.
            unsafe { &mut *a.back.add(ti) }.append(track, n);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            // Per 1024 frames, so sub-block splits don't look cheaper.
            let ns = started.elapsed().as_nanos() as f32 * 1024.0 / n.max(1) as f32;
            let e = track.job_ns;
            track.job_ns = if ns > e {
                e + (ns - e) * 0.5
            } else {
                e + (ns - e) * 0.01
            };
        }
    }

    /// RT. The body of a track job, after its input bus was gathered into `track.a`.
    fn render_track(
        &self,
        lc: &LaneCtx<'_>,
        ti: usize,
        track: &mut TrackRt,
        nodes: &mut NodeTable<'_>,
        voices: &mut [crate::warp::Voice],
        fork: &dyn Fork,
    ) {
        let n = lc.n;
        let off = lc.off;
        let sr = self.sr;
        let flags = lc.flags;
        let playing = flags.playing;
        let timing = lc.timing;
        let tempo = timing.tempo;
        let (b0, b1) = (timing.b0, timing.b1);
        let info = lc.info;
        let tdesc = &self.descs[ti];
        let TrackRt {
            chain,
            a,
            b,
            out_events,
            notes,
            racks,
            sidechain: rt_sidechain,
            sc_points,
            next_note_id,
            src_scratch,
            overflow,
            underruns,
            chain_racks,
            modulation,
            input_tap,
            taps,
            vca,
            expression,
            ..
        } = track;

        // --- monitored hardware input ---
        if track.monitor
            && let Some((l, r)) = crate::recording::input_channels(track.audio_input)
        {
            for (ch, hw) in [(0usize, l), (1, r)] {
                if let Some(input) = lc.inputs.get(hw as usize) {
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
                // Queued for a sub-block of another length (an anticipated track's events
                // come from the real-time side's): keep them inside this one.
                let mut e = *e;
                e.offset = e.offset.min(n.saturating_sub(1) as u32);
                c.events.push(e);
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
        // Clips ending at or before this beat do nothing in this sub-block (notes, chased
        // notes, expression, audio): skip them with a binary search.
        let live_from = timing.beat_at(-1.0).min(timing.event_range().0);
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
                let first = automation_rt::first_live_clip(&track.clip_ends, live_from).min(end);
                let clips = &tdesc.clips[first..end];
                // MIDI expression (`crate::expression`, v0.3): lanes before the note-ons,
                // note expressions of the sounding notes before their note-offs.
                if flags.chase_notes {
                    expression.reset();
                }
                expression.render_lanes(&tdesc.expression, clips, timing, sink.events);
                for clip in clips {
                    if matches!(clip.content, ClipContentDesc::Midi { .. }) {
                        if flags.chase_notes {
                            sched::chase_notes(clip, timing, &mut sink);
                        }
                        sched::schedule_notes(clip, timing, &mut sink);
                    }
                }
                expression.render_notes(&tdesc.expression, clips, timing, sink.notes, sink.events);
                sink.end_notes(timing);
            } else if flags.release_notes {
                expression.stop(0, sink.events);
            }
        } else if flags.release_notes {
            notes.clear();
        }
        // Frozen track (`crate::freeze`, v0.2): the render replaces clips and chain (the
        // controller compiles neither), or the frozen part of the chain (partial freeze: the
        // live tail compiled into `chain` processes the render below).
        if playing
            && let Some(frozen) = &tdesc.frozen
            && !crate::freeze::render_frozen(frozen, self.sources, info, sr, a, n)
        {
            *underruns += 1;
        }
        if playing && tdesc.kind == TrackKind::Audio {
            let end = tdesc.clips.partition_point(|c| c.start < b1);
            let first = automation_rt::first_live_clip(&track.clip_ends, live_from).min(end);
            for clip in &tdesc.clips[first..end] {
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
                // Complex-warped clips through the track's stretcher voices (`crate::warp`);
                // the others exactly `sched::render_audio`.
                let ok = crate::warp::render(
                    voices,
                    track.warp_scratch.as_mut(),
                    clip,
                    source,
                    ref_bpm,
                    timing,
                    out,
                    src_scratch,
                );
                if !ok {
                    *underruns += 1;
                }
            }
        }

        taps.write(InputTap::PreFx, a, n);
        sc_points.write(ScPoint::Track(InputTap::PreFx), a, n);

        // --- automation (`crate::automation_rt`: sample-accurate, CONTRACTS.md §12.7) ---
        // Enabled lanes/envelopes always drive their target (precedence per event, see
        // docs/CONTRACTS.md §4 "Automation precedence"): node params are re-sent after a
        // live change or a locate; mixer targets are ramped in the fader stage below.
        if track.auto_dirty || flags.reset_nodes || flags.relocated {
            track.auto_last.fill(f64::NAN);
            track.env_last.fill(f64::NAN);
            track.auto_dirty = false;
        }
        let mixer_auto = apply_automation(
            tdesc,
            &track.clip_ends,
            &mut track.auto_window,
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
        // Whether this job did real work (`TrackRt::awake`: decides parallel dispatch while
        // stopped).
        let mut awake =
            track.monitor || !racks.is_empty() || !chain_racks.is_empty() || tdesc.frozen.is_some();
        for k in 0..chain.len() {
            let (head, tail) = chain.split_at_mut(k + 1);
            let entry = &mut head[k];
            // Off = MIDI thru (v0.2 `midi-fx`, `ChainRt::route_midi`): its notes reach the
            // next device (its own params don't). One still fading out runs without events.
            *overflow |= entry.route_midi(tail.first_mut().map(|e| &mut e.events));
            *overflow |= entry.events.overflowed();
            let key = entry.key;
            // The bypass path keeps the input (`ChainRt::blend`), before anything writes it.
            entry.push_dry(a, n);
            // Sidechain PDC: delay the main signal before this entry if planned.
            rt_sidechain.align_main(key, a, n);
            let runs = entry.runs();
            // Drum rack: pad chains feed the rack node's input (`crate::drum_rack`).
            if runs && !racks.is_empty() && racks.is_rack(key) {
                *overflow |= racks.run_pads(
                    key,
                    nodes,
                    entry.events.as_slice(),
                    info,
                    sr as f32,
                    a,
                    n,
                    flags.reset_nodes,
                    &mut ScIo {
                        taps: rt_sidechain,
                        points: sc_points,
                    },
                    chain_racks,
                );
            }
            // Rack chains (`crate::rack_chains`, v0.2): chains feed the rack node's input.
            if runs && !chain_racks.is_empty() && chain_racks.is_rack(key) {
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
                    &mut ScIo {
                        taps: rt_sidechain,
                        points: sc_points,
                    },
                    // Drum racks on instrument rack chains (`crate::drum_rack`).
                    Some(racks),
                    fork,
                );
            }
            modulation.pre_node(k, key, entry.events.as_slice(), a, n);
            let Some((node, node_rt)) = nodes.get_rt(key) else {
                sc_points.write(ScPoint::After(key), a, n);
                continue;
            };
            if flags.reset_nodes {
                node.reset();
                node_rt.asleep = false;
            }
            if !runs {
                entry.bypass(a, n);
                sc_points.write(ScPoint::After(key), a, n);
                continue;
            }
            out_events.clear();
            let (n_in, n_out) = (
                (entry.channels.0 as usize).min(2),
                (entry.channels.1 as usize).min(2),
            );
            // Silence skipping: a node that reported `Silent` stays asleep
            // while it gets no events and its input stays digital silence; its output is
            // then silence too (`ProcessStatus::Silent`'s contract), so don't call it.
            let skip = flags.skip_silence
                && node_rt.asleep
                && !node_rt.watched
                && entry.events.is_empty()
                && !entry.sidechain
                && a[..n_in].iter().all(|ch| ch[..n].iter().all(|&s| s == 0.0))
                // SAFETY: `key` is in this track's chain (see `HwIoTable::get`).
                && unsafe { self.hw.get(key) }.is_none()
                && !node.wake_requested();
            if skip {
                for ch in &mut b[..n_out] {
                    ch[..n].fill(0.0);
                }
            } else {
                awake = true;
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
                // External device (`crate::hw_io`): MIDI to the hardware, the return as its
                // sidechain, its hardware send as outputs 2..4.
                // SAFETY: `key` is in this track's chain (see `HwIoTable::get`).
                if let Some(hw) = unsafe { self.hw.get(key) } {
                    hw.push_events(entry.events.as_slice(), info.sample_time);
                    let (ret, [sl, sr_]) = hw.split(n);
                    let [o0, o1] = outs;
                    let mut outs4: [&mut [f32]; 4] = [o0, o1, sl, sr_];
                    let n_out4 = if n_out < 2 || outs4[2].is_empty() {
                        n_out
                    } else {
                        4
                    };
                    let mut buffers = AudioBuffers {
                        inputs: &ins[..n_in],
                        outputs: &mut outs4[..n_out4],
                    };
                    let status = node.process_sidechain(&mut ctx, &mut buffers, &ret);
                    node_rt.asleep = status == ProcessStatus::Silent;
                } else {
                    let mut buffers = AudioBuffers {
                        inputs: &ins[..n_in],
                        outputs: &mut outs[..n_out],
                    };
                    // Sidechain (`crate::sidechain`): a tapped source's aligned signal.
                    let status = match entry
                        .sidechain
                        .then(|| rt_sidechain.read(key, sc_points, n))
                        .flatten()
                    {
                        Some(sc) => node.process_sidechain(&mut ctx, &mut buffers, &sc),
                        None => node.process(&mut ctx, &mut buffers),
                    };
                    node_rt.asleep = status == ProcessStatus::Silent;
                }
            }
            *overflow |= out_events.overflowed();
            if !chain_racks.is_empty() {
                chain_racks.finish(key, out_events);
            }
            // A device fading out sends no MIDI (it is thru already).
            entry.gate_output(out_events);
            match n_out {
                0 => {}
                1 => {
                    let [bl, br] = &mut *b;
                    br[..n].copy_from_slice(&bl[..n]);
                    entry.blend(b, n);
                    std::mem::swap(a, b);
                }
                _ => {
                    entry.blend(b, n);
                    std::mem::swap(a, b);
                }
            }
            sc_points.write(ScPoint::After(key), a, n);
            // Note/MIDI output feeds the next device.
            if let Some(next) = tail.first_mut() {
                for e in out_events.as_slice() {
                    next.events.push(*e);
                }
            }
        }
        track.awake = awake;
        taps.write(InputTap::PostFx, a, n);
        sc_points.write(ScPoint::Track(InputTap::PostFx), a, n);

        // --- sends (pre), fader, sends (post) ---
        // Each send's signal goes to its own buffer, gathered by the destination's job.
        let gate = track.gate.current();
        for pass_pre in [true, false] {
            if !pass_pre {
                // Automated volume/pan: ramped per sample (`crate::automation_rt`).
                if !automation_rt::fader(
                    tdesc,
                    &track.clip_ends,
                    timing,
                    mixer_auto,
                    a,
                    &mut track.volume,
                    &mut track.pan,
                ) {
                    apply_fader(a, &mut track.volume, &mut track.pan, n);
                }
                // VCA gain (`crate::vca`, v0.2).
                vca.apply(a, n);
                // Sidechain tap (`crate::sidechain`): post-fader but *before* the mute/solo
                // gate, so muting or soloing-out a track silences what it contributes to
                // the mix while it keeps keying the devices it sidechains (a muted ghost
                // kick still ducks the bass; soloing the bass still hears it duck). Its
                // latency is `out_lat` (taken before the PDC output delay).
                if let Some(tap) = &mut track.tap {
                    tap[0][..n].copy_from_slice(&a[0][..n]);
                    tap[1][..n].copy_from_slice(&a[1][..n]);
                }
                sc_points.write(ScPoint::Track(InputTap::PostFader), a, n);
                // Mute/solo gate, then the post-fader tap.
                apply_gate(a, &mut track.gate, n);
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
                if !automation_rt::send_level(tdesc, &track.clip_ends, timing, mixer_auto, send) {
                    let [sl, sr_] = &mut send.buf;
                    scale(&mut sl[..n], &mut send.level, false);
                    scale(&mut sr_[..n], &mut send.level, true);
                }
            }
        }

        // --- meter + output ---
        track.meter.add(&a[0][..n], &a[1][..n]);
        // The output stays in `a`: gathered by the destination bus's job, or summed into
        // the hardware output by the audio thread (master).
        let [al, ar] = &mut *a;
        track.output_delay.process(&mut al[..n], &mut ar[..n]);
    }
}

/// Anticipative processing on/off for an engine ([`EngineHandle::anticipation_switch`]):
/// any thread, read by the audio thread once per block.
#[derive(Clone, Debug)]
pub struct AnticipationSwitch(Arc<AtomicBool>);

impl AnticipationSwitch {
    pub fn set(&self, on: bool) {
        self.0.store(on, Ordering::Relaxed);
    }

    pub fn is_on(&self) -> bool {
        self.0.load(Ordering::Relaxed)
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
    /// Hardware I/O rings ([`crate::hw_io`], v0.3).
    pub(crate) hw_io: crate::hw_io::HwIoHandle,
    /// Shared with [`Engine::suspend`].
    suspend: Arc<AtomicBool>,
    /// Shared with the engine.
    anticipate: AnticipationSwitch,
    /// Measured job times per track (`Output::JobCost`), and the job graph of the last
    /// publish for re-ranking with them ([`EngineHandle::poll`]).
    job_costs: JobCosts,
}

/// The handle's side of measured job priorities (`crate::parallel`): the published graph's
/// shape, the costs it was last ranked with, and the latest measurements.
#[derive(Default)]
struct JobCosts {
    measured: std::collections::HashMap<TrackId, u32>,
    /// Publish counter (`RenderSnapshot::seq`).
    seq: u64,
    /// Of snapshot `seq`: the job graph as ranked now, its tracks (job id = index) and their
    /// static costs.
    shape: Option<(crate::parallel::JobGraph, Vec<TrackId>, Vec<u64>)>,
    /// Its anticipation graph's shape (`crate::anticipation::AnticRt::graph`).
    ahead_shape: Option<crate::parallel::JobGraph>,
    /// Engine sample time of the last re-rank.
    last: u64,
}

/// Re-rank at most this often (seconds of engine time).
const RERANK_SECONDS: u64 = 2;
/// A track's measured cost has to move by this factor before the graph is re-ranked.
const RERANK_RATIO: f64 = 1.3;

impl JobCosts {
    /// Job costs (µs per 1024 frames, at least 1) of `tracks` from the measurements; tracks
    /// not measured yet get their static cost scaled like the measured ones. `None` while
    /// nothing is measured (hosts without a clock: wasm).
    fn costs(&self, tracks: &[TrackId], statics: &[u64]) -> Option<Vec<u64>> {
        let mut per_unit: Vec<f64> = tracks
            .iter()
            .zip(statics)
            .filter_map(|(t, &c)| self.measured.get(t).map(|&ns| ns as f64 / c.max(1) as f64))
            .collect();
        if per_unit.is_empty() {
            return None;
        }
        per_unit.sort_by(f64::total_cmp);
        let unit = per_unit[per_unit.len() / 2];
        Some(
            tracks
                .iter()
                .zip(statics)
                .map(|(t, &c)| {
                    let ns = self
                        .measured
                        .get(t)
                        .map_or(c as f64 * unit, |&ns| ns as f64);
                    ((ns / 1000.0).round() as u64).max(1)
                })
                .collect(),
        )
    }
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
    /// Suspend (`true`) or resume the audio thread's processing, effective at the next
    /// block: suspended, [`Engine::process`] outputs silence and runs no graph work, while
    /// everything sent here (nodes, graphs, media, transport) is still applied in order.
    /// Hosts suspend while a project loads, so the partial graphs of a progressive load
    /// never run and the callback can't fall behind. Lock-free (an atomic flag, never
    /// lost to a full queue); everything sent before the call is applied before the change.
    pub fn set_suspended(&self, suspended: bool) {
        self.suspend.store(suspended, Ordering::Release);
    }

    /// The last value given to [`Self::set_suspended`].
    pub fn suspended(&self) -> bool {
        self.suspend.load(Ordering::Relaxed)
    }

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
        self.publish_capturing(desc, &[])
    }

    /// [`EngineHandle::publish`] that also gives each of `capture` a post-fader (pre
    /// mute/solo gate) tap buffer, whether or not a sidechain reads it, for offline export
    /// stems ([`crate::offline::OfflineRenderer::capture_tracks`]). Unknown ids are ignored.
    pub(crate) fn publish_capturing(
        &mut self,
        desc: RenderGraphDesc,
        capture: &[TrackId],
    ) -> Result<(), EngineError> {
        self.warp.sync(&desc);
        let mut snapshot = {
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
        for t in &mut snapshot.rt.tracks {
            if t.tap.is_none() && capture.contains(&t.id) {
                t.tap = Some(crate::mixer::stereo(self.config.max_block_size));
            }
        }
        // Rank the jobs by measured times where known (`crate::parallel`).
        self.job_costs.seq += 1;
        snapshot.seq = self.job_costs.seq;
        {
            let jobs = &snapshot.rt.jobs;
            let tracks: Vec<TrackId> = snapshot.desc.tracks.iter().map(|t| t.id).collect();
            let statics: Vec<u64> = (0..tracks.len())
                .map(|id| jobs.cost_of(id).unwrap_or(1))
                .collect();
            if let Some(costs) = self.job_costs.costs(&tracks, &statics) {
                snapshot.rt.jobs = jobs.reweighted(|id| costs[id]);
                if let Some(an) = snapshot.rt.antic.as_mut() {
                    an.graph = an.graph.reweighted(|id| costs[id]);
                }
            }
            self.job_costs.ahead_shape = snapshot.rt.antic.as_ref().map(|an| {
                an.graph
                    .reweighted(|id| snapshot.rt.jobs.cost_of(id).unwrap_or(1))
            });
            let shape = snapshot
                .rt
                .jobs
                .reweighted(|id| snapshot.rt.jobs.cost_of(id).unwrap_or(1));
            self.job_costs.shape = Some((shape, tracks, statics));
        }
        let latency = snapshot.latency();
        let hw = crate::hw_io::HwIoRt::for_graph(&snapshot.desc, self.config.max_block_size);
        self.send(Control::Swap(Box::new(snapshot), hw))?;
        self.latency = latency;
        Ok(())
    }

    /// Start or stop the browser preview voice ([`crate::preview`]). Non-blocking.
    pub fn preview(&mut self, control: crate::preview::PreviewControl) -> Result<(), EngineError> {
        self.send(Control::Preview(control))
    }

    /// Play a short MIDI message (`data`, a channel voice message) on `track`'s
    /// instrument from the next block, whatever its arm / monitor state (piano-roll note
    /// preview; [`crate::recording`]). Non-blocking.
    pub fn preview_note(&mut self, track: TrackId, data: [u8; 3]) -> Result<(), EngineError> {
        self.send(Control::PreviewNote { track, data })
    }

    /// Install (`Some`) or remove (`None`) the "listen on <peer>" stream tap
    /// ([`crate::stream_tap`]). Non-blocking; the previous writer is retired to the
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

    /// Anticipative processing on or off (`crate::anticipation`, CONTRACTS.md §11.7), from
    /// the next block; only takes effect on an engine created with
    /// `EngineConfig::anticipative` (on by default there) and worker threads. Switching off
    /// lets what was rendered ahead play out first. Lock-free, never lost.
    pub fn set_anticipative(&self, on: bool) {
        self.anticipate.set(on);
    }

    /// The switch behind [`Self::set_anticipative`], for another thread (a host's settings).
    pub fn anticipation_switch(&self) -> AnticipationSwitch {
        self.anticipate.clone()
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
                Output::JobCost(track, ns) => {
                    self.job_costs.measured.insert(track, ns);
                }
                Output::Overflow => out.event_overflow = true,
                Output::Underruns(n) => out.underruns += n,
                Output::PreviewEnded(id) => out.preview_ended = Some(id),
            }
        }
        out.playhead = Some(self.playhead());
        self.rerank_jobs();
    }

    /// Non-RT. Re-rank the running graph's jobs with the measured job times when one of them
    /// moved by more than [`RERANK_RATIO`] since the last ranking (at most every
    /// [`RERANK_SECONDS`] of engine time): a new `JobGraph` is built here and swapped in by
    /// the audio thread, nothing else of the snapshot changes.
    fn rerank_jobs(&mut self) {
        let now = self.playhead().sample_time;
        let jc = &mut self.job_costs;
        if now.saturating_sub(jc.last) < RERANK_SECONDS * self.config.sample_rate as u64 {
            return;
        }
        let Some((shape, tracks, statics)) = &jc.shape else {
            return;
        };
        let Some(costs) = jc.costs(tracks, statics) else {
            return;
        };
        jc.last = now;
        let moved = costs.iter().enumerate().any(|(id, &c)| {
            let was = shape.cost_of(id).unwrap_or(1) as f64;
            let r = c as f64 / was.max(1.0);
            !(1.0 / RERANK_RATIO..=RERANK_RATIO).contains(&r)
        });
        if !moved {
            return;
        }
        let jobs = shape.reweighted(|id| costs[id]);
        let copy = jobs.reweighted(|id| costs[id]);
        let ahead = jc
            .ahead_shape
            .as_ref()
            .map(|g| g.reweighted(|id| costs[id]));
        let ahead_copy = ahead.as_ref().map(|g| g.reweighted(|id| costs[id]));
        let seq = jc.seq;
        if self
            .control
            .push(Control::Jobs {
                seq,
                jobs: Box::new(jobs),
                ahead: ahead.map(Box::new),
            })
            .is_ok()
            && let Some(s) = self.job_costs.shape.as_mut()
        {
            s.0 = copy;
            self.job_costs.ahead_shape = ahead_copy;
        }
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

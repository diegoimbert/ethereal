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
//! 3. Per sub-block, process tracks in topological order: input bus (+ monitored hardware
//!    input) → clips (audio rendered, MIDI notes scheduled sample-accurately) → automation
//!    → device chain → pre-fader sends → fader/pan/mute gate → post-fader sends → meter →
//!    PDC-aligned output into the destination bus (or the hardware for master).
//! 4. Publish playhead; every ~33 ms push meter readings.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};

use ether_protocol::meters::TrackMeter;
use ether_protocol::model::{MediaId, TrackId, TrackKind};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::automation::{evaluate, gain_from_plain, to_plain};
use crate::buffer::AudioBuffers;
use crate::config::{EngineConfig, PrepareConfig};
use crate::event::{EventKind, ProcessEvent};
use crate::graph::{
    AutomationDesc, ClipContentDesc, CompileError, NodeInfo, RenderGraphDesc, RenderSnapshot,
    ResolvedTarget, SnapshotRt, compile_with,
};
use crate::media::AudioSource;
use crate::meter::EngineOutputs;
use crate::mixer::{TrackRt, apply_fader, mix_into};
use crate::node::{Node, NodeKey, ProcessContext};
use crate::param::{ParamChange, ParamTarget};
use crate::sched::{self, NoteSink, Timing};
use crate::transport::{PlayheadState, TransportControl, TransportInfo};

/// Registered audio sources (media) the engine can hold.
pub const MAX_SOURCES: usize = 4096;
/// Node-param automation is re-evaluated every this many samples within a sub-block.
const AUTOMATION_STEP: usize = 64;
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
}

enum Garbage {
    Snapshot(#[allow(dead_code)] Box<RenderSnapshot>),
    Node(#[allow(dead_code)] Box<dyn Node>),
    Data(#[allow(dead_code)] crate::node::NodeData),
    Source(#[allow(dead_code)] Arc<dyn AudioSource>),
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
        src_scratch: vec![0.0; config.max_block_size * 8 + 64],
        next_note_id: 0,
        meter_interval: (config.sample_rate as usize / 30).max(1),
        meter_elapsed: 0,
        overflow: false,
        underruns: 0,
        leaked: 0,
        recording: recording_rt,
        metronome: crate::metronome::Metronome::new(config.sample_rate as f32),
        preview: crate::preview::PreviewVoice::new(config.max_block_size),
        executor: Box::new(crate::parallel::SequentialExecutor),
        warp: warp_rt,
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
    src_scratch: Vec<f32>,
    next_note_id: u32,
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
    /// Parallel track processing ([`crate::parallel`], roadmap v2; `multicore` node).
    /// Sequential until a host injects one with [`Engine::set_executor`].
    #[allow(dead_code)]
    executor: Box<dyn crate::parallel::ParallelExecutor>,
    pub(crate) warp: crate::warp::WarpRt,
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
                    let slot = &mut self.nodes[i];
                    slot.generation = key.generation;
                    if let Some(old) = slot.node.replace(node) {
                        self.retire(Garbage::Node(old));
                    }
                }
                Control::RemoveNode { key } => {
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
                    }
                    self.transport.loop_override = None;
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
            }
        }
    }

    fn apply_transport(&mut self, control: TransportControl) {
        let t = &mut self.transport;
        match control {
            TransportControl::Play => {
                if !t.playing {
                    t.chase_notes = true;
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
                    }
                }
                ParamTarget::SendLevel { send } => {
                    if let Some((t, s)) = rt.lookup_send(send) {
                        rt.tracks[t].sends[s].level.set_target(v.max(0.0) as f32);
                    }
                }
                ParamTarget::Node { node, param } => {
                    let event = ProcessEvent {
                        offset: 0,
                        kind: EventKind::Param { param, value: v },
                    };
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
            .chain(rt.pad_index.iter().map(|e| e.0));
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
            src_scratch,
            next_note_id,
            overflow,
            underruns,
            recording,
            warp,
            metronome,
            preview,
            ..
        } = self;
        let RenderSnapshot { desc, tempo, rt } = &mut **snapshot;
        let SnapshotRt {
            order,
            tracks,
            buses,
            sidechain: rt_sidechain,
            latency: graph_latency,
            ..
        } = rt;

        // --- sub-block extent ---
        let playing = transport.playing;
        let b0 = transport.position;
        let s0 = tempo.beats_to_seconds(b0);
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
        let b1 = match (playing, b1) {
            (false, _) => b0,
            (true, Some(b)) => b,
            (true, None) => tempo.seconds_to_beats(s0 + n as f64 / sr),
        };
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
        };
        recording.process(&info, inputs, off, n, &desc.tracks, tracks);

        for bus in buses.iter_mut() {
            bus[0][..n].fill(0.0);
            bus[1][..n].fill(0.0);
        }

        for &ti in order.iter() {
            let tdesc = &desc.tracks[ti];
            let track = &mut tracks[ti];
            let TrackRt {
                chain,
                a,
                b,
                scratch,
                out_events,
                notes,
                racks,
                ..
            } = track;

            // --- input ---
            a[0][..n].copy_from_slice(&buses[ti][0][..n]);
            a[1][..n].copy_from_slice(&buses[ti][1][..n]);
            if track.monitor
                && let Some((l, r)) = crate::recording::input_channels(track.audio_input)
            {
                for (ch, hw) in [(0usize, l), (1, r)] {
                    if let Some(input) = inputs.get(hw as usize) {
                        let src = &input[off.min(input.len())..(off + n).min(input.len())];
                        for (d, s) in a[ch].iter_mut().zip(src) {
                            *d += s;
                        }
                    }
                }
            }

            // --- events: pending live params ---
            racks.begin_block(transport.all_notes_off);
            for c in chain.iter_mut() {
                c.events.clear();
                for e in c.pending.as_slice() {
                    c.events.push(*e);
                }
                c.pending.clear();
                if transport.all_notes_off {
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
                if transport.release_notes {
                    sink.release_all(0);
                }
                if playing {
                    let end = tdesc.clips.partition_point(|c| c.start < b1);
                    for clip in &tdesc.clips[..end] {
                        if matches!(clip.content, ClipContentDesc::Midi { .. }) {
                            if transport.chase_notes {
                                sched::chase_notes(clip, &timing, &mut sink);
                            }
                            sched::schedule_notes(clip, &timing, &mut sink);
                        }
                    }
                    sink.end_notes(&timing);
                }
            } else if transport.release_notes {
                notes.clear();
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
                    let Ok(si) = sources.binary_search_by(|e| e.0.cmp(media)) else {
                        continue;
                    };
                    let ref_bpm = tempo.bpm_at(clip.start);
                    let [al, ar] = &mut *a;
                    if !warp.render(
                        clip,
                        &*sources[si].1,
                        ref_bpm,
                        &timing,
                        [&mut al[..n], &mut ar[..n]],
                        src_scratch,
                    ) {
                        *underruns += 1;
                    }
                }
            }

            // --- automation ---
            // Each target is resolved once per sub-block (see docs/CONTRACTS.md §4
            // "Automation precedence"): a clip envelope of an unmuted clip overlapping the
            // sub-block (or containing the position while stopped) takes the target; the
            // arrangement lane for that target is skipped and re-sends as soon as it is
            // uncovered. Enabled lanes/envelopes always drive their target: mixer targets
            // are re-applied every sub-block, node params are re-sent after a live change
            // or a locate.
            if track.auto_dirty || transport.reset_nodes {
                track.auto_last.fill(f64::NAN);
                track.env_last.fill(f64::NAN);
                track.auto_dirty = false;
            }
            // Stopped: the "sub-block" is the position itself.
            let env_end = if playing { b1 } else { b0 + 1e-9 };
            let clips_end = tdesc.clips.partition_point(|c| c.start < env_end);
            let active_clips = || {
                tdesc.clips[..clips_end]
                    .iter()
                    .enumerate()
                    .filter(move |(_, c)| !c.muted && c.start + c.length > b0)
            };
            let covered = |target: ResolvedTarget| {
                active_clips().any(|(_, c)| {
                    c.envelopes
                        .iter()
                        .any(|e| e.resolved == target && !e.points.is_empty())
                })
            };
            for (li, lane) in tdesc.automation.iter().enumerate() {
                let last = &mut track.auto_last[li];
                if covered(lane.resolved) {
                    *last = f64::NAN;
                    continue;
                }
                apply_automation(
                    lane,
                    |t| evaluate(&lane.points, t),
                    &timing,
                    playing,
                    last,
                    &mut track.volume,
                    &mut track.pan,
                    &mut track.sends,
                    chain,
                    racks,
                );
            }
            // Envelopes of clips that stopped covering their target send again next time.
            for (ci, clip) in tdesc.clips.iter().enumerate() {
                let active = ci < clips_end && !clip.muted && clip.start + clip.length > b0;
                if !active && !clip.envelopes.is_empty() {
                    let base = track.env_base[ci];
                    track.env_last[base..base + clip.envelopes.len()].fill(f64::NAN);
                }
            }
            for (ci, clip) in active_clips() {
                for (ei, env) in clip.envelopes.iter().enumerate() {
                    let last = &mut track.env_last[track.env_base[ci] + ei];
                    apply_automation(
                        env,
                        |t| sched::content_at(clip, t).and_then(|c| evaluate(&env.points, c)),
                        &timing,
                        playing,
                        last,
                        &mut track.volume,
                        &mut track.pan,
                        &mut track.sends,
                        chain,
                        racks,
                    );
                }
            }

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
                        &info,
                        sr as f32,
                        a,
                        n,
                        transport.reset_nodes,
                    );
                }
                let Some(slot) = nodes.get_mut(key.index as usize) else {
                    continue;
                };
                if slot.generation != key.generation {
                    continue;
                }
                let Some(node) = slot.node.as_mut() else {
                    continue;
                };
                if transport.reset_nodes {
                    node.reset();
                }
                if !entry.enabled {
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
                        transport: &info,
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

            // --- sends (pre), fader, sends (post) ---
            let gate = track.gate.current();
            for pass_pre in [true, false] {
                if !pass_pre {
                    apply_fader(a, &mut track.volume, &mut track.pan, &mut track.gate, n);
                }
                for send in track.sends.iter_mut().filter(|s| s.pre_fader == pass_pre) {
                    let [sl, sr_] = &mut *scratch;
                    sl[..n].copy_from_slice(&a[0][..n]);
                    sr_[..n].copy_from_slice(&a[1][..n]);
                    if pass_pre && gate != 1.0 {
                        for s in sl[..n].iter_mut().chain(sr_[..n].iter_mut()) {
                            *s *= gate;
                        }
                    }
                    send.delay.process(&mut sl[..n], &mut sr_[..n]);
                    let dst = &mut buses[send.target];
                    mix_into(&mut dst[0][..n], &sl[..n], &mut send.level, false);
                    mix_into(&mut dst[1][..n], &sr_[..n], &mut send.level, true);
                }
            }

            // --- meter + output ---
            track.meter.add(&a[0][..n], &a[1][..n]);
            // Sidechain tap: post-fader, before the PDC output delay (latency = out_lat).
            rt_sidechain.write(ti, a, n);
            {
                let [al, ar] = &mut *a;
                track.output_delay.process(&mut al[..n], &mut ar[..n]);
            }
            if let Some(o) = track.output {
                let dst = &mut buses[o];
                for ch in 0..2 {
                    for (d, s) in dst[ch][..n].iter_mut().zip(&a[ch][..n]) {
                        *d += s;
                    }
                }
            } else if track.to_hardware {
                for (ch, out) in outputs.iter_mut().take(2).enumerate() {
                    let end = (off + n).min(out.len());
                    if off >= end {
                        continue;
                    }
                    for (d, s) in out[off..end].iter_mut().zip(&a[ch][..n]) {
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
                transport.release_notes = true;
                transport.chase_notes = true;
            } else {
                transport.position = b1;
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

/// Apply one automation lane for the current sub-block. Mixer targets get their smoother
/// target set from the lane at the sub-block start (every sub-block, so a manual move never
/// sticks while a lane is enabled); node params get `Param` events every
/// [`AUTOMATION_STEP`] samples while playing when the value differs from the last one
/// sent (`last` is reset to NaN to force a re-send).
#[allow(clippy::too_many_arguments)]
fn apply_automation(
    lane: &AutomationDesc,
    value_at: impl Fn(f64) -> Option<f64>,
    timing: &Timing<'_>,
    playing: bool,
    last: &mut f64,
    volume: &mut crate::param::Smoother,
    pan: &mut crate::param::Smoother,
    sends: &mut [crate::mixer::SendRt],
    chain: &mut [crate::mixer::ChainRt],
    racks: &mut crate::drum_rack::RacksRt,
) {
    match lane.resolved {
        ResolvedTarget::TrackVolume | ResolvedTarget::TrackPan | ResolvedTarget::Send { .. } => {
            let Some(v) = value_at(timing.b0) else {
                return;
            };
            let plain = to_plain(&lane.mapping, v);
            let drive = |s: &mut crate::param::Smoother, target: f32| {
                if s.target() != target {
                    s.set_target(target);
                }
            };
            match lane.resolved {
                ResolvedTarget::TrackVolume => {
                    drive(volume, gain_from_plain(&lane.mapping, plain));
                }
                ResolvedTarget::TrackPan => drive(pan, plain.clamp(-1.0, 1.0) as f32),
                ResolvedTarget::Send { send } => {
                    if let Some(s) = sends.iter_mut().find(|s| s.id == send) {
                        drive(&mut s.level, gain_from_plain(&lane.mapping, plain));
                    }
                }
                ResolvedTarget::Node { .. } => {}
            }
        }
        ResolvedTarget::Node { node, param } => {
            // Track-chain node, or a device on a drum pad.
            let events = match chain.iter_mut().find(|c| c.key == node) {
                Some(entry) => &mut entry.events,
                None => match racks.events_mut(node) {
                    Some(events) => events,
                    None => return,
                },
            };
            let steps = if playing { timing.frames } else { 1 };
            let mut o = 0;
            while o < steps {
                if let Some(v) = value_at(timing.beat_at(o as f64))
                    && v != *last
                {
                    *last = v;
                    events.push(ProcessEvent {
                        offset: o as u32,
                        kind: EventKind::Param {
                            param,
                            value: to_plain(&lane.mapping, v),
                        },
                    });
                }
                o += AUTOMATION_STEP;
            }
        }
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

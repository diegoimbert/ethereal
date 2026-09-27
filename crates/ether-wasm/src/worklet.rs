//! Engine side (AudioWorklet): owns all three halves of `ether_core::create` and runs them
//! on the one audio thread the web gives us.
//!
//! Per render quantum ([`EngineHost::render`]):
//! 1. apply pending control frames through the `EngineHandle` (node creation, graph
//!    compile, media chunks, sources), bounded per quantum: at most
//!    [`CONTROL_BUDGET_BYTES`] bytes and [`MAX_FRAMES_PER_QUANTUM`] frames, and never more
//!    than one heavy frame (a `Publish`, which decodes a binary snapshot and compiles it, or a
//!    `MediaBegin`, which allocates the media buffers),
//! 2. `Engine::process` into the host's planar output buffers,
//! 3. every [`REPORT_INTERVAL_BLOCKS`] blocks, poll the handle and write an
//!    [`EngineReport`] to the report ring (dropped if the ring is full),
//! 4. run the `GarbageCollector`.
//!
//! The engine's stream tap (`ether_core::stream_tap`: master + metronome/count-in, minus the
//! preview voice; "listen on <peer>", docs/COLLAB.md §9.1) is installed once at construction
//! and drained after every render into [`EngineHost::tap_output`] (the worklet's second
//! output, fed to a `MediaStreamAudioDestinationNode` by the web sender), with the transport
//! state of the rendered sub-blocks in [`EngineHost::tap_clock`] (layout [`tap_clock`]).
//!
//! Only step 2 is the real-time render. Steps 1 and 4 allocate (snapshot decode and
//! compile, media buffers, dropping retired snapshots) but run on the same thread because
//! an AudioWorkletGlobalScope has no other thread; the budgets bound their cost per block.
//! This is the web-host trade-off accepted in ARCHITECTURE.md ("single-threaded").
//! Snapshots use the binary `ether_core::codec::BinaryCodec` (one allocation per `Vec`,
//! no parsing): for a 64-track / 500-clip project the whole publish quantum costs ~0.8 ms
//! in V8 against a 2.67 ms quantum at 48 kHz (the former JSON decode alone took ~4.5 ms);
//! see [`crate::perf`].
//!
//! The frame cap also keeps core's queues from overflowing: every quantum drains them
//! (control 1024, params 4096 entries), and at most [`MAX_FRAMES_PER_QUANTUM`] entries are
//! pushed per quantum, so a backlog (e.g. after a suspended AudioContext resumes) is
//! applied over several quanta instead of failing with `QueueFull`.

use std::collections::BTreeMap;
use std::sync::Arc;

use ether_core::graph::{AutomationDesc, ResolvedTarget};
use ether_core::protocol::model::MediaId;
use ether_core::stream_tap::{StreamBlock, StreamTapReader, stream_tap_ring};
use ether_core::{
    AudioSource, EngineConfig, EngineHandle, EngineOutputs, GarbageCollector, NodeKey, ParamTarget,
    RenderGraphDesc,
};
use ether_devices::SampleResolver;
use ether_media::InMemorySource;

use crate::proto::{EngineMsg, EngineReport, Frame, MediaAssembler, PREVIEW_MEDIA, REPORT_ERROR};
use crate::ring::{RingMemory, RingReader, RingWriter};

/// Web render quantum.
pub const RENDER_QUANTUM: usize = 128;
/// Control-ring bytes applied per block (bounds non-RT work on the audio thread; about
/// four media chunks).
pub const CONTROL_BUDGET_BYTES: usize = 256 * 1024;
/// Control frames applied per block, well below core's queue capacities.
pub const MAX_FRAMES_PER_QUANTUM: usize = 64;
/// Report every N blocks (~12 ms at 48 kHz / 128 frames).
pub const REPORT_INTERVAL_BLOCKS: u32 = 4;
/// Marks automation whose node is unknown (dropped before publishing).
const DEAD_KEY: NodeKey = NodeKey {
    index: u32::MAX,
    generation: u32::MAX,
};
/// Output channels rendered by the worklet.
pub const OUTPUT_CHANNELS: usize = 2;

/// Stream tap channels (stereo), exposed as `output_ptr(OUTPUT_CHANNELS + c)`.
pub const TAP_CHANNELS: usize = 2;
/// `output_ptr(TAP_CLOCK_SLOT)` points at the [`tap_clock`] header (`f64`s, not `f32`s).
pub const TAP_CLOCK_SLOT: usize = OUTPUT_CHANNELS + TAP_CHANNELS;
/// Stereo frames the tap ring holds (it is drained every block).
const TAP_RING_FRAMES: usize = RENDER_QUANTUM * 8;

/// Layout of the tap clock header (`f64` slots) written by each render: the transport
/// state of the last tapped sub-block, and of the last one that jumped (or followed a gap).
/// Offsets are frames into the rendered block; `latency` is the graph latency (samples).
pub mod tap_clock {
    /// Tapped sub-blocks in the last render (0: nothing tapped; ignore the rest).
    pub const BLOCKS: usize = 0;
    pub const OFFSET: usize = 1;
    pub const POSITION: usize = 2;
    pub const PLAYING: usize = 3;
    pub const RECORDING: usize = 4;
    pub const BPM: usize = 5;
    pub const LATENCY: usize = 6;
    /// 1 when a sub-block of the last render jumped or followed a gap; the `JUMP_*` slots
    /// describe the last such sub-block.
    pub const JUMPED: usize = 7;
    pub const JUMP_OFFSET: usize = 8;
    pub const JUMP_POSITION: usize = 9;
    pub const JUMP_PLAYING: usize = 10;
    pub const JUMP_RECORDING: usize = 11;
    pub const JUMP_BPM: usize = 12;
    pub const JUMP_LATENCY: usize = 13;
    pub const LEN: usize = 14;
}

/// Engine config for the web: one render quantum per `process`, stereo out, no inputs yet.
pub fn web_engine_config(sample_rate: u32) -> EngineConfig {
    EngineConfig {
        sample_rate,
        max_block_size: RENDER_QUANTUM,
        input_channels: 0,
        output_channels: OUTPUT_CHANNELS as u16,
        max_nodes: 1024,
        ..EngineConfig::default()
    }
}

struct Sources<'a>(&'a BTreeMap<MediaId, Arc<dyn AudioSource>>);

impl SampleResolver for Sources<'_> {
    fn resolve(&self, media: MediaId) -> Option<Arc<dyn AudioSource>> {
        self.0.get(&media).cloned()
    }
}

/// The Worklet-side engine host. Generic over the ring memory so it runs natively in tests.
pub struct EngineHost<M: RingMemory> {
    engine: ether_core::Engine,
    handle: EngineHandle,
    gc: GarbageCollector,
    /// `Option` only so it can be taken out while frames borrow `self` mutably.
    control: Option<RingReader<M>>,
    media: MediaAssembler,
    reports: RingWriter<M>,
    /// Worker's virtual key → real engine key.
    keys: BTreeMap<NodeKey, NodeKey>,
    /// Registered sources (also resolves sampler media).
    sources: BTreeMap<MediaId, Arc<dyn AudioSource>>,
    outputs: EngineOutputs,
    report: EngineReport,
    report_buf: Vec<u8>,
    out: Vec<Vec<f32>>,
    /// Stream tap reader and its planar copy of the last render (see the module docs).
    tap: StreamTapReader,
    tap_out: Vec<Vec<f32>>,
    tap_clock: Vec<f64>,
    blocks_since_report: u32,
    blocks: u64,
    /// Errors not yet delivered (report ring was full).
    errors: Vec<String>,
    /// `media-preview`: a natural preview end not yet delivered (reports are lossy).
    preview_ended: Option<u64>,
}

impl<M: RingMemory> EngineHost<M> {
    pub fn new(sample_rate: u32, control: M, reports: M) -> Self {
        let mut parts = ether_core::create(web_engine_config(sample_rate));
        let (tap_writer, tap) = stream_tap_ring(TAP_RING_FRAMES);
        // Applied on the first render; the queue is empty at creation.
        let _ = parts.handle.set_stream_tap(Some(tap_writer));
        Self {
            engine: parts.engine,
            handle: parts.handle,
            gc: parts.gc,
            control: Some(RingReader::with_capacity(control, 128 * 1024)),
            media: MediaAssembler::default(),
            reports: RingWriter::new(reports),
            keys: BTreeMap::new(),
            sources: BTreeMap::new(),
            outputs: EngineOutputs {
                meters: Vec::with_capacity(256),
                ..Default::default()
            },
            report: EngineReport {
                meters: Vec::with_capacity(256),
                ..Default::default()
            },
            report_buf: Vec::with_capacity(EngineReport::encoded_len(256)),
            out: vec![vec![0.0; RENDER_QUANTUM]; OUTPUT_CHANNELS],
            tap,
            tap_out: vec![vec![0.0; RENDER_QUANTUM]; TAP_CHANNELS],
            tap_clock: vec![0.0; tap_clock::LEN],
            blocks_since_report: 0,
            blocks: 0,
            errors: Vec::new(),
            preview_ended: None,
        }
    }

    /// Planar output of the last [`Self::render`] call (`RENDER_QUANTUM` frames).
    pub fn output(&self, channel: usize) -> &[f32] {
        &self.out[channel]
    }

    /// Planar stream tap of the last [`Self::render`] call (`RENDER_QUANTUM` frames).
    pub fn tap_output(&self, channel: usize) -> &[f32] {
        &self.tap_out[channel]
    }

    /// Tap clock header of the last [`Self::render`] call (layout [`tap_clock`]).
    pub fn tap_clock(&self) -> &[f64] {
        &self.tap_clock
    }

    /// Pointer to an output channel (the JS side views it through wasm memory): `0..2` the
    /// main output, `2..4` the stream tap, [`TAP_CLOCK_SLOT`] the tap clock header (`f64`s).
    pub fn output_ptr(&self, channel: usize) -> *const f32 {
        match channel {
            c if c < OUTPUT_CHANNELS => self.out[c].as_ptr(),
            c if c < TAP_CLOCK_SLOT => self.tap_out[c - OUTPUT_CHANNELS].as_ptr(),
            _ => self.tap_clock.as_ptr().cast(),
        }
    }

    /// Render one block of `frames` (`<= RENDER_QUANTUM`) frames.
    pub fn render(&mut self, frames: usize) {
        let frames = frames.min(RENDER_QUANTUM);
        self.apply_control(CONTROL_BUDGET_BYTES);
        {
            let [l, r] = &mut self.out[..] else {
                unreachable!("stereo output")
            };
            let mut outs: [&mut [f32]; OUTPUT_CHANNELS] = [&mut l[..frames], &mut r[..frames]];
            self.engine.process(&[], &mut outs, frames);
        }
        self.drain_tap(frames);
        self.blocks += 1;
        self.blocks_since_report += 1;
        if self.blocks_since_report >= REPORT_INTERVAL_BLOCKS {
            self.blocks_since_report = 0;
            self.send_report();
        }
        self.gc.collect();
    }

    /// **RT.** Move what the engine tapped during this render (exactly `frames` frames
    /// unless blocks were skipped) into `tap_out`, deinterleaved; silence for the rest.
    fn drain_tap(&mut self, frames: usize) {
        let clock = &mut self.tap_clock[..];
        clock.fill(0.0);
        let [tl, tr] = &mut self.tap_out[..] else {
            unreachable!("stereo tap")
        };
        let mut at = 0usize;
        let mut blocks = 0.0;
        while let Ok(block) = self.tap.blocks.pop() {
            let n = block.frames as usize;
            let offset = at.min(frames);
            let put = |c: &mut [f64], base: usize, b: &StreamBlock| {
                c[base] = offset as f64;
                c[base + 1] = b.position;
                c[base + 2] = f64::from(u8::from(b.playing));
                c[base + 3] = f64::from(u8::from(b.recording));
                c[base + 4] = b.bpm;
                c[base + 5] = f64::from(b.latency);
            };
            blocks += 1.0;
            put(clock, tap_clock::OFFSET, &block);
            if block.jump || block.gap {
                clock[tap_clock::JUMPED] = 1.0;
                put(clock, tap_clock::JUMP_OFFSET, &block);
            }
            let Ok(chunk) = self
                .tap
                .audio
                .read_chunk((2 * n).min(self.tap.audio.slots()))
            else {
                continue;
            };
            let (a, b) = chunk.as_slices();
            for (k, &v) in a.iter().chain(b.iter()).enumerate() {
                let i = at + k / 2;
                if i >= frames {
                    break;
                }
                if k % 2 == 0 {
                    tl[i] = v;
                } else {
                    tr[i] = v;
                }
            }
            chunk.commit_all();
            at += n;
        }
        clock[tap_clock::BLOCKS] = blocks;
        for ch in [tl, tr] {
            if at < frames {
                ch[at..frames].fill(0.0);
            }
        }
    }

    /// Apply pending control frames, within `budget` bytes and the per-quantum limits.
    pub fn apply_control(&mut self, budget: usize) {
        let Some(mut control) = self.control.take() else {
            return;
        };
        let mut frames = 0;
        control.drain(budget, |bytes| {
            frames += 1;
            let heavy = self.apply_frame(bytes);
            !heavy && frames < MAX_FRAMES_PER_QUANTUM
        });
        let skipped = control.take_skipped();
        if skipped > 0 {
            self.errors
                .push(format!("control ring corrupt: skipped {skipped} bytes"));
        }
        self.control = Some(control);
    }

    /// Apply one frame; returns `true` if it was heavy (stop draining this quantum).
    fn apply_frame(&mut self, bytes: &[u8]) -> bool {
        let (heavy, res) = match Frame::decode(bytes) {
            Err(e) => (false, Err(format!("bad engine message: {e}"))),
            Ok(Frame::Msg(msg)) => (matches!(msg, EngineMsg::Publish { .. }), self.apply(msg)),
            Ok(Frame::MediaBegin {
                media,
                sample_rate,
                channels,
                frames,
            }) => {
                self.media.begin(media, sample_rate, channels, frames);
                (true, Ok(()))
            }
            Ok(Frame::MediaChunk {
                media,
                channel,
                offset,
                samples,
            }) => (false, self.media.chunk(media, channel, offset, samples)),
            Ok(Frame::MediaEnd { media }) => match self.media.end(media) {
                Some(audio) => (false, self.add_source(media, audio)),
                // Unloaded while loading.
                None => (false, Ok(())),
            },
        };
        if let Err(e) = res {
            self.errors.push(e);
        }
        heavy
    }

    fn add_source(
        &mut self,
        media: MediaId,
        audio: Arc<ether_media::DecodedAudio>,
    ) -> Result<(), String> {
        let source: Arc<dyn AudioSource> = Arc::new(InMemorySource::new(audio));
        self.sources.insert(media, source.clone());
        if media == PREVIEW_MEDIA {
            // Waits for its `Preview` message; not an engine (clip) source.
            return Ok(());
        }
        self.handle
            .add_source(media, source)
            .map_err(|e| e.to_string())
    }

    fn real_key(&self, key: NodeKey) -> Result<NodeKey, String> {
        self.keys
            .get(&key)
            .copied()
            .ok_or_else(|| format!("unknown node {key:?}"))
    }

    fn apply(&mut self, msg: EngineMsg) -> Result<(), String> {
        match msg {
            EngineMsg::CreateBuiltin {
                key,
                device,
                params,
            } => {
                let mut node = ether_devices::create(&device, &Sources(&self.sources));
                for (id, v) in params {
                    node.set_param(id, v);
                }
                let real = self.handle.add_node(node).map_err(|e| e.to_string())?;
                if let Some(old) = self.keys.insert(key, real) {
                    let _ = self.handle.remove_node(old);
                }
                Ok(())
            }
            EngineMsg::DestroyNode { key } => {
                let real = self
                    .keys
                    .remove(&key)
                    .ok_or_else(|| format!("unknown node {key:?}"))?;
                self.handle.remove_node(real).map_err(|e| e.to_string())
            }
            EngineMsg::LoadMedia { media, audio } => self.add_source(media, audio),
            EngineMsg::UnloadMedia { media } => {
                self.media.cancel(media);
                self.sources.remove(&media);
                self.handle.remove_source(media).map_err(|e| e.to_string())
            }
            EngineMsg::Publish { graph } => {
                let (graph, unknown) = self.remap_graph(*graph);
                let res = self.handle.publish(graph).map_err(|e| e.to_string());
                if unknown > 0 {
                    self.errors
                        .push(format!("graph references {unknown} unknown node(s)"));
                }
                res
            }
            EngineMsg::SetParam { mut change } => {
                if let ParamTarget::Node { node, .. } = &mut change.target {
                    *node = self.real_key(*node)?;
                }
                self.handle.set_param(change).map_err(|e| e.to_string())
            }
            EngineMsg::Transport { control } => {
                self.handle.transport(control).map_err(|e| e.to_string())
            }
            EngineMsg::Preview { id, media, gain } => {
                use ether_core::preview::PreviewControl;
                let control = match media {
                    Some(media) => PreviewControl::Play {
                        id,
                        // The voice owns it from now on (retired to the GC when done).
                        source: self
                            .sources
                            .remove(&media)
                            .ok_or_else(|| format!("preview of unknown media {media}"))?,
                        gain,
                    },
                    None => PreviewControl::Stop,
                };
                self.handle.preview(control).map_err(|e| e.to_string())
            }
        }
    }

    /// Map virtual node keys to real ones; drop chain entries/automation whose node is
    /// unknown. Returns the graph and the number of dropped references.
    fn remap_graph(&self, mut graph: RenderGraphDesc) -> (RenderGraphDesc, usize) {
        let keys = &self.keys;
        let mut unknown = 0;
        let mut map = |k: &mut NodeKey| match keys.get(k) {
            Some(real) => {
                *k = *real;
                true
            }
            None => {
                unknown += 1;
                false
            }
        };
        for track in &mut graph.tracks {
            track.chain.retain_mut(|e| map(&mut e.node));
            let lanes = track
                .automation
                .iter_mut()
                .chain(track.clips.iter_mut().flat_map(|c| c.envelopes.iter_mut()));
            for lane in lanes {
                if let ResolvedTarget::Node { node, .. } = &mut lane.resolved
                    && !map(node)
                {
                    *node = DEAD_KEY;
                }
            }
            let live = |a: &AutomationDesc| !matches!(a.resolved, ResolvedTarget::Node { node, .. } if node == DEAD_KEY);
            track.automation.retain(live);
            for c in &mut track.clips {
                c.envelopes.retain(live);
            }
        }
        (graph, unknown)
    }

    fn send_report(&mut self) {
        while let Some(e) = self.errors.first() {
            let mut msg = Vec::with_capacity(1 + e.len());
            msg.push(REPORT_ERROR);
            msg.extend_from_slice(e.as_bytes());
            if !self.reports.try_send_now(&msg) {
                break;
            }
            self.errors.remove(0);
        }
        self.handle.poll(&mut self.outputs);
        self.report.playhead = self.outputs.playhead.unwrap_or_default();
        self.report.meters.clear();
        self.report
            .meters
            .extend(self.outputs.meters.iter().cloned());
        self.report.event_overflow = self.outputs.event_overflow;
        self.report.underruns = self.outputs.underruns;
        self.report.blocks = self.blocks;
        if self.outputs.preview_ended.is_some() {
            self.preview_ended = self.outputs.preview_ended;
        }
        self.report.preview_ended = self.preview_ended;
        self.report.encode_into(&mut self.report_buf);
        // Lossy: if the Worker isn't reading, drop this report (meters are max-held per
        // report, playhead is always the latest; a preview end is kept until delivered).
        if self.reports.try_send_now(&self.report_buf) {
            self.preview_ended = None;
        }
    }

    /// Control bytes written by the Worker but not applied yet.
    pub fn control_backlog(&self) -> usize {
        self.control.as_ref().map_or(0, RingReader::available)
    }

    /// Blocks rendered so far.
    pub fn blocks(&self) -> u64 {
        self.blocks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ring::HeapMemory;

    #[test]
    fn stream_tap_is_drained_every_render() {
        let mut host = EngineHost::new(48_000, HeapMemory::new(1 << 12), HeapMemory::new(1 << 12));
        for i in 0..4 {
            host.render(RENDER_QUANTUM);
            let clock = host.tap_clock();
            assert!(clock[tap_clock::BLOCKS] >= 1.0, "render {i} tapped nothing");
            // The tap installs with a jump (a fresh timeline for the reader).
            assert_eq!(clock[tap_clock::JUMPED], if i == 0 { 1.0 } else { 0.0 });
            assert_eq!(clock[tap_clock::PLAYING], 0.0);
            assert_eq!(host.tap_output(0), host.output(0));
            assert_eq!(host.tap_output(1), host.output(1));
        }
        assert_eq!(
            host.output_ptr(OUTPUT_CHANNELS),
            host.tap_output(0).as_ptr()
        );
        assert_eq!(
            host.output_ptr(TAP_CLOCK_SLOT).cast::<f64>(),
            host.tap_clock().as_ptr()
        );
    }
}

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
//! 4. every [`crate::latency::REPORT_INTERVAL_MS`], send a [`crate::latency::LatencyReport`]
//!    of the nodes whose latency changed (pre-sized buffer, no allocation),
//! 5. run the `GarbageCollector`.
//!
//! A convolution reverb's new IR (from the user's media too, whose audio the
//! Worker asks the controller for again) is read, normalized and FFT-partitioned on the
//! Worker (`ether_devices::fx_space::PreparedIr`) and arrives as [`Frame::IrBegin`] +
//! [`Frame::IrChunk`]s; the host copies them into a `ConvolverAssembly` (within the control
//! budget), zero-fills the rest [`IR_FILL_PER_QUANTUM`] values per quantum, then hands the
//! finished convolver to the node (`Node::set_data`), which crossfades to it. So no quantum
//! builds an IR.
//!
//! The engine's stream tap (`ether_core::stream_tap`: master + metronome/count-in, minus the
//! preview voice; "listen on <peer>") is installed once at construction
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
//! see `crate::perf` (feature `bench`).
//!
//! The frame cap also keeps core's queues from overflowing: every quantum drains them
//! (control 1024, params 4096 entries), and at most [`MAX_FRAMES_PER_QUANTUM`] entries are
//! pushed per quantum, so a backlog (e.g. after a suspended AudioContext resumes) is
//! applied over several quanta instead of failing with `QueueFull`.

use std::collections::BTreeMap;
use std::sync::Arc;

use ether_core::graph::AutomationDesc;
use ether_core::protocol::model::MediaId;
use ether_core::stream_tap::{StreamBlock, StreamTapReader, stream_tap_ring};
use ether_core::{
    AudioSource, EngineConfig, EngineHandle, EngineOutputs, GarbageCollector, NodeKey, ParamTarget,
    RenderGraphDesc,
};
use ether_devices::SampleResolver;
use ether_media::InMemorySource;

use crate::latency::LatencyTracker;
use crate::proto::{
    ANALYSIS_REPORT_BYTES, EngineMsg, EngineReport, Frame, MediaAssembler, PREVIEW_MEDIA,
    REPORT_ERROR, encode_analysis_into,
};
use crate::ring::{RingMemory, RingReader, RingWriter};

/// Web render quantum.
pub const RENDER_QUANTUM: usize = 128;
/// Control-ring bytes applied per block (bounds non-RT work on the audio thread; about
/// four media chunks).
pub const CONTROL_BUDGET_BYTES: usize = 256 * 1024;
/// Complex values (8 bytes) of prepared-IR buffers zero-filled per block (1 MiB: a
/// memset, cheaper than decoding the control budget's 256 KiB of chunks).
pub const IR_FILL_PER_QUANTUM: usize = 128 * 1024;
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

/// Most stretcher voices on the web (feature `warp-complex`): every voice runs on the one
/// AudioWorklet thread (~31 µs per 128-frame stereo block in V8, ~1.2% of the quantum),
/// so Complex clips beyond this play Repitch.
pub const WEB_STRETCH_VOICES: usize = 12;

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

/// `Node::set_data` payload of a data-only built-in change (`EngineMsg::UpdateBuiltin`;
/// the same data the native bridge's `update_builtin` sends): sampler slices,
/// multisampler zones, a convolution reverb's IR (factory and media IRs arrive prepared
/// instead, [`Frame::IrBegin`]; what still comes here is no IR, an unknown factory id or
/// media this Worklet doesn't have: nothing to build). `None` for devices without in-place
/// updates.
pub fn node_data(
    device: &ether_core::protocol::model::BuiltinDevice,
    sources: &dyn SampleResolver,
    sample_rate: u32,
) -> Option<ether_core::node::NodeData> {
    use ether_core::protocol::model::BuiltinDevice;
    match device {
        BuiltinDevice::Sampler { slices, .. } => Some(Box::new(slices.clone())),
        BuiltinDevice::MultiSampler { .. } => Some(Box::new(
            ether_devices::multisampler::zone_set(device, sources),
        )),
        BuiltinDevice::ConvolutionReverb { .. } => {
            ether_devices::fx_space::ir_swap(device, sources, sample_rate as f32)
        }
        BuiltinDevice::VolumeLfo { .. } => ether_devices::fx_modulation::volume_lfo_curve(device),
        _ => None,
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
    /// Real → virtual node keys (analysis frames are reported under the Worker's keys).
    virtual_keys: BTreeMap<NodeKey, NodeKey>,
    /// Reused encoding buffer of analysis reports ([`ANALYSIS_REPORT_BYTES`]).
    analysis_buf: Vec<u8>,
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
    /// Streamed media caches ([`crate::media_stream`]).
    streams: crate::media_stream::WorkletStreams,
    /// Node latency changes reported to the Worker (PDC republish).
    latency: LatencyTracker,
    /// Engine rate (node data built here, e.g. convolution IRs, is made at this rate).
    sample_rate: u32,
    /// Convolution IRs prepared by the Worker being assembled, by virtual node key.
    irs: BTreeMap<NodeKey, ether_devices::fx_space::ConvolverAssembly>,
    /// `keyboard-midi`: the engine's live MIDI ring (`EngineMsg::LiveMidi`). The rest of the
    /// recording rings is dropped: the web build captures nothing.
    live_midi: Option<ether_core::recording::rtrb::Producer<ether_core::recording::LiveMidi>>,
}

impl<M: RingMemory> EngineHost<M> {
    pub fn new(sample_rate: u32, control: M, reports: M) -> Self {
        let config = web_engine_config(sample_rate);
        let latency = LatencyTracker::new(sample_rate, RENDER_QUANTUM, config.max_nodes);
        let mut parts = ether_core::create(config);
        // Complex warp (feature `warp-complex`): the pure-Rust Signalsmith Stretch with the
        // web preset; without it Complex clips play Repitch.
        #[cfg(feature = "warp-complex")]
        {
            parts
                .handle
                .set_stretcher_factory(Arc::new(ether_stretch::RustStretchFactory {
                    preset: ether_stretch::RustPreset::Web,
                }));
            parts.handle.set_stretch_voice_limit(WEB_STRETCH_VOICES);
        }
        let (tap_writer, tap) = stream_tap_ring(TAP_RING_FRAMES);
        // Applied on the first render; the queue is empty at creation.
        let _ = parts.handle.set_stream_tap(Some(tap_writer));
        let live_midi = parts.handle.take_recording_io().map(|io| io.midi_in);
        Self {
            engine: parts.engine,
            handle: parts.handle,
            gc: parts.gc,
            control: Some(RingReader::with_capacity(control, 128 * 1024)),
            media: MediaAssembler::default(),
            reports: RingWriter::new(reports),
            keys: BTreeMap::new(),
            virtual_keys: BTreeMap::new(),
            analysis_buf: Vec::with_capacity(ANALYSIS_REPORT_BYTES),
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
            streams: Default::default(),
            latency,
            sample_rate,
            irs: BTreeMap::new(),
            live_midi,
        }
    }

    /// Prepared IRs still being assembled (tests).
    pub fn pending_irs(&self) -> usize {
        self.irs.len()
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
        self.assemble_ir();
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
        self.send_latency();
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
            Ok(Frame::Msg(msg)) => (
                matches!(
                    msg,
                    EngineMsg::Publish { .. } | EngineMsg::StreamOpen { .. }
                ),
                self.apply(msg),
            ),
            Ok(Frame::StreamChunk {
                media,
                slot,
                chunk,
                channel,
                flags,
                samples,
            }) => {
                self.streams
                    .chunk(media, slot, chunk, channel, flags, samples);
                (false, Ok(()))
            }
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
            Ok(Frame::IrBegin {
                key,
                sample_rate,
                header,
                source,
            }) => (true, self.begin_ir(key, sample_rate, header, &source)),
            Ok(Frame::IrChunk {
                key,
                segment,
                offset,
                values,
            }) => {
                let res = match self.irs.get_mut(&key) {
                    Some(a) => a.push(segment as usize, offset as usize, values),
                    // Superseded, or its begin was refused (reported then).
                    None => Ok(()),
                };
                if res.is_err() {
                    self.irs.remove(&key);
                }
                (false, res)
            }
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

    /// Start assembling a prepared IR for `key` (supersedes one in flight).
    fn begin_ir(
        &mut self,
        key: NodeKey,
        sample_rate: u32,
        header: ether_devices::fx_space::IrHeader,
        source: &ether_core::protocol::model::IrSource,
    ) -> Result<(), String> {
        use ether_devices::fx_space::{ConvolverAssembly, resolve};
        self.irs.remove(&key);
        self.real_key(key)?;
        if sample_rate != self.sample_rate {
            return Err(format!(
                "IR prepared at {sample_rate} Hz for an engine at {} Hz",
                self.sample_rate
            ));
        }
        let input = resolve(Some(source), &Sources(&self.sources))
            .ok_or_else(|| format!("prepared IR of unknown source {source:?}"))?;
        let asm = ConvolverAssembly::begin(input, header, sample_rate as f32)?;
        self.irs.insert(key, asm);
        Ok(())
    }

    /// Zero-fill a prepared IR whose stream has arrived (bounded); once complete,
    /// hand its convolver to the node, which crossfades to it.
    fn assemble_ir(&mut self) {
        let Some((&key, asm)) = self.irs.iter_mut().find(|(_, a)| a.received()) else {
            return;
        };
        if !asm.fill(IR_FILL_PER_QUANTUM) {
            return;
        }
        let Some(conv) = self.irs.remove(&key).and_then(|a| a.finish()) else {
            unreachable!("fill reported the assembly complete")
        };
        let res = self.real_key(key).and_then(|real| {
            self.handle
                .set_node_data(real, ether_devices::fx_space::swap_to(Some(conv)))
                .map_err(|e| e.to_string())
        });
        if let Err(e) = res {
            self.errors.push(e);
        }
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
                self.irs.remove(&key);
                let real = self.handle.add_node(node).map_err(|e| e.to_string())?;
                // The latency the engine compiles PDC with until it changes (not reported).
                self.latency
                    .add(key, self.handle.node_latency(real).unwrap_or(0));
                if let Some(old) = self.keys.insert(key, real) {
                    self.virtual_keys.remove(&old);
                    let _ = self.handle.remove_node(old);
                }
                self.virtual_keys.insert(real, key);
                Ok(())
            }
            EngineMsg::DestroyNode { key } => {
                self.irs.remove(&key);
                let real = self
                    .keys
                    .remove(&key)
                    .ok_or_else(|| format!("unknown node {key:?}"))?;
                self.virtual_keys.remove(&real);
                self.latency.remove(key);
                self.handle.remove_node(real).map_err(|e| e.to_string())
            }
            EngineMsg::LoadMedia { media, audio } => self.add_source(media, audio),
            EngineMsg::UnloadMedia { media } => {
                self.media.cancel(media);
                self.streams.remove(media);
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
            EngineMsg::Suspend { suspended } => {
                self.handle.set_suspended(suspended);
                Ok(())
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
            EngineMsg::NodeScale { key, scale } => {
                let real = self.real_key(key)?;
                self.handle
                    .set_node_data(real, Box::new(scale))
                    .map_err(|e| e.to_string())
            }
            EngineMsg::WatchAnalysis { key, on } => {
                let real = self.real_key(key)?;
                self.handle
                    .watch_analysis(real, on)
                    .map_err(|e| e.to_string())
            }
            EngineMsg::StreamOpen {
                media,
                channels,
                frames,
                slots,
            } => {
                let cache = self.streams.open(media, channels, frames, slots);
                let source: Arc<dyn AudioSource> = cache;
                self.sources.insert(media, source.clone());
                self.handle
                    .add_source(media, source)
                    .map_err(|e| e.to_string())
            }
            EngineMsg::UpdateBuiltin { key, device } => {
                // The newest change wins over a prepared IR still in flight.
                self.irs.remove(&key);
                let real = self.real_key(key)?;
                let Some(data) = node_data(&device, &Sources(&self.sources), self.sample_rate)
                else {
                    return Err(format!("{:?} has no in-place update", device.device_type()));
                };
                self.handle
                    .set_node_data(real, data)
                    .map_err(|e| e.to_string())
            }
            // Stamped 0 = "as soon as possible": the next rendered block, at its first frame
            // (control frames are drained right before each render).
            // From the computer MIDI keyboard: tracks filtered to another port ignore it.
            EngineMsg::LiveMidi { data } => match self.live_midi.as_mut() {
                Some(ring) => ring
                    .push(ether_core::recording::LiveMidi {
                        sample_time: 0,
                        data,
                        port: ether_core::recording::COMPUTER_KEYBOARD_KEY,
                    })
                    .map_err(|_| "live MIDI queue full".to_string()),
                None => Err("live MIDI unavailable".into()),
            },
            EngineMsg::PreviewNote { track, data } => self
                .handle
                .preview_note(track, data)
                .map_err(|e| e.to_string()),
        }
    }

    /// **RT.** Forward the engine's analysis frames to the Worker, one report each, under
    /// the Worker's virtual node keys. Lossy like the other reports: a frame that doesn't
    /// fit the report ring is dropped (the controller keeps the latest per device anyway).
    fn send_analysis(&mut self) {
        let Self {
            handle,
            reports,
            virtual_keys,
            analysis_buf,
            ..
        } = self;
        handle.poll_analysis(|f| {
            if let Some(&key) = virtual_keys.get(&f.node) {
                encode_analysis_into(f, key, analysis_buf);
                let _ = reports.try_send_now(analysis_buf);
            }
        });
    }

    /// **RT.** every report interval, send the latencies that changed since
    /// the last delivered report. Lossy like the other reports, but never lost: an
    /// undelivered change is still a change at the next scan.
    fn send_latency(&mut self) {
        let Self {
            latency,
            keys,
            handle,
            reports,
            ..
        } = self;
        if let Some(report) = latency.tick(keys, handle)
            && reports.try_send_now(report)
        {
            latency.commit();
        }
    }

    /// Map virtual node keys to real ones, everywhere the desc references a node (chains at
    /// any depth, racks, automation, modulation, hardware I/O, sidechain taps); drop what
    /// references an unknown node. Returns the graph and the number of dropped chain
    /// entries and automation lanes.
    ///
    /// Every reference must be mapped: a virtual key left as is may equal the real key of
    /// another node (virtual keys count from 1, real slots from 0), which the compiler
    /// rejects as a node used twice, or silently drives the wrong node.
    fn remap_graph(&self, graph: RenderGraphDesc) -> (RenderGraphDesc, usize) {
        remap_graph(&self.keys, graph)
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
        if !self.streams.is_empty() {
            // Lossy too: the next report carries the cursors again.
            let report = self.streams.encode_report();
            let _ = self.reports.try_send_now(report);
        }
        self.send_analysis();
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

/// [`EngineHost::remap_graph`] with the Worker → engine key map `keys`.
fn remap_graph(
    keys: &BTreeMap<NodeKey, NodeKey>,
    mut graph: RenderGraphDesc,
) -> (RenderGraphDesc, usize) {
    use ether_core::ScPoint;
    use ether_core::graph::ChainEntry;
    use ether_core::modulation::ModSourceDesc;

    let real = |k: &mut NodeKey| match keys.get(k) {
        Some(r) => {
            *k = *r;
            true
        }
        None => false,
    };
    // Drops the entries with an unknown node; returns how many.
    let chain = |c: &mut Vec<ChainEntry>| {
        let len = c.len();
        c.retain_mut(|e| real(&mut e.node));
        len - c.len()
    };
    let mut unknown = 0;
    for track in &mut graph.tracks {
        unknown += chain(&mut track.chain);
        // Drum racks and their pads (pads may hold racks: their chains are in `chain_racks`).
        track.racks.retain_mut(|r| real(&mut r.rack));
        for pad in track.racks.iter_mut().flat_map(|r| &mut r.pads) {
            unknown += chain(&mut pad.chain);
        }
        // Instrument / effect racks at any depth.
        track.chain_racks.retain_mut(|r| real(&mut r.rack));
        for c in track.chain_racks.iter_mut().flat_map(|r| &mut r.chains) {
            unknown += chain(&mut c.chain);
        }
        let lanes = track
            .automation
            .iter_mut()
            .chain(track.clips.iter_mut().flat_map(|c| c.envelopes.iter_mut()));
        for lane in lanes {
            if let Some(node) = lane.resolved.node_mut()
                && !real(node)
            {
                *node = DEAD_KEY;
                unknown += 1;
            }
        }
        let live = |a: &AutomationDesc| a.resolved.node() != Some(DEAD_KEY);
        track.automation.retain(live);
        for c in &mut track.clips {
            c.envelopes.retain(live);
        }
        // Modulators stay (mappings index them): an unknown host matches no node.
        let modulation = &mut track.modulation;
        for m in &mut modulation.modulators {
            if !real(&mut m.host) {
                m.host = DEAD_KEY;
            }
        }
        modulation.mappings.retain_mut(|m| {
            let source = match &mut m.source {
                ModSourceDesc::Macro { rack, .. } => real(rack),
                ModSourceDesc::Modulator(_) => true,
            };
            source && real(&mut m.node)
        });
        modulation.macros.retain_mut(|m| real(&mut m.rack));
        track.hw_io.retain_mut(|h| real(&mut h.node));
        track.sidechain_taps.retain_mut(|t| {
            let at = match &mut t.point {
                ScPoint::After(node) => real(node),
                _ => true,
            };
            real(&mut t.node) && at
        });
    }
    (graph, unknown)
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

    /// Every node reference of the desc is mapped (not only the track chain's): racks, pad
    /// and rack chains, modulation, hardware I/O and sidechain taps. Unknown ones are
    /// dropped, never left as a virtual key that may name another real node.
    #[test]
    fn remap_graph_maps_every_node_reference() {
        use ether_core::graph::{ChainEntry, PadDesc, RackDesc, TrackDesc};
        use ether_core::hw_io::HwIoDesc;
        use ether_core::modulation::{
            MacroDesc, ModMappingDesc, ModSourceDesc, ModulationDesc, ModulatorDesc,
        };
        use ether_core::protocol::devices::ParamScale;
        use ether_core::protocol::model::{
            DrumPadId, ModulatorId, ModulatorKind, ParamId, RackChainId, TrackId, TrackKind, Ulid,
        };
        use ether_core::rack_chains::{ChainRackDesc, ChainRackKind, RackChainDesc};
        use ether_core::{ScPoint, SidechainTapDesc};

        let key = |index| NodeKey {
            index,
            generation: 1,
        };
        // Virtual n → real n - 1 + 100 (no overlap, so a missed mapping shows).
        let keys: BTreeMap<NodeKey, NodeKey> = (1..=8).map(|n| (key(n), key(n + 99))).collect();
        let gone = key(50);
        let entry = |node| ChainEntry {
            node,
            enabled: true,
            sidechain: None,
        };
        let mapping = ether_core::graph::ParamMapping {
            min: 0.0,
            max: 1.0,
            scale: ParamScale::Linear,
            steps: None,
        };
        let map_to = |source, node| ModMappingDesc {
            source,
            node,
            param: ParamId(0),
            depth: 1.0,
            mapping,
            base: 0.0,
        };
        let track = TrackDesc {
            id: TrackId(Ulid(1)),
            kind: TrackKind::Midi,
            chain: vec![entry(key(1)), entry(key(2)), entry(gone)],
            output: None,
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
            racks: vec![
                RackDesc {
                    rack: key(1),
                    pads: vec![PadDesc {
                        pad: DrumPadId(Ulid(2)),
                        note: 36,
                        choke_group: None,
                        chain: vec![entry(key(3)), entry(gone)],
                        volume: 1.0,
                        pan: 0.0,
                        mute: false,
                    }],
                },
                RackDesc {
                    rack: gone,
                    pads: vec![],
                },
            ],
            frozen: None,
            chain_racks: vec![ChainRackDesc {
                rack: key(2),
                kind: ChainRackKind::AudioEffect,
                chains: vec![RackChainDesc {
                    id: RackChainId(Ulid(3)),
                    chain: vec![entry(key(4))],
                    volume: 1.0,
                    pan: 0.0,
                    mute: false,
                    keys: (0, 127),
                    velocities: (0, 127),
                    select: (0, 127),
                }],
                selector: 0,
            }],
            modulation: ModulationDesc {
                modulators: vec![
                    ModulatorDesc {
                        id: ModulatorId(Ulid(4)),
                        host: key(5),
                        kind: ModulatorKind::Lfo,
                        params: vec![],
                        sidechain: None,
                    },
                    ModulatorDesc {
                        id: ModulatorId(Ulid(5)),
                        host: gone,
                        kind: ModulatorKind::Lfo,
                        params: vec![],
                        sidechain: None,
                    },
                ],
                mappings: vec![
                    map_to(ModSourceDesc::Modulator(0), key(4)),
                    map_to(
                        ModSourceDesc::Macro {
                            rack: key(2),
                            index: 0,
                        },
                        key(3),
                    ),
                    map_to(ModSourceDesc::Modulator(0), gone),
                ],
                macros: vec![
                    MacroDesc {
                        rack: key(2),
                        values: Default::default(),
                        first: 0,
                    },
                    MacroDesc {
                        rack: gone,
                        values: Default::default(),
                        first: 0,
                    },
                ],
            },
            input_tap: None,
            vca: None,
            expression: Default::default(),
            hw_io: vec![
                HwIoDesc {
                    node: key(6),
                    routing: Default::default(),
                },
                HwIoDesc {
                    node: gone,
                    routing: Default::default(),
                },
            ],
            sidechain_taps: vec![
                SidechainTapDesc {
                    node: key(7),
                    point: ScPoint::After(key(8)),
                },
                SidechainTapDesc {
                    node: key(7),
                    point: ScPoint::After(gone),
                },
            ],
        };
        let (graph, unknown) = remap_graph(
            &keys,
            RenderGraphDesc {
                tracks: vec![track],
                ..Default::default()
            },
        );
        assert_eq!(unknown, 2, "one track chain entry, one pad chain entry");
        let t = &graph.tracks[0];
        let nodes = |c: &[ChainEntry]| c.iter().map(|e| e.node.index).collect::<Vec<_>>();
        assert_eq!(nodes(&t.chain), [100, 101]);
        assert_eq!(t.racks.len(), 1);
        assert_eq!(t.racks[0].rack.index, 100);
        assert_eq!(nodes(&t.racks[0].pads[0].chain), [102]);
        assert_eq!(t.chain_racks[0].rack.index, 101);
        assert_eq!(nodes(&t.chain_racks[0].chains[0].chain), [103]);
        let m = &t.modulation;
        assert_eq!(m.modulators.len(), 2, "mappings index modulators");
        assert_eq!(m.modulators[0].host.index, 104);
        assert_eq!(m.modulators[1].host, DEAD_KEY);
        assert_eq!(m.mappings.len(), 2);
        assert_eq!(m.mappings[0].node.index, 103);
        assert!(
            matches!(m.mappings[1].source, ModSourceDesc::Macro { rack, .. } if rack.index == 101)
        );
        assert_eq!(m.mappings[1].node.index, 102);
        assert_eq!(m.macros.len(), 1);
        assert_eq!(m.macros[0].rack.index, 101);
        assert_eq!(t.hw_io.len(), 1);
        assert_eq!(t.hw_io[0].node.index, 105);
        assert_eq!(t.sidechain_taps.len(), 1);
        assert_eq!(t.sidechain_taps[0].node.index, 106);
        assert_eq!(t.sidechain_taps[0].point, ScPoint::After(key(107)));
    }

    #[test]
    fn complex_warp_has_a_stretcher_with_the_feature() {
        let host = EngineHost::new(48_000, HeapMemory::new(1 << 12), HeapMemory::new(1 << 12));
        assert_eq!(
            host.handle.stretching_supported(),
            cfg!(feature = "warp-complex")
        );
    }
}

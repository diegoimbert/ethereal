//! Controller side (Worker): [`WebBridge`] implements `EngineBridge` by serializing every
//! call into the control ring for the AudioWorklet, and reads [`EngineReport`]s back.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::sync::Arc;

use ether_controller::{BridgeError, EngineBridge};
use ether_core::analysis::AnalysisFrame;
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{
    Base64Bytes, BuiltinDevice, BuiltinDeviceType, DeviceId, IrSource, MediaId, MediaRef, ParamId,
    PluginFormat, PluginInstance, TrackId,
};
use ether_core::{EngineOutputs, NodeKey, ParamChange, RenderGraphDesc, TransportControl};
use ether_media::DecodedAudio;

use crate::latency::{LatencyReport, LatencyTable};
use crate::proto::{
    EngineMsg, EngineReport, PREVIEW_MEDIA, REPORT_ANALYSIS, REPORT_ERROR, REPORT_LATENCY,
    REPORT_STATE, decode_analysis,
};
use crate::ring::{RingMemory, RingReader, RingWriter};

/// Bytes of reports drained per poll (reports are small; this only bounds a backlog).
const REPORT_BUDGET: usize = 256 * 1024;
/// Analysis frames kept between two `poll_analysis` calls (oldest dropped beyond).
const ANALYSIS_BACKLOG: usize = ether_core::analysis::ANALYSIS_RING;

/// `Decay`, `Size`, `Reverse` defaults of a convolution reverb (its descriptor's).
const REVERB_DEFAULTS: [f64; 3] = [100.0, 100.0, 0.0];
/// Their param ids, in that order.
const REVERB_SHAPING: [ParamId; 3] = {
    use ether_devices::fx_space::convolution_reverb as p;
    [p::DECAY, p::SIZE, p::REVERSE]
};

/// Shared between the bridge (owned by the controller) and the host wrapper, which flushes
/// the ring after every call and surfaces engine errors as notifications.
pub struct BridgeShared<M: RingMemory> {
    pub control: RingWriter<M>,
    pub reports: RingReader<M>,
    /// Errors reported by the Worklet (compile errors, unknown nodes, ...).
    pub errors: Vec<String>,
    /// Blocks rendered by the Worklet as of the last report (0 = not running yet).
    pub blocks: u64,
    /// Analysis frames received since the last `EngineBridge::poll_analysis` (virtual keys).
    pub analysis: Vec<AnalysisFrame>,
    /// Stream reports (cursors, misses) since the last poll.
    pub stream_reports: Vec<crate::media_stream::StreamReport>,
    /// Latest node latencies reported by the Worklet (virtual keys), read by
    /// `EngineBridge::node_latency`.
    pub latencies: LatencyTable,
    /// WebCLAP catalog and reports (`crate::webclap::bridge`).
    pub webclap: crate::webclap::bridge::WebclapShared,
}

impl<M: RingMemory> BridgeShared<M> {
    /// Drain reports into `out` (latest playhead, max-held meters, summed diagnostics).
    pub fn poll_reports(&mut self, out: &mut EngineOutputs) {
        let Self {
            reports,
            errors,
            blocks,
            analysis,
            stream_reports,
            latencies,
            webclap,
            ..
        } = self;
        reports.drain(REPORT_BUDGET, |bytes| {
            match bytes.first() {
                Some(&REPORT_STATE) => match EngineReport::decode(bytes) {
                    Ok(r) => {
                        out.playhead = Some(r.playhead);
                        for m in r.meters {
                            match out.meters.iter_mut().find(|e| e.track == m.track) {
                                Some(e) => {
                                    for ch in 0..2 {
                                        e.peak[ch] = e.peak[ch].max(m.peak[ch]);
                                        e.rms[ch] = e.rms[ch].max(m.rms[ch]);
                                    }
                                    e.clipped |= m.clipped;
                                }
                                None => out.meters.push(m),
                            }
                        }
                        out.event_overflow |= r.event_overflow;
                        out.underruns += r.underruns;
                        if r.preview_ended.is_some() {
                            out.preview_ended = r.preview_ended;
                        }
                        *blocks = r.blocks;
                    }
                    Err(e) => errors.push(format!("bad engine report: {e}")),
                },
                Some(&REPORT_ANALYSIS) => match decode_analysis(bytes) {
                    Ok(f) => {
                        if analysis.len() >= ANALYSIS_BACKLOG {
                            analysis.remove(0);
                        }
                        analysis.push(f);
                    }
                    Err(e) => errors.push(format!("bad analysis report: {e}")),
                },
                Some(&crate::media_stream::REPORT_STREAM) => {
                    match crate::media_stream::decode_report(bytes) {
                        Ok(r) => {
                            // Only the latest matters per stream.
                            stream_reports.retain(|o| r.iter().all(|n| n.media != o.media));
                            stream_reports.extend(r);
                        }
                        Err(e) => errors.push(format!("bad stream report: {e}")),
                    }
                }
                Some(&REPORT_LATENCY) => match LatencyReport::decode(bytes) {
                    Ok(r) => latencies.apply(&r),
                    Err(e) => errors.push(format!("bad latency report: {e}")),
                },
                Some(&REPORT_ERROR) => {
                    errors.push(String::from_utf8_lossy(&bytes[1..]).into_owned())
                }
                Some(&crate::webclap::proto::REPORT_PLUGIN) => {
                    match crate::webclap::proto::PluginReport::decode(bytes) {
                        Some(r) => webclap.reports.push(r),
                        None => errors.push("bad webclap report".into()),
                    }
                }
                _ => errors.push("unknown engine report".into()),
            }
            true
        });
        let skipped = reports.take_skipped();
        if skipped > 0 {
            errors.push(format!("report ring corrupt: skipped {skipped} bytes"));
        }
    }
}

pub type Shared<M> = Rc<RefCell<BridgeShared<M>>>;

/// A live convolution reverb, as the Worker last sent it.
struct Reverb {
    /// `Decay`, `Size` and `Reverse`, so its IR is prepared for the shaping it plays with.
    shaping: [f64; 3],
    /// Its media IR, not delivered yet: waiting for the media's audio (asked of the
    /// controller), or for the media to load at all.
    waiting: Option<MediaId>,
}

/// How a reverb's new IR reaches the Worklet.
enum IrRoute {
    /// Prepared here: the frames that stream it.
    Prepared(Vec<Vec<u8>>),
    /// A media IR whose audio was asked of the controller: it follows prepared.
    Waiting,
    /// Nothing to build (a silent or empty media IR): the reverb plays with no IR.
    Empty,
    /// In the device message, as is: no IR, an unknown factory id, or media the Worklet
    /// doesn't have (it reads no IR from those either: nothing is built there).
    Message,
}

pub fn shared<M: RingMemory>(control: M, reports: M) -> Shared<M> {
    Rc::new(RefCell::new(BridgeShared {
        control: RingWriter::new(control),
        reports: RingReader::new(reports),
        errors: Vec::new(),
        blocks: 0,
        analysis: Vec::new(),
        stream_reports: Vec::new(),
        latencies: LatencyTable::default(),
        webclap: Default::default(),
    }))
}

/// `EngineBridge` over the SharedArrayBuffer rings. Node keys are virtual (see
/// [`EngineMsg`]); the Worklet maps them. Plugins: WebCLAP only (`crate::webclap`).
pub struct WebBridge<M: RingMemory> {
    shared: Shared<M>,
    next_node: u32,
    devices: BTreeMap<DeviceId, (NodeKey, BuiltinDeviceType)>,
    /// Long media streamed from OPFS ([`crate::media_stream`]).
    streams: crate::media_stream::WebStreams,
    /// Last known playhead beat (where `Play` starts).
    playhead_beat: f64,
    /// The Worklet's engine rate (factory IRs are prepared at it).
    sample_rate: u32,
    /// The live convolution reverbs.
    reverbs: BTreeMap<NodeKey, Reverb>,
    /// Media the Worklet has (loaded or streamed): IRs from those are prepared here.
    worklet_media: BTreeSet<MediaId>,
    /// Audio of media IRs, kept only while a reverb waits for it.
    ir_audio: BTreeMap<MediaId, Arc<DecodedAudio>>,
    /// Media whose audio is to be asked of the controller.
    ir_requests: Vec<MediaId>,
    /// Live WebCLAP devices and requests to their instances.
    webclap: crate::webclap::bridge::WebclapBridge,
}

impl<M: RingMemory> WebBridge<M> {
    pub fn new(shared: Shared<M>) -> Self {
        Self {
            shared,
            next_node: 0,
            devices: BTreeMap::new(),
            streams: Default::default(),
            playhead_beat: 0.0,
            sample_rate: 48_000,
            reverbs: BTreeMap::new(),
            worklet_media: BTreeSet::new(),
            ir_audio: BTreeMap::new(),
            ir_requests: Vec::new(),
            webclap: Default::default(),
        }
    }

    /// How the Worker sleeps while a WebCLAP request waits for the Worklet
    /// (`crate::webclap::bridge`).
    pub fn set_webclap_waiter(&mut self, waiter: Box<dyn FnMut(u32)>) {
        self.webclap.set_waiter(waiter);
    }

    fn new_key(&mut self) -> NodeKey {
        self.next_node = self.next_node.wrapping_add(1);
        NodeKey {
            index: self.next_node,
            generation: 1,
        }
    }

    /// Run a WebCLAP request: messages are sent (and flushed) by `send`, replies come
    /// through the report ring.
    fn webclap_call<R>(
        &mut self,
        f: impl FnOnce(
            &mut crate::webclap::bridge::WebclapBridge,
            &mut dyn FnMut(crate::webclap::proto::WebclapMsg),
            &mut dyn FnMut(&mut crate::webclap::bridge::WebclapBridge) -> u64,
        ) -> R,
    ) -> R {
        let shared = self.shared.clone();
        let mut send = |msg: crate::webclap::proto::WebclapMsg| {
            let mut s = shared.borrow_mut();
            for frame in EngineMsg::Webclap(msg).encode() {
                s.control.send(&frame);
            }
            s.control.flush();
        };
        let mut scratch = EngineOutputs::default();
        let shared2 = self.shared.clone();
        let mut pump = |wb: &mut crate::webclap::bridge::WebclapBridge| {
            let mut s = shared2.borrow_mut();
            s.control.flush();
            s.poll_reports(&mut scratch);
            wb.take_reports(&mut s.webclap);
            s.blocks
        };
        f(&mut self.webclap, &mut send, &mut pump)
    }

    /// The Worklet's engine rate (default 48 kHz).
    pub fn set_sample_rate(&mut self, rate: u32) {
        self.sample_rate = rate;
    }

    /// How the IR `ir` of the live reverb `key` gets to the Worklet. Factory
    /// IRs and media IRs whose audio is here are prepared here (read, normalized and
    /// FFT-partitioned: the expensive part) as the frames that stream them; the Worklet
    /// assembles them in bounded steps and swaps them in (crossfade). The audio of a media
    /// IR the Worklet has but this Worker no longer does is asked of the controller
    /// (decoded again: [`EngineBridge::take_ir_media_requests`]).
    fn route_ir(&mut self, key: NodeKey, ir: Option<&IrSource>) -> IrRoute {
        use ether_devices::fx_space::{FACTORY_IRS, IrInput};
        let mut waiting = None;
        let route = match ir {
            Some(IrSource::Factory { id }) => match FACTORY_IRS.iter().position(|s| s.id == id) {
                Some(index) => self.prepare_ir(key, ir, IrInput::Factory(index)),
                None => IrRoute::Message,
            },
            Some(IrSource::Media { media }) if !self.worklet_media.contains(media) => {
                // Prepared once loaded (the controller re-creates the reverb then).
                waiting = Some(*media);
                IrRoute::Message
            }
            Some(IrSource::Media { media }) => match self.ir_audio.get(media) {
                Some(audio) => {
                    let source = Arc::new(ether_media::InMemorySource::new(audio.clone()));
                    self.prepare_ir(key, ir, IrInput::Media(source))
                }
                None => {
                    waiting = Some(*media);
                    if !self.ir_requests.contains(media) {
                        self.ir_requests.push(*media);
                    }
                    IrRoute::Waiting
                }
            },
            None => IrRoute::Message,
        };
        if let Some(r) = self.reverbs.get_mut(&key) {
            r.waiting = waiting;
        }
        self.release_ir_audio();
        route
    }

    fn prepare_ir(
        &self,
        key: NodeKey,
        source: Option<&IrSource>,
        input: ether_devices::fx_space::IrInput,
    ) -> IrRoute {
        use ether_devices::fx_space::{PreparedIr, Shaping};
        let Some(source) = source else {
            return IrRoute::Message;
        };
        let [decay, size, reverse] = self
            .reverbs
            .get(&key)
            .map_or(REVERB_DEFAULTS, |r| r.shaping);
        let shaping = Shaping::from_params(decay, size, reverse);
        match PreparedIr::new(&input, self.sample_rate as f32, shaping) {
            Some(prepared) => IrRoute::Prepared(crate::proto::encode_prepared_ir(
                key,
                source,
                self.sample_rate,
                &prepared,
            )),
            None => IrRoute::Empty,
        }
    }

    /// Drop the media IR audio no reverb waits for any more.
    fn release_ir_audio(&mut self) {
        let reverbs = &self.reverbs;
        self.ir_audio
            .retain(|m, _| reverbs.values().any(|r| r.waiting == Some(*m)));
    }

    /// Deliver the IRs waiting for `media`, whose audio just arrived.
    fn deliver_waiting(&mut self, media: MediaId) {
        let keys: Vec<NodeKey> = self
            .reverbs
            .iter()
            .filter(|(_, r)| r.waiting == Some(media))
            .map(|(k, _)| *k)
            .collect();
        let source = IrSource::Media { media };
        for key in keys {
            match self.route_ir(key, Some(&source)) {
                IrRoute::Prepared(frames) => self.send_frames(frames),
                IrRoute::Empty => self.send(EngineMsg::UpdateBuiltin {
                    key,
                    device: BuiltinDevice::ConvolutionReverb { ir: None },
                }),
                IrRoute::Waiting | IrRoute::Message => {}
            }
        }
    }

    /// Media IR audio kept (tests).
    pub fn ir_audio_kept(&self) -> usize {
        self.ir_audio.len()
    }

    /// How the Worker opens media files for streaming (the host's
    /// OPFS, by byte ranges). Without one, `stream_media` declines (whole-file loads).
    pub fn set_stream_opener(&mut self, opener: crate::media_stream::Opener) {
        self.streams.set_opener(opener);
    }

    /// Is `media` streamed (tests/diagnostics)?
    pub fn is_streamed(&self, media: MediaId) -> bool {
        self.streams.contains(media)
    }

    fn send_frames(&mut self, frames: Vec<Vec<u8>>) {
        let mut shared = self.shared.borrow_mut();
        for frame in frames {
            shared.control.send(&frame);
        }
    }

    /// Stream work outside a borrow of `self.streams`: frames are queued then sent.
    fn with_streams(
        &mut self,
        f: impl FnOnce(&mut crate::media_stream::WebStreams, &mut dyn FnMut(Vec<Vec<u8>>)),
    ) {
        let mut queued: Vec<Vec<u8>> = Vec::new();
        f(&mut self.streams, &mut |frames| queued.extend(frames));
        self.send_frames(queued);
    }

    fn send(&mut self, msg: EngineMsg) {
        let mut shared = self.shared.borrow_mut();
        for frame in msg.encode() {
            shared.control.send(&frame);
        }
    }
}

impl<M: RingMemory> EngineBridge for WebBridge<M> {
    fn create_builtin(
        &mut self,
        device: DeviceId,
        kind: &BuiltinDevice,
        params: &[(ParamId, f64)],
    ) -> Result<NodeKey, BridgeError> {
        let key = self.new_key();
        self.devices.insert(device, (key, kind.device_type()));
        self.shared.borrow_mut().latencies.track(key);
        if let BuiltinDevice::ConvolutionReverb { ir } = kind {
            // Created without its IR, which follows prepared (faded in).
            let mut shaping = REVERB_DEFAULTS;
            for &(id, v) in params {
                if let Some(i) = REVERB_SHAPING.iter().position(|&p| p == id) {
                    shaping[i] = v;
                }
            }
            self.reverbs.insert(
                key,
                Reverb {
                    shaping,
                    waiting: None,
                },
            );
            let route = self.route_ir(key, ir.as_ref());
            if !matches!(route, IrRoute::Message) {
                self.send(EngineMsg::CreateBuiltin {
                    key,
                    device: BuiltinDevice::ConvolutionReverb { ir: None },
                    params: params.to_vec(),
                });
                if let IrRoute::Prepared(frames) = route {
                    self.send_frames(frames);
                }
                return Ok(key);
            }
        }
        self.send(EngineMsg::CreateBuiltin {
            key,
            device: kind.clone(),
            params: params.to_vec(),
        });
        Ok(key)
    }

    /// WebCLAP plugins only: the others (native formats) run in the desktop app.
    fn create_plugin(
        &mut self,
        device: DeviceId,
        plugin: &PluginInstance,
        state: Option<&Base64Bytes>,
    ) -> Result<NodeKey, BridgeError> {
        if plugin.format != PluginFormat::Webclap {
            return Err(BridgeError::Unsupported(format!(
                "{} plugins are not available in the browser (desktop app only)",
                plugin.format.as_str()
            )));
        }
        let key = self.new_key();
        let msg = {
            let shared = self.shared.borrow();
            self.webclap
                .create(&shared.webclap, key, device, plugin, state)?
        };
        self.shared.borrow_mut().latencies.track(key);
        self.send(EngineMsg::Webclap(msg));
        Ok(key)
    }

    fn set_always_process(&mut self, device: DeviceId, on: bool) -> Result<(), BridgeError> {
        if let Some(key) = self.webclap.key(device) {
            self.send(EngineMsg::Webclap(
                crate::webclap::proto::WebclapMsg::AlwaysProcess { key, on },
            ));
        }
        Ok(())
    }

    fn create_offline_plugin(
        &mut self,
        device: DeviceId,
        plugin: &PluginInstance,
        state: Option<&Base64Bytes>,
        sample_rate: u32,
    ) -> Result<Box<dyn ether_core::Node>, BridgeError> {
        if plugin.format != PluginFormat::Webclap {
            return Err(BridgeError::Unsupported(format!(
                "{} plugins are not available in the browser (desktop app only)",
                plugin.format.as_str()
            )));
        }
        let shared = self.shared.borrow();
        self.webclap
            .create_offline(&shared.webclap, device, plugin, state, sample_rate)
    }

    fn poll_plugins(&mut self, out: &mut Vec<(DeviceId, ether_core::plugin::PluginNotification)>) {
        {
            let mut shared = self.shared.borrow_mut();
            self.webclap.take_reports(&mut shared.webclap);
        }
        self.webclap.poll_plugins(out);
    }

    fn plugin_state(&mut self, device: DeviceId) -> Result<Option<Base64Bytes>, BridgeError> {
        self.webclap_call(|wb, send, pump| wb.plugin_state(device, send, pump))
            .unwrap_or(Ok(None))
    }

    fn plugin_param_values(&mut self, device: DeviceId) -> Vec<(ParamId, f64)> {
        if self.webclap.key(device).is_none() {
            return Vec::new();
        }
        self.webclap_call(|wb, send, pump| wb.param_values(device, send, pump))
    }

    fn probe_plugin(&mut self, plugin: &PluginInstance) -> Option<DeviceDescriptor> {
        crate::webclap::bridge::WebclapBridge::probe(&self.shared.borrow().webclap, plugin)
    }

    fn plugin_catalog(&self) -> Option<ether_controller::PluginCatalogView> {
        Some(crate::webclap::bridge::WebclapBridge::catalog_view(
            &self.shared.borrow().webclap,
        ))
    }

    fn destroy_node(&mut self, key: NodeKey) -> Result<(), BridgeError> {
        self.devices.retain(|_, (k, _)| *k != key);
        self.webclap.forget_key(key);
        if self.reverbs.remove(&key).is_some() {
            self.release_ir_audio();
        }
        self.shared.borrow_mut().latencies.forget(key);
        self.send(EngineMsg::DestroyNode { key });
        Ok(())
    }

    fn set_node_scale(
        &mut self,
        device: DeviceId,
        scale: ether_core::protocol::model::MusicalScale,
    ) -> Result<bool, BridgeError> {
        let Some(&(key, _)) = self.devices.get(&device) else {
            return Ok(false);
        };
        self.send(EngineMsg::NodeScale { key, scale });
        Ok(true)
    }

    /// Data-only changes of a live built-in (sampler slices, multisampler zones,
    /// convolution reverb IR) go to the Worklet as `UpdateBuiltin`: the node takes them in
    /// place (`Node::set_data`), so they don't click. Anything else: re-create.
    fn update_builtin(
        &mut self,
        device: DeviceId,
        kind: &BuiltinDevice,
    ) -> Result<bool, BridgeError> {
        let Some(&(key, ty)) = self.devices.get(&device) else {
            return Ok(false);
        };
        let updatable = matches!(
            kind,
            BuiltinDevice::Sampler { .. }
                | BuiltinDevice::MultiSampler { .. }
                | BuiltinDevice::ConvolutionReverb { .. }
                | BuiltinDevice::VolumeLfo { .. }
        );
        if !updatable || ty != kind.device_type() {
            return Ok(false);
        }
        if let BuiltinDevice::ConvolutionReverb { ir } = kind {
            match self.route_ir(key, ir.as_ref()) {
                IrRoute::Prepared(frames) => {
                    self.send_frames(frames);
                    return Ok(true);
                }
                // The current IR plays until the new one is swapped in.
                IrRoute::Waiting => return Ok(true),
                IrRoute::Empty => {
                    self.send(EngineMsg::UpdateBuiltin {
                        key,
                        device: BuiltinDevice::ConvolutionReverb { ir: None },
                    });
                    return Ok(true);
                }
                IrRoute::Message => {}
            }
        }
        self.send(EngineMsg::UpdateBuiltin {
            key,
            device: kind.clone(),
        });
        Ok(true)
    }

    fn load_media(
        &mut self,
        media: &MediaRef,
        audio: Arc<DecodedAudio>,
    ) -> Result<(), BridgeError> {
        self.worklet_media.insert(media.id);
        // Kept for the reverbs waiting for it (the controller re-creates them now).
        if self.reverbs.values().any(|r| r.waiting == Some(media.id)) {
            self.ir_audio.insert(media.id, audio.clone());
        }
        self.send(EngineMsg::LoadMedia {
            media: media.id,
            audio,
        });
        Ok(())
    }

    fn unload_media(&mut self, media: MediaId) -> Result<(), BridgeError> {
        self.worklet_media.remove(&media);
        self.ir_audio.remove(&media);
        self.streams.remove(media);
        self.send(EngineMsg::UnloadMedia { media });
        Ok(())
    }

    fn publish(&mut self, graph: RenderGraphDesc) -> Result<(), BridgeError> {
        self.streams.observe_graph(&graph);
        self.send(EngineMsg::Publish {
            graph: Box::new(graph),
        });
        Ok(())
    }

    fn set_param(&mut self, change: ParamChange) -> Result<(), BridgeError> {
        if let ether_core::ParamTarget::Node { node, param } = change.target
            && let Some(r) = self.reverbs.get_mut(&node)
            && let Some(i) = REVERB_SHAPING.iter().position(|&p| p == param)
        {
            r.shaping[i] = change.value;
        }
        self.send(EngineMsg::SetParam { change });
        Ok(())
    }

    fn transport(&mut self, control: TransportControl) -> Result<(), BridgeError> {
        // The jump target's chunks go first (the ring is FIFO).
        if !self.streams.is_empty() {
            let now = crate::media_stream::now_ms();
            match &control {
                TransportControl::Locate { position } => {
                    let beat = position.0;
                    self.with_streams(|s, send| s.prime_at(beat, now, send));
                }
                TransportControl::Play => {
                    let beat = self.playhead_beat;
                    self.with_streams(|s, send| s.prime_at(beat, now, send));
                }
                TransportControl::SetLoop { enabled, region } => {
                    self.streams
                        .observe_loop(*enabled, region.start.0, region.end.0);
                }
                _ => {}
            }
        }
        if let TransportControl::Locate { position } = &control {
            self.playhead_beat = position.0;
        }
        self.send(EngineMsg::Transport { control });
        Ok(())
    }

    /// In order with everything sent before (the control ring is FIFO and never drops).
    fn begin_render_preview(
        &mut self,
        spec: ether_core::render_preview::PreviewSpec,
    ) -> Result<(), BridgeError> {
        self.send(EngineMsg::RenderPreviewBegin { spec });
        Ok(())
    }

    fn push_render_preview(&mut self, left: &[f32], right: &[f32]) {
        // 16-bit on the wire (what the preview holds anyway): half the bytes.
        use ether_core::render_preview::to_i16;
        let samples = left
            .iter()
            .zip(right)
            .flat_map(|(&l, &r)| [to_i16(l), to_i16(r)])
            .collect();
        self.send(EngineMsg::RenderPreviewChunk { samples });
    }

    fn end_render_preview(&mut self) {
        self.send(EngineMsg::RenderPreviewEnd);
    }

    fn set_suspended(&mut self, suspended: bool) -> Result<(), BridgeError> {
        self.send(EngineMsg::Suspend { suspended });
        Ok(())
    }

    /// `media-preview`: the audio goes through the ordinary media chunk path under
    /// [`PREVIEW_MEDIA`], then one `Preview` message plays it (`None` stops).
    fn preview(
        &mut self,
        id: u64,
        audio: Option<Arc<DecodedAudio>>,
        gain: f32,
    ) -> Result<(), BridgeError> {
        let media = audio.map(|audio| {
            self.send(EngineMsg::LoadMedia {
                media: PREVIEW_MEDIA,
                audio,
            });
            PREVIEW_MEDIA
        });
        self.send(EngineMsg::Preview { id, media, gain });
        Ok(())
    }

    fn poll(&mut self, out: &mut EngineOutputs) {
        out.clear();
        let reports = {
            let mut shared = self.shared.borrow_mut();
            shared.control.flush();
            shared.poll_reports(out);
            std::mem::take(&mut shared.stream_reports)
        };
        if let Some(p) = &out.playhead {
            self.playhead_beat = p.position.0;
        }
        // Follow the worklet's cursors, keep the read-ahead filled.
        if !self.streams.is_empty() {
            let now = crate::media_stream::now_ms();
            for r in reports {
                self.streams.report(r.media, r.cursors, r.missed, now);
            }
            let pending = self.shared.borrow().control.pending_bytes();
            if pending < crate::media_stream::MAX_PENDING_BYTES {
                self.with_streams(|s, send| s.pump(now, send));
                self.shared.borrow_mut().control.flush();
            }
        }
    }

    /// Audio streaming (CONTRACTS.md §13.1): stream long media from OPFS through the
    /// worklet's chunk cache ([`crate::media_stream`]).
    fn stream_media(
        &mut self,
        source: &ether_controller::media_stream::StreamSource,
    ) -> Result<bool, BridgeError> {
        if !self.streams.can_stream() {
            return Ok(false);
        }
        let id = source.media.id;
        if self.streams.remove(id) {
            self.send(EngineMsg::UnloadMedia { media: id });
        }
        let now = crate::media_stream::now_ms();
        let rate = source.engine_sample_rate;
        let mut queued: Vec<Vec<u8>> = Vec::new();
        let res = self
            .streams
            .open(source, rate, now, &mut |frames| queued.extend(frames));
        self.send_frames(queued);
        res.map_err(|e| BridgeError::Other(e.to_string()))?;
        self.worklet_media.insert(id);
        Ok(true)
    }

    fn take_ir_media_requests(&mut self) -> Vec<MediaId> {
        std::mem::take(&mut self.ir_requests)
    }

    /// Prepares the IRs waiting for `media` from its audio, then drops it.
    fn ir_media(&mut self, media: MediaId, audio: Arc<DecodedAudio>) -> Result<(), BridgeError> {
        if !self.worklet_media.contains(&media) || audio.sample_rate != self.sample_rate {
            return Ok(());
        }
        self.ir_audio.insert(media, audio);
        self.deliver_waiting(media);
        self.release_ir_audio();
        Ok(())
    }

    fn descriptor(&mut self, device: DeviceId) -> Option<DeviceDescriptor> {
        if let Some((_, kind)) = self.devices.get(&device) {
            return Some(ether_devices::descriptor(*kind));
        }
        self.webclap
            .descriptor(&self.shared.borrow().webclap, device)
    }

    /// Frames forwarded by the Worklet (`REPORT_ANALYSIS`), drained with the other reports
    /// by [`EngineBridge::poll`], which the controller's tick calls first.
    fn poll_analysis(&mut self, out: &mut Vec<AnalysisFrame>) {
        out.append(&mut self.shared.borrow_mut().analysis);
    }

    fn watch_analysis(&mut self, node: NodeKey, on: bool) -> Result<(), BridgeError> {
        self.send(EngineMsg::WatchAnalysis { key: node, on });
        Ok(())
    }

    /// The latest latency the Worklet reported for `key` (drained by
    /// [`EngineBridge::poll`]); `None` while it still has the latency it was created with,
    /// which the Worklet's engine already compiles PDC with.
    fn node_latency(&self, key: NodeKey) -> Option<u32> {
        self.shared.borrow().latencies.get(key)
    }

    /// `keyboard-midi`: a binary frame the Worklet pushes into the engine's live MIDI ring.
    fn live_midi(&mut self, data: [u8; 3]) -> Result<(), BridgeError> {
        self.send(EngineMsg::LiveMidi { data });
        Ok(())
    }

    fn preview_note(&mut self, track: TrackId, data: [u8; 3]) -> Result<(), BridgeError> {
        self.send(EngineMsg::PreviewNote { track, data });
        Ok(())
    }
}

#[cfg(test)]
mod preview_tests {
    //! `media-preview` over the rings: audio shipped under `PREVIEW_MEDIA`, played by one
    //! `Preview` message, natural end reported back by id; stop fades to silence.
    use super::*;
    use crate::ring::HeapMemory;
    use crate::worklet::{EngineHost, RENDER_QUANTUM};

    fn setup() -> (WebBridge<HeapMemory>, EngineHost<HeapMemory>) {
        let control = HeapMemory::new(1 << 20);
        let reports = HeapMemory::new(1 << 14);
        let bridge = WebBridge::new(shared(control.clone(), reports.clone()));
        (bridge, EngineHost::new(48_000, control, reports))
    }

    fn dc(level: f32, frames: usize) -> Arc<DecodedAudio> {
        Arc::new(DecodedAudio {
            sample_rate: 48_000,
            channels: vec![vec![level; frames]],
        })
    }

    /// Render `blocks` quanta; returns the peak of the left output.
    fn render(host: &mut EngineHost<HeapMemory>, blocks: usize) -> f32 {
        (0..blocks)
            .map(|_| {
                host.render(RENDER_QUANTUM);
                host.output(0).iter().fold(0.0f32, |m, s| m.max(s.abs()))
            })
            .fold(0.0, f32::max)
    }

    #[test]
    fn preview_plays_through_the_worklet_and_reports_its_end() {
        let (mut bridge, mut host) = setup();
        let mut out = EngineOutputs::default();
        bridge.preview(1, Some(dc(0.5, 4_000)), 1.0).unwrap();
        bridge.poll(&mut out);
        assert!(render(&mut host, 10) > 0.49, "the preview is audible");
        bridge.poll(&mut out);
        assert_eq!(out.preview_ended, None, "still playing");
        render(&mut host, 40);
        bridge.poll(&mut out);
        assert_eq!(out.preview_ended, Some(1));
        bridge.poll(&mut out);
        assert_eq!(out.preview_ended, None, "reported once");
        assert!(host.output(0).iter().all(|&s| s == 0.0));

        // Stop fades a long preview out; it is never reported.
        bridge.preview(2, Some(dc(0.5, 48_000)), 1.0).unwrap();
        bridge.poll(&mut out);
        assert!(render(&mut host, 10) > 0.49);
        bridge.preview(2, None, 0.0).unwrap();
        bridge.poll(&mut out);
        render(&mut host, 3);
        assert_eq!(render(&mut host, 10), 0.0, "silent after the fade");
        bridge.poll(&mut out);
        assert_eq!(out.preview_ended, None);
        assert!(bridge.shared.borrow().errors.is_empty());
    }
}

#[cfg(test)]
mod analysis_tests {
    //! `fx-analysis`: watches reach the Worklet's engine and its frames come back to the
    //! controller under the Worker's virtual node keys.
    use super::*;
    use crate::ring::HeapMemory;
    use crate::worklet::{EngineHost, RENDER_QUANTUM};
    use ether_core::analysis::AnalysisKind;
    use ether_core::graph::{ChainEntry, TrackDesc};
    use ether_core::protocol::model::{TrackId, TrackKind, Ulid};

    fn track(id: u128, kind: TrackKind, output: Option<TrackId>) -> TrackDesc {
        TrackDesc {
            modulation: Default::default(),
            vca: Default::default(),
            chain_racks: Default::default(),
            frozen: Default::default(),
            input_tap: Default::default(),
            id: TrackId(Ulid(id)),
            kind,
            chain: vec![],
            output,
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
            racks: Vec::new(),
            expression: Default::default(),
            hw_io: Vec::new(),
            sidechain_taps: Vec::new(),
        }
    }

    fn render(host: &mut EngineHost<HeapMemory>, blocks: usize) {
        for _ in 0..blocks {
            host.render(RENDER_QUANTUM);
        }
    }

    #[test]
    fn analysis_frames_are_forwarded_from_the_worklet() {
        let control = HeapMemory::new(1 << 20);
        let reports = HeapMemory::new(1 << 18);
        let mut bridge = WebBridge::new(shared(control.clone(), reports.clone()));
        let mut host = EngineHost::new(48_000, control, reports);
        let device = DeviceId(Ulid(9));
        let key = bridge
            .create_builtin(
                device,
                &BuiltinDevice::new(BuiltinDeviceType::SpectrumAnalyzer),
                &[],
            )
            .unwrap();
        let master = track(1, TrackKind::Master, None);
        let mut audio = track(2, TrackKind::Audio, Some(master.id));
        audio.chain = vec![ChainEntry {
            node: key,
            enabled: true,
            sidechain: None,
        }];
        bridge
            .publish(RenderGraphDesc {
                tracks: vec![master, audio],
                ..Default::default()
            })
            .unwrap();
        let mut outputs = EngineOutputs::default();
        let mut frames = Vec::new();
        bridge.poll(&mut outputs);
        render(&mut host, 8);
        // Unwatched: nothing comes back.
        bridge.poll(&mut outputs);
        bridge.poll_analysis(&mut frames);
        assert!(frames.is_empty());
        bridge.watch_analysis(key, true).unwrap();
        bridge.poll(&mut outputs);
        // Half a second.
        for _ in 0..30 {
            render(&mut host, 6);
            bridge.poll(&mut outputs);
            bridge.poll_analysis(&mut frames);
        }
        assert!(
            (12..=16).contains(&frames.len()),
            "{} frames in 0.5 s",
            frames.len()
        );
        assert!(frames.iter().all(|f| f.node == key));
        assert!(frames.iter().all(|f| f.kind == AnalysisKind::Spectrum));
        assert_eq!(frames[0].values().len(), 258);
        assert!(bridge.shared.borrow().errors.is_empty());
        // Unwatch stops them.
        bridge.watch_analysis(key, false).unwrap();
        bridge.poll(&mut outputs);
        render(&mut host, 60);
        bridge.poll(&mut outputs);
        frames.clear();
        bridge.poll_analysis(&mut frames);
        assert!(frames.len() <= 1, "{}", frames.len());
    }
}

#[cfg(test)]
mod update_tests {
    //! v0.3 (`fx-space`): data-only built-in changes (convolution IR, sampler slices,
    //! multisampler zones) reach the Worklet's live node as `UpdateBuiltin` instead of a
    //! re-create; other changes are left to the controller (re-create).
    use super::*;
    use crate::proto::Frame;
    use crate::ring::HeapMemory;
    use crate::worklet::{EngineHost, RENDER_QUANTUM};
    use ether_core::protocol::model::{IrSource, SampleZone, SliceSettings, Ulid};

    fn reverb(id: Option<&str>) -> BuiltinDevice {
        BuiltinDevice::ConvolutionReverb {
            ir: id.map(|id| IrSource::Factory { id: id.into() }),
        }
    }

    #[test]
    fn update_builtin_round_trips_through_the_ring() {
        let msg = EngineMsg::UpdateBuiltin {
            key: NodeKey {
                index: 3,
                generation: 1,
            },
            device: reverb(Some("hall")),
        };
        let frames = msg.encode();
        assert_eq!(frames.len(), 1);
        match Frame::decode(&frames[0]).unwrap() {
            Frame::Msg(decoded) => assert_eq!(decoded, msg),
            _ => panic!("expected a message frame"),
        }
    }

    #[test]
    fn data_only_changes_update_the_live_node() {
        let control = HeapMemory::new(1 << 20);
        let reports = HeapMemory::new(1 << 16);
        let mut bridge = WebBridge::new(shared(control.clone(), reports.clone()));
        let mut host = EngineHost::new(48_000, control, reports);
        let mut out = EngineOutputs::default();
        let d = DeviceId(Ulid(1));
        bridge
            .create_builtin(d, &reverb(Some("room")), &[])
            .unwrap();
        assert_eq!(bridge.update_builtin(d, &reverb(Some("hall"))), Ok(true));
        assert_eq!(bridge.update_builtin(d, &reverb(None)), Ok(true));
        let s = DeviceId(Ulid(2));
        let sampler = |markers: usize| BuiltinDevice::Sampler {
            sample: None,
            slices: SliceSettings {
                enabled: true,
                base_note: 36,
                markers: vec![ether_core::protocol::model::Seconds(0.0); markers],
            },
        };
        bridge.create_builtin(s, &sampler(1), &[]).unwrap();
        assert_eq!(bridge.update_builtin(s, &sampler(2)), Ok(true));
        let m = DeviceId(Ulid(3));
        let ms = |n: usize| BuiltinDevice::MultiSampler {
            zones: vec![SampleZone::default(); n],
        };
        bridge.create_builtin(m, &ms(1), &[]).unwrap();
        assert_eq!(bridge.update_builtin(m, &ms(2)), Ok(true));
        // Another type's kind, devices without in-place updates, unknown devices: refused.
        assert_eq!(bridge.update_builtin(s, &reverb(Some("hall"))), Ok(false));
        let c = DeviceId(Ulid(4));
        bridge
            .create_builtin(c, &BuiltinDevice::Compressor, &[])
            .unwrap();
        assert_eq!(
            bridge.update_builtin(c, &BuiltinDevice::Compressor),
            Ok(false)
        );
        assert_eq!(
            bridge.update_builtin(DeviceId(Ulid(9)), &reverb(None)),
            Ok(false)
        );
        for _ in 0..8 {
            host.render(RENDER_QUANTUM);
        }
        bridge.poll(&mut out);
        let errors = bridge.shared.borrow().errors.clone();
        assert!(errors.is_empty(), "{errors:?}");
    }
}

#[cfg(test)]
mod live_midi_tests {
    //! `keyboard-midi`: live MIDI from the controller reaches the Worklet's engine (the
    //! instrument of a monitored MIDI track plays it) and stops on note-off.
    use super::*;
    use crate::proto::Frame;
    use crate::ring::HeapMemory;
    use crate::worklet::{EngineHost, RENDER_QUANTUM};
    use ether_core::graph::{ChainEntry, TrackDesc};
    use ether_core::protocol::model::{TrackId, TrackKind, Ulid};

    fn track(id: u128, kind: TrackKind, output: Option<TrackId>) -> TrackDesc {
        TrackDesc {
            modulation: Default::default(),
            vca: Default::default(),
            chain_racks: Default::default(),
            frozen: Default::default(),
            input_tap: Default::default(),
            id: TrackId(Ulid(id)),
            kind,
            chain: vec![],
            output,
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
            racks: Vec::new(),
            expression: Default::default(),
            hw_io: Vec::new(),
            sidechain_taps: Vec::new(),
        }
    }

    #[test]
    fn live_midi_round_trips_as_a_binary_frame() {
        let msg = EngineMsg::LiveMidi {
            data: [0x90, 60, 100],
        };
        let frames = msg.encode();
        assert_eq!(frames, vec![vec![b'M', 0x90, 60, 100]]);
        match Frame::decode(&frames[0]).unwrap() {
            Frame::Msg(decoded) => assert_eq!(decoded, msg),
            _ => panic!("expected a message frame"),
        }
        assert!(Frame::decode(&[b'M', 0x90]).is_err());
    }

    #[test]
    fn preview_note_round_trips_as_a_binary_frame() {
        let msg = EngineMsg::PreviewNote {
            track: TrackId(ether_core::protocol::model::Ulid(0x0102_0304)),
            data: [0x90, 60, 100],
        };
        let frames = msg.encode();
        assert_eq!(frames[0].len(), 20);
        match Frame::decode(&frames[0]).unwrap() {
            Frame::Msg(decoded) => assert_eq!(decoded, msg),
            _ => panic!("expected a message frame"),
        }
        assert!(Frame::decode(&frames[0][..19]).is_err());
    }

    /// Peak of `track` over `blocks` render quanta.
    fn peak(
        bridge: &mut WebBridge<HeapMemory>,
        host: &mut EngineHost<HeapMemory>,
        track: TrackId,
        blocks: usize,
    ) -> f32 {
        let mut max = 0.0f32;
        let mut outputs = EngineOutputs::default();
        for _ in 0..blocks {
            host.render(RENDER_QUANTUM);
            bridge.poll(&mut outputs);
            for m in outputs.meters.iter().filter(|m| m.track == track) {
                max = max.max(m.peak[0]).max(m.peak[1]);
            }
        }
        max
    }

    fn setup(monitor: bool) -> (WebBridge<HeapMemory>, EngineHost<HeapMemory>, TrackId) {
        let control = HeapMemory::new(1 << 20);
        let reports = HeapMemory::new(1 << 18);
        let mut bridge = WebBridge::new(shared(control.clone(), reports.clone()));
        let host = EngineHost::new(48_000, control, reports);
        let key = bridge
            .create_builtin(DeviceId(Ulid(9)), &BuiltinDevice::PolySynth, &[])
            .unwrap();
        let master = track(1, TrackKind::Master, None);
        let mut midi = track(2, TrackKind::Midi, Some(master.id));
        midi.monitor = monitor;
        midi.armed = monitor;
        midi.chain = vec![ChainEntry {
            node: key,
            enabled: true,
            sidechain: None,
        }];
        let id = midi.id;
        bridge
            .publish(RenderGraphDesc {
                tracks: vec![master, midi],
                ..Default::default()
            })
            .unwrap();
        (bridge, host, id)
    }

    #[test]
    fn live_notes_play_the_monitored_tracks_instrument() {
        let (mut bridge, mut host, midi) = setup(true);
        assert_eq!(peak(&mut bridge, &mut host, midi, 8), 0.0);
        bridge.live_midi([0x90, 60, 100]).unwrap();
        assert!(peak(&mut bridge, &mut host, midi, 40) > 0.01);
        bridge.live_midi([0x80, 60, 0]).unwrap();
        // Past the release: silent again (no stuck note).
        peak(&mut bridge, &mut host, midi, 1500);
        assert!(peak(&mut bridge, &mut host, midi, 20) < 1e-4);
        let errors = bridge.shared.borrow().errors.clone();
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn unmonitored_tracks_ignore_live_notes() {
        let (mut bridge, mut host, midi) = setup(false);
        bridge.live_midi([0x90, 60, 100]).unwrap();
        assert_eq!(peak(&mut bridge, &mut host, midi, 40), 0.0);
    }

    /// A preview note plays the track's instrument even unmonitored, then stops.
    #[test]
    fn preview_notes_play_an_unmonitored_tracks_instrument() {
        let (mut bridge, mut host, midi) = setup(false);
        bridge.preview_note(midi, [0x90, 60, 100]).unwrap();
        assert!(peak(&mut bridge, &mut host, midi, 40) > 0.01);
        bridge.preview_note(midi, [0x80, 60, 0]).unwrap();
        peak(&mut bridge, &mut host, midi, 1500);
        assert!(peak(&mut bridge, &mut host, midi, 20) < 1e-4);
        let errors = bridge.shared.borrow().errors.clone();
        assert!(errors.is_empty(), "{errors:?}");
    }
}

//! Controller side (Worker): [`WebBridge`] implements `EngineBridge` by serializing every
//! call into the control ring for the AudioWorklet, and reads [`EngineReport`]s back.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use ether_controller::{BridgeError, EngineBridge};
use ether_core::analysis::AnalysisFrame;
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{
    Base64Bytes, BuiltinDevice, BuiltinDeviceType, DeviceId, MediaId, MediaRef, ParamId,
    PluginInstance,
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
    /// `audio-streaming`: stream reports (cursors, misses) since the last poll.
    pub stream_reports: Vec<crate::media_stream::StreamReport>,
    /// `web-latency`: latest node latencies reported by the Worklet (virtual keys), read by
    /// `EngineBridge::node_latency`.
    pub latencies: LatencyTable,
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

pub fn shared<M: RingMemory>(control: M, reports: M) -> Shared<M> {
    Rc::new(RefCell::new(BridgeShared {
        control: RingWriter::new(control),
        reports: RingReader::new(reports),
        errors: Vec::new(),
        blocks: 0,
        analysis: Vec::new(),
        stream_reports: Vec::new(),
        latencies: LatencyTable::default(),
    }))
}

/// `EngineBridge` over the SharedArrayBuffer rings. Node keys are virtual (see
/// [`EngineMsg`]); the Worklet maps them. No plugins on the web.
pub struct WebBridge<M: RingMemory> {
    shared: Shared<M>,
    next_node: u32,
    devices: BTreeMap<DeviceId, (NodeKey, BuiltinDeviceType)>,
    /// `audio-streaming`: long media streamed from OPFS ([`crate::media_stream`]).
    streams: crate::media_stream::WebStreams,
    /// Last known playhead beat (where `Play` starts).
    playhead_beat: f64,
}

impl<M: RingMemory> WebBridge<M> {
    pub fn new(shared: Shared<M>) -> Self {
        Self {
            shared,
            next_node: 0,
            devices: BTreeMap::new(),
            streams: Default::default(),
            playhead_beat: 0.0,
        }
    }

    /// `audio-streaming`: how the Worker opens media files for streaming (the host's
    /// OPFS, by byte ranges). Without one, `stream_media` declines (whole-file loads).
    pub fn set_stream_opener(&mut self, opener: crate::media_stream::Opener) {
        self.streams.set_opener(opener);
    }

    /// `audio-streaming`: is `media` streamed (tests/diagnostics)?
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
        self.next_node = self.next_node.wrapping_add(1);
        let key = NodeKey {
            index: self.next_node,
            generation: 1,
        };
        self.devices.insert(device, (key, kind.device_type()));
        self.shared.borrow_mut().latencies.track(key);
        self.send(EngineMsg::CreateBuiltin {
            key,
            device: kind.clone(),
            params: params.to_vec(),
        });
        Ok(key)
    }

    fn create_plugin(
        &mut self,
        _device: DeviceId,
        _plugin: &PluginInstance,
        _state: Option<&Base64Bytes>,
    ) -> Result<NodeKey, BridgeError> {
        Err(BridgeError::Unsupported(
            "plugins are not available in the browser".into(),
        ))
    }

    fn destroy_node(&mut self, key: NodeKey) -> Result<(), BridgeError> {
        self.devices.retain(|_, (k, _)| *k != key);
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
        );
        if !updatable || ty != kind.device_type() {
            return Ok(false);
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
        self.send(EngineMsg::LoadMedia {
            media: media.id,
            audio,
        });
        Ok(())
    }

    fn unload_media(&mut self, media: MediaId) -> Result<(), BridgeError> {
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
        self.send(EngineMsg::SetParam { change });
        Ok(())
    }

    fn transport(&mut self, control: TransportControl) -> Result<(), BridgeError> {
        // `audio-streaming`: the jump target's chunks go first (the ring is FIFO).
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
        // `audio-streaming`: follow the worklet's cursors, keep the read-ahead filled.
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

    /// `audio-streaming` (CONTRACTS.md §13.1): stream long media from OPFS through the
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
        Ok(true)
    }

    fn descriptor(&mut self, device: DeviceId) -> Option<DeviceDescriptor> {
        let (_, kind) = self.devices.get(&device)?;
        Some(ether_devices::descriptor(*kind))
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

    /// `web-latency`: the latest latency the Worklet reported for `key` (drained by
    /// [`EngineBridge::poll`]); `None` while it still has the latency it was created with,
    /// which the Worklet's engine already compiles PDC with.
    fn node_latency(&self, key: NodeKey) -> Option<u32> {
        self.shared.borrow().latencies.get(key)
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
            monitor: false,
            armed: false,
            clips: vec![],
            automation: vec![],
            racks: Vec::new(),
            expression: Default::default(),
            hw_io: Vec::new(),
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

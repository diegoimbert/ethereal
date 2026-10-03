//! `audio-streaming` (CONTRACTS.md §13.1) in the controller: long media are handed to a
//! streaming host (`EngineBridge::stream_media`) instead of being decoded into memory; one
//! decode pass still builds the peaks (identical to the whole-file ones); hosts that can't
//! stream get the v0.2 whole-file load; underruns are reported, throttled.

mod common;

use std::sync::Arc;

use common::{FakeBridge, FakeHost, T0, events, ok, sine, wav};
use ether_controller::media_stream::{STREAM_MIN_SECONDS, StreamSource, UNDERRUN_REPORT_MS};
use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::{BridgeError, Controller, ControllerConfig, EngineBridge, EtherController};
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::media::*;
use ether_core::protocol::model::*;
use ether_core::protocol::project::ProjectCommand;
use ether_core::protocol::*;
use ether_core::{EngineOutputs, NodeKey, ParamChange, RenderGraphDesc, TransportControl};
use ether_media::DecodedAudio;

/// How the fake host answers `stream_media`.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Stream,
    Unsupported,
    Fail,
}

struct StreamBridge {
    inner: FakeBridge,
    mode: Mode,
    streams: Vec<StreamSource>,
    unloaded: Vec<MediaId>,
    underruns: u32,
}

impl EngineBridge for StreamBridge {
    fn create_builtin(
        &mut self,
        device: DeviceId,
        kind: &BuiltinDevice,
        params: &[(ParamId, f64)],
    ) -> Result<NodeKey, BridgeError> {
        self.inner.create_builtin(device, kind, params)
    }
    fn create_plugin(
        &mut self,
        device: DeviceId,
        plugin: &PluginInstance,
        state: Option<&Base64Bytes>,
    ) -> Result<NodeKey, BridgeError> {
        self.inner.create_plugin(device, plugin, state)
    }
    fn destroy_node(&mut self, key: NodeKey) -> Result<(), BridgeError> {
        self.inner.destroy_node(key)
    }
    fn load_media(
        &mut self,
        media: &MediaRef,
        audio: Arc<DecodedAudio>,
    ) -> Result<(), BridgeError> {
        self.inner.load_media(media, audio)
    }
    fn unload_media(&mut self, media: MediaId) -> Result<(), BridgeError> {
        self.unloaded.push(media);
        self.inner.unload_media(media)
    }
    fn publish(&mut self, graph: RenderGraphDesc) -> Result<(), BridgeError> {
        self.inner.publish(graph)
    }
    fn set_param(&mut self, change: ParamChange) -> Result<(), BridgeError> {
        self.inner.set_param(change)
    }
    fn transport(&mut self, control: TransportControl) -> Result<(), BridgeError> {
        self.inner.transport(control)
    }
    fn poll(&mut self, out: &mut EngineOutputs) {
        self.inner.poll(out);
        out.underruns = std::mem::take(&mut self.underruns);
    }
    fn descriptor(&mut self, device: DeviceId) -> Option<DeviceDescriptor> {
        self.inner.descriptor(device)
    }
    fn stream_media(&mut self, source: &StreamSource) -> Result<bool, BridgeError> {
        match self.mode {
            Mode::Stream => {
                self.streams.push(source.clone());
                Ok(true)
            }
            Mode::Unsupported => Ok(false),
            Mode::Fail => Err(BridgeError::Other("cannot open".into())),
        }
    }
}

type Ctl = EtherController<StreamBridge, FakeHost, MemoryStore, MemoryLibrary>;

struct H {
    ctl: Ctl,
    req: u32,
    ids: IdGen,
}

const SR: u32 = 8_000;

impl H {
    fn new(mode: Mode, seconds: f64) -> (Self, Vec<u8>) {
        let frames = (SR as f64 * seconds) as usize;
        let bytes = wav(
            SR,
            &[sine(SR, 440.0, frames, 0.5), sine(SR, 110.0, frames, 0.25)],
        );
        let mut lib = MemoryLibrary::new();
        lib.add_root("lib", "Library");
        lib.add_file("lib", "long.wav", bytes.clone());
        let mut store = MemoryStore::new();
        store.now_ms = T0;
        let bridge = StreamBridge {
            inner: FakeBridge::default(),
            mode,
            streams: vec![],
            unloaded: vec![],
            underruns: 0,
        };
        let mut h = Self {
            ctl: EtherController::with_config(
                bridge,
                FakeHost { now: T0 },
                store,
                lib,
                ControllerConfig::default(),
            ),
            req: 1,
            ids: IdGen::new(7),
        };
        let id = h.ids.next_project_id(T0);
        h.send(Command::Project(ProjectCommand::Create {
            id,
            name: "Streaming".into(),
        }));
        (h, bytes)
    }

    fn send(&mut self, command: Command) -> Vec<ServerMessage> {
        let mut out = Vec::new();
        self.req += 1;
        self.ctl.handle(
            ClientMessage {
                id: self.req,
                gesture: None,
                command,
            },
            &mut out,
        );
        out
    }

    fn tick(&mut self) -> Vec<ServerMessage> {
        let mut out = Vec::new();
        let now = self.ctl.host.now;
        self.ctl.tick(now, &mut out);
        out
    }

    fn drain(&mut self) -> Vec<Event> {
        let mut all = Vec::new();
        for _ in 0..10_000 {
            all.extend(events(&self.tick()));
            if !self.ctl.media_pending() {
                return all;
            }
        }
        panic!("media jobs did not finish");
    }

    fn import(&mut self) -> MediaRef {
        let id: MediaId = self.ids.next(T0);
        let out = self.send(Command::Media(MediaCommand::Import {
            id,
            source: MediaSource::Location {
                location: BrowseLocation::Library { id: "lib".into() },
                path: "long.wav".into(),
            },
        }));
        let ReplyValue::Media { media } = ok(&out) else {
            panic!("import reply")
        };
        media
    }

    fn peaks(&mut self, media: &MediaRef) -> PeakData {
        let out = self.send(Command::Media(MediaCommand::GetPeaks {
            request: PeakRequest {
                media: media.id,
                samples_per_peak: 256,
                start_frame: 0.0,
                frame_count: media.frames as f64,
            },
        }));
        let ReplyValue::Peaks { peaks } = ok(&out) else {
            panic!("peaks reply")
        };
        peaks
    }
}

#[test]
fn long_media_stream_and_keep_their_peaks() {
    let seconds = STREAM_MIN_SECONDS + 5.0;
    let (mut h, _) = H::new(Mode::Stream, seconds);
    let media = h.import();
    let evs = h.drain();
    // Registered with the host, never decoded into the engine.
    let b = &h.ctl.bridge;
    assert_eq!(b.streams.len(), 1);
    let s = &b.streams[0];
    assert_eq!(s.media.id, media.id);
    assert_eq!(s.media.frames, media.frames);
    assert_eq!(s.engine_sample_rate, 48_000);
    assert_eq!(s.external_path, None);
    assert_eq!(s.project, h.ctl.project().unwrap().id);
    assert!(b.inner.media.is_empty(), "no whole-file load");
    assert!(evs.contains(&Event::Media {
        event: MediaEvent::PeaksReady { media: media.id }
    }));
    assert!(evs.contains(&Event::Media {
        event: MediaEvent::ImportProgress {
            media: media.id,
            progress: 1.0
        }
    }));
    let streamed = h.peaks(&media);

    // The same file through the whole-file path: identical peaks.
    let (mut w, _) = H::new(Mode::Unsupported, seconds);
    let m2 = w.import();
    w.drain();
    assert!(w.ctl.bridge.streams.is_empty());
    assert_eq!(w.ctl.bridge.inner.media.len(), 1, "whole-file load");
    assert_eq!(w.peaks(&m2), streamed);
}

#[test]
fn short_media_are_not_streamed() {
    let (mut h, _) = H::new(Mode::Stream, 3.0);
    h.import();
    h.drain();
    assert!(h.ctl.bridge.streams.is_empty());
    assert_eq!(h.ctl.bridge.inner.media.len(), 1);
}

#[test]
fn a_host_that_fails_to_stream_gets_the_whole_file() {
    let (mut h, _) = H::new(Mode::Fail, STREAM_MIN_SECONDS + 1.0);
    let media = h.import();
    h.drain();
    assert!(h.ctl.bridge.inner.media.contains_key(&media.id));
}

#[test]
fn reopening_streams_at_once_with_cached_peaks() {
    let (mut h, _) = H::new(Mode::Stream, STREAM_MIN_SECONDS + 2.0);
    let media = h.import();
    h.drain();
    let pid = h.ctl.project().unwrap().id;
    let out = h.send(Command::Project(ProjectCommand::Save));
    ok(&out);
    // Switching project unloads the stream.
    let other = h.ids.next_project_id(T0);
    ok(&h.send(Command::Project(ProjectCommand::Create {
        id: other,
        name: "Other".into(),
    })));
    assert!(h.ctl.bridge.unloaded.contains(&media.id));
    h.ctl.bridge.streams.clear();
    let out = h.send(Command::Project(ProjectCommand::Open { id: pid }));
    ok(&out);
    // One tick: peaks come from the cache, the stream is registered, no decode pass.
    let evs = events(&h.tick());
    assert_eq!(h.ctl.bridge.streams.len(), 1);
    assert!(!h.ctl.media_pending());
    assert!(evs.contains(&Event::Media {
        event: MediaEvent::PeaksReady { media: media.id }
    }));
}

#[test]
fn underruns_are_reported_throttled() {
    let (mut h, _) = H::new(Mode::Stream, STREAM_MIN_SECONDS + 1.0);
    h.import();
    h.drain();
    h.ctl.host.now += UNDERRUN_REPORT_MS;
    h.ctl.bridge.underruns = 3;
    let report = |evs: Vec<Event>| {
        evs.into_iter().find_map(|e| match e {
            Event::Media {
                event: MediaEvent::StreamUnderruns { count },
            } => Some(count),
            _ => None,
        })
    };
    assert_eq!(report(events(&h.tick())), Some(3));
    // Within the throttle window: accumulated, not reported.
    h.ctl.bridge.underruns = 2;
    h.ctl.host.now += 10;
    assert_eq!(report(events(&h.tick())), None);
    h.ctl.bridge.underruns = 1;
    h.ctl.host.now += UNDERRUN_REPORT_MS;
    assert_eq!(report(events(&h.tick())), Some(3));
    // Nothing new: silent.
    h.ctl.host.now += UNDERRUN_REPORT_MS;
    assert_eq!(report(events(&h.tick())), None);
}

//! Offline export (`export` node): the offline render equals the realtime render
//! (sample-exact after the latency drop), stems sum to the mix, WAV/FLAC decode round
//! trips, cancel, the download path (stores without `write_export`), and errors.

mod common;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use common::*;
use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::store::{ProjectStore, StoreError};
use ether_controller::{BridgeError, Controller, ControllerConfig, EngineBridge, EtherController};
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::devices::{DeviceCategory, DeviceCommand, DeviceDescriptor, DeviceSpec};
use ether_core::protocol::export::*;
use ether_core::protocol::media::{BrowseLocation, DirectoryListing, MediaCommand, MediaSource};
use ether_core::protocol::mixer::MixerCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteSpec};
use ether_core::protocol::project::{ProjectCommand, ProjectSummary};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::transport::TransportCommand;
use ether_core::protocol::*;
use ether_core::{
    AudioSource, Engine, EngineConfig, EngineHandle, EngineOutputs, GarbageCollector, NodeKey,
    ParamChange, RenderGraphDesc, TransportControl,
};
use ether_media::{DecodedAudio, InMemorySource};

const SR: u32 = 48_000;
const BLOCK: usize = ether_core::offline::OFFLINE_MAX_BLOCK;

// ─── A real engine behind the bridge (for the offline == realtime test) ─────────────────

struct Sources<'a>(&'a HashMap<MediaId, Arc<dyn AudioSource>>);
impl ether_devices::SampleResolver for Sources<'_> {
    fn resolve(&self, media: MediaId) -> Option<Arc<dyn AudioSource>> {
        self.0.get(&media).cloned()
    }
}

struct RealBridge {
    engine: Engine,
    handle: EngineHandle,
    gc: GarbageCollector,
    sources: HashMap<MediaId, Arc<dyn AudioSource>>,
    devices: BTreeMap<DeviceId, BuiltinDeviceType>,
}

impl RealBridge {
    fn new() -> Self {
        let parts = ether_core::create(EngineConfig {
            sample_rate: SR,
            max_block_size: BLOCK,
            ..Default::default()
        });
        Self {
            engine: parts.engine,
            handle: parts.handle,
            gc: parts.gc,
            sources: HashMap::new(),
            devices: BTreeMap::new(),
        }
    }

    /// Render `frames` of the live engine (stereo), in engine-sized blocks.
    fn render(&mut self, frames: usize) -> [Vec<f32>; 2] {
        let mut out = [Vec::new(), Vec::new()];
        let mut l = vec![0.0f32; BLOCK];
        let mut r = vec![0.0f32; BLOCK];
        let mut done = 0;
        while done < frames {
            let n = BLOCK.min(frames - done);
            self.engine.process(&[], &mut [&mut l[..n], &mut r[..n]], n);
            out[0].extend_from_slice(&l[..n]);
            out[1].extend_from_slice(&r[..n]);
            self.gc.collect();
            done += n;
        }
        out
    }
}

impl EngineBridge for RealBridge {
    fn create_builtin(
        &mut self,
        device: DeviceId,
        kind: &BuiltinDevice,
        params: &[(ParamId, f64)],
    ) -> Result<NodeKey, BridgeError> {
        let mut node = ether_devices::create(kind, &Sources(&self.sources));
        for (id, v) in params {
            node.set_param(*id, *v);
        }
        self.devices.insert(device, kind.device_type());
        self.handle
            .add_node(node)
            .map_err(|e| BridgeError::Other(e.to_string()))
    }
    fn create_plugin(
        &mut self,
        _: DeviceId,
        _: &PluginInstance,
        _: Option<&Base64Bytes>,
    ) -> Result<NodeKey, BridgeError> {
        Err(BridgeError::Unsupported("no plugins".into()))
    }
    fn destroy_node(&mut self, key: NodeKey) -> Result<(), BridgeError> {
        self.handle
            .remove_node(key)
            .map_err(|e| BridgeError::Other(e.to_string()))
    }
    fn load_media(
        &mut self,
        media: &MediaRef,
        audio: Arc<DecodedAudio>,
    ) -> Result<(), BridgeError> {
        let src: Arc<dyn AudioSource> = Arc::new(InMemorySource::new(audio));
        self.sources.insert(media.id, src.clone());
        self.handle
            .add_source(media.id, src)
            .map_err(|e| BridgeError::Other(e.to_string()))
    }
    fn unload_media(&mut self, media: MediaId) -> Result<(), BridgeError> {
        self.sources.remove(&media);
        self.handle
            .remove_source(media)
            .map_err(|e| BridgeError::Other(e.to_string()))
    }
    fn publish(&mut self, graph: RenderGraphDesc) -> Result<(), BridgeError> {
        self.handle
            .publish(graph)
            .map_err(|e| BridgeError::Other(e.to_string()))
    }
    fn set_param(&mut self, change: ParamChange) -> Result<(), BridgeError> {
        self.handle
            .set_param(change)
            .map_err(|e| BridgeError::Other(e.to_string()))
    }
    fn transport(&mut self, control: TransportControl) -> Result<(), BridgeError> {
        self.handle
            .transport(control)
            .map_err(|e| BridgeError::Other(e.to_string()))
    }
    fn poll(&mut self, out: &mut EngineOutputs) {
        self.handle.poll(out);
    }
    fn descriptor(&mut self, device: DeviceId) -> Option<DeviceDescriptor> {
        self.devices
            .get(&device)
            .map(|t| ether_devices::descriptor(*t))
    }
}

// ─── A store without `write_export` (web/remote: downloads) ─────────────────────────────

struct DownloadStore(MemoryStore);

impl ProjectStore for DownloadStore {
    fn list(&mut self) -> Result<Vec<ProjectSummary>, StoreError> {
        self.0.list()
    }
    fn create(&mut self, id: ProjectId) -> Result<(), StoreError> {
        self.0.create(id)
    }
    fn load(&mut self, id: ProjectId) -> Result<String, StoreError> {
        self.0.load(id)
    }
    fn save(&mut self, id: ProjectId, json: &str) -> Result<ProjectSummary, StoreError> {
        self.0.save(id, json)
    }
    fn duplicate(&mut self, from: ProjectId, to: ProjectId) -> Result<(), StoreError> {
        self.0.duplicate(from, to)
    }
    fn delete(&mut self, id: ProjectId) -> Result<(), StoreError> {
        self.0.delete(id)
    }
    fn read(&mut self, id: ProjectId, rel: &str) -> Result<Vec<u8>, StoreError> {
        self.0.read(id, rel)
    }
    fn write(&mut self, id: ProjectId, rel: &str, bytes: &[u8]) -> Result<(), StoreError> {
        self.0.write(id, rel, bytes)
    }
    fn list_dir(&mut self, id: ProjectId, rel: &str) -> Result<DirectoryListing, StoreError> {
        self.0.list_dir(id, rel)
    }
}

// ─── A small generic driver ─────────────────────────────────────────────────────────────

struct Drive<B: EngineBridge, S: ProjectStore> {
    ctl: EtherController<B, FakeHost, S, MemoryLibrary>,
    ids: IdGen,
    next: u32,
    pid: ProjectId,
}

impl<B: EngineBridge, S: ProjectStore> Drive<B, S> {
    fn new(bridge: B, mut store: S, library: MemoryLibrary) -> Self {
        let _ = store.list();
        let mut d = Self {
            ctl: EtherController::with_config(
                bridge,
                FakeHost { now: T0 },
                store,
                library,
                ControllerConfig {
                    engine_sample_rate: SR,
                    ..Default::default()
                },
            ),
            ids: IdGen::new(7),
            next: 1,
            pid: IdGen::new(1).next_project_id(T0),
        };
        d.new_project("Song");
        d
    }

    fn new_project(&mut self, name: &str) {
        self.pid = self.ids.next_project_id(T0);
        self.ok(Command::Project(ProjectCommand::Create {
            id: self.pid,
            name: name.into(),
        }));
    }

    fn id<I: Id>(&mut self) -> I {
        self.ids.next(T0)
    }

    fn send(&mut self, command: Command) -> Vec<ServerMessage> {
        let id = self.next;
        self.next += 1;
        let mut out = Vec::new();
        self.ctl.handle(
            ClientMessage {
                id,
                gesture: None,
                command,
            },
            &mut out,
        );
        out
    }

    fn ok(&mut self, command: Command) -> ReplyValue {
        ok(&self.send(command))
    }

    fn tick(&mut self) -> Vec<ServerMessage> {
        let mut out = Vec::new();
        self.ctl.host.now += 16;
        let now = self.ctl.host.now;
        self.ctl.tick(now, &mut out);
        out
    }

    fn settle_media(&mut self) {
        for _ in 0..10_000 {
            self.tick();
            if !self.ctl.media_pending() {
                return;
            }
        }
        panic!("media did not load");
    }

    fn track(&mut self, kind: TrackKind, name: &str, parent: Option<TrackId>) -> TrackId {
        let id = self.id();
        self.ok(Command::Track(TrackCommand::Create {
            id,
            kind,
            name: Some(name.into()),
            color: None,
            parent,
            before: None,
        }));
        id
    }

    fn device(&mut self, track: TrackId, device: BuiltinDevice) -> DeviceId {
        let id = self.id();
        self.ok(Command::Device(DeviceCommand::Insert {
            id,
            track,
            device: DeviceSpec::Builtin { device },
            before: None,
        }));
        id
    }

    fn master(&self) -> TrackId {
        self.ctl
            .project()
            .unwrap()
            .tracks
            .values()
            .find(|t| t.kind == TrackKind::Master)
            .unwrap()
            .id
    }

    /// A MIDI clip at `start` with a few notes.
    fn notes(&mut self, track: TrackId, start: f64, pitches: &[u8]) {
        let clip = self.id();
        self.ok(Command::Clip(ClipCommand::CreateMidi {
            id: clip,
            track,
            start: Beats(start),
            length: Beats(4.0),
            name: None,
        }));
        if pitches.is_empty() {
            return;
        }
        let notes = pitches
            .iter()
            .enumerate()
            .map(|(i, p)| NoteSpec {
                id: self.id(),
                pitch: *p,
                velocity: 0.8,
                start: Beats(i as f64 * 0.75),
                duration: Beats(0.5),
            })
            .collect();
        self.ok(Command::Note(NoteCommand::Add { clip, notes }));
    }

    /// Start a render; returns the job id.
    fn render(&mut self, request: ExportRequest) -> String {
        let job = format!("job{}", self.next);
        let v = self.ok(Command::Export(ExportCommand::Render {
            job: job.clone(),
            request,
        }));
        assert_eq!(v, ReplyValue::ExportStarted { job: job.clone() });
        job
    }

    /// Tick until the job ends; returns its final event and every export event seen.
    fn finish(&mut self, job: &str) -> (ExportEvent, Vec<ExportEvent>) {
        let mut seen = Vec::new();
        for _ in 0..100_000 {
            for e in events(&self.tick()) {
                if let Event::Export { event } = e {
                    seen.push(event.clone());
                    match &event {
                        ExportEvent::Progress { job: j, .. } => assert_eq!(j, job),
                        _ => return (event, seen),
                    }
                }
            }
        }
        panic!("export did not finish");
    }

    fn files(&mut self, request: ExportRequest) -> Vec<String> {
        let job = self.render(request);
        match self.finish(&job).0 {
            ExportEvent::Done {
                result: ExportResult::Files { files },
                ..
            } => files,
            other => panic!("unexpected {other:?}"),
        }
    }

    fn read(&mut self, path: &str) -> Vec<u8> {
        self.ctl.store.read(self.pid, path).expect("exported file")
    }
}

fn request(range: ExportRange, container: AudioContainer, depth: BitDepth) -> ExportRequest {
    ExportRequest {
        range,
        format: ExportFormat {
            container,
            bit_depth: depth,
            sample_rate: None,
        },
        mode: ExportMode::Mix,
        normalize: false,
        tail_seconds: 0.0,
        name: None,
    }
}

fn custom(start: f64, end: f64) -> ExportRange {
    ExportRange::Custom {
        start: Beats(start),
        end: Beats(end),
    }
}

fn decode(bytes: &[u8], ext: &str) -> DecodedAudio {
    ether_media::decode(bytes, Some(ext)).expect("decodes")
}

fn peak(ch: &[Vec<f32>]) -> f32 {
    ch.iter().flatten().fold(0.0, |m, s| m.max(s.abs()))
}

fn library() -> MemoryLibrary {
    let mut lib = MemoryLibrary::new();
    lib.add_root("lib", "Library");
    // 1.5 s at 44.1 kHz: the controller resamples to the engine rate.
    let l = sine(44_100, 330.0, 66_150, 0.4);
    let r = sine(44_100, 165.0, 66_150, 0.3);
    lib.add_file("lib", "tone.wav", wav(44_100, &[l, r]));
    lib
}

fn import<B: EngineBridge, S: ProjectStore>(d: &mut Drive<B, S>, track: TrackId, at: f64) {
    let media: MediaId = d.id();
    d.ok(Command::Media(MediaCommand::Import {
        id: media,
        source: MediaSource::Location {
            location: BrowseLocation::Library { id: "lib".into() },
            path: "tone.wav".into(),
        },
    }));
    let clip = d.id();
    d.ok(Command::Clip(ClipCommand::CreateAudio {
        id: clip,
        track,
        start: Beats(at),
        media,
    }));
}

/// Synth + audio clip + effects + a latency-reporting limiter on master.
fn build_song<B: EngineBridge, S: ProjectStore>(d: &mut Drive<B, S>) {
    let midi = d.track(TrackKind::Midi, "Keys", None);
    d.device(midi, BuiltinDevice::Synth);
    d.device(midi, BuiltinDevice::Delay);
    d.notes(midi, 0.0, &[60, 64, 67, 72]);
    d.notes(midi, 4.0, &[62, 65, 69]);
    let audio = d.track(TrackKind::Audio, "Tone", None);
    d.device(audio, BuiltinDevice::Compressor);
    import(d, audio, 1.0);
    let master = d.master();
    d.device(master, BuiltinDevice::Limiter);
    d.settle_media();
}

#[test]
fn offline_render_equals_realtime_render() {
    let mut d = Drive::new(RealBridge::new(), MemoryStore::new(), library());
    build_song(&mut d);
    d.tick(); // publish
    let files = d.files(request(
        custom(0.0, 8.0),
        AudioContainer::Wav,
        BitDepth::Float32,
    ));
    assert_eq!(files, vec!["exports/Song.wav".to_string()]);
    let exported = decode(&d.read(&files[0]), "wav");
    // 8 beats at 120 bpm = 4 s.
    assert_eq!(exported.frames(), 4 * SR as usize);
    assert_eq!(exported.sample_rate, SR);
    assert!(peak(&exported.channels) > 0.05, "audible");

    // Realtime: play the live engine from 0; its output is `latency` frames late.
    let latency = d.ctl.bridge.handle.latency() as usize;
    assert!(latency > 0, "the limiter's lookahead is compensated");
    d.ok(Command::Transport(TransportCommand::Play));
    let live = d.ctl.bridge.render(latency + exported.frames());
    for c in 0..2 {
        let live = &live[c][latency..];
        let off = &exported.channels[c];
        let first = live.iter().zip(off).position(|(a, b)| a != b);
        assert_eq!(first, None, "channel {c} differs at frame {first:?}");
    }
}

#[test]
fn stems_sum_to_the_mix() {
    let mut d = Drive::new(FakeBridge::default(), MemoryStore::new(), library());
    let a = d.track(TrackKind::Midi, "Lead", None);
    d.device(a, BuiltinDevice::Synth);
    d.notes(a, 0.0, &[60, 67]);
    let group = d.track(TrackKind::Group, "Bus", None);
    let b = d.track(TrackKind::Audio, "Tone", Some(group));
    import(&mut d, b, 0.0);
    let ret = d.track(TrackKind::Return, "Echo", None);
    d.device(ret, BuiltinDevice::Delay);
    let send = d.id();
    d.ok(Command::Mixer(MixerCommand::CreateSend {
        id: send,
        from: a,
        to: ret,
        level: Decibels(-6.0),
        pre_fader: false,
    }));
    let master = d.master();
    d.ok(Command::Mixer(MixerCommand::SetVolume {
        track: master,
        volume: Decibels(-3.0),
    }));
    d.settle_media();

    let mut req = request(custom(0.0, 4.0), AudioContainer::Wav, BitDepth::Float32);
    req.tail_seconds = 0.5;
    let mix = d.files(req.clone());
    let mix = decode(&d.read(&mix[0]), "wav");
    req.mode = ExportMode::Stems {
        tracks: vec![a, b, a],
    };
    let stems = d.files(req);
    assert_eq!(
        stems,
        vec![
            "exports/Song - Lead.wav".to_string(),
            "exports/Song - Tone.wav".to_string()
        ]
    );
    let stems: Vec<DecodedAudio> = stems.iter().map(|f| decode(&d.read(f), "wav")).collect();
    assert_eq!(mix.frames(), (2.5 * SR as f64) as usize);
    for s in &stems {
        assert_eq!(s.frames(), mix.frames());
        assert!(peak(&s.channels) > 0.01, "each stem is audible");
    }
    let mut max_err = 0.0f32;
    for c in 0..2 {
        for i in 0..mix.frames() {
            let sum: f32 = stems.iter().map(|s| s.channels[c][i]).sum();
            max_err = max_err.max((sum - mix.channels[c][i]).abs());
        }
    }
    assert!(max_err < 1e-4, "stems sum to the mix (max error {max_err})");
}

#[test]
fn formats_round_trip() {
    let mut d = Drive::new(FakeBridge::default(), MemoryStore::new(), library());
    let t = d.track(TrackKind::Audio, "Tone", None);
    import(&mut d, t, 0.0);
    d.settle_media();
    let float = d.files(request(
        custom(0.0, 2.0),
        AudioContainer::Wav,
        BitDepth::Float32,
    ));
    let reference = decode(&d.read(&float[0]), "wav");
    assert_eq!(reference.frames(), SR as usize);
    for (container, depth, ext, tol) in [
        (AudioContainer::Wav, BitDepth::Int16, "wav", 3.0 / 32768.0),
        (
            AudioContainer::Wav,
            BitDepth::Int24,
            "wav",
            2.0 / 8_388_608.0,
        ),
        (AudioContainer::Flac, BitDepth::Int16, "flac", 3.0 / 32768.0),
        (
            AudioContainer::Flac,
            BitDepth::Int24,
            "flac",
            2.0 / 8_388_608.0,
        ),
    ] {
        let files = d.files(request(custom(0.0, 2.0), container, depth));
        assert!(files[0].ends_with(ext));
        let got = decode(&d.read(&files[0]), ext);
        assert_eq!(got.frames(), reference.frames(), "{container:?} {depth:?}");
        for (a, b) in got.channels.iter().zip(&reference.channels) {
            for (x, y) in a.iter().zip(b) {
                assert!((x - y).abs() <= tol, "{container:?} {depth:?}: {x} vs {y}");
            }
        }
    }

    // Resampled, normalized, named.
    let mut req = request(custom(0.0, 2.0), AudioContainer::Flac, BitDepth::Int24);
    req.format.sample_rate = Some(44_100);
    req.normalize = true;
    req.name = Some("Take/1".into());
    let files = d.files(req);
    assert_eq!(files, vec!["exports/Take_1.flac".to_string()]);
    let got = decode(&d.read(&files[0]), "flac");
    assert_eq!(got.sample_rate, 44_100);
    assert_eq!(got.frames(), 44_100);
    let p = peak(&got.channels);
    assert!((p - 0.98855).abs() < 2e-3, "normalized to -0.1 dBFS: {p}");
}

#[test]
fn metronome_is_never_rendered() {
    let mut d = Drive::new(FakeBridge::default(), MemoryStore::new(), library());
    d.ok(Command::Transport(TransportCommand::SetMetronome {
        enabled: true,
    }));
    let t = d.track(TrackKind::Midi, "Empty", None);
    d.notes(t, 0.0, &[]);
    let files = d.files(request(
        ExportRange::Project,
        AudioContainer::Wav,
        BitDepth::Float32,
    ));
    let got = decode(&d.read(&files[0]), "wav");
    assert_eq!(got.frames(), 2 * SR as usize, "4 beats");
    assert_eq!(peak(&got.channels), 0.0);
}

#[test]
fn cancel_and_one_job_at_a_time() {
    let mut d = Drive::new(FakeBridge::default(), MemoryStore::new(), library());
    let t = d.track(TrackKind::Midi, "Keys", None);
    d.device(t, BuiltinDevice::Synth);
    d.notes(t, 0.0, &[60]);
    let mut req = request(custom(0.0, 64.0), AudioContainer::Wav, BitDepth::Int16);
    req.tail_seconds = 60.0;
    let job = d.render(req.clone());
    let busy = d.send(Command::Export(ExportCommand::Render {
        job: "other".into(),
        request: req.clone(),
    }));
    assert_eq!(err(&busy).code, ErrorCode::InvalidState);
    let first = events(&d.tick());
    assert!(first.iter().any(|e| matches!(
        e,
        Event::Export {
            event: ExportEvent::Progress { .. }
        }
    )));
    // Cancelling another job id is a no-op.
    d.ok(Command::Export(ExportCommand::Cancel {
        job: "nope".into(),
    }));
    let out = d.send(Command::Export(ExportCommand::Cancel { job: job.clone() }));
    ok(&out);
    assert!(events(&out).contains(&Event::Export {
        event: ExportEvent::Cancelled { job: job.clone() }
    }));
    for _ in 0..5 {
        assert!(
            events(&d.tick())
                .iter()
                .all(|e| !matches!(e, Event::Export { .. }))
        );
    }
    // A new job can start.
    req.tail_seconds = 0.0;
    req.range = custom(0.0, 1.0);
    let job = d.render(req);
    assert!(matches!(d.finish(&job).0, ExportEvent::Done { .. }));
}

#[test]
fn downloads_when_the_store_keeps_no_exports() {
    let mut d = Drive::new(
        FakeBridge::default(),
        DownloadStore(MemoryStore::new()),
        library(),
    );
    let t = d.track(TrackKind::Audio, "Tone", None);
    import(&mut d, t, 0.0);
    d.settle_media();
    let mut req = request(custom(0.0, 2.0), AudioContainer::Wav, BitDepth::Int16);
    req.mode = ExportMode::Stems { tracks: vec![t] };
    let job = d.render(req);
    let ExportEvent::Done {
        result: ExportResult::Download { downloads },
        ..
    } = d.finish(&job).0
    else {
        panic!("download result")
    };
    assert_eq!(downloads.len(), 1);
    let dl = &downloads[0];
    assert_eq!(dl.name, "Song - Tone.wav");
    assert_eq!(dl.mime, "audio/wav");
    // Pull it in small chunks.
    let mut bytes = Vec::new();
    loop {
        let v = d.ok(Command::Export(ExportCommand::ReadChunk {
            token: dl.token.clone(),
            offset: bytes.len() as f64,
            length: 50_000,
        }));
        let ReplyValue::Bytes { chunk } = v else {
            panic!()
        };
        assert_eq!(chunk.offset, bytes.len() as f64);
        bytes.extend_from_slice(&chunk.data.0);
        if chunk.eof {
            break;
        }
    }
    assert_eq!(bytes.len() as f64, dl.size);
    let got = decode(&bytes, "wav");
    assert_eq!(got.frames(), SR as usize);
    assert!(peak(&got.channels) > 0.1);
    // Nothing went to the store.
    assert!(
        d.ctl
            .store
            .0
            .read(d.pid, "exports/Song - Tone.wav")
            .is_err()
    );

    d.ok(Command::Export(ExportCommand::Release {
        token: dl.token.clone(),
    }));
    let gone = d.send(Command::Export(ExportCommand::ReadChunk {
        token: dl.token.clone(),
        offset: 0.0,
        length: 10,
    }));
    assert_eq!(err(&gone).code, ErrorCode::NotFound);

    // Switching projects drops the downloads.
    let job = d.render(request(
        custom(0.0, 1.0),
        AudioContainer::Wav,
        BitDepth::Int16,
    ));
    let ExportEvent::Done {
        result: ExportResult::Download { downloads },
        ..
    } = d.finish(&job).0
    else {
        panic!()
    };
    d.new_project("Other");
    d.tick();
    let gone = d.send(Command::Export(ExportCommand::ReadChunk {
        token: downloads[0].token.clone(),
        offset: 0.0,
        length: 10,
    }));
    assert_eq!(err(&gone).code, ErrorCode::NotFound);
}

#[test]
fn invalid_requests_and_plugin_failures() {
    let bridge = FakeBridge {
        plugins: Some(
            [(
                "fx".to_string(),
                plugin_descriptor("Fx", DeviceCategory::AudioEffect),
            )]
            .into(),
        ),
        ..Default::default()
    };
    let mut d = Drive::new(bridge, MemoryStore::new(), library());
    let bad = |d: &mut Drive<FakeBridge, MemoryStore>, r: ExportRequest| {
        err(&d.send(Command::Export(ExportCommand::Render {
            job: "j".into(),
            request: r,
        })))
        .code
    };
    let base = request(custom(0.0, 4.0), AudioContainer::Wav, BitDepth::Int16);
    // Empty project range.
    assert_eq!(
        bad(
            &mut d,
            request(ExportRange::Project, AudioContainer::Wav, BitDepth::Int16)
        ),
        ErrorCode::InvalidArgument
    );
    assert_eq!(
        bad(
            &mut d,
            request(custom(0.0, 4.0), AudioContainer::Flac, BitDepth::Float32)
        ),
        ErrorCode::InvalidArgument
    );
    assert_eq!(
        bad(
            &mut d,
            request(custom(4.0, 4.0), AudioContainer::Wav, BitDepth::Int16)
        ),
        ErrorCode::InvalidArgument
    );
    let mut r = base.clone();
    r.tail_seconds = -1.0;
    assert_eq!(bad(&mut d, r), ErrorCode::InvalidArgument);
    let mut r = base.clone();
    r.mode = ExportMode::Stems { tracks: vec![] };
    assert_eq!(bad(&mut d, r), ErrorCode::InvalidArgument);
    let mut r = base.clone();
    r.mode = ExportMode::Stems {
        tracks: vec![d.id()],
    };
    assert_eq!(bad(&mut d, r), ErrorCode::NotFound);
    let mut r = base.clone();
    r.format.sample_rate = Some(1000);
    assert_eq!(bad(&mut d, r), ErrorCode::InvalidArgument);

    // A plugin this host can't instantiate offline fails the job (never skipped).
    let t = d.track(TrackKind::Audio, "Fx", None);
    let dev: DeviceId = d.id();
    d.ok(Command::Device(DeviceCommand::Insert {
        id: dev,
        track: t,
        device: DeviceSpec::Plugin {
            plugin_id: "fx".into(),
            sandboxed: None,
            format: None,
        },
        before: None,
    }));
    let job = d.render(base);
    match d.finish(&job).0 {
        ExportEvent::Failed { message, .. } => {
            assert!(message.contains("can't be rendered offline"), "{message}")
        }
        other => panic!("expected failure, got {other:?}"),
    }
}

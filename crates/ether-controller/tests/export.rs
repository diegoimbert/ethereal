//! Offline export (`export`): the offline render equals the realtime render
//! (sample-exact after the latency drop), stems sum to the mix, WAV/FLAC/MP3 decode round
//! trips, cancel, the download path (stores without `write_export`), and errors.

mod common;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use common::*;
use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::store::{ProjectStore, StoreError};
use ether_controller::{
    BridgeError, Controller, ControllerConfig, EngineBridge, EtherController, HostServices,
};
use ether_core::protocol::automation::{AutomationCommand, PointSpec};
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
/// The offline engine's block size. Automation and tempo ramps are evaluated at block
/// rate by the engine, so sample-exact equality with a live render needs the live engine
/// to run the same blocks (`offline_render_equals_realtime_render`); at another block size
/// only rounding differs when there is no automation/ramp
/// (`offline_render_is_block_size_independent_without_ramps`).
const OFFLINE_BLOCK: usize = ether_core::offline::OFFLINE_MAX_BLOCK;

// ─── A real engine behind the bridge (for the offline == realtime test) ─────────────────

struct Sources<'a>(&'a HashMap<MediaId, Arc<dyn AudioSource>>);
impl ether_devices::SampleResolver for Sources<'_> {
    fn resolve(&self, media: MediaId) -> Option<Arc<dyn AudioSource>> {
        self.0.get(&media).cloned()
    }
}

struct RealBridge {
    block: usize,
    engine: Engine,
    handle: EngineHandle,
    gc: GarbageCollector,
    sources: HashMap<MediaId, Arc<dyn AudioSource>>,
    devices: BTreeMap<DeviceId, BuiltinDeviceType>,
}

impl RealBridge {
    fn new(block: usize) -> Self {
        let parts = ether_core::create(EngineConfig {
            sample_rate: SR,
            max_block_size: block,
            ..Default::default()
        });
        Self {
            block,
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
        let block = self.block;
        let mut l = vec![0.0f32; block];
        let mut r = vec![0.0f32; block];
        let mut done = 0;
        while done < frames {
            let n = block.min(frames - done);
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

/// Manual clock, or the wall clock (`wall`) for the tick-duration test.
struct TestHost {
    now: u64,
    wall: bool,
}

impl HostServices for TestHost {
    fn now_ms(&self) -> u64 {
        if self.wall {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as u64)
        } else {
            self.now
        }
    }
    fn random_seed(&mut self) -> u64 {
        0x5eed
    }
}

struct Drive<B: EngineBridge, S: ProjectStore> {
    ctl: EtherController<B, TestHost, S, MemoryLibrary>,
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
                TestHost {
                    now: T0,
                    wall: false,
                },
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
        in_memory: None,
        extra_formats: None,
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
fn build_song<B: EngineBridge, S: ProjectStore>(d: &mut Drive<B, S>, automation: bool) {
    let midi = d.track(TrackKind::Midi, "Keys", None);
    d.device(midi, BuiltinDevice::Synth);
    let delay = d.device(midi, BuiltinDevice::Delay);
    d.notes(midi, 0.0, &[60, 64, 67, 72]);
    d.notes(midi, 4.0, &[62, 65, 69]);
    let audio = d.track(TrackKind::Audio, "Tone", None);
    d.device(audio, BuiltinDevice::Compressor);
    import(d, audio, 1.0);
    let master = d.master();
    d.device(master, BuiltinDevice::Limiter);
    if !automation {
        d.settle_media();
        return;
    }
    // Automation: a track volume ramp and a device param ramp.
    lane(
        d,
        AutomationTarget::TrackVolume { track: midi },
        &[(0.0, 0.9), (3.0, 0.4), (7.0, 1.0)],
    );
    let param = ether_devices::descriptor(BuiltinDeviceType::Delay).params[0].id;
    lane(
        d,
        AutomationTarget::DeviceParam {
            device: delay,
            param,
        },
        &[(0.0, 0.2), (8.0, 0.8)],
    );
    d.settle_media();
}

fn lane<B: EngineBridge, S: ProjectStore>(
    d: &mut Drive<B, S>,
    target: AutomationTarget,
    points: &[(f64, f64)],
) {
    let track = match target {
        AutomationTarget::TrackVolume { track } => track,
        AutomationTarget::DeviceParam { device, .. } => {
            d.ctl.project().unwrap().devices[&device].track
        }
        _ => unreachable!(),
    };
    let id: AutomationLaneId = d.id();
    d.ok(Command::Automation(AutomationCommand::CreateLane {
        id,
        owner: AutomationOwner::Track { track },
        target,
    }));
    let points = points
        .iter()
        .map(|(t, v)| PointSpec {
            id: d.id(),
            time: Beats(*t),
            value: *v,
            curve: CurveShape::Linear,
        })
        .collect();
    d.ok(Command::Automation(AutomationCommand::AddPoints {
        lane: id,
        points,
    }));
}

/// Give the project a tempo ramp (120 → 150 bpm over beats 0..6, then constant) by
/// rewriting its file and reopening it (no tempo commands needed).
fn tempo_ramp<B: EngineBridge, S: ProjectStore>(d: &mut Drive<B, S>) {
    let mut p = d.ctl.project().unwrap().clone();
    for t in p.tempo_points.values_mut() {
        t.curve = TempoCurve::Linear;
    }
    let id: TempoPointId = d.id();
    p.tempo_points.insert(
        id,
        TempoPoint {
            id,
            time: Beats(6.0),
            bpm: 150.0,
            curve: TempoCurve::Step,
        },
    );
    // Switch away first (the controller saves the open project on switch), then rewrite
    // the file and reopen it.
    let song = d.pid;
    d.new_project("Scratch");
    let json = ether_core::protocol::model::file::save(&p, "test").unwrap();
    d.ctl.store.save(song, &json).unwrap();
    d.pid = song;
    d.ok(Command::Project(ProjectCommand::Open { id: song }));
    d.settle_media();
}

/// Export 8 beats as float WAV, then play the live engine from 0 (its output is
/// `latency` frames late). Returns (offline, live aligned) channels.
fn render_both(d: &mut Drive<RealBridge, MemoryStore>) -> (Vec<Vec<f32>>, Vec<Vec<f32>>) {
    d.tick(); // publish
    let files = d.files(request(
        custom(0.0, 8.0),
        AudioContainer::Wav,
        BitDepth::Float32,
    ));
    assert_eq!(files, vec!["exports/Song.wav".to_string()]);
    let exported = decode(&d.read(&files[0]), "wav");
    assert_eq!(exported.sample_rate, SR);
    assert!(peak(&exported.channels) > 0.05, "audible");
    let latency = d.ctl.bridge.handle.latency() as usize;
    assert!(latency > 0, "the limiter's lookahead is compensated");
    d.ok(Command::Transport(TransportCommand::Play));
    let live = d.ctl.bridge.render(latency + exported.frames());
    let live = live.iter().map(|c| c[latency..].to_vec()).collect();
    (exported.channels, live)
}

#[test]
fn offline_render_equals_realtime_render() {
    let mut d = Drive::new(
        RealBridge::new(OFFLINE_BLOCK),
        MemoryStore::new(),
        library(),
    );
    build_song(&mut d, true);
    tempo_ramp(&mut d);
    assert_eq!(d.ctl.project().unwrap().tempo_points.len(), 2);
    let (offline, live) = render_both(&mut d);
    // 8 beats: 6 ramping 120 → 150 bpm (ln(150/120) · 60 · 6 / 30 s), then 2 at 150 bpm.
    let seconds = (150.0f64 / 120.0).ln() * 60.0 * 6.0 / 30.0 + 2.0 * 60.0 / 150.0;
    let expected = (seconds * SR as f64).round() as usize;
    assert!(
        offline[0].len().abs_diff(expected) <= 1,
        "{} vs {expected}",
        offline[0].len()
    );
    for c in 0..2 {
        let first = live[c].iter().zip(&offline[c]).position(|(a, b)| a != b);
        assert_eq!(first, None, "channel {c} differs at frame {first:?}");
    }
}

#[test]
fn offline_render_is_block_size_independent_without_ramps() {
    let mut d = Drive::new(RealBridge::new(128), MemoryStore::new(), library());
    build_song(&mut d, false);
    let (offline, live) = render_both(&mut d);
    for c in 0..2 {
        let max = live[c]
            .iter()
            .zip(&offline[c])
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(max < 1e-6, "channel {c}: max difference {max}");
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

/// The example projects (#518, factory project templates `factory/example-*`): each one,
/// made like the welcome screen makes it, renders its loop with every track audible (the
/// drum pads, the instruments) and the mix unclipped.
#[test]
fn example_projects_render_every_track() {
    use ether_core::protocol::templates::{TemplateCommand, TemplateKind};
    let mut d = Drive::new(FakeBridge::default(), MemoryStore::new(), library());
    let listed = match d.ok(Command::Template(TemplateCommand::List {
        kind: Some(TemplateKind::Project),
    })) {
        ReplyValue::Templates { templates } => templates,
        other => panic!("{other:?}"),
    };
    let examples: Vec<_> = listed
        .into_iter()
        .filter(|t| t.factory && t.id.starts_with("factory/example-"))
        .collect();
    assert_eq!(examples.len(), 4, "{examples:?}");
    for ex in examples {
        d.pid = d.ids.next_project_id(T0);
        d.ok(Command::Template(TemplateCommand::NewProject {
            id: d.pid,
            name: ex.name.clone(),
            template: Some(ex.id.clone()),
        }));
        d.settle_media();
        let p = d.ctl.project().unwrap().clone();
        assert!(p.settings.loop_enabled, "{}", ex.id);
        let end = p.settings.loop_region.end.0;
        let tracks: Vec<TrackId> = p
            .tracks
            .values()
            .filter(|t| t.kind == TrackKind::Midi)
            .map(|t| t.id)
            .collect();
        assert!(tracks.len() >= 2, "{}", ex.id);
        let req = request(custom(0.0, end), AudioContainer::Wav, BitDepth::Float32);
        let mix = d.files(req.clone());
        let mix = decode(&d.read(&mix[0]), "wav");
        let level = peak(&mix.channels);
        println!("{:32} mix peak {:6.1} dBFS", ex.id, 20.0 * level.log10());
        assert!((0.2..1.0).contains(&level), "{}: mix peak {level}", ex.id);
        let mut stems = req;
        stems.mode = ExportMode::Stems {
            tracks: tracks.clone(),
        };
        let files = d.files(stems);
        assert_eq!(files.len(), tracks.len(), "{}", ex.id);
        for f in &files {
            let s = decode(&d.read(f), "wav");
            let level = peak(&s.channels);
            println!("  {f:40} peak {:6.1} dBFS", 20.0 * level.log10());
            assert!(level > 0.05, "{} {f}: peak {level}", ex.id);
        }
        // The drums: each pad sounds on its own (soloing the others away is runtime
        // state, so check that every pad's device is a playing instrument instead).
        for pad in p.drum_pads.values() {
            assert!(
                p.devices
                    .values()
                    .any(|dev| dev.pad == Some(pad.id) && dev.enabled),
                "{}: pad {}",
                ex.id,
                pad.name
            );
        }
    }
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

/// Cancelling an in-memory render (the static publish) mid-way: `Cancelled`, then silence; no
/// outputs (files or downloads) of it exist, and the next job's result holds only its own.
#[test]
fn cancel_mid_render_leaves_no_outputs() {
    let mut d = Drive::new(FakeBridge::default(), MemoryStore::new(), library());
    let t = d.track(TrackKind::Midi, "Keys", None);
    d.device(t, BuiltinDevice::Synth);
    d.notes(t, 0.0, &[60]);
    let mut req = request(custom(0.0, 64.0), AudioContainer::Wav, BitDepth::Int16);
    req.tail_seconds = 60.0;
    req.in_memory = Some(true);
    let job = d.render(req.clone());
    d.tick();
    let out = d.send(Command::Export(ExportCommand::Cancel { job: job.clone() }));
    assert!(events(&out).contains(&Event::Export {
        event: ExportEvent::Cancelled { job: job.clone() }
    }));
    for _ in 0..200 {
        assert!(
            events(&d.tick())
                .iter()
                .all(|e| !matches!(e, Event::Export { .. })),
            "nothing after Cancelled (no Done, no downloads)"
        );
    }
    // The next job's result is its own: a single download, nothing left of the cancelled one.
    req.tail_seconds = 0.0;
    req.range = custom(0.0, 1.0);
    let job = d.render(req);
    let ExportEvent::Done {
        result: ExportResult::Download { downloads },
        ..
    } = d.finish(&job).0
    else {
        panic!("download result")
    };
    assert_eq!(downloads.len(), 1);
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
    // MP3 has 32, 44.1 and 48 kHz only.
    let mut r = request(custom(0.0, 4.0), AudioContainer::Mp3, BitDepth::Int24);
    r.format.sample_rate = Some(96_000);
    assert_eq!(bad(&mut d, r), ErrorCode::InvalidArgument);
    // Extra formats: each container once, at most two, each valid.
    let flac = ExportFormat {
        container: AudioContainer::Flac,
        bit_depth: BitDepth::Int24,
        sample_rate: None,
    };
    let mp3 = ExportFormat {
        container: AudioContainer::Mp3,
        ..flac
    };
    let wav = ExportFormat {
        container: AudioContainer::Wav,
        ..flac
    };
    for extra in [
        vec![wav],
        vec![flac, flac],
        vec![flac, mp3, wav],
        vec![ExportFormat {
            bit_depth: BitDepth::Float32,
            ..flac
        }],
    ] {
        let mut r = base.clone();
        r.extra_formats = Some(extra);
        assert_eq!(bad(&mut d, r), ErrorCode::InvalidArgument);
    }

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
            name: None,
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

#[test]
fn ticks_stay_bounded() {
    let mut d = Drive::new(FakeBridge::default(), MemoryStore::new(), library());
    // Many devices (node creation is spread over units) and a long, heavy file.
    for i in 0..4 {
        let t = d.track(TrackKind::Midi, &format!("Keys {i}"), None);
        d.device(t, BuiltinDevice::Synth);
        for fx in [
            BuiltinDevice::Delay,
            BuiltinDevice::Compressor,
            BuiltinDevice::Eq,
            BuiltinDevice::Reverb,
        ] {
            d.device(t, fx);
        }
        d.notes(t, 0.0, &[60 + i, 64 + i]);
    }
    let t = d.track(TrackKind::Audio, "Tone", None);
    import(&mut d, t, 0.0);
    d.settle_media();
    let mut req = request(custom(0.0, 60.0), AudioContainer::Flac, BitDepth::Int16);
    req.normalize = true;
    req.format.sample_rate = Some(44_100);
    req.mode = ExportMode::Stems {
        tracks: vec![
            t,
            d.ctl
                .project()
                .unwrap()
                .tracks
                .values()
                .find(|t| t.kind == TrackKind::Midi)
                .unwrap()
                .id,
        ],
    };

    // Work units are small whatever the export length (deterministic).
    let job = d.render(req.clone());
    let files = match d.finish(&job).0 {
        ExportEvent::Done {
            result: ExportResult::Files { files },
            ..
        } => files,
        other => panic!("{other:?}"),
    };
    assert_eq!(files.len(), 2);
    let stats = d.ctl.export_unit_stats();
    assert!(stats.max_frames <= 4096, "{stats:?}");
    assert!(stats.max_devices <= 8, "{stats:?}");
    assert!(stats.max_plugins <= 1, "{stats:?}");
    assert!(stats.units > 100, "{stats:?}");

    // With the wall clock, a tick stops after its budget (12 ms) plus one small unit.
    // Timing is at the mercy of the scheduler on a loaded machine: opt in with
    // ETHER_TIMING_TESTS=1 (the unit counters above are the deterministic check).
    if std::env::var_os("ETHER_TIMING_TESTS").is_none() {
        return;
    }
    d.ctl.host.wall = true;
    let job = d.render(req);
    let mut durations = Vec::new();
    loop {
        let t0 = std::time::Instant::now();
        let out = d.tick();
        durations.push(t0.elapsed().as_secs_f64() * 1000.0);
        if events(&out).iter().any(
            |e| matches!(e, Event::Export { event: ExportEvent::Done { job: j, .. } } if *j == job),
        ) {
            break;
        }
        assert!(durations.len() < 100_000, "export finished");
    }
    durations.sort_by(f64::total_cmp);
    let p90 = durations[durations.len() * 9 / 10];
    let max = *durations.last().unwrap();
    // Generous margins: the machine may be loaded.
    assert!(p90 < 20.0, "90th percentile tick {p90:.1} ms");
    assert!(max < 150.0, "worst tick {max:.1} ms");
}

#[test]
fn exports_never_overwrite_files() {
    let mut d = Drive::new(FakeBridge::default(), MemoryStore::new(), library());
    let t = d.track(TrackKind::Audio, "Tone", None);
    import(&mut d, t, 0.0);
    d.settle_media();
    let req = request(custom(0.0, 1.0), AudioContainer::Wav, BitDepth::Int16);
    assert_eq!(d.files(req.clone()), vec!["exports/Song.wav".to_string()]);
    assert_eq!(
        d.files(req.clone()),
        vec!["exports/Song (2).wav".to_string()]
    );
    assert_eq!(d.files(req), vec!["exports/Song (3).wav".to_string()]);
}

/// A pass-through "plugin" node for offline renders.
struct Thru;
impl ether_core::Node for Thru {
    fn prepare(&mut self, _: &ether_core::PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        _: &mut ether_core::ProcessContext<'_>,
        audio: &mut ether_core::AudioBuffers<'_, '_>,
    ) -> ether_core::ProcessStatus {
        for (o, i) in audio.outputs.iter_mut().zip(audio.inputs.iter()) {
            o.copy_from_slice(i);
        }
        ether_core::ProcessStatus::Continue
    }
}

/// `FakeBridge` whose live plugin state can't be read, with offline plugins supported.
struct StatelessBridge {
    inner: FakeBridge,
    offline_states: Vec<Option<Base64Bytes>>,
}

impl EngineBridge for StatelessBridge {
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
        self.inner.poll(out)
    }
    fn descriptor(&mut self, device: DeviceId) -> Option<DeviceDescriptor> {
        self.inner.descriptor(device)
    }
    fn plugin_state(&mut self, _: DeviceId) -> Result<Option<Base64Bytes>, BridgeError> {
        Err(BridgeError::Other("plugin host is busy".into()))
    }
    fn create_offline_plugin(
        &mut self,
        _: DeviceId,
        _: &PluginInstance,
        state: Option<&Base64Bytes>,
        _: u32,
    ) -> Result<Box<dyn ether_core::Node>, BridgeError> {
        self.offline_states.push(state.cloned());
        Ok(Box::new(Thru))
    }
}

#[test]
fn unreadable_plugin_state_warns_and_uses_the_saved_state() {
    let bridge = StatelessBridge {
        inner: FakeBridge {
            plugins: Some(
                [(
                    "fx".to_string(),
                    plugin_descriptor("Fx", DeviceCategory::AudioEffect),
                )]
                .into(),
            ),
            ..Default::default()
        },
        offline_states: Vec::new(),
    };
    let mut d = Drive::new(bridge, MemoryStore::new(), library());
    let t = d.track(TrackKind::Audio, "Tone", None);
    import(&mut d, t, 0.0);
    let dev: DeviceId = d.id();
    d.ok(Command::Device(DeviceCommand::Insert {
        id: dev,
        track: t,
        device: DeviceSpec::Plugin {
            plugin_id: "fx".into(),
            sandboxed: None,
            format: None,
            name: None,
        },
        before: None,
    }));
    d.settle_media();
    let saved = d.ctl.project().unwrap().devices[&dev].kind.clone();
    let job = d.render(request(
        custom(0.0, 1.0),
        AudioContainer::Wav,
        BitDepth::Int16,
    ));
    let mut warnings = Vec::new();
    let mut done = false;
    for _ in 0..10_000 {
        for e in events(&d.tick()) {
            match e {
                Event::Notification {
                    level: NotificationLevel::Warning,
                    message,
                    ..
                } => warnings.push(message),
                Event::Export {
                    event: ExportEvent::Done { job: j, .. },
                } if j == job => done = true,
                Event::Export {
                    event: ExportEvent::Failed { message, .. },
                } => panic!("{message}"),
                _ => {}
            }
        }
        if done {
            break;
        }
    }
    assert!(done, "export finished");
    assert!(
        warnings
            .iter()
            .any(|m| m.contains("could not read the current state")
                && m.contains("last saved state")),
        "{warnings:?}"
    );
    // The offline instance got the document's (saved) state.
    let DeviceKind::Plugin { plugin } = saved else {
        panic!()
    };
    assert_eq!(d.ctl.bridge.offline_states, vec![plugin.state]);
}

/// Bouncing the mix and stems twice gives identical bytes. (Static shares published these,
/// and the manifest part is private, but the render itself is a kept feature.)
#[test]
fn bounce_mix_and_stems_is_deterministic() {
    let mut d = Drive::new(FakeBridge::default(), MemoryStore::new(), library());
    let a = d.track(TrackKind::Midi, "Lead", None);
    d.device(a, BuiltinDevice::Synth);
    d.notes(a, 0.0, &[60, 67]);
    let group = d.track(TrackKind::Group, "Bus", None);
    let b = d.track(TrackKind::Audio, "Tone", Some(group));
    import(&mut d, b, 0.0);
    let ret = d.track(TrackKind::Return, "Echo", None);
    d.device(ret, BuiltinDevice::Delay);
    d.settle_media();

    let stems = vec![a, b];
    let range = ExportRange::Project;
    let bounce = |d: &mut Drive<FakeBridge, MemoryStore>| {
        let mut req = request(range, AudioContainer::Flac, BitDepth::Int24);
        let mut files = d.files(req.clone());
        req.mode = ExportMode::Stems {
            tracks: stems.clone(),
        };
        files.extend(d.files(req));
        files.iter().map(|f| d.read(f)).collect::<Vec<_>>()
    };
    let first = bounce(&mut d);
    let second = bounce(&mut d);
    assert_eq!(first.len(), 3, "the mix and two stems");
    assert!(first.iter().all(|f| f.len() > 64));
    assert_eq!(first, second, "bouncing twice gives identical files");
    // The one-render bounce (mix + stems, what the share does now) gives the same files.
    let mut req = request(range, AudioContainer::Flac, BitDepth::Int24);
    req.mode = ExportMode::MixAndStems {
        tracks: stems.clone(),
    };
    let one: Vec<Vec<u8>> = d.files(req).iter().map(|f| d.read(f)).collect();
    assert_eq!(one, first, "one render gives the same mix and stems");
}

/// `in_memory` keeps the results as downloads even when the store could write them (static
/// sharing uploads the bytes; nothing lands in the project folder).
#[test]
fn in_memory_exports_are_downloads_on_a_writing_store() {
    let mut d = Drive::new(FakeBridge::default(), MemoryStore::new(), library());
    let t = d.track(TrackKind::Audio, "Tone", None);
    import(&mut d, t, 0.0);
    d.settle_media();
    let mut req = request(custom(0.0, 2.0), AudioContainer::Flac, BitDepth::Int24);
    req.in_memory = Some(true);
    let job = d.render(req);
    let ExportEvent::Done {
        result: ExportResult::Download { downloads },
        ..
    } = d.finish(&job).0
    else {
        panic!("download result")
    };
    assert_eq!(downloads.len(), 1);
    assert_eq!(downloads[0].mime, "audio/flac");
    assert!(
        d.ctl
            .store
            .list_dir(d.pid, "exports")
            .map_or(true, |l| l.entries.is_empty()),
        "nothing was written to the project"
    );
}

/// Frames an MP3 export decodes to after the gapless trim (`export::encode::mp3_trimmed_frames`:
/// the signal rounded up so that it and the 528 + 529 samples of codec delay fill whole
/// 1152-sample frames).
fn mp3_trimmed(frames: usize) -> usize {
    (frames + 1057).div_ceil(1152) * 1152 - 1057
}

/// The shift of `b` against `a` (within `±max` frames) with the least squared difference.
fn best_shift(a: &[f32], b: &[f32], max: isize) -> isize {
    let n = a.len().min(b.len()) as isize;
    (-max..=max)
        .min_by(|&x, &y| {
            let err = |s: isize| -> f64 {
                (max..n - max)
                    .map(|i| (a[i as usize] as f64 - b[(i + s) as usize] as f64).powi(2))
                    .sum()
            };
            err(x).total_cmp(&err(y))
        })
        .unwrap()
}

/// What static sharing renders (docs/SHARING.md §14): the mix and stems as MP3 for
/// listening, the same render as FLAC for downloads (`extra_formats`). The MP3s decode to
/// the FLAC's length rounded up to the codec frame grid, after a lead the listener page
/// skips (`decoded - trimmed`, the same for every file), and then the mix, the stems and
/// their sum line up sample for sample.
#[test]
fn lossy_share_files_stay_aligned_with_the_lossless_render() {
    let mut d = Drive::new(FakeBridge::default(), MemoryStore::new(), library());
    let a = d.track(TrackKind::Midi, "Lead", None);
    d.device(a, BuiltinDevice::Synth);
    d.notes(a, 0.0, &[60, 67]);
    let group = d.track(TrackKind::Group, "Bus", None);
    let b = d.track(TrackKind::Audio, "Tone", Some(group));
    import(&mut d, b, 1.0);
    d.settle_media();

    let mut req = request(custom(0.0, 4.0), AudioContainer::Mp3, BitDepth::Int24);
    req.tail_seconds = 0.5;
    req.mode = ExportMode::MixAndStems { tracks: vec![a, b] };
    req.extra_formats = Some(vec![ExportFormat {
        container: AudioContainer::Flac,
        bit_depth: BitDepth::Int24,
        sample_rate: None,
    }]);
    let files = d.files(req);
    assert_eq!(
        files,
        [
            "exports/Song.mp3",
            "exports/Song - Lead.mp3",
            "exports/Song - Tone.mp3",
            "exports/Song.flac",
            "exports/Song - Lead.flac",
            "exports/Song - Tone.flac",
        ],
        "the MP3s in request order, then the FLACs"
    );
    let flac: Vec<DecodedAudio> = files[3..]
        .iter()
        .map(|f| decode(&d.read(f), "flac"))
        .collect();
    let n = flac[0].frames();
    assert_eq!(n, (2.5 * SR as f64) as usize);
    let mp3: Vec<DecodedAudio> = files[..3]
        .iter()
        .map(|f| {
            let bytes = d.read(f);
            // 320 kbps CBR.
            let kbps = bytes.len() as f64 * 8.0 / (n as f64 / SR as f64) / 1000.0;
            assert!((320.0..340.0).contains(&kbps), "{f}: {kbps} kbps");
            decode(&bytes, "mp3")
        })
        .collect();
    let lead = mp3[0].frames() - mp3_trimmed(n);
    // Whatever this decoder trims, the extra samples are the codec delay at the start.
    assert!(lead <= 1152 + 1057, "lead {lead}");
    for m in &mp3 {
        assert_eq!(m.sample_rate, SR);
        assert_eq!(
            m.frames() - mp3_trimmed(n),
            lead,
            "the same lead in every file"
        );
    }
    let listen = |m: &DecodedAudio, c: usize| m.channels[c][lead..lead + n].to_vec();
    for c in 0..2 {
        // Each MP3 lines up with its FLAC.
        for (m, f) in mp3.iter().zip(&flac) {
            assert_eq!(best_shift(&f.channels[c], &listen(m, c), 16), 0);
        }
        // The summed MP3 stems line up with the MP3 mix, and sum close to it.
        let mix = listen(&mp3[0], c);
        let sum: Vec<f32> = (0..n)
            .map(|i| listen(&mp3[1], c)[i] + listen(&mp3[2], c)[i])
            .collect();
        assert_eq!(best_shift(&mix, &sum, 16), 0, "stems in sync with the mix");
        let (sig, err) = mix
            .iter()
            .zip(&sum)
            .fold((0.0f64, 0.0f64), |(s, e), (x, y)| {
                (s + (*x as f64).powi(2), e + (*x as f64 - *y as f64).powi(2))
            });
        let snr = 10.0 * (sig / err).log10();
        assert!(
            snr > 20.0,
            "stems sum to the mix within the codec noise: {snr:.1} dB"
        );
    }
}

/// #434: files streamed into the store (`create_export`) are the files encoded in memory
/// (downloads, and stores with only `write_export`), byte for byte, for every container,
/// resampled or not, normalized or not, mix and stems.
#[test]
fn streamed_files_equal_in_memory_files() {
    fn bytes<S: ProjectStore>(d: &mut Drive<FakeBridge, S>, req: ExportRequest) -> Vec<Vec<u8>> {
        let job = d.render(req);
        match d.finish(&job).0 {
            ExportEvent::Done {
                result: ExportResult::Files { files },
                ..
            } => files.iter().map(|f| d.read(f)).collect(),
            ExportEvent::Done {
                result: ExportResult::Download { downloads },
                ..
            } => downloads
                .iter()
                .map(|dl| {
                    let mut bytes = Vec::new();
                    loop {
                        let ReplyValue::Bytes { chunk } =
                            d.ok(Command::Export(ExportCommand::ReadChunk {
                                token: dl.token.clone(),
                                offset: bytes.len() as f64,
                                length: 1 << 20,
                            }))
                        else {
                            panic!("bytes")
                        };
                        bytes.extend_from_slice(&chunk.data.0);
                        if chunk.eof {
                            break;
                        }
                    }
                    bytes
                })
                .collect(),
            other => panic!("{other:?}"),
        }
    }
    let mut d = Drive::new(FakeBridge::default(), MemoryStore::new(), library());
    let a = d.track(TrackKind::Midi, "Lead", None);
    d.device(a, BuiltinDevice::Synth);
    d.notes(a, 0.0, &[60, 67]);
    let b = d.track(TrackKind::Audio, "Tone", None);
    import(&mut d, b, 0.5);
    d.settle_media();
    let fmt = |container, bit_depth, sample_rate| ExportFormat {
        container,
        bit_depth,
        sample_rate,
    };
    let mut req = request(custom(0.0, 6.0), AudioContainer::Wav, BitDepth::Int16);
    req.tail_seconds = 0.3;
    req.mode = ExportMode::MixAndStems { tracks: vec![a, b] };
    req.extra_formats = Some(vec![
        fmt(AudioContainer::Flac, BitDepth::Int24, Some(44_100)),
        fmt(AudioContainer::Mp3, BitDepth::Int16, None),
    ]);
    for normalize in [false, true] {
        req.normalize = normalize;
        let streamed = bytes(&mut d, req.clone());
        assert_eq!(streamed.len(), 9, "three files in three formats");
        let mut in_memory = req.clone();
        in_memory.in_memory = Some(true);
        assert!(
            bytes(&mut d, in_memory) == streamed,
            "normalize {normalize}"
        );
    }
}

/// #562: a document claiming far more frames than the media holds (hand-edited file,
/// imported bundle, peer snapshot) exports like it plays: the source is padded by at most
/// `MAX_PAD_FRAMES` (it used to allocate the claimed length and abort).
#[test]
fn export_bounds_the_document_frame_count() {
    for claimed in [u64::MAX, 1_000_000_000_000] {
        let mut d = Drive::new(FakeBridge::default(), MemoryStore::new(), library());
        let t = d.track(TrackKind::Audio, "Tone", None);
        import(&mut d, t, 0.0);
        d.settle_media();
        let mut p = d.ctl.project().unwrap().clone();
        for m in p.media.values_mut() {
            m.frames = claimed;
        }
        let song = d.pid;
        d.new_project("Scratch");
        let json = ether_core::protocol::model::file::save(&p, "test").unwrap();
        d.ctl.store.save(song, &json).unwrap();
        d.pid = song;
        d.ok(Command::Project(ProjectCommand::Open { id: song }));
        d.settle_media();
        assert_eq!(
            d.ctl
                .project()
                .unwrap()
                .media
                .values()
                .next()
                .unwrap()
                .frames,
            claimed
        );

        let files = d.files(request(
            custom(0.0, 2.0),
            AudioContainer::Wav,
            BitDepth::Float32,
        ));
        let got = decode(&d.read(&files[0]), "wav");
        assert_eq!(got.frames(), SR as usize);
        assert!(peak(&got.channels) > 0.1, "the tone renders");
    }
}

//! Freeze, flatten, bounce and consolidate (CONTRACTS.md §12.3), against a
//! real engine behind the bridge: a frozen track sounds like the live one (also under a
//! tempo ramp), its devices get no nodes, it can't be edited except its mixer, unfreeze and
//! undo restore it, flatten/bounce/consolidate produce clips that play the render, and
//! the job protocol (progress, one job at a time, cancel, project closed).

use crate::common;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use common::*;
use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::store::ProjectStore;
use ether_controller::{
    BridgeError, Controller, ControllerConfig, EngineBridge, EtherController, HostServices,
};
use ether_core::protocol::automation::{AutomationCommand, PointSpec};
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::devices::{DeviceCategory, DeviceCommand, DeviceDescriptor, DeviceSpec};
use ether_core::protocol::freeze::{BounceTarget, FreezeCommand, FreezeEvent};
use ether_core::protocol::media::{BrowseLocation, MediaCommand, MediaSource};
use ether_core::protocol::mixer::MixerCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteSpec};
use ether_core::protocol::project::{EditCommand, ProjectCommand};
use ether_core::protocol::racks::RackCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::transport::TransportCommand;
use ether_core::protocol::*;
use ether_core::{
    AudioSource, Engine, EngineConfig, EngineHandle, EngineOutputs, GarbageCollector, NodeKey,
    ParamChange, RenderGraphDesc, TransportControl,
};
use ether_media::{DecodedAudio, InMemorySource};

const SR: u32 = 48_000;
const BLOCK: usize = 256;

// ─── A real engine behind the bridge ─────────────────────────────────────────────────────

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
    /// Live device nodes (created minus destroyed).
    nodes: BTreeMap<NodeKey, DeviceId>,
    /// The last published graph.
    last_graph: Option<RenderGraphDesc>,
    /// Plugin devices: all [`Loading`] instruments.
    plugins: BTreeSet<DeviceId>,
}

/// Fake plugin instrument (`plugin_id: "loading"`): a held note plays a constant 0.5 on both
/// sides, but only once [`LOADING_MS`] of real time have passed since the instance was
/// created; before that it is silent, like a sample player still loading its sound on a
/// thread of its own after its state was set.
struct Loading {
    since: std::time::Instant,
    held: bool,
}
const LOADING_MS: u64 = 100;
impl ether_core::Node for Loading {
    fn prepare(&mut self, _: &ether_core::PrepareConfig) {}
    fn reset(&mut self) {
        self.held = false;
    }
    fn channels(&self) -> (u16, u16) {
        (0, 2)
    }
    fn process(
        &mut self,
        ctx: &mut ether_core::ProcessContext<'_>,
        audio: &mut ether_core::AudioBuffers<'_, '_>,
    ) -> ether_core::ProcessStatus {
        let ready = self.since.elapsed() >= std::time::Duration::from_millis(LOADING_MS);
        let mut events = ctx.events.iter().peekable();
        for i in 0..ctx.frames {
            while let Some(e) = events.next_if(|e| e.offset as usize <= i) {
                match e.kind {
                    ether_core::EventKind::NoteOn { .. } => self.held = true,
                    ether_core::EventKind::NoteOff { .. } | ether_core::EventKind::AllNotesOff => {
                        self.held = false
                    }
                    _ => {}
                }
            }
            let v = if self.held && ready { 0.5 } else { 0.0 };
            for o in audio.outputs.iter_mut() {
                o[i] = v;
            }
        }
        ether_core::ProcessStatus::Continue
    }
}

fn loading_descriptor() -> DeviceDescriptor {
    let mut d = plugin_descriptor("Loading", DeviceCategory::Instrument);
    d.audio_inputs = 0;
    d.midi_input = true;
    d
}

fn loading_node() -> Box<dyn ether_core::Node> {
    Box::new(Loading {
        since: std::time::Instant::now(),
        held: false,
    })
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
            nodes: BTreeMap::new(),
            last_graph: None,
            plugins: BTreeSet::new(),
        }
    }

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
        let key = self
            .handle
            .add_node(node)
            .map_err(|e| BridgeError::Other(e.to_string()))?;
        self.nodes.insert(key, device);
        Ok(key)
    }
    fn create_plugin(
        &mut self,
        device: DeviceId,
        plugin: &PluginInstance,
        _: Option<&Base64Bytes>,
    ) -> Result<NodeKey, BridgeError> {
        if plugin.plugin_id != "loading" {
            return Err(BridgeError::Unsupported("no plugins".into()));
        }
        self.plugins.insert(device);
        let key = self
            .handle
            .add_node(loading_node())
            .map_err(|e| BridgeError::Other(e.to_string()))?;
        self.nodes.insert(key, device);
        Ok(key)
    }
    fn create_offline_plugin(
        &mut self,
        _: DeviceId,
        _: &PluginInstance,
        _: Option<&Base64Bytes>,
        _: u32,
    ) -> Result<Box<dyn ether_core::Node>, BridgeError> {
        Ok(loading_node())
    }
    fn destroy_node(&mut self, key: NodeKey) -> Result<(), BridgeError> {
        self.nodes.remove(&key);
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
        self.last_graph = Some(graph.clone());
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
        if self.plugins.contains(&device) {
            return Some(loading_descriptor());
        }
        self.devices
            .get(&device)
            .map(|t| ether_devices::descriptor(*t))
    }
}

// ─── Driver ───────────────────────────────────────────────────────────────────────────────

struct TestHost {
    now: u64,
}

impl HostServices for TestHost {
    fn now_ms(&self) -> u64 {
        self.now
    }
    fn random_seed(&mut self) -> u64 {
        0x5eed
    }
}

struct Drive {
    ctl: EtherController<RealBridge, TestHost, MemoryStore, MemoryLibrary>,
    ids: IdGen,
    next: u32,
    pid: ProjectId,
}

impl Drive {
    fn new() -> Self {
        let mut lib = MemoryLibrary::new();
        lib.add_root("lib", "Library");
        let l = sine(SR, 330.0, SR as usize * 2, 0.4);
        let r = sine(SR, 165.0, SR as usize * 2, 0.3);
        lib.add_file("lib", "tone.wav", wav(SR, &[l, r]));
        let mut store = MemoryStore::new();
        let _ = store.list();
        let mut d = Self {
            ctl: EtherController::with_config(
                RealBridge::new(),
                TestHost { now: T0 },
                store,
                lib,
                ControllerConfig {
                    engine_sample_rate: SR,
                    ..Default::default()
                },
            ),
            ids: IdGen::new(7),
            next: 1,
            pid: IdGen::new(1).next_project_id(T0),
        };
        d.pid = d.ids.next_project_id(T0);
        d.ok(Command::Project(ProjectCommand::Create {
            id: d.pid,
            name: "Song".into(),
        }));
        d
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

    fn err(&mut self, command: Command) -> CommandError {
        err(&self.send(command))
    }

    fn tick(&mut self) -> Vec<ServerMessage> {
        let mut out = Vec::new();
        self.ctl.host.now += 16;
        let now = self.ctl.host.now;
        self.ctl.tick(now, &mut out);
        out
    }

    fn settle(&mut self) {
        for _ in 0..10_000 {
            self.tick();
            if !self.ctl.media_pending() {
                self.tick();
                return;
            }
        }
        panic!("media did not load");
    }

    fn p(&self) -> &Project {
        self.ctl.project().unwrap()
    }

    fn track(&mut self, kind: TrackKind, name: &str) -> TrackId {
        let id = self.id();
        self.ok(Command::Track(TrackCommand::Create {
            id,
            kind,
            name: Some(name.into()),
            color: None,
            parent: None,
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

    fn midi_clip(&mut self, track: TrackId, start: f64, pitches: &[u8]) -> ClipId {
        let clip = self.id();
        self.ok(Command::Clip(ClipCommand::CreateMidi {
            id: clip,
            track,
            start: Beats(start),
            length: Beats(4.0),
            name: None,
        }));
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
        clip
    }

    fn audio_clip(&mut self, track: TrackId, at: f64) -> ClipId {
        let media: MediaId = self.id();
        self.ok(Command::Media(MediaCommand::Import {
            id: media,
            source: MediaSource::Location {
                location: BrowseLocation::Library { id: "lib".into() },
                path: "tone.wav".into(),
            },
        }));
        let clip = self.id();
        self.ok(Command::Clip(ClipCommand::CreateAudio {
            id: clip,
            track,
            start: Beats(at),
            media,
        }));
        clip
    }

    /// Run a render job to its end; returns the final event and the progress seen.
    fn finish(&mut self, job: &str) -> (FreezeEvent, Vec<f32>) {
        let mut progress = Vec::new();
        for _ in 0..100_000 {
            for e in events(&self.tick()) {
                if let Event::Freeze { event } = e {
                    match &event {
                        FreezeEvent::Progress {
                            job: j,
                            progress: p,
                            ..
                        } => {
                            assert_eq!(j, job);
                            progress.push(*p);
                        }
                        _ => return (event, progress),
                    }
                }
            }
        }
        panic!("render did not finish");
    }

    fn freeze(&mut self, track: TrackId) -> MediaId {
        let media: MediaId = self.id();
        let job = format!("freeze{}", self.next);
        assert_eq!(
            self.ok(Command::Freeze(FreezeCommand::Freeze {
                job: job.clone(),
                track,
                media,
                up_to: None,
            })),
            ReplyValue::RenderStarted { job: job.clone() }
        );
        let (end, progress) = self.finish(&job);
        assert_eq!(end, FreezeEvent::Done { job });
        assert_eq!(progress.last(), Some(&1.0));
        assert!(progress.windows(2).all(|w| w[0] <= w[1]), "{progress:?}");
        self.settle();
        media
    }

    /// Play `frames` of the live engine from beat 0.
    fn play(&mut self, frames: usize) -> [Vec<f32>; 2] {
        self.ok(Command::Transport(TransportCommand::Stop));
        self.ok(Command::Transport(TransportCommand::Locate {
            position: Beats(0.0),
        }));
        self.tick();
        self.ok(Command::Transport(TransportCommand::Play));
        let out = self.ctl.bridge.render(frames);
        self.ok(Command::Transport(TransportCommand::Stop));
        self.tick();
        out
    }

    fn master(&self) -> TrackId {
        self.p()
            .tracks
            .values()
            .find(|t| t.kind == TrackKind::Master)
            .unwrap()
            .id
    }
}

fn peak(ch: &[Vec<f32>]) -> f32 {
    ch.iter().flatten().fold(0.0, |m, s| m.max(s.abs()))
}

fn max_diff(a: &[Vec<f32>; 2], b: &[Vec<f32>; 2]) -> f32 {
    (0..2)
        .map(|c| {
            a[c].iter()
                .zip(&b[c])
                .map(|(x, y)| (x - y).abs())
                .fold(0.0f32, f32::max)
        })
        .fold(0.0, f32::max)
}

/// `max_diff` after the first `skip` frames (the anti-click ramp at a clip start).
fn max_diff_from(a: &[Vec<f32>; 2], b: &[Vec<f32>; 2], skip: usize) -> f32 {
    let cut = |x: &[Vec<f32>; 2]| [x[0][skip..].to_vec(), x[1][skip..].to_vec()];
    max_diff(&cut(a), &cut(b))
}

/// A synth track with a delay, a fader and pan away from unity, two clips.
fn keys(d: &mut Drive) -> (TrackId, DeviceId) {
    let t = d.track(TrackKind::Midi, "Keys");
    d.device(t, BuiltinDevice::Synth);
    let delay = d.device(t, BuiltinDevice::Delay);
    d.midi_clip(t, 0.0, &[60, 64, 67, 72]);
    d.midi_clip(t, 4.0, &[62, 65, 69]);
    d.ok(Command::Mixer(MixerCommand::SetVolume {
        track: t,
        volume: Decibels(-6.0),
    }));
    d.ok(Command::Mixer(MixerCommand::SetPan {
        track: t,
        pan: Pan(0.3),
    }));
    d.settle();
    (t, delay)
}

const FRAMES: usize = SR as usize * 5;

#[test]
fn frozen_track_sounds_like_the_live_track() {
    let mut d = Drive::new();
    let (t, _) = keys(&mut d);
    let live = d.play(FRAMES);
    assert!(peak(&live) > 0.01, "audible");
    let nodes_before = d.ctl.bridge.nodes.len();
    let media = d.freeze(t);
    // The render is project media, the devices have no nodes, the graph plays the render.
    let f = d.p().tracks[&t].freeze.clone().expect("frozen");
    assert_eq!(f.media, media);
    assert_eq!(f.start, Seconds(0.0));
    let m = d.p().media[&media].clone();
    assert!(m.file.starts_with("media/"), "{}", m.file);
    assert_eq!(m.location, MediaLocation::Project);
    assert_eq!(m.sample_rate, SR);
    // 8 beats at 120 bpm (4 s) plus the delay's tail, trimmed at -90 dB.
    assert!(m.frames as usize > 4 * SR as usize, "{}", m.frames);
    assert!(m.frames as usize <= (4.0 + 30.5) as usize * SR as usize);
    assert_eq!(d.ctl.bridge.nodes.len(), nodes_before - 2);
    assert!(d.ctl.store.read(d.pid, &m.file).is_ok());
    let frozen = d.play(FRAMES);
    let diff = max_diff(&live, &frozen);
    assert!(diff < 1e-4, "frozen playback differs by {diff}");

    // Mixer controls stay live on a frozen track.
    d.ok(Command::Mixer(MixerCommand::SetVolume {
        track: t,
        volume: Decibels(-12.0),
    }));
    let quieter = d.play(FRAMES);
    let ratio = peak(&quieter) / peak(&frozen);
    assert!((ratio - 0.5).abs() < 0.02, "{ratio}");

    // Unfreeze: the devices come back.
    d.ok(Command::Freeze(FreezeCommand::Unfreeze { track: t }));
    d.tick();
    assert!(d.p().tracks[&t].freeze.is_none());
    assert_eq!(d.ctl.bridge.nodes.len(), nodes_before);
}

#[test]
fn freeze_plays_at_song_time_under_a_tempo_ramp() {
    let mut d = Drive::new();
    let (t, _) = keys(&mut d);
    // 120 → 150 bpm ramp over beats 0..6.
    let mut p = d.p().clone();
    for tp in p.tempo_points.values_mut() {
        tp.curve = TempoCurve::Linear;
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
    let song = d.pid;
    let scratch = d.ids.next_project_id(T0);
    d.ok(Command::Project(ProjectCommand::Create {
        id: scratch,
        name: "Scratch".into(),
    }));
    let json = ether_core::protocol::model::file::save(&p, "test").unwrap();
    d.ctl.store.save(song, &json).unwrap();
    d.ok(Command::Project(ProjectCommand::Open { id: song }));
    d.settle();
    let live = d.play(FRAMES);
    d.freeze(t);
    let frozen = d.play(FRAMES);
    let diff = max_diff(&live, &frozen);
    assert!(diff < 1e-4, "frozen playback differs by {diff}");
}

#[test]
fn frozen_tracks_reject_content_edits_but_not_mixer_edits() {
    let mut d = Drive::new();
    let (t, delay) = keys(&mut d);
    let clip = d.p().clips.values().find(|c| c.track == t).unwrap().id;
    let other = d.track(TrackKind::Midi, "Other");
    let other_clip = d.midi_clip(other, 0.0, &[60]);
    d.freeze(t);
    let rejected = [
        Command::Note(NoteCommand::Add {
            clip,
            notes: vec![NoteSpec {
                id: d.id(),
                pitch: 70,
                velocity: 1.0,
                start: Beats(0.0),
                duration: Beats(1.0),
            }],
        }),
        Command::Clip(ClipCommand::Delete { ids: vec![clip] }),
        Command::Clip(ClipCommand::CreateMidi {
            id: d.id(),
            track: t,
            start: Beats(8.0),
            length: Beats(1.0),
            name: None,
        }),
        Command::Clip(ClipCommand::Move {
            moves: vec![ether_core::protocol::clips::ClipMove {
                id: other_clip,
                track: t,
                start: Beats(12.0),
            }],
        }),
        Command::Device(DeviceCommand::SetParam {
            device: delay,
            param: ether_devices::descriptor(BuiltinDeviceType::Delay).params[0].id,
            value: 0.5,
        }),
        Command::Device(DeviceCommand::Remove { id: delay }),
        Command::Device(DeviceCommand::Insert {
            id: d.id(),
            track: t,
            device: DeviceSpec::Builtin {
                device: BuiltinDevice::Reverb,
            },
            before: None,
        }),
        Command::Automation(AutomationCommand::CreateLane {
            id: d.id(),
            owner: AutomationOwner::Track { track: t },
            target: AutomationTarget::DeviceParam {
                device: delay,
                param: ParamId(0),
            },
        }),
    ];
    for c in rejected {
        let before = d.p().clone();
        let e = d.err(c.clone());
        assert_eq!(e.code, ErrorCode::InvalidState, "{c:?}");
        assert!(e.message.contains("frozen"), "{}", e.message);
        assert_eq!(d.p(), &before);
    }
    // Mixer, name, color, volume automation and other tracks stay editable.
    let lane: AutomationLaneId = d.id();
    for c in [
        Command::Mixer(MixerCommand::SetMute {
            track: t,
            mute: true,
        }),
        Command::Track(TrackCommand::Rename {
            id: t,
            name: "Frozen keys".into(),
        }),
        Command::Automation(AutomationCommand::CreateLane {
            id: lane,
            owner: AutomationOwner::Track { track: t },
            target: AutomationTarget::TrackVolume { track: t },
        }),
        Command::Automation(AutomationCommand::AddPoints {
            lane,
            points: vec![PointSpec {
                id: d.id(),
                time: Beats(0.0),
                value: 0.5,
                curve: CurveShape::Linear,
            }],
        }),
        Command::Note(NoteCommand::Add {
            clip: other_clip,
            notes: vec![NoteSpec {
                id: d.id(),
                pitch: 72,
                velocity: 1.0,
                start: Beats(1.0),
                duration: Beats(1.0),
            }],
        }),
    ] {
        d.ok(c);
    }
    // Unfreezing makes it editable again.
    d.ok(Command::Freeze(FreezeCommand::Unfreeze { track: t }));
    d.ok(Command::Device(DeviceCommand::Remove { id: delay }));
}

#[test]
fn undo_restores_the_live_track() {
    let mut d = Drive::new();
    let (t, _) = keys(&mut d);
    let nodes = d.ctl.bridge.nodes.len();
    let media = d.freeze(t);
    d.ok(Command::Edit(EditCommand::Undo));
    d.tick();
    assert!(d.p().tracks[&t].freeze.is_none());
    assert!(!d.p().media.contains_key(&media));
    assert_eq!(d.ctl.bridge.nodes.len(), nodes);
    d.ok(Command::Edit(EditCommand::Redo));
    d.tick();
    assert!(d.p().tracks[&t].freeze.is_some());
}

#[test]
fn flatten_replaces_the_track_with_its_render() {
    let mut d = Drive::new();
    let (t, _) = keys(&mut d);
    let ret = d.track(TrackKind::Return, "Verb");
    let send: SendId = d.id();
    d.ok(Command::Mixer(MixerCommand::CreateSend {
        id: send,
        from: t,
        to: ret,
        level: Decibels(-10.0),
        pre_fader: false,
    }));
    d.settle();
    let live = d.play(FRAMES);
    d.freeze(t);
    let position = |p: &Project, id: TrackId| p.tracks_ordered().iter().position(|x| x.id == id);
    let index = position(d.p(), t);
    let (clip, new_track): (ClipId, TrackId) = (d.id(), d.id());
    d.ok(Command::Freeze(FreezeCommand::Flatten {
        track: t,
        clip,
        new_track,
    }));
    d.settle();
    // The MIDI track became an audio track at its place, with its mixer and sends.
    assert!(!d.p().tracks.contains_key(&t));
    let nt = d.p().tracks[&new_track].clone();
    assert_eq!(nt.kind, TrackKind::Audio);
    assert_eq!(nt.name, "Keys");
    assert_eq!(nt.mixer.volume, Decibels(-6.0));
    assert_eq!(
        position(d.p(), new_track),
        index,
        "takes the MIDI track's place"
    );
    let sends: Vec<&TrackSend> = d
        .p()
        .sends
        .values()
        .filter(|s| s.from == new_track)
        .collect();
    assert_eq!(sends.len(), 1);
    assert_eq!(sends[0].id, ether_model::derive_id(new_track, 0));
    assert_eq!(sends[0].to, ret);
    let c = &d.p().clips[&clip];
    assert_eq!(c.track, new_track);
    assert_eq!(c.start, Beats(0.0));
    let ClipContent::Audio(a) = &c.content else {
        panic!("audio clip")
    };
    assert!(a.warp.enabled);
    assert_eq!(
        d.p()
            .devices
            .values()
            .filter(|x| x.track == new_track)
            .count(),
        0
    );
    let flat = d.play(FRAMES);
    // Rendered clips get the engine's 64-sample anti-click ramp at their start.
    let diff = max_diff_from(&live, &flat, 64);
    assert!(diff < 1e-3, "flattened playback differs by {diff}");
    // One undo step brings the MIDI track back.
    d.ok(Command::Edit(EditCommand::Undo));
    assert!(d.p().tracks.contains_key(&t));
    assert!(!d.p().tracks.contains_key(&new_track));
}

#[test]
fn flatten_an_audio_track_keeps_the_track() {
    let mut d = Drive::new();
    let t = d.track(TrackKind::Audio, "Tone");
    d.device(t, BuiltinDevice::Compressor);
    d.audio_clip(t, 1.0);
    d.settle();
    let live = d.play(FRAMES);
    d.freeze(t);
    let clip: ClipId = d.id();
    let unused: TrackId = d.id();
    d.ok(Command::Freeze(FreezeCommand::Flatten {
        track: t,
        clip,
        new_track: unused,
    }));
    d.settle();
    let clips: Vec<&Clip> = d.p().clips.values().filter(|c| c.track == t).collect();
    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0].id, clip);
    assert!(d.p().tracks[&t].freeze.is_none());
    assert_eq!(d.p().devices.values().filter(|x| x.track == t).count(), 0);
    let flat = d.play(FRAMES);
    // Rendered clips get the engine's 64-sample anti-click ramp at their start.
    let diff = max_diff_from(&live, &flat, 64);
    assert!(diff < 1e-3, "flattened playback differs by {diff}");
    // Flatten needs a frozen track.
    let clip2: ClipId = d.id();
    let e = d.err(Command::Freeze(FreezeCommand::Flatten {
        track: t,
        clip: clip2,
        new_track: unused,
    }));
    assert_eq!(e.code, ErrorCode::InvalidState);
}

#[test]
fn bounce_to_a_new_track_and_in_place() {
    let mut d = Drive::new();
    let (t, _) = keys(&mut d);
    let live = d.play(FRAMES);
    // New track (post-chain): the source is muted, the bounce sounds like it (pre-fader, so
    // the new track's unity fader and centre pan differ: compare with the fader copied).
    let (media, nt, clip): (MediaId, TrackId, ClipId) = (d.id(), d.id(), d.id());
    let job = "b1".to_string();
    d.ok(Command::Freeze(FreezeCommand::Bounce {
        job: job.clone(),
        track: t,
        start: Beats(0.0),
        end: Beats(8.0),
        include_chain: true,
        media,
        target: BounceTarget::NewTrack { track: nt, clip },
    }));
    assert_eq!(d.finish(&job).0, FreezeEvent::Done { job });
    d.settle();
    assert!(d.p().tracks[&t].mixer.mute);
    assert_eq!(d.p().tracks[&nt].kind, TrackKind::Audio);
    assert_eq!(d.p().tracks[&nt].name, "Keys Bounce");
    assert_eq!(d.p().clips[&clip].track, nt);
    // The range, plus the decay of the synth and delay.
    assert!(d.p().media[&media].frames > 4 * SR as u64);
    d.ok(Command::Mixer(MixerCommand::SetVolume {
        track: nt,
        volume: Decibels(-6.0),
    }));
    d.ok(Command::Mixer(MixerCommand::SetPan {
        track: nt,
        pan: Pan(0.3),
    }));
    // Let the fader and pan smoothing settle first.
    d.play(SR as usize / 10);
    let bounced = d.play(4 * SR as usize);
    // Anti-click ramps at both clip edges.
    let n = 4 * SR as usize - 64;
    let live4 = [live[0][..n].to_vec(), live[1][..n].to_vec()];
    let bounced = [bounced[0][..n].to_vec(), bounced[1][..n].to_vec()];
    let diff = max_diff_from(&live4, &bounced, 64);
    assert!(diff < 1e-3, "bounce differs by {diff}");

    // In place (audio, pre-chain): the clips in the range are replaced.
    let a = d.track(TrackKind::Audio, "Tone");
    d.device(a, BuiltinDevice::Compressor);
    let original = d.audio_clip(a, 0.0);
    d.settle();
    let (media, clip): (MediaId, ClipId) = (d.id(), d.id());
    let e = d.err(Command::Freeze(FreezeCommand::Bounce {
        job: "b2".into(),
        track: a,
        start: Beats(1.0),
        end: Beats(3.0),
        include_chain: true,
        media,
        target: BounceTarget::InPlace { clip },
    }));
    assert_eq!(e.code, ErrorCode::InvalidArgument);
    d.ok(Command::Freeze(FreezeCommand::Bounce {
        job: "b3".into(),
        track: a,
        start: Beats(1.0),
        end: Beats(3.0),
        include_chain: false,
        media,
        target: BounceTarget::InPlace { clip },
    }));
    assert!(matches!(d.finish("b3").0, FreezeEvent::Done { .. }));
    let clips: Vec<(f64, f64)> = {
        let mut v: Vec<(f64, f64)> = d
            .p()
            .clips
            .values()
            .filter(|c| c.track == a)
            .map(|c| (c.start.0, c.length.0))
            .collect();
        v.sort_by(|x, y| x.0.total_cmp(&y.0));
        v
    };
    assert_eq!(clips[0], (0.0, 1.0));
    assert_eq!(clips[1], (1.0, 2.0));
    assert!((clips[2].0 - 3.0).abs() < 1e-9);
    assert_eq!(d.p().clips[&original].length, Beats(1.0));
}

/// Bounce to a new track keeps the chain's tail (instrument release, reverb) past the range,
/// but nothing after the range: the clip is longer than the source and ends in silence.
#[test]
fn bounce_to_a_new_track_keeps_the_tail() {
    let mut d = Drive::new();
    let t = d.track(TrackKind::Midi, "Pad");
    d.device(t, BuiltinDevice::Synth);
    d.device(t, BuiltinDevice::Reverb);
    d.midi_clip(t, 0.0, &[60, 64, 67]);
    // A later clip: cut out of the render, so it never leaks into the tail.
    d.midi_clip(t, 4.0, &[72, 76, 79]);
    d.settle();
    let (media, nt, clip): (MediaId, TrackId, ClipId) = (d.id(), d.id(), d.id());
    d.ok(Command::Freeze(FreezeCommand::Bounce {
        job: "tail".into(),
        track: t,
        start: Beats(0.0),
        end: Beats(4.0),
        include_chain: true,
        media,
        target: BounceTarget::NewTrack { track: nt, clip },
    }));
    assert!(matches!(d.finish("tail").0, FreezeEvent::Done { .. }));
    d.settle();
    let frames = d.p().media[&media].frames as usize;
    let range = 2 * SR as usize;
    assert!(
        frames > range + SR as usize / 10,
        "no tail: {frames} frames"
    );
    assert!(d.p().clips[&clip].length.0 > 4.0);
    // The source is muted: the output is the bounce alone (once the mute ramp has run).
    d.play(SR as usize / 10);
    let out = d.play(frames + SR as usize / 2);
    let tail = [
        out[0][range..range + SR as usize / 10].to_vec(),
        out[1][range..range + SR as usize / 10].to_vec(),
    ];
    assert!(peak(&tail) > 1e-3, "the tail is audible after the range");
    let end = [
        out[0][frames - 512..frames].to_vec(),
        out[1][frames - 512..frames].to_vec(),
    ];
    assert!(
        peak(&end) < 1e-3,
        "the clip ends in silence: {}",
        peak(&end)
    );
}

/// A time selection over two tracks, cutting through their clips: one new track per track
/// holding exactly the range (plus the tail), as one render job and one undo step; tracks
/// with nothing in the range are skipped.
#[test]
fn bounce_a_time_selection_to_new_tracks() {
    let mut d = Drive::new();
    let (m, _) = keys(&mut d);
    let a = d.track(TrackKind::Audio, "Tone");
    d.audio_clip(a, 0.0);
    let empty = d.track(TrackKind::Audio, "Empty");
    d.settle();
    let before = d.p().clone();
    let (seed_tracks, seed_clips, seed_media): (TrackId, ClipId, MediaId) =
        (d.id(), d.id(), d.id());
    let range = |tracks: Vec<TrackId>, start: f64, end: f64| {
        Command::Freeze(FreezeCommand::BounceRange {
            job: "r".into(),
            tracks,
            start: Beats(start),
            end: Beats(end),
            seed_tracks,
            seed_clips,
            seed_media,
        })
    };
    // Nothing in the range: refused, nothing changes.
    for c in [
        range(vec![empty], 1.0, 3.0),
        range(vec![m, a], 20.0, 24.0),
        range(vec![m], 2.0, 2.0),
        // Longer than a render may be (`MAX_RENDER_SECONDS`).
        range(vec![m, a], 0.0, 1e12),
    ] {
        assert_eq!(d.err(c).code, ErrorCode::InvalidArgument);
    }
    assert_eq!(d.p(), &before);

    d.ok(range(vec![m, a, empty], 1.0, 3.0));
    assert!(matches!(d.finish("r").0, FreezeEvent::Done { .. }));
    d.settle();
    let order = |d: &Drive| {
        let mut v: Vec<&Track> = d
            .p()
            .tracks
            .values()
            .filter(|t| t.parent.is_none() && t.kind != TrackKind::Master)
            .collect();
        v.sort_by(|x, y| x.order.cmp(&y.order));
        v.into_iter().map(|t| t.id).collect::<Vec<_>>()
    };
    let (nm, na): (TrackId, TrackId) = (
        ether_model::derive_id(seed_tracks, 0),
        ether_model::derive_id(seed_tracks, 1),
    );
    assert!(
        !d.p()
            .tracks
            .contains_key(&ether_model::derive_id::<TrackId, TrackId>(seed_tracks, 2))
    );
    assert_eq!(order(&d), vec![m, nm, a, na, empty]);
    assert_eq!(d.p().tracks[&nm].name, "Keys Bounce");
    assert_eq!(d.p().tracks[&na].name, "Tone Bounce");
    for (i, (src, new)) in [(m, nm), (a, na)].into_iter().enumerate() {
        let clip: ClipId = ether_model::derive_id(seed_clips, i as u32);
        let media: MediaId = ether_model::derive_id(seed_media, i as u32);
        let c = &d.p().clips[&clip];
        assert_eq!((c.track, c.start), (new, Beats(1.0)));
        assert!(d.p().tracks[&src].mixer.mute);
        assert!(d.p().media[&media].frames >= SR as u64, "the range is 1 s");
    }
    // The audio track has no chain: exactly the range (plus the anti-click pad).
    let tone: MediaId = ether_model::derive_id(seed_media, 1);
    assert!(d.p().media[&tone].frames <= SR as u64 + 512);
    // Sources muted (once the mute ramp has run): nothing before the selection start, the
    // bounce inside it.
    d.play(SR as usize / 10);
    let out = d.play(2 * SR as usize);
    let half = SR as usize / 2;
    let pre = [out[0][..half - 64].to_vec(), out[1][..half - 64].to_vec()];
    assert!(peak(&pre) < 1e-6, "silent before the range: {}", peak(&pre));
    let inside = [
        out[0][half + 256..3 * half].to_vec(),
        out[1][half + 256..3 * half].to_vec(),
    ];
    assert!(peak(&inside) > 0.01, "the range plays");
    // One undo step removes both tracks and unmutes the sources.
    d.ok(Command::Edit(EditCommand::Undo));
    d.settle();
    assert_eq!(order(&d), vec![m, a, empty]);
    assert!(!d.p().tracks[&m].mixer.mute && !d.p().tracks[&a].mixer.mute);
}

#[test]
fn consolidate_midi_is_instant_and_audio_renders() {
    let mut d = Drive::new();
    let m = d.track(TrackKind::Midi, "Keys");
    d.device(m, BuiltinDevice::Synth);
    d.midi_clip(m, 0.0, &[60, 64]);
    d.midi_clip(m, 4.0, &[67]);
    let (seed_clips, seed_notes, seed_media): (ClipId, ClipId, MediaId) = (d.id(), d.id(), d.id());
    let v = d.ok(Command::Freeze(FreezeCommand::Consolidate {
        job: "c1".into(),
        tracks: vec![m],
        start: Beats(0.0),
        end: Beats(8.0),
        seed_clips,
        seed_notes,
        seed_media,
    }));
    assert_eq!(v, ReplyValue::Unit);
    let clip: ClipId = ether_model::derive_id(seed_clips, 0);
    let clips: Vec<&Clip> = d.p().clips.values().filter(|c| c.track == m).collect();
    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0].id, clip);
    assert_eq!(clips[0].length, Beats(8.0));
    let mut notes: Vec<(f64, u8, NoteId)> = d
        .p()
        .notes_of(clip)
        .iter()
        .map(|n| (n.start.0, n.pitch, n.id))
        .collect();
    notes.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(
        notes,
        vec![
            (0.0, 60, ether_model::derive_id(seed_notes, 0)),
            (0.75, 64, ether_model::derive_id(seed_notes, 1)),
            (4.0, 67, ether_model::derive_id(seed_notes, 2)),
        ]
    );

    // Audio: rendered pre-chain, one clip per track.
    let a = d.track(TrackKind::Audio, "Tone");
    d.audio_clip(a, 0.0);
    d.audio_clip(a, 5.0);
    d.settle();
    let (seed_clips, seed_notes, seed_media): (ClipId, ClipId, MediaId) = (d.id(), d.id(), d.id());
    let v = d.ok(Command::Freeze(FreezeCommand::Consolidate {
        job: "c2".into(),
        tracks: vec![a],
        start: Beats(0.0),
        end: Beats(8.0),
        seed_clips,
        seed_notes,
        seed_media,
    }));
    assert_eq!(v, ReplyValue::RenderStarted { job: "c2".into() });
    assert!(matches!(d.finish("c2").0, FreezeEvent::Done { .. }));
    let media: MediaId = ether_model::derive_id(seed_media, 0);
    let clip: ClipId = ether_model::derive_id(seed_clips, 0);
    // The clip crossing the range end keeps its part after it.
    let mut clips: Vec<&Clip> = d.p().clips.values().filter(|c| c.track == a).collect();
    clips.sort_by(|x, y| x.start.0.total_cmp(&y.start.0));
    assert_eq!(clips.len(), 2, "{clips:?}");
    assert_eq!(clips[0].id, clip);
    assert_eq!((clips[0].start, clips[0].length), (Beats(0.0), Beats(8.0)));
    assert_eq!((clips[1].start, clips[1].length), (Beats(8.0), Beats(1.0)));
    assert_eq!(d.p().media[&media].frames, 4 * SR as u64);
}

#[test]
fn one_job_at_a_time_cancel_and_errors() {
    let mut d = Drive::new();
    let (t, _) = keys(&mut d);
    let empty = d.track(TrackKind::Midi, "Empty");
    let group = d.track(TrackKind::Group, "Group");
    let mid: MediaId = d.id();
    let e = d.err(Command::Freeze(FreezeCommand::Freeze {
        job: "x".into(),
        track: empty,
        media: mid,
        up_to: None,
    }));
    assert_eq!(e.code, ErrorCode::InvalidArgument);
    let mid: MediaId = d.id();
    let e = d.err(Command::Freeze(FreezeCommand::Freeze {
        job: "x".into(),
        track: group,
        media: mid,
        up_to: None,
    }));
    assert_eq!(e.code, ErrorCode::InvalidArgument);
    let mid: MediaId = d.id();
    d.ok(Command::Freeze(FreezeCommand::Freeze {
        job: "j1".into(),
        track: t,
        media: mid,
        up_to: None,
    }));
    let mid: MediaId = d.id();
    let e = d.err(Command::Freeze(FreezeCommand::Freeze {
        job: "j2".into(),
        track: t,
        media: mid,
        up_to: None,
    }));
    assert_eq!(e.code, ErrorCode::InvalidState);
    d.tick();
    let out = d.send(Command::Freeze(FreezeCommand::Cancel { job: "j1".into() }));
    assert!(events(&out).contains(&Event::Freeze {
        event: FreezeEvent::Cancelled { job: "j1".into() }
    }));
    for _ in 0..50 {
        assert!(
            events(&d.tick())
                .iter()
                .all(|e| !matches!(e, Event::Freeze { .. }))
        );
    }
    assert!(d.p().tracks[&t].freeze.is_none());
    // A new job can start; closing the project fails it.
    let mid: MediaId = d.id();
    d.ok(Command::Freeze(FreezeCommand::Freeze {
        job: "j3".into(),
        track: t,
        media: mid,
        up_to: None,
    }));
    let other = d.ids.next_project_id(T0);
    d.ok(Command::Project(ProjectCommand::Create {
        id: other,
        name: "Other".into(),
    }));
    let (end, _) = d.finish("j3");
    assert!(matches!(end, FreezeEvent::Failed { .. }), "{end:?}");
}

#[test]
fn export_plays_frozen_tracks() {
    use ether_core::protocol::export::*;
    let mut d = Drive::new();
    let (t, _) = keys(&mut d);
    let master = d.master();
    let _ = master;
    d.freeze(t);
    let job = "e1".to_string();
    d.ok(Command::Export(ExportCommand::Render {
        job: job.clone(),
        request: ExportRequest {
            range: ExportRange::Custom {
                start: Beats(0.0),
                end: Beats(8.0),
            },
            format: ExportFormat {
                container: AudioContainer::Wav,
                bit_depth: BitDepth::Float32,
                sample_rate: None,
            },
            mode: ExportMode::Mix,
            normalize: false,
            in_memory: None,
            extra_formats: None,
            folder: None,
            tail_seconds: 0.0,
            name: None,
        },
    }));
    let mut done = None;
    for _ in 0..100_000 {
        for e in events(&d.tick()) {
            if let Event::Export {
                event: ExportEvent::Done { result, .. },
            } = e
            {
                done = Some(result);
            }
        }
        if done.is_some() {
            break;
        }
    }
    let Some(ExportResult::Files { files }) = done else {
        panic!("export failed")
    };
    let bytes = d.ctl.store.read(d.pid, &files[0]).unwrap();
    let audio = ether_media::decode(&bytes, Some("wav")).unwrap();
    assert!(
        peak(&audio.channels) > 0.01,
        "the frozen render is exported"
    );
}

// ─── Freeze Group ─────────────────────────────────────────────────────────────────────────

impl Drive {
    fn group_of(&mut self, ids: &[TrackId], name: &str) -> TrackId {
        let group: TrackId = self.id();
        self.ok(Command::Track(TrackCommand::GroupSelected {
            ids: ids.to_vec(),
            group,
            name: Some(name.into()),
        }));
        group
    }

    fn unfreeze(&mut self, track: TrackId) {
        self.ok(Command::Freeze(FreezeCommand::Unfreeze { track }));
        self.settle();
    }
}

/// Group "Band" = Keys (synth + delay) and Pad (synth), a delay on the group, fader -3 dB.
fn band(d: &mut Drive) -> (TrackId, TrackId, TrackId) {
    let (keys_t, _) = keys(d);
    let pad = d.track(TrackKind::Midi, "Pad");
    d.device(pad, BuiltinDevice::Synth);
    d.midi_clip(pad, 0.0, &[48, 55]);
    let g = d.group_of(&[keys_t, pad], "Band");
    d.device(g, BuiltinDevice::Delay);
    d.ok(Command::Mixer(MixerCommand::SetVolume {
        track: g,
        volume: Decibels(-3.0),
    }));
    d.settle();
    (g, keys_t, pad)
}

#[test]
fn frozen_group_sounds_like_the_live_group_and_children_are_not_processed() {
    let mut d = Drive::new();
    let (g, keys_t, pad) = band(&mut d);
    let live = d.play(FRAMES);
    assert!(peak(&live) > 0.01, "audible");
    let nodes_before = d.ctl.bridge.nodes.len();
    let media = d.freeze(g);
    let f = d.p().tracks[&g].freeze.clone().expect("group frozen");
    assert_eq!(f.media, media);
    assert!(d.p().tracks[&keys_t].freeze.is_none());
    assert_eq!(d.p().frozen_ancestor(keys_t), Some(g));
    assert_eq!(d.p().frozen_ancestor(g), None);
    // Group delay + Keys synth + Keys delay + Pad synth: no nodes while frozen.
    assert_eq!(d.ctl.bridge.nodes.len(), nodes_before - 4);
    let frozen = d.play(FRAMES);
    let diff = max_diff(&live, &frozen);
    assert!(diff < 1e-4, "frozen group playback differs by {diff}");

    // Compiled graph: children are inert (no clips, no output, no chain).
    let graph = d.ctl.bridge.last_graph.clone().expect("graph");
    for child in [keys_t, pad] {
        let td = graph.tracks.iter().find(|t| t.id == child).unwrap();
        assert!(td.clips.is_empty() && td.chain.is_empty() && td.output.is_none());
        assert!(td.sends.is_empty() && !td.solo && td.sidechain_taps.is_empty());
    }
    let gd = graph.tracks.iter().find(|t| t.id == g).unwrap();
    assert!(gd.frozen.is_some() && gd.chain.is_empty());

    // Soloing or muting a child is a no-op: its solo does not gate the other tracks.
    d.ok(Command::Mixer(MixerCommand::SetSolo {
        track: keys_t,
        solo: true,
        exclusive: false,
    }));
    d.ok(Command::Mixer(MixerCommand::SetMute {
        track: pad,
        mute: true,
    }));
    d.tick();
    let still = d.play(FRAMES);
    assert!(
        max_diff(&frozen, &still) < 1e-6,
        "child solo/mute must not matter"
    );
    d.ok(Command::Mixer(MixerCommand::SetSolo {
        track: keys_t,
        solo: false,
        exclusive: false,
    }));
    d.ok(Command::Mixer(MixerCommand::SetMute {
        track: pad,
        mute: false,
    }));

    // The group's fader stays live.
    d.ok(Command::Mixer(MixerCommand::SetVolume {
        track: g,
        volume: Decibels(-9.0),
    }));
    let quieter = d.play(FRAMES);
    let ratio = peak(&quieter) / peak(&frozen);
    assert!((ratio - 0.5).abs() < 0.03, "{ratio}");
    d.ok(Command::Mixer(MixerCommand::SetVolume {
        track: g,
        volume: Decibels(-3.0),
    }));

    // Let the fader's smoothing settle, then unfreeze: everything is back.
    let _ = d.play(4800);
    d.unfreeze(g);
    assert!(d.p().tracks[&g].freeze.is_none());
    assert_eq!(d.ctl.bridge.nodes.len(), nodes_before);
    let back = d.play(FRAMES);
    let diff = max_diff(&live, &back);
    assert!(diff < 1e-4, "unfrozen group differs by {diff}");
}

#[test]
fn frozen_group_locks_its_children() {
    let mut d = Drive::new();
    let (g, keys_t, pad) = band(&mut d);
    let clip = d.p().clips.values().find(|c| c.track == keys_t).unwrap().id;
    let outside = d.track(TrackKind::Midi, "Outside");
    d.freeze(g);
    let locked = [
        Command::Clip(ClipCommand::Delete { ids: vec![clip] }),
        Command::Clip(ClipCommand::CreateMidi {
            id: d.id(),
            track: pad,
            start: Beats(8.0),
            length: Beats(4.0),
            name: None,
        }),
        Command::Device(DeviceCommand::Insert {
            id: d.id(),
            track: pad,
            device: DeviceSpec::Builtin {
                device: BuiltinDevice::Delay,
            },
            before: None,
        }),
        // On the frozen group itself too.
        Command::Device(DeviceCommand::Insert {
            id: d.id(),
            track: g,
            device: DeviceSpec::Builtin {
                device: BuiltinDevice::Delay,
            },
            before: None,
        }),
        Command::Track(TrackCommand::Delete { id: pad }),
        Command::Track(TrackCommand::Move {
            id: pad,
            parent: None,
            before: None,
        }),
        Command::Track(TrackCommand::Move {
            id: outside,
            parent: Some(g),
            before: None,
        }),
        Command::Track(TrackCommand::Create {
            id: d.id(),
            kind: TrackKind::Audio,
            name: None,
            color: None,
            parent: Some(g),
            before: None,
        }),
        Command::Freeze(FreezeCommand::Unfreeze { track: pad }),
    ];
    for c in locked {
        let e = d.err(c.clone());
        assert_eq!(e.code, ErrorCode::InvalidState, "{c:?}: {}", e.message);
    }
    // A child cannot be frozen on its own.
    let media: MediaId = d.id();
    let e = d.err(Command::Freeze(FreezeCommand::Freeze {
        job: "x".into(),
        track: pad,
        media,
        up_to: None,
    }));
    assert_eq!(e.code, ErrorCode::InvalidState);
    // Mixer edits of the group stay possible; renaming too.
    d.ok(Command::Mixer(MixerCommand::SetPan {
        track: g,
        pan: Pan(0.2),
    }));
    d.ok(Command::Track(TrackCommand::Rename {
        id: g,
        name: "Band 2".into(),
    }));
    // Unfreezing unlocks.
    d.unfreeze(g);
    d.ok(Command::Clip(ClipCommand::Delete { ids: vec![clip] }));
}

#[test]
fn group_freeze_undo_redo() {
    let mut d = Drive::new();
    let (g, _, _) = band(&mut d);
    let nodes = d.ctl.bridge.nodes.len();
    let media = d.freeze(g);
    d.ok(Command::Edit(EditCommand::Undo));
    d.tick();
    assert!(d.p().tracks[&g].freeze.is_none());
    assert!(!d.p().media.contains_key(&media));
    assert_eq!(d.ctl.bridge.nodes.len(), nodes);
    d.ok(Command::Edit(EditCommand::Redo));
    d.tick();
    assert!(d.p().tracks[&g].freeze.is_some());
    // Unfreeze is undoable too.
    d.unfreeze(g);
    d.ok(Command::Edit(EditCommand::Undo));
    d.tick();
    assert!(d.p().tracks[&g].freeze.is_some());
}

#[test]
fn nested_groups_freeze_inner_then_outer() {
    let mut d = Drive::new();
    let (inner, keys_t, pad) = band(&mut d);
    let lead = d.track(TrackKind::Midi, "Lead");
    d.device(lead, BuiltinDevice::Synth);
    d.midi_clip(lead, 0.0, &[72, 76]);
    let outer = d.group_of(&[inner, lead], "All");
    d.settle();
    let live = d.play(FRAMES);
    assert!(peak(&live) > 0.01);
    d.freeze(inner);
    let with_inner = d.play(FRAMES);
    assert!(max_diff(&live, &with_inner) < 1e-4);
    // The outer group renders with the inner group's frozen audio.
    d.freeze(outer);
    assert_eq!(d.p().frozen_ancestor(inner), Some(outer));
    assert_eq!(d.p().frozen_ancestor(keys_t), Some(inner));
    let all = d.play(FRAMES);
    assert!(max_diff(&live, &all) < 1e-3, "{}", max_diff(&live, &all));
    // The inner group is locked by the outer one.
    let e = d.err(Command::Freeze(FreezeCommand::Unfreeze { track: inner }));
    assert_eq!(e.code, ErrorCode::InvalidState);
    // Unfreezing the outer group leaves the inner one frozen.
    d.unfreeze(outer);
    assert!(d.p().tracks[&inner].freeze.is_some());
    assert!(d.p().frozen_ancestor(inner).is_none());
    assert!(d.p().frozen_ancestor(pad).is_some());
    let again = d.play(FRAMES);
    assert!(max_diff(&live, &again) < 1e-4);
}

#[test]
fn freeze_group_rejects_children_routed_outside_and_empty_groups() {
    let mut d = Drive::new();
    let (g, keys_t, _) = band(&mut d);
    let bus = d.track(TrackKind::Return, "Bus");
    let empty = d.track(TrackKind::Group, "Empty");
    let media: MediaId = d.id();
    let e = d.err(Command::Freeze(FreezeCommand::Freeze {
        job: "e".into(),
        track: empty,
        media,
        up_to: None,
    }));
    assert_eq!(e.code, ErrorCode::InvalidArgument);
    d.ok(Command::Mixer(MixerCommand::SetOutput {
        track: keys_t,
        output: TrackOutput::Track { track: bus },
    }));
    let e = d.err(Command::Freeze(FreezeCommand::Freeze {
        job: "r".into(),
        track: g,
        media,
        up_to: None,
    }));
    assert_eq!(e.code, ErrorCode::InvalidState, "{}", e.message);
    assert!(e.message.contains("routed outside"), "{}", e.message);
}

#[test]
fn freeze_group_warns_about_sends_and_sidechains() {
    let mut d = Drive::new();
    let (g, keys_t, _) = band(&mut d);
    let ret = d.track(TrackKind::Return, "Verb");
    let send: SendId = d.id();
    d.ok(Command::Mixer(MixerCommand::CreateSend {
        id: send,
        from: keys_t,
        to: ret,
        level: Decibels(-10.0),
        pre_fader: false,
    }));
    let bass = d.track(TrackKind::Midi, "Bass");
    let comp = d.device(bass, BuiltinDevice::Compressor);
    d.ok(Command::Device(DeviceCommand::SetSidechain {
        device: comp,
        source: Some(keys_t),
        tap: None,
    }));
    d.settle();
    let media: MediaId = d.id();
    let out = d.send(Command::Freeze(FreezeCommand::Freeze {
        job: "w".into(),
        track: g,
        media,
        up_to: None,
    }));
    let warning = events(&out)
        .into_iter()
        .find_map(|e| match e {
            Event::Notification {
                level: NotificationLevel::Warning,
                message,
                ..
            } => Some(message),
            _ => None,
        })
        .expect("a warning");
    assert!(
        warning.contains("1 send") && warning.contains("1 sidechain"),
        "{warning}"
    );
    let (end, _) = d.finish("w");
    assert_eq!(end, FreezeEvent::Done { job: "w".into() });
}

#[test]
fn flatten_group_replaces_the_group_with_one_audio_track() {
    let mut d = Drive::new();
    let (g, keys_t, pad) = band(&mut d);
    let ret = d.track(TrackKind::Return, "Verb");
    let send: SendId = d.id();
    d.ok(Command::Mixer(MixerCommand::CreateSend {
        id: send,
        from: g,
        to: ret,
        level: Decibels(-10.0),
        pre_fader: false,
    }));
    d.settle();
    let live = d.play(FRAMES);
    d.freeze(g);
    let index = d.p().tracks_ordered().iter().position(|x| x.id == g);
    let (clip, new_track): (ClipId, TrackId) = (d.id(), d.id());
    d.ok(Command::Freeze(FreezeCommand::Flatten {
        track: g,
        clip,
        new_track,
    }));
    d.settle();
    for gone in [g, keys_t, pad] {
        assert!(!d.p().tracks.contains_key(&gone));
    }
    let nt = d.p().tracks[&new_track].clone();
    assert_eq!((nt.kind, nt.name.as_str()), (TrackKind::Audio, "Band"));
    assert_eq!(nt.mixer.volume, Decibels(-3.0));
    assert_eq!(
        d.p()
            .tracks_ordered()
            .iter()
            .position(|x| x.id == new_track),
        index
    );
    assert_eq!(d.p().clips.len(), 1);
    assert_eq!(d.p().clips[&clip].track, new_track);
    assert_eq!(d.p().devices.len(), 0);
    let sends: Vec<_> = d
        .p()
        .sends
        .values()
        .filter(|s| s.from == new_track)
        .collect();
    assert_eq!((sends.len(), sends[0].to), (1, ret));
    let flat = d.play(FRAMES);
    let diff = max_diff_from(&live, &flat, 64);
    assert!(diff < 1e-3, "flattened group differs by {diff}");
    // One undo step brings the group and its children back (frozen).
    d.ok(Command::Edit(EditCommand::Undo));
    d.settle();
    assert!(d.p().tracks.contains_key(&g) && d.p().tracks.contains_key(&pad));
    assert!(!d.p().tracks.contains_key(&new_track));
    assert!(d.p().tracks[&g].freeze.is_some());
    d.ok(Command::Edit(EditCommand::Redo));
    assert!(d.p().tracks.contains_key(&new_track));
}

#[test]
fn flatten_needs_a_frozen_group() {
    let mut d = Drive::new();
    let (g, _, _) = band(&mut d);
    let (clip, new_track): (ClipId, TrackId) = (d.id(), d.id());
    let e = d.err(Command::Freeze(FreezeCommand::Flatten {
        track: g,
        clip,
        new_track,
    }));
    assert_eq!(e.code, ErrorCode::InvalidState);
}

#[test]
fn export_plays_frozen_groups() {
    use ether_core::protocol::export::*;
    let mut d = Drive::new();
    let (g, _, _) = band(&mut d);
    d.freeze(g);
    d.ok(Command::Export(ExportCommand::Render {
        job: "eg".into(),
        request: ExportRequest {
            range: ExportRange::Custom {
                start: Beats(0.0),
                end: Beats(8.0),
            },
            format: ExportFormat {
                container: AudioContainer::Wav,
                bit_depth: BitDepth::Float32,
                sample_rate: None,
            },
            mode: ExportMode::Mix,
            normalize: false,
            in_memory: None,
            extra_formats: None,
            folder: None,
            tail_seconds: 0.0,
            name: None,
        },
    }));
    let mut done = None;
    for _ in 0..100_000 {
        for e in events(&d.tick()) {
            if let Event::Export {
                event: ExportEvent::Done { result, .. },
            } = e
            {
                done = Some(result);
            }
        }
        if done.is_some() {
            break;
        }
    }
    let Some(ExportResult::Files { files }) = done else {
        panic!("export failed")
    };
    let bytes = d.ctl.store.read(d.pid, &files[0]).unwrap();
    let audio = ether_media::decode(&bytes, Some("wav")).unwrap();
    assert!(
        peak(&audio.channels) > 0.01,
        "the frozen group render is exported"
    );
}

// ─── One-pass stems ───────────────────────────────────────────────────────────────────────

use ether_core::protocol::export::*;

impl Drive {
    /// Run an export of the whole project (Float32 WAV, 0.5 s tail) and decode its files.
    fn export_files(&mut self, mode: ExportMode) -> Vec<DecodedAudio> {
        let job = format!("x{}", self.next);
        self.ok(Command::Export(ExportCommand::Render {
            job,
            request: ExportRequest {
                range: ExportRange::Custom {
                    start: Beats(0.0),
                    end: Beats(8.0),
                },
                format: ExportFormat {
                    container: AudioContainer::Wav,
                    bit_depth: BitDepth::Float32,
                    sample_rate: None,
                },
                mode,
                normalize: false,
                in_memory: None,
                extra_formats: None,
                folder: None,
                tail_seconds: 0.5,
                name: None,
            },
        }));
        let mut done = None;
        for _ in 0..1_000_000 {
            for e in events(&self.tick()) {
                match e {
                    Event::Export {
                        event: ExportEvent::Done { result, .. },
                    } => done = Some(result),
                    Event::Export {
                        event: ExportEvent::Failed { message, .. },
                    } => panic!("export failed: {message}"),
                    _ => {}
                }
            }
            if done.is_some() {
                break;
            }
        }
        let Some(ExportResult::Files { files }) = done else {
            panic!("export did not finish")
        };
        files
            .iter()
            .map(|f| {
                let bytes = self.ctl.store.read(self.pid, f).unwrap();
                ether_media::decode(&bytes, Some("wav")).unwrap()
            })
            .collect()
    }
}

/// The mix and every stem of one full render equal the earlier separate renders (one pass per
/// stem) sample for sample: a frozen track, a muted track, a latency-reporting limiter, a
/// group with a child, a return and a track with a send (which keeps a pass of its own).
#[test]
fn one_pass_stems_equal_per_track_passes() {
    let mut d = Drive::new();
    let (keys_t, _) = keys(&mut d);
    let tone = d.track(TrackKind::Audio, "Tone");
    d.device(tone, BuiltinDevice::Limiter);
    d.audio_clip(tone, 1.0);
    let ghost = d.track(TrackKind::Midi, "Ghost");
    d.device(ghost, BuiltinDevice::Synth);
    d.midi_clip(ghost, 0.0, &[48, 55]);
    d.ok(Command::Mixer(MixerCommand::SetMute {
        track: ghost,
        mute: true,
    }));
    let child = d.track(TrackKind::Audio, "Child");
    d.audio_clip(child, 2.0);
    let group = d.group_of(&[child], "Bus");
    d.device(group, BuiltinDevice::Compressor);
    let ret = d.track(TrackKind::Return, "Verb");
    d.device(ret, BuiltinDevice::Delay);
    let sender = d.track(TrackKind::Midi, "Sender");
    d.device(sender, BuiltinDevice::Synth);
    d.midi_clip(sender, 1.0, &[72, 76]);
    let send: SendId = d.id();
    d.ok(Command::Mixer(MixerCommand::CreateSend {
        id: send,
        from: sender,
        to: ret,
        level: Decibels(-8.0),
        pre_fader: false,
    }));
    d.settle();
    d.freeze(keys_t);

    let stems = vec![keys_t, tone, ghost, child, group, ret, sender];

    // Reference: the old behavior, one render per stem (a lone stem is never shared).
    let t0 = std::time::Instant::now();
    let mix_ref = d.export_files(ExportMode::Mix).remove(0);
    let separate: Vec<DecodedAudio> = stems
        .iter()
        .map(|&t| {
            d.export_files(ExportMode::Stems { tracks: vec![t] })
                .remove(0)
        })
        .collect();
    let old = t0.elapsed();

    let t1 = std::time::Instant::now();
    let mut all = d.export_files(ExportMode::MixAndStems {
        tracks: stems.clone(),
    });
    let new = t1.elapsed();
    assert_eq!(all.len(), 1 + stems.len());
    let mix = all.remove(0);
    let max_diff = |a: &DecodedAudio, b: &DecodedAudio| {
        assert_eq!(a.frames(), b.frames());
        a.channels
            .iter()
            .zip(&b.channels)
            .flat_map(|(x, y)| x.iter().zip(y).map(|(x, y)| (x - y).abs()))
            .fold(0.0f32, f32::max)
    };
    assert_eq!(max_diff(&mix, &mix_ref), 0.0, "the mix is unchanged");
    for ((s, r), t) in all.iter().zip(&separate).zip(&stems) {
        assert!(peak(&r.channels) > 0.001, "stem {t:?} is audible");
        assert_eq!(max_diff(s, r), 0.0, "stem {t:?}");
    }
    // Stems alone share one render too (several capturable stems).
    let alone = d.export_files(ExportMode::Stems {
        tracks: stems.clone(),
    });
    for ((s, r), t) in alone.iter().zip(&separate).zip(&stems) {
        assert_eq!(max_diff(s, r), 0.0, "stems-only {t:?}");
    }
    eprintln!(
        "one-pass stems bench: mix + {} stems: {} separate renders {old:?}, one-pass {new:?}",
        stems.len(),
        1 + stems.len()
    );
    // An empty stem list is just the mix.
    let only = d.export_files(ExportMode::MixAndStems { tracks: vec![] });
    assert_eq!(only.len(), 1);
    assert_eq!(max_diff(&only[0], &mix_ref), 0.0);
}

// ─── Freeze up to a device (partial chain freeze, #25) ───────────────────────────────────

/// Freeze `track` up to (and including) `up_to`; the devices after it stay live.
fn freeze_up_to(d: &mut Drive, track: TrackId, up_to: DeviceId) -> MediaId {
    let media: MediaId = d.id();
    let job = format!("partial{}", d.next);
    assert_eq!(
        d.ok(Command::Freeze(FreezeCommand::Freeze {
            job: job.clone(),
            track,
            media,
            up_to: Some(up_to),
        })),
        ReplyValue::RenderStarted { job: job.clone() }
    );
    let (end, _) = d.finish(&job);
    assert_eq!(end, FreezeEvent::Done { job });
    d.settle();
    media
}

/// `keys` plus a utility after the delay (the live tail of a freeze up to the delay).
fn keys_with_tail(d: &mut Drive) -> (TrackId, DeviceId, DeviceId) {
    let (t, delay) = keys(d);
    let utility = d.device(t, BuiltinDevice::Utility);
    d.settle();
    (t, delay, utility)
}

#[test]
fn freeze_up_to_a_device_keeps_the_tail_live() {
    let mut d = Drive::new();
    let (t, delay, utility) = keys_with_tail(&mut d);
    let live = d.play(FRAMES);
    assert!(peak(&live) > 0.01, "audible");
    let nodes_before = d.ctl.bridge.nodes.len();
    freeze_up_to(&mut d, t, delay);
    let f = d.p().tracks[&t].freeze.clone().expect("frozen");
    assert_eq!(f.up_to, Some(delay));
    assert_eq!(d.p().frozen_chain_len(t), Some(2));
    assert!(d.p().device_frozen(delay));
    assert!(!d.p().device_frozen(utility));
    // Synth and delay lose their nodes; the utility keeps its node and runs after the render.
    assert_eq!(d.ctl.bridge.nodes.len(), nodes_before - 2);
    assert!(d.ctl.bridge.nodes.values().any(|x| *x == utility));
    let graph = d.ctl.bridge.last_graph.clone().unwrap();
    let td = graph.tracks.iter().find(|x| x.id == t).unwrap();
    assert!(td.frozen.is_some());
    assert!(td.clips.is_empty());
    assert_eq!(td.chain.len(), 1);
    let frozen = d.play(FRAMES);
    let diff = max_diff(&live, &frozen);
    assert!(diff < 1e-4, "partially frozen playback differs by {diff}");

    // The live tail stays editable: its gain changes the output.
    d.ok(Command::Device(DeviceCommand::SetParam {
        device: utility,
        param: ether_devices::utility::params::GAIN,
        value: -6.0,
    }));
    let quieter = d.play(FRAMES);
    // After the gain smoothing of the first blocks.
    let after = |x: &[Vec<f32>; 2]| {
        [
            x[0][SR as usize / 4..].to_vec(),
            x[1][SR as usize / 4..].to_vec(),
        ]
    };
    let ratio = peak(&after(&quieter)) / peak(&after(&frozen));
    assert!((ratio - 0.5).abs() < 0.02, "{ratio}");

    // Devices up to the freeze point stay locked.
    for c in [
        Command::Device(DeviceCommand::SetParam {
            device: delay,
            param: ether_devices::descriptor(BuiltinDeviceType::Delay).params[0].id,
            value: 0.5,
        }),
        Command::Device(DeviceCommand::Remove { id: delay }),
        Command::Device(DeviceCommand::Insert {
            id: d.id(),
            track: t,
            device: DeviceSpec::Builtin {
                device: BuiltinDevice::Reverb,
            },
            before: Some(delay),
        }),
        Command::Device(DeviceCommand::Move {
            id: utility,
            track: t,
            before: Some(delay),
        }),
        Command::Automation(AutomationCommand::CreateLane {
            id: d.id(),
            owner: AutomationOwner::Track { track: t },
            target: AutomationTarget::DeviceParam {
                device: delay,
                param: ParamId(0),
            },
        }),
    ] {
        let before = d.p().clone();
        let e = d.err(c.clone());
        assert_eq!(e.code, ErrorCode::InvalidState, "{c:?}");
        assert_eq!(d.p(), &before);
    }
    // ...and so do the clips.
    let clip = d.p().clips.values().find(|c| c.track == t).unwrap().id;
    let e = d.err(Command::Clip(ClipCommand::Delete { ids: vec![clip] }));
    assert_eq!(e.code, ErrorCode::InvalidState);

    // Devices can be added to, automated in and removed from the tail.
    let reverb: DeviceId = d.id();
    for c in [
        Command::Device(DeviceCommand::Insert {
            id: reverb,
            track: t,
            device: DeviceSpec::Builtin {
                device: BuiltinDevice::Reverb,
            },
            before: Some(utility),
        }),
        Command::Automation(AutomationCommand::CreateLane {
            id: d.id(),
            owner: AutomationOwner::Track { track: t },
            target: AutomationTarget::DeviceParam {
                device: utility,
                param: ParamId(0),
            },
        }),
        Command::Device(DeviceCommand::Move {
            id: reverb,
            track: t,
            before: None,
        }),
        Command::Device(DeviceCommand::Remove { id: reverb }),
    ] {
        d.ok(c);
    }

    // Unfreeze: the whole chain is live again.
    d.ok(Command::Freeze(FreezeCommand::Unfreeze { track: t }));
    d.tick();
    assert!(d.p().tracks[&t].freeze.is_none());
    assert_eq!(d.ctl.bridge.nodes.len(), nodes_before);
}

#[test]
fn freeze_up_to_the_instrument_keeps_every_audio_effect_live() {
    let mut d = Drive::new();
    let (t, delay) = keys(&mut d);
    let synth = d.p().devices_of(t)[0].id;
    let live = d.play(FRAMES);
    freeze_up_to(&mut d, t, synth);
    assert_eq!(d.p().tracks[&t].freeze.as_ref().unwrap().up_to, Some(synth));
    assert!(!d.p().device_frozen(delay));
    let frozen = d.play(FRAMES);
    let diff = max_diff(&live, &frozen);
    assert!(diff < 1e-4, "partially frozen playback differs by {diff}");
}

#[test]
fn freeze_points_are_validated() {
    let mut d = Drive::new();
    let t = d.track(TrackKind::Midi, "Arp");
    let arp = d.device(t, BuiltinDevice::Arpeggiator);
    d.device(t, BuiltinDevice::Synth);
    let delay = d.device(t, BuiltinDevice::Delay);
    d.midi_clip(t, 0.0, &[60, 64]);
    let other = d.track(TrackKind::Audio, "Other");
    let foreign = d.device(other, BuiltinDevice::Utility);
    d.settle();
    for up_to in [arp, foreign] {
        let media: MediaId = d.id();
        let e = d.err(Command::Freeze(FreezeCommand::Freeze {
            job: "x".into(),
            track: t,
            media,
            up_to: Some(up_to),
        }));
        assert_eq!(e.code, ErrorCode::InvalidArgument, "{}", e.message);
    }
    // Up to the last device = the whole chain, like a plain freeze.
    freeze_up_to(&mut d, t, delay);
    assert_eq!(d.p().tracks[&t].freeze.as_ref().unwrap().up_to, None);
    assert!(d.p().device_frozen(delay));
    let id: DeviceId = d.id();
    let e = d.err(Command::Device(DeviceCommand::Insert {
        id,
        track: t,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::Reverb,
        },
        before: None,
    }));
    assert_eq!(e.code, ErrorCode::InvalidState);
}

#[test]
fn flatten_a_partial_freeze_keeps_the_live_tail() {
    let mut d = Drive::new();
    let (t, delay, utility) = keys_with_tail(&mut d);
    d.ok(Command::Device(DeviceCommand::SetParam {
        device: utility,
        param: ether_devices::utility::params::GAIN,
        value: -6.0,
    }));
    d.settle();
    let live = d.play(FRAMES);
    freeze_up_to(&mut d, t, delay);
    let (clip, new_track): (ClipId, TrackId) = (d.id(), d.id());
    d.ok(Command::Freeze(FreezeCommand::Flatten {
        track: t,
        clip,
        new_track,
    }));
    d.settle();
    assert!(!d.p().tracks.contains_key(&t));
    // The utility moved to the new audio track, after the render.
    let chain: Vec<DeviceId> = d.p().devices_of(new_track).iter().map(|x| x.id).collect();
    assert_eq!(chain, vec![utility]);
    assert!(!d.p().devices.contains_key(&delay));
    let flat = d.play(FRAMES);
    // The moved utility is a fresh node: skip its gain smoothing.
    let diff = max_diff_from(&live, &flat, SR as usize / 4);
    assert!(diff < 1e-3, "flattened playback differs by {diff}");
}

#[test]
fn flatten_a_partially_frozen_audio_track_keeps_its_tail() {
    let mut d = Drive::new();
    let t = d.track(TrackKind::Audio, "Tone");
    let eq = d.device(t, BuiltinDevice::Eq);
    let utility = d.device(t, BuiltinDevice::Utility);
    d.audio_clip(t, 0.0);
    d.settle();
    freeze_up_to(&mut d, t, eq);
    let (clip, new_track): (ClipId, TrackId) = (d.id(), d.id());
    d.ok(Command::Freeze(FreezeCommand::Flatten {
        track: t,
        clip,
        new_track,
    }));
    let chain: Vec<DeviceId> = d.p().devices_of(t).iter().map(|x| x.id).collect();
    assert_eq!(chain, vec![utility]);
    assert!(d.p().tracks[&t].freeze.is_none());
    assert_eq!(d.p().clips[&clip].track, t);
}

/// Records what a render for another site reports, and may run renders elsewhere.
#[derive(Default)]
struct Offload {
    offload: bool,
    updates: std::rc::Rc<std::cell::RefCell<Vec<ether_controller::ext::RenderUpdate>>>,
}

impl ether_controller::ext::ControllerExt<RealBridge, TestHost, MemoryStore, MemoryLibrary>
    for Offload
{
    fn offload_render(
        &mut self,
        _ctl: &mut EtherController<RealBridge, TestHost, MemoryStore, MemoryLibrary>,
        _command: &FreezeCommand,
        job: &str,
        _finish: &ether_controller::ext::Finish,
        _out: &mut dyn ether_controller::MessageSink,
    ) -> Option<ReplyValue> {
        self.offload
            .then(|| ReplyValue::RenderStarted { job: job.into() })
    }

    fn render_update(
        &mut self,
        _ctl: &mut EtherController<RealBridge, TestHost, MemoryStore, MemoryLibrary>,
        _site: SiteId,
        _job: &str,
        update: ether_controller::ext::RenderUpdate,
    ) {
        self.updates.borrow_mut().push(update);
    }
}

/// `ControllerExt::offload_render` runs a render elsewhere (nothing renders here), and
/// `freeze_for` renders here for another site: its result goes to the extension, the
/// document does not change.
#[test]
fn renders_may_run_elsewhere_or_for_another_site() {
    let mut d = Drive::new();
    let (t, _) = keys(&mut d);
    d.ctl.add_extension(Offload {
        offload: true,
        ..Default::default()
    });
    let media: MediaId = d.id();
    assert_eq!(
        d.ok(Command::Freeze(FreezeCommand::Freeze {
            job: "away".into(),
            track: t,
            media,
            up_to: None,
        })),
        ReplyValue::RenderStarted { job: "away".into() }
    );
    assert!(
        events(&d.tick())
            .iter()
            .all(|e| !matches!(e, Event::Freeze { .. }))
    );

    let mut d = Drive::new();
    let (t, _) = keys(&mut d);
    let ext = Offload::default();
    let updates = ext.updates.clone();
    d.ctl.add_extension(ext);
    let before = d.p().clone();
    let media: MediaId = d.id();
    let mut out = Vec::new();
    let now = d.ctl.host.now_ms();
    let started = d.ctl.freeze_for(
        SiteId(7),
        &FreezeCommand::Freeze {
            job: "for7".into(),
            track: t,
            media,
            up_to: None,
        },
        now,
        &mut out,
    );
    assert_eq!(
        started,
        Ok(ReplyValue::RenderStarted { job: "for7".into() })
    );
    for _ in 0..100_000 {
        d.tick();
        if updates
            .borrow()
            .iter()
            .any(|u| matches!(u, ether_controller::ext::RenderUpdate::Done { .. }))
        {
            break;
        }
    }
    assert!(
        matches!(updates.borrow().last(), Some(ether_controller::ext::RenderUpdate::Done { media }) if media.len() == 1),
        "{:?}",
        updates.borrow()
    );
    assert_eq!(d.p().tracks, before.tracks, "the document does not change");
}

// ─── A switched-off MIDI effect in a rack chain (OSS #191) ───────────────────────────────

/// An instrument rack (its chain: [Velocity, switched off, if `with_velocity`] → Synth),
/// then a delay. Returns the track and the rack.
fn rack_keys(d: &mut Drive, with_velocity: bool) -> (TrackId, DeviceId) {
    use ether_core::protocol::racks::RackCommand;
    let t = d.track(TrackKind::Midi, "Rack");
    let rack = d.device(t, BuiltinDevice::new(BuiltinDeviceType::InstrumentRack));
    let chain: RackChainId = d.id();
    d.ok(Command::Rack(RackCommand::AddChain {
        id: chain,
        rack,
        name: None,
        before: None,
    }));
    let insert = |d: &mut Drive, ty| {
        let id: DeviceId = d.id();
        d.ok(Command::Rack(RackCommand::InsertDevice {
            id,
            chain,
            device: DeviceSpec::Builtin {
                device: BuiltinDevice::new(ty),
            },
            before: None,
        }));
        id
    };
    if with_velocity {
        let velocity = insert(d, BuiltinDeviceType::Velocity);
        d.ok(Command::Device(DeviceCommand::SetEnabled {
            id: velocity,
            enabled: false,
        }));
    }
    insert(d, BuiltinDeviceType::Synth);
    d.device(t, BuiltinDevice::new(BuiltinDeviceType::Delay));
    d.midi_clip(t, 0.0, &[60, 64, 67, 72]);
    d.settle();
    (t, rack)
}

#[test]
fn a_switched_off_midi_effect_in_a_rack_chain_plays_live_and_frozen() {
    let absent = {
        let mut d = Drive::new();
        rack_keys(&mut d, false);
        d.play(FRAMES)
    };
    assert!(peak(&absent) > 0.01, "audible");
    // Live: as if the switched-off effect were not there.
    let mut d = Drive::new();
    let (t, rack) = rack_keys(&mut d, true);
    let diff = max_diff(&d.play(FRAMES), &absent);
    assert!(diff < 1e-6, "live differs from no effect by {diff}");
    // Frozen up to the rack (the delay stays live), then the whole track.
    freeze_up_to(&mut d, t, rack);
    let diff = max_diff(&d.play(FRAMES), &absent);
    assert!(diff < 1e-4, "partially frozen playback differs by {diff}");
    d.ok(Command::Freeze(FreezeCommand::Unfreeze { track: t }));
    d.tick();
    d.freeze(t);
    let diff = max_diff(&d.play(FRAMES), &absent);
    assert!(diff < 1e-4, "frozen playback differs by {diff}");
}

// ─── Convert to sampler ─────────────────────────────────────────────────────────────────

fn instrument_rack(d: &mut Drive, track: TrackId) -> (DeviceId, DeviceId) {
    let rack = d.device(track, BuiltinDevice::new(BuiltinDeviceType::InstrumentRack));
    let chain: RackChainId = d.id();
    d.ok(Command::Rack(RackCommand::AddChain {
        id: chain,
        rack,
        name: None,
        before: None,
    }));
    let synth: DeviceId = d.id();
    d.ok(Command::Rack(RackCommand::InsertDevice {
        id: synth,
        chain,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::new(BuiltinDeviceType::Synth),
        },
        before: None,
    }));
    (rack, synth)
}

#[test]
fn converts_an_instrument_rack_to_a_multisampler() {
    let mut d = Drive::new();
    let t = d.track(TrackKind::Midi, "Rack");
    let (rack, _synth) = instrument_rack(&mut d, t);
    // A clip the converted track keeps: its note 61 is between two sampled keys, so it plays
    // the nearest sample pitched.
    d.midi_clip(t, 0.0, &[61]);
    d.settle();
    let new_device: DeviceId = d.id();
    let seed: MediaId = d.id();
    let job = format!("sample{}", d.next);
    assert_eq!(
        d.ok(Command::Freeze(FreezeCommand::ConvertToSampler {
            job: job.clone(),
            device: rack,
            new_device,
            seed_media: seed,
            keys: vec![60, 62, 64],
            velocities: vec![64, 127],
            pre_roll_seconds: Seconds(0.0),
            warm_up_note: false,
            note_seconds: Seconds(0.2),
            tail_seconds: Seconds(0.1),
        })),
        ReplyValue::RenderStarted { job: job.clone() }
    );
    let (end, progress) = d.finish(&job);
    assert_eq!(end, FreezeEvent::Done { job });
    assert_eq!(progress.last(), Some(&1.0));
    assert!(progress.windows(2).all(|w| w[0] <= w[1]), "{progress:?}");
    d.settle();

    let files = {
        let p = d.p();
        assert!(!p.devices.contains_key(&rack), "the rack is replaced");
        assert_eq!(
            p.devices_of(t).iter().map(|d| d.id).collect::<Vec<_>>(),
            vec![new_device],
            "in the rack's place"
        );
        let dev = &p.devices[&new_device];
        assert_eq!(dev.name, "Rack Sampler");
        assert_eq!(dev.params[&ParamId(0)], 0.0, "volume at unity");
        assert_eq!(dev.params[&ParamId(5)], 0.0, "no double velocity scaling");
        let DeviceKind::Builtin {
            device: BuiltinDevice::MultiSampler { zones },
        } = &dev.kind
        else {
            panic!("not a multisampler: {:?}", dev.kind);
        };
        assert_eq!(zones.len(), 6, "3 keys × 2 velocity layers");
        let roots: Vec<u8> = zones.iter().map(|z| z.root_key).collect();
        assert_eq!(roots, vec![60, 60, 62, 62, 64, 64]);
        assert_eq!(
            zones
                .iter()
                .map(|z| (z.velocities.lo, z.velocities.hi))
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([(1, 95), (96, 127)])
        );
        // Every key plays the nearest sample (one zone per velocity layer); the edges cover
        // the keyboard.
        for key in 0..=127u8 {
            assert_eq!(zones.iter().filter(|z| z.keys.contains(key)).count(), 2);
        }
        assert_eq!(zones[0].keys, Zone { lo: 0, hi: 61 });
        assert_eq!(zones[4].keys, Zone { lo: 64, hi: 127 });
        assert_eq!(p.media.len(), 6);
        zones
            .iter()
            .map(|z| {
                let m = p.media.get(&z.media.expect("zone media")).expect("media");
                assert!(m.file.starts_with("media/"));
                assert!(m.frames > 0);
                m.file.clone()
            })
            .collect::<Vec<_>>()
    };
    for file in files {
        assert!(d.ctl.store.read(d.pid, &file).is_ok());
    }
    let heard = d.play(SR as usize / 3);
    assert!(peak(&heard) > 0.001, "the converted track plays");

    d.ok(Command::Edit(EditCommand::Undo));
    d.settle();
    let p = d.p();
    assert!(p.devices.contains_key(&rack), "the rack comes back");
    assert!(!p.devices.contains_key(&new_device));
    assert!(p.media.is_empty(), "the renders go with the undo step");
}

#[test]
fn convert_to_sampler_validates_the_target_and_the_count() {
    let mut d = Drive::new();
    let t = d.track(TrackKind::Midi, "Keys");
    let synth = d.device(t, BuiltinDevice::new(BuiltinDeviceType::Synth));
    let delay = d.device(t, BuiltinDevice::new(BuiltinDeviceType::Delay));
    let cmd = |device: DeviceId, keys: Vec<u8>, velocities: Vec<u8>| {
        Command::Freeze(FreezeCommand::ConvertToSampler {
            job: "x".into(),
            device,
            new_device: DeviceId::NIL,
            seed_media: MediaId::NIL,
            keys,
            velocities,
            pre_roll_seconds: Seconds(0.0),
            warm_up_note: false,
            note_seconds: Seconds(0.2),
            tail_seconds: Seconds(0.1),
        })
    };
    let invalid = |d: &mut Drive, c: Command| {
        assert_eq!(err(&d.send(c)).code, ErrorCode::InvalidArgument);
    };
    // A delay is not an instrument.
    invalid(&mut d, cmd(delay, vec![60], vec![127]));
    // 128 keys × 5 layers is more than MAX_ZONES.
    invalid(
        &mut d,
        cmd(synth, (0..=127).collect(), vec![25, 50, 75, 100, 127]),
    );
    // Bad lists: no key, a key past 127, no velocity, velocity 0, too many velocities.
    invalid(&mut d, cmd(synth, vec![], vec![127]));
    invalid(&mut d, cmd(synth, vec![128], vec![127]));
    invalid(&mut d, cmd(synth, vec![60], vec![]));
    invalid(&mut d, cmd(synth, vec![60], vec![0]));
    invalid(&mut d, cmd(synth, vec![60], (1..=17).collect()));
    // Note and tail lengths: above 0 (the note), finite, at most a minute each.
    let timed = |note: f64, tail: f64| {
        Command::Freeze(FreezeCommand::ConvertToSampler {
            job: "x".into(),
            device: synth,
            new_device: DeviceId::NIL,
            seed_media: MediaId::NIL,
            keys: vec![60],
            velocities: vec![127],
            pre_roll_seconds: Seconds(0.0),
            warm_up_note: false,
            note_seconds: Seconds(note),
            tail_seconds: Seconds(tail),
        })
    };
    for (note, tail) in [
        (0.0, 0.1),
        (61.0, 0.1),
        (1e10, 0.1),
        (f64::INFINITY, 0.1),
        (0.2, -1.0),
        (0.2, 61.0),
        (0.2, 1e9),
        (0.2, f64::NAN),
    ] {
        invalid(&mut d, timed(note, tail));
    }
}

/// Interleaved 32-bit float WAV (the renders) → channels.
fn float_wav(bytes: &[u8]) -> Vec<Vec<f32>> {
    let n_ch = u16::from_le_bytes([bytes[22], bytes[23]]) as usize;
    // Walk the chunks to `data` (a float WAV also has `fact`).
    let mut at = 12;
    let data = loop {
        let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        if &bytes[at..at + 4] == b"data" {
            break &bytes[at + 8..at + 8 + len];
        }
        at += 8 + len + (len & 1);
    };
    let frames = data.len() / 4 / n_ch;
    (0..n_ch)
        .map(|c| {
            (0..frames)
                .map(|i| {
                    let o = (i * n_ch + c) * 4;
                    f32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]])
                })
                .collect()
        })
        .collect()
}

fn onset(x: &[f32]) -> Option<usize> {
    x.iter().position(|s| s.abs() > 1e-4)
}

/// A percussive synth (no attack, 40 ms decay to silence) on a MIDI track (inside an
/// instrument rack after a 4x-oversampled saturator, which adds latency, when `latent`),
/// playing `key` at full velocity at beat 0. Returns the track and the device to convert.
fn percussive(d: &mut Drive, waveform: f64, latent: bool) -> (TrackId, DeviceId) {
    use ether_devices::synth::params as sp;
    let t = d.track(TrackKind::Midi, "Perc");
    let (target, synth) = if latent {
        let (rack, synth) = instrument_rack(d, t);
        let chain = d.p().devices[&synth].chain.expect("in the rack");
        let sat: DeviceId = d.id();
        d.ok(Command::Rack(RackCommand::InsertDevice {
            id: sat,
            chain,
            device: DeviceSpec::Builtin {
                device: BuiltinDevice::new(BuiltinDeviceType::Saturator),
            },
            before: None,
        }));
        (rack, synth)
    } else {
        let synth = d.device(t, BuiltinDevice::new(BuiltinDeviceType::Synth));
        (synth, synth)
    };
    for (param, value) in [
        (sp::WAVEFORM, waveform),
        (sp::ATTACK, 0.0),
        (sp::DECAY, 40.0),
        (sp::SUSTAIN, 0.0),
        (sp::RELEASE, 20.0),
        (sp::CUTOFF, 20_000.0),
    ] {
        d.ok(Command::Device(DeviceCommand::SetParam {
            device: synth,
            param,
            value,
        }));
    }
    d.settle();
    (t, target)
}

/// One note at beat 0 on `track`, full velocity (a new clip; the previous ones removed).
fn one_note(d: &mut Drive, track: TrackId, key: u8) {
    let old: Vec<ClipId> = d
        .p()
        .clips
        .values()
        .filter(|c| c.track == track)
        .map(|c| c.id)
        .collect();
    for id in old {
        d.ok(Command::Clip(ClipCommand::Delete { ids: vec![id] }));
    }
    let clip = d.id();
    d.ok(Command::Clip(ClipCommand::CreateMidi {
        id: clip,
        track,
        start: Beats(0.0),
        length: Beats(4.0),
        name: None,
    }));
    let note = d.id();
    d.ok(Command::Note(NoteCommand::Add {
        clip,
        notes: vec![NoteSpec {
            id: note,
            pitch: key,
            velocity: 1.0,
            start: Beats(0.0),
            duration: Beats(0.25),
        }],
    }));
    d.settle();
}

/// Convert `device` sampling key 60 only (one velocity layer); returns the render.
fn sample_key_60(d: &mut Drive, device: DeviceId) -> Vec<Vec<f32>> {
    sample_key_60_after(d, device, 0.0)
}

/// [`sample_key_60`] with `pre_roll` seconds before the note.
fn sample_key_60_after(d: &mut Drive, device: DeviceId, pre_roll: f64) -> Vec<Vec<f32>> {
    sample_key_60_with(d, device, pre_roll, false)
}

/// [`sample_key_60_after`], with a warm-up note when `warm_up_note`.
fn sample_key_60_with(
    d: &mut Drive,
    device: DeviceId,
    pre_roll: f64,
    warm_up_note: bool,
) -> Vec<Vec<f32>> {
    let new_device: DeviceId = d.id();
    let seed: MediaId = d.id();
    let job = format!("sample{}", d.next);
    d.ok(Command::Freeze(FreezeCommand::ConvertToSampler {
        job: job.clone(),
        device,
        new_device,
        seed_media: seed,
        keys: vec![60],
        velocities: vec![127],
        pre_roll_seconds: Seconds(pre_roll),
        warm_up_note,
        note_seconds: Seconds(0.2),
        tail_seconds: Seconds(0.2),
    }));
    assert_eq!(d.finish(&job).0, FreezeEvent::Done { job });
    d.settle();
    let file = d
        .p()
        .media
        .values()
        .next()
        .expect("one render")
        .file
        .clone();
    float_wav(&d.ctl.store.read(d.pid, &file).unwrap())
}

/// Converting a percussive synth keeps its attack: the render starts on the note's first
/// frame and equals the live instrument sample for sample, and the multisampler adds no
/// fade-in (its default 1 ms amp attack used to take 4.8 dB off the first millisecond), so
/// the converted track plays the note bit-exact.
#[test]
fn convert_to_sampler_keeps_the_attack() {
    let mut d = Drive::new();
    let (t, synth) = percussive(&mut d, 1.0, false);
    one_note(&mut d, t, 60);
    let n = SR as usize / 10;
    let live = d.play(n);
    let sample = sample_key_60(&mut d, synth);
    let after = d.play(n);
    let dev = d.p().devices_of(t)[0].clone();
    assert_eq!(
        dev.params[&ether_devices::multisampler::multi_sampler::AMP_ATTACK],
        0.0,
        "no fade-in"
    );
    for c in 0..2 {
        assert_eq!(onset(&live[c]), Some(1), "the saw's first frame is 0");
        assert_eq!(onset(&sample[c]), onset(&live[c]), "not trimmed, not late");
        assert_eq!(&sample[c][..n], &live[c][..], "the render is the live note");
        assert_eq!(after[c], live[c], "the sampler plays it back bit-exact");
    }
}

/// A bypassed instrument still converts: it is sampled switched on, not as the silence it
/// plays while off.
#[test]
fn converts_a_bypassed_instrument() {
    let mut d = Drive::new();
    let (t, synth) = percussive(&mut d, 1.0, false);
    one_note(&mut d, t, 60);
    let n = SR as usize / 10;
    let live = d.play(n);
    d.ok(Command::Device(DeviceCommand::SetEnabled {
        id: synth,
        enabled: false,
    }));
    let sample = sample_key_60(&mut d, synth);
    for c in 0..2 {
        assert_eq!(&sample[c][..n], &live[c][..], "the instrument, switched on");
    }
}

/// An instrument nested in an instrument rack converts in its rack chain. Its sample is the
/// instrument alone: not the rack's other chain, not its chain's volume (still applied after
/// the sampler), but with the rack macro that moves one of its params baked in.
#[test]
fn converts_an_instrument_nested_in_a_rack() {
    use ether_core::protocol::racks::ModulationCommand;
    use ether_devices::synth::params as sp;
    let mut d = Drive::new();
    let (t, _) = percussive(&mut d, 1.0, false);
    // Replace the plain synth by a rack holding a percussive synth (chain A) and a square
    // synth (chain B).
    let plain = d.p().devices_of(t)[0].id;
    d.ok(Command::Device(DeviceCommand::Remove { id: plain }));
    let (rack, synth) = instrument_rack(&mut d, t);
    for (param, value) in [
        (sp::WAVEFORM, 1.0),
        (sp::ATTACK, 0.0),
        (sp::DECAY, 40.0),
        (sp::SUSTAIN, 0.0),
        (sp::RELEASE, 20.0),
        (sp::CUTOFF, 20_000.0),
    ] {
        d.ok(Command::Device(DeviceCommand::SetParam {
            device: synth,
            param,
            value,
        }));
    }
    let chain_a = d.p().devices[&synth].chain.unwrap();
    let chain_b: RackChainId = d.id();
    d.ok(Command::Rack(RackCommand::AddChain {
        id: chain_b,
        rack,
        name: None,
        before: None,
    }));
    let other: DeviceId = d.id();
    d.ok(Command::Rack(RackCommand::InsertDevice {
        id: other,
        chain: chain_b,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::new(BuiltinDeviceType::Synth),
        },
        before: None,
    }));
    // Macro 1 at 0.8 turns the synth's volume down (depth -0.5).
    let mapping: ModMappingId = d.id();
    d.ok(Command::Modulation(ModulationCommand::Map {
        id: mapping,
        source: ModSource::Macro { rack, index: 0 },
        device: synth,
        param: sp::VOLUME,
        depth: -0.5,
    }));
    d.ok(Command::Device(DeviceCommand::SetParam {
        device: rack,
        param: ether_model::rack_macro_param(0),
        value: 0.8,
    }));
    // The reference: chain A alone, at unity, as it plays live (macro included).
    let mute_b = |d: &mut Drive, mute: bool| {
        d.ok(Command::Rack(RackCommand::SetChainMix {
            id: chain_b,
            volume: None,
            pan: None,
            mute: Some(mute),
            solo: None,
        }))
    };
    mute_b(&mut d, true);
    one_note(&mut d, t, 60);
    let n = SR as usize / 10;
    let live = d.play(n);
    // Chain B playing, chain A turned down: neither is in the sample.
    mute_b(&mut d, false);
    d.ok(Command::Rack(RackCommand::SetChainMix {
        id: chain_a,
        volume: Some(Decibels(-6.0)),
        pan: None,
        mute: None,
        solo: None,
    }));
    d.settle();
    let sample = sample_key_60(&mut d, synth);
    // Live playback glides the synth's volume from its unmodulated value to the macro's
    // when it starts; the sample has the macro's value from its first frame. Past that glide
    // they match.
    let from = SR as usize / 50;
    for c in 0..2 {
        let level = peak(std::slice::from_ref(&live[c]));
        assert!(level > 0.01, "a sound");
        let diff = live[c][from..]
            .iter()
            .zip(&sample[c][from..])
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(
            diff < 0.01 * level,
            "ch{c}: the synth alone, macro baked in ({diff} of {level})"
        );
    }
    // The sampler took the synth's place in chain A; chain B and the rack are untouched.
    let p = d.p();
    assert!(!p.devices.contains_key(&synth));
    let in_a: Vec<&Device> = p
        .devices
        .values()
        .filter(|x| x.chain == Some(chain_a))
        .collect();
    assert_eq!(in_a.len(), 1);
    assert!(matches!(
        &in_a[0].kind,
        DeviceKind::Builtin {
            device: BuiltinDevice::MultiSampler { .. }
        }
    ));
    assert_eq!(p.devices[&other].chain, Some(chain_b));
    assert_eq!(
        p.devices_of(t).iter().map(|x| x.id).collect::<Vec<_>>(),
        vec![rack]
    );
}

/// A pre-roll: the render starts that long before its note (the instrument already running),
/// and the multisampler's `Offset` skips it, so the converted track still plays the note
/// bit-exact from its first frame.
#[test]
fn convert_to_sampler_pre_roll_is_skipped_by_the_offset() {
    let mut d = Drive::new();
    let (t, synth) = percussive(&mut d, 1.0, false);
    one_note(&mut d, t, 60);
    let n = SR as usize / 10;
    let live = d.play(n);
    let sample = sample_key_60_after(&mut d, synth, 0.2);
    let pre = SR as usize / 5;
    let after = d.play(n);
    let dev = d.p().devices_of(t)[0].clone();
    assert_eq!(
        dev.params[&ether_devices::multisampler::multi_sampler::OFFSET],
        200.0
    );
    for c in 0..2 {
        assert_eq!(onset(&sample[c]), Some(pre + 1), "the note is 0.2 s in");
        assert_eq!(&sample[c][pre..pre + n], &live[c][..], "then the live note");
        assert_eq!(after[c], live[c], "played back from the note, bit-exact");
    }
}

/// A plugin still loading its sound for a while after it is created ([`Loading`]: silent
/// for its first 100 ms of real time) gets the offset as real time before its note: the
/// render holds before the note until then, so the sample has the note from its first
/// frame. Without an offset there's no wait, and the note is rendered silent.
#[test]
fn convert_to_sampler_offset_waits_for_plugins_in_real_time() {
    let mut d = Drive::new();
    let t = d.track(TrackKind::Midi, "Plug");
    let plug: DeviceId = d.id();
    d.ok(Command::Device(DeviceCommand::Insert {
        id: plug,
        track: t,
        device: DeviceSpec::Plugin {
            plugin_id: "loading".into(),
            sandboxed: None,
            format: None,
            name: None,
        },
        before: None,
    }));
    d.settle();
    let started = std::time::Instant::now();
    let sample = sample_key_60_after(&mut d, plug, 0.2);
    assert!(
        started.elapsed() >= std::time::Duration::from_millis(200),
        "it waited"
    );
    let pre = SR as usize / 5;
    for (c, ch) in sample.iter().enumerate() {
        assert_eq!(
            onset(ch),
            Some(pre),
            "ch{c}: the note, at full level, on its frame"
        );
        assert!(
            ch[pre..pre + SR as usize / 10]
                .iter()
                .all(|s| (s - 0.5).abs() < 1e-6)
        );
    }
    // The same plugin without an offset: no wait, the note comes before it is ready.
    let mut d = Drive::new();
    let t = d.track(TrackKind::Midi, "Plug");
    let plug: DeviceId = d.id();
    d.ok(Command::Device(DeviceCommand::Insert {
        id: plug,
        track: t,
        device: DeviceSpec::Plugin {
            plugin_id: "loading".into(),
            sandboxed: None,
            format: None,
            name: None,
        },
        before: None,
    }));
    d.settle();
    let sample = sample_key_60_after(&mut d, plug, 0.0);
    assert!(
        onset(&sample[0]).is_none(),
        "rendered before the plugin was ready"
    );
}

/// The warm-up note: with a pre-roll, the key also plays for 5 ms at the render's start (in
/// the lead-in the sampler's `Offset` skips), and the note still plays on its frame.
#[test]
fn convert_to_sampler_plays_a_warm_up_note_in_the_pre_roll() {
    let mut d = Drive::new();
    let (t, synth) = percussive(&mut d, 1.0, false);
    one_note(&mut d, t, 60);
    let n = SR as usize / 10;
    let live = d.play(n);
    let sample = sample_key_60_with(&mut d, synth, 0.2, true);
    let pre = SR as usize / 5;
    for c in 0..2 {
        // The warm-up from the start, then (decayed: 40 ms) silence before the note.
        assert_eq!(onset(&sample[c]), Some(1), "the warm-up note at the start");
        assert!(
            peak(&[sample[c][pre - SR as usize / 50..pre].to_vec()]) < 1e-3,
            "silent before the note"
        );
        let diff = live[c]
            .iter()
            .zip(&sample[c][pre..pre + n])
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(diff < 1e-6, "ch{c}: then the note, as live ({diff})");
    }
}

/// A clip's pitch-bend lane bends a built-in instrument like a plugin: through an
/// instrument rack and a MIDI effect in front of the synth, a full bend up plays the note
/// 2 semitones up (the default `Bend Range`).
#[test]
fn a_clip_bend_lane_bends_a_builtin_inside_a_rack() {
    use ether_core::protocol::expression::ExpressionCommand;
    use ether_devices::synth::params as sp;
    let mut d = Drive::new();
    let t = d.track(TrackKind::Midi, "Bend");
    let (_rack, synth) = instrument_rack(&mut d, t);
    let chain = d.p().devices[&synth].chain.unwrap();
    let pitch: DeviceId = d.id();
    d.ok(Command::Rack(RackCommand::InsertDevice {
        id: pitch,
        chain,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::new(BuiltinDeviceType::Pitch),
        },
        before: Some(synth),
    }));
    for (param, value) in [
        (sp::WAVEFORM, 0.0),
        (sp::CUTOFF, 20_000.0),
        (sp::SUSTAIN, 100.0),
    ] {
        d.ok(Command::Device(DeviceCommand::SetParam {
            device: synth,
            param,
            value,
        }));
    }
    let clip = d.midi_clip(t, 0.0, &[69]);
    let hz = |d: &mut Drive| {
        d.settle();
        let out = d.play(SR as usize / 4);
        let x = &out[0][SR as usize / 20..];
        let ups: Vec<usize> = (1..x.len())
            .filter(|&i| x[i - 1] < 0.0 && x[i] >= 0.0)
            .collect();
        (ups.len() - 1) as f32 * SR as f32 / (ups[ups.len() - 1] - ups[0]) as f32
    };
    let plain = hz(&mut d);
    assert!((plain - 440.0).abs() < 2.0, "unbent: {plain}");
    let lane: ExpressionLaneId = d.id();
    d.ok(Command::Expression(ExpressionCommand::CreateLane {
        id: lane,
        clip,
        kind: ExpressionKind::PitchBend,
    }));
    d.ok(Command::Expression(ExpressionCommand::SetPoints {
        lane,
        points: vec![ExpressionPoint {
            time: Beats(0.0),
            value: 1.0,
            curve: CurveShape::Step,
        }],
    }));
    let bent = hz(&mut d);
    let up2 = 440.0 * 2f32.powf(2.0 / 12.0);
    assert!((bent - up2).abs() < 2.0, "bent: {bent} (expected {up2})");
}

/// An instrument with latency (a rack with an oversampled saturator after the synth): the
/// render drops it, so the sample starts on the note's frame like an unlatent one.
#[test]
fn convert_to_sampler_compensates_the_latency() {
    let latency = ether_devices::fx_color::saturator_device::LATENCY;
    let mut d = Drive::new();
    let (t, rack) = percussive(&mut d, 1.0, true);
    one_note(&mut d, t, 60);
    let n = SR as usize / 10;
    let live = d.play(n + latency);
    let sample = sample_key_60(&mut d, rack);
    for c in 0..2 {
        let at = onset(&sample[c]).expect("a sound");
        assert!(at <= 1, "the sample starts on the note (frame {at})");
        // The live engine delays the track by the latency; the render is that signal
        // without the delay (the saturator's linear-phase filters ring a little ahead).
        let diff = live[c][latency..]
            .iter()
            .zip(&sample[c])
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(diff < 1e-6, "ch{c}: {diff}");
    }
}

/// Keys between samples repitch the nearest one (plain resampling, never pitch
/// preservation): an octave up plays twice as fast, so the decay is twice as short and the
/// pitch an octave higher.
#[test]
fn convert_to_sampler_repitches_without_preserving_time() {
    let mut d = Drive::new();
    let (t, synth) = percussive(&mut d, 0.0, false);
    // A longer decay: enough cycles to count.
    d.ok(Command::Device(DeviceCommand::SetParam {
        device: synth,
        param: ether_devices::synth::params::DECAY,
        value: 150.0,
    }));
    one_note(&mut d, t, 60);
    let _ = sample_key_60(&mut d, synth);
    let n = SR as usize / 2;
    let play = |d: &mut Drive, key: u8| {
        one_note(d, t, key);
        d.play(n)[0].clone()
    };
    let root = play(&mut d, 60);
    let up = play(&mut d, 72);
    // Time to fall 40 dB below the peak, and positive-going zero crossings (pitch).
    let fall = |x: &[f32]| {
        let peak = x.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        let last = x.iter().rposition(|s| s.abs() > peak * 0.01).unwrap();
        last as f64
    };
    let crossings = |x: &[f32], frames: usize| {
        x[..frames]
            .windows(2)
            .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
            .count() as f64
    };
    let ratio = fall(&root) / fall(&up);
    assert!((ratio - 2.0).abs() < 0.1, "decay ratio {ratio}");
    let rate = |x: &[f32]| crossings(x, fall(x) as usize) / fall(x);
    let pitch = rate(&up) / rate(&root);
    assert!((pitch - 2.0).abs() < 0.15, "pitch ratio {pitch}");
}

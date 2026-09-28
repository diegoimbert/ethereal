//! Freeze, flatten, bounce and consolidate (`freeze-bounce`, CONTRACTS.md §12.3), against a
//! real engine behind the bridge: a frozen track sounds like the live one (also under a
//! tempo ramp), its devices get no nodes, it can't be edited except its mixer, unfreeze and
//! undo restore it, flatten/bounce/consolidate produce clips that play the render, and
//! the job protocol (progress, one job at a time, cancel, project closed).

mod common;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use common::*;
use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::store::ProjectStore;
use ether_controller::{
    BridgeError, Controller, ControllerConfig, EngineBridge, EtherController, HostServices,
};
use ether_core::protocol::automation::{AutomationCommand, PointSpec};
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::devices::{DeviceCommand, DeviceDescriptor, DeviceSpec};
use ether_core::protocol::freeze::{BounceTarget, FreezeCommand, FreezeEvent};
use ether_core::protocol::media::{BrowseLocation, MediaCommand, MediaSource};
use ether_core::protocol::mixer::MixerCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteSpec};
use ether_core::protocol::project::{EditCommand, ProjectCommand};
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
        _: DeviceId,
        _: &PluginInstance,
        _: Option<&Base64Bytes>,
    ) -> Result<NodeKey, BridgeError> {
        Err(BridgeError::Unsupported("no plugins".into()))
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
    assert_eq!(d.p().media[&media].frames, 4 * SR as u64);
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
    }));
    assert_eq!(e.code, ErrorCode::InvalidArgument);
    let mid: MediaId = d.id();
    let e = d.err(Command::Freeze(FreezeCommand::Freeze {
        job: "x".into(),
        track: group,
        media: mid,
    }));
    assert_eq!(e.code, ErrorCode::InvalidArgument);
    let mid: MediaId = d.id();
    d.ok(Command::Freeze(FreezeCommand::Freeze {
        job: "j1".into(),
        track: t,
        media: mid,
    }));
    let mid: MediaId = d.id();
    let e = d.err(Command::Freeze(FreezeCommand::Freeze {
        job: "j2".into(),
        track: t,
        media: mid,
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

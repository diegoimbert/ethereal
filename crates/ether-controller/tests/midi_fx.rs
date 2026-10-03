//! v0.2 `midi-fx` controller side: MIDI effects precede the instrument
//! (`check_chain_order`), and Scale Quantize / Random get the resolved `MusicalScale`
//! (`EngineBridge::set_node_scale` on creation and on track/project scale or `Scale`
//! source changes; the offline export pushes it too, BCR-C). The live path and the export
//! run a real engine, so "the scale arrived" is checked by ear: quantized notes render
//! exactly like the in-scale notes played directly.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use common::*;
use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::store::ProjectStore;
use ether_controller::{BridgeError, Controller, ControllerConfig, EngineBridge, EtherController};
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::devices::{DeviceCommand, DeviceDescriptor, DeviceSpec};
use ether_core::protocol::export::*;
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteSpec};
use ether_core::protocol::project::ProjectCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::transport::TransportCommand;
use ether_core::protocol::*;
use ether_core::{
    AudioSource, Engine, EngineConfig, EngineHandle, EngineOutputs, GarbageCollector, NodeKey,
    ParamChange, RenderGraphDesc, TransportControl,
};
use ether_devices::midi_fx::scale_quantize;
use ether_media::DecodedAudio;

const SR: u32 = 48_000;

// ─── chain order ────────────────────────────────────────────────────────────────────────

fn insert(
    h: &mut Harness,
    track: TrackId,
    ty: BuiltinDeviceType,
    before: Option<DeviceId>,
) -> (DeviceId, Vec<ServerMessage>) {
    let id: DeviceId = h.id();
    let out = h.send(Command::Device(DeviceCommand::Insert {
        id,
        track,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::new(ty),
        },
        before,
    }));
    (id, out)
}

fn midi_track(h: &mut Harness) -> TrackId {
    let id: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id,
        kind: TrackKind::Midi,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    id
}

#[test]
fn midi_effects_must_precede_the_instrument() {
    let mut h = Harness::with_project();
    let t = midi_track(&mut h);
    let (synth, out) = insert(&mut h, t, BuiltinDeviceType::Synth, None);
    ok(&out);
    // After the instrument: rejected, document untouched.
    let before = h.project().clone();
    let (_, out) = insert(&mut h, t, BuiltinDeviceType::Arpeggiator, None);
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    assert_eq!(h.project(), &before);
    // Before it: fine, and audio effects may still follow the instrument.
    let (arp, out) = insert(&mut h, t, BuiltinDeviceType::Arpeggiator, Some(synth));
    ok(&out);
    let (_, out) = insert(&mut h, t, BuiltinDeviceType::Chord, Some(synth));
    ok(&out);
    let (_, out) = insert(&mut h, t, BuiltinDeviceType::Delay, None);
    ok(&out);
    // Moving a MIDI effect behind the instrument: rejected.
    let before = h.project().clone();
    let out = h.send(Command::Device(DeviceCommand::Move {
        id: arp,
        track: t,
        before: None,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    assert_eq!(h.project(), &before);
    // The chain compiles in order: arpeggiator, chord, synth, delay.
    h.tick();
    let graph = h.ctl.bridge.last_graph();
    assert_eq!(
        graph.tracks.iter().find(|x| x.id == t).unwrap().chain.len(),
        4
    );
}

// ─── a real engine behind the bridge, recording scale pushes ────────────────────────────

struct NoSources;
impl ether_devices::SampleResolver for NoSources {
    fn resolve(&self, _: MediaId) -> Option<Arc<dyn AudioSource>> {
        None
    }
}

struct RealBridge {
    engine: Engine,
    handle: EngineHandle,
    gc: GarbageCollector,
    nodes: BTreeMap<DeviceId, (NodeKey, BuiltinDeviceType)>,
    /// Every `set_node_scale`, in order.
    scales: Vec<(DeviceId, MusicalScale)>,
}

impl RealBridge {
    fn new() -> Self {
        let parts = ether_core::create(EngineConfig {
            sample_rate: SR,
            max_block_size: ether_core::offline::OFFLINE_MAX_BLOCK,
            ..Default::default()
        });
        Self {
            engine: parts.engine,
            handle: parts.handle,
            gc: parts.gc,
            nodes: BTreeMap::new(),
            scales: Vec::new(),
        }
    }

    fn render(&mut self, frames: usize) -> Vec<Vec<f32>> {
        let block = ether_core::offline::OFFLINE_MAX_BLOCK;
        let (mut l, mut r) = (vec![0.0f32; block], vec![0.0f32; block]);
        let mut out = vec![Vec::new(), Vec::new()];
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

fn other(e: impl ToString) -> BridgeError {
    BridgeError::Other(e.to_string())
}

impl EngineBridge for RealBridge {
    fn create_builtin(
        &mut self,
        device: DeviceId,
        kind: &BuiltinDevice,
        params: &[(ParamId, f64)],
    ) -> Result<NodeKey, BridgeError> {
        let mut node = ether_devices::create(kind, &NoSources);
        for (id, v) in params {
            node.set_param(*id, *v);
        }
        let key = self.handle.add_node(node).map_err(other)?;
        self.nodes.insert(device, (key, kind.device_type()));
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
        self.nodes.retain(|_, (k, _)| *k != key);
        self.handle.remove_node(key).map_err(other)
    }
    fn load_media(&mut self, _: &MediaRef, _: Arc<DecodedAudio>) -> Result<(), BridgeError> {
        Ok(())
    }
    fn unload_media(&mut self, _: MediaId) -> Result<(), BridgeError> {
        Ok(())
    }
    fn publish(&mut self, graph: RenderGraphDesc) -> Result<(), BridgeError> {
        self.handle.publish(graph).map_err(other)
    }
    fn set_param(&mut self, change: ParamChange) -> Result<(), BridgeError> {
        self.handle.set_param(change).map_err(other)
    }
    fn transport(&mut self, control: TransportControl) -> Result<(), BridgeError> {
        self.handle.transport(control).map_err(other)
    }
    fn poll(&mut self, out: &mut EngineOutputs) {
        self.handle.poll(out);
    }
    fn descriptor(&mut self, device: DeviceId) -> Option<DeviceDescriptor> {
        self.nodes
            .get(&device)
            .map(|(_, t)| ether_devices::descriptor(*t))
    }
    fn set_node_scale(
        &mut self,
        device: DeviceId,
        scale: MusicalScale,
    ) -> Result<bool, BridgeError> {
        let Some(&(key, _)) = self.nodes.get(&device) else {
            return Ok(false);
        };
        self.scales.push((device, scale));
        self.handle
            .set_node_data(key, Box::new(scale))
            .map_err(other)?;
        Ok(true)
    }
}

struct Drive {
    ctl: EtherController<RealBridge, FakeHost, MemoryStore, MemoryLibrary>,
    ids: IdGen,
    next: u32,
    pid: ProjectId,
}

impl Drive {
    fn new() -> Self {
        let mut store = MemoryStore::new();
        store.now_ms = T0;
        let mut d = Self {
            ctl: EtherController::with_config(
                RealBridge::new(),
                FakeHost { now: T0 },
                store,
                MemoryLibrary::new(),
                ControllerConfig {
                    engine_sample_rate: SR,
                    ..Default::default()
                },
            ),
            ids: IdGen::new(11),
            next: 1,
            pid: IdGen::new(1).next_project_id(T0),
        };
        d.pid = d.ids.next_project_id(T0);
        let pid = d.pid;
        d.ok(Command::Project(ProjectCommand::Create {
            id: pid,
            name: "Song".into(),
        }));
        d
    }

    fn id<I: Id>(&mut self) -> I {
        self.ids.next(T0)
    }

    fn ok(&mut self, command: Command) -> ReplyValue {
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
        ok(&out)
    }

    fn tick(&mut self) -> Vec<ServerMessage> {
        let mut out = Vec::new();
        self.ctl.host.now += 16;
        let now = self.ctl.host.now;
        self.ctl.tick(now, &mut out);
        out
    }

    fn track(&mut self) -> TrackId {
        let id = self.id();
        self.ok(Command::Track(TrackCommand::Create {
            id,
            kind: TrackKind::Midi,
            name: None,
            color: None,
            parent: None,
            before: None,
        }));
        id
    }

    fn device(&mut self, track: TrackId, ty: BuiltinDeviceType) -> DeviceId {
        let id = self.id();
        self.ok(Command::Device(DeviceCommand::Insert {
            id,
            track,
            device: DeviceSpec::Builtin {
                device: BuiltinDevice::new(ty),
            },
            before: None,
        }));
        id
    }

    fn notes(&mut self, track: TrackId, pitches: &[u8]) {
        let clip = self.id();
        self.ok(Command::Clip(ClipCommand::CreateMidi {
            id: clip,
            track,
            start: Beats(0.0),
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
                start: Beats(i as f64),
                duration: Beats(0.5),
            })
            .collect();
        self.ok(Command::Note(NoteCommand::Add { clip, notes }));
    }

    /// The pushes since the last call.
    fn pushed(&mut self) -> Vec<(DeviceId, MusicalScale)> {
        std::mem::take(&mut self.ctl.bridge.scales)
    }

    /// Export beats 0..4 as float WAV (the offline engine, BCR-C).
    fn export(&mut self) -> Vec<Vec<f32>> {
        let job = format!("job{}", self.next);
        self.ok(Command::Export(ExportCommand::Render {
            job: job.clone(),
            request: ExportRequest {
                range: ExportRange::Custom {
                    start: Beats(0.0),
                    end: Beats(4.0),
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
        for _ in 0..100_000 {
            for e in events(&self.tick()) {
                let Event::Export { event } = e else {
                    continue;
                };
                match event {
                    ExportEvent::Progress { .. } => {}
                    ExportEvent::Done {
                        result: ExportResult::Files { files },
                        ..
                    } => {
                        let bytes = self.ctl.store.read(self.pid, &files[0]).unwrap();
                        return ether_media::decode(&bytes, Some("wav")).unwrap().channels;
                    }
                    other => panic!("export: {other:?}"),
                }
            }
        }
        panic!("export did not finish");
    }

    /// Play the live engine from 0 for beats 0..4 (120 bpm).
    fn live(&mut self) -> Vec<Vec<f32>> {
        self.tick();
        self.ok(Command::Transport(TransportCommand::Play));
        self.ctl.bridge.render(4 * SR as usize / 2)
    }
}

fn c_major() -> MusicalScale {
    MusicalScale {
        root: 0,
        kind: ScaleKind::Major,
    }
}

fn assert_same(a: &[Vec<f32>], b: &[Vec<f32>], what: &str) {
    let peak = a.iter().flatten().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(peak > 0.01, "{what}: audible");
    for c in 0..2 {
        let n = a[c].len().min(b[c].len());
        assert!(n > SR as usize, "{what}: rendered");
        let max = (0..n)
            .map(|i| (a[c][i] - b[c][i]).abs())
            .fold(0.0, f32::max);
        assert!(max < 1e-6, "{what}: channel {c} differs by {max}");
    }
}

/// The reference: the in-scale notes played directly into a synth.
fn reference(pitches: &[u8]) -> (Vec<Vec<f32>>, Vec<Vec<f32>>) {
    let mut d = Drive::new();
    let t = d.track();
    d.device(t, BuiltinDeviceType::Synth);
    d.notes(t, pitches);
    d.tick();
    let offline = d.export();
    (offline, d.live())
}

#[test]
fn scale_quantize_gets_the_track_scale_live_and_in_export() {
    // C# and F# snap down (ties go down) to C and F in C major.
    let (ref_offline, ref_live) = reference(&[60, 65]);

    let mut d = Drive::new();
    d.ok(Command::Project(ProjectCommand::SetScale {
        scale: c_major(),
    }));
    let t = d.track();
    let sq = d.device(t, BuiltinDeviceType::ScaleQuantize);
    d.device(t, BuiltinDeviceType::Synth);
    d.notes(t, &[61, 66]);
    d.tick();
    // Pushed on creation: the track follows the project scale.
    assert_eq!(d.pushed(), [(sq, c_major())]);
    // Nothing changed: no new push.
    d.tick();
    assert!(d.pushed().is_empty());

    let offline = d.export();
    assert_same(&offline, &ref_offline, "export");
    let live = d.live();
    assert_same(&live, &ref_live, "live");
}

#[test]
fn scale_pushes_follow_track_project_and_source_changes() {
    let mut d = Drive::new();
    let t = d.track();
    let sq = d.device(t, BuiltinDeviceType::ScaleQuantize);
    let rnd = d.device(t, BuiltinDeviceType::Randomizer);
    let vel = d.device(t, BuiltinDeviceType::Velocity);
    d.tick();
    let first = d.pushed();
    let project = d.ctl.project().unwrap().settings.scale;
    assert!(first.contains(&(sq, project)) && first.contains(&(rnd, project)));
    assert!(
        !first.iter().any(|(id, _)| *id == vel),
        "Velocity takes no scale"
    );

    // Project scale edit: both follow (the track follows the project).
    d.ok(Command::Project(ProjectCommand::SetScale {
        scale: c_major(),
    }));
    d.tick();
    let mut got = d.pushed();
    got.sort_by_key(|(id, _)| *id);
    let mut want = vec![(sq, c_major()), (rnd, c_major())];
    want.sort_by_key(|(id, _)| *id);
    assert_eq!(got, want);

    // Track scale edit.
    let d_dorian = MusicalScale {
        root: 2,
        kind: ScaleKind::Dorian,
    };
    d.ok(Command::Track(TrackCommand::SetScale {
        id: t,
        scale: TrackScale::Custom { scale: d_dorian },
    }));
    d.tick();
    let mut got = d.pushed();
    got.sort_by_key(|(id, _)| *id);
    let mut want = vec![(sq, d_dorian), (rnd, d_dorian)];
    want.sort_by_key(|(id, _)| *id);
    assert_eq!(got, want);

    // Scale Quantize's `Scale` source = Project: the project scale, not the track's.
    d.ok(Command::Device(DeviceCommand::SetParam {
        device: sq,
        param: scale_quantize::SOURCE,
        value: 1.0,
    }));
    d.tick();
    assert_eq!(d.pushed(), [(sq, c_major())]);

    // A chromatic track: chromatic.
    d.ok(Command::Track(TrackCommand::SetScale {
        id: t,
        scale: TrackScale::Chromatic,
    }));
    d.tick();
    assert_eq!(d.pushed(), [(rnd, MusicalScale::default())]);
}

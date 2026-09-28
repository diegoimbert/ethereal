//! `latency-republish`: when a built-in node's reported latency changes mid-session (a
//! lookahead param), the controller tick republishes the graph so PDC keeps the track aligned
//! with the others; with stable latencies the tick never republishes.
//!
//! Runs against a real engine behind the bridge. The Gate device is backed by a test node
//! (a pure delay whose delay and reported latency follow the Gate's `Lookahead` param), so
//! the test depends on nothing but the latency contract.

mod common;

use std::collections::HashMap;
use std::sync::Arc;

use common::*;
use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::{
    BridgeError, Controller, ControllerConfig, EngineBridge, EtherController, HostServices,
};
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::devices::{DeviceCommand, DeviceDescriptor, DeviceSpec};
use ether_core::protocol::media::{BrowseLocation, MediaCommand, MediaSource};
use ether_core::protocol::mixer::MixerCommand;
use ether_core::protocol::project::ProjectCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::transport::TransportCommand;
use ether_core::protocol::*;
use ether_core::{
    AudioSource, Engine, EngineConfig, EngineHandle, EngineOutputs, EventKind, GarbageCollector,
    NodeKey, ParamChange, RenderGraphDesc, TransportControl,
};
use ether_media::{DecodedAudio, InMemorySource};

const SR: u32 = 48_000;
const BLOCK: usize = 256;
/// The Gate's `Lookahead` param (ms).
const LOOKAHEAD: ParamId = ParamId(8);
const MAX_DELAY: usize = 4096;

// ─── Test node: a delay whose latency follows a param ────────────────────────────────────

struct Lookahead {
    ring: [Vec<f32>; 2],
    pos: usize,
    delay: usize,
}

impl Lookahead {
    fn new() -> Self {
        Self {
            ring: [vec![0.0; MAX_DELAY], vec![0.0; MAX_DELAY]],
            pos: 0,
            delay: 0,
        }
    }
    fn set_ms(&mut self, ms: f64) {
        self.delay = ((ms * 0.001 * SR as f64).round() as usize).min(MAX_DELAY - 1);
    }
}

impl ether_core::Node for Lookahead {
    fn prepare(&mut self, _: &ether_core::PrepareConfig) {}
    fn reset(&mut self) {
        for r in &mut self.ring {
            r.fill(0.0);
        }
    }
    fn process(
        &mut self,
        ctx: &mut ether_core::ProcessContext<'_>,
        audio: &mut ether_core::AudioBuffers<'_, '_>,
    ) -> ether_core::ProcessStatus {
        for e in ctx.events {
            if let EventKind::Param { param, value } = e.kind
                && param == LOOKAHEAD
            {
                self.set_ms(value);
            }
        }
        let start = self.pos;
        for (c, (o, i)) in audio
            .outputs
            .iter_mut()
            .zip(audio.inputs.iter())
            .enumerate()
            .take(2)
        {
            let ring = &mut self.ring[c];
            let mut pos = start;
            for (o, i) in o.iter_mut().zip(i.iter()) {
                ring[pos] = *i;
                *o = ring[(pos + MAX_DELAY - self.delay) % MAX_DELAY];
                pos = (pos + 1) % MAX_DELAY;
            }
        }
        self.pos = (start + ctx.frames) % MAX_DELAY;
        ether_core::ProcessStatus::Continue
    }
    fn latency(&self) -> u32 {
        self.delay as u32
    }
}

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
    publishes: usize,
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
            publishes: 0,
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

fn other(e: impl std::fmt::Display) -> BridgeError {
    BridgeError::Other(e.to_string())
}

impl EngineBridge for RealBridge {
    fn create_builtin(
        &mut self,
        _: DeviceId,
        kind: &BuiltinDevice,
        params: &[(ParamId, f64)],
    ) -> Result<NodeKey, BridgeError> {
        let node: Box<dyn ether_core::Node> = if matches!(kind, BuiltinDevice::Gate) {
            let mut n = Lookahead::new();
            if let Some((_, ms)) = params.iter().find(|(p, _)| *p == LOOKAHEAD) {
                n.set_ms(*ms);
            }
            Box::new(n)
        } else {
            let mut node = ether_devices::create(kind, &Sources(&self.sources));
            for (id, v) in params {
                node.set_param(*id, *v);
            }
            node
        };
        self.handle.add_node(node).map_err(other)
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
        self.handle.remove_node(key).map_err(other)
    }
    fn load_media(&mut self, media: &MediaRef, audio: Arc<DecodedAudio>) -> Result<(), BridgeError> {
        let src: Arc<dyn AudioSource> = Arc::new(InMemorySource::new(audio));
        self.sources.insert(media.id, src.clone());
        self.handle.add_source(media.id, src).map_err(other)
    }
    fn unload_media(&mut self, media: MediaId) -> Result<(), BridgeError> {
        self.sources.remove(&media);
        self.handle.remove_source(media).map_err(other)
    }
    fn publish(&mut self, graph: RenderGraphDesc) -> Result<(), BridgeError> {
        self.publishes += 1;
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
    fn descriptor(&mut self, _: DeviceId) -> Option<DeviceDescriptor> {
        None
    }
    fn node_latency(&self, key: NodeKey) -> Option<u32> {
        self.handle.node_latency(key)
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
    gate: DeviceId,
    a: TrackId,
    b: TrackId,
}

impl Drive {
    /// Track A: tone clip → Gate (lookahead `ms`). Track B (reference): the same clip, dry.
    fn new(ms: f64) -> Self {
        let mut lib = MemoryLibrary::new();
        lib.add_root("lib", "Library");
        // A tone with a sharp onset (after silence) and a changing pitch: misalignment of
        // even a few samples shows.
        let mut tone = vec![0.0f32; 4800];
        tone.extend(sine(SR, 330.0, SR as usize / 2, 0.4));
        tone.extend(sine(SR, 517.0, SR as usize / 2, 0.3));
        lib.add_file("lib", "tone.wav", wav(SR, &[tone.clone(), tone]));
        let mut ids = IdGen::new(7);
        let (gate, a, b) = (ids.next(T0), ids.next(T0), ids.next(T0));
        let mut d = Self {
            ctl: EtherController::with_config(
                RealBridge::new(),
                TestHost { now: T0 },
                MemoryStore::new(),
                lib,
                ControllerConfig {
                    engine_sample_rate: SR,
                    ..Default::default()
                },
            ),
            ids,
            next: 1,
            gate,
            a,
            b,
        };
        let pid = d.ids.next_project_id(T0);
        d.ok(Command::Project(ProjectCommand::Create {
            id: pid,
            name: "Song".into(),
        }));
        let media: MediaId = d.id();
        d.ok(Command::Media(MediaCommand::Import {
            id: media,
            source: MediaSource::Location {
                location: BrowseLocation::Library { id: "lib".into() },
                path: "tone.wav".into(),
            },
        }));
        d.track(a, "A");
        d.track(b, "B");
        for t in [d.a, d.b] {
            let clip = d.id();
            d.ok(Command::Clip(ClipCommand::CreateAudio {
                id: clip,
                track: t,
                start: Beats(0.0),
                media,
            }));
        }
        d.ok(Command::Device(DeviceCommand::Insert {
            id: d.gate,
            track: d.a,
            device: DeviceSpec::Builtin {
                device: BuiltinDevice::Gate,
            },
            before: None,
        }));
        d.ok(Command::Device(DeviceCommand::SetParam {
            device: d.gate,
            param: LOOKAHEAD,
            value: ms,
        }));
        for _ in 0..10_000 {
            d.tick();
            if !d.ctl.media_pending() {
                break;
            }
        }
        assert!(!d.ctl.media_pending(), "media did not load");
        d.idle();
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

    fn track(&mut self, id: TrackId, name: &str) {
        self.ok(Command::Track(TrackCommand::Create {
            id,
            kind: TrackKind::Audio,
            name: Some(name.into()),
            color: None,
            parent: None,
            before: None,
        }));
    }

    fn tick(&mut self) {
        self.ctl.host.now += 16;
        let now = self.ctl.host.now;
        self.ctl.tick(now, &mut Vec::new());
    }

    /// Let the engine and the controller settle: blocks (param changes reach nodes, which
    /// report their latency) interleaved with ticks, well past the republish debounce.
    fn idle(&mut self) {
        for _ in 0..12 {
            self.ctl.bridge.render(BLOCK);
            self.tick();
        }
    }

    fn mute(&mut self, track: TrackId, mute: bool) {
        self.ok(Command::Mixer(MixerCommand::SetMute { track, mute }));
    }

    /// Play `frames` from beat 0 with only `track` audible.
    fn play_only(&mut self, track: TrackId) -> Vec<f32> {
        let (a, b) = (self.a, self.b);
        self.mute(a, track != a);
        self.mute(b, track != b);
        self.ok(Command::Transport(TransportCommand::Stop));
        self.ok(Command::Transport(TransportCommand::Locate {
            position: Beats(0.0),
        }));
        self.ctl.bridge.render(BLOCK);
        self.ok(Command::Transport(TransportCommand::Play));
        let out = self.ctl.bridge.render(SR as usize / 4);
        self.ok(Command::Transport(TransportCommand::Stop));
        self.ctl.bridge.render(BLOCK);
        out[0].clone()
    }

    /// Onset (first sample above -40 dBFS) of `track` alone and of the reference track.
    fn onsets(&mut self, track: TrackId) -> (usize, usize) {
        let onset = |x: &[f32]| x.iter().position(|s| s.abs() > 0.01).expect("silent");
        let t = self.play_only(track);
        let r = self.play_only(self.b);
        (onset(&t), onset(&r))
    }
}

fn samples(ms: f64) -> usize {
    (ms * 0.001 * SR as f64).round() as usize
}

/// Sanity: a lookahead set before the first publish is compensated.
#[test]
fn initial_lookahead_is_compensated() {
    let mut d = Drive::new(10.0);
    assert_eq!(d.ctl.bridge.handle.latency() as usize, samples(10.0));
    let a = d.a;
    let (t, r) = d.onsets(a);
    assert_eq!(t, r);
}

/// Changing the lookahead mid-session keeps the track aligned with the reference after the
/// next tick (both ways: up, down, back to zero).
#[test]
fn changed_lookahead_republishes_and_stays_aligned() {
    let mut d = Drive::new(0.0);
    let a = d.a;
    let (t, r) = d.onsets(a);
    assert_eq!(t, r, "aligned at start");
    let base = r;
    for ms in [10.0, 3.0, 0.0, 7.5] {
        let before = d.ctl.bridge.publishes;
        let gate = d.gate;
        d.ok(Command::Device(DeviceCommand::SetParam {
            device: gate,
            param: LOOKAHEAD,
            value: ms,
        }));
        // A param change doesn't republish by itself.
        assert_eq!(d.ctl.bridge.publishes, before, "SetParam republished");
        d.idle();
        assert!(
            d.ctl.bridge.publishes > before,
            "no republish after lookahead -> {ms} ms"
        );
        assert_eq!(d.ctl.bridge.handle.latency() as usize, samples(ms));
        let (t, r) = d.onsets(a);
        assert_eq!(t, r, "track misaligned after lookahead -> {ms} ms");
        assert_eq!(r, base + samples(ms), "reference not delayed by the new PDC");
    }
}

/// With stable latencies the tick never republishes; a single change republishes once
/// (debounced), not every tick.
#[test]
fn no_republish_when_latencies_are_stable() {
    let mut d = Drive::new(5.0);
    let before = d.ctl.bridge.publishes;
    for _ in 0..200 {
        d.ctl.bridge.render(BLOCK);
        d.tick();
    }
    assert_eq!(d.ctl.bridge.publishes, before, "republished with stable latencies");
    let gate = d.gate;
    d.ok(Command::Device(DeviceCommand::SetParam {
        device: gate,
        param: LOOKAHEAD,
        value: 2.0,
    }));
    for _ in 0..200 {
        d.ctl.bridge.render(BLOCK);
        d.tick();
    }
    assert_eq!(d.ctl.bridge.publishes, before + 1, "one republish per change");
}

/// The republish is debounced: while the lookahead keeps changing every block (a knob being
/// dragged), publishes are at least `LATENCY_REPUBLISH_MS` apart.
#[test]
fn republish_is_debounced() {
    let mut d = Drive::new(0.0);
    let before = d.ctl.bridge.publishes;
    let gate = d.gate;
    let start = d.ctl.host.now;
    // 100 ticks (1.6 s at 16 ms), a new value every tick.
    for i in 0..100 {
        d.ok(Command::Device(DeviceCommand::SetParam {
            device: gate,
            param: LOOKAHEAD,
            value: (i % 10) as f64 + 0.5,
        }));
        d.ctl.bridge.render(BLOCK);
        d.tick();
    }
    let elapsed = d.ctl.host.now - start;
    let n = d.ctl.bridge.publishes - before;
    assert!(n > 0);
    assert!(
        n as u64 <= elapsed / ether_controller::LATENCY_REPUBLISH_MS + 1,
        "{n} publishes in {elapsed} ms"
    );
}

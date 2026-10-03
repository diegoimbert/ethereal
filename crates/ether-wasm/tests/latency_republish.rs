//! `web-latency` (CONTRACTS.md §13.13): the native `latency-republish` test ported to the
//! web pipeline. The real `EtherController` drives a `WebBridge`; the Worklet's
//! `EngineHost` renders on the other side of heap-backed rings (the SAB stand-in). When a
//! built-in's latency changes in the Worklet (the Gate's `Lookahead`), its latency report
//! reaches the Worker's `node_latency`, the controller tick republishes and PDC keeps the
//! track sample-aligned with a dry reference track. The Limiter's fixed 5 ms lookahead is
//! compensated from the first publish and never causes a republish.
//!
//! Real devices only (no test node): the Gate with `Floor` at 0 dB is a pure delay of its
//! lookahead, so the delayed track must equal the reference sample for sample.

use std::sync::Arc;

use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::{
    BridgeError, Controller, ControllerConfig, EngineBridge, EtherController, HostServices,
};
use ether_core::analysis::AnalysisFrame;
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::devices::{DeviceCommand, DeviceDescriptor, DeviceSpec};
use ether_core::protocol::media::{BrowseLocation, MediaCommand, MediaSource};
use ether_core::protocol::mixer::MixerCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::project::ProjectCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::transport::TransportCommand;
use ether_core::protocol::*;
use ether_core::{EngineOutputs, NodeKey, ParamChange, RenderGraphDesc, TransportControl};
use ether_devices::fx_dynamics::gate;
use ether_media::DecodedAudio;
use ether_wasm::bridge::{self, WebBridge};
use ether_wasm::latency::REPORT_INTERVAL_MS;
use ether_wasm::ring::HeapMemory;
use ether_wasm::worklet::{EngineHost, RENDER_QUANTUM, tap_clock};

const SR: u32 = 48_000;
const T0: u64 = 1_750_000_000_000;
/// Controller tick period (ms) and the audio rendered per tick (frames).
const TICK_MS: u64 = 16;
const TICK_FRAMES: usize = SR as usize * TICK_MS as usize / 1000;

// ─── The web bridge, counting publishes ──────────────────────────────────────────────────

struct Counting {
    web: WebBridge<HeapMemory>,
    publishes: usize,
}

impl EngineBridge for Counting {
    fn create_builtin(
        &mut self,
        device: DeviceId,
        kind: &BuiltinDevice,
        params: &[(ParamId, f64)],
    ) -> Result<NodeKey, BridgeError> {
        self.web.create_builtin(device, kind, params)
    }
    fn create_plugin(
        &mut self,
        device: DeviceId,
        plugin: &PluginInstance,
        state: Option<&Base64Bytes>,
    ) -> Result<NodeKey, BridgeError> {
        self.web.create_plugin(device, plugin, state)
    }
    fn destroy_node(&mut self, key: NodeKey) -> Result<(), BridgeError> {
        self.web.destroy_node(key)
    }
    fn load_media(
        &mut self,
        media: &MediaRef,
        audio: Arc<DecodedAudio>,
    ) -> Result<(), BridgeError> {
        self.web.load_media(media, audio)
    }
    fn unload_media(&mut self, media: MediaId) -> Result<(), BridgeError> {
        self.web.unload_media(media)
    }
    fn publish(&mut self, graph: RenderGraphDesc) -> Result<(), BridgeError> {
        self.publishes += 1;
        self.web.publish(graph)
    }
    fn set_param(&mut self, change: ParamChange) -> Result<(), BridgeError> {
        self.web.set_param(change)
    }
    fn transport(&mut self, control: TransportControl) -> Result<(), BridgeError> {
        self.web.transport(control)
    }
    fn poll(&mut self, out: &mut EngineOutputs) {
        self.web.poll(out)
    }
    fn descriptor(&mut self, device: DeviceId) -> Option<DeviceDescriptor> {
        self.web.descriptor(device)
    }
    fn set_node_scale(
        &mut self,
        device: DeviceId,
        scale: MusicalScale,
    ) -> Result<bool, BridgeError> {
        self.web.set_node_scale(device, scale)
    }
    fn poll_analysis(&mut self, out: &mut Vec<AnalysisFrame>) {
        self.web.poll_analysis(out)
    }
    fn watch_analysis(&mut self, node: NodeKey, on: bool) -> Result<(), BridgeError> {
        self.web.watch_analysis(node, on)
    }
    fn node_latency(&self, key: NodeKey) -> Option<u32> {
        self.web.node_latency(key)
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
    ctl: EtherController<Counting, TestHost, MemoryStore, MemoryLibrary>,
    host: EngineHost<HeapMemory>,
    shared: bridge::Shared<HeapMemory>,
    ids: IdGen,
    next: u32,
    fx: DeviceId,
    a: TrackId,
    b: TrackId,
}

fn wav(channels: &[Vec<f32>]) -> Vec<u8> {
    let n_ch = channels.len() as u16;
    let frames = channels[0].len();
    let data_len = frames * n_ch as usize * 2;
    let mut b = Vec::with_capacity(44 + data_len);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&n_ch.to_le_bytes());
    b.extend_from_slice(&SR.to_le_bytes());
    b.extend_from_slice(&(SR * n_ch as u32 * 2).to_le_bytes());
    b.extend_from_slice(&(n_ch * 2).to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(data_len as u32).to_le_bytes());
    for i in 0..frames {
        for ch in channels {
            let s = (ch[i].clamp(-1.0, 1.0) * 32767.0).round() as i16;
            b.extend_from_slice(&s.to_le_bytes());
        }
    }
    b
}

fn sine(hz: f32, frames: usize, amp: f32) -> Vec<f32> {
    (0..frames)
        .map(|i| amp * (2.0 * std::f32::consts::PI * hz * i as f32 / SR as f32).sin())
        .collect()
}

fn samples(ms: f64) -> usize {
    (ms * 0.001 * SR as f64).round() as usize
}

impl Drive {
    /// Track A: tone clip → `device` with `params`. Track B (reference): the same clip, dry.
    fn new(device: BuiltinDevice, params: &[(ParamId, f64)]) -> Self {
        let mut lib = MemoryLibrary::new();
        lib.add_root("lib", "Library");
        // A sharp onset after silence, then a pitch change: a shift of one sample shows.
        let mut tone = vec![0.0f32; 4800];
        tone.extend(sine(330.0, SR as usize / 4, 0.4));
        tone.extend(sine(517.0, SR as usize / 4, 0.3));
        lib.add_file("lib", "tone.wav", wav(&[tone.clone(), tone]));
        let control = HeapMemory::new(1 << 22);
        let reports = HeapMemory::new(1 << 16);
        let shared = bridge::shared(control.clone(), reports.clone());
        let mut ids = IdGen::new(7);
        let (fx, a, b) = (ids.next(T0), ids.next(T0), ids.next(T0));
        let mut d = Self {
            ctl: EtherController::with_config(
                Counting {
                    web: WebBridge::new(shared.clone()),
                    publishes: 0,
                },
                TestHost { now: T0 },
                MemoryStore::new(),
                lib,
                ControllerConfig {
                    engine_sample_rate: SR,
                    ..Default::default()
                },
            ),
            host: EngineHost::new(SR, control, reports),
            shared,
            ids,
            next: 1,
            fx,
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
            id: fx,
            track: a,
            device: DeviceSpec::Builtin { device },
            before: None,
        }));
        for &(param, value) in params {
            d.set(param, value);
        }
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
        for m in &out {
            if let ServerMessage::Reply(r) = m
                && r.id == id
            {
                match &r.result {
                    ReplyResult::Ok { value } => return value.clone(),
                    ReplyResult::Err { error } => panic!("command failed: {error:?}"),
                }
            }
        }
        panic!("no reply")
    }

    fn set(&mut self, param: ParamId, value: f64) {
        let device = self.fx;
        self.ok(Command::Device(DeviceCommand::SetParam {
            device,
            param,
            value,
        }));
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
        self.ctl.host.now += TICK_MS;
        let now = self.ctl.host.now;
        self.ctl.tick(now, &mut Vec::new());
    }

    fn publishes(&self) -> usize {
        self.ctl.bridge.publishes
    }

    /// Render `frames` in Worklet quanta; returns the left output.
    fn render(&mut self, frames: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(frames);
        while out.len() < frames {
            let n = RENDER_QUANTUM.min(frames - out.len());
            self.host.render(n);
            out.extend_from_slice(&self.host.output(0)[..n]);
        }
        out
    }

    /// The Worklet's graph latency (PDC included) as of its last render.
    fn graph_latency(&self) -> usize {
        self.host.tap_clock()[tap_clock::LATENCY] as usize
    }

    /// Let both sides settle: real-time-like interleaving of Worklet audio and controller
    /// ticks, well past the report interval and the republish debounce.
    fn idle(&mut self) {
        for _ in 0..30 {
            self.render(TICK_FRAMES);
            self.tick();
        }
    }

    fn mute(&mut self, track: TrackId, mute: bool) {
        self.ok(Command::Mixer(MixerCommand::SetMute { track, mute }));
    }

    /// Play a quarter second from beat 0 with only `track` audible.
    fn play_only(&mut self, track: TrackId) -> Vec<f32> {
        let (a, b) = (self.a, self.b);
        self.mute(a, track != a);
        self.mute(b, track != b);
        self.ok(Command::Transport(TransportCommand::Stop));
        self.ok(Command::Transport(TransportCommand::Locate {
            position: Beats(0.0),
        }));
        self.tick();
        self.render(TICK_FRAMES);
        self.ok(Command::Transport(TransportCommand::Play));
        self.tick();
        // Render from the quantum that applies `Play`; the output is compared relative to
        // the reference, rendered the same way.
        let out = self.render(SR as usize / 4);
        self.ok(Command::Transport(TransportCommand::Stop));
        self.tick();
        self.render(TICK_FRAMES);
        out
    }

    /// Render the track and the reference; returns `(onset of track, onset of reference,
    /// largest sample difference)`.
    fn compare(&mut self) -> (usize, usize, f32) {
        let onset = |x: &[f32]| x.iter().position(|s| s.abs() > 0.01).expect("silent");
        let a = self.a;
        let t = self.play_only(a);
        let r = self.play_only(self.b);
        let diff = t
            .iter()
            .zip(&r)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0f32, f32::max);
        (onset(&t), onset(&r), diff)
    }

    fn errors(&self) -> Vec<String> {
        self.shared.borrow().errors.clone()
    }
}

/// Gate params making it a pure delay of its lookahead (never closes, unity output).
fn open_gate(lookahead_ms: f64) -> Vec<(ParamId, f64)> {
    vec![
        (gate::THRESHOLD, -80.0),
        (gate::RANGE, 0.0),
        (gate::LOOKAHEAD, lookahead_ms),
    ]
}

/// Sanity: a lookahead set before the first publish is compensated on the web.
#[test]
fn initial_lookahead_is_compensated() {
    let mut d = Drive::new(BuiltinDevice::Gate, &open_gate(10.0));
    assert_eq!(d.graph_latency(), samples(10.0));
    let (t, r, diff) = d.compare();
    assert_eq!(t, r);
    assert!(diff < 1e-5, "max difference {diff}");
    assert!(d.errors().is_empty(), "{:?}", d.errors());
}

/// Changing the lookahead mid-session reaches the Worker through the latency report and
/// republishes: the track stays sample-aligned with the reference (up, down, zero, again).
#[test]
fn changed_lookahead_republishes_and_stays_aligned() {
    let mut d = Drive::new(BuiltinDevice::Gate, &open_gate(0.0));
    assert_eq!(d.graph_latency(), 0);
    let (t, r, diff) = d.compare();
    assert_eq!(t, r, "aligned at start");
    assert!(diff < 1e-5);
    let base = r;
    for ms in [10.0, 3.0, 0.0, 7.5] {
        let before = d.publishes();
        d.set(gate::LOOKAHEAD, ms);
        assert_eq!(d.publishes(), before, "SetParam republished");
        d.idle();
        assert!(
            d.publishes() > before,
            "no republish after lookahead -> {ms} ms"
        );
        assert_eq!(d.graph_latency(), samples(ms), "PDC after {ms} ms");
        let (t, r, diff) = d.compare();
        assert_eq!(t, r, "track misaligned after lookahead -> {ms} ms");
        assert_eq!(
            r,
            base + samples(ms),
            "reference not delayed by the new PDC"
        );
        assert!(diff < 1e-5, "max difference {diff} after {ms} ms");
    }
    assert!(d.errors().is_empty(), "{:?}", d.errors());
}

/// With stable latencies nothing republishes; one change republishes exactly once.
#[test]
fn no_republish_when_latencies_are_stable() {
    let mut d = Drive::new(BuiltinDevice::Gate, &open_gate(5.0));
    let before = d.publishes();
    for _ in 0..200 {
        d.render(TICK_FRAMES);
        d.tick();
    }
    assert_eq!(d.publishes(), before, "republished with stable latencies");
    d.set(gate::LOOKAHEAD, 2.0);
    for _ in 0..200 {
        d.render(TICK_FRAMES);
        d.tick();
    }
    assert_eq!(d.publishes(), before + 1, "one republish per change");
}

/// A knob drag (a new lookahead every tick) republishes at most once per debounce period,
/// and the final value is compensated once it stops.
#[test]
fn republish_is_debounced() {
    let mut d = Drive::new(BuiltinDevice::Gate, &open_gate(0.0));
    let before = d.publishes();
    let start = d.ctl.host.now;
    for i in 0..100 {
        d.set(gate::LOOKAHEAD, (i % 10) as f64 + 0.5);
        d.render(TICK_FRAMES);
        d.tick();
    }
    let elapsed = d.ctl.host.now - start;
    let n = d.publishes() - before;
    assert!(n > 0);
    let period = ether_controller::LATENCY_REPUBLISH_MS.max(REPORT_INTERVAL_MS);
    assert!(
        n as u64 <= elapsed / period + 1,
        "{n} publishes in {elapsed} ms"
    );
    d.idle();
    assert_eq!(d.graph_latency(), samples(9.5));
    let (t, r, _) = d.compare();
    assert_eq!(t, r);
}

/// The Limiter's fixed lookahead (5 ms = 240 samples at 48 kHz) is compensated from the
/// first publish, its baseline latency is never reported, so it never republishes; the
/// limited track (gain 0 dB, ceiling well above the tone) stays aligned with the reference.
#[test]
fn limiter_lookahead_is_compensated_without_republish() {
    use ether_devices::limiter::{LOOKAHEAD_MS, params as lp};
    let mut d = Drive::new(BuiltinDevice::Limiter, &[(lp::CEILING, 0.0)]);
    let lookahead = samples(f64::from(LOOKAHEAD_MS));
    assert_eq!(lookahead, 240);
    assert_eq!(d.graph_latency(), lookahead);
    let before = d.publishes();
    for _ in 0..100 {
        d.render(TICK_FRAMES);
        d.tick();
    }
    assert_eq!(d.publishes(), before, "a fixed latency republished");
    let (t, r, diff) = d.compare();
    assert_eq!(t, r);
    assert!(diff < 1e-4, "max difference {diff}");
}

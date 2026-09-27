//! Sidechain end to end (sidechain node): the real controller compiles a project into the
//! real engine with the real built-in devices, rendered offline on the test thread.
//!
//! A "kick" MIDI track (short sine notes on beats 1..3, its output routed nowhere so only
//! its sidechain tap is heard) feeds the Compressor on a sustained "pad" MIDI track:
//! - the pad ducks while each kick sounds and recovers between kicks (the compressor's gain
//!   reduction follows the sidechain envelope);
//! - with a latent device on the kick (the Limiter's 5 ms lookahead), the whole result is
//!   the same rendering shifted by exactly that latency: the sidechain stays sample-aligned
//!   with the pad (the pad's main signal is delayed to meet it);
//! - every block is rendered under `assert_no_alloc`.

use std::collections::HashMap;
use std::sync::Arc;

use assert_no_alloc::assert_no_alloc;
use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::{
    BridgeError, Controller, ControllerConfig, EngineBridge, EtherController, HostServices,
};
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::devices::{DeviceCommand, DeviceDescriptor, DeviceSpec};
use ether_core::protocol::message::{ClientMessage, ServerMessage};
use ether_core::protocol::mixer::MixerCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteSpec};
use ether_core::protocol::project::ProjectCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::transport::TransportCommand;
use ether_core::protocol::{Command, ReplyResult};
use ether_core::{
    AudioSource, Engine, EngineConfig, EngineHandle, EngineOutputs, GarbageCollector, NodeKey,
    ParamChange, RenderGraphDesc, TransportControl, create,
};
use ether_devices::{compressor, limiter, synth};
use ether_media::{DecodedAudio, InMemorySource};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const SR: u32 = 48_000;
const BLOCK: usize = 256;
/// Samples per beat at the default 120 bpm.
const BEAT: usize = SR as usize / 2;

/// `EngineBridge` over a real in-process engine (like the native bridge, without audio
/// device, plugins or recording).
struct Bridge {
    handle: EngineHandle,
    sources: HashMap<MediaId, Arc<dyn AudioSource>>,
}

struct Sources<'a>(&'a HashMap<MediaId, Arc<dyn AudioSource>>);

impl ether_devices::SampleResolver for Sources<'_> {
    fn resolve(&self, media: MediaId) -> Option<Arc<dyn AudioSource>> {
        self.0.get(&media).cloned()
    }
}

fn engine_err(e: impl std::fmt::Display) -> BridgeError {
    BridgeError::Other(e.to_string())
}

impl EngineBridge for Bridge {
    fn create_builtin(
        &mut self,
        _device: DeviceId,
        kind: &BuiltinDevice,
        params: &[(ParamId, f64)],
    ) -> Result<NodeKey, BridgeError> {
        let mut node = ether_devices::create(kind, &Sources(&self.sources));
        for (id, v) in params {
            node.set_param(*id, *v);
        }
        self.handle.add_node(node).map_err(engine_err)
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
        self.handle.remove_node(key).map_err(engine_err)
    }
    fn load_media(
        &mut self,
        media: &MediaRef,
        audio: Arc<DecodedAudio>,
    ) -> Result<(), BridgeError> {
        let source: Arc<dyn AudioSource> = Arc::new(InMemorySource::new(audio));
        self.handle
            .add_source(media.id, source.clone())
            .map_err(engine_err)?;
        self.sources.insert(media.id, source);
        Ok(())
    }
    fn unload_media(&mut self, media: MediaId) -> Result<(), BridgeError> {
        if self.sources.remove(&media).is_some() {
            self.handle.remove_source(media).map_err(engine_err)?;
        }
        Ok(())
    }
    fn publish(&mut self, graph: RenderGraphDesc) -> Result<(), BridgeError> {
        self.handle.publish(graph).map_err(engine_err)
    }
    fn set_param(&mut self, change: ParamChange) -> Result<(), BridgeError> {
        self.handle.set_param(change).map_err(engine_err)
    }
    fn transport(&mut self, control: TransportControl) -> Result<(), BridgeError> {
        self.handle.transport(control).map_err(engine_err)
    }
    fn poll(&mut self, out: &mut EngineOutputs) {
        self.handle.poll(out);
    }
    fn descriptor(&mut self, _: DeviceId) -> Option<DeviceDescriptor> {
        None
    }
}

struct Host;

impl HostServices for Host {
    fn now_ms(&self) -> u64 {
        1_750_000_000_000
    }
    fn random_seed(&mut self) -> u64 {
        7
    }
}

type Ctl = EtherController<Bridge, Host, MemoryStore, MemoryLibrary>;

struct Rig {
    ctl: Ctl,
    engine: Engine,
    _gc: GarbageCollector,
    ids: IdGen,
    next: u32,
}

impl Rig {
    fn new() -> Self {
        let parts = create(EngineConfig {
            sample_rate: SR,
            max_block_size: BLOCK,
            max_nodes: 64,
            ..EngineConfig::default()
        });
        let bridge = Bridge {
            handle: parts.handle,
            sources: HashMap::new(),
        };
        let config = ControllerConfig {
            engine_sample_rate: SR,
            autosave_after_ms: None,
            ..ControllerConfig::default()
        };
        let ctl = EtherController::with_config(
            bridge,
            Host,
            MemoryStore::new(),
            MemoryLibrary::new(),
            config,
        );
        let mut rig = Self {
            ctl,
            engine: parts.engine,
            _gc: parts.gc,
            ids: IdGen::new(3),
            next: 1,
        };
        let id = rig.ids.next_project_id(1);
        rig.ok(Command::Project(ProjectCommand::Create {
            id,
            name: "Sidechain".into(),
        }));
        rig
    }

    fn id<I: Id>(&mut self) -> I {
        self.ids.next(1)
    }

    fn ok(&mut self, command: Command) {
        let id = self.next;
        self.next += 1;
        let mut out = Vec::new();
        self.ctl.handle(
            ClientMessage {
                id,
                gesture: None,
                command: command.clone(),
            },
            &mut out,
        );
        let reply = out.iter().find_map(|m| match m {
            ServerMessage::Reply(r) if r.id == id => Some(r),
            _ => None,
        });
        assert!(
            matches!(reply.map(|r| &r.result), Some(ReplyResult::Ok { .. })),
            "{command:?} → {reply:?}"
        );
    }

    fn tick(&mut self) {
        let mut out = Vec::new();
        self.ctl.tick(1_750_000_000_000, &mut out);
    }

    fn midi_track(&mut self, name: &str) -> TrackId {
        let id = self.id();
        self.ok(Command::Track(TrackCommand::Create {
            id,
            kind: TrackKind::Midi,
            name: Some(name.into()),
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

    fn param(&mut self, device: DeviceId, param: ParamId, value: f64) {
        self.ok(Command::Device(DeviceCommand::SetParam {
            device,
            param,
            value,
        }));
    }

    /// A sine synth with a flat envelope.
    fn sine_synth(&mut self, track: TrackId, volume_db: f64) {
        let s = self.device(track, BuiltinDeviceType::Synth);
        self.param(s, synth::params::WAVEFORM, 0.0);
        self.param(s, synth::params::ATTACK, 0.0);
        self.param(s, synth::params::SUSTAIN, 100.0);
        self.param(s, synth::params::RELEASE, 1.0);
        self.param(s, synth::params::CUTOFF, 20_000.0);
        self.param(s, synth::params::VOLUME, volume_db);
    }

    fn notes(&mut self, track: TrackId, notes: &[(u8, f64, f64)]) {
        let clip = self.id();
        self.ok(Command::Clip(ClipCommand::CreateMidi {
            id: clip,
            track,
            start: Beats(0.0),
            length: Beats(8.0),
            name: None,
        }));
        let notes = notes
            .iter()
            .map(|&(pitch, start, duration)| NoteSpec {
                id: self.id(),
                pitch,
                velocity: 1.0,
                start: Beats(start),
                duration: Beats(duration),
            })
            .collect();
        self.ok(Command::Note(NoteCommand::Add { clip, notes }));
    }

    /// Publish, play from 0 and render `frames` of the master output (left).
    fn play(&mut self, frames: usize) -> Vec<f32> {
        self.tick();
        self.ok(Command::Transport(TransportCommand::Play));
        self.tick();
        let mut out = vec![0.0f32; frames];
        let mut r = vec![0.0f32; BLOCK];
        let mut done = 0;
        while done < frames {
            let n = BLOCK.min(frames - done);
            let engine = &mut self.engine;
            let (l, rr) = (&mut out[done..done + n], &mut r[..n]);
            assert_no_alloc(|| {
                let mut outs: [&mut [f32]; 2] = [l, rr];
                engine.process(&[], &mut outs, n);
            });
            done += n;
        }
        out
    }
}

struct Setup {
    sidechain: bool,
    latent_kick: bool,
}

/// Kick notes on beats 1, 2, 3 (50 ms); a sustained pad note over beats 0..4.
const KICK_LEN: usize = BEAT / 10;

fn render(s: Setup) -> Vec<f32> {
    let mut rig = Rig::new();
    let kick = rig.midi_track("Kick");
    let pad = rig.midi_track("Pad");
    rig.sine_synth(kick, -6.0);
    if s.latent_kick {
        rig.device(kick, BuiltinDeviceType::Limiter);
    }
    // The kick is only heard through the sidechain.
    rig.ok(Command::Mixer(MixerCommand::SetOutput {
        track: kick,
        output: TrackOutput::None,
    }));
    rig.notes(kick, &[(48, 1.0, 0.1), (48, 2.0, 0.1), (48, 3.0, 0.1)]);

    rig.sine_synth(pad, -36.0);
    let comp = rig.device(pad, BuiltinDeviceType::Compressor);
    rig.param(comp, compressor::params::THRESHOLD, -24.0);
    rig.param(comp, compressor::params::RATIO, 20.0);
    rig.param(comp, compressor::params::ATTACK, 1.0);
    rig.param(comp, compressor::params::RELEASE, 30.0);
    rig.notes(pad, &[(69, 0.0, 4.0)]);
    if s.sidechain {
        rig.ok(Command::Device(DeviceCommand::SetSidechain {
            device: comp,
            source: Some(kick),
        }));
    }
    rig.play(4 * BEAT)
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

#[test]
fn kick_ducks_the_pad_and_gain_reduction_follows_the_envelope() {
    let dry = render(Setup {
        sidechain: false,
        latent_kick: false,
    });
    let wet = render(Setup {
        sidechain: true,
        latent_kick: false,
    });
    // The pad alone sits under the threshold's knee: it is not compressed by itself.
    let level = rms(&dry[BEAT / 2..BEAT]);
    assert!(level > 0.001, "pad level {level}");
    // Identical before the first kick.
    let pre = rms(&wet[BEAT / 2..BEAT]) / level;
    assert!((pre - 1.0).abs() < 0.01, "before the first kick: {pre}");
    for b in 1..4 {
        let at = b * BEAT;
        // Second half of the kick: ducked by at least 14 dB.
        let during = rms(&wet[at + KICK_LEN / 2..at + KICK_LEN])
            / rms(&dry[at + KICK_LEN / 2..at + KICK_LEN]);
        assert!(during < 0.2, "beat {b}: during the kick {during}");
        // Recovered well before the next beat (release 30 ms).
        let end = (at + BEAT).min(4 * BEAT);
        let after = rms(&wet[end - BEAT / 4..end]) / rms(&dry[end - BEAT / 4..end]);
        assert!(after > 0.95, "beat {b}: between kicks {after}");
    }
}

#[test]
fn a_latent_source_keeps_the_sidechain_sample_aligned() {
    let plain = render(Setup {
        sidechain: true,
        latent_kick: false,
    });
    let latent = render(Setup {
        sidechain: true,
        latent_kick: true,
    });
    let lat = limiter::lookahead_samples(SR as f32) as usize;
    // The pad (and its ducking) comes out exactly `lat` later: its main signal was delayed
    // to meet the latent sidechain.
    assert!(latent[..lat].iter().all(|&v| v == 0.0), "delayed start");
    let n = plain.len() - lat;
    let diff = (0..n)
        .map(|i| (latent[i + lat] - plain[i]).abs())
        .fold(0.0f32, f32::max);
    assert!(diff < 1e-6, "max difference {diff}");
    // Sanity: the ducking is really there in both.
    let at = BEAT + KICK_LEN;
    assert!(rms(&plain[at - 600..at]) < rms(&plain[BEAT / 2..BEAT]) * 0.2);
}

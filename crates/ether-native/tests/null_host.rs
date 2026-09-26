//! Integration test: the full native host on the `null` audio backend (no device), with a
//! thin fake controller (the real one is the `controller` node's), checking that the
//! engine runs on the audio thread and that playhead (~60 Hz) and meter (~30 Hz) streams,
//! replies and host-handled commands reach the subscriber.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ether_controller::{Controller, EngineBridge, MessageSink};
use ether_core::graph::TrackDesc;
use ether_core::protocol::engine::EngineCommand;
use ether_core::protocol::message::{
    Command, ErrorCode, PlayheadFrame, Reply, ReplyResult, ReplyValue,
};
use ether_core::protocol::meters::MeterFrame;
use ether_core::protocol::model::{Project, Seconds, TrackId, TrackKind, Ulid};
use ether_core::protocol::plugins::PluginCommand;
use ether_core::protocol::transport::{PlayheadUpdate, TransportCommand};
use ether_core::protocol::{ClientMessage, ServerMessage};
use ether_core::{EngineOutputs, RenderGraphDesc, TransportControl};
use ether_native::audio::{AudioBackendKind, AudioSettings};
use ether_native::host::{HostConfig, HostOptions, NativeHost};
use ether_native::test_util::TempDir;
use ether_native::{DedicatedThread, NativeBridge};

/// Minimal controller: publishes a graph with a master track, handles transport, and turns
/// engine outputs into Playhead/Meters messages on tick.
struct FakeController {
    bridge: NativeBridge,
    outputs: EngineOutputs,
}

impl FakeController {
    fn new(mut bridge: NativeBridge) -> Self {
        let master = TrackDesc {
            id: TrackId(Ulid(1)),
            kind: TrackKind::Master,
            chain: vec![],
            output: None,
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
        };
        bridge
            .publish(RenderGraphDesc {
                version: 1,
                tracks: vec![master],
                ..Default::default()
            })
            .unwrap();
        Self {
            bridge,
            outputs: EngineOutputs::default(),
        }
    }
}

impl Controller for FakeController {
    fn handle(&mut self, message: ClientMessage, out: &mut dyn MessageSink) {
        let value = match message.command {
            Command::Transport(TransportCommand::Play) => {
                self.bridge.transport(TransportControl::Play).unwrap();
                ReplyValue::Unit
            }
            Command::Transport(TransportCommand::Stop) => {
                self.bridge.transport(TransportControl::Stop).unwrap();
                ReplyValue::Unit
            }
            Command::Project(_) => panic!("fake controller: no projects"),
            _ => ReplyValue::Unit,
        };
        out.send(ServerMessage::Reply(Reply {
            id: message.id,
            result: ReplyResult::Ok { value },
        }));
    }

    fn tick(&mut self, _now_ms: u64, out: &mut dyn MessageSink) {
        self.bridge.poll(&mut self.outputs);
        if let Some(p) = self.outputs.playhead {
            out.send(ServerMessage::Playhead(PlayheadFrame {
                transport: PlayheadUpdate {
                    position: p.position,
                    seconds: Seconds(p.seconds),
                    playing: p.playing,
                    bpm: p.bpm,
                },
                session: vec![],
            }));
        }
        if !self.outputs.meters.is_empty() {
            out.send(ServerMessage::Meters(MeterFrame {
                tracks: std::mem::take(&mut self.outputs.meters),
                cpu_load: self.outputs.cpu_load,
            }));
        }
    }

    fn project(&self) -> Option<&Project> {
        None
    }
}

#[derive(Default)]
struct Collected {
    replies: Vec<Reply>,
    playhead: Vec<(Instant, PlayheadFrame)>,
    meters: Vec<(Instant, MeterFrame)>,
    events: usize,
}

fn start(tmp: &TempDir) -> (NativeHost, Arc<Mutex<Collected>>) {
    let host = NativeHost::start(
        HostConfig {
            audio: Some(AudioSettings {
                backend: AudioBackendKind::Null,
                max_block_size: 256,
                ..Default::default()
            }),
            data_dir: tmp.path().to_path_buf(),
            instance: "test".into(),
            projects_root: tmp.path().join("projects"),
            library_roots: vec![],
        },
        HostOptions {
            main_thread: Arc::new(DedicatedThread::new()),
            instantiate: Arc::new(|_, id| {
                Err(ether_core::plugin::PluginError::NotFound(id.to_string()))
            }),
            controller: Box::new(|bridge, _services, _store, _library| {
                Box::new(FakeController::new(bridge))
            }),
        },
    )
    .unwrap();
    let collected = Arc::new(Mutex::new(Collected::default()));
    let c = collected.clone();
    host.subscribe(Arc::new(move |m| {
        let mut c = c.lock().unwrap();
        let now = Instant::now();
        match m {
            ServerMessage::Reply(r) => c.replies.push(r),
            ServerMessage::Playhead(p) => c.playhead.push((now, p)),
            ServerMessage::Meters(f) => c.meters.push((now, f)),
            ServerMessage::Event(_) => c.events += 1,
        }
    }));
    (host, collected)
}

fn msg(id: u32, command: Command) -> ClientMessage {
    ClientMessage {
        id,
        gesture: None,
        command,
    }
}

fn wait_reply(c: &Arc<Mutex<Collected>>, id: u32) -> Reply {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(r) = c.lock().unwrap().replies.iter().find(|r| r.id == id) {
            return r.clone();
        }
        assert!(Instant::now() < deadline, "no reply for {id}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn null_backend_streams_playhead_and_meters() {
    let tmp = TempDir::new("null-host");
    let (host, c) = start(&tmp);
    assert_eq!(host.audio_info().backend, AudioBackendKind::Null);

    host.send(msg(1, Command::Transport(TransportCommand::Play)))
        .unwrap();
    let r = wait_reply(&c, 1);
    assert!(matches!(r.result, ReplyResult::Ok { .. }));

    std::thread::sleep(Duration::from_millis(600));
    {
        let c = c.lock().unwrap();
        // Playhead at ~60 Hz: ~36 frames in 600 ms (loose bounds for loaded machines).
        assert!(
            c.playhead.len() >= 10 && c.playhead.len() <= 60,
            "playhead frames: {}",
            c.playhead.len()
        );
        let last = &c.playhead.last().unwrap().1.transport;
        assert!(last.playing);
        // 120 bpm default: ~1.2 beats in 600 ms.
        assert!(
            last.position.0 > 0.3 && last.position.0 < 3.0,
            "position {}",
            last.position.0
        );
        // Positions are monotonic while playing (no loop).
        let playing: Vec<f64> = c
            .playhead
            .iter()
            .filter(|(_, p)| p.transport.playing)
            .map(|(_, p)| p.transport.position.0)
            .collect();
        assert!(playing.windows(2).all(|w| w[1] >= w[0]));

        // Meters coalesced to ~30 Hz, carrying the master track.
        assert!(
            c.meters.len() >= 5 && c.meters.len() <= 25,
            "meter frames: {} playhead {} gaps {:?}",
            c.meters.len(),
            c.playhead.len(),
            c.meters
                .windows(2)
                .map(|w| w[1].0.duration_since(w[0].0))
                .collect::<Vec<_>>()
        );
        assert!(
            c.meters
                .iter()
                .all(|(_, m)| m.tracks.iter().any(|t| t.track == TrackId(Ulid(1))))
        );
        let min_gap = c
            .meters
            .windows(2)
            .map(|w| w[1].0.duration_since(w[0].0))
            .min()
            .unwrap();
        assert!(min_gap >= Duration::from_millis(30), "{min_gap:?}");
    }

    // Host-handled commands.
    host.send(msg(2, Command::Engine(EngineCommand::GetStatus)))
        .unwrap();
    match wait_reply(&c, 2).result {
        ReplyResult::Ok {
            value: ReplyValue::Status { status },
        } => {
            assert!(status.running);
            assert_eq!(status.backend, "null");
            assert_eq!(status.sample_rate, 48_000);
            assert_eq!(status.instance, "test");
        }
        other => panic!("{other:?}"),
    }
    host.send(msg(3, Command::Plugin(PluginCommand::List)))
        .unwrap();
    assert!(matches!(
        wait_reply(&c, 3).result,
        ReplyResult::Ok {
            value: ReplyValue::Plugins { .. }
        }
    ));

    // A panicking controller answers with an Internal error and keeps running.
    host.send(msg(
        4,
        Command::Project(ether_core::protocol::project::ProjectCommand::Get),
    ))
    .unwrap();
    match wait_reply(&c, 4).result {
        ReplyResult::Err { error } => assert_eq!(error.code, ErrorCode::Internal),
        other => panic!("{other:?}"),
    }
    host.send(msg(5, Command::Transport(TransportCommand::Stop)))
        .unwrap();
    assert!(matches!(wait_reply(&c, 5).result, ReplyResult::Ok { .. }));

    // Switching the audio config moves the engine to a new backend (offline → null).
    host.send(msg(
        6,
        Command::Engine(EngineCommand::SetAudioConfig {
            config: ether_core::protocol::engine::AudioConfig {
                backend: Some("offline".into()),
                host: None,
                output_device: None,
                input_device: None,
                sample_rate: None,
                buffer_size: Some(128),
            },
        }),
    ))
    .unwrap();
    match wait_reply(&c, 6).result {
        ReplyResult::Ok {
            value: ReplyValue::Status { status },
        } => assert_eq!(status.backend, "offline"),
        other => panic!("{other:?}"),
    }
    assert!(tmp.path().join("config/audio.json").is_file());

    host.shutdown();
}

#[test]
fn json_messages_and_bad_input() {
    let tmp = TempDir::new("null-host-json");
    let (host, c) = start(&tmp);
    host.send_json(
        r#"{"id":9,"gesture":null,"command":{"domain":"Transport","command":{"type":"Play"}}}"#,
    )
    .unwrap();
    assert!(matches!(wait_reply(&c, 9).result, ReplyResult::Ok { .. }));
    assert!(host.send_json("{not json").is_err());
    drop(host);
}

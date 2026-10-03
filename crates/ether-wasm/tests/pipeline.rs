//! Worker ↔ Worklet pipeline, natively: controller (the test-only `fake` stand-in, and the
//! real `EtherController` at the end) + `WebBridge` on one side,
//! `EngineHost` on the other, connected by heap-backed rings (the SAB stand-in).

use std::sync::Arc;

use ether_controller::{Controller, EngineBridge, HostServices};
use ether_core::graph::{ClipContentDesc, ClipDesc, TrackDesc};
use ether_core::protocol::message::{Command, Event, ReplyResult, ReplyValue};
use ether_core::protocol::model::{ClipId, MediaId, MediaRef, ProjectId, TrackId, TrackKind, Ulid};
use ether_core::protocol::project::ProjectCommand;
use ether_core::protocol::transport::TransportCommand;
use ether_core::protocol::{ClientMessage, ServerMessage};
use ether_core::{EngineOutputs, RenderGraphDesc, TransportControl};
use ether_media::DecodedAudio;
use ether_wasm::bridge::{self, WebBridge};
mod fake;
use ether_wasm::ring::HeapMemory;
use ether_wasm::store::{MemFs, WebStore};
use ether_wasm::worklet::{EngineHost, RENDER_QUANTUM};
use fake::FakeController;

struct TestHost;

impl HostServices for TestHost {
    fn now_ms(&self) -> u64 {
        1_750_000_000_000
    }
    fn random_seed(&mut self) -> u64 {
        42
    }
}

type Fake = FakeController<WebBridge<HeapMemory>, TestHost, WebStore<MemFs>>;

fn setup(ring: usize) -> (Fake, EngineHost<HeapMemory>, bridge::Shared<HeapMemory>) {
    let control = HeapMemory::new(ring);
    let reports = HeapMemory::new(1 << 14);
    let shared = bridge::shared(control.clone(), reports.clone());
    let ctl = FakeController::new(
        WebBridge::new(shared.clone()),
        TestHost,
        WebStore::new(MemFs::new()),
    );
    (ctl, EngineHost::new(48_000, control, reports), shared)
}

fn send(ctl: &mut impl Controller, id: u32, command: Command) -> Vec<ServerMessage> {
    let mut out = Vec::new();
    ctl.handle(
        ClientMessage {
            id,
            gesture: None,
            command,
        },
        &mut out,
    );
    out
}

fn reply(out: &[ServerMessage]) -> &ReplyResult {
    match out.last() {
        Some(ServerMessage::Reply(r)) => &r.result,
        other => panic!("last message must be the reply, got {other:?}"),
    }
}

fn pid() -> ProjectId {
    ProjectId::v7(1_750_000_000_000, [7; 10])
}

#[test]
fn create_play_and_playhead_advances() {
    let (mut ctl, mut engine, _fs) = setup(1 << 16);
    let out = send(
        &mut ctl,
        1,
        Command::Project(ProjectCommand::Create {
            id: pid(),
            name: "Web".into(),
        }),
    );
    assert!(matches!(
        out[0],
        ServerMessage::Event(Event::ProjectLoaded { .. })
    ));
    assert!(
        matches!(reply(&out), ReplyResult::Ok { value: ReplyValue::Project { project } } if project.settings.name == "Web")
    );

    let out = send(&mut ctl, 2, Command::Transport(TransportCommand::Play));
    assert!(
        out.iter().any(
            |m| matches!(m, ServerMessage::Event(Event::Transport { state }) if state.playing)
        )
    );

    // ~0.5 s of audio.
    for _ in 0..188 {
        engine.render(RENDER_QUANTUM);
    }
    let mut out = Vec::new();
    ctl.tick(0, &mut out);
    let ServerMessage::Playhead(frame) = &out[0] else {
        panic!("expected playhead, got {out:?}")
    };
    assert!(frame.transport.playing);
    // 120 bpm → 2 beats/s.
    let beats = frame.transport.position.0;
    assert!((beats - 1.0).abs() < 0.05, "position {beats}");

    let out = send(&mut ctl, 3, Command::Transport(TransportCommand::Stop));
    assert!(matches!(reply(&out), ReplyResult::Ok { .. }));
    for _ in 0..8 {
        engine.render(RENDER_QUANTUM);
    }
    let mut out = Vec::new();
    ctl.tick(0, &mut out);
    let ServerMessage::Playhead(frame) = &out[0] else {
        panic!()
    };
    assert!(!frame.transport.playing);
    assert_eq!(frame.transport.position.0, 0.0, "stop returns to start");
}

#[test]
fn project_is_persisted_and_reopened() {
    let (mut ctl, _engine, _fs) = setup(1 << 16);
    send(
        &mut ctl,
        1,
        Command::Project(ProjectCommand::Create {
            id: pid(),
            name: "Persisted".into(),
        }),
    );
    let out = send(&mut ctl, 2, Command::Project(ProjectCommand::List));
    let ReplyResult::Ok {
        value: ReplyValue::Projects { projects },
    } = reply(&out)
    else {
        panic!()
    };
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].name, "Persisted");
    let out = send(
        &mut ctl,
        3,
        Command::Project(ProjectCommand::Open { id: pid() }),
    );
    assert!(
        matches!(reply(&out), ReplyResult::Ok { value: ReplyValue::Project { project } } if project.id == pid())
    );
    let out = send(&mut ctl, 4, Command::Project(ProjectCommand::Save));
    assert!(matches!(reply(&out), ReplyResult::Err { .. }));
}

/// Media (larger than the control ring) streams through, and an audio clip plays it.
#[test]
fn media_streams_through_small_ring_and_plays() {
    let control = HeapMemory::new(1 << 12);
    let reports = HeapMemory::new(1 << 14);
    let shared = bridge::shared(control.clone(), reports.clone());
    let mut bridge = WebBridge::new(shared.clone());
    let mut engine = EngineHost::new(48_000, control, reports);

    let media_id = MediaId(Ulid(9));
    let frames = 48_000;
    let audio = Arc::new(DecodedAudio {
        sample_rate: 48_000,
        channels: vec![vec![0.5; frames], vec![-0.25; frames]],
    });
    let media = MediaRef {
        location: Default::default(),
        id: media_id,
        name: "tone".into(),
        file: "media/tone.wav".into(),
        sample_rate: 48_000,
        channels: 2,
        frames: frames as u64,
        hash: None,
    };
    bridge.load_media(&media, audio).unwrap();
    assert!(shared.borrow().control.pending_bytes() > 300_000);

    let master = TrackId(Ulid(1));
    let track = |id, kind, output, clips| TrackDesc {
        modulation: Default::default(),
        vca: Default::default(),
        chain_racks: Default::default(),
        frozen: Default::default(),
        input_tap: Default::default(),
        id,
        kind,
        chain: vec![],
        output,
        group: None,
        sends: vec![],
        volume: 1.0,
        pan: 0.0,
        mute: false,
        solo: false,
        audio_input: None,
        monitor: false,
        armed: false,
        clips,
        automation: vec![],
        racks: Vec::new(),
        expression: Default::default(),
        hw_io: Vec::new(),
    };
    let clip = ClipDesc {
        id: ClipId(Ulid(3)),
        start: 0.0,
        length: 8.0,
        offset: 0.0,
        looping: None,
        muted: false,
        content: ClipContentDesc::Audio {
            media: media_id,
            gain: 1.0,
            transpose: 0.0,
            fade_in: 0.0,
            fade_out: 0.0,
            fade_in_curve: Default::default(),
            fade_out_curve: Default::default(),
            reversed: false,
            warp: None,
        },
        envelopes: vec![],
    };
    bridge
        .publish(RenderGraphDesc {
            version: 1,
            tracks: vec![
                track(master, TrackKind::Master, None, vec![]),
                track(TrackId(Ulid(2)), TrackKind::Audio, Some(master), vec![clip]),
            ],
            ..Default::default()
        })
        .unwrap();
    bridge.transport(TransportControl::Play).unwrap();

    let mut out = EngineOutputs::default();
    let mut peak = 0.0f32;
    for _ in 0..4000 {
        engine.render(RENDER_QUANTUM);
        bridge.poll(&mut out);
        peak = peak.max(engine.output(0).iter().fold(0.0, |m, s| m.max(s.abs())));
        if peak > 0.1 && shared.borrow().control.pending_bytes() == 0 {
            break;
        }
    }
    assert_eq!(shared.borrow().control.pending_bytes(), 0);
    assert!(peak > 0.1, "audio clip should be audible, peak {peak}");
    assert!(
        shared.borrow().errors.is_empty(),
        "{:?}",
        shared.borrow().errors
    );
}

#[test]
fn engine_errors_come_back_to_the_worker() {
    let (mut ctl, mut engine, shared) = setup(1 << 16);
    let unknown = ether_core::NodeKey {
        index: 77,
        generation: 1,
    };
    ctl.bridge().destroy_node(unknown).unwrap();
    for _ in 0..8 {
        engine.render(RENDER_QUANTUM);
    }
    let mut out = EngineOutputs::default();
    ctl.bridge().poll(&mut out);
    assert!(out.playhead.is_some());
    let errors = &shared.borrow().errors;
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("unknown node"), "{errors:?}");
    assert!(shared.borrow().blocks >= 4);
}

/// Built-in devices are created in the Worklet under virtual keys that the graph and param
/// changes reference; the Worklet maps them to real engine keys.
#[test]
fn builtin_synth_under_virtual_key_plays_notes() {
    use ether_core::graph::{ChainEntry, NoteDesc};
    use ether_core::protocol::model::{BuiltinDevice, DeviceId};
    use ether_core::{ParamChange, ParamTarget};

    let control = HeapMemory::new(1 << 16);
    let reports = HeapMemory::new(1 << 14);
    let shared = bridge::shared(control.clone(), reports.clone());
    let mut bridge = WebBridge::new(shared.clone());
    let mut engine = EngineHost::new(48_000, control, reports);

    let device = DeviceId(Ulid(5));
    let key = bridge
        .create_builtin(device, &BuiltinDevice::Synth, &[])
        .unwrap();
    assert!(bridge.descriptor(device).is_some());
    let master = TrackId(Ulid(1));
    let midi = TrackDesc {
        modulation: Default::default(),
        vca: Default::default(),
        chain_racks: Default::default(),
        frozen: Default::default(),
        input_tap: Default::default(),
        id: TrackId(Ulid(2)),
        kind: TrackKind::Midi,
        chain: vec![ChainEntry {
            node: key,
            enabled: true,
            sidechain: None,
        }],
        output: Some(master),
        group: None,
        sends: vec![],
        volume: 1.0,
        pan: 0.0,
        mute: false,
        solo: false,
        audio_input: None,
        monitor: false,
        armed: false,
        clips: vec![ClipDesc {
            id: ClipId(Ulid(3)),
            start: 0.0,
            length: 4.0,
            offset: 0.0,
            looping: None,
            muted: false,
            content: ClipContentDesc::Midi {
                notes: vec![NoteDesc {
                    start: 0.0,
                    duration: 2.0,
                    key: 60,
                    velocity: 1.0,
                    release_velocity: 0.5,
                }],
            },
            envelopes: vec![],
        }],
        automation: vec![],
        racks: Vec::new(),
        expression: Default::default(),
        hw_io: Vec::new(),
    };
    let mut master_desc = midi.clone();
    master_desc.id = master;
    master_desc.kind = TrackKind::Master;
    master_desc.chain.clear();
    master_desc.clips.clear();
    master_desc.output = None;
    bridge
        .publish(RenderGraphDesc {
            version: 1,
            tracks: vec![master_desc, midi],
            ..Default::default()
        })
        .unwrap();
    let info = bridge.descriptor(device).unwrap().params[0].clone();
    let (param, default) = (info.id, info.default);
    bridge
        .set_param(ParamChange {
            target: ParamTarget::Node { node: key, param },
            value: default,
        })
        .unwrap();
    bridge.transport(TransportControl::Play).unwrap();

    let mut peak = 0.0f32;
    for _ in 0..100 {
        engine.render(RENDER_QUANTUM);
        peak = peak.max(engine.output(0).iter().fold(0.0, |m, s| m.max(s.abs())));
    }
    let mut out = EngineOutputs::default();
    bridge.poll(&mut out);
    assert!(
        shared.borrow().errors.is_empty(),
        "{:?}",
        shared.borrow().errors
    );
    assert!(peak > 0.01, "synth should sound, peak {peak}");
    assert!(!out.meters.is_empty());

    bridge.destroy_node(key).unwrap();
    engine.render(RENDER_QUANTUM);
    bridge.poll(&mut out);
    assert!(shared.borrow().errors.is_empty());
}

/// A backlog (e.g. after a suspended AudioContext resumes) is applied over several quanta:
/// bounded frames per quantum, one heavy frame (Publish) per quantum, and nothing is lost
/// to core's queue limits.
#[test]
fn backlog_is_applied_in_bounded_steps_without_queue_overflow() {
    use ether_core::{ParamChange, ParamTarget};
    use ether_wasm::worklet::MAX_FRAMES_PER_QUANTUM;

    let control = HeapMemory::new(1 << 20);
    let reports = HeapMemory::new(1 << 14);
    let shared = bridge::shared(control.clone(), reports.clone());
    let mut bridge = WebBridge::new(shared.clone());
    let mut engine = EngineHost::new(48_000, control, reports);
    let master = TrackId(Ulid(1));
    let graph = RenderGraphDesc {
        version: 1,
        tracks: vec![TrackDesc {
            modulation: Default::default(),
            vca: Default::default(),
            chain_racks: Default::default(),
            frozen: Default::default(),
            input_tap: Default::default(),
            id: master,
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
            racks: Vec::new(),
            expression: Default::default(),
            hw_io: Vec::new(),
        }],
        ..Default::default()
    };

    // Two publishes: the first quantum stops right after the first one.
    bridge.publish(graph.clone()).unwrap();
    bridge.publish(graph).unwrap();
    engine.render(RENDER_QUANTUM);
    assert!(
        engine.control_backlog() > 0,
        "second publish waits a quantum"
    );
    engine.render(RENDER_QUANTUM);
    assert_eq!(engine.control_backlog(), 0);

    // 5000 param changes: more than core's control queue, applied in bounded steps.
    let n = 5000;
    for i in 0..n {
        bridge
            .set_param(ParamChange {
                target: ParamTarget::TrackVolume { track: master },
                value: i as f64 / n as f64,
            })
            .unwrap();
    }
    let mut quanta = 0;
    while engine.control_backlog() > 0 || shared.borrow().control.pending_bytes() > 0 {
        engine.render(RENDER_QUANTUM);
        shared.borrow_mut().control.flush();
        quanta += 1;
        assert!(quanta < 10_000);
    }
    assert!(quanta >= n / MAX_FRAMES_PER_QUANTUM, "{quanta} quanta");
    let mut out = EngineOutputs::default();
    engine.render(RENDER_QUANTUM);
    bridge.poll(&mut out);
    assert!(
        shared.borrow().errors.is_empty(),
        "{:?}",
        shared.borrow().errors
    );
}

/// The real `EtherController` over the web bridge, rings, Worklet host and store.
#[test]
fn ether_controller_creates_plays_and_saves_through_the_web_host() {
    use ether_controller::{ControllerConfig, EtherController};
    use ether_wasm::store::WebLibrary;

    let control = HeapMemory::new(1 << 16);
    let reports = HeapMemory::new(1 << 14);
    let shared = bridge::shared(control.clone(), reports.clone());
    let fs = MemFs::new();
    let mut ctl = EtherController::with_config(
        WebBridge::new(shared.clone()),
        TestHost,
        WebStore::new(fs.clone()),
        WebLibrary::new(fs.clone()),
        ControllerConfig {
            engine_sample_rate: 48_000,
            ..ControllerConfig::default()
        },
    );
    let mut engine = EngineHost::new(48_000, control, reports);

    let out = send(
        &mut ctl,
        1,
        Command::Project(ProjectCommand::Create {
            id: pid(),
            name: "Real".into(),
        }),
    );
    assert!(matches!(reply(&out), ReplyResult::Ok { .. }), "{out:?}");
    let out = send(&mut ctl, 2, Command::Transport(TransportCommand::Play));
    assert!(matches!(reply(&out), ReplyResult::Ok { .. }), "{out:?}");

    let mut position = 0.0;
    for i in 0..200u64 {
        engine.render(RENDER_QUANTUM);
        if i % 8 == 0 {
            let mut out = Vec::new();
            ctl.tick(1_750_000_000_000 + i, &mut out);
            for m in out {
                if let ServerMessage::Playhead(f) = m {
                    position = f.transport.position.0;
                }
            }
        }
    }
    assert!(position > 0.5, "playhead {position}");
    assert!(
        shared.borrow().errors.is_empty(),
        "{:?}",
        shared.borrow().errors
    );
    let out = send(&mut ctl, 3, Command::Project(ProjectCommand::List));
    let ReplyResult::Ok {
        value: ReplyValue::Projects { projects },
    } = reply(&out)
    else {
        panic!("{out:?}")
    };
    assert_eq!(projects[0].name, "Real");
}

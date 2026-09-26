//! Worker ↔ Worklet pipeline, natively: controller (fake) + `WebBridge` on one side,
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
use ether_wasm::fake::FakeController;
use ether_wasm::ring::HeapMemory;
use ether_wasm::store::{MemFs, WebStore};
use ether_wasm::worklet::{EngineHost, RENDER_QUANTUM};

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

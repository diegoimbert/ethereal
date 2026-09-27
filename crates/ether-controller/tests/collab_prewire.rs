//! base-53 pre-wiring (docs/COLLAB.md §8-§10): what works before the `presence-v2`,
//! `stream-host` and `stream-listen` nodes land, and what replies `Unsupported` until then.
//! Each node removes ONLY its own `Unsupported` assertions here (shared file) and tests
//! its behaviour in its own test file.

#[path = "collab_support.rs"]
mod support;

use ether_collab::memory::Hub;
use ether_core::protocol::collab::*;
use ether_core::protocol::model::*;
use ether_core::protocol::*;
use support::*;

fn error_code(out: &[ServerMessage]) -> ErrorCode {
    match out.last() {
        Some(ServerMessage::Reply(Reply {
            result: ReplyResult::Err { error },
            ..
        })) => error.code,
        other => panic!("expected an error reply, got {other:#?}"),
    }
}

fn collab_events(out: &[ServerMessage]) -> Vec<CollabEvent> {
    out.iter()
        .filter_map(|m| match m {
            ServerMessage::Event(Event::Collab { event }) => Some(event.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn node_commands_reply_unsupported_until_implemented() {
    let hub = Hub::default();
    let mut sites = session(&hub, 2);
    let host = sites[0].ctl.collab_site();
    let s = &mut sites[1];
    let clock = StreamClock {
        rtp: 0,
        position: Beats(0.0),
        playing: false,
        recording: false,
        bpm: 120.0,
        loop_enabled: false,
        loop_region: BeatRange {
            start: Beats(0.0),
            end: Beats(4.0),
        },
        metronome: false,
        discontinuity: true,
    };
    for c in [
        // presence-v2
        CollabCommand::SetPointer { pointer: None },
        // stream-listen
        CollabCommand::Listen { host },
        CollabCommand::StopListening,
        // stream-host
        CollabCommand::SetHosting {
            allow: true,
            ui_sender: true,
            remote_transport: true,
        },
        CollabCommand::SendStreamClock {
            to: host,
            stream: 1,
            clock: clock.clone(),
        },
    ] {
        let out = s.send(Command::Collab(c.clone()));
        assert_eq!(error_code(&out), ErrorCode::Unsupported, "{c:?}");
    }
}

#[test]
fn ice_server_settings_override_the_relay() {
    let hub = Hub::default();
    let mut sites = session(&hub, 1);
    let s = &mut sites[0];
    let stun = vec![IceServer {
        urls: vec!["stun:example.test:3478".into()],
        username: None,
        credential: None,
    }];
    let out = s.send(Command::Collab(CollabCommand::SetIceServers {
        servers: Some(stun.clone()),
    }));
    assert_eq!(
        collab_events(&out),
        [CollabEvent::IceServers {
            servers: stun,
            source: IceServerSource::Settings
        }]
    );
    let out = s.send(Command::Collab(CollabCommand::SetIceServers {
        servers: None,
    }));
    assert_eq!(
        collab_events(&out),
        [CollabEvent::IceServers {
            servers: vec![],
            source: IceServerSource::Relay
        }],
        "the in-memory hub advertises none"
    );
}

#[test]
fn signals_are_forwarded_only_inside_a_session() {
    let hub = Hub::default();
    let mut lone = Site::on_hub(5, &hub);
    let out = lone.send(Command::Collab(CollabCommand::SendSignal {
        to: SiteId(1),
        stream: 1,
        signal: StreamSignal::Bye { reason: None },
    }));
    assert_eq!(error_code(&out), ErrorCode::InvalidState);

    let mut sites = session(&hub, 2);
    let a = sites[0].ctl.collab_site();
    let b = sites[1].ctl.collab_site();
    let out = sites[1].send(Command::Collab(CollabCommand::SendSignal {
        to: b,
        stream: 1,
        signal: StreamSignal::Bye { reason: None },
    }));
    assert_eq!(
        error_code(&out),
        ErrorCode::InvalidArgument,
        "not to itself"
    );
    sites[1].ok(Command::Collab(CollabCommand::SendSignal {
        to: a,
        stream: 1,
        signal: StreamSignal::Bye { reason: None },
    }));
    let mut refs: Vec<&mut Site> = sites.iter_mut().collect();
    settle(&mut refs, &hub);
}

#[test]
fn controller_owned_presence_fields_are_overwritten() {
    let hub = Hub::default();
    let mut sites = session(&hub, 2);
    let a = sites[0].ctl.collab_site();
    let viewport = ArrangerViewport {
        start: Beats(0.0),
        end: Beats(16.0),
        top_track: None,
        top_offset: 0.0,
    };
    sites[1].ok(Command::Collab(CollabCommand::SetPresence {
        presence: PresenceState {
            viewport: Some(viewport.clone()),
            following: Some(a),
            listening_to: Some(a),
            can_host: true,
            ..PresenceState::default()
        },
    }));
    sites[1].advance(1_000);
    let mut refs: Vec<&mut Site> = sites.iter_mut().collect();
    settle(&mut refs, &hub);
    let peers = sites[0].peers();
    let state = &peers[0].state;
    assert_eq!(state.viewport, Some(viewport), "UI-owned fields travel");
    assert_eq!(state.following, Some(a));
    assert_eq!(state.listening_to, None, "controller-owned");
    assert!(!state.can_host, "controller-owned");
}

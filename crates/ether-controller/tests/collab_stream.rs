//! Listen on a peer, both sides together (`stream-listen` × `stream-host`; docs/COLLAB.md
//! §9): real controllers on the in-memory relay, the host with a UI sender (web build), the
//! UIs' WebRTC played by the test (signals and clocks through `SendSignal`/`SendStreamClock`).

#[path = "collab_support.rs"]
mod support;

use ether_collab::memory::Hub;
use ether_core::TransportControl;
use ether_core::protocol::collab::*;
use ether_core::protocol::model::*;
use ether_core::protocol::transport::TransportCommand;
use ether_core::protocol::*;
use support::*;

fn events(site: &Site) -> Vec<CollabEvent> {
    site.log
        .iter()
        .filter_map(|m| match m {
            ServerMessage::Event(Event::Collab { event }) => Some(event.clone()),
            _ => None,
        })
        .collect()
}

fn status(site: &Site) -> ListenStatus {
    events(site)
        .into_iter()
        .rev()
        .find_map(|e| match e {
            CollabEvent::ListenStatus { status } => Some(status),
            _ => None,
        })
        .unwrap_or_default()
}

fn signals(site: &Site) -> Vec<(SiteId, u32, StreamSignal)> {
    events(site)
        .into_iter()
        .filter_map(|e| match e {
            CollabEvent::Signal {
                from,
                stream,
                signal,
            } => Some((from, stream, signal)),
            _ => None,
        })
        .collect()
}

fn last_transport(site: &Site) -> ether_core::protocol::transport::TransportState {
    site.log
        .iter()
        .rev()
        .find_map(|m| match m {
            ServerMessage::Event(Event::Transport { state }) => Some(state.clone()),
            _ => None,
        })
        .expect("a transport event")
}

fn engine_calls(site: &Site) -> Vec<TransportControl> {
    site.ctl
        .bridge
        .calls
        .iter()
        .filter_map(|c| match c {
            Call::Transport(t) => Some(*t),
            _ => None,
        })
        .collect()
}

fn presence_of(site: &Site, of: SiteId) -> Option<PresenceState> {
    site.peers()
        .into_iter()
        .find(|p| p.site == of)
        .map(|p| p.state)
}

fn clock(rtp: u32, position: f64, playing: bool) -> StreamClock {
    StreamClock {
        rtp,
        position: Beats(position),
        playing,
        recording: false,
        bpm: 100.0,
        loop_enabled: false,
        loop_region: BeatRange {
            start: Beats(0.0),
            end: Beats(8.0),
        },
        metronome: false,
        discontinuity: true,
        count_in_end: None,
    }
}

/// `n` sites, each declaring a UI sender (so each can host).
fn world(hub: &Hub, n: usize) -> Vec<Site> {
    let mut sites = session(hub, n);
    for s in sites.iter_mut() {
        s.ok(Command::Collab(CollabCommand::SetHosting {
            allow: true,
            ui_sender: true,
            remote_transport: true,
        }));
    }
    flush(&mut sites, hub);
    sites
}

/// Let presence throttles pass and deliver everything.
fn flush(sites: &mut [Site], hub: &Hub) {
    for s in sites.iter_mut() {
        s.advance(1_000);
    }
    let mut refs: Vec<&mut Site> = sites.iter_mut().collect();
    settle(&mut refs, hub);
}

#[test]
fn a_web_host_streams_to_a_listener_and_shares_its_transport() {
    let hub = Hub::default();
    let mut s = world(&hub, 2);
    let host = s[0].ctl.collab_site();
    let me = s[1].ctl.collab_site();
    assert!(presence_of(&s[1], host).expect("host presence").can_host);

    s[1].ok(Command::Collab(CollabCommand::Listen { host }));
    flush(&mut s, &hub);
    let ListenState::Connecting { stream, .. } = status(&s[1]).listening else {
        panic!("connecting: {:?}", status(&s[1]));
    };
    // The host's UI sender gets the listener.
    assert_eq!(
        status(&s[0]).listeners,
        [ListenerLink {
            site: me,
            stream,
            endpoint: StreamEndpoint::Ui
        }]
    );

    // Offer (host UI) → listener UI; answer back → host UI.
    let offer = StreamSignal::Offer {
        sdp: "v=0 o".into(),
    };
    s[0].ok(Command::Collab(CollabCommand::SendSignal {
        to: me,
        stream,
        signal: offer.clone(),
    }));
    flush(&mut s, &hub);
    assert!(signals(&s[1]).contains(&(host, stream, offer)));
    let answer = StreamSignal::Answer {
        sdp: "v=0 a".into(),
    };
    s[1].ok(Command::Collab(CollabCommand::SendSignal {
        to: host,
        stream,
        signal: answer.clone(),
    }));
    flush(&mut s, &hub);
    assert!(signals(&s[0]).contains(&(me, stream, answer)));

    // Media flows: the host's anchors make the listener Listening, its transport follows.
    s[0].ok(Command::Collab(CollabCommand::SendStreamClock {
        to: me,
        stream,
        clock: clock(1_000, 4.0, true),
    }));
    flush(&mut s, &hub);
    assert!(matches!(
        status(&s[1]).listening,
        ListenState::Listening { host: h, .. } if h == host
    ));
    let t = last_transport(&s[1]);
    assert!(t.playing);
    assert_eq!(t.bpm, 100.0);
    assert_eq!(presence_of(&s[0], me).unwrap().listening_to, Some(host));

    // The listener's Play/Locate run on the host's engine, never on its own.
    let host_before = engine_calls(&s[0]).len();
    s[1].ok(Command::Transport(TransportCommand::Play));
    s[1].ok(Command::Transport(TransportCommand::Locate {
        position: Beats(16.0),
    }));
    flush(&mut s, &hub);
    let host_calls = engine_calls(&s[0]);
    assert!(host_calls[host_before..].contains(&TransportControl::Play));
    assert!(
        host_calls[host_before..].contains(&TransportControl::Locate {
            position: Beats(16.0)
        })
    );
    assert!(!engine_calls(&s[1]).contains(&TransportControl::Play));

    // A stop anchor freezes the listener's transport; StopListening ends the host's link.
    s[0].ok(Command::Collab(CollabCommand::SendStreamClock {
        to: me,
        stream,
        clock: clock(9_000, 16.0, false),
    }));
    flush(&mut s, &hub);
    assert!(!last_transport(&s[1]).playing);
    s[1].ok(Command::Collab(CollabCommand::StopListening));
    flush(&mut s, &hub);
    assert!(status(&s[0]).listeners.is_empty());
    assert_eq!(status(&s[1]).listening, ListenState::Off);
    assert_eq!(
        engine_calls(&s[1]).last(),
        Some(&TransportControl::Locate {
            position: Beats(16.0)
        }),
        "back on the local engine where it was heard"
    );
}

#[test]
fn a_connecting_site_cannot_host_no_chains() {
    let hub = Hub::default();
    let mut s = world(&hub, 3);
    let a = s[0].ctl.collab_site();
    let b = s[1].ctl.collab_site();
    assert!(presence_of(&s[2], b).unwrap().can_host);

    // B asks A (still connecting: no anchor, `listening_to` unset).
    s[1].ok(Command::Collab(CollabCommand::Listen { host: a }));
    flush(&mut s, &hub);
    assert!(matches!(
        status(&s[1]).listening,
        ListenState::Connecting { .. }
    ));
    let seen = presence_of(&s[2], b).unwrap();
    assert_eq!(seen.listening_to, None);
    assert!(!seen.can_host, "a connecting site does not offer hosting");

    // C asks B anyway: refused with a Bye, B hosts nobody.
    s[2].ok(Command::Collab(CollabCommand::Listen { host: b }));
    flush(&mut s, &hub);
    assert!(matches!(
        status(&s[2]).listening,
        ListenState::Ended { host, .. } if host == b
    ));
    assert!(status(&s[1]).listeners.is_empty());
}

#[test]
fn a_host_that_starts_listening_ends_its_own_streams() {
    let hub = Hub::default();
    let mut s = world(&hub, 3);
    let a = s[0].ctl.collab_site();
    let c = s[2].ctl.collab_site();
    // B listens to A.
    s[1].ok(Command::Collab(CollabCommand::Listen { host: a }));
    flush(&mut s, &hub);
    assert_eq!(status(&s[0]).listeners.len(), 1);

    // A starts listening to C: B's stream ends with the reason, A hosts nobody.
    s[0].ok(Command::Collab(CollabCommand::Listen { host: c }));
    flush(&mut s, &hub);
    assert!(status(&s[0]).listeners.is_empty());
    assert_eq!(
        status(&s[1]).listening,
        ListenState::Ended {
            host: a,
            reason: "host started listening elsewhere".into()
        }
    );
    assert!(matches!(
        status(&s[0]).listening,
        ListenState::Connecting { host, .. } if host == c
    ));
}

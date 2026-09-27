//! Listen on a peer, listener side (`stream-listen`; docs/COLLAB.md §9): a real controller
//! listening to a **scripted host** (a raw link on the in-memory hub that answers like a
//! host bridge would: offer, ICE, clock anchors, bye).

#[path = "collab_support.rs"]
mod support;

use ether_collab::memory::{Hub, MemoryLink};
use ether_collab::{CollabTransport, ConnectRequest};
use ether_core::TransportControl;
use ether_core::protocol::collab::*;
use ether_core::protocol::model::*;
use ether_core::protocol::recording::RecordingCommand;
use ether_core::protocol::transport::{TransportCommand, TransportState};
use ether_core::protocol::*;
use support::*;

/// A scripted host site: joins the session over a raw link and speaks the wire directly.
struct FakeHost {
    link: MemoryLink,
    site: SiteId,
    /// Everything received (after the join handshake).
    inbox: Vec<CollabMessage>,
}

impl FakeHost {
    fn join(hub: &Hub, site: u64, name: &str) -> Self {
        let mut link = hub.link(&ConnectRequest {
            server: "ws://hub".into(),
            session: "jam".into(),
            token: None,
            client: "scripted host".into(),
        });
        let site = SiteId(site);
        link.send(&CollabMessage::Hello {
            site,
            actor: None,
            name: name.into(),
            protocol_version: ether_collab::wire::COLLAB_PROTOCOL_VERSION,
        });
        link.send(&CollabMessage::SyncRequest {
            site,
            version: Base64Bytes(Vec::new()),
        });
        Self {
            link,
            site,
            inbox: Vec::new(),
        }
    }

    fn send(&mut self, m: CollabMessage) {
        self.link.send(&m);
    }

    fn poll(&mut self) {
        let mut v = Vec::new();
        self.link.poll(&mut v);
        self.inbox.extend(v);
    }

    /// Site-to-site messages received, oldest first.
    fn routed(&mut self) -> Vec<CollabMessage> {
        self.poll();
        self.inbox
            .iter()
            .filter(|m| m.route().is_some())
            .cloned()
            .collect()
    }

    fn signal(&mut self, to: SiteId, stream: u32, signal: StreamSignal) {
        self.send(CollabMessage::Signal {
            from: self.site,
            to,
            stream,
            signal,
        });
    }

    fn clock(&mut self, to: SiteId, stream: u32, clock: StreamClock) {
        self.send(CollabMessage::StreamClock {
            from: self.site,
            to,
            stream,
            clock,
        });
    }
}

const HOST: u64 = 0xdead_beef;

fn anchor(rtp: u32, position: f64, playing: bool) -> StreamClock {
    StreamClock {
        rtp,
        position: Beats(position),
        playing,
        recording: false,
        bpm: 90.0,
        loop_enabled: true,
        loop_region: BeatRange {
            start: Beats(4.0),
            end: Beats(12.0),
        },
        metronome: true,
        discontinuity: true,
        count_in_end: None,
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

fn listen_state(site: &Site) -> Option<ListenState> {
    site.log.iter().rev().find_map(|m| match m {
        ServerMessage::Event(Event::Collab {
            event: CollabEvent::ListenStatus { status },
        }) => Some(status.listening.clone()),
        _ => None,
    })
}

fn last_transport(site: &Site) -> Option<TransportState> {
    site.log.iter().rev().find_map(|m| match m {
        ServerMessage::Event(Event::Transport { state }) => Some(state.clone()),
        _ => None,
    })
}

fn error_code(out: &[ServerMessage]) -> ErrorCode {
    match out.last() {
        Some(ServerMessage::Reply(Reply {
            result: ReplyResult::Err { error },
            ..
        })) => error.code,
        other => panic!("expected an error reply, got {other:#?}"),
    }
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

struct World {
    hub: Hub,
    sites: Vec<Site>,
    host: FakeHost,
}

impl World {
    /// Site 0 (a peer), site 1 (the listener), and the scripted host.
    fn new() -> Self {
        let hub = Hub::default();
        let sites = session(&hub, 2);
        let host = FakeHost::join(&hub, HOST, "Diego");
        let mut w = Self { hub, sites, host };
        w.settle();
        w
    }

    /// Tick every site, deliver, and drain the scripted host until nothing moves.
    fn settle(&mut self) {
        for _ in 0..500 {
            for s in self.sites.iter_mut() {
                s.tick();
            }
            let delivered = self.hub.deliver();
            self.host.poll();
            let waiting: usize = self.hub.links().iter().map(|c| self.hub.outgoing(*c)).sum();
            let pending: usize = self.sites.iter().map(|s| s.ctl.collab_pending()).sum();
            if delivered == 0 && waiting == 0 && pending == 0 {
                for s in self.sites.iter_mut() {
                    s.tick();
                }
                return;
            }
        }
        panic!("did not settle");
    }

    fn listener(&mut self) -> &mut Site {
        &mut self.sites[1]
    }

    fn me(&mut self) -> SiteId {
        self.sites[1].ctl.collab_site()
    }

    /// `Listen` on the scripted host; returns the stream id it received.
    fn listen(&mut self) -> u32 {
        self.listener().ok(Command::Collab(CollabCommand::Listen {
            host: SiteId(HOST),
        }));
        self.settle();
        let me = self.me();
        match self.host.routed().last() {
            Some(CollabMessage::Listen { from, to, stream }) => {
                assert_eq!((*from, *to), (me, SiteId(HOST)));
                *stream
            }
            other => panic!("expected Listen at the host, got {other:?}"),
        }
    }

    /// Listen, then offer + first anchor (media flowing).
    fn listening(&mut self) -> u32 {
        let stream = self.listen();
        let me = self.me();
        self.host.signal(
            me,
            stream,
            StreamSignal::Offer {
                sdp: "v=0 offer".into(),
            },
        );
        self.host.clock(me, stream, anchor(1_000, 8.0, true));
        self.settle();
        assert_eq!(
            listen_state(&self.sites[1]),
            Some(ListenState::Listening {
                host: SiteId(HOST),
                stream
            })
        );
        stream
    }
}

#[test]
fn happy_path_signals_clocks_status_and_presence() {
    let mut w = World::new();
    let me = w.me();
    let out = w.listener().send(Command::Collab(CollabCommand::Listen {
        host: SiteId(HOST),
    }));
    let stream = match collab_events(&out).as_slice() {
        [
            CollabEvent::ListenStatus {
                status:
                    ListenStatus {
                        listening: ListenState::Connecting { host, stream },
                        listeners,
                    },
            },
        ] => {
            assert_eq!(*host, SiteId(HOST));
            assert!(listeners.is_empty());
            *stream
        }
        other => panic!("expected Connecting, got {other:?}"),
    };
    assert_ne!(stream, 0);
    w.settle();
    assert!(matches!(
        w.host.routed().as_slice(),
        [CollabMessage::Listen { stream: s, .. }] if *s == stream
    ));

    // Offer and ICE go to the UI receiver.
    let offer = StreamSignal::Offer {
        sdp: "v=0 offer".into(),
    };
    let ice = StreamSignal::Ice {
        candidate: IceCandidate {
            candidate: "candidate:1 1 udp 1 10.0.0.1 5000 typ host".into(),
            sdp_mid: Some("0".into()),
            sdp_m_line_index: Some(0),
            username_fragment: None,
        },
    };
    w.host.signal(me, stream, offer.clone());
    w.host.signal(me, stream, ice.clone());
    let before = w.sites[1].log.len();
    w.settle();
    let events = collab_events(&w.sites[1].log[before..]);
    assert!(events.contains(&CollabEvent::Signal {
        from: SiteId(HOST),
        stream,
        signal: offer
    }));
    assert!(events.contains(&CollabEvent::Signal {
        from: SiteId(HOST),
        stream,
        signal: ice
    }));

    // The UI's answer reaches the host.
    let answer = StreamSignal::Answer {
        sdp: "v=0 answer".into(),
    };
    w.listener().ok(Command::Collab(CollabCommand::SendSignal {
        to: SiteId(HOST),
        stream,
        signal: answer.clone(),
    }));
    w.settle();
    assert!(w.host.routed().contains(&CollabMessage::Signal {
        from: me,
        to: SiteId(HOST),
        stream,
        signal: answer
    }));
    assert_eq!(
        listen_state(&w.sites[1]),
        Some(ListenState::Connecting {
            host: SiteId(HOST),
            stream
        }),
        "still connecting until the first clock"
    );

    // First clock: Listening, the clock goes to the UI, the transport mirrors the host.
    let a = anchor(48_000, 4.5, true);
    w.host.clock(me, stream, a.clone());
    let before = w.sites[1].log.len();
    w.settle();
    let events = collab_events(&w.sites[1].log[before..]);
    assert!(events.contains(&CollabEvent::StreamClock {
        from: SiteId(HOST),
        stream,
        clock: a
    }));
    assert_eq!(
        listen_state(&w.sites[1]),
        Some(ListenState::Listening {
            host: SiteId(HOST),
            stream
        })
    );
    let t = last_transport(&w.sites[1]).expect("transport event");
    assert!(t.playing && t.metronome && t.loop_enabled);
    assert_eq!(t.bpm, 90.0);
    assert_eq!(t.loop_region.end, Beats(12.0));
    assert!(
        !engine_calls(&w.sites[1]).contains(&TransportControl::Play),
        "the local engine never plays while listening"
    );

    // Our presence says whom we listen to (controller-owned).
    w.sites[1].advance(1_000);
    w.settle();
    let peers = w.sites[0].peers();
    let me_seen = peers.iter().find(|p| p.site == me).expect("listener");
    assert_eq!(me_seen.state.listening_to, Some(SiteId(HOST)));

    // Get re-emits the status.
    let out = w.listener().send(Command::Collab(CollabCommand::Get));
    assert!(collab_events(&out).iter().any(|e| matches!(
        e,
        CollabEvent::ListenStatus { status } if matches!(status.listening, ListenState::Listening { .. })
    )));
}

#[test]
fn stale_and_unsolicited_signals_are_dropped() {
    let mut w = World::new();
    let me = w.me();
    let stream = w.listen();
    let peer = w.sites[0].ctl.collab_site();
    let before = w.sites[1].log.len();
    // Wrong stream id (an old request), and the right id from another site.
    w.host.signal(
        me,
        stream.wrapping_add(1),
        StreamSignal::Offer { sdp: "old".into() },
    );
    w.host
        .clock(me, stream.wrapping_add(1), anchor(0, 0.0, true));
    w.sites[0].ok(Command::Collab(CollabCommand::SendSignal {
        to: me,
        stream,
        signal: StreamSignal::Offer {
            sdp: "unsolicited".into(),
        },
    }));
    w.settle();
    let events = collab_events(&w.sites[1].log[before..]);
    assert!(
        !events.iter().any(|e| matches!(
            e,
            CollabEvent::Signal { .. } | CollabEvent::StreamClock { .. }
        )),
        "{events:?}"
    );
    assert!(matches!(
        listen_state(&w.sites[1]),
        Some(ListenState::Connecting { .. })
    ));
    let _ = peer;
}

#[test]
fn transport_commands_are_forwarded_and_recording_refused() {
    let mut w = World::new();
    let me = w.me();
    let stream = w.listening();
    let loop_before = w.sites[1].project().settings.loop_enabled;
    let calls_before = engine_calls(&w.sites[1]).len();

    let region = BeatRange {
        start: Beats(0.0),
        end: Beats(16.0),
    };
    for (c, r) in [
        (TransportCommand::Play, TransportRequest::Play),
        (TransportCommand::Stop, TransportRequest::Stop),
        (TransportCommand::TogglePlay, TransportRequest::TogglePlay),
        (
            TransportCommand::Locate {
                position: Beats(32.0),
            },
            TransportRequest::Locate {
                position: Beats(32.0),
            },
        ),
        (
            TransportCommand::SetLoopEnabled {
                enabled: !loop_before,
            },
            TransportRequest::SetLoopEnabled {
                enabled: !loop_before,
            },
        ),
        (
            TransportCommand::SetLoopRegion { region },
            TransportRequest::SetLoopRegion { region },
        ),
    ] {
        assert_eq!(w.listener().ok(Command::Transport(c)), ReplyValue::Unit);
        w.settle();
        assert_eq!(
            w.host.routed().last(),
            Some(&CollabMessage::TransportRequest {
                from: me,
                to: SiteId(HOST),
                stream,
                request: r
            })
        );
    }
    assert_eq!(
        engine_calls(&w.sites[1]).len(),
        calls_before,
        "nothing reached the local engine"
    );
    assert_eq!(
        w.sites[1].project().settings.loop_enabled,
        loop_before,
        "loop changes are the host's"
    );
    // Invalid positions are refused locally.
    let out = w
        .listener()
        .send(Command::Transport(TransportCommand::Locate {
            position: Beats(-1.0),
        }));
    assert_eq!(error_code(&out), ErrorCode::InvalidArgument);

    // Recording is refused while listening.
    let out = w
        .listener()
        .send(Command::Recording(RecordingCommand::SetRecording {
            enabled: true,
        }));
    assert_eq!(error_code(&out), ErrorCode::InvalidState);

    // Tempo is a document edit (replicates as usual), the metronome a local setting.
    w.listener()
        .ok(Command::Transport(TransportCommand::SetTempo {
            bpm: 100.0,
        }));
    w.listener()
        .ok(Command::Transport(TransportCommand::SetMetronome {
            enabled: true,
        }));
    assert!(w.sites[1].project().settings.metronome);
    w.settle();
    assert_eq!(
        w.sites[0].project().tempo_map().bpm_at(Beats(0.0)),
        100.0,
        "tempo replicated"
    );
    assert!(
        !w.host
            .routed()
            .iter()
            .any(|m| matches!(m, CollabMessage::TransportRequest { request, .. }
                if !matches!(request, TransportRequest::Play | TransportRequest::Stop
                    | TransportRequest::TogglePlay | TransportRequest::Locate { .. }
                    | TransportRequest::SetLoopEnabled { .. } | TransportRequest::SetLoopRegion { .. }))),
    );
}

#[test]
fn the_local_transport_is_held_stopped() {
    let mut w = World::new();
    w.listener().ok(Command::Transport(TransportCommand::Play));
    assert_eq!(
        engine_calls(&w.sites[1]).last(),
        Some(&TransportControl::Play)
    );
    w.listen();
    assert_eq!(
        engine_calls(&w.sites[1]).last(),
        Some(&TransportControl::Stop),
        "Listen stops the local timeline"
    );
    assert!(!last_transport(&w.sites[1]).unwrap().playing);
}

#[test]
fn stop_listening_unlistens_and_restores_the_last_heard_position() {
    let mut w = World::new();
    let me = w.me();
    let stream = w.listening();
    let paused = anchor(2_000, 6.0, false);
    w.host.clock(me, stream, paused);
    w.settle();
    let out = w
        .listener()
        .send(Command::Collab(CollabCommand::StopListening));
    assert!(collab_events(&out).contains(&CollabEvent::ListenStatus {
        status: ListenStatus::default()
    }));
    assert_eq!(
        engine_calls(&w.sites[1]).last(),
        Some(&TransportControl::Locate {
            position: Beats(6.0)
        }),
        "stays stopped, located where it was heard"
    );
    let t = last_transport(&w.sites[1]).unwrap();
    assert!(!t.playing);
    assert_eq!(t.start_position, Beats(6.0));
    w.settle();
    assert_eq!(
        w.host.routed().last(),
        Some(&CollabMessage::Unlisten {
            from: me,
            to: SiteId(HOST),
            stream
        })
    );
    // Back on the local transport: commands are local again.
    let before = w.host.routed().len();
    w.listener().ok(Command::Transport(TransportCommand::Play));
    w.settle();
    assert_eq!(w.host.routed().len(), before);
    assert_eq!(
        engine_calls(&w.sites[1]).last(),
        Some(&TransportControl::Play)
    );
    // StopListening again is a no-op.
    w.listener()
        .ok(Command::Collab(CollabCommand::StopListening));
    w.settle();
    assert_eq!(w.host.routed().len(), before);
    // Presence cleared.
    w.sites[1].advance(1_000);
    w.settle();
    let peers = w.sites[0].peers();
    assert_eq!(
        peers
            .iter()
            .find(|p| p.site == me)
            .unwrap()
            .state
            .listening_to,
        None
    );
}

#[test]
fn host_bye_ends_the_stream_with_its_reason() {
    let mut w = World::new();
    let me = w.me();
    let stream = w.listen();
    w.host.signal(
        me,
        stream,
        StreamSignal::Bye {
            reason: Some("too many listeners".into()),
        },
    );
    w.settle();
    assert_eq!(
        listen_state(&w.sites[1]),
        Some(ListenState::Ended {
            host: SiteId(HOST),
            reason: "too many listeners".into()
        })
    );
    // No Unlisten for a stream the host ended.
    assert!(
        !w.host
            .routed()
            .iter()
            .any(|m| matches!(m, CollabMessage::Unlisten { .. }))
    );
    // The next Listen clears Ended.
    let again = w.listen();
    assert_ne!(again, 0);
    assert!(matches!(
        listen_state(&w.sites[1]),
        Some(ListenState::Connecting { .. })
    ));
}

#[test]
fn a_receiver_failure_ends_the_stream() {
    let mut w = World::new();
    let stream = w.listening();
    w.listener().ok(Command::Collab(CollabCommand::SendSignal {
        to: SiteId(HOST),
        stream,
        signal: StreamSignal::Bye {
            reason: Some("no audio for 10 s".into()),
        },
    }));
    assert_eq!(
        listen_state(&w.sites[1]),
        Some(ListenState::Ended {
            host: SiteId(HOST),
            reason: "no audio for 10 s".into()
        })
    );
    let t = last_transport(&w.sites[1]).unwrap();
    assert!(!t.playing, "back on the (stopped) local transport");
    w.settle();
    assert!(matches!(
        w.host.routed().last(),
        Some(CollabMessage::Signal {
            signal: StreamSignal::Bye { .. },
            ..
        })
    ));
}

#[test]
fn the_host_leaving_ends_the_stream() {
    let mut w = World::new();
    let me = w.me();
    let stream = w.listening();
    // Playing at 90 bpm (1.5 beats/s) from 8.0; the host leaves 100 ms after the anchor.
    w.sites[1].advance(100);
    let site = w.host.site;
    w.host.send(CollabMessage::Leave { site });
    w.settle();
    assert_eq!(
        listen_state(&w.sites[1]),
        Some(ListenState::Ended {
            host: SiteId(HOST),
            reason: "Diego left".into()
        })
    );
    let located = engine_calls(&w.sites[1])
        .into_iter()
        .rev()
        .find_map(|c| match c {
            TransportControl::Locate { position } => Some(position.0),
            _ => None,
        })
        .unwrap();
    assert!(
        (8.0..8.4).contains(&located),
        "the last heard position, extrapolated a little: {located}"
    );
    assert!(
        !engine_calls(&w.sites[1]).contains(&TransportControl::Play),
        "never auto-plays"
    );
    let _ = (me, stream);
}

#[test]
fn leaving_the_session_unlistens() {
    let mut w = World::new();
    let me = w.me();
    let stream = w.listening();
    w.listener().ok(Command::Collab(CollabCommand::Leave));
    w.settle();
    assert!(w.host.routed().contains(&CollabMessage::Unlisten {
        from: me,
        to: SiteId(HOST),
        stream
    }));
    assert_eq!(listen_state(&w.sites[1]), Some(ListenState::Off));
}

#[test]
fn listen_is_validated() {
    let hub = Hub::default();
    let mut lone = Site::on_hub(5, &hub);
    let out = lone.send(Command::Collab(CollabCommand::Listen { host: SiteId(9) }));
    assert_eq!(error_code(&out), ErrorCode::InvalidState);

    let mut w = World::new();
    let me = w.me();
    let out = w
        .listener()
        .send(Command::Collab(CollabCommand::Listen { host: me }));
    assert_eq!(error_code(&out), ErrorCode::InvalidArgument);
    let out = w
        .listener()
        .send(Command::Collab(CollabCommand::Listen { host: SiteId(77) }));
    assert_eq!(error_code(&out), ErrorCode::NotFound);
    // Listening twice to the same host is a no-op; to another host switches.
    let stream = w.listen();
    w.listener().ok(Command::Collab(CollabCommand::Listen {
        host: SiteId(HOST),
    }));
    w.settle();
    assert_eq!(
        w.host
            .routed()
            .iter()
            .filter(|m| matches!(m, CollabMessage::Listen { .. }))
            .count(),
        1
    );
    let other = w.sites[0].ctl.collab_site();
    w.listener()
        .ok(Command::Collab(CollabCommand::Listen { host: other }));
    w.settle();
    assert!(w.host.routed().contains(&CollabMessage::Unlisten {
        from: me,
        to: SiteId(HOST),
        stream
    }));
    assert!(w.sites[1].log.iter().any(|m| matches!(
        m,
        ServerMessage::Event(Event::Collab {
            event: CollabEvent::ListenStatus { status: ListenStatus { listening: ListenState::Connecting { host, .. }, .. } }
        }) if *host == other
    )));
}

#[test]
fn a_relay_drop_ends_the_stream_at_the_last_heard_position() {
    let mut w = World::new();
    let me = w.me();
    let stream = w.listening();
    w.host.clock(me, stream, anchor(2_000, 6.0, false));
    w.settle();
    // The listener's own link to the relay dies (conns: site 0, the listener, the host).
    let conn = w.hub.links()[1];
    w.hub.kill(conn);
    w.sites[1].tick();
    assert_eq!(
        listen_state(&w.sites[1]),
        Some(ListenState::Ended {
            host: SiteId(HOST),
            reason: "the connection to the relay was lost".into()
        })
    );
    assert_eq!(
        engine_calls(&w.sites[1]).last(),
        Some(&TransportControl::Locate {
            position: Beats(6.0)
        })
    );
    assert!(!last_transport(&w.sites[1]).unwrap().playing);
}

#[test]
fn the_tick_guard_restops_an_engine_that_started_on_its_own() {
    let mut w = World::new();
    w.listening();
    let state = |playing| ether_core::PlayheadState {
        playing,
        recording: false,
        position: Beats(0.0),
        seconds: 0.0,
        bpm: 120.0,
        sample_time: 0,
    };
    w.sites[1].ctl.bridge.playhead = Some(state(false));
    w.sites[1].tick();
    let before = engine_calls(&w.sites[1]).len();
    // The engine reports playing (started from elsewhere): the controller adopts it, then
    // the listener's tick stops it again.
    w.sites[1].ctl.bridge.playhead = Some(state(true));
    w.sites[1].tick();
    w.sites[1].tick();
    let calls = engine_calls(&w.sites[1]);
    assert!(
        calls[before..].contains(&TransportControl::Stop),
        "{:?}",
        &calls[before..]
    );
    assert!(!calls[before..].contains(&TransportControl::Play));
}

#[test]
fn local_non_transport_commands_still_reach_the_engine() {
    let mut w = World::new();
    w.listening();
    // A preview (local audition) is not intercepted: it goes to the bridge, whose fake
    // answers that previews are unsupported on this host.
    let out = w.listener().send(Command::Media(
        ether_core::protocol::media::MediaCommand::StopPreview,
    ));
    match out.last() {
        Some(ServerMessage::Reply(Reply {
            result: ReplyResult::Err { error },
            ..
        })) => assert!(error.message.contains("preview"), "{error:?}"),
        other => panic!("expected the bridge's answer, got {other:?}"),
    }
    // Record-arming (live input monitoring) stays local too.
    let track: TrackId = w.sites[1].id();
    w.listener().ok(Command::Track(
        ether_core::protocol::tracks::TrackCommand::Create {
            id: track,
            kind: TrackKind::Midi,
            name: None,
            color: None,
            parent: None,
            before: None,
        },
    ));
    w.listener().ok(Command::Recording(RecordingCommand::Arm {
        track,
        armed: true,
        exclusive: false,
    }));
    assert!(w.sites[1].ctl.armed().contains(&track));
}

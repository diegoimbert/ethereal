//! "Listen on <peer>", host side (`stream-host`; docs/COLLAB.md §9.2, §9.3, §9.5): a real
//! controller hosting through the in-memory relay, a scripted engine bridge standing in for
//! the native sender, and raw relay links standing in for listeners.

#[path = "common/mod.rs"]
mod common;

use std::sync::Arc;

use common::{Call, FakeBridge, T0};
use ether_collab::memory::{Hub, MemoryLink};
use ether_collab::wire::COLLAB_PROTOCOL_VERSION;
use ether_collab::{CollabTransport, ConnectRequest};
use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::streaming::{StreamCapabilities, StreamLinkState, StreamOutput};
use ether_controller::{
    BridgeError, Controller, ControllerConfig, EngineBridge, EtherController, HostServices,
};
use ether_core::plugin::PluginNotification;
use ether_core::protocol::collab::*;
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::*;
use ether_core::protocol::project::ProjectCommand;
use ether_core::protocol::*;
use ether_core::{EngineOutputs, NodeKey, ParamChange, RenderGraphDesc, TransportControl};
use ether_media::DecodedAudio;

// ─── Scripted bridge ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum StreamCall {
    StartCapture,
    StopCapture,
    Open(SiteId, u32, Vec<IceServer>),
    Signal(SiteId, u32, StreamSignal),
    Close(SiteId, u32),
}

/// [`FakeBridge`] plus the stream hooks: records calls, emits scripted outputs.
#[derive(Default)]
struct StreamBridge {
    inner: FakeBridge,
    native: bool,
    calls: Vec<StreamCall>,
    outputs: Vec<StreamOutput>,
    fail_open: bool,
}

impl EngineBridge for StreamBridge {
    fn create_builtin(
        &mut self,
        device: DeviceId,
        kind: &BuiltinDevice,
        params: &[(ParamId, f64)],
    ) -> Result<NodeKey, BridgeError> {
        self.inner.create_builtin(device, kind, params)
    }
    fn create_plugin(
        &mut self,
        device: DeviceId,
        plugin: &PluginInstance,
        state: Option<&Base64Bytes>,
    ) -> Result<NodeKey, BridgeError> {
        self.inner.create_plugin(device, plugin, state)
    }
    fn destroy_node(&mut self, key: NodeKey) -> Result<(), BridgeError> {
        self.inner.destroy_node(key)
    }
    fn load_media(
        &mut self,
        media: &MediaRef,
        audio: Arc<DecodedAudio>,
    ) -> Result<(), BridgeError> {
        self.inner.load_media(media, audio)
    }
    fn unload_media(&mut self, media: MediaId) -> Result<(), BridgeError> {
        self.inner.unload_media(media)
    }
    fn publish(&mut self, graph: RenderGraphDesc) -> Result<(), BridgeError> {
        self.inner.publish(graph)
    }
    fn set_param(&mut self, change: ParamChange) -> Result<(), BridgeError> {
        self.inner.set_param(change)
    }
    fn transport(&mut self, control: TransportControl) -> Result<(), BridgeError> {
        self.inner.transport(control)
    }
    fn poll(&mut self, out: &mut EngineOutputs) {
        self.inner.poll(out)
    }
    fn descriptor(&mut self, device: DeviceId) -> Option<DeviceDescriptor> {
        self.inner.descriptor(device)
    }
    fn poll_plugins(&mut self, out: &mut Vec<(DeviceId, PluginNotification)>) {
        self.inner.poll_plugins(out)
    }
    fn plugin_state(&mut self, device: DeviceId) -> Result<Option<Base64Bytes>, BridgeError> {
        self.inner.plugin_state(device)
    }

    fn stream_capabilities(&self) -> StreamCapabilities {
        StreamCapabilities {
            native_sender: self.native,
        }
    }
    fn start_stream_capture(&mut self) -> Result<(), BridgeError> {
        self.calls.push(StreamCall::StartCapture);
        Ok(())
    }
    fn stop_stream_capture(&mut self) -> Result<(), BridgeError> {
        self.calls.push(StreamCall::StopCapture);
        Ok(())
    }
    fn stream_open(
        &mut self,
        listener: SiteId,
        stream: u32,
        ice: &[IceServer],
    ) -> Result<(), BridgeError> {
        if self.fail_open {
            return Err(BridgeError::Other("no socket".into()));
        }
        self.calls
            .push(StreamCall::Open(listener, stream, ice.to_vec()));
        Ok(())
    }
    fn stream_signal(
        &mut self,
        listener: SiteId,
        stream: u32,
        signal: &StreamSignal,
    ) -> Result<(), BridgeError> {
        self.calls
            .push(StreamCall::Signal(listener, stream, signal.clone()));
        Ok(())
    }
    fn stream_close(&mut self, listener: SiteId, stream: u32) -> Result<(), BridgeError> {
        self.calls.push(StreamCall::Close(listener, stream));
        Ok(())
    }
    fn poll_stream(&mut self, out: &mut Vec<StreamOutput>) {
        out.append(&mut self.outputs);
    }
}

// ─── Sites ──────────────────────────────────────────────────────────────────────────────

struct Clock {
    now: u64,
    seed: u64,
}

impl HostServices for Clock {
    fn now_ms(&self) -> u64 {
        self.now
    }
    fn random_seed(&mut self) -> u64 {
        self.seed
    }
}

type Ctl = EtherController<StreamBridge, Clock, MemoryStore, MemoryLibrary>;

/// The hosting site: a real controller.
struct Host {
    ctl: Ctl,
    next: u32,
    log: Vec<ServerMessage>,
}

impl Host {
    fn new(hub: &Hub, native: bool) -> Self {
        let mut store = MemoryStore::new();
        store.now_ms = T0;
        let config = ControllerConfig {
            autosave_after_ms: None,
            ..ControllerConfig::default()
        };
        let mut ctl = EtherController::with_config(
            StreamBridge {
                native,
                ..StreamBridge::default()
            },
            Clock {
                now: T0,
                seed: 0x4057,
            },
            store,
            MemoryLibrary::new(),
            config,
        );
        ctl.set_collab_connector(hub.connector());
        let mut h = Self {
            ctl,
            next: 1,
            log: Vec::new(),
        };
        let id = IdGen::new(99).next_project_id(T0);
        h.ok(Command::Project(ProjectCommand::Create {
            id,
            name: "Jam".into(),
        }));
        h.ok(Command::Collab(CollabCommand::Join {
            server: "ws://hub".into(),
            session: "jam".into(),
            token: None,
            name: "Host".into(),
        }));
        h
    }

    fn site(&mut self) -> SiteId {
        self.ctl.collab_site()
    }

    fn send(&mut self, command: Command) -> Vec<ServerMessage> {
        let mut out = Vec::new();
        self.ctl.handle(
            ClientMessage {
                id: self.next,
                gesture: None,
                command,
            },
            &mut out,
        );
        self.next += 1;
        self.log.extend(out.iter().cloned());
        out
    }

    fn ok(&mut self, command: Command) -> ReplyValue {
        common::ok(&self.send(command))
    }

    fn tick(&mut self, ms: u64) {
        self.ctl.host.now += ms;
        self.ctl.store.now_ms = self.ctl.host.now;
        let mut out = Vec::new();
        let now = self.ctl.host.now;
        self.ctl.tick(now, &mut out);
        self.log.extend(out);
    }

    fn hosting(&mut self, allow: bool, ui_sender: bool, remote_transport: bool) {
        self.ok(Command::Collab(CollabCommand::SetHosting {
            allow,
            ui_sender,
            remote_transport,
        }));
    }

    fn stream_calls(&mut self) -> Vec<StreamCall> {
        std::mem::take(&mut self.ctl.bridge.calls)
    }

    fn transport_calls(&self) -> Vec<TransportControl> {
        self.ctl
            .bridge
            .inner
            .calls
            .iter()
            .filter_map(|c| match c {
                Call::Transport(t) => Some(*t),
                _ => None,
            })
            .collect()
    }

    /// Collab events the UI received (and forget them).
    fn take_events(&mut self) -> Vec<CollabEvent> {
        std::mem::take(&mut self.log)
            .into_iter()
            .filter_map(|m| match m {
                ServerMessage::Event(Event::Collab { event }) => Some(event),
                _ => None,
            })
            .collect()
    }
}

/// A listener: a raw relay link speaking the wire protocol (the listener side of the
/// controller is another node's).
struct Peer {
    site: SiteId,
    link: MemoryLink,
    inbox: Vec<CollabMessage>,
}

impl Peer {
    fn join(hub: &Hub, site: u64) -> Self {
        let mut link = hub.link(&ConnectRequest {
            server: "ws://hub".into(),
            session: "jam".into(),
            token: None,
            client: "peer".into(),
        });
        let site = SiteId(site);
        link.send(&CollabMessage::Hello {
            site,
            actor: None,
            name: format!("Peer {}", site.0),
            protocol_version: COLLAB_PROTOCOL_VERSION,
        });
        link.send(&CollabMessage::SyncRequest {
            site,
            version: Base64Bytes(Vec::new()),
        });
        Self {
            site,
            link,
            inbox: Vec::new(),
        }
    }

    fn send(&mut self, m: CollabMessage) {
        self.link.send(&m);
    }

    fn poll(&mut self) {
        self.link.poll(&mut self.inbox);
    }

    /// Messages from `host` addressed to us (signals and clocks), and forget everything.
    fn take_routed(&mut self) -> Vec<CollabMessage> {
        self.poll();
        std::mem::take(&mut self.inbox)
            .into_iter()
            .filter(|m| m.route().is_some_and(|(_, to)| to == self.site))
            .collect()
    }

    fn signals(&mut self) -> Vec<(u32, StreamSignal)> {
        self.take_routed()
            .into_iter()
            .filter_map(|m| match m {
                CollabMessage::Signal { stream, signal, .. } => Some((stream, signal)),
                _ => None,
            })
            .collect()
    }

    /// The host's last presence this peer saw.
    fn host_presence(&mut self, host: SiteId) -> Option<PresenceState> {
        self.poll();
        self.inbox.iter().rev().find_map(|m| match m {
            CollabMessage::Presence { presence } if presence.site == host => {
                Some(presence.state.clone())
            }
            _ => None,
        })
    }

    fn listen(&mut self, host: SiteId, stream: u32) {
        self.send(CollabMessage::Listen {
            from: self.site,
            to: host,
            stream,
        });
    }

    fn request(&mut self, host: SiteId, stream: u32, request: TransportRequest) {
        self.send(CollabMessage::TransportRequest {
            from: self.site,
            to: host,
            stream,
            request,
        });
    }
}

fn settle(hub: &Hub, host: &mut Host, peers: &mut [&mut Peer]) {
    for _ in 0..50 {
        host.tick(20);
        hub.tick(host.ctl.host.now);
        let moved = hub.deliver();
        for p in peers.iter_mut() {
            p.poll();
        }
        if moved == 0 && hub.queued() == 0 {
            host.tick(20);
            hub.deliver();
            return;
        }
    }
}

fn setup(native: bool, peers: usize) -> (Hub, Host, Vec<Peer>) {
    let hub = Hub::default();
    let mut host = Host::new(&hub, native);
    settle(&hub, &mut host, &mut []);
    let mut ps: Vec<Peer> = (0..peers)
        .map(|i| Peer::join(&hub, 0x100 + i as u64))
        .collect();
    {
        let mut refs: Vec<&mut Peer> = ps.iter_mut().collect();
        settle(&hub, &mut host, &mut refs);
    }
    host.take_events();
    host.ctl.bridge.inner.calls.clear();
    for p in &mut ps {
        p.inbox.clear();
    }
    (hub, host, ps)
}

fn bye(reason: &str) -> StreamSignal {
    StreamSignal::Bye {
        reason: Some(reason.into()),
    }
}

fn clock(rtp: u32, position: f64) -> StreamClock {
    StreamClock {
        rtp,
        position: Beats(position),
        playing: true,
        recording: false,
        bpm: 120.0,
        loop_enabled: false,
        loop_region: BeatRange {
            start: Beats(0.0),
            end: Beats(4.0),
        },
        metronome: false,
        discontinuity: false,
        count_in_end: None,
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────────────────

#[test]
fn a_native_host_streams_through_the_engine_bridge() {
    let (hub, mut host, mut peers) = setup(true, 1);
    let me = host.site();
    let p = &mut peers[0];
    let ice = vec![IceServer {
        urls: vec!["stun:relay.test:4000".into()],
        username: None,
        credential: None,
    }];
    host.ok(Command::Collab(CollabCommand::SetIceServers {
        servers: Some(ice.clone()),
    }));
    p.listen(me, 7);
    settle(&hub, &mut host, &mut [p]);
    assert_eq!(
        host.stream_calls(),
        [StreamCall::StartCapture, StreamCall::Open(p.site, 7, ice)]
    );

    // The sender's offer and anchors go to the listener only; its answer comes back.
    host.ctl.bridge.outputs.extend([
        StreamOutput::Signal {
            listener: p.site,
            stream: 7,
            signal: StreamSignal::Offer { sdp: "v=0".into() },
        },
        StreamOutput::State {
            listener: p.site,
            stream: 7,
            state: StreamLinkState::Connected,
        },
        StreamOutput::Clock {
            listener: p.site,
            stream: 7,
            clock: clock(1_000, 2.0),
        },
        // A stale output for a stream that is not ours is dropped.
        StreamOutput::Clock {
            listener: p.site,
            stream: 8,
            clock: clock(1_000, 2.0),
        },
    ]);
    settle(&hub, &mut host, &mut [p]);
    let routed = p.take_routed();
    assert_eq!(
        routed,
        [
            CollabMessage::Signal {
                from: me,
                to: p.site,
                stream: 7,
                signal: StreamSignal::Offer { sdp: "v=0".into() },
            },
            CollabMessage::StreamClock {
                from: me,
                to: p.site,
                stream: 7,
                clock: clock(1_000, 2.0),
            },
        ]
    );
    let answer = StreamSignal::Answer {
        sdp: "v=0 a".into(),
    };
    p.send(CollabMessage::Signal {
        from: p.site,
        to: me,
        stream: 7,
        signal: answer.clone(),
    });
    settle(&hub, &mut host, &mut [p]);
    assert_eq!(host.stream_calls(), [StreamCall::Signal(p.site, 7, answer)]);
    // Signals of a native stream never reach the UI.
    assert!(
        !host
            .take_events()
            .iter()
            .any(|e| matches!(e, CollabEvent::Signal { .. }))
    );

    // Unlisten: closed and capture stopped, no Bye.
    p.send(CollabMessage::Unlisten {
        from: p.site,
        to: me,
        stream: 7,
    });
    settle(&hub, &mut host, &mut [p]);
    assert_eq!(
        host.stream_calls(),
        [StreamCall::Close(p.site, 7), StreamCall::StopCapture]
    );
    assert!(p.signals().is_empty());
}

#[test]
fn can_host_follows_the_policy_and_the_sender() {
    let (hub, mut host, mut peers) = setup(false, 1);
    let me = host.site();
    let p = &mut peers[0];
    settle(&hub, &mut host, &mut [p]);
    // No native sender, no UI sender declared.
    let state = p.host_presence(me);
    assert!(state.is_none_or(|s| !s.can_host));

    host.hosting(true, true, true);
    host.tick(200);
    settle(&hub, &mut host, &mut [p]);
    assert!(p.host_presence(me).expect("presence").can_host);

    host.hosting(false, true, true);
    host.tick(200);
    settle(&hub, &mut host, &mut [p]);
    assert!(!p.host_presence(me).expect("presence").can_host);
}

#[test]
fn listen_is_refused_with_a_reason() {
    // No sender at all.
    let (hub, mut host, mut peers) = setup(false, 1);
    let me = host.site();
    let p = &mut peers[0];
    p.listen(me, 1);
    settle(&hub, &mut host, &mut [p]);
    assert_eq!(p.signals(), [(1, bye("this peer cannot host a stream"))]);

    // Hosting turned off.
    host.hosting(false, true, true);
    p.listen(me, 2);
    settle(&hub, &mut host, &mut [p]);
    assert_eq!(p.signals(), [(2, bye("the host does not allow listening"))]);

    // The native sender cannot open a connection.
    let (hub, mut host, mut peers) = setup(true, 1);
    let me = host.site();
    let p = &mut peers[0];
    host.ctl.bridge.fail_open = true;
    p.listen(me, 3);
    settle(&hub, &mut host, &mut [p]);
    let signals = p.signals();
    assert!(
        matches!(&signals[..], [(3, StreamSignal::Bye { reason: Some(r) })] if r.contains("no socket")),
        "{signals:?}"
    );
    assert_eq!(
        host.stream_calls(),
        [StreamCall::StartCapture, StreamCall::StopCapture],
        "no capture left running"
    );
}

#[test]
fn at_most_eight_listeners() {
    let (hub, mut host, mut peers) = setup(true, 9);
    let me = host.site();
    for (i, p) in peers.iter_mut().enumerate() {
        p.listen(me, 10 + i as u32);
    }
    let mut refs: Vec<&mut Peer> = peers.iter_mut().collect();
    settle(&hub, &mut host, &mut refs);
    let calls = host.stream_calls();
    let opened = calls
        .iter()
        .filter(|c| matches!(c, StreamCall::Open(..)))
        .count();
    assert_eq!(opened, 8);
    assert_eq!(
        calls
            .iter()
            .filter(|c| matches!(c, StreamCall::StartCapture))
            .count(),
        1,
        "one capture for all listeners"
    );
    let refused: Vec<_> = peers.iter_mut().flat_map(|p| p.signals()).collect();
    assert_eq!(refused, [(18, bye("the host already has 8 listeners"))]);

    // A listener leaving frees a slot; the last one stops the capture.
    let gone = peers.remove(0);
    let gone_site = gone.site;
    drop(gone);
    let mut refs: Vec<&mut Peer> = peers.iter_mut().collect();
    settle(&hub, &mut host, &mut refs);
    assert_eq!(host.stream_calls(), [StreamCall::Close(gone_site, 10)]);
    let last = peers.len() - 1;
    peers[last].listen(me, 30);
    let mut refs: Vec<&mut Peer> = peers.iter_mut().collect();
    settle(&hub, &mut host, &mut refs);
    assert!(
        host.stream_calls()
            .iter()
            .any(|c| *c == StreamCall::Open(peers[last].site, 30, vec![]))
    );
}

#[test]
fn streams_end_on_failure_leave_and_policy() {
    let (hub, mut host, mut peers) = setup(true, 2);
    let me = host.site();
    let (a, b) = peers.split_at_mut(1);
    let (a, b) = (&mut a[0], &mut b[0]);
    a.listen(me, 1);
    b.listen(me, 2);
    settle(&hub, &mut host, &mut [a, b]);
    host.stream_calls();

    // The sender reports a failed link: the listener gets a Bye with the reason.
    host.ctl.bridge.outputs.push(StreamOutput::State {
        listener: a.site,
        stream: 1,
        state: StreamLinkState::Failed {
            reason: "ICE failed".into(),
        },
    });
    settle(&hub, &mut host, &mut [a, b]);
    assert_eq!(a.signals(), [(1, bye("ICE failed"))]);
    assert_eq!(host.stream_calls(), [StreamCall::Close(a.site, 1)]);

    // A listener's Bye ends its stream without a Bye back.
    b.send(CollabMessage::Signal {
        from: b.site,
        to: me,
        stream: 2,
        signal: StreamSignal::Bye { reason: None },
    });
    settle(&hub, &mut host, &mut [a, b]);
    assert_eq!(
        host.stream_calls(),
        [StreamCall::Close(b.site, 2), StreamCall::StopCapture]
    );
    assert!(b.signals().is_empty());

    // Hosting turned off: every listener gets a Bye.
    a.listen(me, 3);
    b.listen(me, 4);
    settle(&hub, &mut host, &mut [a, b]);
    host.hosting(false, false, true);
    settle(&hub, &mut host, &mut [a, b]);
    assert_eq!(a.signals(), [(3, bye("hosting was turned off"))]);
    assert_eq!(b.signals(), [(4, bye("hosting was turned off"))]);

    // The host leaves: Byes, streams closed, capture stopped.
    host.hosting(true, false, true);
    a.listen(me, 5);
    settle(&hub, &mut host, &mut [a, b]);
    host.stream_calls();
    host.ok(Command::Collab(CollabCommand::Leave));
    settle(&hub, &mut host, &mut [a, b]);
    assert_eq!(a.signals(), [(5, bye("the host left the session"))]);
    assert_eq!(
        host.stream_calls(),
        [StreamCall::Close(a.site, 5), StreamCall::StopCapture]
    );
}

#[test]
fn a_new_listen_replaces_the_listeners_old_stream_and_stale_signals_are_dropped() {
    let (hub, mut host, mut peers) = setup(true, 1);
    let me = host.site();
    let p = &mut peers[0];
    p.listen(me, 1);
    settle(&hub, &mut host, &mut [p]);
    p.listen(me, 2);
    settle(&hub, &mut host, &mut [p]);
    let calls = host.stream_calls();
    assert!(calls.contains(&StreamCall::Close(p.site, 1)), "{calls:?}");
    assert!(calls.contains(&StreamCall::Open(p.site, 2, vec![])));
    // An answer for the old stream goes nowhere near the sender.
    p.send(CollabMessage::Signal {
        from: p.site,
        to: me,
        stream: 1,
        signal: StreamSignal::Answer { sdp: "old".into() },
    });
    settle(&hub, &mut host, &mut [p]);
    assert!(host.stream_calls().is_empty());
}

#[test]
fn a_web_host_routes_signals_and_clocks_through_the_ui() {
    let (hub, mut host, mut peers) = setup(false, 1);
    let me = host.site();
    let p = &mut peers[0];
    host.hosting(true, true, true);
    p.listen(me, 9);
    settle(&hub, &mut host, &mut [p]);
    assert!(host.stream_calls().is_empty(), "no native sender");
    host.take_events();

    // The UI's offer goes out (SendSignal is forwarded by the collab module).
    host.ok(Command::Collab(CollabCommand::SendSignal {
        to: p.site,
        stream: 9,
        signal: StreamSignal::Offer { sdp: "web".into() },
    }));
    // The listener's answer and ICE reach the UI.
    let answer = StreamSignal::Answer { sdp: "ans".into() };
    p.send(CollabMessage::Signal {
        from: p.site,
        to: me,
        stream: 9,
        signal: answer.clone(),
    });
    settle(&hub, &mut host, &mut [p]);
    assert_eq!(
        p.signals(),
        [(9, StreamSignal::Offer { sdp: "web".into() })]
    );
    assert!(host.take_events().contains(&CollabEvent::Signal {
        from: p.site,
        stream: 9,
        signal: answer,
    }));

    // The UI's anchors go to that listener; stale ones are dropped.
    host.ok(Command::Collab(CollabCommand::SendStreamClock {
        to: p.site,
        stream: 9,
        clock: clock(5, 1.0),
    }));
    host.ok(Command::Collab(CollabCommand::SendStreamClock {
        to: p.site,
        stream: 99,
        clock: clock(6, 1.0),
    }));
    settle(&hub, &mut host, &mut [p]);
    assert_eq!(
        p.take_routed(),
        [CollabMessage::StreamClock {
            from: me,
            to: p.site,
            stream: 9,
            clock: clock(5, 1.0),
        }]
    );

    // The UI's Bye (its peer connection failed) ends the stream.
    host.ok(Command::Collab(CollabCommand::SendSignal {
        to: p.site,
        stream: 9,
        signal: bye("connection failed"),
    }));
    host.ok(Command::Collab(CollabCommand::SendStreamClock {
        to: p.site,
        stream: 9,
        clock: clock(7, 1.0),
    }));
    settle(&hub, &mut host, &mut [p]);
    assert_eq!(
        p.signals(),
        [(9, bye("connection failed"))],
        "no clock after the end"
    );

    // The UI sender going away ends its streams.
    p.listen(me, 10);
    settle(&hub, &mut host, &mut [p]);
    host.hosting(true, false, true);
    settle(&hub, &mut host, &mut [p]);
    assert_eq!(p.signals(), [(10, bye("the host stopped streaming"))]);
}

#[test]
fn hosting_commands_need_a_session() {
    let hub = Hub::default();
    let mut host = Host::new(&hub, true);
    host.ok(Command::Collab(CollabCommand::Leave));
    // Outside a session the policy is accepted (and forgotten).
    host.hosting(true, true, true);
    let out = host.send(Command::Collab(CollabCommand::SendStreamClock {
        to: SiteId(5),
        stream: 1,
        clock: clock(0, 0.0),
    }));
    assert!(matches!(
        out.last(),
        Some(ServerMessage::Reply(Reply {
            result: ReplyResult::Err { error },
            ..
        })) if error.code == ErrorCode::InvalidState
    ));
}

#[test]
fn listeners_transport_requests_apply_on_the_host() {
    let (hub, mut host, mut peers) = setup(true, 2);
    let me = host.site();
    let (a, b) = peers.split_at_mut(1);
    let (a, b) = (&mut a[0], &mut b[0]);
    a.listen(me, 1);
    settle(&hub, &mut host, &mut [a, b]);

    a.request(me, 1, TransportRequest::Play);
    a.request(
        me,
        1,
        TransportRequest::Locate {
            position: Beats(8.0),
        },
    );
    settle(&hub, &mut host, &mut [a, b]);
    assert_eq!(
        host.transport_calls(),
        [
            TransportControl::Play,
            TransportControl::Locate {
                position: Beats(8.0)
            }
        ]
    );

    // Loop changes: site-local settings, outside the undo history.
    let region = BeatRange {
        start: Beats(4.0),
        end: Beats(12.0),
    };
    a.request(me, 1, TransportRequest::SetLoopRegion { region });
    a.request(me, 1, TransportRequest::SetLoopEnabled { enabled: true });
    settle(&hub, &mut host, &mut [a, b]);
    let settings = &host.ctl.project().unwrap().settings;
    assert!(settings.loop_enabled);
    assert_eq!(settings.loop_region, region);
    let out = host.send(Command::Edit(
        ether_core::protocol::project::EditCommand::Undo,
    ));
    assert!(
        matches!(
            out.last(),
            Some(ServerMessage::Reply(Reply {
                result: ReplyResult::Err { .. },
                ..
            }))
        ),
        "nothing to undo"
    );

    // Not a listener (or a stale stream): ignored.
    let before = host.transport_calls().len();
    b.request(me, 1, TransportRequest::Stop);
    a.request(me, 2, TransportRequest::Stop);
    settle(&hub, &mut host, &mut [a, b]);
    assert_eq!(host.transport_calls().len(), before);

    // Remote transport off: ignored.
    host.hosting(true, false, false);
    a.request(me, 1, TransportRequest::Stop);
    settle(&hub, &mut host, &mut [a, b]);
    assert_eq!(host.transport_calls().len(), before);
    host.hosting(true, false, true);
    a.request(me, 1, TransportRequest::Stop);
    settle(&hub, &mut host, &mut [a, b]);
    assert_eq!(host.transport_calls().last(), Some(&TransportControl::Stop));
}

#[test]
fn transport_requests_are_throttled_per_listener() {
    let (hub, mut host, mut peers) = setup(true, 1);
    let me = host.site();
    let p = &mut peers[0];
    p.listen(me, 1);
    settle(&hub, &mut host, &mut [p]);

    // A burst of 60 locates and toggles arrives at once.
    for i in 0..60 {
        if i % 3 == 0 {
            p.request(me, 1, TransportRequest::TogglePlay);
        } else {
            p.request(
                me,
                1,
                TransportRequest::Locate {
                    position: Beats(i as f64),
                },
            );
        }
    }
    // Deliver everything within one host tick.
    hub.deliver();
    host.tick(1);
    let applied = host.transport_calls().len();
    assert!(applied <= 20, "{applied} applied at once");
    // Once the budget refills, only the latest Locate is still applied.
    for _ in 0..10 {
        host.tick(100);
    }
    let calls = host.transport_calls();
    assert_eq!(calls.len(), applied + 1, "{calls:?}");
    assert_eq!(
        calls.last(),
        Some(&TransportControl::Locate {
            position: Beats(59.0)
        })
    );
}

#[test]
fn a_hosts_relay_reconnect_ends_its_streams() {
    let (hub, mut host, mut peers) = setup(true, 1);
    let me = host.site();
    let p = &mut peers[0];
    p.listen(me, 1);
    settle(&hub, &mut host, &mut [p]);
    host.stream_calls();
    // The host's link drops (not fatal: it reconnects), so the streams do not survive it.
    let conn = hub.links().into_iter().min().expect("the host's link");
    hub.kill(conn);
    host.tick(20);
    assert_eq!(
        host.stream_calls(),
        [StreamCall::Close(p.site, 1), StreamCall::StopCapture]
    );
    // An old listener's signal after the reconnect goes nowhere near the sender.
    settle(&hub, &mut host, &mut [p]);
    p.send(CollabMessage::Signal {
        from: p.site,
        to: me,
        stream: 1,
        signal: StreamSignal::Answer { sdp: "late".into() },
    });
    settle(&hub, &mut host, &mut [p]);
    assert!(host.stream_calls().is_empty());
}

#[test]
fn transport_requests_are_ignored_while_the_host_records_or_counts_in() {
    use ether_core::protocol::recording::RecordingCommand;
    use ether_core::protocol::transport::TransportCommand;
    for count_in in [0, 1] {
        let (hub, mut host, mut peers) = setup(true, 1);
        let me = host.site();
        let p = &mut peers[0];
        p.listen(me, 1);
        settle(&hub, &mut host, &mut [p]);
        host.ok(Command::Recording(RecordingCommand::SetCountIn {
            bars: count_in,
        }));
        host.ok(Command::Recording(RecordingCommand::SetRecording {
            enabled: true,
        }));
        let before = host.transport_calls().len();
        p.request(me, 1, TransportRequest::Stop);
        p.request(
            me,
            1,
            TransportRequest::Locate {
                position: Beats(3.0),
            },
        );
        p.request(me, 1, TransportRequest::SetLoopEnabled { enabled: true });
        settle(&hub, &mut host, &mut [p]);
        assert_eq!(
            host.transport_calls().len(),
            before,
            "count-in {count_in}: only the host stops its own recording"
        );
        assert!(!host.ctl.project().unwrap().settings.loop_enabled);
        // Once the host stopped recording, requests apply again.
        host.ok(Command::Recording(RecordingCommand::SetRecording {
            enabled: false,
        }));
        host.ok(Command::Transport(TransportCommand::Stop));
        let before = host.transport_calls().len();
        p.request(
            me,
            1,
            TransportRequest::Locate {
                position: Beats(3.0),
            },
        );
        settle(&hub, &mut host, &mut [p]);
        assert_eq!(
            host.transport_calls()[before..],
            [TransportControl::Locate {
                position: Beats(3.0)
            }]
        );
    }
}

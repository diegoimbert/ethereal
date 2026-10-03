//! In-memory sharing services for tests and simulations (docs/SHARING.md §12): a fake
//! signaling service ([`FakeNet::signal_connector`], the room state machine of §3.3 in
//! brief) and fake WebRTC endpoints ([`FakeNet::endpoint`]) whose data channels are
//! in-process pipes ([`pipe`]) carrying [`dc`](super::dc) fragments, like the real ones.
//!
//! Controls: the service going down ([`FakeNet::set_down`]), no ICE route
//! ([`FakeNet::set_unreachable`]), dropped connections ([`FakeNet::kill_links`]), and a
//! tampering service that swaps the host's fingerprint ([`FakeNet::set_tamper`]).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use ether_protocol::collab::{IceServer, StreamSignal};
use ether_protocol::share::{
    PeerId, SIGNAL_PROTOCOL_VERSION, SignalClientMessage, SignalRefusal, SignalServerMessage,
};

use super::dc::{Reassembler, fragment};
use super::keys::door_hash;
use super::{
    BoxPeerEndpoint, BoxPeerLink, BoxSignalLink, PeerEndpoint, PeerLink, PeerOutput,
    SignalConnector, SignalLink,
};
use crate::LinkState;
use crate::wire::WireFrame;

// ─── Data channel pipes ─────────────────────────────────────────────────────────────────

struct PipeInner {
    /// Fragments sent by side 0 and side 1, not yet read by the other side.
    q: [VecDeque<Vec<u8>>; 2],
    bytes: [usize; 2],
    closed: Option<String>,
}

/// One end of a [`pipe`].
pub struct PipeEnd {
    inner: Arc<Mutex<PipeInner>>,
    side: usize,
    re: Reassembler,
}

/// A connected, ordered and reliable channel: two [`PeerLink`] ends. Frames are cut into
/// data channel fragments and reassembled, like a real data channel. `buffered()` is what
/// the other side has not read yet.
pub fn pipe() -> (BoxPeerLink, BoxPeerLink) {
    let (a, b, _) = pipe_with_handle();
    (a, b)
}

fn pipe_with_handle() -> (BoxPeerLink, BoxPeerLink, Arc<Mutex<PipeInner>>) {
    let inner = Arc::new(Mutex::new(PipeInner {
        q: [VecDeque::new(), VecDeque::new()],
        bytes: [0, 0],
        closed: None,
    }));
    let end = |side| PipeEnd {
        inner: inner.clone(),
        side,
        re: Reassembler::default(),
    };
    (Box::new(end(0)), Box::new(end(1)), inner)
}

impl PeerLink for PipeEnd {
    fn send(&mut self, frame: &WireFrame) {
        let mut p = self.inner.lock().expect("pipe");
        if p.closed.is_some() {
            return;
        }
        for f in fragment(frame) {
            p.bytes[self.side] += f.len();
            p.q[self.side].push_back(f);
        }
    }

    fn poll(&mut self, out: &mut Vec<WireFrame>) {
        let mut p = self.inner.lock().expect("pipe");
        let other = 1 - self.side;
        while let Some(f) = p.q[other].pop_front() {
            p.bytes[other] -= f.len();
            match self.re.push(&f) {
                Ok(Some(frame)) => out.push(frame),
                Ok(None) => {}
                Err(e) => {
                    p.closed = Some(e);
                    return;
                }
            }
        }
    }

    fn buffered(&self) -> usize {
        self.inner.lock().expect("pipe").bytes[self.side]
    }

    fn state(&self) -> LinkState {
        match &self.inner.lock().expect("pipe").closed {
            None => LinkState::Open,
            Some(reason) => LinkState::Closed {
                reason: reason.clone(),
                fatal: false,
            },
        }
    }

    fn close(&mut self) {
        let mut p = self.inner.lock().expect("pipe");
        p.closed.get_or_insert_with(|| "closed".into());
    }
}

// ─── The fake network ───────────────────────────────────────────────────────────────────

struct Room {
    token: String,
    doors: HashSet<String>,
    host: Option<u64>,
    peers: HashMap<PeerId, u64>,
    /// Joiner sockets with a valid door, waiting for the host (with their door).
    waiting: Vec<(u64, String)>,
    last_seen: Option<f64>,
}

struct Sock {
    room: String,
    host: bool,
    inbox: VecDeque<SignalServerMessage>,
    closed: Option<(String, bool)>,
    peer: Option<PeerId>,
}

struct Offer {
    endpoint: u64,
    peer: PeerId,
    fingerprint: String,
}

struct Answered {
    endpoint: u64,
    peer: PeerId,
    link: BoxPeerLink,
    host_fingerprint: String,
}

#[derive(Default)]
struct Net {
    rooms: HashMap<String, Room>,
    socks: HashMap<u64, Sock>,
    next: u64,
    next_peer: PeerId,
    down: bool,
    unreachable: bool,
    tamper: bool,
    clock: f64,
    ice: Vec<IceServer>,
    offers: HashMap<u64, Offer>,
    answered: HashMap<u64, Answered>,
    outputs: HashMap<u64, Vec<PeerOutput>>,
    /// Connected pipes per (endpoint, peer).
    pipes: Vec<(u64, PeerId, Arc<Mutex<PipeInner>>)>,
    /// Signals seen by the service (for assertions).
    signals: usize,
}

/// See the module docs. Cheap to clone (shared state).
#[derive(Clone, Default)]
pub struct FakeNet {
    inner: Arc<Mutex<Net>>,
}

impl FakeNet {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Net> {
        self.inner.lock().expect("fake net")
    }

    /// Opens sockets on the fake service (`<any>/v1/rooms/<room>/{host,join}`).
    pub fn signal_connector(&self) -> SignalConnector {
        let net = self.clone();
        Box::new(move |url: &str| -> BoxSignalLink { Box::new(net.open_socket(url)) })
    }

    /// A fake WebRTC endpoint with this DTLS fingerprint.
    pub fn endpoint(&self, fingerprint: &str) -> BoxPeerEndpoint {
        let mut n = self.lock();
        n.next += 1;
        let id = n.next;
        Box::new(FakeEndpoint {
            net: self.clone(),
            id,
            fingerprint: fingerprint.to_string(),
        })
    }

    /// The service is unreachable: open sockets close, new ones fail.
    pub fn set_down(&self, down: bool) {
        let mut n = self.lock();
        n.down = down;
        if down {
            let ids: Vec<u64> = n.socks.keys().copied().collect();
            for id in ids {
                n.server_close(id, "network down", false);
            }
        }
    }

    /// ICE finds no route (both peers get `PeerOutput::Failed`).
    pub fn set_unreachable(&self, unreachable: bool) {
        self.lock().unreachable = unreachable;
    }

    /// The service swaps the host's fingerprint in what the joiner sees (a man in the middle).
    pub fn set_tamper(&self, tamper: bool) {
        self.lock().tamper = tamper;
    }

    /// ICE servers advertised in the welcomes.
    pub fn set_ice_servers(&self, ice: Vec<IceServer>) {
        self.lock().ice = ice;
    }

    /// Every data channel drops ("connection lost").
    pub fn kill_links(&self) {
        let mut n = self.lock();
        for (_, _, p) in n.pipes.drain(..) {
            p.lock().expect("pipe").closed = Some("connection lost".into());
        }
    }

    /// Advance the service clock (`last_seen_ms`).
    pub fn set_clock(&self, ms: f64) {
        self.lock().clock = ms;
    }

    pub fn room_exists(&self, room: &str) -> bool {
        self.lock().rooms.contains_key(room)
    }

    pub fn doors(&self, room: &str) -> usize {
        self.lock().rooms.get(room).map_or(0, |r| r.doors.len())
    }

    pub fn host_online(&self, room: &str) -> bool {
        self.lock()
            .rooms
            .get(room)
            .is_some_and(|r| r.host.is_some())
    }

    /// Open signaling sockets.
    pub fn sockets(&self) -> usize {
        self.lock()
            .socks
            .values()
            .filter(|s| s.closed.is_none())
            .count()
    }

    /// Data channels open.
    pub fn links(&self) -> usize {
        self.lock()
            .pipes
            .iter()
            .filter(|(_, _, p)| p.lock().expect("pipe").closed.is_none())
            .count()
            / 2
    }

    pub fn signals(&self) -> usize {
        self.lock().signals
    }

    fn open_socket(&self, url: &str) -> FakeSocket {
        let mut n = self.lock();
        n.next += 1;
        let id = n.next;
        let parsed = url.split_once("/v1/rooms/").and_then(|(_, rest)| {
            let (room, kind) = rest.split_once('/')?;
            Some((room.to_string(), kind == "host"))
        });
        let (room, host) = parsed.clone().unwrap_or_default();
        let closed = if n.down {
            Some(("network down".to_string(), false))
        } else if parsed.is_none() {
            Some(("bad signaling URL".to_string(), true))
        } else {
            None
        };
        n.socks.insert(
            id,
            Sock {
                room,
                host,
                inbox: VecDeque::new(),
                closed,
                peer: None,
            },
        );
        FakeSocket {
            net: self.clone(),
            id,
        }
    }
}

impl Net {
    fn push(&mut self, sock: u64, m: SignalServerMessage) {
        if let Some(s) = self.socks.get_mut(&sock)
            && s.closed.is_none()
        {
            s.inbox.push_back(m);
        }
    }

    fn refuse(&mut self, sock: u64, reason: SignalRefusal, message: &str) {
        self.push(
            sock,
            SignalServerMessage::Refused {
                reason,
                message: message.into(),
            },
        );
        self.server_close(sock, message, true);
    }

    /// The service closes `sock` (its queued messages stay readable).
    fn server_close(&mut self, sock: u64, reason: &str, fatal: bool) {
        let Some(s) = self.socks.get_mut(&sock) else {
            return;
        };
        if s.closed.is_some() {
            return;
        }
        s.closed = Some((reason.into(), fatal));
        self.forget(sock);
    }

    /// Room bookkeeping when a socket goes away.
    fn forget(&mut self, sock: u64) {
        let (room, host, peer) = match self.socks.get(&sock) {
            Some(s) => (s.room.clone(), s.host, s.peer),
            None => return,
        };
        let clock = self.clock;
        let Some(r) = self.rooms.get_mut(&room) else {
            return;
        };
        if host {
            if r.host == Some(sock) {
                r.host = None;
                r.last_seen = Some(clock);
            }
            return;
        }
        r.waiting.retain(|(s, _)| *s != sock);
        if let Some(p) = peer
            && r.peers.remove(&p).is_some()
            && let Some(h) = r.host
        {
            self.push(h, SignalServerMessage::PeerLeft { peer: p });
        }
    }

    fn pair(&mut self, room: &str, sock: u64) {
        self.next_peer += 1;
        let peer = self.next_peer;
        let ice = self.ice.clone();
        let Some(r) = self.rooms.get_mut(room) else {
            return;
        };
        let Some(host) = r.host else { return };
        r.peers.insert(peer, sock);
        if let Some(s) = self.socks.get_mut(&sock) {
            s.peer = Some(peer);
        }
        self.push(
            sock,
            SignalServerMessage::JoinWelcome {
                peer,
                ice_servers: ice,
            },
        );
        self.push(host, SignalServerMessage::PeerArrived { peer });
    }

    fn client(&mut self, sock: u64, m: SignalClientMessage) {
        let Some(s) = self.socks.get(&sock) else {
            return;
        };
        if s.closed.is_some() {
            return;
        }
        let (room, is_host, own_peer) = (s.room.clone(), s.host, s.peer);
        match m {
            SignalClientMessage::HostHello {
                protocol,
                host_token,
                doors,
                ..
            } => {
                if !is_host {
                    return self.refuse(sock, SignalRefusal::Malformed, "not a host socket");
                }
                if protocol != SIGNAL_PROTOCOL_VERSION {
                    return self.refuse(sock, SignalRefusal::Version, "version");
                }
                if let Some(r) = self.rooms.get(&room)
                    && r.token != host_token
                {
                    return self.refuse(sock, SignalRefusal::NotHost, "not the host");
                }
                let r = self.rooms.entry(room.clone()).or_insert_with(|| Room {
                    token: host_token,
                    doors: HashSet::new(),
                    host: None,
                    peers: HashMap::new(),
                    waiting: Vec::new(),
                    last_seen: None,
                });
                r.doors = doors.into_iter().collect();
                let old = r.host.replace(sock);
                let waiting = std::mem::take(&mut r.waiting);
                if let Some(old) = old.filter(|o| *o != sock) {
                    self.server_close(old, "replaced by a new host connection", true);
                }
                let ice = self.ice.clone();
                self.push(
                    sock,
                    SignalServerMessage::HostWelcome {
                        ice_servers: ice,
                        room_ttl_s: 30 * 86_400,
                    },
                );
                for (s, door) in waiting {
                    let valid = self
                        .rooms
                        .get(&room)
                        .is_some_and(|r| r.doors.contains(&door_hash(&door)));
                    if valid {
                        self.pair(&room, s);
                    } else {
                        self.refuse(s, SignalRefusal::InvalidInvite, "invalid invite");
                    }
                }
            }
            SignalClientMessage::SetDoors { doors } => {
                if let Some(r) = self.rooms.get_mut(&room)
                    && r.host == Some(sock)
                {
                    r.doors = doors.into_iter().collect();
                }
            }
            SignalClientMessage::CloseRoom => {
                let Some(r) = self.rooms.get(&room) else {
                    return;
                };
                if r.host != Some(sock) {
                    return;
                }
                let r = self.rooms.remove(&room).expect("checked");
                let mut socks: Vec<u64> = r.peers.values().copied().collect();
                socks.extend(r.waiting.iter().map(|(s, _)| *s));
                socks.push(sock);
                for s in socks {
                    self.server_close(s, "the room was closed", true);
                }
            }
            SignalClientMessage::JoinHello { protocol, door, .. } => {
                if is_host {
                    return self.refuse(sock, SignalRefusal::Malformed, "not a join socket");
                }
                if protocol != SIGNAL_PROTOCOL_VERSION {
                    return self.refuse(sock, SignalRefusal::Version, "version");
                }
                let Some(r) = self.rooms.get_mut(&room) else {
                    return self.refuse(sock, SignalRefusal::InvalidInvite, "invalid invite");
                };
                if !r.doors.contains(&door_hash(&door)) {
                    return self.refuse(sock, SignalRefusal::InvalidInvite, "invalid invite");
                }
                if r.host.is_some() {
                    self.pair(&room, sock);
                } else {
                    r.waiting.push((sock, door));
                    let last_seen_ms = r.last_seen;
                    self.push(sock, SignalServerMessage::HostOffline { last_seen_ms });
                }
            }
            SignalClientMessage::Signal { peer, signal } => {
                self.signals += 1;
                let Some(r) = self.rooms.get(&room) else {
                    return;
                };
                if is_host {
                    if let Some(&j) = r.peers.get(&peer) {
                        self.push(j, SignalServerMessage::Signal { peer, signal });
                    }
                } else if let (Some(p), Some(h)) = (own_peer, r.host) {
                    self.push(h, SignalServerMessage::Signal { peer: p, signal });
                }
            }
            SignalClientMessage::EndPeer { peer, reason } => {
                let j = self
                    .rooms
                    .get_mut(&room)
                    .filter(|r| r.host == Some(sock))
                    .and_then(|r| r.peers.remove(&peer));
                if let Some(j) = j {
                    self.server_close(j, reason.as_deref().unwrap_or("ended by the host"), false);
                }
            }
            SignalClientMessage::Ping => self.push(sock, SignalServerMessage::Pong),
        }
    }
}

/// A socket on the fake service.
pub struct FakeSocket {
    net: FakeNet,
    id: u64,
}

impl SignalLink for FakeSocket {
    fn send(&mut self, message: &SignalClientMessage) {
        self.net.lock().client(self.id, message.clone());
    }

    fn poll(&mut self, out: &mut Vec<SignalServerMessage>) {
        if let Some(s) = self.net.lock().socks.get_mut(&self.id) {
            out.extend(s.inbox.drain(..));
        }
    }

    fn state(&self) -> LinkState {
        let n = self.net.lock();
        match n.socks.get(&self.id).and_then(|s| s.closed.clone()) {
            None => LinkState::Open,
            Some((reason, fatal)) => LinkState::Closed { reason, fatal },
        }
    }

    fn close(&mut self) {
        let mut n = self.net.lock();
        n.forget(self.id);
        if let Some(s) = n.socks.get_mut(&self.id) {
            s.closed.get_or_insert(("closed".into(), false));
        }
    }
}

impl Drop for FakeSocket {
    fn drop(&mut self) {
        self.close();
        self.net.lock().socks.remove(&self.id);
    }
}

/// A fake WebRTC endpoint: an offer names the offerer; the answerer creates the pipe.
pub struct FakeEndpoint {
    net: FakeNet,
    id: u64,
    fingerprint: String,
}

fn token_of(sdp: &str, prefix: &str) -> Option<u64> {
    sdp.strip_prefix(prefix)?.trim().parse().ok()
}

impl PeerEndpoint for FakeEndpoint {
    fn open(&mut self, peer: PeerId, offer: bool, _ice: &[IceServer]) {
        if !offer {
            return;
        }
        let mut n = self.net.lock();
        n.next += 1;
        let token = n.next;
        n.offers.insert(
            token,
            Offer {
                endpoint: self.id,
                peer,
                fingerprint: self.fingerprint.clone(),
            },
        );
        n.outputs
            .entry(self.id)
            .or_default()
            .push(PeerOutput::Signal {
                peer,
                signal: StreamSignal::Offer {
                    sdp: format!("fake-offer {token}"),
                },
            });
    }

    fn signal(&mut self, peer: PeerId, signal: StreamSignal) {
        let mut n = self.net.lock();
        match signal {
            StreamSignal::Offer { sdp } => {
                let Some(offer) =
                    token_of(&sdp, "fake-offer ").and_then(|t| n.offers.remove(&t).map(|o| (t, o)))
                else {
                    return;
                };
                let (token, offer) = offer;
                if n.unreachable {
                    let reason = "ICE failed: no route to the peer".to_string();
                    n.outputs
                        .entry(self.id)
                        .or_default()
                        .push(PeerOutput::Failed {
                            peer,
                            reason: reason.clone(),
                        });
                    n.outputs
                        .entry(offer.endpoint)
                        .or_default()
                        .push(PeerOutput::Failed {
                            peer: offer.peer,
                            reason,
                        });
                    return;
                }
                let (joiner, host, handle) = pipe_with_handle();
                n.pipes.push((self.id, peer, handle.clone()));
                n.pipes.push((offer.endpoint, offer.peer, handle));
                let host_fp = if n.tamper {
                    "sha-256 EV:IL".to_string()
                } else {
                    self.fingerprint.clone()
                };
                n.answered.insert(
                    token,
                    Answered {
                        endpoint: offer.endpoint,
                        peer: offer.peer,
                        link: joiner,
                        host_fingerprint: host_fp,
                    },
                );
                let out = n.outputs.entry(self.id).or_default();
                out.push(PeerOutput::Signal {
                    peer,
                    signal: StreamSignal::Answer {
                        sdp: format!("fake-answer {token}"),
                    },
                });
                out.push(PeerOutput::Connected {
                    peer,
                    link: host,
                    local_fingerprint: self.fingerprint.clone(),
                    remote_fingerprint: offer.fingerprint,
                });
            }
            StreamSignal::Answer { sdp } => {
                let Some(a) = token_of(&sdp, "fake-answer ").and_then(|t| n.answered.remove(&t))
                else {
                    return;
                };
                if a.endpoint != self.id {
                    return;
                }
                n.outputs
                    .entry(self.id)
                    .or_default()
                    .push(PeerOutput::Connected {
                        peer: a.peer,
                        link: a.link,
                        local_fingerprint: self.fingerprint.clone(),
                        remote_fingerprint: a.host_fingerprint,
                    });
            }
            StreamSignal::Ice { .. } | StreamSignal::Bye { .. } => {}
        }
    }

    fn poll(&mut self, out: &mut Vec<PeerOutput>) {
        if let Some(o) = self.net.lock().outputs.get_mut(&self.id) {
            out.append(o);
        }
    }

    fn close(&mut self, peer: PeerId) {
        let mut n = self.net.lock();
        let id = self.id;
        n.pipes.retain(|(e, p, pipe)| {
            if *e == id && *p == peer {
                pipe.lock()
                    .expect("pipe")
                    .closed
                    .get_or_insert_with(|| "closed".into());
                false
            } else {
                true
            }
        });
        n.offers
            .retain(|_, o| !(o.endpoint == id && o.peer == peer));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::share::invite::LinkKey;
    use crate::share::keys;

    #[test]
    fn pipes_carry_fragmented_frames_in_order() {
        let (mut a, mut b) = pipe();
        let big = WireFrame::Binary(vec![7; 100_000]);
        a.send(&WireFrame::Text("one".into()));
        a.send(&big);
        assert!(a.buffered() > 100_000);
        let mut got = Vec::new();
        b.poll(&mut got);
        assert_eq!(got, [WireFrame::Text("one".into()), big]);
        assert_eq!(a.buffered(), 0);
        b.close();
        assert!(matches!(a.state(), LinkState::Closed { .. }));
    }

    fn key() -> LinkKey {
        keys::new_key().unwrap()
    }

    #[test]
    fn introduction_and_data_channel() {
        let net = FakeNet::new();
        let k = key();
        let door = keys::door(&k, "room");
        let mut connect = net.signal_connector();
        let mut host = connect("wss://x/signal/v1/rooms/room/host");
        host.send(&SignalClientMessage::HostHello {
            protocol: 1,
            host_token: "t".into(),
            doors: vec![keys::door_hash(&door)],
            app: "test".into(),
        });
        let mut joiner = connect("wss://x/signal/v1/rooms/room/join");
        joiner.send(&SignalClientMessage::JoinHello {
            protocol: 1,
            door,
            app: "test".into(),
        });
        let (mut hm, mut jm) = (Vec::new(), Vec::new());
        host.poll(&mut hm);
        joiner.poll(&mut jm);
        let SignalServerMessage::PeerArrived { peer } = hm[1] else {
            panic!("{hm:?}")
        };
        let SignalServerMessage::JoinWelcome { peer: jp, .. } = jm[0] else {
            panic!("{jm:?}")
        };
        let mut he = net.endpoint("sha-256 HH");
        let mut je = net.endpoint("sha-256 JJ");
        he.open(peer, false, &[]);
        je.open(jp, true, &[]);
        let mut out = Vec::new();
        je.poll(&mut out);
        let Some(PeerOutput::Signal { signal, .. }) = out.pop() else {
            panic!()
        };
        joiner.send(&SignalClientMessage::Signal { peer: 0, signal });
        hm.clear();
        host.poll(&mut hm);
        let SignalServerMessage::Signal { peer: p, signal } = hm.remove(0) else {
            panic!()
        };
        assert_eq!(p, peer);
        he.signal(p, signal);
        he.poll(&mut out);
        let [
            PeerOutput::Signal { signal, .. },
            PeerOutput::Connected { link: mut hl, .. },
        ] = <[PeerOutput; 2]>::try_from(std::mem::take(&mut out))
            .ok()
            .unwrap()
        else {
            panic!()
        };
        je.signal(jp, signal);
        je.poll(&mut out);
        let Some(PeerOutput::Connected {
            link: mut jl,
            remote_fingerprint,
            ..
        }) = out.pop()
        else {
            panic!()
        };
        assert_eq!(remote_fingerprint, "sha-256 HH");
        jl.send(&WireFrame::Text("hi".into()));
        let mut f = Vec::new();
        hl.poll(&mut f);
        assert_eq!(f, [WireFrame::Text("hi".into())]);
        assert_eq!(net.links(), 1);
        // A wrong door is refused like an unknown room.
        let mut bad = connect("wss://x/signal/v1/rooms/room/join");
        bad.send(&SignalClientMessage::JoinHello {
            protocol: 1,
            door: keys::door(&key(), "room"),
            app: "x".into(),
        });
        let mut m = Vec::new();
        bad.poll(&mut m);
        assert!(matches!(
            m.as_slice(),
            [SignalServerMessage::Refused {
                reason: SignalRefusal::InvalidInvite,
                ..
            }]
        ));
        assert!(matches!(bad.state(), LinkState::Closed { fatal: true, .. }));
    }

    #[test]
    fn joiners_wait_for_an_offline_host() {
        let net = FakeNet::new();
        let k = key();
        let door = keys::door(&k, "room");
        let mut connect = net.signal_connector();
        let hello = SignalClientMessage::HostHello {
            protocol: 1,
            host_token: "t".into(),
            doors: vec![keys::door_hash(&door)],
            app: "test".into(),
        };
        let mut host = connect("wss://x/v1/rooms/room/host");
        host.send(&hello);
        host.close();
        let mut joiner = connect("wss://x/v1/rooms/room/join");
        joiner.send(&SignalClientMessage::JoinHello {
            protocol: 1,
            door,
            app: "test".into(),
        });
        let mut m = Vec::new();
        joiner.poll(&mut m);
        assert!(matches!(m[0], SignalServerMessage::HostOffline { .. }));
        let mut host = connect("wss://x/v1/rooms/room/host");
        host.send(&hello);
        m.clear();
        joiner.poll(&mut m);
        assert!(matches!(m[0], SignalServerMessage::JoinWelcome { .. }));
        // Another token cannot take the room.
        let mut thief = connect("wss://x/v1/rooms/room/host");
        thief.send(&SignalClientMessage::HostHello {
            protocol: 1,
            host_token: "other".into(),
            doors: vec![],
            app: "x".into(),
        });
        m.clear();
        thief.poll(&mut m);
        assert!(matches!(
            m[0],
            SignalServerMessage::Refused {
                reason: SignalRefusal::NotHost,
                ..
            }
        ));
        host.send(&SignalClientMessage::CloseRoom);
        assert!(!net.room_exists("room"));
        assert!(matches!(joiner.state(), LinkState::Closed { .. }));
    }
}

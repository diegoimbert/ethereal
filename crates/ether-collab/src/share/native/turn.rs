//! A small sans-IO TURN client (RFC 8656 subset, long-term credentials) for the share
//! endpoint's relayed candidates (docs/SHARING.md §6.1). It covers what a data-channel
//! pairing needs:
//! - **Allocate** (UDP relay, `REQUESTED-TRANSPORT` 17): the unauthenticated first try, the
//!   401 challenge (`REALM`, `NONCE`), the signed retry, 438 stale-nonce retries, and one
//!   clean-up retry after 437 (a leftover allocation on the same 5-tuple);
//! - **Refresh** before the lifetime ends, and `LIFETIME 0` to release;
//! - **CreatePermission** and **ChannelBind** on demand, the first time a datagram goes to a
//!   peer (queued until one of them succeeds), refreshed before they expire (5 min, 10 min);
//! - outgoing data as `ChannelData` once a channel is bound, `Send` indications before (or
//!   when the server refuses channels); incoming `ChannelData` and `Data` indications.
//!
//! Messages use the `stun` crate's codec (already in the tree for the relay's STUN server):
//! `MESSAGE-INTEGRITY` with the long-term key MD5(username:realm:password), `FINGERPRINT`.
//! Responses that carry a `MESSAGE-INTEGRITY` are checked and dropped when it is wrong.
//!
//! The same state machine runs over UDP (one message per datagram, retransmissions
//! 0.5/1/2 s) and over a TCP or TLS stream (no retransmissions; `ChannelData` padded to 4
//! bytes; [`split_stream`] cuts the byte stream into messages). The share thread owns the
//! sockets and the clock and calls [`Allocation::handle_input`],
//! [`Allocation::handle_timeout`], [`Allocation::send`], then drains
//! [`Allocation::poll_transmit`] and [`Allocation::poll_event`].

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::time::{Duration, Instant};

use stun::agent::TransactionId;
use stun::attributes::{
    ATTR_CHANNEL_NUMBER, ATTR_DATA, ATTR_LIFETIME, ATTR_NONCE, ATTR_REALM,
    ATTR_REQUESTED_TRANSPORT, ATTR_USERNAME, ATTR_XOR_PEER_ADDRESS, ATTR_XOR_RELAYED_ADDRESS,
    ATTR_XORMAPPED_ADDRESS, AttrType,
};
use stun::error_code::ErrorCodeAttribute;
use stun::fingerprint::FINGERPRINT;
use stun::integrity::MessageIntegrity;
use stun::message::{
    CLASS_ERROR_RESPONSE, CLASS_INDICATION, CLASS_REQUEST, CLASS_SUCCESS_RESPONSE, Getter,
    METHOD_ALLOCATE, METHOD_CHANNEL_BIND, METHOD_CREATE_PERMISSION, METHOD_DATA, METHOD_REFRESH,
    METHOD_SEND, Message, MessageType, Method, Setter,
};
use stun::textattrs::TextAttribute;
use stun::xoraddr::XorMappedAddress;

/// Default `turn:` port (RFC 8656 §3.1).
pub const DEFAULT_PORT: u16 = 3478;
/// Default `turns:` port.
pub const DEFAULT_TLS_PORT: u16 = 5349;
/// UDP, the only relayed transport a data channel needs.
const PROTO_UDP: u8 = 17;
/// What we ask for; servers may grant less.
const REQUESTED_LIFETIME: u32 = 600;
/// Permissions live 300 s (RFC 8656 §9); refreshed after this.
const PERMISSION_REFRESH: Duration = Duration::from_secs(240);
/// Channel bindings live 600 s (RFC 8656 §12); refreshed after this.
const CHANNEL_REFRESH: Duration = Duration::from_secs(540);
/// UDP retransmission: first RTO, then doubled; a transaction fails after
/// [`TRANSACTION_TIMEOUT`] (a stream transport only waits that long).
const RTO: Duration = Duration::from_millis(500);
const TRANSACTION_TIMEOUT: Duration = Duration::from_secs(5);
/// Channel numbers (RFC 8656 §12).
const FIRST_CHANNEL: u16 = 0x4000;
const LAST_CHANNEL: u16 = 0x4FFF;
/// Datagrams held while a permission is pending (all peers together).
const MAX_QUEUED: usize = 128;
/// Peer addresses (and IPs) one allocation tracks: a pairing's remote candidates.
const MAX_PEERS: usize = 256;

/// How the client reaches the TURN server.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Transport {
    Udp,
    Tcp,
    Tls,
}

impl Transport {
    pub fn is_stream(self) -> bool {
        !matches!(self, Transport::Udp)
    }
}

/// A parsed `turn:`/`turns:` URL (RFC 7065).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TurnUrl {
    pub transport: Transport,
    pub host: String,
    pub port: u16,
}

impl TurnUrl {
    /// Resolve to the first IPv4 address (the share socket is IPv4). Blocking (DNS): call off
    /// the share thread.
    pub fn resolve(&self) -> Option<SocketAddr> {
        (self.host.as_str(), self.port)
            .to_socket_addrs()
            .ok()?
            .find(SocketAddr::is_ipv4)
    }
}

/// `turn:host[:port][?transport=udp|tcp]` or `turns:host[:port][?transport=tcp]`; anything
/// else (`stun:`, DTLS `turns:…?transport=udp`, bad ports) is `None`.
pub fn parse_turn_url(url: &str) -> Option<TurnUrl> {
    let (secure, rest) = if let Some(r) = url.strip_prefix("turns:") {
        (true, r)
    } else {
        (false, url.strip_prefix("turn:")?)
    };
    let (hostport, query) = rest.split_once('?').unwrap_or((rest, ""));
    let mut tcp = secure;
    for kv in query.split('&').filter(|s| !s.is_empty()) {
        match kv.split_once('=') {
            Some(("transport", "udp")) if !secure => tcp = false,
            Some(("transport", "udp")) => return None,
            Some(("transport", "tcp")) => tcp = true,
            Some(("transport", _)) => return None,
            _ => {}
        }
    }
    let default = if secure {
        DEFAULT_TLS_PORT
    } else {
        DEFAULT_PORT
    };
    let (host, port) = if let Some(v6) = hostport.strip_prefix('[') {
        let (host, after) = v6.split_once(']')?;
        match after.strip_prefix(':') {
            Some(p) => (host, p.parse().ok()?),
            None if after.is_empty() => (host, default),
            None => return None,
        }
    } else {
        match hostport.rsplit_once(':') {
            Some((host, p)) => (host, p.parse().ok()?),
            None => (hostport, default),
        }
    };
    if host.is_empty() || port == 0 {
        return None;
    }
    let transport = match (secure, tcp) {
        (true, _) => Transport::Tls,
        (false, true) => Transport::Tcp,
        (false, false) => Transport::Udp,
    };
    Some(TurnUrl {
        transport,
        host: host.to_string(),
        port,
    })
}

/// The order a pairing tries TURN servers in: UDP first (lowest latency), then TLS (443
/// first: it passes most firewalls), then plain TCP. Duplicates are dropped, at most `max`.
pub fn attempt_order(urls: Vec<(TurnUrl, Credentials)>, max: usize) -> Vec<(TurnUrl, Credentials)> {
    let rank = |u: &TurnUrl| match (u.transport, u.port) {
        (Transport::Udp, _) => 0,
        (Transport::Tls, 443) => 1,
        (Transport::Tls, _) => 2,
        (Transport::Tcp, _) => 3,
    };
    let mut v: Vec<(usize, (TurnUrl, Credentials))> = urls.into_iter().enumerate().collect();
    v.sort_by_key(|(i, (u, _))| (rank(u), *i));
    let mut out: Vec<(TurnUrl, Credentials)> = Vec::new();
    for (_, (u, c)) in v {
        if !out.iter().any(|(o, _)| *o == u) {
            out.push((u, c));
        }
    }
    out.truncate(max);
    out
}

/// Long-term credentials (from the advertised `IceServer`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

/// What an [`Allocation`] reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The relayed transport address (the relay candidate) and our address as the server
    /// saw it (a server-reflexive candidate, for free).
    Allocated {
        relayed: SocketAddr,
        mapped: Option<SocketAddr>,
    },
    /// The allocation could not be made, or was lost (refresh refused, server gone).
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Kind {
    Allocate,
    Refresh(u32),
    Permission(Vec<IpAddr>),
    Bind(u16, SocketAddr),
}

struct Transaction {
    tid: [u8; 12],
    kind: Kind,
    raw: Vec<u8>,
    started: Instant,
    next: Option<Instant>,
    rto: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Allocating,
    Allocated { refresh_at: Instant },
    Failed,
    Released,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Grant {
    Pending,
    Granted { refresh_at: Instant },
    Refused,
}

struct Channel {
    number: u16,
    grant: Grant,
}

/// One TURN allocation (see the module docs).
pub struct Allocation {
    server: SocketAddr,
    transport: Transport,
    creds: Credentials,
    /// `(realm, nonce)` once challenged: every later request is signed.
    auth: Option<(String, String)>,
    key: Option<MessageIntegrity>,
    state: State,
    relayed: Option<SocketAddr>,
    transactions: Vec<Transaction>,
    permissions: HashMap<IpAddr, Grant>,
    channels: HashMap<SocketAddr, Channel>,
    by_number: HashMap<u16, SocketAddr>,
    next_channel: u16,
    queued: VecDeque<(SocketAddr, Vec<u8>)>,
    mismatch_retried: bool,
    transmits: VecDeque<Vec<u8>>,
    events: VecDeque<Event>,
}

/// What [`Allocation::handle_input`] made of a message from the server.
#[derive(Debug, PartialEq, Eq)]
pub enum Received {
    /// Relayed data from `peer` (to deliver as if it arrived at the relayed address).
    Data(SocketAddr, Vec<u8>),
    /// A response or something we ignore.
    Consumed,
}

struct Raw(AttrType, Vec<u8>);

impl Setter for Raw {
    fn add_to(&self, m: &mut Message) -> Result<(), stun::Error> {
        m.add(self.0, &self.1);
        Ok(())
    }
}

fn xor_addr(a: SocketAddr) -> XorMappedAddress {
    XorMappedAddress {
        ip: a.ip(),
        port: a.port(),
    }
}

fn get_addr(m: &Message, t: AttrType) -> Option<SocketAddr> {
    let mut a = XorMappedAddress::default();
    a.get_from_as(m, t).ok()?;
    Some(SocketAddr::new(a.ip, a.port))
}

fn random_tid() -> [u8; 12] {
    let mut tid = [0u8; 12];
    let _ = getrandom::fill(&mut tid);
    tid
}

/// A `ChannelData` message (RFC 8656 §12.4); padded to 4 bytes on stream transports.
pub fn channel_data(number: u16, data: &[u8], pad: bool) -> Vec<u8> {
    let mut v = Vec::with_capacity(4 + data.len() + 3);
    v.extend_from_slice(&number.to_be_bytes());
    v.extend_from_slice(&(data.len() as u16).to_be_bytes());
    v.extend_from_slice(data);
    if pad {
        v.resize(v.len().div_ceil(4) * 4, 0);
    }
    v
}

/// `(channel, data)` of a `ChannelData` message, or `None`.
pub fn parse_channel_data(buf: &[u8]) -> Option<(u16, &[u8])> {
    if buf.len() < 4 {
        return None;
    }
    let number = u16::from_be_bytes([buf[0], buf[1]]);
    let len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
    if !(FIRST_CHANNEL..=LAST_CHANNEL).contains(&number) || 4 + len > buf.len() {
        return None;
    }
    Some((number, &buf[4..4 + len]))
}

/// Cut complete messages (STUN or `ChannelData`, RFC 8656 §12.5 framing) off the front of
/// a stream buffer. Returns the messages, or `None` when the stream is not TURN (the
/// connection should be dropped). Partial data stays in `buf`.
pub fn split_stream(buf: &mut Vec<u8>) -> Option<Vec<Vec<u8>>> {
    let mut out = Vec::new();
    let mut at = 0;
    while buf.len() - at >= 4 {
        let b = &buf[at..];
        let len = u16::from_be_bytes([b[2], b[3]]) as usize;
        let total = match b[0] >> 6 {
            0 => 20 + len,
            1 => (4 + len).div_ceil(4) * 4,
            _ => return None,
        };
        if b.len() < total {
            break;
        }
        let msg_len = if b[0] >> 6 == 1 { 4 + len } else { total };
        out.push(b[..msg_len].to_vec());
        at += total;
    }
    buf.drain(..at);
    Some(out)
}

impl Allocation {
    /// Start allocating on `server` (the first, unauthenticated Allocate is queued).
    pub fn new(server: SocketAddr, transport: Transport, creds: Credentials, now: Instant) -> Self {
        let mut a = Self {
            server,
            transport,
            creds,
            auth: None,
            key: None,
            state: State::Allocating,
            relayed: None,
            transactions: Vec::new(),
            permissions: HashMap::new(),
            channels: HashMap::new(),
            by_number: HashMap::new(),
            next_channel: FIRST_CHANNEL,
            queued: VecDeque::new(),
            mismatch_retried: false,
            transmits: VecDeque::new(),
            events: VecDeque::new(),
        };
        a.start(Kind::Allocate, now);
        a
    }

    pub fn server(&self) -> SocketAddr {
        self.server
    }

    pub fn transport(&self) -> Transport {
        self.transport
    }

    /// The relayed address, once allocated.
    pub fn relayed(&self) -> Option<SocketAddr> {
        match self.state {
            State::Allocated { .. } => self.relayed,
            _ => None,
        }
    }

    pub fn is_failed(&self) -> bool {
        matches!(self.state, State::Failed | State::Released)
    }

    pub fn username(&self) -> &str {
        &self.creds.username
    }

    /// The next message for the server (a datagram, or bytes for the stream).
    pub fn poll_transmit(&mut self) -> Option<Vec<u8>> {
        self.transmits.pop_front()
    }

    pub fn poll_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    /// When [`handle_timeout`](Self::handle_timeout) wants to run next.
    pub fn next_timeout(&self) -> Option<Instant> {
        let mut t: Option<Instant> = None;
        let mut at = |i: Instant| t = Some(t.map_or(i, |t| t.min(i)));
        for x in &self.transactions {
            at(x.next.unwrap_or(x.started + TRANSACTION_TIMEOUT));
        }
        if let State::Allocated { refresh_at } = self.state {
            at(refresh_at);
            for g in self.permissions.values() {
                if let Grant::Granted { refresh_at } = g {
                    at(*refresh_at);
                }
            }
            for c in self.channels.values() {
                if let Grant::Granted { refresh_at } = c.grant {
                    at(refresh_at);
                }
            }
        }
        t
    }

    /// Retransmit, time out transactions, refresh what is due.
    pub fn handle_timeout(&mut self, now: Instant) {
        let mut expired = Vec::new();
        for x in &mut self.transactions {
            if now >= x.started + TRANSACTION_TIMEOUT {
                expired.push(x.tid);
            } else if let Some(next) = x.next
                && now >= next
            {
                self.transmits.push_back(x.raw.clone());
                x.rto *= 2;
                let n = now + x.rto;
                x.next = (n < x.started + TRANSACTION_TIMEOUT).then_some(n);
            }
        }
        for tid in expired {
            if let Some(i) = self.transactions.iter().position(|x| x.tid == tid) {
                let x = self.transactions.remove(i);
                self.failed(x.kind, "the TURN server did not answer".into());
            }
        }
        let State::Allocated { refresh_at } = self.state else {
            return;
        };
        if now >= refresh_at && !self.in_flight(|k| matches!(k, Kind::Refresh(_))) {
            self.state = State::Allocated {
                refresh_at: now + Duration::from_secs(3600),
            };
            self.start(Kind::Refresh(REQUESTED_LIFETIME), now);
        }
        let due: Vec<IpAddr> = self
            .permissions
            .iter()
            .filter(|(_, g)| matches!(g, Grant::Granted { refresh_at } if now >= *refresh_at))
            .map(|(ip, _)| *ip)
            .collect();
        if !due.is_empty() {
            for ip in &due {
                self.permissions.insert(
                    *ip,
                    Grant::Granted {
                        refresh_at: now + Duration::from_secs(3600),
                    },
                );
            }
            self.start(Kind::Permission(due), now);
        }
        let due: Vec<(u16, SocketAddr)> = self
            .channels
            .iter()
            .filter(|(_, c)| matches!(c.grant, Grant::Granted { refresh_at } if now >= refresh_at))
            .map(|(a, c)| (c.number, *a))
            .collect();
        for (n, a) in due {
            if let Some(c) = self.channels.get_mut(&a) {
                c.grant = Grant::Granted {
                    refresh_at: now + Duration::from_secs(3600),
                };
            }
            self.start(Kind::Bind(n, a), now);
        }
    }

    /// Relay `data` to `peer` (dropped before the allocation exists or after it failed).
    pub fn send(&mut self, peer: SocketAddr, data: &[u8], now: Instant) {
        if !matches!(self.state, State::Allocated { .. }) || peer.is_ipv4() != self.server.is_ipv4()
        {
            return;
        }
        if let Some(c) = self.channels.get(&peer)
            && matches!(c.grant, Grant::Granted { .. })
        {
            let m = channel_data(c.number, data, self.transport.is_stream());
            self.transmits.push_back(m);
            return;
        }
        let ip = peer.ip();
        match self.permissions.get(&ip).copied() {
            Some(Grant::Granted { .. }) => {
                let m = self.indication(peer, data);
                self.transmits.push_back(m);
                self.bind(peer, now);
            }
            Some(Grant::Refused) => {}
            Some(Grant::Pending) => self.queue(peer, data),
            None => {
                if self.permissions.len() >= MAX_PEERS {
                    return;
                }
                self.permissions.insert(ip, Grant::Pending);
                self.start(Kind::Permission(vec![ip]), now);
                self.bind(peer, now);
                self.queue(peer, data);
            }
        }
    }

    /// The transport under the allocation broke (a TCP/TLS connection closed).
    pub fn lost(&mut self, reason: String) {
        self.fail(reason);
    }

    /// Release the allocation (`Refresh` with `LIFETIME 0`, sent once, not awaited).
    pub fn release(&mut self, now: Instant) {
        let _ = now;
        if matches!(self.state, State::Allocated { .. }) {
            self.fire(Kind::Refresh(0));
        }
        self.state = State::Released;
        self.transactions.clear();
        self.queued.clear();
    }

    /// A message from the server (one datagram, or one framed message off the stream).
    pub fn handle_input(&mut self, buf: &[u8], now: Instant) -> Received {
        if let Some((number, data)) = parse_channel_data(buf) {
            return match self.by_number.get(&number) {
                Some(peer) if !self.is_failed() => Received::Data(*peer, data.to_vec()),
                _ => Received::Consumed,
            };
        }
        if !stun::message::is_message(buf) {
            return Received::Consumed;
        }
        let mut m = Message::new();
        if m.unmarshal_binary(buf).is_err() {
            return Received::Consumed;
        }
        if m.typ.class == CLASS_INDICATION {
            if m.typ.method == METHOD_DATA
                && !self.is_failed()
                && let Some(peer) = get_addr(&m, ATTR_XOR_PEER_ADDRESS)
                && let Ok(data) = m.get(ATTR_DATA)
                && matches!(
                    self.permissions.get(&peer.ip()),
                    Some(Grant::Granted { .. })
                )
            {
                return Received::Data(peer, data);
            }
            return Received::Consumed;
        }
        let tid = m.transaction_id.0;
        let Some(i) = self.transactions.iter().position(|x| x.tid == tid) else {
            return Received::Consumed;
        };
        // A response that claims integrity must have it (an off-path forger has neither the
        // transaction id nor the key; this guards the on-path case).
        if m.contains(stun::attributes::ATTR_MESSAGE_INTEGRITY)
            && let Some(key) = &self.key
            && key.check(&mut m).is_err()
        {
            return Received::Consumed;
        }
        let x = self.transactions.remove(i);
        match m.typ.class {
            CLASS_SUCCESS_RESPONSE => self.succeeded(x.kind, &m, now),
            CLASS_ERROR_RESPONSE => self.error(x.kind, &m, now),
            _ => {}
        }
        Received::Consumed
    }

    fn queue(&mut self, peer: SocketAddr, data: &[u8]) {
        if self.queued.len() >= MAX_QUEUED {
            self.queued.pop_front();
        }
        self.queued.push_back((peer, data.to_vec()));
    }

    /// Send what was waiting for a permission on `ip` (or a channel to its peers).
    fn flush(&mut self, ip: IpAddr, now: Instant) {
        let (ready, rest): (VecDeque<_>, VecDeque<_>) =
            self.queued.drain(..).partition(|(p, _)| p.ip() == ip);
        self.queued = rest;
        for (peer, data) in ready {
            self.send(peer, &data, now);
        }
    }

    fn bind(&mut self, peer: SocketAddr, now: Instant) {
        if self.channels.contains_key(&peer)
            || self.channels.len() >= MAX_PEERS
            || self.next_channel > LAST_CHANNEL
        {
            return;
        }
        let number = self.next_channel;
        self.next_channel += 1;
        self.channels.insert(
            peer,
            Channel {
                number,
                grant: Grant::Pending,
            },
        );
        self.start(Kind::Bind(number, peer), now);
    }

    fn in_flight(&self, f: impl Fn(&Kind) -> bool) -> bool {
        self.transactions.iter().any(|x| f(&x.kind))
    }

    fn method(kind: &Kind) -> Method {
        match kind {
            Kind::Allocate => METHOD_ALLOCATE,
            Kind::Refresh(_) => METHOD_REFRESH,
            Kind::Permission(_) => METHOD_CREATE_PERMISSION,
            Kind::Bind(..) => METHOD_CHANNEL_BIND,
        }
    }

    fn encode(&self, kind: &Kind, tid: [u8; 12]) -> Vec<u8> {
        let mut setters: Vec<Box<dyn Setter>> = vec![
            Box::new(TransactionId(tid)),
            Box::new(MessageType::new(Self::method(kind), CLASS_REQUEST)),
        ];
        match kind {
            Kind::Allocate => {
                setters.push(Box::new(Raw(
                    ATTR_REQUESTED_TRANSPORT,
                    vec![PROTO_UDP, 0, 0, 0],
                )));
                setters.push(Box::new(Raw(
                    ATTR_LIFETIME,
                    REQUESTED_LIFETIME.to_be_bytes().to_vec(),
                )));
            }
            Kind::Refresh(l) => {
                setters.push(Box::new(Raw(ATTR_LIFETIME, l.to_be_bytes().to_vec())))
            }
            Kind::Permission(ips) => {
                for ip in ips {
                    setters.push(Box::new(XorPeer(SocketAddr::new(*ip, 0))));
                }
            }
            Kind::Bind(n, peer) => {
                let mut v = n.to_be_bytes().to_vec();
                v.extend_from_slice(&[0, 0]);
                setters.push(Box::new(Raw(ATTR_CHANNEL_NUMBER, v)));
                setters.push(Box::new(XorPeer(*peer)));
            }
        }
        if let (Some((realm, nonce)), Some(key)) = (&self.auth, &self.key) {
            setters.push(Box::new(TextAttribute::new(
                ATTR_USERNAME,
                self.creds.username.clone(),
            )));
            setters.push(Box::new(TextAttribute::new(ATTR_REALM, realm.clone())));
            setters.push(Box::new(TextAttribute::new(ATTR_NONCE, nonce.clone())));
            setters.push(Box::new(MessageIntegrity(key.0.clone())));
        }
        setters.push(Box::new(FINGERPRINT));
        let mut m = Message::new();
        // Only fails on oversized attributes; ours are bounded.
        let _ = m.build(&setters);
        m.raw
    }

    fn indication(&self, peer: SocketAddr, data: &[u8]) -> Vec<u8> {
        let setters: Vec<Box<dyn Setter>> = vec![
            Box::new(TransactionId(random_tid())),
            Box::new(MessageType::new(METHOD_SEND, CLASS_INDICATION)),
            Box::new(XorPeer(peer)),
            Box::new(Raw(ATTR_DATA, data.to_vec())),
        ];
        let mut m = Message::new();
        let _ = m.build(&setters);
        m.raw
    }

    /// Send a request once, without waiting for (or retransmitting until) its answer.
    fn fire(&mut self, kind: Kind) {
        let raw = self.encode(&kind, random_tid());
        self.transmits.push_back(raw);
    }

    fn start(&mut self, kind: Kind, now: Instant) {
        let tid = random_tid();
        let raw = self.encode(&kind, tid);
        self.transmits.push_back(raw.clone());
        let next = (!self.transport.is_stream()).then_some(now + RTO);
        self.transactions.push(Transaction {
            tid,
            kind,
            raw,
            started: now,
            next,
            rto: RTO,
        });
    }

    fn fail(&mut self, reason: String) {
        if matches!(self.state, State::Failed | State::Released) {
            return;
        }
        self.state = State::Failed;
        self.transactions.clear();
        self.queued.clear();
        self.events.push_back(Event::Failed(reason));
    }

    fn failed(&mut self, kind: Kind, reason: String) {
        match kind {
            Kind::Allocate | Kind::Refresh(_) => self.fail(reason),
            Kind::Permission(ips) => {
                // Refused (e.g. 403: a private address the server will not reach), or a
                // refresh that failed: no more data to these IPs.
                for ip in &ips {
                    self.permissions.insert(*ip, Grant::Refused);
                }
                self.queued.retain(|(p, _)| !ips.contains(&p.ip()));
            }
            Kind::Bind(_, peer) => {
                // No channel for this peer: Send indications carry its data.
                if let Some(c) = self.channels.get_mut(&peer) {
                    c.grant = Grant::Refused;
                }
            }
        }
    }

    fn succeeded(&mut self, kind: Kind, m: &Message, now: Instant) {
        match kind {
            Kind::Allocate => {
                let Some(relayed) = get_addr(m, ATTR_XOR_RELAYED_ADDRESS) else {
                    return self.fail("the TURN server sent no relayed address".into());
                };
                let lifetime = lifetime(m).unwrap_or(REQUESTED_LIFETIME);
                self.relayed = Some(relayed);
                self.state = State::Allocated {
                    refresh_at: now + refresh_after(lifetime),
                };
                let mapped = get_addr(m, ATTR_XORMAPPED_ADDRESS);
                self.events.push_back(Event::Allocated { relayed, mapped });
            }
            Kind::Refresh(0) => {}
            Kind::Refresh(_) => {
                let lifetime = lifetime(m).unwrap_or(REQUESTED_LIFETIME);
                if let State::Allocated { .. } = self.state {
                    self.state = State::Allocated {
                        refresh_at: now + refresh_after(lifetime),
                    };
                }
            }
            Kind::Permission(ips) => {
                for ip in ips {
                    self.permissions.insert(
                        ip,
                        Grant::Granted {
                            refresh_at: now + PERMISSION_REFRESH,
                        },
                    );
                    self.flush(ip, now);
                }
            }
            Kind::Bind(number, peer) => {
                if let Some(c) = self.channels.get_mut(&peer) {
                    c.grant = Grant::Granted {
                        refresh_at: now + CHANNEL_REFRESH,
                    };
                }
                self.by_number.insert(number, peer);
                // A binding installs (or refreshes) the permission for the peer's IP.
                let ip = peer.ip();
                let refresh_at = now + PERMISSION_REFRESH;
                let flush = !matches!(self.permissions.get(&ip), Some(Grant::Granted { .. }));
                if flush
                    || matches!(self.permissions.get(&ip), Some(Grant::Granted { refresh_at: r }) if *r < refresh_at)
                {
                    self.permissions.insert(ip, Grant::Granted { refresh_at });
                }
                if flush {
                    self.flush(ip, now);
                }
            }
        }
    }

    fn error(&mut self, kind: Kind, m: &Message, now: Instant) {
        let mut e = ErrorCodeAttribute::default();
        let code = if e.get_from(m).is_ok() { e.code.0 } else { 0 };
        let reason = String::from_utf8_lossy(&e.reason).into_owned();
        let realm = TextAttribute::get_from_as(m, ATTR_REALM)
            .ok()
            .map(|t| t.text);
        let nonce = TextAttribute::get_from_as(m, ATTR_NONCE)
            .ok()
            .map(|t| t.text);
        match code {
            // The challenge (first Allocate), or a stale nonce: sign with what we got and
            // try again. A second 401 means the credentials are wrong.
            401 | 438 => {
                let challenged = code == 401 && self.auth.is_some();
                let Some(nonce) = nonce else {
                    return self.failed(kind, format!("TURN error {code} without a nonce"));
                };
                if challenged {
                    return self.failed(kind, "the TURN server refused the credentials".into());
                }
                let realm = realm
                    .or_else(|| self.auth.as_ref().map(|(r, _)| r.clone()))
                    .unwrap_or_default();
                self.key = Some(MessageIntegrity::new_long_term_integrity(
                    self.creds.username.clone(),
                    realm.clone(),
                    self.creds.password.clone(),
                ));
                self.auth = Some((realm, nonce));
                self.start(kind, now);
            }
            // A leftover allocation on our 5-tuple: release it once and allocate again.
            437 if kind == Kind::Allocate && !self.mismatch_retried && self.auth.is_some() => {
                self.mismatch_retried = true;
                self.fire(Kind::Refresh(0));
                self.start(Kind::Allocate, now);
            }
            // Signed and refused: most servers answer a bad MESSAGE-INTEGRITY with 400 or
            // 401, a username change with 441.
            400 | 441 if self.auth.is_some() => {
                self.failed(
                    kind,
                    format!("the TURN server refused the credentials (error {code})"),
                );
            }
            _ => {
                let what = if reason.is_empty() {
                    format!("TURN error {code}")
                } else {
                    format!("TURN error {code} ({reason})")
                };
                self.failed(kind, what);
            }
        }
    }
}

struct XorPeer(SocketAddr);

impl Setter for XorPeer {
    fn add_to(&self, m: &mut Message) -> Result<(), stun::Error> {
        xor_addr(self.0).add_to_as(m, ATTR_XOR_PEER_ADDRESS)
    }
}

fn lifetime(m: &Message) -> Option<u32> {
    let v = m.get(ATTR_LIFETIME).ok()?;
    Some(u32::from_be_bytes(v.get(..4)?.try_into().ok()?))
}

/// Refresh a minute before expiry (half-way through very short lifetimes).
fn refresh_after(lifetime: u32) -> Duration {
    let l = Duration::from_secs(lifetime.max(2) as u64);
    if l > Duration::from_secs(120) {
        l - Duration::from_secs(60)
    } else {
        l / 2
    }
}

/// A minimal in-process TURN server for tests: one allocation per client, long-term
/// credentials, permissions, channels, `Send`/`Data` indications and `ChannelData`. It
/// relays between clients by "relayed address" without real sockets: [`FakeServer::relay`]
/// returns what the server would send where.
#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::collections::HashSet;

    pub const REALM: &str = "ethereal.test";
    pub const NONCE: &str = "nonce-1";

    #[derive(Default)]
    pub struct Alloc {
        pub relayed: Option<SocketAddr>,
        pub perms: HashSet<IpAddr>,
        pub channels: HashMap<u16, SocketAddr>,
    }

    /// Keyed by the client's address.
    pub struct FakeServer {
        pub creds: Credentials,
        pub allocs: HashMap<SocketAddr, Alloc>,
        pub next_port: u16,
        pub refuse_channels: bool,
        pub stale_once: bool,
        pub lifetime: u32,
        pub requests: Vec<(Method, bool)>,
    }

    pub enum Out {
        /// A response or a `Data`/`ChannelData` to client `to`.
        To(Vec<u8>),
        /// Relayed bytes leaving the relay at `from` (a relayed address) towards `peer`.
        Relayed {
            from: SocketAddr,
            peer: SocketAddr,
            data: Vec<u8>,
        },
    }

    impl FakeServer {
        pub fn new(creds: Credentials) -> Self {
            Self {
                creds,
                allocs: HashMap::new(),
                next_port: 50_000,
                refuse_channels: false,
                stale_once: false,
                lifetime: 600,
                requests: Vec::new(),
            }
        }

        fn key(&self) -> MessageIntegrity {
            MessageIntegrity::new_long_term_integrity(
                self.creds.username.clone(),
                REALM.into(),
                self.creds.password.clone(),
            )
        }

        pub fn respond(
            &self,
            req: &Message,
            class: stun::message::MessageClass,
            attrs: Vec<Box<dyn Setter>>,
            sign: bool,
        ) -> Vec<u8> {
            let mut setters: Vec<Box<dyn Setter>> = vec![
                Box::new(TransactionId(req.transaction_id.0)),
                Box::new(MessageType::new(req.typ.method, class)),
            ];
            setters.extend(attrs);
            if sign {
                setters.push(Box::new(self.key()));
            }
            let mut m = Message::new();
            m.build(&setters).unwrap();
            m.raw
        }

        fn err(&self, req: &Message, code: u16, nonce: bool) -> Vec<u8> {
            let mut attrs: Vec<Box<dyn Setter>> = vec![Box::new(ErrorCodeAttribute {
                code: stun::error_code::ErrorCode(code),
                reason: b"test".to_vec(),
            })];
            if nonce {
                attrs.push(Box::new(TextAttribute::new(ATTR_REALM, REALM.into())));
                attrs.push(Box::new(TextAttribute::new(ATTR_NONCE, NONCE.into())));
            }
            self.respond(req, CLASS_ERROR_RESPONSE, attrs, false)
        }

        /// The server's reaction to `buf` from `client`.
        pub fn handle(&mut self, client: SocketAddr, buf: &[u8]) -> Vec<Out> {
            if let Some((n, data)) = parse_channel_data(buf) {
                let Some(a) = self.allocs.get(&client) else {
                    return vec![];
                };
                let (Some(from), Some(peer)) = (a.relayed, a.channels.get(&n)) else {
                    return vec![];
                };
                return vec![Out::Relayed {
                    from,
                    peer: *peer,
                    data: data.to_vec(),
                }];
            }
            let mut m = Message::new();
            m.unmarshal_binary(buf).unwrap();
            if m.typ.class == CLASS_INDICATION {
                assert_eq!(m.typ.method, METHOD_SEND);
                let a = &self.allocs[&client];
                let peer = get_addr(&m, ATTR_XOR_PEER_ADDRESS).unwrap();
                if !a.perms.contains(&peer.ip()) {
                    return vec![];
                }
                return vec![Out::Relayed {
                    from: a.relayed.unwrap(),
                    peer,
                    data: m.get(ATTR_DATA).unwrap(),
                }];
            }
            assert_eq!(m.typ.class, CLASS_REQUEST);
            stun::fingerprint::FINGERPRINT
                .check(&m)
                .expect("a fingerprint");
            let signed = m.contains(stun::attributes::ATTR_MESSAGE_INTEGRITY);
            self.requests.push((m.typ.method, signed));
            if !signed {
                return vec![Out::To(self.err(&m, 401, true))];
            }
            let user = TextAttribute::get_from_as(&m, ATTR_USERNAME).unwrap().text;
            if user != self.creds.username || self.key().check(&mut m.clone()).is_err() {
                return vec![Out::To(self.err(&m, 401, true))];
            }
            if self.stale_once {
                self.stale_once = false;
                return vec![Out::To(self.err(&m, 438, true))];
            }
            let resp = match m.typ.method {
                METHOD_ALLOCATE => {
                    if self
                        .allocs
                        .get(&client)
                        .is_some_and(|a| a.relayed.is_some())
                    {
                        self.err(&m, 437, false)
                    } else {
                        let relayed = SocketAddr::from(([198, 51, 100, 1], self.next_port));
                        self.next_port += 1;
                        self.allocs.insert(
                            client,
                            Alloc {
                                relayed: Some(relayed),
                                ..Alloc::default()
                            },
                        );
                        self.respond(
                            &m,
                            CLASS_SUCCESS_RESPONSE,
                            vec![
                                Box::new(XorRelayed(relayed)),
                                Box::new(xor_addr(client)),
                                Box::new(Raw(ATTR_LIFETIME, self.lifetime.to_be_bytes().to_vec())),
                            ],
                            true,
                        )
                    }
                }
                METHOD_REFRESH => {
                    let l = lifetime(&m).unwrap_or(600);
                    if l == 0 {
                        self.allocs.remove(&client);
                    }
                    self.respond(
                        &m,
                        CLASS_SUCCESS_RESPONSE,
                        vec![Box::new(Raw(ATTR_LIFETIME, l.to_be_bytes().to_vec()))],
                        true,
                    )
                }
                METHOD_CREATE_PERMISSION => {
                    let a = self.allocs.get_mut(&client).unwrap();
                    for attr in &m.attributes.0 {
                        if attr.typ == ATTR_XOR_PEER_ADDRESS {
                            let mut one = Message::new();
                            one.transaction_id = m.transaction_id;
                            one.add(ATTR_XOR_PEER_ADDRESS, &attr.value);
                            a.perms
                                .insert(get_addr(&one, ATTR_XOR_PEER_ADDRESS).unwrap().ip());
                        }
                    }
                    self.respond(&m, CLASS_SUCCESS_RESPONSE, vec![], true)
                }
                METHOD_CHANNEL_BIND => {
                    if self.refuse_channels {
                        self.err(&m, 400, false)
                    } else {
                        let v = m.get(ATTR_CHANNEL_NUMBER).unwrap();
                        let n = u16::from_be_bytes([v[0], v[1]]);
                        let peer = get_addr(&m, ATTR_XOR_PEER_ADDRESS).unwrap();
                        let a = self.allocs.get_mut(&client).unwrap();
                        a.channels.insert(n, peer);
                        a.perms.insert(peer.ip());
                        self.respond(&m, CLASS_SUCCESS_RESPONSE, vec![], true)
                    }
                }
                other => panic!("unexpected method {other:?}"),
            };
            vec![Out::To(resp)]
        }

        /// Data from `peer` arriving at relayed address `relayed`: what the server sends to
        /// the client that owns it (nothing without a permission).
        pub fn inbound(
            &self,
            relayed: SocketAddr,
            peer: SocketAddr,
            data: &[u8],
        ) -> Option<(SocketAddr, Vec<u8>)> {
            let (client, a) = self
                .allocs
                .iter()
                .find(|(_, a)| a.relayed == Some(relayed))?;
            if !a.perms.contains(&peer.ip()) {
                return None;
            }
            if let Some((n, _)) = a.channels.iter().find(|(_, p)| **p == peer) {
                return Some((*client, channel_data(*n, data, false)));
            }
            let setters: Vec<Box<dyn Setter>> = vec![
                Box::new(TransactionId(random_tid())),
                Box::new(MessageType::new(METHOD_DATA, CLASS_INDICATION)),
                Box::new(XorPeer(peer)),
                Box::new(Raw(ATTR_DATA, data.to_vec())),
            ];
            let mut m = Message::new();
            m.build(&setters).unwrap();
            Some((*client, m.raw))
        }
    }

    struct XorRelayed(SocketAddr);

    impl Setter for XorRelayed {
        fn add_to(&self, m: &mut Message) -> Result<(), stun::Error> {
            xor_addr(self.0).add_to_as(m, ATTR_XOR_RELAYED_ADDRESS)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{FakeServer, Out};
    use super::*;

    fn creds() -> Credentials {
        Credentials {
            username: "1700000000:ada".into(),
            password: "secret".into(),
        }
    }

    const CLIENT: &str = "192.0.2.10:40000";
    const SERVER: &str = "203.0.113.1:3478";

    fn client() -> SocketAddr {
        CLIENT.parse().unwrap()
    }

    /// Run the client against the fake server until neither has anything left to say.
    /// Returns the data the server relayed out.
    fn pump(
        a: &mut Allocation,
        s: &mut FakeServer,
        now: Instant,
    ) -> Vec<(SocketAddr, SocketAddr, Vec<u8>)> {
        let mut relayed = Vec::new();
        for _ in 0..64 {
            let Some(t) = a.poll_transmit() else { break };
            for o in s.handle(client(), &t) {
                match o {
                    Out::To(bytes) => {
                        assert_eq!(a.handle_input(&bytes, now), Received::Consumed);
                    }
                    Out::Relayed { from, peer, data } => relayed.push((from, peer, data)),
                }
            }
        }
        relayed
    }

    fn allocated(now: Instant) -> (Allocation, FakeServer, SocketAddr) {
        let mut s = FakeServer::new(creds());
        let mut a = Allocation::new(SERVER.parse().unwrap(), Transport::Udp, creds(), now);
        pump(&mut a, &mut s, now);
        let Some(Event::Allocated { relayed, mapped }) = a.poll_event() else {
            panic!("allocated");
        };
        assert_eq!(mapped, Some(client()));
        assert_eq!(a.relayed(), Some(relayed));
        (a, s, relayed)
    }

    #[test]
    fn urls() {
        let u = |s: &str| parse_turn_url(s);
        assert_eq!(
            u("turn:turn.cloudflare.com:3478?transport=udp"),
            Some(TurnUrl {
                transport: Transport::Udp,
                host: "turn.cloudflare.com".into(),
                port: 3478
            })
        );
        assert_eq!(u("turn:t.example").unwrap().port, 3478);
        assert_eq!(
            u("turn:t.example?transport=tcp").unwrap().transport,
            Transport::Tcp
        );
        assert_eq!(
            u("turns:turn.cloudflare.com:443?transport=tcp"),
            Some(TurnUrl {
                transport: Transport::Tls,
                host: "turn.cloudflare.com".into(),
                port: 443
            })
        );
        assert_eq!(u("turns:t.example").unwrap().port, 5349);
        assert_eq!(u("turn:[::1]:9").unwrap().host, "::1");
        assert_eq!(u("turn:[::1]").unwrap().port, 3478);
        assert_eq!(
            u("turns:t.example?transport=udp"),
            None,
            "DTLS is not supported"
        );
        assert_eq!(u("turn:t.example?transport=sctp"), None);
        assert_eq!(u("stun:t.example"), None);
        assert_eq!(u("turn::3478"), None);
        assert_eq!(u("turn:t.example:0"), None);
        assert_eq!(
            u("turn:127.0.0.1:5000").unwrap().resolve(),
            Some("127.0.0.1:5000".parse().unwrap())
        );
    }

    #[test]
    fn attempts_udp_then_tls_443_then_tcp() {
        let c = creds();
        let urls = [
            "turns:t.example:5349?transport=tcp",
            "turn:t.example:3478?transport=tcp",
            "turns:t.example:443?transport=tcp",
            "turn:t.example:3478?transport=udp",
            "turn:t.example:3478",
            "turn:t.example:53?transport=udp",
        ];
        let order = attempt_order(
            urls.iter()
                .map(|u| (parse_turn_url(u).unwrap(), c.clone()))
                .collect(),
            4,
        );
        let got: Vec<(Transport, u16)> = order.iter().map(|(u, _)| (u.transport, u.port)).collect();
        assert_eq!(
            got,
            vec![
                (Transport::Udp, 3478),
                (Transport::Udp, 53),
                (Transport::Tls, 443),
                (Transport::Tls, 5349)
            ]
        );
    }

    #[test]
    fn allocate_with_the_challenge() {
        let now = Instant::now();
        let (a, s, relayed) = allocated(now);
        assert_eq!(relayed.ip(), IpAddr::from([198, 51, 100, 1]));
        // Unsigned first try, then the signed one.
        assert_eq!(
            s.requests,
            vec![(METHOD_ALLOCATE, false), (METHOD_ALLOCATE, true)]
        );
        assert!(!a.is_failed());
    }

    #[test]
    fn wrong_password_fails() {
        let now = Instant::now();
        let mut s = FakeServer::new(creds());
        let mut a = Allocation::new(
            SERVER.parse().unwrap(),
            Transport::Udp,
            Credentials {
                password: "nope".into(),
                ..creds()
            },
            now,
        );
        pump(&mut a, &mut s, now);
        assert_eq!(
            a.poll_event(),
            Some(Event::Failed(
                "the TURN server refused the credentials".into()
            ))
        );
        assert!(a.is_failed());
        assert_eq!(a.relayed(), None);
    }

    #[test]
    fn stale_nonce_is_retried() {
        let now = Instant::now();
        let mut s = FakeServer::new(creds());
        s.stale_once = true;
        let mut a = Allocation::new(SERVER.parse().unwrap(), Transport::Udp, creds(), now);
        pump(&mut a, &mut s, now);
        assert!(matches!(a.poll_event(), Some(Event::Allocated { .. })));
    }

    #[test]
    fn no_answer_retransmits_then_fails() {
        let t0 = Instant::now();
        let mut a = Allocation::new(SERVER.parse().unwrap(), Transport::Udp, creds(), t0);
        let first = a.poll_transmit().unwrap();
        assert_eq!(a.poll_transmit(), None);
        assert_eq!(a.next_timeout(), Some(t0 + RTO));
        a.handle_timeout(t0 + RTO);
        assert_eq!(a.poll_transmit(), Some(first.clone()), "same transaction");
        a.handle_timeout(t0 + RTO * 3);
        assert_eq!(a.poll_transmit(), Some(first));
        a.handle_timeout(t0 + TRANSACTION_TIMEOUT);
        assert_eq!(
            a.poll_event(),
            Some(Event::Failed("the TURN server did not answer".into()))
        );
        assert_eq!(a.next_timeout(), None);
    }

    #[test]
    fn stream_transports_never_retransmit() {
        let t0 = Instant::now();
        let mut a = Allocation::new(SERVER.parse().unwrap(), Transport::Tls, creds(), t0);
        assert!(a.poll_transmit().is_some());
        assert_eq!(a.next_timeout(), Some(t0 + TRANSACTION_TIMEOUT));
        a.handle_timeout(t0 + Duration::from_secs(2));
        assert_eq!(a.poll_transmit(), None);
    }

    #[test]
    fn data_waits_for_the_permission_then_uses_the_channel() {
        let now = Instant::now();
        let (mut a, mut s, relayed) = allocated(now);
        let peer: SocketAddr = "192.0.2.77:6000".parse().unwrap();
        a.send(peer, b"one", now);
        a.send(peer, b"two", now);
        // CreatePermission + ChannelBind out; the data waited and leaves once granted (as
        // Send indications or ChannelData, in order).
        let out = pump(&mut a, &mut s, now);
        let datas: Vec<&[u8]> = out.iter().map(|(_, _, d)| d.as_slice()).collect();
        assert_eq!(datas, vec![b"one".as_slice(), b"two"]);
        assert!(out.iter().all(|(f, p, _)| *f == relayed && *p == peer));
        // Bound now: ChannelData (4-byte header).
        a.send(peer, b"three", now);
        let t = a.poll_transmit().unwrap();
        assert_eq!(
            parse_channel_data(&t).map(|(_, d)| d),
            Some(b"three".as_slice())
        );
        // Inbound both ways: ChannelData from a bound peer, a Data indication from another
        // peer on a permitted IP, nothing from a stranger.
        let (_, bytes) = s.inbound(relayed, peer, b"back").unwrap();
        assert_eq!(
            a.handle_input(&bytes, now),
            Received::Data(peer, b"back".to_vec())
        );
        let other: SocketAddr = "192.0.2.77:7000".parse().unwrap();
        let (_, bytes) = s.inbound(relayed, other, b"hi").unwrap();
        assert_eq!(
            a.handle_input(&bytes, now),
            Received::Data(other, b"hi".to_vec())
        );
        assert!(
            s.inbound(relayed, "192.0.2.99:1".parse().unwrap(), b"x")
                .is_none()
        );
    }

    #[test]
    fn refused_channels_fall_back_to_send_indications() {
        let now = Instant::now();
        let (mut a, mut s, _) = allocated(now);
        s.refuse_channels = true;
        let peer: SocketAddr = "192.0.2.77:6000".parse().unwrap();
        a.send(peer, b"one", now);
        let out = pump(&mut a, &mut s, now);
        assert_eq!(out.len(), 1);
        a.send(peer, b"two", now);
        let t = a.poll_transmit().unwrap();
        assert!(stun::message::is_message(&t), "a Send indication");
        assert_eq!(a.poll_transmit(), None, "no second ChannelBind");
        assert_eq!(s.handle(client(), &t).len(), 1);
    }

    #[test]
    fn refreshes_before_expiry_and_releases() {
        let t0 = Instant::now();
        let mut s = FakeServer::new(creds());
        s.lifetime = 300;
        let mut a = Allocation::new(SERVER.parse().unwrap(), Transport::Udp, creds(), t0);
        pump(&mut a, &mut s, t0);
        assert!(matches!(a.poll_event(), Some(Event::Allocated { .. })));
        let peer: SocketAddr = "192.0.2.77:6000".parse().unwrap();
        a.send(peer, b"x", t0);
        pump(&mut a, &mut s, t0);
        s.requests.clear();
        // The permission (240 s) and the allocation (300 - 60 s) are refreshed together.
        let t = a.next_timeout().unwrap();
        assert_eq!(t, t0 + Duration::from_secs(240));
        a.handle_timeout(t);
        pump(&mut a, &mut s, t);
        let methods: Vec<Method> = s.requests.iter().map(|(m, _)| *m).collect();
        assert!(methods.contains(&METHOD_REFRESH), "{methods:?}");
        assert!(methods.contains(&METHOD_CREATE_PERMISSION), "{methods:?}");
        assert!(a.next_timeout().unwrap() > t + Duration::from_secs(200));
        // The channel binding at 540 s.
        s.requests.clear();
        let t = t0 + CHANNEL_REFRESH;
        a.handle_timeout(t);
        pump(&mut a, &mut s, t);
        assert!(s.requests.iter().any(|(m, _)| *m == METHOD_CHANNEL_BIND));
        assert!(a.poll_event().is_none(), "nothing failed");

        a.release(t);
        pump(&mut a, &mut s, t);
        assert!(s.allocs.is_empty(), "released");
        assert!(a.is_failed());
        assert_eq!(a.next_timeout(), None);
        assert_eq!(a.poll_event(), None);
    }

    #[test]
    fn a_leftover_allocation_is_released_and_replaced() {
        let now = Instant::now();
        let mut s = FakeServer::new(creds());
        let mut old = Allocation::new(SERVER.parse().unwrap(), Transport::Udp, creds(), now);
        pump(&mut old, &mut s, now);
        // Same client address, a new allocation: 437, Refresh(0), Allocate again.
        let mut a = Allocation::new(SERVER.parse().unwrap(), Transport::Udp, creds(), now);
        pump(&mut a, &mut s, now);
        assert!(
            matches!(a.poll_event(), Some(Event::Allocated { .. })),
            "{:?}",
            s.requests
        );
    }

    #[test]
    fn forged_responses_are_ignored() {
        let now = Instant::now();
        let mut s = FakeServer::new(creds());
        let mut a = Allocation::new(SERVER.parse().unwrap(), Transport::Udp, creds(), now);
        // Challenge, then intercept the signed Allocate and answer it with the wrong key.
        let t = a.poll_transmit().unwrap();
        let Out::To(challenge) = s.handle(client(), &t).remove(0) else {
            panic!()
        };
        a.handle_input(&challenge, now);
        let signed = a.poll_transmit().unwrap();
        let mut evil = FakeServer::new(Credentials {
            password: "other".into(),
            ..creds()
        });
        evil.creds.password = "other".into();
        let mut req = Message::new();
        req.unmarshal_binary(&signed).unwrap();
        let forged = evil.respond(
            &req,
            CLASS_SUCCESS_RESPONSE,
            vec![Box::new(XorPeer("6.6.6.6:6".parse().unwrap()))],
            true,
        );
        a.handle_input(&forged, now);
        assert_eq!(a.poll_event(), None, "dropped");
        // The real answer still lands.
        for o in s.handle(client(), &signed) {
            if let Out::To(b) = o {
                a.handle_input(&b, now);
            }
        }
        assert!(matches!(a.poll_event(), Some(Event::Allocated { .. })));
    }

    #[test]
    fn channel_data_and_stream_framing() {
        assert_eq!(channel_data(0x4001, b"abcde", false).len(), 9);
        let padded = channel_data(0x4001, b"abcde", true);
        assert_eq!(padded.len(), 12);
        assert_eq!(
            parse_channel_data(&padded),
            Some((0x4001, b"abcde".as_slice()))
        );
        assert_eq!(parse_channel_data(&[0x80, 0, 0, 0]), None);
        assert_eq!(parse_channel_data(&[0x40, 0, 0, 9, 1]), None, "truncated");

        let a = Allocation::new(
            SERVER.parse().unwrap(),
            Transport::Tcp,
            creds(),
            Instant::now(),
        );
        let stun_msg = a.transmits[0].clone();
        let mut stream = Vec::new();
        stream.extend_from_slice(&padded);
        stream.extend_from_slice(&stun_msg);
        stream.extend_from_slice(&padded[..6]);
        let msgs = split_stream(&mut stream).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(
            parse_channel_data(&msgs[0]),
            Some((0x4001, b"abcde".as_slice()))
        );
        assert_eq!(msgs[1], stun_msg);
        assert_eq!(stream.len(), 6, "the partial message waits");
        stream.extend_from_slice(&padded[6..]);
        assert_eq!(split_stream(&mut stream).unwrap().len(), 1);
        assert!(stream.is_empty());
        assert!(split_stream(&mut vec![0xC0, 0, 0, 0]).is_none());
    }
}

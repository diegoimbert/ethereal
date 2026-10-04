//! The `ether-share` thread: one UDP socket, one str0m `Rtc` per pairing (sans-IO: this
//! thread owns the socket and the clock). A normal thread: it may allocate and block.
//!
//! Loop: commands → wait for a datagram (until the next `Rtc`/STUN/TURN deadline, or a wake
//! datagram from a link) → drain the socket (STUN responses by transaction id, datagrams
//! from a TURN server to its allocation, everything else to the `Rtc` that `accepts` it) →
//! due timeouts → queued fragments into SCTP → poll every `Rtc` until it wants time. Idle
//! (no pairing, no STUN query, no allocation): blocks on the commands.
//!
//! TURN (docs/SHARING.md §6.1): a pairing given TURN servers tries them in
//! [`turn::attempt_order`] (UDP, then TLS, then TCP) until one allocation succeeds; its
//! relayed address becomes a relay candidate in the pairing's `Rtc`. One allocation per
//! server is shared by every pairing (a UDP allocation is bound to our socket's 5-tuple,
//! so a second one on the same server would be refused): datagrams `Rtc`s send *from* a
//! relayed address go through that allocation, and what it relays in is handed to the
//! pairing whose `Rtc` accepts it. An allocation nobody has used for [`RELAY_IDLE`] is
//! released. Relay only ("Hide my IP"): no host or server-reflexive candidates, and
//! nothing is ever sent from the socket to a peer directly.

use std::collections::VecDeque;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, TryRecvError};
use ether_protocol::collab::{IceCandidate, IceServer, StreamSignal};
use ether_protocol::share::PeerId;
use str0m::change::{SdpAnswer, SdpOffer, SdpPendingOffer};
use str0m::channel::ChannelId;
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc};

use super::stream::StreamConn;
use super::turn::{self, Credentials, Received, Transport, TurnUrl};
use super::{Cmd, LinkShared, NativeConfig, NativeLink, RELAY_ONLY_NO_TURN, WAKE, Waker, stun};
use crate::share::PeerOutput;
use crate::share::dc::{DC_LABEL, Reassembler};
use crate::wire::WireFrame;

/// Defensive cap on concurrent pairings (the hub admits `MAX_PARTICIPANTS` = 16).
pub const MAX_PAIRINGS: usize = 32;
/// A STUN Binding request is retransmitted once after this, and given up after
/// [`STUN_GIVE_UP`].
const STUN_RETRANSMIT: Duration = Duration::from_millis(500);
const STUN_GIVE_UP: Duration = Duration::from_secs(2);
const MAX_STUN_URLS: usize = 2;
const MAX_STUN_QUERIES: usize = 64;
/// Longest socket wait when nothing is due sooner.
const IDLE_POLL: Duration = Duration::from_millis(50);
/// Datagrams read per wake before timeouts and sends get their turn.
const MAX_BURST: usize = 256;
/// SCTP buffered-amount-low threshold: below it, queued fragments are pushed again.
const LOW_WATER: usize = 64 * 1024;
/// TURN servers one pairing tries (the advertised list has up to six URLs).
const MAX_TURN_ATTEMPTS: usize = 3;
/// Concurrent allocations (one per server is shared by every pairing).
const MAX_RELAYS: usize = 8;
/// An allocation no pairing uses is released after this (a re-pairing reuses it).
pub const RELAY_IDLE: Duration = Duration::from_secs(30);

/// The IPv4 address of the interface the default route goes through (no packet is sent).
fn default_route_ip() -> Option<Ipv4Addr> {
    let s = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    // TEST-NET-1: routed like any public address, never contacted (connect() on UDP only
    // selects the route).
    s.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    match s.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

struct LinkEnd {
    inbound: Sender<WireFrame>,
    shared: Arc<LinkShared>,
}

struct Pairing {
    peer: PeerId,
    conn: u64,
    rtc: Rtc,
    /// Joiner: the offer waiting for its answer.
    pending: Option<SdpPendingOffer>,
    /// Our description was sent: local candidates may trickle.
    described: bool,
    /// The remote description is set: remote candidates may be added.
    remote_described: bool,
    early_remote: Vec<Candidate>,
    held_local: Vec<String>,
    /// The `a=mid` of the data channel's m-line (for trickled candidates).
    mid: Option<String>,
    channel: Option<ChannelId>,
    link: Option<LinkEnd>,
    outq: VecDeque<Vec<u8>>,
    reasm: Reassembler,
    created: Instant,
    disconnected_since: Option<Instant>,
    next_timeout: Instant,
    stun_pending: usize,
    /// "Hide my IP": relay candidates only, never a direct datagram.
    relay_only: bool,
    /// TURN servers still to try, in order.
    turn_attempts: VecDeque<(TurnUrl, Credentials)>,
    /// A TURN server is being tried (the end-of-candidates marker waits).
    turn_pending: bool,
    /// A relay candidate was added.
    relayed: bool,
    /// Why the last TURN server failed.
    turn_error: Option<String>,
    eoc_sent: bool,
    /// Terminal: the reason (the pairing is removed after the current drive).
    done: Option<String>,
}

fn mid_of(sdp: &str) -> Option<String> {
    sdp.lines()
        .find_map(|l| l.trim().strip_prefix("a=mid:"))
        .map(|m| m.trim().to_string())
}

impl Pairing {
    fn new(peer: PeerId, conn: u64, host: &[SocketAddr], now: Instant) -> Self {
        let mut rtc = Rtc::builder().build(now);
        for &a in host {
            if let Ok(c) = Candidate::host(a, "udp") {
                rtc.add_local_candidate(c);
            }
        }
        Self {
            peer,
            conn,
            rtc,
            pending: None,
            described: false,
            remote_described: false,
            early_remote: Vec::new(),
            held_local: Vec::new(),
            mid: None,
            channel: None,
            link: None,
            outq: VecDeque::new(),
            reasm: Reassembler::default(),
            created: now,
            disconnected_since: None,
            next_timeout: now,
            stun_pending: 0,
            relay_only: false,
            turn_attempts: VecDeque::new(),
            turn_pending: false,
            relayed: false,
            turn_error: None,
            eoc_sent: false,
            done: None,
        }
    }

    fn connected(&self) -> bool {
        self.link.is_some()
    }

    fn fail(&mut self, reason: impl Into<String>) {
        if self.done.is_none() {
            self.done = Some(reason.into());
        }
    }

    fn signal(&self, signal: StreamSignal) -> PeerOutput {
        PeerOutput::Signal {
            peer: self.peer,
            signal,
        }
    }

    fn ice(&self, candidate: String) -> PeerOutput {
        self.signal(StreamSignal::Ice {
            candidate: IceCandidate {
                candidate,
                sdp_mid: self.mid.clone(),
                sdp_m_line_index: Some(0),
                username_fragment: None,
            },
        })
    }

    /// Our description went out: release held candidates (and the end marker if due).
    fn set_described(&mut self, out: &mut Vec<PeerOutput>) {
        self.described = true;
        for c in std::mem::take(&mut self.held_local) {
            out.push(self.ice(c));
        }
        self.maybe_end_of_candidates(out);
    }

    fn trickle(&mut self, candidate: String, out: &mut Vec<PeerOutput>) {
        if self.described {
            out.push(self.ice(candidate));
        } else {
            self.held_local.push(candidate);
        }
    }

    fn maybe_end_of_candidates(&mut self, out: &mut Vec<PeerOutput>) {
        if self.stun_pending == 0 && !self.turn_pending && !self.eoc_sent && self.described {
            self.eoc_sent = true;
            out.push(self.ice(String::new()));
        }
    }

    fn add_srflx(&mut self, mapped: SocketAddr, base: SocketAddr, out: &mut Vec<PeerOutput>) {
        if mapped == base {
            return; // no NAT: the host candidate covers it
        }
        let Ok(c) = Candidate::server_reflexive(mapped, base, "udp") else {
            return;
        };
        if let Some(c) = self.rtc.add_local_candidate(c) {
            let s = c.to_sdp_string();
            self.trickle(s, out);
        }
    }

    /// The relay candidate of an allocation (`local`: our address towards the server).
    fn add_relayed(&mut self, relayed: SocketAddr, local: SocketAddr, out: &mut Vec<PeerOutput>) {
        self.turn_pending = false;
        self.turn_attempts.clear();
        if let Ok(c) = Candidate::relayed(relayed, local, "udp")
            && let Some(c) = self.rtc.add_local_candidate(c)
        {
            self.relayed = true;
            let s = c.to_sdp_string();
            self.trickle(s, out);
        }
        self.maybe_end_of_candidates(out);
    }

    fn set_remote_described(&mut self) {
        self.remote_described = true;
        for c in std::mem::take(&mut self.early_remote) {
            self.rtc.add_remote_candidate(c);
        }
    }

    /// The joiner's side: the data channel and the offer.
    fn make_offer(&mut self, out: &mut Vec<PeerOutput>) {
        let mut api = self.rtc.sdp_api();
        let id = api.add_channel(DC_LABEL.into());
        let Some((offer, pending)) = api.apply() else {
            return self.fail("could not create the offer");
        };
        self.channel = Some(id);
        self.pending = Some(pending);
        let sdp = offer.to_sdp_string();
        self.mid = mid_of(&sdp);
        out.push(self.signal(StreamSignal::Offer { sdp }));
        self.set_described(out);
    }

    fn on_signal(&mut self, signal: StreamSignal, now: Instant, out: &mut Vec<PeerOutput>) {
        match signal {
            StreamSignal::Offer { sdp } => {
                if self.pending.is_some() || self.remote_described {
                    return; // the joiner offers once; we never renegotiate
                }
                let offer = match SdpOffer::from_sdp_string(&sdp) {
                    Ok(o) => o,
                    Err(e) => return self.fail(format!("bad offer: {e}")),
                };
                match self.rtc.sdp_api().accept_offer(offer) {
                    Ok(answer) => {
                        let sdp = answer.to_sdp_string();
                        self.mid = mid_of(&sdp);
                        self.set_remote_described();
                        out.push(self.signal(StreamSignal::Answer { sdp }));
                        self.set_described(out);
                    }
                    Err(e) => self.fail(format!("offer rejected: {e}")),
                }
            }
            StreamSignal::Answer { sdp } => {
                let Some(pending) = self.pending.take() else {
                    return; // not ours, or a second answer
                };
                let answer = match SdpAnswer::from_sdp_string(&sdp) {
                    Ok(a) => a,
                    Err(e) => return self.fail(format!("bad answer: {e}")),
                };
                match self.rtc.sdp_api().accept_answer(pending, answer) {
                    Ok(()) => self.set_remote_described(),
                    Err(e) => self.fail(format!("answer rejected: {e}")),
                }
            }
            StreamSignal::Ice { candidate } => {
                let s = candidate.candidate.trim();
                let s = s.strip_prefix("a=").unwrap_or(s);
                if s.is_empty() {
                    return; // end of candidates
                }
                // Unparsable (e.g. mDNS `.local` names): ignored; peer-reflexive covers them.
                if let Ok(c) = Candidate::from_sdp_string(s) {
                    if self.remote_described {
                        self.rtc.add_remote_candidate(c);
                    } else if self.early_remote.len() < 64 {
                        self.early_remote.push(c);
                    }
                }
            }
            StreamSignal::Bye { reason } => {
                let _ = self.rtc.close();
                self.fail(reason.unwrap_or_else(|| "the peer ended the connection".into()));
            }
        }
        self.next_timeout = now;
    }

    fn receive(&mut self, input: Input<'_>) {
        if let Err(e) = self.rtc.handle_input(input) {
            tracing::debug!("share input: {e}");
        }
    }

    fn timeout(&mut self, now: Instant) {
        if now >= self.next_timeout
            && let Err(e) = self.rtc.handle_input(Input::Timeout(now))
        {
            self.fail(e.to_string());
        }
    }

    /// Hand queued fragments to SCTP while it has room.
    fn push_fragments(&mut self) {
        let (Some(id), Some(link)) = (self.channel, &self.link) else {
            return;
        };
        let Some(mut ch) = self.rtc.channel(id) else {
            return;
        };
        while let Some(f) = self.outq.front() {
            match ch.write(true, f) {
                Ok(true) => {
                    link.shared.queued.fetch_sub(f.len(), Ordering::AcqRel);
                    self.outq.pop_front();
                }
                Ok(false) => break,
                Err(e) => {
                    self.done = Some(format!("data channel write: {e}"));
                    break;
                }
            }
        }
        let buffered = ch.buffered_amount();
        link.shared.sctp.store(buffered, Ordering::Release);
    }

    /// Poll the `Rtc` until it wants time: send its datagrams, turn its events into outputs.
    fn drive(
        &mut self,
        send: &mut dyn FnMut(SocketAddr, SocketAddr, &[u8]),
        config: &NativeConfig,
        now: Instant,
        out: &mut Vec<PeerOutput>,
        cmd: &Sender<Cmd>,
        waker: &Arc<Waker>,
    ) {
        self.push_fragments();
        loop {
            match self.rtc.poll_output() {
                Ok(Output::Timeout(t)) => {
                    self.next_timeout = t;
                    break;
                }
                Ok(Output::Transmit(t)) => send(t.source, t.destination, &t.contents),
                Ok(Output::Event(e)) => self.on_event(e, now, out, cmd, waker),
                Err(e) => {
                    self.fail(e.to_string());
                    break;
                }
            }
        }
        if self.done.is_none() {
            if !self.rtc.is_alive() {
                self.fail("connection closed");
            } else if let Some(since) = self.disconnected_since
                && self.connected()
                && now.duration_since(since) >= config.disconnect_timeout
            {
                self.fail("the peer stopped answering (ICE disconnected)");
            } else if !self.connected()
                && now.duration_since(self.created) >= config.connect_timeout
            {
                self.fail("could not connect to the peer (ICE/DTLS timed out)");
            }
        }
    }

    fn on_event(
        &mut self,
        e: Event,
        now: Instant,
        out: &mut Vec<PeerOutput>,
        cmd: &Sender<Cmd>,
        waker: &Arc<Waker>,
    ) {
        match e {
            Event::ChannelOpen(id, label) => {
                if label != DC_LABEL || self.link.is_some() {
                    return; // only our channel, once
                }
                self.channel = Some(id);
                if let Some(mut ch) = self.rtc.channel(id) {
                    ch.set_buffered_amount_low_threshold(LOW_WATER);
                }
                let (local, remote) = {
                    let api = self.rtc.direct_api();
                    (
                        api.local_dtls_fingerprint().to_string(),
                        api.remote_dtls_fingerprint().map(|f| f.to_string()),
                    )
                };
                let Some(remote) = remote else {
                    return self.fail("no remote DTLS fingerprint");
                };
                let (tx, rx) = crossbeam_channel::unbounded();
                let shared = LinkShared::new();
                self.link = Some(LinkEnd {
                    inbound: tx,
                    shared: shared.clone(),
                });
                let link = NativeLink {
                    conn: self.conn,
                    cmd: cmd.clone(),
                    inbound: rx,
                    shared,
                    waker: waker.clone(),
                };
                out.push(PeerOutput::Connected {
                    peer: self.peer,
                    link: Box::new(link),
                    local_fingerprint: local,
                    remote_fingerprint: remote,
                });
            }
            Event::ChannelData(d) => {
                if Some(d.id) != self.channel {
                    return;
                }
                let Some(link) = &self.link else { return };
                match self.reasm.push(&d.data) {
                    Ok(Some(frame)) => {
                        let _ = link.inbound.send(frame);
                    }
                    Ok(None) => {}
                    Err(e) => self.fail(format!("bad data channel message: {e}")),
                }
            }
            Event::ChannelBufferedAmountLow(id) if Some(id) == self.channel => {
                self.push_fragments();
            }
            Event::ChannelClose(id) if Some(id) == self.channel => {
                self.fail("the peer closed the data channel");
            }
            Event::IceConnectionStateChange(s) => match s {
                IceConnectionState::Disconnected => {
                    self.disconnected_since.get_or_insert(now);
                }
                IceConnectionState::Connected | IceConnectionState::Completed => {
                    self.disconnected_since = None;
                }
                _ => {}
            },
            _ => {}
        }
    }

    /// Start a clean close (SCTP shutdown, DTLS close_notify, so the peer notices at once)
    /// and send what that produces; events are dropped.
    fn close_and_drain(&mut self, send: &mut dyn FnMut(SocketAddr, SocketAddr, &[u8])) {
        let _ = self.rtc.close();
        for _ in 0..64 {
            match self.rtc.poll_output() {
                Ok(Output::Transmit(t)) => send(t.source, t.destination, &t.contents),
                Ok(Output::Event(_)) => {}
                Ok(Output::Timeout(_)) | Err(_) => break,
            }
        }
    }

    /// Report the end: `Failed` before the channel opened, the link's state after.
    fn finish(&mut self, out: &mut Vec<PeerOutput>) {
        let reason = self.done.clone().unwrap_or_else(|| "closed".into());
        match self.link.take() {
            Some(link) => link.shared.close(reason),
            None => out.push(PeerOutput::Failed {
                peer: self.peer,
                reason,
            }),
        }
    }
}

struct StunQuery {
    tid: stun::TransactionId,
    conn: u64,
    server: SocketAddr,
    base: SocketAddr,
    sent: Instant,
    resent: bool,
}

/// One TURN allocation, shared by the pairings that use (or wait for) its relay candidate.
struct Relay {
    id: u64,
    alloc: turn::Allocation,
    /// TCP/TLS: the connection's I/O thread (UDP uses the share socket).
    stream: Option<StreamConn>,
    /// Pairings whose `Rtc` holds the relay candidate.
    users: Vec<u64>,
    /// Pairings waiting for the allocation.
    waiting: Vec<u64>,
    idle_since: Option<Instant>,
}

/// Hand an allocation's messages to its server.
fn flush_relay(socket: &UdpSocket, r: &mut Relay) {
    while let Some(t) = r.alloc.poll_transmit() {
        match &r.stream {
            Some(s) => s.send(t),
            None => {
                let _ = socket.send_to(&t, r.alloc.server());
            }
        }
    }
}

/// Where an `Rtc` datagram goes: from a relayed address, through its allocation; from one
/// of our host addresses, straight out of the socket; from anything else (an allocation
/// that is gone), nowhere: a relay-only pairing never reveals our address.
fn route(
    socket: &UdpSocket,
    hosts: &[SocketAddr],
    relays: &mut [Relay],
    now: Instant,
    source: SocketAddr,
    destination: SocketAddr,
    data: &[u8],
) {
    if let Some(r) = relays
        .iter_mut()
        .find(|r| r.alloc.relayed() == Some(source))
    {
        r.alloc.send(destination, data, now);
        flush_relay(socket, r);
    } else if hosts.contains(&source) {
        // Unreachable destinations (e.g. IPv6 candidates on our IPv4 socket) fail here:
        // ICE picks another pair.
        let _ = socket.send_to(data, destination);
    }
}

pub(crate) struct Thread {
    config: NativeConfig,
    socket: UdpSocket,
    port: u16,
    lan: Option<Ipv4Addr>,
    cmd_rx: Receiver<Cmd>,
    cmd_tx: Sender<Cmd>,
    out_tx: Sender<PeerOutput>,
    out: Vec<PeerOutput>,
    pairings: Vec<Pairing>,
    next_conn: u64,
    stun: Vec<StunQuery>,
    relays: Vec<Relay>,
    next_relay: u64,
    waker: Arc<Waker>,
    buf: Vec<u8>,
}

impl Thread {
    pub(crate) fn new(
        config: &NativeConfig,
        cmd_rx: Receiver<Cmd>,
        cmd_tx: Sender<Cmd>,
        out_tx: Sender<PeerOutput>,
    ) -> std::io::Result<Self> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        let port = socket.local_addr()?.port();
        let lan = if config.default_route {
            default_route_ip()
        } else {
            None
        };
        Ok(Self {
            config: config.clone(),
            socket,
            port,
            lan,
            cmd_rx,
            cmd_tx,
            out_tx,
            out: Vec::new(),
            pairings: Vec::new(),
            next_conn: 1,
            stun: Vec::new(),
            relays: Vec::new(),
            next_relay: 1,
            waker: Arc::new(Waker::new(port)),
            buf: vec![0; 2048],
        })
    }

    pub(crate) fn port(&self) -> u16 {
        self.port
    }

    pub(crate) fn waker(&self) -> Arc<Waker> {
        self.waker.clone()
    }

    fn host_candidates(&self) -> Vec<SocketAddr> {
        let mut v = Vec::new();
        if let Some(ip) = self.lan {
            v.push(SocketAddr::new(IpAddr::V4(ip), self.port));
        }
        if self.config.loopback {
            v.push(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), self.port));
        }
        v
    }

    /// Every address the socket may send from (host candidates, and the bases of
    /// server-reflexive ones).
    fn host_sources(&self) -> Vec<SocketAddr> {
        let mut v = vec![SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), self.port)];
        if let Some(ip) = self.lan {
            v.push(SocketAddr::new(IpAddr::V4(ip), self.port));
        }
        v
    }

    /// The local candidate address a datagram from `source` arrived at (the socket is bound
    /// to 0.0.0.0 and std has no IP_PKTINFO: loopback traffic arrives on 127.0.0.1,
    /// everything else on the default-route interface).
    fn local_for(&self, source: SocketAddr) -> SocketAddr {
        let ip = match (source.ip().is_loopback(), self.lan) {
            (false, Some(lan)) => lan,
            _ => Ipv4Addr::LOCALHOST,
        };
        SocketAddr::new(IpAddr::V4(ip), self.port)
    }

    pub(crate) fn run(mut self) {
        loop {
            if self.pairings.is_empty() && self.stun.is_empty() && self.relays.is_empty() {
                match self.cmd_rx.recv() {
                    Ok(Cmd::Shutdown) | Err(_) => return self.shutdown(),
                    Ok(cmd) => self.command(cmd),
                }
            }
            loop {
                match self.cmd_rx.try_recv() {
                    Ok(Cmd::Shutdown) | Err(TryRecvError::Disconnected) => {
                        return self.shutdown();
                    }
                    Ok(cmd) => self.command(cmd),
                    Err(TryRecvError::Empty) => break,
                }
            }
            let now = Instant::now();
            self.stun_timeouts(now);
            self.relay_timeouts(now);
            self.drive_all(now);
            self.flush();

            let now = Instant::now();
            let mut deadline = now + IDLE_POLL;
            for p in &self.pairings {
                deadline = deadline.min(p.next_timeout);
            }
            for q in &self.stun {
                deadline = deadline.min(q.sent + STUN_RETRANSMIT);
            }
            for r in &self.relays {
                if let Some(t) = r.alloc.next_timeout() {
                    deadline = deadline.min(t);
                }
            }
            let wait = deadline
                .saturating_duration_since(now)
                .max(Duration::from_millis(1));
            self.waker.sleeping.store(true, Ordering::SeqCst);
            if self.cmd_rx.is_empty() {
                let _ = self.socket.set_read_timeout(Some(wait));
                self.read_burst();
            }
            self.waker.sleeping.store(false, Ordering::SeqCst);
            let now = Instant::now();
            for p in &mut self.pairings {
                p.timeout(now);
            }
        }
    }

    /// One blocking read (bounded by the read timeout), then whatever else is waiting.
    fn read_burst(&mut self) {
        let mut n_read = 0;
        // Ends on the timeout, once drained, or on ICMP port unreachable surfacing as a recv
        // error.
        while let Ok((n, source)) = self.socket.recv_from(&mut self.buf) {
            self.datagram(n, source);
            n_read += 1;
            if n_read == 1 {
                let _ = self.socket.set_nonblocking(true);
            }
            if n_read >= MAX_BURST {
                break;
            }
        }
        if n_read > 0 {
            let _ = self.socket.set_nonblocking(false);
        }
    }

    fn shutdown(&mut self) {
        let hosts = self.host_sources();
        let now = Instant::now();
        for mut p in self.pairings.drain(..) {
            p.close_and_drain(&mut |s, d, b| {
                route(&self.socket, &hosts, &mut self.relays, now, s, d, b)
            });
            if let Some(link) = p.link.take() {
                link.shared.close("sharing stopped");
            }
        }
        for mut r in self.relays.drain(..) {
            r.alloc.release(now);
            flush_relay(&self.socket, &mut r);
        }
    }

    fn flush(&mut self) {
        for o in self.out.drain(..) {
            let _ = self.out_tx.send(o);
        }
    }

    fn command(&mut self, cmd: Cmd) {
        let now = Instant::now();
        match cmd {
            Cmd::Open {
                peer,
                offer,
                ice,
                relay_only,
            } => self.open(peer, offer, ice, relay_only, now),
            Cmd::Signal { peer, signal } => {
                if let Some(p) = self.pairings.iter_mut().find(|p| p.peer == peer) {
                    p.on_signal(signal, now, &mut self.out);
                }
            }
            Cmd::Close { peer } => {
                while let Some(i) = self.pairings.iter().position(|p| p.peer == peer) {
                    self.remove(i, "closed");
                }
            }
            Cmd::Send { conn, fragments } => {
                if let Some(p) = self.pairings.iter_mut().find(|p| p.conn == conn) {
                    p.outq.extend(fragments);
                    p.push_fragments();
                } else {
                    // The pairing is gone: the link is closed; nothing to account for.
                }
            }
            Cmd::CloseLink { conn } => {
                if let Some(i) = self.pairings.iter().position(|p| p.conn == conn) {
                    self.remove(i, "closed");
                }
            }
            Cmd::StunResolved { conn, server } => self.stun_resolved(conn, server, now),
            Cmd::TurnResolved {
                conn,
                url,
                creds,
                server,
            } => self.turn_resolved(conn, url, creds, server, now),
            Cmd::TurnStream { relay, msgs } => {
                if let Some(i) = self.relays.iter().position(|r| r.id == relay) {
                    for m in msgs {
                        self.relay_input(i, &m, now);
                    }
                }
            }
            Cmd::TurnStreamClosed { relay, reason } => {
                if let Some(i) = self.relays.iter().position(|r| r.id == relay) {
                    self.relays[i].alloc.lost(reason);
                    self.relay_events(i, now);
                }
            }
            Cmd::Shutdown => {}
        }
    }

    /// Drop pairing `i` without an output (the owner closed it).
    fn remove(&mut self, i: usize, reason: &str) {
        let mut p = self.pairings.remove(i);
        let hosts = self.host_sources();
        let now = Instant::now();
        p.close_and_drain(&mut |s, d, b| {
            route(&self.socket, &hosts, &mut self.relays, now, s, d, b)
        });
        if let Some(link) = p.link.take() {
            link.shared.close(reason);
        }
        self.forget(p.conn);
    }

    /// A pairing is gone: its STUN queries and its hold on allocations.
    fn forget(&mut self, conn: u64) {
        self.stun.retain(|q| q.conn != conn);
        for r in &mut self.relays {
            r.users.retain(|c| *c != conn);
            r.waiting.retain(|c| *c != conn);
        }
    }

    fn open(
        &mut self,
        peer: PeerId,
        offer: bool,
        ice: Vec<IceServer>,
        relay_only: bool,
        now: Instant,
    ) {
        // Re-opening a peer replaces its pairing.
        while let Some(i) = self.pairings.iter().position(|p| p.peer == peer) {
            self.remove(i, "replaced");
        }
        if self.pairings.len() >= MAX_PAIRINGS {
            self.out.push(PeerOutput::Failed {
                peer,
                reason: "too many peer connections".into(),
            });
            return;
        }
        let turns: Vec<(TurnUrl, Credentials)> = ice
            .iter()
            .filter_map(|s| {
                let creds = Credentials {
                    username: s.username.clone()?,
                    password: s.credential.clone()?,
                };
                Some(
                    s.urls
                        .iter()
                        .filter_map(|u| turn::parse_turn_url(u))
                        .map(move |u| (u, creds.clone())),
                )
            })
            .flatten()
            .collect();
        let turns = turn::attempt_order(turns, MAX_TURN_ATTEMPTS);
        if relay_only && turns.is_empty() {
            self.out.push(PeerOutput::Failed {
                peer,
                reason: RELAY_ONLY_NO_TURN.into(),
            });
            return;
        }
        let conn = self.next_conn;
        self.next_conn += 1;
        let hosts = if relay_only {
            Vec::new()
        } else {
            self.host_candidates()
        };
        let mut p = Pairing::new(peer, conn, &hosts, now);
        p.relay_only = relay_only;
        p.turn_pending = !turns.is_empty();
        p.turn_attempts = turns.into();
        let urls: Vec<String> = if relay_only {
            Vec::new() // a server-reflexive candidate is our public address
        } else {
            ice.iter()
                .flat_map(|s| s.urls.iter())
                .filter(|u| stun::parse_stun_url(u).is_some())
                .take(MAX_STUN_URLS)
                .cloned()
                .collect()
        };
        p.stun_pending = urls.len();
        if !urls.is_empty() {
            // DNS may block: resolve on a short-lived thread, report through the commands.
            let tx = self.cmd_tx.clone();
            let waker = self.waker.clone();
            let spawned = std::thread::Builder::new()
                .name("ether-share-stun".into())
                .spawn(move || {
                    for u in urls {
                        let server = stun::resolve_stun_url(&u);
                        let _ = tx.send(Cmd::StunResolved { conn, server });
                        waker.wake();
                    }
                });
            if spawned.is_err() {
                p.stun_pending = 0;
            }
        }
        if offer {
            p.make_offer(&mut self.out);
        }
        let try_turn = p.turn_pending;
        self.pairings.push(p);
        if try_turn {
            self.next_turn(conn, now);
        }
    }

    /// Try pairing `conn`'s next TURN server, or give up on a relay candidate.
    fn next_turn(&mut self, conn: u64, now: Instant) {
        let Some(p) = self.pairings.iter_mut().find(|p| p.conn == conn) else {
            return;
        };
        let Some((url, creds)) = p.turn_attempts.pop_front() else {
            p.turn_pending = false;
            if p.relay_only && !p.relayed {
                let why = p.turn_error.clone().unwrap_or_default();
                p.fail(format!("could not reach the TURN relay: {why}"));
            }
            p.maybe_end_of_candidates(&mut self.out);
            return;
        };
        if let Ok(ip) = url.host.parse::<IpAddr>() {
            let server = Some(SocketAddr::new(ip, url.port)).filter(SocketAddr::is_ipv4);
            return self.turn_resolved(conn, url, creds, server, now);
        }
        // DNS may block: resolve on a short-lived thread, report through the commands.
        let tx = self.cmd_tx.clone();
        let waker = self.waker.clone();
        let spawned = std::thread::Builder::new()
            .name("ether-share-turn-dns".into())
            .spawn(move || {
                let server = url.resolve();
                let _ = tx.send(Cmd::TurnResolved {
                    conn,
                    url,
                    creds,
                    server,
                });
                waker.wake();
            });
        if spawned.is_err() {
            self.next_turn(conn, now);
        }
    }

    fn turn_failed(&mut self, conn: u64, reason: String, now: Instant) {
        if let Some(p) = self.pairings.iter_mut().find(|p| p.conn == conn) {
            p.turn_error = Some(reason);
            self.next_turn(conn, now);
        }
    }

    fn turn_resolved(
        &mut self,
        conn: u64,
        url: TurnUrl,
        creds: Credentials,
        server: Option<SocketAddr>,
        now: Instant,
    ) {
        if !self.pairings.iter().any(|p| p.conn == conn) {
            return;
        }
        let Some(server) = server else {
            return self.turn_failed(conn, format!("could not resolve {}", url.host), now);
        };
        // One allocation per server, shared.
        if let Some(i) = self.relays.iter().position(|r| {
            !r.alloc.is_failed()
                && r.alloc.server() == server
                && r.alloc.transport() == url.transport
        }) {
            let r = &mut self.relays[i];
            r.idle_since = None;
            match r.alloc.relayed() {
                Some(relayed) => {
                    r.users.push(conn);
                    let local = self.local_for(server);
                    if let Some(p) = self.pairings.iter_mut().find(|p| p.conn == conn) {
                        p.add_relayed(relayed, local, &mut self.out);
                    }
                }
                None => r.waiting.push(conn),
            }
            return;
        }
        if self.relays.len() >= MAX_RELAYS {
            return self.turn_failed(conn, "too many TURN allocations".into(), now);
        }
        let id = self.next_relay;
        self.next_relay += 1;
        let stream = if url.transport.is_stream() {
            match StreamConn::open(
                id,
                server,
                url.transport,
                url.host.clone(),
                self.config.extra_tls_roots.clone(),
                self.cmd_tx.clone(),
                self.waker.clone(),
            ) {
                Ok(s) => Some(s),
                Err(e) => return self.turn_failed(conn, e.to_string(), now),
            }
        } else {
            None
        };
        let mut r = Relay {
            id,
            alloc: turn::Allocation::new(server, url.transport, creds, now),
            stream,
            users: Vec::new(),
            waiting: vec![conn],
            idle_since: None,
        };
        flush_relay(&self.socket, &mut r);
        self.relays.push(r);
    }

    /// A message from relay `i`'s server.
    fn relay_input(&mut self, i: usize, msg: &[u8], now: Instant) {
        let r = &mut self.relays[i];
        if let Received::Data(peer, data) = r.alloc.handle_input(msg, now)
            && let Some(relayed) = r.alloc.relayed()
            && let Ok(contents) = data.as_slice().try_into()
        {
            let input = Input::Receive(
                now,
                Receive {
                    proto: Protocol::Udp,
                    source: peer,
                    destination: relayed,
                    contents,
                },
            );
            let users = &r.users;
            if let Some(p) = self
                .pairings
                .iter_mut()
                .filter(|p| users.contains(&p.conn))
                .find(|p| p.rtc.accepts(&input))
            {
                p.receive(input);
            }
        }
        flush_relay(&self.socket, &mut self.relays[i]);
        self.relay_events(i, now);
    }

    /// Hand relay `i`'s events to the pairings that wait for it.
    fn relay_events(&mut self, i: usize, now: Instant) {
        while let Some(e) = self.relays[i].alloc.poll_event() {
            match e {
                turn::Event::Allocated { relayed, mapped } => {
                    let r = &mut self.relays[i];
                    let waiting = std::mem::take(&mut r.waiting);
                    r.users.extend(&waiting);
                    let server = r.alloc.server();
                    let udp = r.alloc.transport() == Transport::Udp;
                    let local = self.local_for(server);
                    for conn in waiting {
                        let Some(p) = self.pairings.iter_mut().find(|p| p.conn == conn) else {
                            continue;
                        };
                        // Over UDP the server saw our socket's public address: a free
                        // server-reflexive candidate (not for "Hide my IP").
                        if udp
                            && !p.relay_only
                            && let Some(mapped) = mapped
                        {
                            p.add_srflx(mapped, local, &mut self.out);
                        }
                        p.add_relayed(relayed, local, &mut self.out);
                    }
                }
                turn::Event::Failed(reason) => {
                    tracing::debug!("share TURN allocation failed: {reason}");
                    let waiting = std::mem::take(&mut self.relays[i].waiting);
                    for conn in waiting {
                        self.turn_failed(conn, reason.clone(), now);
                    }
                }
            }
        }
    }

    /// Relay retransmissions and refreshes; failed and idle allocations go away.
    fn relay_timeouts(&mut self, now: Instant) {
        for i in 0..self.relays.len() {
            let r = &mut self.relays[i];
            if r.alloc.next_timeout().is_some_and(|t| now >= t) {
                r.alloc.handle_timeout(now);
                flush_relay(&self.socket, r);
                self.relay_events(i, now);
            }
        }
        let socket = &self.socket;
        self.relays.retain_mut(|r| {
            if r.alloc.is_failed() {
                return false;
            }
            if r.users.is_empty() && r.waiting.is_empty() {
                let since = *r.idle_since.get_or_insert(now);
                if now.duration_since(since) >= RELAY_IDLE {
                    r.alloc.release(now);
                    flush_relay(socket, r);
                    return false;
                }
            } else {
                r.idle_since = None;
            }
            true
        });
    }

    fn stun_resolved(&mut self, conn: u64, server: Option<SocketAddr>, now: Instant) {
        let base = server.map(|s| self.local_for(s));
        let full = self.stun.len() >= MAX_STUN_QUERIES;
        let Some(p) = self.pairings.iter_mut().find(|p| p.conn == conn) else {
            return;
        };
        let (Some(server), Some(base), false) = (server, base, full) else {
            p.stun_pending = p.stun_pending.saturating_sub(1);
            p.maybe_end_of_candidates(&mut self.out);
            return;
        };
        let mut tid = [0u8; 12];
        let _ = getrandom::fill(&mut tid);
        let _ = self.socket.send_to(&stun::binding_request(&tid), server);
        self.stun.push(StunQuery {
            tid,
            conn,
            server,
            base,
            sent: now,
            resent: false,
        });
    }

    fn stun_timeouts(&mut self, now: Instant) {
        let mut i = 0;
        while i < self.stun.len() {
            let q = &mut self.stun[i];
            let age = now.duration_since(q.sent);
            if age >= STUN_GIVE_UP {
                let conn = q.conn;
                self.stun.swap_remove(i);
                if let Some(p) = self.pairings.iter_mut().find(|p| p.conn == conn) {
                    p.stun_pending = p.stun_pending.saturating_sub(1);
                    p.maybe_end_of_candidates(&mut self.out);
                }
                continue;
            }
            if age >= STUN_RETRANSMIT && !q.resent {
                q.resent = true;
                let _ = self
                    .socket
                    .send_to(&stun::binding_request(&q.tid), q.server);
            }
            i += 1;
        }
    }

    fn datagram(&mut self, n: usize, source: SocketAddr) {
        let now = Instant::now();
        let data = &self.buf[..n];
        if data == WAKE && source.ip().is_loopback() {
            return;
        }
        // Our own STUN transactions first (str0m never sees them).
        if let Some(tid) = stun::binding_response_tid(data)
            && let Some(i) = self.stun.iter().position(|q| q.tid == tid)
        {
            let q = self.stun.swap_remove(i);
            let mapped = stun::parse_binding_response(data).map(|(_, a)| a);
            if let Some(p) = self.pairings.iter_mut().find(|p| p.conn == q.conn) {
                if let Some(mapped) = mapped {
                    p.add_srflx(mapped, q.base, &mut self.out);
                }
                p.stun_pending = p.stun_pending.saturating_sub(1);
                p.maybe_end_of_candidates(&mut self.out);
            }
            return;
        }
        // A UDP TURN server talks to its allocation.
        if let Some(i) = self
            .relays
            .iter()
            .position(|r| r.stream.is_none() && r.alloc.server() == source)
        {
            let msg = data.to_vec();
            return self.relay_input(i, &msg, now);
        }
        let destination = self.local_for(source);
        let Ok(contents) = data.try_into() else {
            return;
        };
        let input = Input::Receive(
            now,
            Receive {
                proto: Protocol::Udp,
                source,
                destination,
                contents,
            },
        );
        // A relay-only pairing takes nothing that bypassed its relay.
        if let Some(p) = self
            .pairings
            .iter_mut()
            .filter(|p| !p.relay_only)
            .find(|p| p.rtc.accepts(&input))
        {
            p.receive(input);
        }
    }

    fn drive_all(&mut self, now: Instant) {
        let hosts = self.host_sources();
        for p in &mut self.pairings {
            let (socket, relays) = (&self.socket, &mut self.relays);
            p.drive(
                &mut |s, d, b| route(socket, &hosts, relays, now, s, d, b),
                &self.config,
                now,
                &mut self.out,
                &self.cmd_tx,
                &self.waker,
            );
        }
        let mut i = 0;
        while i < self.pairings.len() {
            if self.pairings[i].done.is_some() {
                let mut p = self.pairings.remove(i);
                p.close_and_drain(&mut |s, d, b| {
                    route(&self.socket, &hosts, &mut self.relays, now, s, d, b)
                });
                p.finish(&mut self.out);
                self.forget(p.conn);
            } else {
                i += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mids() {
        assert_eq!(
            mid_of("v=0\r\nm=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\na=mid:0\r\n"),
            Some("0".into())
        );
        assert_eq!(mid_of("v=0\r\n"), None);
    }

    /// The offer str0m makes for the joiner is a data-channel offer browsers accept.
    #[test]
    fn joiner_offer_shape() {
        let now = Instant::now();
        let mut p = Pairing::new(1, 1, &["127.0.0.1:5000".parse().unwrap()], now);
        let mut out = Vec::new();
        p.make_offer(&mut out);
        let PeerOutput::Signal {
            signal: StreamSignal::Offer { sdp },
            ..
        } = &out[0]
        else {
            panic!("an offer first");
        };
        assert!(sdp.contains("m=application"), "{sdp}");
        assert!(sdp.contains("webrtc-datachannel"), "{sdp}");
        assert!(sdp.contains("a=fingerprint:sha-256 "), "{sdp}");
        assert!(sdp.contains("127.0.0.1 5000 typ host"), "{sdp}");
        // No STUN servers: end of candidates right away.
        assert!(matches!(
            &out[1],
            PeerOutput::Signal { signal: StreamSignal::Ice { candidate }, .. } if candidate.candidate.is_empty()
        ));
    }
}

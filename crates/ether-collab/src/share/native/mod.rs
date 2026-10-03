//! Native [`PeerEndpoint`]: WebRTC data channels with str0m (docs/SHARING.md §6.1).
//!
//! One `ether-share` thread owns one UDP socket and every pairing's `Rtc` (sans-IO, like
//! the listen-on-peer sender in `ether-native/src/stream`). The endpoint and its links talk
//! to it over channels and never block:
//! - [`NativePeers::open`] creates a pairing: the joiner (`offer`) adds the data channel
//!   ([`DC_LABEL`](crate::share::dc::DC_LABEL)) and sends the offer; the host waits for it
//!   and answers. Host candidates (loopback and the default-route interface) are in the SDP;
//!   one server-reflexive candidate per `stun:` URL (at most two) is trickled, then the
//!   end-of-candidates marker. TURN URLs are ignored (str0m has no TURN client, §11).
//! - When the channel opens: [`PeerOutput::Connected`] with a [`NativeLink`] and both DTLS
//!   fingerprints (`sha-256 AB:CD:...`, as in SDP).
//! - No channel within [`NativeConfig::connect_timeout`] (30 s): [`PeerOutput::Failed`].
//!   A connected link whose ICE stays disconnected [`NativeConfig::disconnect_timeout`]
//!   (consent freshness), whose channel closes, or that receives a bad fragment closes
//!   (its [`PeerLink::state`] becomes `Closed`).
//!
//! - "Hide my IP (relay only)" ([`PeerEndpoint::set_relay_only`]): this endpoint has no
//!   TURN client, so with it on every `open` fails at once ([`RELAY_ONLY_UNSUPPORTED`])
//!   rather than leaking host and srflx candidates.
//!
//! The thread starts on the first `open` (a build that never shares spawns nothing) and
//! stops when the endpoint is dropped.

mod stun;
mod thread;

use std::collections::VecDeque;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use ether_protocol::collab::{IceServer, StreamSignal};
use ether_protocol::share::PeerId;

use super::{PeerEndpoint, PeerLink, PeerOutput, dc};
use crate::LinkState;
use crate::wire::WireFrame;

/// Why a pairing fails at once while "Hide my IP (relay only)" is on.
pub const RELAY_ONLY_UNSUPPORTED: &str = "\"Hide my IP\" needs a TURN relay, which this desktop build cannot use yet: turn it off to connect";

/// What the endpoint advertises and how long it waits.
#[derive(Clone, Debug)]
pub struct NativeConfig {
    /// Advertise `127.0.0.1` (peers on the same machine; tests).
    pub loopback: bool,
    /// Advertise the default-route interface address.
    pub default_route: bool,
    /// A pairing whose data channel is not open this long after `open` has failed.
    pub connect_timeout: Duration,
    /// A connected link whose ICE stays disconnected this long is closed.
    pub disconnect_timeout: Duration,
}

impl Default for NativeConfig {
    fn default() -> Self {
        Self {
            loopback: true,
            default_route: true,
            connect_timeout: Duration::from_secs(30),
            disconnect_timeout: Duration::from_secs(8),
        }
    }
}

pub(crate) enum Cmd {
    Open {
        peer: PeerId,
        offer: bool,
        ice: Vec<IceServer>,
    },
    Signal {
        peer: PeerId,
        signal: StreamSignal,
    },
    Close {
        peer: PeerId,
    },
    /// Fragments of one frame for link `conn`.
    Send {
        conn: u64,
        fragments: Vec<Vec<u8>>,
    },
    CloseLink {
        conn: u64,
    },
    StunResolved {
        conn: u64,
        server: Option<SocketAddr>,
    },
    Shutdown,
}

/// Wakes the share thread out of its socket read (a tiny datagram to its own port) when a
/// command was queued while it was sleeping. The thread sets `sleeping`, then checks the
/// command channel before it blocks; a sender queues, then swaps `sleeping` off: one of the
/// two always sees the other.
pub(crate) struct Waker {
    socket: Option<UdpSocket>,
    target: SocketAddr,
    pub(crate) sleeping: AtomicBool,
}

/// The wake datagram: never STUN, DTLS or RTP (RFC 7983 first byte 101), so no `Rtc`
/// accepts it.
pub(crate) const WAKE: &[u8] = b"ewak";

impl Waker {
    pub(crate) fn new(port: u16) -> Self {
        Self {
            socket: UdpSocket::bind("127.0.0.1:0").ok(),
            target: SocketAddr::from(([127, 0, 0, 1], port)),
            sleeping: AtomicBool::new(false),
        }
    }

    pub(crate) fn wake(&self) {
        if self.sleeping.swap(false, Ordering::SeqCst)
            && let Some(s) = &self.socket
        {
            let _ = s.send_to(WAKE, self.target);
        }
    }
}

/// State shared by a [`NativeLink`] and the share thread.
pub(crate) struct LinkShared {
    /// Bytes queued by `send` and not yet handed to SCTP.
    pub(crate) queued: AtomicUsize,
    /// SCTP's send buffer for the channel (written by the thread).
    pub(crate) sctp: AtomicUsize,
    pub(crate) state: Mutex<LinkState>,
}

impl LinkShared {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            queued: AtomicUsize::new(0),
            sctp: AtomicUsize::new(0),
            state: Mutex::new(LinkState::Open),
        })
    }

    pub(crate) fn close(&self, reason: impl Into<String>) {
        let mut s = self.state.lock().expect("link state lock");
        if !matches!(*s, LinkState::Closed { .. }) {
            *s = LinkState::Closed {
                reason: reason.into(),
                fatal: false,
            };
        }
    }

    fn is_open(&self) -> bool {
        matches!(
            *self.state.lock().expect("link state lock"),
            LinkState::Open
        )
    }
}

/// An open data channel (see [`PeerLink`]).
pub struct NativeLink {
    conn: u64,
    cmd: Sender<Cmd>,
    inbound: Receiver<WireFrame>,
    shared: Arc<LinkShared>,
    waker: Arc<Waker>,
}

impl PeerLink for NativeLink {
    fn send(&mut self, frame: &WireFrame) {
        if !self.shared.is_open() {
            return;
        }
        let fragments = dc::fragment(frame);
        let bytes: usize = fragments.iter().map(Vec::len).sum();
        self.shared.queued.fetch_add(bytes, Ordering::AcqRel);
        if self
            .cmd
            .send(Cmd::Send {
                conn: self.conn,
                fragments,
            })
            .is_err()
        {
            self.shared.queued.fetch_sub(bytes, Ordering::AcqRel);
            self.shared.close("the share thread stopped");
            return;
        }
        self.waker.wake();
    }

    fn poll(&mut self, out: &mut Vec<WireFrame>) {
        out.extend(self.inbound.try_iter());
    }

    fn buffered(&self) -> usize {
        self.shared.queued.load(Ordering::Acquire) + self.shared.sctp.load(Ordering::Acquire)
    }

    fn state(&self) -> LinkState {
        self.shared.state.lock().expect("link state lock").clone()
    }

    fn close(&mut self) {
        if self.shared.is_open() {
            self.shared.close("closed");
            let _ = self.cmd.send(Cmd::CloseLink { conn: self.conn });
            self.waker.wake();
        }
    }
}

impl Drop for NativeLink {
    fn drop(&mut self) {
        self.close();
    }
}

struct Running {
    cmd: Sender<Cmd>,
    out: Receiver<PeerOutput>,
    waker: Arc<Waker>,
    port: u16,
    thread: Option<JoinHandle<()>>,
}

/// The native [`PeerEndpoint`] (see the module docs).
pub struct NativePeers {
    config: NativeConfig,
    relay_only: bool,
    running: Option<Running>,
    /// Failures produced without the thread (it could not start).
    failed: VecDeque<PeerOutput>,
}

impl NativePeers {
    pub fn new(config: NativeConfig) -> Self {
        Self {
            config,
            relay_only: false,
            running: None,
            failed: VecDeque::new(),
        }
    }

    /// The UDP port of the share socket (`None` before the first `open`).
    pub fn port(&self) -> Option<u16> {
        self.running.as_ref().map(|r| r.port)
    }

    fn running(&mut self) -> Result<&Running, String> {
        if self.running.is_none() {
            let (cmd_tx, cmd_rx) = crossbeam_channel::unbounded();
            let (out_tx, out_rx) = crossbeam_channel::unbounded();
            let t = thread::Thread::new(&self.config, cmd_rx, cmd_tx.clone(), out_tx)
                .map_err(|e| format!("could not open the sharing socket: {e}"))?;
            let port = t.port();
            let waker = t.waker();
            let handle = std::thread::Builder::new()
                .name("ether-share".into())
                .spawn(move || t.run())
                .map_err(|e| format!("could not start the sharing thread: {e}"))?;
            self.running = Some(Running {
                cmd: cmd_tx,
                out: out_rx,
                waker,
                port,
                thread: Some(handle),
            });
        }
        Ok(self.running.as_ref().expect("just started"))
    }

    fn command(&self, cmd: Cmd) {
        if let Some(r) = &self.running {
            let _ = r.cmd.send(cmd);
            r.waker.wake();
        }
    }
}

impl Default for NativePeers {
    fn default() -> Self {
        Self::new(NativeConfig::default())
    }
}

impl PeerEndpoint for NativePeers {
    fn open(&mut self, peer: PeerId, offer: bool, ice_servers: &[IceServer]) {
        if self.relay_only {
            self.failed.push_back(PeerOutput::Failed {
                peer,
                reason: RELAY_ONLY_UNSUPPORTED.into(),
            });
            return;
        }
        match self.running() {
            Ok(_) => self.command(Cmd::Open {
                peer,
                offer,
                ice: ice_servers.to_vec(),
            }),
            Err(reason) => self.failed.push_back(PeerOutput::Failed { peer, reason }),
        }
    }

    fn signal(&mut self, peer: PeerId, signal: StreamSignal) {
        self.command(Cmd::Signal { peer, signal });
    }

    fn poll(&mut self, out: &mut Vec<PeerOutput>) {
        out.extend(self.failed.drain(..));
        if let Some(r) = &self.running {
            out.extend(r.out.try_iter());
        }
    }

    fn close(&mut self, peer: PeerId) {
        self.command(Cmd::Close { peer });
    }

    fn set_relay_only(&mut self, relay_only: bool) {
        self.relay_only = relay_only;
    }
}

impl Drop for NativePeers {
    fn drop(&mut self) {
        if let Some(mut r) = self.running.take() {
            let _ = r.cmd.send(Cmd::Shutdown);
            r.waker.wake();
            if let Some(t) = r.thread.take() {
                let _ = t.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn candidates(out: &[PeerOutput]) -> Vec<String> {
        out.iter()
            .filter_map(|o| match o {
                PeerOutput::Signal {
                    signal: StreamSignal::Ice { candidate },
                    ..
                } => Some(candidate.candidate.clone()),
                _ => None,
            })
            .collect()
    }

    fn config() -> NativeConfig {
        NativeConfig {
            default_route: false,
            ..NativeConfig::default()
        }
    }

    /// One `stun:` server (a loopback responder) gives one trickled srflx candidate, then
    /// the end-of-candidates marker; TURN URLs are ignored.
    #[test]
    fn srflx_candidates_trickle_after_the_offer() {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        let responder = std::thread::spawn(move || {
            let mut buf = [0u8; 512];
            let (n, from) = server.recv_from(&mut buf).unwrap();
            let tid = stun::binding_request_tid(&buf[..n]).expect("a binding request");
            let mapped: SocketAddr = "203.0.113.5:4242".parse().unwrap();
            server
                .send_to(&stun::binding_response(&tid, mapped), from)
                .unwrap();
        });
        let mut ep = NativePeers::new(config());
        assert!(ep.port().is_none(), "no thread before the first open");
        ep.open(
            3,
            true,
            &[IceServer {
                urls: vec![
                    format!("stun:127.0.0.1:{}", addr.port()),
                    "turn:turn.example:3478".into(),
                ],
                username: Some("u".into()),
                credential: Some("p".into()),
            }],
        );
        assert!(ep.port().is_some());
        let mut out = Vec::new();
        let t = Instant::now();
        while !candidates(&out).iter().any(String::is_empty) {
            assert!(t.elapsed() < Duration::from_secs(5), "no end of candidates");
            ep.poll(&mut out);
            std::thread::sleep(Duration::from_millis(2));
        }
        responder.join().unwrap();
        assert!(matches!(
            &out[0],
            PeerOutput::Signal {
                peer: 3,
                signal: StreamSignal::Offer { .. }
            }
        ));
        let c = candidates(&out);
        assert_eq!(c.len(), 2, "{c:?}");
        assert!(c[0].contains("203.0.113.5 4242 typ srflx"), "{c:?}");
        assert!(c[1].is_empty());
    }

    /// The host holds its candidates until its answer is out (browsers reject candidates
    /// before the remote description).
    #[test]
    fn the_host_answers_before_it_trickles() {
        let mut joiner = NativePeers::new(config());
        let mut host = NativePeers::new(config());
        host.open(1, false, &[]);
        joiner.open(1, true, &[]);
        let mut out = Vec::new();
        let t = Instant::now();
        let offer = loop {
            joiner.poll(&mut out);
            if let Some(sdp) = out.iter().find_map(|o| match o {
                PeerOutput::Signal {
                    signal: StreamSignal::Offer { sdp },
                    ..
                } => Some(sdp.clone()),
                _ => None,
            }) {
                break sdp;
            }
            assert!(t.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(2));
        };
        let mut hout = Vec::new();
        std::thread::sleep(Duration::from_millis(20));
        host.poll(&mut hout);
        assert!(hout.is_empty(), "nothing before the offer");
        host.signal(1, StreamSignal::Offer { sdp: offer });
        let t = Instant::now();
        while hout.len() < 2 {
            host.poll(&mut hout);
            assert!(t.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(matches!(
            &hout[0],
            PeerOutput::Signal { signal: StreamSignal::Answer { sdp }, .. } if sdp.contains("127.0.0.1")
        ));
        assert_eq!(candidates(&hout), vec![String::new()]);
    }

    #[test]
    fn relay_only_fails_at_once_without_a_socket() {
        let mut ep = NativePeers::new(config());
        ep.set_relay_only(true);
        ep.open(4, true, &[]);
        let mut out = Vec::new();
        ep.poll(&mut out);
        assert!(matches!(
            out.as_slice(),
            [PeerOutput::Failed { peer: 4, reason }] if reason == RELAY_ONLY_UNSUPPORTED
        ));
        assert!(ep.port().is_none(), "nothing was opened");
        ep.set_relay_only(false);
        ep.open(4, true, &[]);
        assert!(ep.port().is_some());
    }
}

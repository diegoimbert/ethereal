//! The sender thread (docs/COLLAB.md §9.1): tap ring → 48 kHz → 20 ms Opus frames → one
//! str0m `Rtc` per listener, all on one UDP socket (sans-IO: this thread owns the socket
//! and the clock). A normal thread, never the audio thread: it may allocate and block.
//!
//! Loop: commands (bounded channel) → tap blocks (anchors) and audio (resample, frame,
//! encode, write to every connected listener) → one socket read with a timeout (≤ 5 ms
//! while capturing, so the ring is drained promptly; the next `Rtc`/STUN deadline
//! otherwise) → STUN responses are demuxed by transaction id, everything else goes to the
//! `Rtc` that `accepts` it → due `Rtc` timeouts → poll every `Rtc` until it wants time.
//! Idle (no capture, no listener): blocks on the command channel.

use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, TryRecvError};
use ether_controller::streaming::{StreamLinkState, StreamOutput};
use ether_core::protocol::collab::{IceServer, StreamClock, StreamSignal};
use ether_core::protocol::model::{BeatRange, Beats, SiteId};
use ether_core::stream_tap::StreamTapReader;
use str0m::Input;
use str0m::net::{Protocol, Receive};

use super::StreamTransport;
use super::anchors::{ANCHOR_INTERVAL_SAMPLES, BlockClock, ClockTracker};
use super::peer::{DEFAULT_BITRATE, MAX_BITRATE, MIN_BITRATE, Peer};
use super::resample::{FRAME, Framer};
use super::stun;

/// Defensive cap (the controller enforces `MAX_LISTENERS` = 8).
pub const MAX_PEERS: usize = 8;
/// A STUN Binding request is retransmitted once after this, and given up after
/// [`STUN_GIVE_UP`].
const STUN_RETRANSMIT: Duration = Duration::from_millis(500);
const STUN_GIVE_UP: Duration = Duration::from_secs(2);
/// At most this many `stun:` URLs are queried per listener.
const MAX_STUN_URLS: usize = 2;
const MAX_STUN_QUERIES: usize = 32;
/// Socket read timeout while capturing (tap polling period).
const CAPTURE_POLL: Duration = Duration::from_millis(5);
const IDLE_POLL: Duration = Duration::from_millis(20);

pub(crate) enum Cmd {
    StartCapture {
        reader: StreamTapReader,
        sample_rate: u32,
    },
    StopCapture,
    Open {
        listener: SiteId,
        stream: u32,
        ice: Vec<IceServer>,
    },
    Signal {
        listener: SiteId,
        stream: u32,
        signal: StreamSignal,
    },
    Close {
        listener: SiteId,
        stream: u32,
    },
    Transport(StreamTransport),
    StunResolved {
        listener: SiteId,
        stream: u32,
        server: Option<SocketAddr>,
    },
    Shutdown,
}

/// Which local addresses become host candidates.
#[derive(Clone, Debug)]
pub struct SenderConfig {
    /// Advertise `127.0.0.1` (listeners on the same machine; tests).
    pub loopback: bool,
    /// Advertise the default-route interface address (found by "connecting" a UDP socket
    /// to a public address: no packet is sent, no extra dependency).
    pub default_route: bool,
}

impl Default for SenderConfig {
    fn default() -> Self {
        Self {
            loopback: true,
            default_route: true,
        }
    }
}

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

struct Capture {
    reader: StreamTapReader,
    tracker: ClockTracker,
    framer: Framer,
}

struct StunQuery {
    tid: stun::TransactionId,
    key: (SiteId, u32),
    server: SocketAddr,
    base: SocketAddr,
    sent: Instant,
    resent: bool,
}

pub(crate) struct Thread {
    socket: UdpSocket,
    port: u16,
    loopback: bool,
    lan: Option<Ipv4Addr>,
    cmd_rx: Receiver<Cmd>,
    cmd_tx: Sender<Cmd>,
    out_tx: Sender<StreamOutput>,
    out: Vec<StreamOutput>,
    peers: Vec<Peer>,
    capture: Option<Capture>,
    /// Stream index where the next capture starts (the stream clock never goes back).
    n_next: u64,
    transport: StreamTransport,
    transport_changed: bool,
    encoder: opus::Encoder,
    /// Encoder look-ahead (samples at 48 kHz): compensated in the RTP timestamps.
    lookahead: u64,
    bitrate: u64,
    stun: Vec<StunQuery>,
    frame: Vec<f32>,
    packet: Vec<u8>,
    scratch: Vec<f32>,
    buf: Vec<u8>,
}

impl Thread {
    pub(crate) fn new(
        config: &SenderConfig,
        cmd_rx: Receiver<Cmd>,
        cmd_tx: Sender<Cmd>,
        out_tx: Sender<StreamOutput>,
    ) -> std::io::Result<Self> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        let port = socket.local_addr()?.port();
        let lan = if config.default_route {
            default_route_ip()
        } else {
            None
        };
        let mut encoder =
            opus::Encoder::new(48_000, opus::Channels::Stereo, opus::Application::Audio)
                .map_err(|e| std::io::Error::other(e.to_string()))?;
        let _ = encoder.set_bitrate(opus::Bitrate::Bits(DEFAULT_BITRATE as i32));
        let _ = encoder.set_vbr(true);
        let _ = encoder.set_inband_fec(true);
        // In-band FEC only kicks in with an expected loss > 0.
        let _ = encoder.set_packet_loss_perc(5);
        let _ = encoder.set_dtx(false);
        let _ = encoder.set_signal(opus::Signal::Music);
        let lookahead = encoder.get_lookahead().unwrap_or(0).max(0) as u64;
        Ok(Self {
            socket,
            port,
            loopback: config.loopback,
            lan,
            cmd_rx,
            cmd_tx,
            out_tx,
            out: Vec::new(),
            peers: Vec::new(),
            capture: None,
            n_next: 0,
            transport: StreamTransport::default(),
            transport_changed: false,
            encoder,
            lookahead,
            bitrate: DEFAULT_BITRATE,
            stun: Vec::new(),
            frame: vec![0.0; 2 * FRAME],
            packet: vec![0; 4000],
            scratch: Vec::new(),
            buf: vec![0; 2048],
        })
    }

    pub(crate) fn port(&self) -> u16 {
        self.port
    }

    fn host_candidates(&self) -> Vec<SocketAddr> {
        let mut v = Vec::new();
        if let Some(ip) = self.lan {
            v.push(SocketAddr::new(IpAddr::V4(ip), self.port));
        }
        if self.loopback {
            v.push(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), self.port));
        }
        v
    }

    /// The local candidate address a datagram from `source` arrived at (our socket is bound
    /// to 0.0.0.0, and std has no IP_PKTINFO: loopback traffic arrives on 127.0.0.1,
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
            // Idle: nothing to drive, sleep on the command channel.
            if self.peers.is_empty() && self.capture.is_none() && self.stun.is_empty() {
                match self.cmd_rx.recv() {
                    Ok(Cmd::Shutdown) | Err(_) => return,
                    Ok(cmd) => self.command(cmd),
                }
            }
            loop {
                match self.cmd_rx.try_recv() {
                    Ok(Cmd::Shutdown) | Err(TryRecvError::Disconnected) => return,
                    Ok(cmd) => self.command(cmd),
                    Err(TryRecvError::Empty) => break,
                }
            }
            let now = Instant::now();
            self.read_tap(now);
            self.update_bitrate();
            self.stun_timeouts(now);
            self.drive_all(now);
            self.flush();

            let now = Instant::now();
            let mut deadline = now
                + if self.capture.is_some() {
                    CAPTURE_POLL
                } else {
                    IDLE_POLL
                };
            for p in &self.peers {
                deadline = deadline.min(p.next_timeout);
            }
            for q in &self.stun {
                deadline = deadline.min(q.sent + STUN_RETRANSMIT);
            }
            let wait = deadline
                .saturating_duration_since(now)
                .max(Duration::from_millis(1));
            let _ = self.socket.set_read_timeout(Some(wait));
            match self.socket.recv_from(&mut self.buf) {
                Ok((n, source)) => self.datagram(n, source),
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                // ICMP port unreachable surfaces as a recv error on some systems: ignore.
                Err(_) => {}
            }
            let now = Instant::now();
            for p in &mut self.peers {
                p.timeout(now);
            }
            self.drive_all(now);
            self.flush();
        }
    }

    fn flush(&mut self) {
        for o in self.out.drain(..) {
            // Bounded: if the controller stops polling, drop (anchors are periodic, and a
            // stream whose signals are lost fails and is ended by the listener).
            let _ = self.out_tx.try_send(o);
        }
    }

    fn peer_mut(&mut self, key: (SiteId, u32)) -> Option<&mut Peer> {
        self.peers.iter_mut().find(|p| p.key() == key)
    }

    fn command(&mut self, cmd: Cmd) {
        let now = Instant::now();
        match cmd {
            Cmd::StartCapture {
                reader,
                sample_rate,
            } => {
                self.stop_capture();
                self.capture = Some(Capture {
                    reader,
                    tracker: ClockTracker::new(sample_rate, self.n_next),
                    framer: Framer::new(sample_rate, self.n_next),
                });
            }
            Cmd::StopCapture => self.stop_capture(),
            Cmd::Open {
                listener,
                stream,
                ice,
            } => self.open(listener, stream, ice, now),
            Cmd::Signal {
                listener,
                stream,
                signal,
            } => {
                if let Some(p) = self.peer_mut((listener, stream)) {
                    p.on_signal(&signal, now);
                }
            }
            Cmd::Close { listener, stream } => {
                if let Some(i) = self
                    .peers
                    .iter()
                    .position(|p| p.key() == (listener, stream))
                {
                    let mut p = self.peers.remove(i);
                    // Best effort DTLS close_notify; no state output (the controller closed).
                    let _ = p.rtc.close();
                    let mut sink = Vec::new();
                    p.drive(&self.socket, now, &mut sink);
                    self.stun.retain(|q| q.key != (listener, stream));
                }
            }
            Cmd::Transport(t) => {
                if t != self.transport {
                    self.transport = t;
                    self.transport_changed = true;
                }
            }
            Cmd::StunResolved {
                listener,
                stream,
                server,
            } => self.stun_resolved((listener, stream), server, now),
            Cmd::Shutdown => {}
        }
    }

    fn stop_capture(&mut self) {
        if let Some(c) = self.capture.take() {
            // A partial frame is discarded; the next capture continues the stream clock.
            self.n_next = c.framer.next_n();
        }
    }

    fn open(&mut self, listener: SiteId, stream: u32, ice: Vec<IceServer>, now: Instant) {
        if self.peer_mut((listener, stream)).is_some() {
            return;
        }
        if self.peers.len() >= MAX_PEERS {
            self.out.push(StreamOutput::State {
                listener,
                stream,
                state: StreamLinkState::Failed {
                    reason: "too many listeners".into(),
                },
            });
            return;
        }
        let (mut peer, sdp) = Peer::new(listener, stream, &self.host_candidates(), now);
        self.out.push(StreamOutput::Signal {
            listener,
            stream,
            signal: StreamSignal::Offer { sdp },
        });
        let urls: Vec<String> = ice
            .iter()
            .flat_map(|s| s.urls.iter())
            .filter(|u| stun::parse_stun_url(u).is_some())
            .take(MAX_STUN_URLS)
            .cloned()
            .collect();
        peer.stun_pending = urls.len();
        if !urls.is_empty() {
            // DNS may block: resolve on a short-lived thread, report through the commands.
            let tx = self.cmd_tx.clone();
            let spawned = std::thread::Builder::new()
                .name("ether-stream-stun".into())
                .spawn(move || {
                    for u in urls {
                        let server = stun::resolve_stun_url(&u);
                        let _ = tx.send(Cmd::StunResolved {
                            listener,
                            stream,
                            server,
                        });
                    }
                });
            if spawned.is_err() {
                peer.stun_pending = 0;
            }
        }
        peer.maybe_end_of_candidates(&mut self.out);
        peer.drive(&self.socket, now, &mut self.out);
        self.peers.push(peer);
    }

    fn stun_resolved(&mut self, key: (SiteId, u32), server: Option<SocketAddr>, now: Instant) {
        let base = server.map(|s| self.local_for(s));
        let full = self.stun.len() >= MAX_STUN_QUERIES;
        let Some(peer) = self.peers.iter_mut().find(|p| p.key() == key) else {
            return;
        };
        let (Some(server), Some(base), false) = (server, base, full) else {
            peer.stun_pending = peer.stun_pending.saturating_sub(1);
            peer.maybe_end_of_candidates(&mut self.out);
            return;
        };
        let mut tid = [0u8; 12];
        let _ = getrandom::fill(&mut tid);
        let _ = self.socket.send_to(&stun::binding_request(&tid), server);
        self.stun.push(StunQuery {
            tid,
            key,
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
                let key = q.key;
                self.stun.swap_remove(i);
                if let Some(p) = self.peers.iter_mut().find(|p| p.key() == key) {
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
        // Our own STUN transactions first (str0m never sees them).
        if let Some(tid) = stun::binding_response_tid(data)
            && let Some(i) = self.stun.iter().position(|q| q.tid == tid)
        {
            let q = self.stun.swap_remove(i);
            let mapped = stun::parse_binding_response(data).map(|(_, a)| a);
            if let Some(p) = self.peers.iter_mut().find(|p| p.key() == q.key) {
                if let Some(mapped) = mapped {
                    p.add_srflx(mapped, q.base, &mut self.out);
                }
                p.stun_pending = p.stun_pending.saturating_sub(1);
                p.maybe_end_of_candidates(&mut self.out);
            }
            return;
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
        if let Some(p) = self.peers.iter_mut().find(|p| p.rtc.accepts(&input)) {
            p.receive(input);
        }
    }

    fn drive_all(&mut self, now: Instant) {
        for p in &mut self.peers {
            p.drive(&self.socket, now, &mut self.out);
        }
        let before = self.peers.len();
        self.peers.retain(|p| p.done.is_none());
        if self.peers.len() != before {
            let alive: Vec<_> = self.peers.iter().map(Peer::key).collect();
            self.stun.retain(|q| alive.contains(&q.key));
        }
    }

    /// One shared encoder: its bitrate follows the lowest estimate of the connected
    /// listeners (so the most constrained link is not congested), clamped to 48-192 kbit/s.
    fn update_bitrate(&mut self) {
        let target = self
            .peers
            .iter()
            .filter(|p| p.connected)
            .map(|p| p.estimate.unwrap_or(DEFAULT_BITRATE))
            .min()
            .unwrap_or(DEFAULT_BITRATE)
            .clamp(MIN_BITRATE, MAX_BITRATE);
        if target.abs_diff(self.bitrate) >= 4_000 {
            self.bitrate = target;
            let _ = self.encoder.set_bitrate(opus::Bitrate::Bits(target as i32));
        }
    }

    /// Drain the tap: anchors per block, audio → frames → Opus → listeners.
    fn read_tap(&mut self, now: Instant) {
        let Some(c) = self.capture.as_mut() else {
            return;
        };
        let mut clocks: Vec<BlockClock> = Vec::new();
        while let Ok(&b) = c.reader.blocks.peek() {
            let need = 2 * b.frames as usize;
            if c.reader.audio.slots() < need {
                break; // the audio of this header is not committed yet
            }
            let _ = c.reader.blocks.pop();
            clocks.push(c.tracker.on_block(&b));
            if let Ok(chunk) = c.reader.audio.read_chunk(need) {
                let (a, z) = chunk.as_slices();
                self.scratch.clear();
                self.scratch.extend_from_slice(a);
                self.scratch.extend_from_slice(z);
                chunk.commit_all();
                c.framer.push(&self.scratch);
            }
        }
        for bc in &clocks {
            self.anchors(bc);
        }
        let any = self.peers.iter().any(|p| p.connected);
        let Some(c) = self.capture.as_mut() else {
            return;
        };
        while let Some(n) = c.framer.pop_frame(&mut self.frame) {
            if !any {
                continue; // keep the clock running, nobody to send to
            }
            let len = match self.encoder.encode_float(&self.frame, &mut self.packet) {
                Ok(len) => len,
                Err(e) => {
                    tracing::debug!("opus encode: {e}");
                    continue;
                }
            };
            let data: Arc<[u8]> = Arc::from(&self.packet[..len]);
            // The decoded sample at RTP `o + m` is stream sample `m`: the frame's first
            // decoded sample is the input `lookahead` samples earlier.
            let ts_base = (1u64 << 33) + n - self.lookahead;
            for p in &mut self.peers {
                p.write(data.clone(), ts_base, now);
            }
        }
    }

    fn anchors(&mut self, bc: &BlockClock) {
        let force = std::mem::take(&mut self.transport_changed);
        let t = self.transport;
        let count_in_end = t
            .count_in_end
            .filter(|&end| bc.playing && bc.recording && bc.position < end)
            .map(Beats);
        for p in self.peers.iter_mut().filter(|p| p.connected) {
            let due = bc.discontinuity
                || force
                || p.last_anchor_n
                    .is_none_or(|last| bc.n >= last + ANCHOR_INTERVAL_SAMPLES);
            if !due {
                continue;
            }
            p.last_anchor_n = Some(bc.n);
            self.out.push(StreamOutput::Clock {
                listener: p.listener,
                stream: p.stream,
                clock: StreamClock {
                    rtp: bc.rtp(p.offset),
                    position: Beats(bc.position),
                    playing: bc.playing,
                    recording: bc.recording,
                    bpm: bc.bpm,
                    loop_enabled: t.loop_enabled,
                    loop_region: BeatRange {
                        start: Beats(t.loop_start),
                        end: Beats(t.loop_end),
                    },
                    metronome: t.metronome,
                    discontinuity: bc.discontinuity,
                    count_in_end,
                },
            });
        }
    }
}

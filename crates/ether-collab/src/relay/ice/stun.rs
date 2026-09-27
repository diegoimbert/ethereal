//! The STUN Binding responder (RFC 8489; docs/COLLAB.md §10).
//!
//! [`StunResponder`] is pure (time is injected): a Binding request gets a Binding success
//! response with the same transaction id, XOR-MAPPED-ADDRESS (the request's source) and
//! FINGERPRINT, nothing else: 40 bytes for IPv4, 52 for IPv6. Malformed packets, anything
//! but a Binding request, and requests over the rate caps (per source IP, and global) are
//! dropped silently. [`StunThread`] runs it on a UDP socket.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use stun::fingerprint::FINGERPRINT;
use stun::message::{BINDING_REQUEST, BINDING_SUCCESS, Message, Setter};
use stun::xoraddr::XorMappedAddress;

/// Responses per second to one source IP.
pub const PER_IP_PER_SECOND: u32 = 50;
/// Responses per second for the whole responder (≤ 52 KB/s of reflected traffic).
pub const GLOBAL_PER_SECOND: u32 = 1000;
/// Source IPs tracked at once (stale ones are evicted first; when every tracked IP is
/// live, requests from new IPs are dropped).
pub const MAX_TRACKED_IPS: usize = 4096;

const HEADER: usize = 20;
const MAGIC_COOKIE: [u8; 4] = [0x21, 0x12, 0xa4, 0x42];
const WINDOW: Duration = Duration::from_secs(1);

/// A fixed one-second window counter.
#[derive(Clone, Copy, Debug)]
struct Window {
    start: Instant,
    count: u32,
}

impl Window {
    fn roll(&mut self, now: Instant) {
        if now.saturating_duration_since(self.start) >= WINDOW {
            self.start = now;
            self.count = 0;
        }
    }
}

/// Per-source-IP and global responses-per-second caps (one-second windows), with a
/// bounded map of source IPs.
#[derive(Debug)]
pub struct RateCaps {
    per_ip: HashMap<IpAddr, Window>,
    global: Option<Window>,
    per_ip_limit: u32,
    global_limit: u32,
    max_ips: usize,
}

impl RateCaps {
    pub fn new(per_ip: u32, global: u32, max_ips: usize) -> Self {
        Self {
            per_ip: HashMap::new(),
            global: None,
            per_ip_limit: per_ip,
            global_limit: global,
            max_ips: max_ips.max(1),
        }
    }

    /// Source IPs currently tracked (at most `max_ips`).
    pub fn tracked_ips(&self) -> usize {
        self.per_ip.len()
    }

    /// Count one response to `ip` at `now` if both the global and `ip`'s window have room.
    pub fn admit(&mut self, ip: IpAddr, now: Instant) -> bool {
        let ip = ip.to_canonical();
        let global = self.global.get_or_insert(Window {
            start: now,
            count: 0,
        });
        global.roll(now);
        if global.count >= self.global_limit {
            return false;
        }
        if !self.per_ip.contains_key(&ip) && self.per_ip.len() >= self.max_ips {
            self.per_ip
                .retain(|_, w| now.saturating_duration_since(w.start) < WINDOW);
            if self.per_ip.len() >= self.max_ips {
                return false;
            }
        }
        let w = self.per_ip.entry(ip).or_insert(Window {
            start: now,
            count: 0,
        });
        w.roll(now);
        if w.count >= self.per_ip_limit {
            return false;
        }
        w.count += 1;
        global.count += 1;
        true
    }
}

impl Default for RateCaps {
    fn default() -> Self {
        Self::new(PER_IP_PER_SECOND, GLOBAL_PER_SECOND, MAX_TRACKED_IPS)
    }
}

/// The rate-limited Binding responder (no I/O).
#[derive(Debug, Default)]
pub struct StunResponder {
    caps: RateCaps,
}

impl StunResponder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_limits(per_ip: u32, global: u32, max_ips: usize) -> Self {
        Self {
            caps: RateCaps::new(per_ip, global, max_ips),
        }
    }

    /// Source IPs currently tracked (bounded by the `max_ips` limit).
    pub fn tracked_ips(&self) -> usize {
        self.caps.tracked_ips()
    }

    /// The response to `packet` from `from` at `now`, or `None` (dropped).
    pub fn handle(&mut self, packet: &[u8], from: SocketAddr, now: Instant) -> Option<Vec<u8>> {
        let request = parse_binding_request(packet)?;
        if !self.caps.admit(from.ip(), now) {
            return None;
        }
        binding_success(
            &request,
            SocketAddr::new(from.ip().to_canonical(), from.port()),
        )
    }
}

/// `true` if `packet` looks like a STUN Binding request (type and cookie only; used to
/// route packets on a port shared with TURN).
pub fn is_binding_request(packet: &[u8]) -> bool {
    packet.len() >= HEADER && packet[0..2] == [0x00, 0x01] && packet[4..8] == MAGIC_COOKIE
}

/// A well-formed Binding request (exact length, 32-bit aligned, valid attributes, a
/// correct FINGERPRINT if it has one), decoded.
fn parse_binding_request(packet: &[u8]) -> Option<Message> {
    if !is_binding_request(packet) {
        return None;
    }
    let length = u16::from_be_bytes([packet[2], packet[3]]) as usize;
    if packet.len() != HEADER + length || !length.is_multiple_of(4) {
        return None;
    }
    let mut m = Message::new();
    m.unmarshal_binary(packet).ok()?;
    if m.typ != BINDING_REQUEST {
        return None;
    }
    if m.contains(stun::attributes::ATTR_FINGERPRINT) && FINGERPRINT.check(&m).is_err() {
        return None;
    }
    Some(m)
}

/// Header + XOR-MAPPED-ADDRESS + FINGERPRINT, same transaction id.
fn binding_success(request: &Message, mapped: SocketAddr) -> Option<Vec<u8>> {
    let setters: [Box<dyn Setter>; 4] = [
        Box::new(Message {
            transaction_id: request.transaction_id,
            ..Default::default()
        }),
        Box::new(BINDING_SUCCESS),
        Box::new(XorMappedAddress {
            ip: mapped.ip(),
            port: mapped.port(),
        }),
        Box::new(FINGERPRINT),
    ];
    let mut m = Message::new();
    m.build(&setters).ok()?;
    Some(m.raw)
}

/// A thread answering Binding requests on a UDP socket; dropping it stops it.
#[derive(Debug)]
pub struct StunThread {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

const READ_TIMEOUT: Duration = Duration::from_millis(100);

impl StunThread {
    pub fn start(socket: UdpSocket) -> std::io::Result<Self> {
        let addr = socket.local_addr()?;
        socket.set_read_timeout(Some(READ_TIMEOUT))?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let stop = stop.clone();
            std::thread::Builder::new()
                .name("collab-relay-stun".into())
                .spawn(move || run(&socket, &stop))?
        };
        Ok(Self {
            addr,
            stop,
            thread: Some(thread),
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }
}

impl Drop for StunThread {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn run(socket: &UdpSocket, stop: &AtomicBool) {
    let mut responder = StunResponder::new();
    let mut buf = [0u8; 2048];
    while !stop.load(Ordering::Relaxed) {
        let (n, from) = match socket.recv_from(&mut buf) {
            Ok(v) => v,
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::WouldBlock
                        | ErrorKind::TimedOut
                        | ErrorKind::Interrupted
                        | ErrorKind::ConnectionReset
                        | ErrorKind::ConnectionRefused
                ) =>
            {
                continue;
            }
            Err(e) => {
                tracing::warn!(%e, "STUN socket failed; STUN stopped");
                return;
            }
        };
        if let Some(response) = responder.handle(&buf[..n], from, Instant::now())
            && let Err(e) = socket.send_to(&response, from)
        {
            tracing::debug!(%e, %from, "STUN response not sent");
        }
    }
}

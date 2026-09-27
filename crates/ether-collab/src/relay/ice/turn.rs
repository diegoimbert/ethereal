//! The TURN server (cargo feature `turn`, `--turn`; docs/COLLAB.md §10): the webrtc-rs
//! `turn` crate in a tokio runtime on its own thread, owning the relay's UDP port.
//!
//! Our hooks around it:
//! - the listening socket is wrapped ([`ListenConn`]): Binding requests are answered by the
//!   capped [`StunResponder`], and unauthenticated requests (which get a 401 bigger than
//!   the request) are capped the same way, so the port stays a bounded reflector;
//! - the auth handler recomputes the TURN REST credential ([`TurnCredentials`]);
//! - the relay address generator enforces the allocation quotas (per username, per relay)
//!   and hands out relay sockets wrapped in [`RelayConn`], which drops traffic to or from
//!   denied peer addresses ([`PeerFilter`]), over the per-allocation bandwidth, or past the
//!   credential's expiry;
//! - a reaper deletes the allocations of expired usernames.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::net::UdpSocket;
use turn::auth::{AuthHandler, generate_auth_key};
use turn::relay::RelayAddressGenerator;
use turn::server::Server;
use turn::server::config::{ConnConfig, ServerConfig};
use webrtc_util::Conn;

use super::credentials::{TurnCredentials, check_username, unix_now};
use super::peers::PeerFilter;
use super::stun::{RateCaps, StunResponder, is_binding_request};
use super::{DEFAULT_TURN_PORTS, IceConfig};

/// Allocations one username (one site's credential) may hold.
pub const MAX_ALLOCATIONS_PER_USERNAME: usize = 4;
/// Allocations the relay holds at most.
pub const MAX_ALLOCATIONS: usize = 64;
/// Relayed bytes per second per allocation, both directions (512 kbit/s); burst: one
/// second's worth. Excess datagrams are dropped.
pub const ALLOCATION_BYTES_PER_SECOND: u64 = 512_000 / 8;
/// The TURN realm.
pub const REALM: &str = "ether-collab";

const STOP_POLL: Duration = Duration::from_millis(100);
const REAP_EVERY: Duration = Duration::from_secs(10);
const STUN_ATTR_MESSAGE_INTEGRITY: u16 = 0x0008;

type TurnResult<T> = Result<T, turn::Error>;

/// The username (and its expiry) of the last request the auth handler accepted. The
/// server's read loop handles one datagram at a time (auth, then allocation), so the
/// generator reads the username of the Allocate request it is serving from here.
type LastAuth = Arc<Mutex<Option<(String, u64)>>>;

struct Auth {
    creds: Arc<TurnCredentials>,
    last: LastAuth,
}

impl AuthHandler for Auth {
    fn auth_handle(&self, username: &str, realm: &str, _src: SocketAddr) -> TurnResult<Vec<u8>> {
        let now = unix_now();
        let (Some(password), Some((expiry, _))) = (
            self.creds.password_for(username, now),
            check_username(username, now),
        ) else {
            *self.last.lock().expect("auth lock") = None;
            return Err(turn::Error::Other("bad or expired TURN username".into()));
        };
        *self.last.lock().expect("auth lock") = Some((username.to_string(), expiry));
        Ok(generate_auth_key(username, realm, &password))
    }
}

/// Live allocations, per username (with the username's expiry) and in total.
#[derive(Debug, Default)]
struct Quota {
    total: usize,
    users: HashMap<String, (usize, u64)>,
}

/// One allocation's slot in the [`Quota`], released on close or drop.
#[derive(Debug)]
struct QuotaSlot {
    quota: Arc<Mutex<Quota>>,
    username: String,
    released: AtomicBool,
}

impl QuotaSlot {
    fn acquire(quota: &Arc<Mutex<Quota>>, username: &str, expiry: u64) -> TurnResult<Self> {
        let mut q = quota.lock().expect("quota lock");
        let user = q.users.get(username).map_or(0, |(n, _)| *n);
        if q.total >= MAX_ALLOCATIONS || user >= MAX_ALLOCATIONS_PER_USERNAME {
            return Err(turn::Error::Other("allocation quota reached".into()));
        }
        q.total += 1;
        q.users.insert(username.to_string(), (user + 1, expiry));
        Ok(Self {
            quota: quota.clone(),
            username: username.to_string(),
            released: AtomicBool::new(false),
        })
    }

    fn release(&self) {
        if self.released.swap(true, Ordering::AcqRel) {
            return;
        }
        let mut q = self.quota.lock().expect("quota lock");
        q.total = q.total.saturating_sub(1);
        if let Some((n, _)) = q.users.get_mut(&self.username) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                q.users.remove(&self.username);
            }
        }
    }
}

impl Drop for QuotaSlot {
    fn drop(&mut self) {
        self.release();
    }
}

/// Token bucket in bytes.
#[derive(Debug)]
struct ByteBucket {
    tokens: f64,
    at: Instant,
}

impl ByteBucket {
    fn new() -> Self {
        Self {
            tokens: ALLOCATION_BYTES_PER_SECOND as f64,
            at: Instant::now(),
        }
    }

    fn take(&mut self, bytes: usize) -> bool {
        let rate = ALLOCATION_BYTES_PER_SECOND as f64;
        let now = Instant::now();
        self.tokens = (self.tokens + now.duration_since(self.at).as_secs_f64() * rate).min(rate);
        self.at = now;
        if self.tokens < bytes as f64 {
            return false;
        }
        self.tokens -= bytes as f64;
        true
    }
}

/// An allocation's relay socket: peer filter, bandwidth cap and credential expiry.
struct RelayConn {
    socket: Arc<UdpSocket>,
    slot: QuotaSlot,
    expiry: u64,
    filter: Arc<PeerFilter>,
    bucket: Mutex<ByteBucket>,
}

impl RelayConn {
    fn pass(&self, peer: SocketAddr, bytes: usize) -> bool {
        unix_now() < self.expiry
            && self.filter.allows(peer.ip())
            && self.bucket.lock().expect("bucket lock").take(bytes)
    }
}

fn unsupported() -> webrtc_util::Error {
    webrtc_util::Error::Other("not supported on a TURN relay socket".into())
}

#[async_trait]
impl Conn for RelayConn {
    async fn connect(&self, _addr: SocketAddr) -> webrtc_util::Result<()> {
        Err(unsupported())
    }

    async fn recv(&self, _buf: &mut [u8]) -> webrtc_util::Result<usize> {
        Err(unsupported())
    }

    async fn recv_from(&self, buf: &mut [u8]) -> webrtc_util::Result<(usize, SocketAddr)> {
        loop {
            let (n, from) = match self.socket.recv_from(buf).await {
                Ok(v) => v,
                Err(e) if transient(&e) => continue,
                Err(e) => return Err(e.into()),
            };
            if self.pass(from, n) {
                return Ok((n, from));
            }
        }
    }

    async fn send(&self, _buf: &[u8]) -> webrtc_util::Result<usize> {
        Err(unsupported())
    }

    async fn send_to(&self, buf: &[u8], target: SocketAddr) -> webrtc_util::Result<usize> {
        if !self.pass(target, buf.len()) {
            // Dropped like any UDP datagram (no error: the sender is not told).
            return Ok(buf.len());
        }
        Ok(self.socket.send_to(buf, target).await?)
    }

    fn local_addr(&self) -> webrtc_util::Result<SocketAddr> {
        Ok(self.socket.local_addr()?)
    }

    fn remote_addr(&self) -> Option<SocketAddr> {
        None
    }

    async fn close(&self) -> webrtc_util::Result<()> {
        self.slot.release();
        Ok(())
    }

    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
        self
    }
}

fn transient(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        ErrorKind::ConnectionReset | ErrorKind::ConnectionRefused | ErrorKind::Interrupted
    )
}

/// The TURN listening socket: Binding requests go to the capped [`StunResponder`];
/// unauthenticated requests are capped per source and globally; the rest goes to TURN.
struct ListenConn {
    socket: Arc<UdpSocket>,
    stun: Mutex<StunResponder>,
    unauthenticated: Mutex<RateCaps>,
}

impl ListenConn {
    /// `false` if the datagram must be dropped (an unauthenticated request over the caps).
    fn admit(&self, packet: &[u8], from: SocketAddr) -> bool {
        // STUN requests only (ChannelData starts with bits 01; indications and responses
        // get no answer, so they reflect nothing).
        let typ = if packet.len() >= 20 {
            u16::from_be_bytes([packet[0], packet[1]])
        } else {
            0xffff
        };
        if typ & 0xc000 != 0 || typ & 0x0110 != 0 {
            return true;
        }
        let mut m = stun::message::Message::new();
        if m.unmarshal_binary(packet).is_err() {
            return false;
        }
        let authenticated = m
            .attributes
            .0
            .iter()
            .any(|a| a.typ.0 == STUN_ATTR_MESSAGE_INTEGRITY);
        authenticated
            || self
                .unauthenticated
                .lock()
                .expect("caps lock")
                .admit(from.ip(), Instant::now())
    }
}

#[async_trait]
impl Conn for ListenConn {
    async fn connect(&self, _addr: SocketAddr) -> webrtc_util::Result<()> {
        Err(unsupported())
    }

    async fn recv(&self, _buf: &mut [u8]) -> webrtc_util::Result<usize> {
        Err(unsupported())
    }

    async fn recv_from(&self, buf: &mut [u8]) -> webrtc_util::Result<(usize, SocketAddr)> {
        loop {
            let (n, from) = match self.socket.recv_from(buf).await {
                Ok(v) => v,
                Err(e) if transient(&e) => continue,
                Err(e) => return Err(e.into()),
            };
            let packet = &buf[..n];
            if is_binding_request(packet) {
                let response =
                    self.stun
                        .lock()
                        .expect("stun lock")
                        .handle(packet, from, Instant::now());
                if let Some(r) = response {
                    let _ = self.socket.send_to(&r, from).await;
                }
                continue;
            }
            if self.admit(packet, from) {
                return Ok((n, from));
            }
        }
    }

    async fn send(&self, _buf: &[u8]) -> webrtc_util::Result<usize> {
        Err(unsupported())
    }

    async fn send_to(&self, buf: &[u8], target: SocketAddr) -> webrtc_util::Result<usize> {
        Ok(self.socket.send_to(buf, target).await?)
    }

    fn local_addr(&self) -> webrtc_util::Result<SocketAddr> {
        Ok(self.socket.local_addr()?)
    }

    fn remote_addr(&self) -> Option<SocketAddr> {
        None
    }

    async fn close(&self) -> webrtc_util::Result<()> {
        Ok(())
    }

    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
        self
    }
}

/// Binds relay sockets in the port range, under the quotas.
struct Generator {
    listen_ip: IpAddr,
    relay_ip: IpAddr,
    ports: (u16, u16),
    last: LastAuth,
    quota: Arc<Mutex<Quota>>,
    filter: Arc<PeerFilter>,
}

fn random_u16() -> u16 {
    let mut b = [0u8; 2];
    let _ = getrandom::fill(&mut b);
    u16::from_le_bytes(b)
}

#[async_trait]
impl RelayAddressGenerator for Generator {
    fn validate(&self) -> TurnResult<()> {
        Ok(())
    }

    async fn allocate_conn(
        &self,
        use_ipv4: bool,
        requested_port: u16,
    ) -> TurnResult<(Arc<dyn Conn + Send + Sync>, SocketAddr)> {
        if use_ipv4 != self.relay_ip.is_ipv4() {
            return Err(turn::Error::Other("unsupported address family".into()));
        }
        let (username, expiry) = self
            .last
            .lock()
            .expect("auth lock")
            .clone()
            .ok_or_else(|| turn::Error::Other("unauthenticated allocation".into()))?;
        let slot = QuotaSlot::acquire(&self.quota, &username, expiry)?;
        let (lo, hi) = self.ports;
        let socket = if requested_port != 0 {
            if !(lo..=hi).contains(&requested_port) {
                return Err(turn::Error::Other("port outside the relay range".into()));
            }
            UdpSocket::bind(SocketAddr::new(self.listen_ip, requested_port)).await?
        } else {
            let mut bound = None;
            for _ in 0..16 {
                let port = lo + random_u16() % (hi - lo + 1).max(1);
                if let Ok(s) = UdpSocket::bind(SocketAddr::new(self.listen_ip, port)).await {
                    bound = Some(s);
                    break;
                }
            }
            bound.ok_or(turn::Error::ErrMaxRetriesExceeded)?
        };
        let mut relay_addr = socket.local_addr()?;
        relay_addr.set_ip(self.relay_ip);
        let conn = RelayConn {
            socket: Arc::new(socket),
            slot,
            expiry,
            filter: self.filter.clone(),
            bucket: Mutex::new(ByteBucket::new()),
        };
        Ok((Arc::new(conn), relay_addr))
    }
}

/// The TURN server thread; dropping it stops it.
#[derive(Debug)]
pub struct TurnThread {
    addr: SocketAddr,
    relay_ip: IpAddr,
    ports: (u16, u16),
    allow_private: bool,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl TurnThread {
    /// Run TURN on `socket` (already bound: the relay's UDP port). Returns the thread and
    /// the credential minter its auth handler checks against.
    pub fn start(
        socket: std::net::UdpSocket,
        config: &IceConfig,
    ) -> Result<(Self, Arc<TurnCredentials>), String> {
        let addr = socket.local_addr().map_err(|e| e.to_string())?;
        let listen_ip = addr.ip();
        let relay_ip = match config.public_ip {
            Some(ip) => ip,
            None if listen_ip.is_unspecified() => {
                return Err(
                    "--turn needs --public-ip when listening on an unspecified address".into(),
                );
            }
            None => listen_ip,
        };
        let ports = config.turn_ports.unwrap_or(DEFAULT_TURN_PORTS);
        if ports.0 == 0 || ports.0 > ports.1 {
            return Err(format!("bad TURN port range {}-{}", ports.0, ports.1));
        }
        let creds = Arc::new(TurnCredentials::new()?);
        let filter = Arc::new(PeerFilter {
            allow_private: config.turn_allow_private,
            own: vec![listen_ip, relay_ip],
        });
        socket.set_nonblocking(true).map_err(|e| e.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let thread = {
            let (stop, creds) = (stop.clone(), creds.clone());
            std::thread::Builder::new()
                .name("collab-relay-turn".into())
                .spawn(move || {
                    let rt = match tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        Ok(rt) => rt,
                        Err(e) => {
                            let _ = ready_tx.send(Err(format!("tokio runtime: {e}")));
                            return;
                        }
                    };
                    rt.block_on(serve(
                        socket, listen_ip, relay_ip, ports, creds, filter, stop, ready_tx,
                    ));
                })
                .map_err(|e| e.to_string())?
        };
        let this = Self {
            addr,
            relay_ip,
            ports,
            allow_private: config.turn_allow_private,
            stop,
            thread: Some(thread),
        };
        match ready_rx.recv() {
            Ok(Ok(())) => Ok((this, creds)),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("TURN thread exited".into()),
        }
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// For the status line.
    pub fn describe(&self) -> String {
        format!(
            "relayed {} ports {}-{}{}",
            self.relay_ip,
            self.ports.0,
            self.ports.1,
            if self.allow_private {
                ", private peers allowed"
            } else {
                ""
            }
        )
    }
}

impl Drop for TurnThread {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn serve(
    socket: std::net::UdpSocket,
    listen_ip: IpAddr,
    relay_ip: IpAddr,
    ports: (u16, u16),
    creds: Arc<TurnCredentials>,
    filter: Arc<PeerFilter>,
    stop: Arc<AtomicBool>,
    ready: std::sync::mpsc::Sender<Result<(), String>>,
) {
    let socket = match UdpSocket::from_std(socket) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            let _ = ready.send(Err(format!("udp socket: {e}")));
            return;
        }
    };
    let last: LastAuth = Arc::default();
    let quota: Arc<Mutex<Quota>> = Arc::default();
    let listener = ListenConn {
        socket,
        stun: Mutex::new(StunResponder::new()),
        unauthenticated: Mutex::new(RateCaps::default()),
    };
    let config = ServerConfig {
        conn_configs: vec![ConnConfig {
            conn: Arc::new(listener),
            relay_addr_generator: Box::new(Generator {
                listen_ip,
                relay_ip,
                ports,
                last: last.clone(),
                quota: quota.clone(),
                filter,
            }),
        }],
        realm: REALM.into(),
        auth_handler: Arc::new(Auth { creds, last }),
        channel_bind_timeout: Duration::ZERO,
        alloc_close_notify: None,
    };
    let server = match Server::new(config).await {
        Ok(s) => s,
        Err(e) => {
            let _ = ready.send(Err(format!("TURN server: {e}")));
            return;
        }
    };
    let _ = ready.send(Ok(()));
    let mut reaped = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        tokio::time::sleep(STOP_POLL).await;
        if reaped.elapsed() < REAP_EVERY {
            continue;
        }
        reaped = Instant::now();
        // Allocations never outlive their credential.
        let now = unix_now();
        let expired: Vec<String> = quota
            .lock()
            .expect("quota lock")
            .users
            .iter()
            .filter(|(_, (_, expiry))| *expiry <= now)
            .map(|(u, _)| u.clone())
            .collect();
        for username in expired {
            let _ = server.delete_allocations_by_username(username).await;
        }
    }
    let _ = server.close().await;
}

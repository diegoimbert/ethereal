//! The relay over WebSocket (native): `ether-collab-relay`, dev port base `+3`.
//!
//! Same handshake and hardening as `ether-server` (`ether_protocol::remote`): the session is
//! the URL path (`ws://host:port/<session>`), the first text frame is a `ClientHello` (token
//! compared in constant time; serving without a token is only allowed on loopback, and then
//! only loopback `Host`/`Origin`s may upgrade), answered by a `ServerHello`. Before the hello
//! a connection has an absolute deadline, a 64 KiB message limit and counts against
//! [`RelayServerConfig::max_pending_handshakes`]. Afterwards every frame is one
//! [`CollabMessage`](ether_protocol::collab::CollabMessage) (JSON text, media chunks as
//! binary frames), writes time out and idle connections are pinged, then dropped. A slow
//! reader is dropped when its outgoing queue passes a message count or a byte budget. A
//! site id claimed by a new connection makes the relay ping its current holder, which is
//! dropped if it doesn't answer within `RelayConfig::site_probe_ms`.
//!
//! Threads: one accept thread, one per connection; the [`Relay`] state machine sits behind
//! a mutex and routes outgoing messages into per-connection bounded queues.

use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, TryRecvError};
use ether_protocol::collab::CollabMessage;
use ether_protocol::remote::{
    ClientHello, HelloRejection, PROTOCOL_VERSION, ServerCapabilities, ServerHello, ServerInfo,
};
use tungstenite::handshake::HandshakeError;
use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::http::StatusCode;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::{CloseFrame, WebSocketConfig};
use tungstenite::{Message, WebSocket};

use super::{ConnId, Outgoing, Relay, RelayConfig};
use crate::wire::{
    MAX_COLLAB_MESSAGE_BYTES, WireFrame, decode_binary, decode_text, encode_frame,
    session_from_path,
};

pub const CLOSE_AUTH: u16 = 4001;
pub const CLOSE_VERSION: u16 = 4002;
pub const CLOSE_BUSY: u16 = 4003;
pub const CLOSE_IDLE: u16 = 4004;
/// A slow reader (its outgoing queue passed its bound) or a half-open connection another
/// connection of the same site took over.
pub const CLOSE_DROPPED: u16 = 4005;

const POLL: Duration = Duration::from_millis(4);
/// How often the relay's clock advances (site id probes time out).
const TICK: Duration = Duration::from_millis(100);
const PRE_AUTH_POLL: Duration = Duration::from_millis(10);
const MAX_PRE_AUTH_MESSAGE_BYTES: usize = 64 << 10;
/// Outgoing queue per connection. A site this far behind is disconnected (it resyncs on
/// reconnect).
pub const CONN_QUEUE: usize = 8192;

/// Outgoing queue of a connection: at least [`CONN_QUEUE`], and room for everything a late
/// joiner is sent at once (cached media chunks, snapshot, the full log, peers), so a joiner
/// is never disconnected by its own catch-up (and can't loop on reconnects).
pub fn conn_queue(relay: &RelayConfig) -> usize {
    let media_chunks = relay.max_media_bytes / crate::wire::MEDIA_CHUNK_BYTES + 1;
    CONN_QUEUE.max(relay.max_log + media_chunks + 4 * relay.max_sites_per_session + 64)
}

/// Bytes a connection may have queued (encoded size): `max_queued_bytes`, and at least
/// room for a late joiner's catch-up (the cached media, a maximal snapshot, and the log
/// at a nominal 4 KiB per transaction; queued messages are shared between connections,
/// so this bounds how far one reader lags, not memory per reader).
pub fn conn_byte_budget(config: &RelayServerConfig) -> usize {
    let relay = &config.relay;
    let catch_up = relay.max_media_bytes + 2 * MAX_COLLAB_MESSAGE_BYTES + relay.max_log * 4096;
    config.max_queued_bytes.max(catch_up)
}

/// Encoded size of a message (as the frame that carries it).
fn wire_size(m: &CollabMessage) -> usize {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0 += b.len();
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    match m {
        CollabMessage::Media { data, .. } => data.0.len() + 256,
        CollabMessage::Snapshot { data } => data.0.len() * 4 / 3 + 64,
        _ => {
            let mut c = Count(0);
            let _ = serde_json::to_writer(&mut c, m);
            c.0
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RelayServerConfig {
    /// Default: `127.0.0.1:0`.
    pub bind: SocketAddr,
    /// Shared secret sites send in `ClientHello::token`; `None` only on loopback.
    pub token: Option<String>,
    pub name: Option<String>,
    pub instance: String,
    pub relay: RelayConfig,
    pub handshake_timeout: Duration,
    pub max_pending_handshakes: usize,
    pub write_timeout: Duration,
    pub ping_interval: Duration,
    pub idle_timeout: Duration,
    /// Messages per second a site may send (burst: one second's worth); a site above it is
    /// disconnected.
    pub max_messages_per_second: u32,
    /// Outgoing bytes a connection may have queued before it is dropped as too slow
    /// (raised to fit a late joiner's catch-up, see [`conn_byte_budget`]).
    pub max_queued_bytes: usize,
}

impl Default for RelayServerConfig {
    fn default() -> Self {
        Self {
            bind: SocketAddr::from(([127, 0, 0, 1], 0)),
            token: None,
            name: None,
            instance: std::env::var("ETHER_INSTANCE").unwrap_or_default(),
            relay: RelayConfig::default(),
            handshake_timeout: Duration::from_secs(10),
            max_pending_handshakes: 32,
            write_timeout: Duration::from_secs(10),
            ping_interval: Duration::from_secs(20),
            idle_timeout: Duration::from_secs(60),
            max_messages_per_second: 2_000,
            max_queued_bytes: 256 << 20,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RelayServerError {
    #[error("refusing to serve without a token on non-loopback address {0}")]
    InsecureBind(SocketAddr),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
}

/// Constant-time token comparison (length is not secret).
pub fn token_matches(expected: &str, given: &str) -> bool {
    let (a, b) = (expected.as_bytes(), given.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// `n` random bytes from the OS as lowercase hex.
pub fn random_hex(n: usize) -> Result<String, String> {
    let mut bytes = vec![0u8; n];
    getrandom::fill(&mut bytes).map_err(|e| format!("no OS entropy: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn loggable(s: &str) -> String {
    s.chars().take(80).flat_map(char::escape_debug).collect()
}

/// `true` if a `Host` header value (`name[:port]`) names the loopback interface.
pub fn is_loopback_host(host: &str) -> bool {
    let host = host.trim();
    let name = if let Some(rest) = host.strip_prefix('[') {
        match rest.split_once(']') {
            Some((inner, tail)) if tail.is_empty() || tail.starts_with(':') => inner,
            _ => return false,
        }
    } else if host.matches(':').count() == 1 {
        host.split(':').next().unwrap_or_default()
    } else {
        host
    };
    name.eq_ignore_ascii_case("localhost")
        || name
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// `true` if an `Origin` header value is a loopback page.
pub fn is_loopback_origin(origin: &str) -> bool {
    let Some((scheme, rest)) = origin.trim().split_once("://") else {
        return false;
    };
    matches!(scheme, "http" | "https") && is_loopback_host(rest.split('/').next().unwrap_or(""))
}

fn check_unauthenticated_upgrade(req: &Request) -> Result<(), &'static str> {
    let header = |name: &str| req.headers().get(name).and_then(|v| v.to_str().ok());
    if !header("host").is_some_and(is_loopback_host) {
        return Err("Host must be a loopback address when the relay has no token");
    }
    if let Some(origin) = header("origin")
        && !is_loopback_origin(origin)
    {
        return Err("Origin must be a loopback page when the relay has no token");
    }
    Ok(())
}

/// A connection's outgoing queue: messages with their encoded size, and the bytes queued.
struct Queue {
    tx: Sender<(Arc<CollabMessage>, usize)>,
    bytes: Arc<AtomicUsize>,
}

struct Shared {
    relay: Mutex<Relay>,
    queues: Mutex<HashMap<ConnId, Queue>>,
    /// Connections the relay wants pinged now (contested site ids).
    pings: Mutex<HashSet<ConnId>>,
    /// Connections the relay dropped, with why (their threads close the sockets).
    kicked: Mutex<HashMap<ConnId, String>>,
    started: Instant,
    byte_budget: usize,
    config: RelayServerConfig,
    info: ServerInfo,
    stop: AtomicBool,
    pending: AtomicUsize,
    next_conn: AtomicU64,
}

impl Shared {
    /// Run `f` on the relay and route its output **under the relay lock**, so every
    /// connection's queue receives messages in the relay's (total) order.
    fn with_relay<R>(&self, f: impl FnOnce(&mut Relay, &mut Vec<Outgoing>) -> R) -> R {
        let mut relay = self.relay.lock().expect("relay lock");
        let mut out = Vec::new();
        relay.tick(self.started.elapsed().as_millis() as u64, &mut out);
        let r = f(&mut relay, &mut out);
        self.route(out);
        let pings = relay.take_pings();
        if !pings.is_empty() {
            self.pings.lock().expect("pings lock").extend(pings);
        }
        let closing = relay.take_closing();
        if !closing.is_empty() {
            let mut queues = self.queues.lock().expect("queues lock");
            let mut kicked = self.kicked.lock().expect("kicked lock");
            for (conn, why) in closing {
                queues.remove(&conn);
                kicked.insert(conn, why);
            }
        }
        r
    }

    /// Queue messages for their connections; a full queue (count or bytes) drops that
    /// connection's sender (its thread then closes the socket).
    fn route(&self, out: Vec<Outgoing>) {
        if out.is_empty() {
            return;
        }
        let mut queues = self.queues.lock().expect("queues lock");
        // Broadcasts share one message: size it once.
        let mut sizes: HashMap<*const CollabMessage, usize> = HashMap::new();
        for (conn, m) in out {
            let Some(q) = queues.get(&conn) else { continue };
            let size = *sizes
                .entry(Arc::as_ptr(&m))
                .or_insert_with(|| wire_size(&m));
            let queued = q.bytes.fetch_add(size, Ordering::AcqRel) + size;
            let sent = queued <= self.byte_budget && q.tx.try_send((m, size)).is_ok();
            if !sent {
                queues.remove(&conn);
            }
        }
    }
}

struct PendingSlot<'a>(Option<&'a AtomicUsize>);

impl PendingSlot<'_> {
    fn release(&mut self) {
        if let Some(n) = self.0.take() {
            n.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

impl Drop for PendingSlot<'_> {
    fn drop(&mut self) {
        self.release();
    }
}

/// A running relay; dropping it stops it.
pub struct RelayServer {
    addr: SocketAddr,
    shared: Arc<Shared>,
    accept: Option<JoinHandle<()>>,
    ticker: Option<JoinHandle<()>>,
}

impl RelayServer {
    pub fn start(config: RelayServerConfig) -> Result<Self, RelayServerError> {
        if config.token.is_none() && !config.bind.ip().is_loopback() {
            return Err(RelayServerError::InsecureBind(config.bind));
        }
        let listener = TcpListener::bind(config.bind)?;
        let addr = listener.local_addr()?;
        let info = ServerInfo {
            name: config
                .name
                .clone()
                .unwrap_or_else(|| "ether-collab-relay".into()),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            protocol_version: PROTOCOL_VERSION,
            instance: config.instance.clone(),
            auth_required: config.token.is_some(),
            capabilities: ServerCapabilities {
                collab: true,
                ..ServerCapabilities::default()
            },
        };
        let shared = Arc::new(Shared {
            relay: Mutex::new(Relay::new(config.relay.clone())),
            queues: Mutex::new(HashMap::new()),
            pings: Mutex::new(HashSet::new()),
            kicked: Mutex::new(HashMap::new()),
            started: Instant::now(),
            byte_budget: conn_byte_budget(&config),
            config,
            info,
            stop: AtomicBool::new(false),
            pending: AtomicUsize::new(0),
            next_conn: AtomicU64::new(1),
        });
        let accept = {
            let shared = shared.clone();
            std::thread::Builder::new()
                .name("collab-relay-accept".into())
                .spawn(move || accept_loop(listener, shared))?
        };
        let ticker = {
            let shared = shared.clone();
            std::thread::Builder::new()
                .name("collab-relay-tick".into())
                .spawn(move || {
                    while !shared.stop.load(Ordering::Relaxed) {
                        std::thread::sleep(TICK);
                        shared.with_relay(|_, _| {});
                    }
                })?
        };
        tracing::info!(%addr, auth = shared.info.auth_required, "collab relay listening");
        Ok(Self {
            addr,
            shared,
            accept: Some(accept),
            ticker: Some(ticker),
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// `ws://<addr>`.
    pub fn url(&self) -> String {
        format!("ws://{}", self.addr)
    }

    /// Run `f` on the relay state (tests, diagnostics).
    pub fn with_relay<T>(&self, f: impl FnOnce(&Relay) -> T) -> T {
        f(&self.shared.relay.lock().expect("relay lock"))
    }

    /// Block until the process is terminated.
    pub fn wait(mut self) {
        if let Some(t) = self.accept.take() {
            let _ = t.join();
        }
    }

    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(self.addr);
        if let Some(t) = self.accept.take() {
            let _ = t.join();
        }
        if let Some(t) = self.ticker.take() {
            let _ = t.join();
        }
    }
}

impl Drop for RelayServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn accept_loop(listener: TcpListener, shared: Arc<Shared>) {
    let mut connections: Vec<JoinHandle<()>> = Vec::new();
    for stream in listener.incoming() {
        if shared.stop.load(Ordering::Relaxed) {
            break;
        }
        let Ok(stream) = stream else { continue };
        connections.retain(|c| !c.is_finished());
        if shared.pending.fetch_add(1, Ordering::AcqRel) >= shared.config.max_pending_handshakes {
            shared.pending.fetch_sub(1, Ordering::AcqRel);
            drop(stream);
            continue;
        }
        let shared2 = shared.clone();
        match std::thread::Builder::new()
            .name("collab-relay-conn".into())
            .spawn(move || {
                if let Err(e) = serve_connection(stream, &shared2) {
                    tracing::debug!(%e, "relay connection ended");
                }
            }) {
            Ok(h) => connections.push(h),
            Err(e) => {
                // The closure (and its pending slot accounting) never ran.
                shared.pending.fetch_sub(1, Ordering::AcqRel);
                tracing::warn!(%e, "could not spawn a connection thread");
            }
        }
    }
    for c in connections {
        let _ = c.join();
    }
}

fn would_block(e: &tungstenite::Error) -> bool {
    matches!(e, tungstenite::Error::Io(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut))
}

fn close_with(ws: &mut WebSocket<TcpStream>, code: u16, reason: &str) {
    let _ = ws.close(Some(CloseFrame {
        code: CloseCode::from(code),
        reason: reason.to_string().into(),
    }));
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        match ws.read() {
            Ok(_) => {}
            Err(e) if would_block(&e) => std::thread::sleep(PRE_AUTH_POLL),
            Err(_) => break,
        }
    }
}

fn reject(ws: &mut WebSocket<TcpStream>, reason: HelloRejection, message: &str, code: u16) {
    let hello = ServerHello::Rejected {
        reason,
        message: message.to_string(),
    };
    if let Ok(json) = serde_json::to_string(&hello) {
        let _ = ws.send(Message::text(json));
    }
    close_with(ws, code, message);
}

fn serve_connection(stream: TcpStream, shared: &Shared) -> Result<(), String> {
    let mut slot = PendingSlot(Some(&shared.pending));
    if shared.stop.load(Ordering::Relaxed) {
        return Ok(());
    }
    let deadline = Instant::now() + shared.config.handshake_timeout;
    let expired = || Instant::now() >= deadline;
    stream.set_nodelay(true).ok();
    stream.set_nonblocking(true).map_err(|e| e.to_string())?;
    let unauthenticated = shared.config.token.is_none();
    let path = Arc::new(Mutex::new(String::new()));
    let path_w = path.clone();
    #[allow(clippy::result_large_err)] // tungstenite's callback signature
    let check = move |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
        let refuse = |why: &str, status: StatusCode| {
            let mut r = ErrorResponse::new(Some(why.to_string()));
            *r.status_mut() = status;
            r
        };
        if unauthenticated && let Err(why) = check_unauthenticated_upgrade(req) {
            return Err(refuse(why, StatusCode::FORBIDDEN));
        }
        if session_from_path(req.uri().path()).is_none() {
            return Err(refuse("bad session name", StatusCode::NOT_FOUND));
        }
        *path_w.lock().expect("path lock") = req.uri().path().to_string();
        Ok(resp)
    };
    let pre_auth = WebSocketConfig::default()
        .max_message_size(Some(MAX_PRE_AUTH_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_PRE_AUTH_MESSAGE_BYTES));
    let mut handshake = tungstenite::accept_hdr_with_config(stream, check, Some(pre_auth));
    let mut ws = loop {
        match handshake {
            Ok(ws) => break ws,
            Err(HandshakeError::Interrupted(mid)) => {
                if expired() {
                    return Err("upgrade: deadline".into());
                }
                std::thread::sleep(PRE_AUTH_POLL);
                handshake = mid.handshake();
            }
            Err(HandshakeError::Failure(e)) => return Err(format!("upgrade: {e}")),
        }
    };
    let session = session_from_path(&path.lock().expect("path lock"))
        .ok_or("no session")?
        .to_string();

    let hello = loop {
        if expired() {
            close_with(&mut ws, 1008, "hello deadline");
            return Err("hello: deadline".into());
        }
        match ws.read() {
            Ok(Message::Text(t)) => break t,
            Ok(Message::Ping(_) | Message::Pong(_)) => {}
            Err(e) if would_block(&e) => std::thread::sleep(PRE_AUTH_POLL),
            Err(e) => return Err(format!("hello: {e}")),
            Ok(_) => {
                close_with(&mut ws, 1002, "expected a ClientHello text frame");
                return Err("no hello".into());
            }
        }
    };
    let Ok(hello) = serde_json::from_str::<ClientHello>(&hello) else {
        close_with(&mut ws, 1002, "expected a ClientHello text frame");
        return Err("bad hello".into());
    };
    if hello.protocol_version != PROTOCOL_VERSION {
        let msg = format!(
            "protocol version {} is not supported (relay: {PROTOCOL_VERSION})",
            hello.protocol_version
        );
        reject(
            &mut ws,
            HelloRejection::UnsupportedVersion,
            &msg,
            CLOSE_VERSION,
        );
        return Err(msg);
    }
    if let Some(expected) = &shared.config.token
        && !hello
            .token
            .as_deref()
            .is_some_and(|t| token_matches(expected, t))
    {
        tracing::warn!(client = %loggable(&hello.client), "relay rejected a bad token");
        reject(
            &mut ws,
            HelloRejection::BadToken,
            "invalid token",
            CLOSE_AUTH,
        );
        return Err("bad token".into());
    }
    let conn = shared.next_conn.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = crossbeam_channel::bounded(conn_queue(&shared.config.relay));
    let bytes = Arc::new(AtomicUsize::new(0));
    // Register the queue before the relay can route anything to this connection.
    shared.queues.lock().expect("queues lock").insert(
        conn,
        Queue {
            tx,
            bytes: bytes.clone(),
        },
    );
    if let Err(refusal) = shared
        .relay
        .lock()
        .expect("relay lock")
        .connect(conn, &session)
    {
        shared.queues.lock().expect("queues lock").remove(&conn);
        let msg = refusal.to_string();
        reject(&mut ws, HelloRejection::Busy, &msg, CLOSE_BUSY);
        return Err(msg);
    }
    slot.release();
    let welcome = ServerHello::Welcome {
        server: shared.info.clone(),
        session: session.clone(),
    };
    tracing::info!(%session, conn, client = %loggable(&hello.client), "site connected");
    let result = (|| {
        ws.set_config(|c| {
            c.max_message_size = Some(MAX_COLLAB_MESSAGE_BYTES);
            c.max_frame_size = Some(MAX_COLLAB_MESSAGE_BYTES);
        });
        let socket = ws.get_ref();
        socket.set_nonblocking(false).map_err(|e| e.to_string())?;
        socket
            .set_read_timeout(Some(POLL))
            .map_err(|e| e.to_string())?;
        socket
            .set_write_timeout(Some(shared.config.write_timeout))
            .map_err(|e| e.to_string())?;
        ws.send(Message::text(
            serde_json::to_string(&welcome).map_err(|e| e.to_string())?,
        ))
        .map_err(|e| e.to_string())?;
        pump(&mut ws, &rx, &bytes, conn, shared)
    })();
    shared.queues.lock().expect("queues lock").remove(&conn);
    shared.pings.lock().expect("pings lock").remove(&conn);
    shared.kicked.lock().expect("kicked lock").remove(&conn);
    shared.with_relay(|relay, out| relay.disconnect(conn, out));
    tracing::info!(%session, conn, "site disconnected");
    result
}

fn pump(
    ws: &mut WebSocket<TcpStream>,
    rx: &Receiver<(Arc<CollabMessage>, usize)>,
    bytes: &AtomicUsize,
    conn: ConnId,
    shared: &Shared,
) -> Result<(), String> {
    let mut last_inbound = Instant::now();
    let mut last_ping = Instant::now();
    // Token bucket for the message rate limit, and the presence/pointer/signal throttles.
    let rate = f64::from(shared.config.max_messages_per_second.max(1));
    let mut budget = rate;
    let mut refilled = Instant::now();
    let mut limiter = super::limits::SiteLimiter::default();
    loop {
        if shared.stop.load(Ordering::Relaxed) {
            close_with(ws, 1001, "relay shutting down");
            return Ok(());
        }
        if let Some(why) = shared.kicked.lock().expect("kicked lock").remove(&conn) {
            close_with(ws, CLOSE_DROPPED, &why);
            return Err(why);
        }
        if shared.pings.lock().expect("pings lock").remove(&conn) {
            // The relay asks whether we are alive (another connection claims our site).
            last_ping = Instant::now();
            ws.send(Message::Ping(Default::default()))
                .map_err(|e| e.to_string())?;
        }
        let quiet = last_inbound.elapsed();
        if quiet >= shared.config.idle_timeout {
            close_with(ws, CLOSE_IDLE, "idle timeout");
            return Err("idle timeout".into());
        }
        if quiet >= shared.config.ping_interval
            && last_ping.elapsed() >= shared.config.ping_interval
        {
            last_ping = Instant::now();
            ws.send(Message::Ping(Default::default()))
                .map_err(|e| e.to_string())?;
        }
        let mut wrote = false;
        loop {
            match rx.try_recv() {
                Ok((m, size)) => {
                    bytes.fetch_sub(size, Ordering::AcqRel);
                    let frame = match encode_frame(&m) {
                        WireFrame::Text(t) => Message::text(t),
                        WireFrame::Binary(b) => Message::binary(b),
                    };
                    ws.write(frame).map_err(|e| e.to_string())?;
                    wrote = true;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    let _ = ws.flush();
                    close_with(ws, CLOSE_DROPPED, "site too slow");
                    return Err("outgoing queue closed".into());
                }
            }
        }
        if wrote {
            ws.flush().map_err(|e| e.to_string())?;
        }
        let read = ws.read();
        if read.is_ok() {
            last_inbound = Instant::now();
        }
        let decoded = match read {
            Ok(Message::Text(t)) => decode_text(&t),
            Ok(Message::Binary(b)) => decode_binary(&b),
            Ok(Message::Pong(_)) => {
                // Alive: a contest on our site id (if any) is lost by the newcomer.
                shared.with_relay(|relay, _| relay.heard(conn));
                continue;
            }
            Ok(_) => continue,
            Err(e) if would_block(&e) => continue,
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                return Ok(());
            }
            Err(e) => return Err(e.to_string()),
        };
        let message = match decoded {
            Ok(m) => m,
            Err(e) => {
                tracing::debug!(conn, %e, "malformed collab message");
                continue;
            }
        };
        budget = (budget + refilled.elapsed().as_secs_f64() * rate).min(rate);
        refilled = Instant::now();
        budget -= 1.0;
        if budget < 0.0 {
            tracing::warn!(conn, "site exceeded the message rate limit");
            close_with(ws, 1008, "rate limit");
            return Err("rate limit".into());
        }
        if !limiter.admit(&message, shared.started.elapsed().as_millis() as u64) {
            continue;
        }
        let leave = matches!(message, CollabMessage::Leave { .. });
        let r = shared.with_relay(|relay, out| relay.message(conn, message, out));
        if let Err(e) = r {
            tracing::debug!(conn, reason = %e.reason, "collab message dropped");
            if e.disconnect {
                close_with(ws, 1008, &e.reason);
                return Err(e.reason);
            }
        }
        if leave {
            close_with(ws, 1000, "left");
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use ether_protocol::model::Base64Bytes;

    use super::*;

    fn shared(byte_budget: usize) -> Shared {
        let config = RelayServerConfig::default();
        Shared {
            relay: Mutex::new(Relay::new(config.relay.clone())),
            queues: Mutex::new(HashMap::new()),
            pings: Mutex::new(HashSet::new()),
            kicked: Mutex::new(HashMap::new()),
            started: Instant::now(),
            byte_budget,
            info: ServerInfo {
                name: String::new(),
                app_version: String::new(),
                protocol_version: PROTOCOL_VERSION,
                instance: String::new(),
                auth_required: false,
                capabilities: ServerCapabilities::default(),
            },
            config,
            stop: AtomicBool::new(false),
            pending: AtomicUsize::new(0),
            next_conn: AtomicU64::new(1),
        }
    }

    fn media(bytes: usize) -> Arc<CollabMessage> {
        Arc::new(CollabMessage::Media {
            file: "media/a.wav".into(),
            hash: "h".into(),
            offset: 0,
            total: bytes as u64,
            data: Base64Bytes(vec![0; bytes]),
        })
    }

    #[test]
    fn a_reader_past_its_byte_budget_is_dropped() {
        let s = shared(10_000);
        let (tx, rx) = crossbeam_channel::bounded(1000);
        let bytes = Arc::new(AtomicUsize::new(0));
        s.queues.lock().unwrap().insert(
            1,
            Queue {
                tx,
                bytes: bytes.clone(),
            },
        );
        // Well under the message count, within the byte budget: queued.
        s.route(vec![(1, media(4_000)), (1, media(4_000))]);
        assert!(s.queues.lock().unwrap().contains_key(&1));
        // The reader drains one: its budget frees up.
        let (_, size) = rx.try_recv().unwrap();
        bytes.fetch_sub(size, Ordering::AcqRel);
        s.route(vec![(1, media(4_000))]);
        assert!(s.queues.lock().unwrap().contains_key(&1));
        // Past the budget: the queue is dropped (the connection closes as too slow).
        s.route(vec![(1, media(4_000))]);
        assert!(!s.queues.lock().unwrap().contains_key(&1));
    }

    #[test]
    fn the_byte_budget_fits_a_catch_up() {
        let config = RelayServerConfig::default();
        assert!(conn_byte_budget(&config) >= config.relay.max_media_bytes);
        assert!(conn_byte_budget(&config) >= config.max_queued_bytes);
    }
}

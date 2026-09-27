//! The relay over WebSocket (native): `ether-collab-relay`, dev port base `+3`.
//!
//! Same handshake and hardening as `ether-server` (`ether_protocol::remote`): the session is
//! the URL path (`ws://host:port/<session>`), the first text frame is a `ClientHello` (token
//! compared in constant time; serving without a token is only allowed on loopback, and then
//! only loopback `Host`/`Origin`s may upgrade), answered by a `ServerHello`. Before the hello
//! a connection has an absolute deadline, a 64 KiB message limit and counts against
//! [`RelayServerConfig::max_pending_handshakes`]. Afterwards every frame is one
//! [`CollabMessage`](ether_protocol::collab::CollabMessage) (JSON text, media chunks as
//! binary frames), writes time out and idle connections are pinged, then dropped.
//!
//! Threads: one accept thread, one per connection; the [`Relay`] state machine sits behind
//! a mutex and routes outgoing messages into per-connection bounded queues.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError};
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

const POLL: Duration = Duration::from_millis(4);
const PRE_AUTH_POLL: Duration = Duration::from_millis(10);
const MAX_PRE_AUTH_MESSAGE_BYTES: usize = 64 << 10;
/// At most one presence update per site per this interval (20 Hz); extra ones are dropped.
const PRESENCE_INTERVAL: Duration = Duration::from_millis(50);
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

struct Shared {
    relay: Mutex<Relay>,
    queues: Mutex<HashMap<ConnId, Sender<Arc<CollabMessage>>>>,
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
        let r = f(&mut relay, &mut out);
        self.route(out);
        r
    }

    /// Queue messages for their connections; a full queue drops that connection's sender
    /// (its thread then closes the socket).
    fn route(&self, out: Vec<Outgoing>) {
        if out.is_empty() {
            return;
        }
        let mut queues = self.queues.lock().expect("queues lock");
        for (conn, m) in out {
            if let Some(tx) = queues.get(&conn) {
                match tx.try_send(m) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                        queues.remove(&conn);
                    }
                }
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
        tracing::info!(%addr, auth = shared.info.auth_required, "collab relay listening");
        Ok(Self {
            addr,
            shared,
            accept: Some(accept),
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
    let (tx, rx) =
        crossbeam_channel::bounded::<Arc<CollabMessage>>(conn_queue(&shared.config.relay));
    // Register the queue before the relay can route anything to this connection.
    shared.queues.lock().expect("queues lock").insert(conn, tx);
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
        pump(&mut ws, &rx, conn, shared)
    })();
    shared.queues.lock().expect("queues lock").remove(&conn);
    shared.with_relay(|relay, out| relay.disconnect(conn, out));
    tracing::info!(%session, conn, "site disconnected");
    result
}

fn pump(
    ws: &mut WebSocket<TcpStream>,
    rx: &Receiver<Arc<CollabMessage>>,
    conn: ConnId,
    shared: &Shared,
) -> Result<(), String> {
    let mut last_inbound = Instant::now();
    let mut last_ping = Instant::now();
    // Token bucket for the message rate limit, and the presence throttle.
    let rate = f64::from(shared.config.max_messages_per_second.max(1));
    let mut budget = rate;
    let mut refilled = Instant::now();
    let mut last_presence: Option<Instant> = None;
    loop {
        if shared.stop.load(Ordering::Relaxed) {
            close_with(ws, 1001, "relay shutting down");
            return Ok(());
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
                Ok(m) => {
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
                    close_with(ws, 1008, "site too slow");
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
        if matches!(message, CollabMessage::Presence { .. }) {
            if last_presence.is_some_and(|t| t.elapsed() < PRESENCE_INTERVAL) {
                continue;
            }
            last_presence = Some(Instant::now());
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

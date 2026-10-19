//! `ether-server`: a headless Ethereal engine reachable over WebSocket (roadmap v2; see
//! CONTRACTS.md §11.5).
//!
//! It runs the same native host as the desktop app (`ether-native`: controller, audio
//! backend, disk store, plugins) without a window, and serves the UI protocol described in
//! `ether_protocol::remote` (hello/auth handshake, JSON text frames, binary frames for bulk
//! bytes). The browser UI connects with `ui/src/transport/ws/WsTransport`.
//!
//! - **One controller, many clients.** Every client sees every patch and event; replies go
//!   to the client that sent the request ([`router`]).
//! - **Auth.** A shared token in `ClientHello::token`, compared in constant time. Serving
//!   without a token is only allowed on loopback. Tokens are never logged.
//! - **Hardening.** Before the hello is accepted a connection has an absolute deadline
//!   ([`ServerConfig::handshake_timeout`]), a 64 KiB message limit, and counts against
//!   [`ServerConfig::max_pending_handshakes`]. Afterwards writes time out
//!   ([`ServerConfig::write_timeout`]) and the server pings idle clients, dropping them after
//!   [`ServerConfig::idle_timeout`] without any frame. Without a token, upgrades whose `Host`
//!   or `Origin` is not loopback are refused (no web page can drive a local server).
//! - **Threads.** One accept thread, one thread per connection (blocking tungstenite with
//!   a short read timeout, so it can also write what the router queued), plus a janitor
//!   that removes abandoned upload staging files.
//!
//! Ports: never hardcoded. `ServerConfig::bind` defaults to loopback on port 0 (OS-chosen);
//! dev instances use the `+4` offset (`PORT_OFFSETS.remote`) of the instance base port
//! (`scripts/dev-env.mjs`, README "Running multiple dev instances").

pub mod agent_bridge;
pub mod cli;
pub mod frames;
pub mod router;

use std::io::ErrorKind;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::TryRecvError;
use ether_native::{NativeHost, PatchCompat};
use ether_protocol::remote::{ClientHello, HelloRejection, ServerHello};
use ether_protocol::{CommandError, ErrorCode, Reply, ReplyResult, ServerMessage};
use tungstenite::handshake::HandshakeError;
use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::http::StatusCode;
use tungstenite::protocol::frame::coding::CloseCode;
use tungstenite::protocol::{CloseFrame, WebSocketConfig};
use tungstenite::{Message, WebSocket};

pub use ether_protocol::remote::{PROTOCOL_VERSION, ServerCapabilities, ServerInfo};

use crate::frames::{DecodeError, Frame, decode_client_binary, decode_client_text};
use crate::router::{CLIENT_QUEUE, ClientId, Router};

/// Close codes after a `Rejected` hello (`ether_protocol::remote`).
pub const CLOSE_AUTH: u16 = 4001;
pub const CLOSE_VERSION: u16 = 4002;
pub const CLOSE_BUSY: u16 = 4003;
/// Close code for clients dropped after [`ServerConfig::idle_timeout`].
pub const CLOSE_IDLE: u16 = 4004;

/// Read poll interval of a connection (bounds the latency of pushed messages).
const POLL: Duration = Duration::from_millis(4);
/// Poll interval while a connection is not authenticated yet (non-blocking socket).
const PRE_AUTH_POLL: Duration = Duration::from_millis(10);
/// Largest accepted WebSocket message (a 1 MiB upload chunk in base64 JSON fits).
pub const MAX_MESSAGE_BYTES: usize = 4 << 20;
/// Largest accepted message before the hello was accepted.
pub const MAX_PRE_AUTH_MESSAGE_BYTES: usize = 64 << 10;
/// Upload staging files untouched for this long are deleted by the janitor.
pub const STALE_UPLOAD_AGE: Duration = Duration::from_secs(30 * 60);
const JANITOR_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, PartialEq)]
pub struct ServerConfig {
    /// Listen address. Default: `127.0.0.1:0` (loopback, OS-chosen port).
    pub bind: SocketAddr,
    /// Shared secret clients must send in `ClientHello::token`. `None` = no auth, only
    /// allowed on loopback addresses.
    pub token: Option<String>,
    /// Display name in `ServerInfo` (default: host name).
    pub name: Option<String>,
    /// Allow a second client while one is connected (default: true; all clients share the
    /// one controller). `false` rejects extra clients with `Busy`.
    pub allow_multiple_clients: bool,
    /// Upper bound on simultaneous clients (`Busy` beyond it).
    pub max_clients: usize,
    /// Dev instance id reported in `ServerInfo` (`ETHER_INSTANCE`).
    pub instance: String,
    /// The host's `projects_root`, for the upload janitor (`None`: no janitor).
    pub projects_root: Option<PathBuf>,
    /// Absolute deadline for the HTTP upgrade plus the hello (default 10 s).
    pub handshake_timeout: Duration,
    /// Connections still in the upgrade/hello phase; more are closed on accept (default 32).
    pub max_pending_handshakes: usize,
    /// A write (or flush) blocked this long drops the client (default 10 s).
    pub write_timeout: Duration,
    /// Ping a client after this long without an inbound frame (default 20 s).
    pub ping_interval: Duration,
    /// Drop a client after this long without an inbound frame (pongs count; default 60 s).
    pub idle_timeout: Duration,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: SocketAddr::from(([127, 0, 0, 1], 0)),
            token: None,
            name: None,
            allow_multiple_clients: true,
            max_clients: 16,
            instance: ether_native::instance::instance_id(),
            projects_root: None,
            handshake_timeout: Duration::from_secs(10),
            max_pending_handshakes: 32,
            write_timeout: Duration::from_secs(10),
            ping_interval: Duration::from_secs(20),
            idle_timeout: Duration::from_secs(60),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("refusing to serve without a token on non-loopback address {0}")]
    InsecureBind(SocketAddr),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("engine: {0}")]
    Host(String),
}

/// Constant-time token comparison (length is not secret).
pub fn token_matches(expected: &str, given: &str) -> bool {
    let (a, b) = (expected.as_bytes(), given.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// `n` random bytes from the OS as lowercase hex (tokens, session ids). Fails rather than
/// falling back to guessable values.
pub fn random_hex(n: usize) -> Result<String, String> {
    let mut bytes = vec![0u8; n];
    getrandom::fill(&mut bytes).map_err(|e| format!("no OS entropy: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// A client-supplied string made safe for logs: at most 80 chars, control and quote
/// characters escaped.
pub fn loggable(s: &str) -> String {
    s.chars().take(80).flat_map(char::escape_debug).collect()
}

/// `true` if a `Host` header value (`name[:port]`) names the loopback interface. Anything
/// after the name other than a numeric port (`[::1]junk`, `localhost:x`) is refused.
pub fn is_loopback_host(host: &str) -> bool {
    // Nothing, or `:port` (decimal digits only, `+80` is not a port; RFC 3986 allows an
    // empty one).
    let port_ok = |rest: &str| {
        rest.is_empty()
            || rest.strip_prefix(':').is_some_and(|p| {
                p.is_empty() || (p.bytes().all(|b| b.is_ascii_digit()) && p.parse::<u16>().is_ok())
            })
    };
    let host = host.trim();
    let name = if let Some(rest) = host.strip_prefix('[') {
        match rest.split_once(']') {
            Some((inner, after)) if port_ok(after) => inner,
            _ => return false,
        }
    } else if host.matches(':').count() == 1 {
        let (name, port) = host.split_at(host.find(':').unwrap_or(host.len()));
        if !port_ok(port) {
            return false;
        }
        name
    } else {
        host
    };
    name.eq_ignore_ascii_case("localhost")
        || name
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// `true` if an `Origin` header value (`scheme://host[:port]`) is a loopback page.
pub fn is_loopback_origin(origin: &str) -> bool {
    let Some((scheme, rest)) = origin.trim().split_once("://") else {
        return false; // includes "null" (sandboxed/file pages)
    };
    matches!(scheme, "http" | "https") && is_loopback_host(rest.split('/').next().unwrap_or(""))
}

/// Upgrade policy without a token: only loopback `Host`s, and only loopback `Origin`s
/// when a browser sends one (blocks other web pages and DNS rebinding).
fn check_unauthenticated_upgrade(req: &Request) -> Result<(), &'static str> {
    let header = |name: &str| req.headers().get(name).and_then(|v| v.to_str().ok());
    if !header("host").is_some_and(is_loopback_host) {
        return Err("Host must be a loopback address when the server has no token");
    }
    if let Some(origin) = header("origin")
        && !is_loopback_origin(origin)
    {
        return Err("Origin must be a loopback page when the server has no token");
    }
    Ok(())
}

/// Host name for `ServerInfo::name`.
fn default_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "ether-server".into())
}

/// The engine a [`Server`] forwards client messages to.
///
/// [`Server::start`] owns a [`NativeHost`]; [`Server::attach`] serves an engine that
/// somebody else owns and also drives (the desktop app's agent bridge, `agent-api`: the UI
/// and the agents share one controller).
pub trait Engine: Send + Sync {
    /// Queue one (already id-remapped) client message.
    fn send(&self, message: ether_protocol::ClientMessage) -> Result<(), String>;

    /// Where clients that predate `PatchChange::DeviceParam` register while connected
    /// (audit #4). `None`: the engine has no such switch (it always sends field patches).
    fn patch_compat(&self) -> Option<PatchCompat> {
        None
    }
}

impl Engine for NativeHost {
    fn send(&self, message: ether_protocol::ClientMessage) -> Result<(), String> {
        NativeHost::send(self, message).map_err(|e| e.to_string())
    }

    fn patch_compat(&self) -> Option<PatchCompat> {
        Some(NativeHost::patch_compat(self).clone())
    }
}

impl<E: Engine + ?Sized> Engine for Arc<E> {
    fn send(&self, message: ether_protocol::ClientMessage) -> Result<(), String> {
        (**self).send(message)
    }

    fn patch_compat(&self) -> Option<PatchCompat> {
        (**self).patch_compat()
    }
}

enum HostRef {
    /// Started with [`Server::start`]: the router is the host's subscriber.
    Owned(Box<NativeHost>),
    /// Started with [`Server::attach`]: the owner feeds [`Router::outbound`].
    Attached(Box<dyn Engine>),
}

impl HostRef {
    fn send(&self, message: ether_protocol::ClientMessage) -> Result<(), String> {
        match self {
            Self::Owned(h) => Engine::send(&**h, message),
            Self::Attached(e) => e.send(message),
        }
    }

    fn patch_compat(&self) -> Option<PatchCompat> {
        match self {
            Self::Owned(h) => Engine::patch_compat(&**h),
            Self::Attached(e) => e.patch_compat(),
        }
    }
}

struct Shared {
    host: HostRef,
    router: Arc<Router>,
    config: ServerConfig,
    info: ServerInfo,
    stop: AtomicBool,
    /// Connections in the upgrade/hello phase.
    pending: Arc<AtomicUsize>,
}

/// Counts a connection as pending until dropped (or released after the hello). Owned, so
/// the accept loop can move it into the connection thread: if that thread can't be
/// spawned, the closure (and the slot with it) is dropped and the slot comes back.
struct PendingSlot(Option<Arc<AtomicUsize>>);

impl PendingSlot {
    /// Take a slot unless `max` are already taken.
    fn acquire(pending: &Arc<AtomicUsize>, max: usize) -> Option<Self> {
        if pending.fetch_add(1, Ordering::AcqRel) >= max {
            pending.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
        Some(Self(Some(pending.clone())))
    }

    fn release(&mut self) {
        if let Some(n) = self.0.take() {
            n.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

impl Drop for PendingSlot {
    fn drop(&mut self) {
        self.release();
    }
}

/// A running server. Dropping it (or [`Server::shutdown`]) stops accepting, closes every
/// connection and shuts the engine down.
pub struct Server {
    addr: SocketAddr,
    shared: Arc<Shared>,
    accept: Option<JoinHandle<()>>,
    janitor: Option<JoinHandle<()>>,
}

impl Server {
    /// Bind and start serving `host` in background threads.
    pub fn start(config: ServerConfig, host: NativeHost) -> Result<Self, ServerError> {
        let router = Arc::new(Router::default());
        {
            let router = router.clone();
            host.subscribe(Arc::new(move |m| router.outbound(m)));
        }
        Self::start_with(config, HostRef::Owned(Box::new(host)), router)
    }

    /// Serve an engine owned (and possibly also driven) by the caller, which must pass
    /// every `ServerMessage` the engine emits to [`Router::outbound`] of `router`. Use a
    /// router with an id base ([`Router::with_id_base`]) when other clients number
    /// requests and gestures in the same engine, and send it only the replies it
    /// [owns](Router::owns_reply).
    pub fn attach(
        config: ServerConfig,
        engine: Box<dyn Engine>,
        router: Arc<Router>,
    ) -> Result<Self, ServerError> {
        Self::start_with(config, HostRef::Attached(engine), router)
    }

    fn start_with(
        config: ServerConfig,
        host: HostRef,
        router: Arc<Router>,
    ) -> Result<Self, ServerError> {
        if config.token.is_none() && !config.bind.ip().is_loopback() {
            if let HostRef::Owned(h) = &host {
                h.unsubscribe();
            }
            return Err(ServerError::InsecureBind(config.bind));
        }
        let listener = match TcpListener::bind(config.bind) {
            Ok(l) => l,
            Err(e) => {
                if let HostRef::Owned(h) = &host {
                    h.unsubscribe();
                }
                return Err(e.into());
            }
        };
        let addr = listener.local_addr()?;
        let info = ServerInfo {
            name: config.name.clone().unwrap_or_else(default_name),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            protocol_version: PROTOCOL_VERSION,
            instance: config.instance.clone(),
            auth_required: config.token.is_some(),
            capabilities: ServerCapabilities {
                plugins: true,
                recording: true,
                upload: true,
                export: true,
                collab: false,
            },
        };
        if let Some(root) = &config.projects_root {
            // Whatever is staged from a previous run belongs to connections that are gone.
            let n = ether_native::uploads::remove_stale(root, Duration::ZERO);
            if n > 0 {
                tracing::info!(files = n, "removed abandoned upload staging files");
            }
        }
        let shared = Arc::new(Shared {
            host,
            router,
            config,
            info,
            stop: AtomicBool::new(false),
            pending: Arc::new(AtomicUsize::new(0)),
        });
        let accept = {
            let shared = shared.clone();
            std::thread::Builder::new()
                .name("ether-server-accept".into())
                .spawn(move || accept_loop(listener, shared))?
        };
        let janitor = match shared.config.projects_root.clone() {
            Some(root) => {
                let shared = shared.clone();
                Some(
                    std::thread::Builder::new()
                        .name("ether-server-janitor".into())
                        .spawn(move || {
                            let mut last = Instant::now();
                            while !shared.stop.load(Ordering::Relaxed) {
                                std::thread::sleep(Duration::from_millis(200));
                                if last.elapsed() >= JANITOR_INTERVAL {
                                    last = Instant::now();
                                    ether_native::uploads::remove_stale(&root, STALE_UPLOAD_AGE);
                                }
                            }
                        })?,
                )
            }
            None => None,
        };
        tracing::info!(%addr, auth = shared.info.auth_required, "ether-server listening");
        Ok(Self {
            addr,
            shared,
            accept: Some(accept),
            janitor,
        })
    }

    /// The bound address (with the OS-chosen port).
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn info(&self) -> &ServerInfo {
        &self.shared.info
    }

    /// Connected clients.
    pub fn client_count(&self) -> usize {
        self.shared.router.client_count()
    }

    /// Connections still in the pre-auth (handshake + hello) phase, i.e. holding one of the
    /// [`ServerConfig::max_pending_handshakes`] slots (diagnostics and tests).
    pub fn pending_handshakes(&self) -> usize {
        self.shared.pending.load(Ordering::Acquire)
    }

    /// Block until the process is terminated.
    pub fn wait(mut self) {
        if let Some(t) = self.accept.take() {
            let _ = t.join();
        }
    }

    /// Graceful stop: stop accepting, close every client with `1001` ("going away") after
    /// writing what was queued for it, then (for [`Server::start`]) shut the engine down,
    /// which saves a dirty project and waits for pending saves. Returns once that is done.
    pub fn shutdown(self) {
        // `Drop` stops the threads; then every holder of `shared` is gone, so dropping it
        // drops the owned `NativeHost`, whose shutdown saves, before this returns.
        drop(self);
    }

    /// Idempotent.
    fn stop(&mut self) {
        if self.shared.stop.swap(true, Ordering::Relaxed) {
            return;
        }
        // Unblock `accept`.
        let _ = TcpStream::connect(self.addr);
        if let Some(t) = self.accept.take() {
            let _ = t.join();
        }
        if let Some(t) = self.janitor.take() {
            let _ = t.join();
        }
        if let HostRef::Owned(h) = &self.shared.host {
            h.unsubscribe();
        }
    }
}

impl Drop for Server {
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
        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(%e, "accept failed");
                continue;
            }
        };
        connections.retain(|c| !c.is_finished());
        let Some(slot) =
            PendingSlot::acquire(&shared.pending, shared.config.max_pending_handshakes)
        else {
            tracing::debug!("too many connections in the handshake phase: closing one");
            drop(stream);
            continue;
        };
        let conn = shared.clone();
        // The slot moves into the thread; a failed spawn drops it (and the stream) here.
        match std::thread::Builder::new()
            .name("ether-server-conn".into())
            .spawn(move || {
                let peer = stream.peer_addr().ok();
                if let Err(e) = serve_connection(stream, &conn, slot) {
                    tracing::debug!(?peer, %e, "connection ended");
                }
            }) {
            Ok(h) => connections.push(h),
            Err(e) => tracing::warn!(%e, "could not spawn a connection thread"),
        }
    }
    for c in connections {
        let _ = c.join();
    }
}

fn pre_auth_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_PRE_AUTH_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_PRE_AUTH_MESSAGE_BYTES))
}

fn would_block(e: &tungstenite::Error) -> bool {
    matches!(e, tungstenite::Error::Io(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut))
}

fn close_with(ws: &mut WebSocket<TcpStream>, code: u16, reason: &str) {
    let _ = ws.close(Some(CloseFrame {
        code: CloseCode::from(code),
        reason: reason.to_string().into(),
    }));
    // Let the close handshake complete (bounded by the read timeout).
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

/// Handshake, then pump frames both ways until either side closes.
fn serve_connection(
    stream: TcpStream,
    shared: &Shared,
    // Released when the hello phase ends (or on any early return).
    mut slot: PendingSlot,
) -> Result<(), String> {
    if shared.stop.load(Ordering::Relaxed) {
        return Ok(());
    }
    let deadline = Instant::now() + shared.config.handshake_timeout;
    let expired = || Instant::now() >= deadline;
    let peer = stream.peer_addr().map_err(|e| e.to_string())?;
    stream.set_nodelay(true).ok();
    // Non-blocking until the hello is accepted, so the deadline is absolute (a client
    // trickling bytes can't extend it).
    stream.set_nonblocking(true).map_err(|e| e.to_string())?;
    let unauthenticated = shared.config.token.is_none();
    #[allow(clippy::result_large_err)] // tungstenite's callback signature
    let check = move |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
        if unauthenticated && let Err(why) = check_unauthenticated_upgrade(req) {
            let mut r = ErrorResponse::new(Some(why.to_string()));
            *r.status_mut() = StatusCode::FORBIDDEN;
            return Err(r);
        }
        Ok(resp)
    };
    let mut handshake = tungstenite::accept_hdr_with_config(stream, check, Some(pre_auth_config()));
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
            Err(HandshakeError::Failure(e)) => {
                tracing::debug!(%peer, %e, "upgrade refused");
                return Err(format!("upgrade: {e}"));
            }
        }
    };

    // --- Hello.
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
    let hello: ClientHello = match serde_json::from_str(&hello) {
        Ok(h) => h,
        Err(_) => {
            close_with(&mut ws, 1002, "expected a ClientHello text frame");
            return Err("bad hello".into());
        }
    };
    if hello.protocol_version != PROTOCOL_VERSION {
        let msg = format!(
            "protocol version {} is not supported (server: {PROTOCOL_VERSION})",
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
    if let Some(expected) = &shared.config.token {
        let ok = hello
            .token
            .as_deref()
            .is_some_and(|t| token_matches(expected, t));
        if !ok {
            // Never log the token itself.
            let client = loggable(&hello.client);
            tracing::warn!(%peer, %client, "rejected a client with a bad token");
            reject(
                &mut ws,
                HelloRejection::BadToken,
                "invalid token",
                CLOSE_AUTH,
            );
            return Err("bad token".into());
        }
    }
    let session = match random_hex(16) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(%e, "cannot create a session id");
            close_with(&mut ws, 1011, "server error");
            return Err(e);
        }
    };
    let (tx, rx) = crossbeam_channel::bounded::<Frame>(CLIENT_QUEUE);
    let (max, multiple) = (
        shared.config.max_clients,
        shared.config.allow_multiple_clients,
    );
    let Ok(client) = shared
        .router
        .add_if(tx, |n| n < max && (multiple || n == 0))
    else {
        reject(
            &mut ws,
            HelloRejection::Busy,
            "another client is connected",
            CLOSE_BUSY,
        );
        return Err("busy".into());
    };
    slot.release();
    let welcome = ServerHello::Welcome {
        server: shared.info.clone(),
        session: session.clone(),
    };
    let client_name = loggable(&hello.client);
    // Audit #4: a client that predates `PatchChange::DeviceParam` gets whole-device upserts
    // (for everyone) while it is connected.
    let _legacy = (!hello.supports(ether_protocol::remote::FEATURE_DEVICE_PARAM_PATCH))
        .then(|| shared.host.patch_compat().map(|c| c.legacy_client()))
        .flatten();
    tracing::info!(%peer, client = %client_name, %session, "client connected");
    let result = (|| {
        // Authenticated: normal limits, blocking socket with a short read timeout (to
        // interleave writes) and a write timeout (a client that stops reading can't block
        // the thread forever).
        ws.set_config(|c| {
            c.max_message_size = Some(MAX_MESSAGE_BYTES);
            c.max_frame_size = Some(MAX_MESSAGE_BYTES);
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
        pump(&mut ws, &rx, client, shared)
    })();
    for m in shared.router.remove(client) {
        let _ = shared.host.send(m);
    }
    tracing::info!(%peer, %session, "client disconnected");
    result
}

fn pump(
    ws: &mut WebSocket<TcpStream>,
    rx: &crossbeam_channel::Receiver<Frame>,
    client: ClientId,
    shared: &Shared,
) -> Result<(), String> {
    let mut last_inbound = Instant::now();
    let mut last_ping = Instant::now();
    loop {
        if shared.stop.load(Ordering::Relaxed) {
            // Deliver what was already queued (replies to requests just made), then close.
            while let Ok(f) = rx.try_recv() {
                let m = match f {
                    Frame::Text(t) => Message::text(t),
                    Frame::Binary(b) => Message::binary(b),
                };
                if ws.write(m).is_err() {
                    break;
                }
            }
            let _ = ws.flush();
            close_with(ws, 1001, "server shutting down");
            return Ok(());
        }
        // Liveness: ping a quiet client; drop one that stays silent (half-open socket,
        // frozen tab). Browsers answer pings automatically.
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
        // Outgoing: everything the router queued.
        let mut wrote = false;
        loop {
            match rx.try_recv() {
                Ok(Frame::Text(t)) => ws.write(Message::text(t)),
                Ok(Frame::Binary(b)) => ws.write(Message::binary(b)),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    let _ = ws.flush();
                    close_with(ws, 1008, "client too slow");
                    return Err("outgoing queue closed".into());
                }
            }
            .map_err(|e| e.to_string())?;
            wrote = true;
        }
        if wrote {
            ws.flush().map_err(|e| e.to_string())?;
        }
        // Incoming (waits at most `POLL`).
        let read = ws.read();
        if read.is_ok() {
            last_inbound = Instant::now();
        }
        let decoded = match read {
            Ok(Message::Text(t)) => decode_client_text(&t),
            Ok(Message::Binary(b)) => decode_client_binary(&b),
            Ok(_) => continue,
            Err(e) if would_block(&e) => continue,
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                return Ok(());
            }
            Err(e) => return Err(e.to_string()),
        };
        match decoded {
            Ok(m) => match shared.router.inbound(client, m) {
                Ok(Some(m)) => shared.host.send(m).map_err(|e| e.to_string())?,
                Ok(None) => {}
                Err(reply) => {
                    let json = serde_json::to_string(&reply).map_err(|e| e.to_string())?;
                    ws.send(Message::text(json)).map_err(|e| e.to_string())?;
                }
            },
            Err(DecodeError { id, message }) => {
                // Answer malformed requests we can identify; ignore the rest.
                tracing::debug!(%message, "malformed client message");
                if let Some(id) = id {
                    let reply = ServerMessage::Reply(Reply {
                        id,
                        result: ReplyResult::Err {
                            error: CommandError {
                                code: ErrorCode::InvalidArgument,
                                message,
                                key: None,
                            },
                        },
                    });
                    let json = serde_json::to_string(&reply).map_err(|e| e.to_string())?;
                    ws.send(Message::text(json)).map_err(|e| e.to_string())?;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_hosts_allow_only_a_numeric_port_after_the_name() {
        for good in [
            "localhost",
            "LOCALHOST:5173",
            "127.0.0.1",
            "127.0.0.1:20000",
            "127.0.0.1:",
            "[::1]",
            "[::1]:8080",
            "::1",
        ] {
            assert!(is_loopback_host(good), "{good}");
        }
        for bad in [
            "[::1]junk",
            "[::1].evil.example",
            "[::1]:80junk",
            "[::1]:+80",
            "[::1]:99999",
            "[::1",
            "localhost:x",
            "localhost:+1",
            "127.0.0.1:80/",
            "127.0.0.1.evil.example",
            "evil.example",
            "[::2]",
            "",
        ] {
            assert!(!is_loopback_host(bad), "{bad}");
        }
        assert!(!is_loopback_origin("http://[::1]junk"));
        assert!(is_loopback_origin("http://[::1]:5173/app"));
    }

    #[test]
    fn pending_slots_are_capped_and_come_back_when_the_thread_cannot_spawn() {
        let pending = Arc::new(AtomicUsize::new(0));
        let a = PendingSlot::acquire(&pending, 2).unwrap();
        let mut b = PendingSlot::acquire(&pending, 2).unwrap();
        assert!(PendingSlot::acquire(&pending, 2).is_none());
        assert_eq!(pending.load(Ordering::Acquire), 2);
        b.release();
        b.release();
        drop(b);
        assert_eq!(pending.load(Ordering::Acquire), 1, "released once");
        // What `accept_loop` does: the slot moves into the connection thread. An impossible
        // stack makes the spawn fail; the slot must not leak (it would shrink the cap for
        // good).
        let spawned = std::thread::Builder::new()
            .stack_size(1 << 60)
            .spawn(move || drop(a));
        assert!(spawned.is_err(), "the spawn was expected to fail");
        assert_eq!(pending.load(Ordering::Acquire), 0);
    }
}

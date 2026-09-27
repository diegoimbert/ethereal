//! `ether-server`: a headless Ethereal engine reachable over WebSocket (roadmap v2, owned by
//! the `remote-engine` node; see `docs/ROADMAP.md`).
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
//! - **Threads.** One accept thread, one thread per connection (blocking tungstenite with
//!   a short read timeout, so it can also write what the router queued), plus a janitor
//!   that removes abandoned upload staging files.
//!
//! Ports: never hardcoded. `ServerConfig::bind` defaults to loopback on port 0 (OS-chosen);
//! dev instances use the `+4` offset (`PORT_OFFSETS.remote`) of the instance base port
//! (`scripts/dev-env.mjs`, README "Running multiple dev instances").

pub mod cli;
pub mod frames;
pub mod router;

use std::io::ErrorKind;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::TryRecvError;
use ether_native::NativeHost;
use ether_protocol::remote::{ClientHello, HelloRejection, ServerHello};
use ether_protocol::{CommandError, ErrorCode, Reply, ReplyResult, ServerMessage};
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

/// Time a client has to complete the HTTP upgrade and send its hello.
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
/// Read poll interval of a connection (bounds the latency of pushed messages).
const POLL: Duration = Duration::from_millis(4);
/// Largest accepted WebSocket message (a 1 MiB upload chunk in base64 JSON fits).
pub const MAX_MESSAGE_BYTES: usize = 4 << 20;
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

/// `n` random bytes as lowercase hex (tokens, session ids).
pub fn random_hex(n: usize) -> String {
    let mut bytes = vec![0u8; n];
    if getrandom::fill(&mut bytes).is_err() {
        // No OS entropy: fall back to clock + pid mixing (still unguessable enough for a
        // session id; tokens are generated at most once per install).
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()) as u64
            ^ u64::from(std::process::id()) << 32;
        let mut x = seed | 1;
        for b in &mut bytes {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *b = x as u8;
        }
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
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

struct Shared {
    host: NativeHost,
    router: Arc<Router>,
    config: ServerConfig,
    info: ServerInfo,
    stop: AtomicBool,
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
        if config.token.is_none() && !config.bind.ip().is_loopback() {
            return Err(ServerError::InsecureBind(config.bind));
        }
        let listener = TcpListener::bind(config.bind)?;
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
        let router = Arc::new(Router::default());
        {
            let router = router.clone();
            host.subscribe(Arc::new(move |m| router.outbound(m)));
        }
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
        // Unblock `accept`.
        let _ = TcpStream::connect(self.addr);
        if let Some(t) = self.accept.take() {
            let _ = t.join();
        }
        if let Some(t) = self.janitor.take() {
            let _ = t.join();
        }
        self.shared.host.unsubscribe();
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
        let shared = shared.clone();
        match std::thread::Builder::new()
            .name("ether-server-conn".into())
            .spawn(move || {
                let peer = stream.peer_addr().ok();
                if let Err(e) = serve_connection(stream, &shared) {
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

fn ws_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES))
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
            Err(tungstenite::Error::Io(e))
                if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
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
fn serve_connection(stream: TcpStream, shared: &Shared) -> Result<(), String> {
    if shared.stop.load(Ordering::Relaxed) {
        return Ok(());
    }
    let peer = stream.peer_addr().map_err(|e| e.to_string())?;
    stream.set_nodelay(true).ok();
    stream
        .set_read_timeout(Some(HELLO_TIMEOUT))
        .map_err(|e| e.to_string())?;
    let mut ws = tungstenite::accept_with_config(stream, Some(ws_config()))
        .map_err(|e| format!("upgrade: {e}"))?;

    // --- Hello.
    let hello = loop {
        match ws.read().map_err(|e| format!("hello: {e}"))? {
            Message::Text(t) => break t,
            Message::Ping(_) | Message::Pong(_) => {}
            _ => {
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
            tracing::warn!(%peer, client = %hello.client, "rejected a client with a bad token");
            reject(
                &mut ws,
                HelloRejection::BadToken,
                "invalid token",
                CLOSE_AUTH,
            );
            return Err("bad token".into());
        }
    }
    let count = shared.router.client_count();
    if count >= shared.config.max_clients || (!shared.config.allow_multiple_clients && count > 0) {
        reject(
            &mut ws,
            HelloRejection::Busy,
            "another client is connected",
            CLOSE_BUSY,
        );
        return Err("busy".into());
    }
    let (tx, rx) = crossbeam_channel::bounded::<Frame>(CLIENT_QUEUE);
    let client = shared.router.add(tx);
    let session = random_hex(16);
    let welcome = ServerHello::Welcome {
        server: shared.info.clone(),
        session: session.clone(),
    };
    tracing::info!(%peer, client = %hello.client, %session, "client connected");
    let result = (|| {
        ws.send(Message::text(
            serde_json::to_string(&welcome).map_err(|e| e.to_string())?,
        ))
        .map_err(|e| e.to_string())?;
        ws.get_ref()
            .set_read_timeout(Some(POLL))
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
    loop {
        if shared.stop.load(Ordering::Relaxed) {
            close_with(ws, 1001, "server shutting down");
            return Ok(());
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
        let decoded = match ws.read() {
            Ok(Message::Text(t)) => decode_client_text(&t),
            Ok(Message::Binary(b)) => decode_client_binary(&b),
            Ok(_) => continue,
            Err(tungstenite::Error::Io(e))
                if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
            {
                continue;
            }
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                return Ok(());
            }
            Err(e) => return Err(e.to_string()),
        };
        match decoded {
            Ok(m) => {
                if let Some(m) = shared.router.inbound(client, m) {
                    shared.host.send(m).map_err(|e| e.to_string())?;
                }
            }
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

//! Remote engines over the remote-engine protocol (CONTRACTS §11.5): the desktop app's
//! agent bridge or an `ether-server`.
//!
//! One I/O thread per connection owns the socket: it writes queued requests, reads every
//! frame (answering the server's pings, so an idle MCP session is not dropped) and hands
//! replies to their callers; events are ignored. A lost connection is re-established on the
//! next request (for the desktop app, from a fresh read of its runtime file, so restarting
//! the app or re-enabling the bridge just works).

use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ether_protocol::remote::{ClientHello, PROTOCOL_VERSION, ServerHello};
use ether_protocol::{ClientMessage, Command, ReplyValue, ServerMessage};
use ether_server::agent_bridge::{RuntimeInfo, read_runtime_file};
use ether_server::frames::{Frame, decode_server};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use crate::backend::{Backend, Pending, wait};

/// Read poll of the I/O thread (bounds the latency of writes).
const POLL: Duration = Duration::from_millis(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Where to connect.
#[derive(Clone, Debug)]
pub enum Target {
    /// An explicit server (`--server`, `--token`).
    Fixed { url: String, token: Option<String> },
    /// The desktop app's agent bridge: the first existing runtime file of `candidates`.
    Desktop { candidates: Vec<PathBuf> },
}

/// Message shown when the desktop app can't be reached.
pub fn desktop_unreachable(candidates: &[PathBuf], why: &str) -> String {
    let files: Vec<String> = candidates.iter().map(|p| p.display().to_string()).collect();
    format!(
        "Ethereal is not reachable ({why}). Open the Ethereal desktop app and turn on \
         'Allow AI agents (MCP)' in its settings (it writes {}). Or run ether-mcp with \
         --project <file.ether> (headless) or --server ws://HOST:PORT --token TOKEN.",
        files.join(" or ")
    )
}

impl Target {
    /// URL and token to connect to now.
    fn resolve(&self) -> Result<(String, Option<String>), String> {
        match self {
            Self::Fixed { url, token } => Ok((url.clone(), token.clone())),
            Self::Desktop { candidates } => {
                let mut why = "the agent bridge is off".to_string();
                for path in candidates {
                    match read_runtime_file(path) {
                        Ok(RuntimeInfo { port, token, .. }) => {
                            return Ok((format!("ws://127.0.0.1:{port}/"), Some(token)));
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => why = format!("cannot read {}: {e}", path.display()),
                    }
                }
                Err(desktop_unreachable(candidates, &why))
            }
        }
    }
}

struct Connection {
    tx: Sender<ClientMessage>,
    alive: Arc<AtomicBool>,
}

/// A remote engine.
pub struct RemoteBackend {
    target: Target,
    pending: Arc<Pending>,
    conn: Mutex<Option<Connection>>,
}

impl RemoteBackend {
    pub fn new(target: Target) -> Self {
        Self {
            target,
            pending: Arc::default(),
            conn: Mutex::new(None),
        }
    }

    /// Connect now (to fail fast on a bad `--server`/`--token`).
    pub fn connect(&self) -> Result<(), String> {
        self.sender().map(|_| ())
    }

    fn sender(&self) -> Result<Sender<ClientMessage>, String> {
        let mut conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(c) = conn.as_ref()
            && c.alive.load(Ordering::Acquire)
        {
            return Ok(c.tx.clone());
        }
        let (url, token) = self.target.resolve()?;
        let c =
            open(&url, token.as_deref(), self.pending.clone()).map_err(|e| match &self.target {
                Target::Desktop { candidates } => desktop_unreachable(candidates, &e),
                Target::Fixed { .. } => format!("cannot connect to {url}: {e}"),
            })?;
        let tx = c.tx.clone();
        *conn = Some(c);
        Ok(tx)
    }
}

impl Backend for RemoteBackend {
    fn request(&self, command: Command) -> Result<ReplyValue, String> {
        let tx = self.sender()?;
        let (id, rx) = self.pending.register();
        let msg = ClientMessage {
            id,
            gesture: None,
            command,
        };
        if tx.send(msg).is_err() {
            self.pending.forget(id);
            return Err("the connection to Ethereal was lost; try again".into());
        }
        wait(&self.pending, id, rx)
    }

    fn describe(&self) -> String {
        match &self.target {
            Target::Fixed { url, .. } => format!("the Ethereal engine at {url}"),
            Target::Desktop { .. } => "the running Ethereal desktop app".into(),
        }
    }
}

type Ws = WebSocket<MaybeTlsStream<TcpStream>>;

fn set_read_timeout(ws: &Ws, t: Duration) {
    if let MaybeTlsStream::Plain(s) = ws.get_ref() {
        let _ = s.set_read_timeout(Some(t));
    }
}

/// Connect, say hello, and start the I/O thread.
fn open(url: &str, token: Option<&str>, pending: Arc<Pending>) -> Result<Connection, String> {
    let (mut ws, _) = tungstenite::connect(url).map_err(|e| e.to_string())?;
    set_read_timeout(&ws, CONNECT_TIMEOUT);
    let hello = ClientHello {
        protocol_version: PROTOCOL_VERSION,
        token: token.map(str::to_string),
        client: format!("ether-mcp {}", env!("CARGO_PKG_VERSION")),
    };
    ws.send(Message::text(
        serde_json::to_string(&hello).map_err(|e| e.to_string())?,
    ))
    .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    let answer = loop {
        if Instant::now() >= deadline {
            return Err("no hello from the server".into());
        }
        match ws.read().map_err(|e| e.to_string())? {
            Message::Text(t) => break t,
            Message::Close(_) => return Err("closed during the hello".into()),
            _ => {}
        }
    };
    match serde_json::from_str::<ServerHello>(&answer).map_err(|e| e.to_string())? {
        ServerHello::Welcome { .. } => {}
        ServerHello::Rejected { reason, message } => {
            return Err(format!("refused ({reason:?}): {message}"));
        }
    }
    set_read_timeout(&ws, POLL);
    let (tx, rx) = std::sync::mpsc::channel();
    let alive = Arc::new(AtomicBool::new(true));
    {
        let alive = alive.clone();
        std::thread::Builder::new()
            .name("ether-mcp-io".into())
            .spawn(move || {
                let why = pump(&mut ws, &rx, &pending);
                alive.store(false, Ordering::Release);
                tracing::warn!(%why, "connection to Ethereal ended");
                pending.fail_all(&format!("the connection to Ethereal was lost ({why})"));
                let _ = ws.close(None);
            })
            .map_err(|e| e.to_string())?;
    }
    Ok(Connection { tx, alive })
}

fn would_block(e: &tungstenite::Error) -> bool {
    matches!(e, tungstenite::Error::Io(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut))
}

/// Run the connection until it ends; returns why.
fn pump(ws: &mut Ws, rx: &Receiver<ClientMessage>, pending: &Pending) -> String {
    loop {
        loop {
            match rx.try_recv() {
                Ok(m) => {
                    let text = match serde_json::to_string(&m) {
                        Ok(t) => t,
                        Err(e) => {
                            pending.complete(m.id, Err(e.to_string()));
                            continue;
                        }
                    };
                    if let Err(e) = ws.send(Message::text(text)) {
                        return e.to_string();
                    }
                }
                Err(TryRecvError::Empty) => break,
                // The backend is gone.
                Err(TryRecvError::Disconnected) => return "closed".into(),
            }
        }
        let frame = match ws.read() {
            Ok(Message::Text(t)) => Frame::Text(t.to_string()),
            Ok(Message::Binary(b)) => Frame::Binary(b.to_vec()),
            Ok(Message::Close(f)) => {
                return f.map_or_else(|| "closed by the server".into(), |f| f.reason.to_string());
            }
            Ok(_) => continue,
            Err(e) if would_block(&e) => continue,
            Err(e) => return e.to_string(),
        };
        match decode_server(&frame) {
            Ok(ServerMessage::Reply(r)) => pending.complete(r.id, Ok(r.result)),
            Ok(_) => {}
            Err(e) => tracing::debug!(%e, "undecodable frame"),
        }
    }
}

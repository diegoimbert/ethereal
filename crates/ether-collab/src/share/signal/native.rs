//! Native signaling socket: tungstenite on one background thread per link, `ws://` or
//! `wss://` (rustls, `ring` provider, webpki roots). Same shape as the relay client
//! (`crate::native::WsClient`): the thread owns the socket, the link talks to it through
//! channels and never blocks.

use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use ether_protocol::share::{SignalClientMessage, SignalServerMessage};
use tungstenite::protocol::WebSocketConfig;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Connector, Message, WebSocket};

use super::{MAX_SIGNAL_FRAME_BYTES, decode, encode, socket_url};
use crate::LinkState;
use crate::share::SignalLink;

const POLL: Duration = Duration::from_millis(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

pub struct WsSignal {
    out: Sender<String>,
    inbound: Receiver<SignalServerMessage>,
    state: Arc<Mutex<LinkState>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl WsSignal {
    pub fn connect(url: &str) -> Self {
        let (out_tx, out_rx) = crossbeam_channel::unbounded();
        let (in_tx, in_rx) = crossbeam_channel::unbounded();
        let state = Arc::new(Mutex::new(LinkState::Connecting));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = match socket_url(url) {
            Err(reason) => {
                *state.lock().expect("state lock") = LinkState::Closed {
                    reason,
                    fatal: true,
                };
                None
            }
            Ok(url) => {
                let (state, stop) = (state.clone(), stop.clone());
                let spawned = std::thread::Builder::new()
                    .name("ether-signal".into())
                    .spawn(move || {
                        let result = run(&url, &out_rx, &in_tx, &state, &stop);
                        *state.lock().expect("state lock") = match result {
                            Ok(()) => LinkState::Closed {
                                reason: "closed".into(),
                                fatal: false,
                            },
                            Err((reason, fatal)) => LinkState::Closed { reason, fatal },
                        };
                    });
                spawned.ok()
            }
        };
        if thread.is_none() && matches!(*state.lock().expect("state lock"), LinkState::Connecting) {
            *state.lock().expect("state lock") = LinkState::Closed {
                reason: "could not start the signaling thread".into(),
                fatal: false,
            };
        }
        Self {
            out: out_tx,
            inbound: in_rx,
            state,
            stop,
            thread,
        }
    }
}

impl SignalLink for WsSignal {
    fn send(&mut self, message: &SignalClientMessage) {
        if !matches!(self.state(), LinkState::Closed { .. }) {
            let _ = self.out.send(encode(message));
        }
    }

    fn poll(&mut self, out: &mut Vec<SignalServerMessage>) {
        out.extend(self.inbound.try_iter());
    }

    fn state(&self) -> LinkState {
        self.state.lock().expect("state lock").clone()
    }

    fn close(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let mut s = self.state.lock().expect("state lock");
        if !matches!(*s, LinkState::Closed { .. }) {
            *s = LinkState::Closed {
                reason: "closed".into(),
                fatal: false,
            };
        }
    }
}

impl Drop for WsSignal {
    fn drop(&mut self) {
        self.close();
    }
}

type RunError = (String, bool);

/// The rustls client configuration (built once): `ring` provider, Mozilla roots.
fn tls_config() -> Result<Arc<rustls::ClientConfig>, String> {
    static CONFIG: OnceLock<Result<Arc<rustls::ClientConfig>, String>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            rustls::ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .map(|b| Arc::new(b.with_root_certificates(roots).with_no_client_auth()))
                .map_err(|e| format!("TLS setup: {e}"))
        })
        .clone()
}

/// `host:port` of a `ws(s)://` URL (default ports 80/443).
fn host_port(url: &str) -> Result<(String, bool), RunError> {
    let (tls, rest) = if let Some(r) = url.strip_prefix("wss://") {
        (true, r)
    } else if let Some(r) = url.strip_prefix("ws://") {
        (false, r)
    } else {
        return Err((format!("bad signaling URL {url}"), true));
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    if authority.is_empty() {
        return Err((format!("bad signaling URL {url}"), true));
    }
    let has_port = match authority.rfind(']') {
        Some(end) => authority[end..].contains(':'),
        None => authority.contains(':'),
    };
    let hp = if has_port {
        authority.to_string()
    } else {
        format!("{authority}:{}", if tls { 443 } else { 80 })
    };
    Ok((hp, tls))
}

fn tcp_of(s: &MaybeTlsStream<TcpStream>) -> &TcpStream {
    match s {
        MaybeTlsStream::Plain(s) => s,
        MaybeTlsStream::Rustls(s) => s.get_ref(),
        _ => unreachable!("only plain and rustls streams are created"),
    }
}

fn run(
    url: &str,
    out: &Receiver<String>,
    inbound: &Sender<SignalServerMessage>,
    state: &Mutex<LinkState>,
    stop: &AtomicBool,
) -> Result<(), RunError> {
    let (hp, tls) = host_port(url)?;
    let addr = hp
        .to_socket_addrs()
        .map_err(|e| (format!("resolve {hp}: {e}"), false))?
        .next()
        .ok_or_else(|| (format!("resolve {hp}: no address"), false))?;
    let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
        .map_err(|e| (format!("connect {hp}: {e}"), false))?;
    stream.set_nodelay(true).ok();
    stream
        .set_read_timeout(Some(HANDSHAKE_TIMEOUT))
        .map_err(|e| (e.to_string(), false))?;
    stream
        .set_write_timeout(Some(WRITE_TIMEOUT))
        .map_err(|e| (e.to_string(), false))?;
    let connector = if tls {
        Connector::Rustls(tls_config().map_err(|e| (e, true))?)
    } else {
        Connector::Plain
    };
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_SIGNAL_FRAME_BYTES))
        .max_frame_size(Some(MAX_SIGNAL_FRAME_BYTES));
    let (mut ws, _) = tungstenite::client_tls_with_config(url, stream, Some(config), Some(connector))
        .map_err(|e| {
            // An HTTP answer instead of an upgrade (404, 403 origin, 426) will not change on
            // a retry.
            let fatal = matches!(&e, tungstenite::HandshakeError::Failure(tungstenite::Error::Http(r)) if r.status().is_client_error());
            (format!("signaling handshake: {e}"), fatal)
        })?;
    tcp_of(ws.get_ref())
        .set_read_timeout(Some(POLL))
        .map_err(|e| (e.to_string(), false))?;
    *state.lock().expect("state lock") = LinkState::Open;
    pump(&mut ws, out, inbound, stop)
}

fn would_block(e: &tungstenite::Error) -> bool {
    matches!(e, tungstenite::Error::Io(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut))
}

fn pump(
    ws: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    out: &Receiver<String>,
    inbound: &Sender<SignalServerMessage>,
    stop: &AtomicBool,
) -> Result<(), RunError> {
    let err = |e: tungstenite::Error| (e.to_string(), false);
    loop {
        let mut wrote = false;
        while let Ok(t) = out.try_recv() {
            ws.write(Message::text(t)).map_err(err)?;
            wrote = true;
        }
        if wrote {
            match ws.flush() {
                Ok(()) => {}
                Err(e) if would_block(&e) => {}
                Err(e) => return Err(err(e)),
            }
        }
        if stop.load(Ordering::Relaxed) {
            let _ = ws.close(None);
            let _ = ws.flush();
            return Ok(());
        }
        match ws.read() {
            Ok(Message::Text(t)) => {
                if let Some(m) = decode(&t)
                    && inbound.send(m).is_err()
                {
                    return Ok(());
                }
            }
            Ok(Message::Close(frame)) => {
                let reason = frame
                    .map(|f| f.reason.to_string())
                    .filter(|r| !r.is_empty())
                    .unwrap_or_else(|| "closed by the signaling service".into());
                return Err((reason, false));
            }
            Ok(_) => {}
            Err(e) if would_block(&e) => {}
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                return Err(("signaling connection closed".into(), false));
            }
            Err(e) => return Err(err(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::time::Instant;

    #[test]
    fn host_ports() {
        assert_eq!(
            host_port("wss://etherealws.pages.dev/signal/v1").unwrap(),
            ("etherealws.pages.dev:443".into(), true)
        );
        assert_eq!(
            host_port("ws://127.0.0.1:8787/v1").unwrap(),
            ("127.0.0.1:8787".into(), false)
        );
        assert_eq!(
            host_port("ws://[::1]/x").unwrap(),
            ("[::1]:80".into(), false)
        );
        assert_eq!(
            host_port("ws://[::1]:9/x").unwrap(),
            ("[::1]:9".into(), false)
        );
        assert!(host_port("http://x").is_err());
    }

    #[test]
    fn tls_config_builds() {
        tls_config().unwrap();
    }

    fn wait(link: &WsSignal, f: impl Fn(&LinkState) -> bool) -> LinkState {
        let t = Instant::now();
        loop {
            let s = link.state();
            if f(&s) || t.elapsed() > Duration::from_secs(10) {
                return s;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// A one-socket echo service: answers `Ping` with `Pong` and then closes.
    #[test]
    fn exchanges_json_frames_over_ws() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            let mut ws = tungstenite::accept(s).unwrap();
            let t = loop {
                if let Message::Text(t) = ws.read().unwrap() {
                    break t;
                }
            };
            let m: SignalClientMessage = serde_json::from_str(&t).unwrap();
            assert_eq!(m, SignalClientMessage::Ping);
            ws.send(Message::text("not json")).unwrap();
            ws.send(Message::text(
                serde_json::to_string(&SignalServerMessage::Pong).unwrap(),
            ))
            .unwrap();
            ws.close(None).unwrap();
            let _ = ws.flush();
            // Drain until the client's close reply.
            while ws.read().is_ok() {}
        });
        let mut link = WsSignal::connect(&format!("http://127.0.0.1:{port}/v1/rooms/r/join"));
        // Queued before the upgrade: sent once open.
        link.send(&SignalClientMessage::Ping);
        assert_eq!(
            wait(&link, |s| *s != LinkState::Connecting),
            LinkState::Open
        );
        let mut got = Vec::new();
        let t = Instant::now();
        while got.is_empty() && t.elapsed() < Duration::from_secs(10) {
            link.poll(&mut got);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(got, vec![SignalServerMessage::Pong]);
        assert!(matches!(
            wait(&link, |s| matches!(s, LinkState::Closed { .. })),
            LinkState::Closed { fatal: false, .. }
        ));
        server.join().unwrap();
    }

    #[test]
    fn failures_close_the_link() {
        let link = WsSignal::connect("ftp://x.test/");
        assert!(matches!(
            link.state(),
            LinkState::Closed { fatal: true, .. }
        ));
        // Nothing listens there (port 9, discard).
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let link = WsSignal::connect(&format!("ws://127.0.0.1:{port}/v1"));
        assert!(matches!(
            wait(&link, |s| matches!(s, LinkState::Closed { .. })),
            LinkState::Closed { fatal: false, .. }
        ));
    }

    #[test]
    fn http_errors_are_fatal() {
        use std::io::{Read, Write};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf);
            s.write_all(b"HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\n\r\n")
                .unwrap();
        });
        let link = WsSignal::connect(&format!("ws://127.0.0.1:{port}/v1"));
        assert!(matches!(
            wait(&link, |s| matches!(s, LinkState::Closed { .. })),
            LinkState::Closed { fatal: true, .. }
        ));
        server.join().unwrap();
    }
}

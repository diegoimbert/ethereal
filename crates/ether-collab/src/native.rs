//! Native WebSocket client (tungstenite, one background thread per link).

use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, TryRecvError};
use ether_protocol::collab::CollabMessage;
use ether_protocol::remote::{ClientHello, HelloRejection, PROTOCOL_VERSION, ServerHello};
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{Message, WebSocket};

use crate::wire::{
    MAX_COLLAB_MESSAGE_BYTES, WireFrame, decode_binary, decode_text, encode_frame, session_url,
};
use crate::{CollabTransport, ConnectRequest, LinkState};

const POLL: Duration = Duration::from_millis(4);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

pub struct WsClient {
    out: Sender<WireFrame>,
    inbound: Receiver<CollabMessage>,
    state: Arc<Mutex<LinkState>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl WsClient {
    pub fn connect(request: &ConnectRequest) -> Self {
        let (out_tx, out_rx) = crossbeam_channel::unbounded();
        let (in_tx, in_rx) = crossbeam_channel::unbounded();
        let state = Arc::new(Mutex::new(LinkState::Connecting));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (state, stop, request) = (state.clone(), stop.clone(), request.clone());
            std::thread::Builder::new()
                .name("collab-link".into())
                .spawn(move || {
                    let result = run(&request, &out_rx, &in_tx, &state, &stop);
                    let mut s = state.lock().expect("state lock");
                    *s = match result {
                        Ok(()) => LinkState::Closed {
                            reason: "closed".into(),
                            fatal: false,
                        },
                        Err((reason, fatal)) => LinkState::Closed { reason, fatal },
                    };
                })
                .ok()
        };
        if thread.is_none() {
            *state.lock().expect("state lock") = LinkState::Closed {
                reason: "could not start the connection thread".into(),
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

impl CollabTransport for WsClient {
    fn send(&mut self, message: &CollabMessage) {
        let _ = self.out.send(encode_frame(message));
    }

    fn poll(&mut self, out: &mut Vec<CollabMessage>) {
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
    }
}

impl Drop for WsClient {
    fn drop(&mut self) {
        self.close();
    }
}

type RunError = (String, bool);

fn would_block(e: &tungstenite::Error) -> bool {
    matches!(e, tungstenite::Error::Io(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut))
}

fn host_port(url: &str) -> Result<String, RunError> {
    let rest = url
        .strip_prefix("ws://")
        .ok_or_else(|| ("only ws:// relays are supported natively".to_string(), true))?;
    let hp = rest.split('/').next().unwrap_or_default();
    if hp.is_empty() {
        return Err(("bad relay URL".into(), true));
    }
    Ok(if hp.contains(':') && !hp.ends_with(']') {
        hp.to_string()
    } else {
        format!("{hp}:80")
    })
}

fn run(
    request: &ConnectRequest,
    out: &Receiver<WireFrame>,
    inbound: &Sender<CollabMessage>,
    state: &Mutex<LinkState>,
    stop: &AtomicBool,
) -> Result<(), RunError> {
    let url = session_url(&request.server, &request.session);
    let addr = host_port(&url)?
        .to_socket_addrs()
        .map_err(|e| (format!("resolve: {e}"), false))?
        .next()
        .ok_or_else(|| ("resolve: no address".to_string(), false))?;
    let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
        .map_err(|e| (format!("connect: {e}"), false))?;
    stream.set_nodelay(true).ok();
    stream
        .set_read_timeout(Some(HELLO_TIMEOUT))
        .map_err(|e| (e.to_string(), false))?;
    stream
        .set_write_timeout(Some(WRITE_TIMEOUT))
        .map_err(|e| (e.to_string(), false))?;
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_COLLAB_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_COLLAB_MESSAGE_BYTES));
    let (mut ws, _) = tungstenite::client::client_with_config(url.as_str(), stream, Some(config))
        .map_err(|e| (format!("upgrade: {e}"), false))?;
    let hello = ClientHello {
        protocol_version: PROTOCOL_VERSION,
        token: request.token.clone(),
        client: request.client.clone(),
    };
    ws.send(Message::text(
        serde_json::to_string(&hello).expect("hello serializes"),
    ))
    .map_err(|e| (format!("hello: {e}"), false))?;
    loop {
        match ws.read() {
            Ok(Message::Text(t)) => {
                match serde_json::from_str::<ServerHello>(&t)
                    .map_err(|e| (format!("bad server hello: {e}"), true))?
                {
                    ServerHello::Welcome { .. } => break,
                    ServerHello::Rejected { reason, message } => {
                        let fatal = matches!(
                            reason,
                            HelloRejection::BadToken
                                | HelloRejection::UnsupportedVersion
                                | HelloRejection::Busy
                        );
                        return Err((message, fatal));
                    }
                }
            }
            Ok(_) => {}
            Err(e) => return Err((format!("hello: {e}"), false)),
        }
    }
    ws.get_ref()
        .set_read_timeout(Some(POLL))
        .map_err(|e| (e.to_string(), false))?;
    *state.lock().expect("state lock") = LinkState::Open;
    pump(&mut ws, out, inbound, stop)
}

fn pump(
    ws: &mut WebSocket<TcpStream>,
    out: &Receiver<WireFrame>,
    inbound: &Sender<CollabMessage>,
    stop: &AtomicBool,
) -> Result<(), RunError> {
    let err = |e: tungstenite::Error| (e.to_string(), false);
    loop {
        let mut wrote = false;
        loop {
            match out.try_recv() {
                Ok(WireFrame::Text(t)) => ws.write(Message::text(t)).map_err(err)?,
                Ok(WireFrame::Binary(b)) => ws.write(Message::binary(b)).map_err(err)?,
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
            wrote = true;
        }
        if wrote {
            ws.flush().map_err(err)?;
        }
        if stop.load(Ordering::Relaxed) {
            let _ = ws.close(None);
            let _ = ws.flush();
            return Ok(());
        }
        let decoded = match ws.read() {
            Ok(Message::Text(t)) => decode_text(&t),
            Ok(Message::Binary(b)) => decode_binary(&b),
            Ok(Message::Close(frame)) => {
                let (reason, fatal) = frame
                    .map(|f| {
                        let code: u16 = f.code.into();
                        (f.reason.to_string(), (4001..=4003).contains(&code))
                    })
                    .unwrap_or_else(|| ("closed by the relay".into(), false));
                return Err((reason, fatal));
            }
            Ok(_) => continue,
            Err(e) if would_block(&e) => continue,
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                return Err(("connection closed".into(), false));
            }
            Err(e) => return Err(err(e)),
        };
        match decoded {
            Ok(m) => {
                if inbound.send(m).is_err() {
                    return Ok(());
                }
            }
            Err(e) => tracing::debug!(%e, "malformed collab message from the relay"),
        }
    }
}

//! TURN over TCP or TLS (`turn:…?transport=tcp`, `turns:`), the fallback for networks that
//! block UDP (docs/SHARING.md §6.1). One small I/O thread per connection owns the stream:
//! it connects (bounded by [`CONNECT_TIMEOUT`]), writes what the share thread queued, reads
//! with a short timeout, cuts the byte stream into TURN messages ([`turn::split_stream`])
//! and hands them to the share thread as commands (waking it). The share thread keeps the
//! TURN state machine; this thread only moves bytes. Dropping the [`StreamConn`] ends the
//! thread (its outgoing channel disconnects).

use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, TryRecvError};
use rustls::pki_types::{CertificateDer, ServerName};

use super::turn::{self, Transport};
use super::{Cmd, Waker};

/// Longest wait for the TCP connection (the Allocate transaction gives up at 5 s anyway).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
/// Read timeout: how long queued writes may wait while the thread sits in `read`.
const READ_SLICE: Duration = Duration::from_millis(5);
/// A stream that buffers more than this without a complete message is not TURN.
const MAX_PENDING: usize = 128 * 1024;

/// The share thread's handle on one TCP/TLS TURN connection.
pub(crate) struct StreamConn {
    tx: Sender<Vec<u8>>,
}

impl StreamConn {
    /// Connect to `server` on a new thread. Messages arrive as [`Cmd::TurnStream`], the end
    /// as [`Cmd::TurnStreamClosed`], both tagged with `relay`.
    pub(crate) fn open(
        relay: u64,
        server: SocketAddr,
        transport: Transport,
        host: String,
        extra_roots: Vec<CertificateDer<'static>>,
        cmd: Sender<Cmd>,
        waker: Arc<Waker>,
    ) -> std::io::Result<Self> {
        let (tx, rx) = crossbeam_channel::unbounded::<Vec<u8>>();
        std::thread::Builder::new()
            .name("ether-share-turn".into())
            .spawn(move || {
                let reason = match connect(server, transport, &host, extra_roots) {
                    Ok(mut s) => pump(&mut *s, &rx, relay, &cmd, &waker),
                    Err(e) => e,
                };
                let _ = cmd.send(Cmd::TurnStreamClosed { relay, reason });
                waker.wake();
            })?;
        Ok(Self { tx })
    }

    /// Queue bytes for the server (never blocks).
    pub(crate) fn send(&self, bytes: Vec<u8>) {
        let _ = self.tx.send(bytes);
    }
}

trait Duplex: Read + Write + Send {}
impl<T: Read + Write + Send> Duplex for T {}

fn tls_config(
    extra_roots: Vec<CertificateDer<'static>>,
) -> Result<Arc<rustls::ClientConfig>, String> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    for c in extra_roots {
        roots.add(c).map_err(|e| format!("TLS root: {e}"))?;
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map(|b| Arc::new(b.with_root_certificates(roots).with_no_client_auth()))
        .map_err(|e| format!("TLS setup: {e}"))
}

fn connect(
    server: SocketAddr,
    transport: Transport,
    host: &str,
    extra_roots: Vec<CertificateDer<'static>>,
) -> Result<Box<dyn Duplex>, String> {
    let tcp = TcpStream::connect_timeout(&server, CONNECT_TIMEOUT)
        .map_err(|e| format!("could not connect to the TURN server: {e}"))?;
    let _ = tcp.set_nodelay(true);
    let slice = |tcp: &TcpStream| {
        tcp.set_read_timeout(Some(READ_SLICE))
            .map_err(|e| e.to_string())
    };
    if transport != Transport::Tls {
        slice(&tcp)?;
        return Ok(Box::new(tcp));
    }
    let name = ServerName::try_from(host.to_string())
        .map_err(|e| format!("bad TURN server name {host}: {e}"))?;
    let mut conn = rustls::ClientConnection::new(tls_config(extra_roots)?, name)
        .map_err(|e| format!("TLS: {e}"))?;
    // The handshake with a plain blocking timeout (a short read timeout would surface as
    // an error from the first write).
    let mut tcp = tcp;
    tcp.set_read_timeout(Some(CONNECT_TIMEOUT))
        .map_err(|e| e.to_string())?;
    while conn.is_handshaking() {
        conn.complete_io(&mut tcp)
            .map_err(|e| format!("TLS handshake with the TURN server: {e}"))?;
    }
    slice(&tcp)?;
    Ok(Box::new(rustls::StreamOwned::new(conn, tcp)))
}

/// Move bytes until the stream or the share thread goes away; the reason it ended.
fn pump(
    s: &mut dyn Duplex,
    rx: &Receiver<Vec<u8>>,
    relay: u64,
    cmd: &Sender<Cmd>,
    waker: &Waker,
) -> String {
    let mut pending = Vec::new();
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        loop {
            match rx.try_recv() {
                Ok(bytes) => {
                    if let Err(e) = s.write_all(&bytes).and_then(|()| s.flush()) {
                        return format!("TURN connection: {e}");
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return "closed".into(),
            }
        }
        match s.read(&mut buf) {
            Ok(0) => return "the TURN server closed the connection".into(),
            Ok(n) => {
                pending.extend_from_slice(&buf[..n]);
                match turn::split_stream(&mut pending) {
                    Some(msgs) if !msgs.is_empty() => {
                        if cmd.send(Cmd::TurnStream { relay, msgs }).is_err() {
                            return "closed".into();
                        }
                        waker.wake();
                    }
                    Some(_) if pending.len() > MAX_PENDING => {
                        return "the TURN server sent something else".into();
                    }
                    Some(_) => {}
                    None => return "the TURN server sent something else".into(),
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) => {}
            Err(e) => return format!("TURN connection: {e}"),
        }
    }
}

//! Network hardening of `ether-server`: half-open / non-reading clients are dropped, the
//! pre-auth phase has an absolute deadline, a message-size limit and a connection cap, and
//! an unauthenticated (loopback) server refuses non-loopback `Host`/`Origin` upgrades.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ether_native::audio::{AudioBackendKind, AudioSettings};
use ether_native::test_util::TempDir;
use ether_native::{DedicatedThread, HostConfig, HostOptions, NativeHost};
use ether_protocol::remote::{ClientHello, PROTOCOL_VERSION, ServerHello};
use ether_server::{Server, ServerConfig};
use tungstenite::client::IntoClientRequest;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

const TOKEN: &str = "t0ken";

fn start(tmp: &TempDir, config: ServerConfig) -> Server {
    let host = NativeHost::start(
        HostConfig {
            audio: Some(AudioSettings {
                backend: AudioBackendKind::Null,
                max_block_size: 256,
                ..Default::default()
            }),
            data_dir: tmp.path().to_path_buf(),
            instance: "test".into(),
            projects_root: tmp.path().join("projects"),
            library_roots: vec![],
        },
        HostOptions {
            main_thread: Arc::new(DedicatedThread::new()),
            ..HostOptions::default()
        },
    )
    .expect("host starts");
    Server::start(config, host).expect("server starts")
}

type Ws = WebSocket<MaybeTlsStream<TcpStream>>;

fn hello(ws: &mut Ws, token: Option<&str>) -> ServerHello {
    let h = ClientHello {
        protocol_version: PROTOCOL_VERSION,
        token: token.map(str::to_string),
        client: "hardening test".into(),
    };
    ws.send(Message::text(serde_json::to_string(&h).unwrap()))
        .unwrap();
    loop {
        match ws.read().expect("hello answer") {
            Message::Text(t) => return serde_json::from_str(&t).unwrap(),
            Message::Ping(_) | Message::Pong(_) => {}
            m => panic!("unexpected {m:?}"),
        }
    }
}

fn connect(addr: SocketAddr, token: Option<&str>) -> Ws {
    let (mut ws, _) = tungstenite::connect(format!("ws://{addr}/")).expect("connect");
    assert!(matches!(hello(&mut ws, token), ServerHello::Welcome { .. }));
    ws
}

fn wait_until(what: &str, timeout: Duration, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// `true` once the server closed the raw socket (EOF or reset), within `timeout`.
fn closed_by_server(s: &mut TcpStream, timeout: Duration) -> bool {
    s.set_read_timeout(Some(timeout)).unwrap();
    let mut buf = [0u8; 4096];
    loop {
        match s.read(&mut buf) {
            Ok(0) => return true,
            Ok(_) => {} // e.g. an HTTP error response: keep reading until EOF
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                return false;
            }
            Err(_) => return true,
        }
    }
}

#[test]
fn clients_that_stop_reading_are_dropped_and_live_ones_stay() {
    let tmp = TempDir::new("server-idle");
    let server = start(
        &tmp,
        ServerConfig {
            token: Some(TOKEN.into()),
            ping_interval: Duration::from_millis(200),
            idle_timeout: Duration::from_millis(800),
            write_timeout: Duration::from_millis(500),
            ..ServerConfig::default()
        },
    );
    let addr = server.local_addr();

    // A client that answers pings (tungstenite replies while reading) survives.
    let mut live = connect(addr, Some(TOKEN));
    if let MaybeTlsStream::Plain(s) = live.get_ref() {
        s.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
    }
    // A client that never reads (nor sends) again: half-open from the server's view.
    let dead = connect(addr, Some(TOKEN));
    // Keep the engine talking to everyone (playhead frames) while we wait.
    live.send(Message::text(
        r#"{"id":1,"gesture":null,"command":{"domain":"Project","command":{"type":"Create","id":"01890a5d-ac96-774b-bcce-b302099a8057","name":"Idle"}}}"#,
    ))
    .unwrap();
    live.send(Message::text(
        r#"{"id":2,"gesture":null,"command":{"domain":"Transport","command":{"type":"Play"}}}"#,
    ))
    .unwrap();
    assert_eq!(server.client_count(), 2);
    let start = Instant::now();
    let mut pings = 0;
    while start.elapsed() < Duration::from_secs(3) {
        match live.read() {
            Ok(Message::Ping(_)) => pings += 1,
            Ok(_) => {}
            Err(tungstenite::Error::Io(_)) => {}
            Err(e) => panic!("live client dropped: {e}"),
        }
        if server.client_count() == 1 && pings > 0 && start.elapsed() > Duration::from_secs(2) {
            break;
        }
    }
    assert!(pings > 0, "the server pings quiet clients");
    assert_eq!(server.client_count(), 1, "the silent client was dropped");
    drop(dead);
}

#[test]
fn pre_auth_deadline_size_limit_and_connection_cap() {
    let tmp = TempDir::new("server-preauth");
    let server = start(
        &tmp,
        ServerConfig {
            token: Some(TOKEN.into()),
            handshake_timeout: Duration::from_millis(700),
            max_pending_handshakes: 2,
            ..ServerConfig::default()
        },
    );
    let addr = server.local_addr();

    // Trickle: one byte of the upgrade request every 50 ms keeps each read short, but the
    // deadline is absolute.
    let request = format!(
        "GET / HTTP/1.1\r\nHost: {addr}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    let mut s = TcpStream::connect(addr).unwrap();
    let t0 = Instant::now();
    let mut cut = false;
    for b in request.bytes() {
        if s.write_all(&[b]).is_err() {
            cut = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
        if t0.elapsed() > Duration::from_secs(5) {
            break;
        }
    }
    assert!(
        cut || closed_by_server(&mut s, Duration::from_secs(2)),
        "trickling client not cut off"
    );
    assert!(t0.elapsed() < Duration::from_secs(5));

    // Oversized hello (> 64 KiB before auth): closed, never welcomed.
    let (mut ws, _) = tungstenite::connect(format!("ws://{addr}/")).unwrap();
    let big = format!(
        r#"{{"protocol_version":1,"token":"{TOKEN}","client":"{}"}}"#,
        "x".repeat(100 << 10)
    );
    let _ = ws.send(Message::text(big));
    loop {
        match ws.read() {
            Ok(Message::Text(t)) => panic!("unexpected answer {t}"),
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(_) => {}
        }
    }
    assert_eq!(server.client_count(), 0);

    // Connection cap: two idle TCP connections fill the pre-auth slots; a third is closed
    // right away. Once the deadline frees the slots, clients get in again.
    let _idle1 = TcpStream::connect(addr).unwrap();
    let _idle2 = TcpStream::connect(addr).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let mut third = TcpStream::connect(addr).unwrap();
    assert!(
        closed_by_server(&mut third, Duration::from_millis(300)),
        "over the pre-auth cap"
    );
    let mut ok = None;
    wait_until(
        "a slot frees up",
        Duration::from_secs(5),
        || match tungstenite::connect(format!("ws://{addr}/")) {
            Ok((mut ws, _)) => {
                let welcomed = matches!(hello(&mut ws, Some(TOKEN)), ServerHello::Welcome { .. });
                ok = Some(ws);
                welcomed
            }
            Err(_) => false,
        },
    );
    assert_eq!(server.client_count(), 1);
}

#[test]
fn no_auth_server_only_accepts_loopback_host_and_origin() {
    let tmp = TempDir::new("server-origin");
    let server = start(&tmp, ServerConfig::default());
    let addr = server.local_addr();
    let attempt = |headers: &[(&'static str, &'static str)]| {
        let mut req = format!("ws://{addr}/").into_client_request().unwrap();
        for (k, v) in headers {
            req.headers_mut()
                .insert(*k, tungstenite::http::HeaderValue::from_str(v).unwrap());
        }
        tungstenite::connect(req)
            .map(|(ws, _)| ws)
            .map_err(Box::new)
    };
    for bad in [
        vec![("Origin", "https://evil.example")],
        vec![("Origin", "null")],
        vec![("Origin", "http://127.0.0.1.evil.example")],
        vec![("Host", "evil.example")],
    ] {
        match attempt(&bad) {
            Err(e) if matches!(*e, tungstenite::Error::Http(_)) => {
                let tungstenite::Error::Http(r) = *e else {
                    unreachable!()
                };
                assert_eq!(r.status(), 403, "{bad:?}");
            }
            Err(e) => panic!("{bad:?}: {e}"),
            Ok(_) => panic!("{bad:?} was accepted"),
        }
    }
    for good in [
        vec![],
        vec![("Origin", "http://localhost:5173")],
        vec![("Origin", "http://127.0.0.1:20000")],
        vec![("Origin", "http://[::1]:8080"), ("Host", "localhost")],
    ] {
        let mut ws = attempt(&good).unwrap_or_else(|e| panic!("{good:?}: {e}"));
        assert!(matches!(hello(&mut ws, None), ServerHello::Welcome { .. }));
    }
}

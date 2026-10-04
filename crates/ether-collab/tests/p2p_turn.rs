//! Native data channels through TURN (docs/SHARING.md §6.1, node `native-turn`): the
//! relay's experimental TURN server (`relay::ice::turn`, docs/COLLAB.md §10, loopback peers
//! allowed) relays between two str0m endpoints that advertise no host candidate at all, so
//! the pairing only works through the relay:
//! - both sides "Hide my IP" (relay only), TURN over UDP;
//! - one side relay only, the other with plain host candidates;
//! - a UDP TURN server that never answers, then `turns:` (TLS) through a local TLS → UDP
//!   bridge in front of the same server; and plain `turn:…?transport=tcp`;
//! - wrong credentials fail the pairing with the TURN reason.
//!
//! `turn_server_stdio` (ignored) runs the same TURN server for the native ↔ browser e2e
//! (`apps/web/e2e/p2p-turn.spec.ts`).

#![cfg(all(feature = "turn", not(target_arch = "wasm32")))]

use std::io::{BufRead, ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, UdpSocket};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ether_collab::LinkState;
use ether_collab::relay::ice::credentials::unix_now;
use ether_collab::relay::ice::turn::{TurnBudgets, TurnThread};
use ether_collab::relay::ice::{IceConfig, TurnCredentials};
use ether_collab::share::native::turn::{channel_data, parse_channel_data, split_stream};
use ether_collab::share::native::{NativeConfig, NativePeers};
use ether_collab::share::{BoxPeerLink, PeerEndpoint, PeerOutput};
use ether_collab::wire::WireFrame;
use ether_protocol::collab::{IceServer, StreamSignal};
use ether_protocol::model::SiteId;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

const PEER: u32 = 9;

fn turn_server() -> (TurnThread, Arc<TurnCredentials>) {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let config = IceConfig {
        turn: true,
        turn_allow_private: true,
        ..IceConfig::default()
    };
    TurnThread::start_with(socket, &config, TurnBudgets::default()).unwrap()
}

fn ice(urls: Vec<String>, creds: &TurnCredentials, site: u64) -> Vec<IceServer> {
    let c = creds.mint(SiteId(site), unix_now());
    vec![IceServer {
        urls,
        username: Some(c.username),
        credential: Some(c.credential),
    }]
}

fn fixture(name: &str) -> String {
    format!(
        "{}/tests/fixtures/native-turn/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// No host candidate at all: only relay candidates can pair.
fn hidden() -> NativeConfig {
    NativeConfig {
        loopback: false,
        default_route: false,
        connect_timeout: Duration::from_secs(30),
        extra_tls_roots: vec![CertificateDer::from_pem_file(fixture("ca.pem")).unwrap()],
        ..NativeConfig::default()
    }
}

fn loopback() -> NativeConfig {
    NativeConfig {
        loopback: true,
        default_route: false,
        ..NativeConfig::default()
    }
}

struct Side {
    ep: NativePeers,
    link: Option<BoxPeerLink>,
    failed: Option<String>,
    candidates: Vec<String>,
}

impl Side {
    fn new(config: NativeConfig) -> Self {
        Self {
            ep: NativePeers::new(config),
            link: None,
            failed: None,
            candidates: Vec::new(),
        }
    }

    fn pump(&mut self) -> Vec<StreamSignal> {
        let mut out = Vec::new();
        self.ep.poll(&mut out);
        let mut signals = Vec::new();
        for o in out {
            match o {
                PeerOutput::Signal { signal, .. } => {
                    match &signal {
                        StreamSignal::Ice { candidate } if !candidate.candidate.is_empty() => {
                            self.candidates.push(candidate.candidate.clone());
                        }
                        StreamSignal::Offer { sdp } | StreamSignal::Answer { sdp } => {
                            self.candidates.extend(
                                sdp.lines()
                                    .filter_map(|l| l.strip_prefix("a=candidate:"))
                                    .map(|c| format!("candidate:{c}")),
                            );
                        }
                        _ => {}
                    }
                    signals.push(signal);
                }
                PeerOutput::Connected { link, .. } => self.link = Some(link),
                PeerOutput::Failed { reason, .. } => self.failed = Some(reason),
            }
        }
        signals
    }
}

fn exchange(a: &mut Side, b: &mut Side) {
    for s in a.pump() {
        b.ep.signal(PEER, s);
    }
    for s in b.pump() {
        a.ep.signal(PEER, s);
    }
}

/// Pair `joiner` and `host` (opened by the caller) and exchange frames both ways.
fn connect_and_echo(joiner: &mut Side, host: &mut Side) {
    let t = Instant::now();
    while joiner.link.is_none() || host.link.is_none() {
        assert!(
            t.elapsed() < Duration::from_secs(25),
            "no data channel (joiner: {:?}, host: {:?})",
            joiner.failed,
            host.failed
        );
        assert!(
            joiner.failed.is_none() && host.failed.is_none(),
            "failed (joiner: {:?}, host: {:?})",
            joiner.failed,
            host.failed
        );
        exchange(joiner, host);
        std::thread::sleep(Duration::from_millis(1));
    }
    // Fragmented frames each way, in order (~100 kB: the TURN server relays 64 kB/s per
    // allocation, COLLAB.md §10).
    let frames: Vec<WireFrame> = (0..4)
        .map(|i| {
            if i % 4 == 3 {
                WireFrame::Text(format!("text {i}"))
            } else {
                WireFrame::Binary(vec![i as u8; 16 * 1024 + i])
            }
        })
        .collect();
    let (mut got_h, mut got_j) = (Vec::new(), Vec::new());
    for f in &frames {
        joiner.link.as_mut().unwrap().send(f);
        host.link.as_mut().unwrap().send(f);
    }
    let t = Instant::now();
    while got_h.len() < frames.len() || got_j.len() < frames.len() {
        assert!(
            t.elapsed() < Duration::from_secs(30),
            "stalled at {}/{}",
            got_h.len(),
            got_j.len()
        );
        host.link.as_mut().unwrap().poll(&mut got_h);
        joiner.link.as_mut().unwrap().poll(&mut got_j);
        assert_eq!(host.link.as_ref().unwrap().state(), LinkState::Open);
        exchange(joiner, host);
        std::thread::sleep(Duration::from_micros(500));
    }
    assert!(
        got_h == frames && got_j == frames,
        "frames arrive whole and in order"
    );
}

fn only_relay_candidates(side: &Side) {
    assert!(!side.candidates.is_empty());
    for c in &side.candidates {
        assert!(
            c.contains(" typ relay"),
            "a non-relay candidate leaked: {c}"
        );
    }
}

#[test]
fn relay_only_on_both_sides_over_udp() {
    let (server, creds) = turn_server();
    let url = format!(
        "turn:127.0.0.1:{}?transport=udp",
        server.local_addr().port()
    );
    let mut joiner = Side::new(hidden());
    let mut host = Side::new(hidden());
    host.ep
        .open(PEER, false, &ice(vec![url.clone()], &creds, 1), true);
    joiner.ep.open(PEER, true, &ice(vec![url], &creds, 2), true);
    connect_and_echo(&mut joiner, &mut host);
    only_relay_candidates(&joiner);
    only_relay_candidates(&host);
}

/// The joiner hides its address; the host has a plain loopback host candidate. The pair
/// is relay (joiner) ↔ host (host): the host answers the relayed address directly.
#[test]
fn relay_only_joiner_and_a_plain_host() {
    let (server, creds) = turn_server();
    let url = format!("turn:127.0.0.1:{}", server.local_addr().port());
    let mut joiner = Side::new(hidden());
    let mut host = Side::new(loopback());
    host.ep.open(PEER, false, &[], false);
    joiner.ep.open(PEER, true, &ice(vec![url], &creds, 2), true);
    connect_and_echo(&mut joiner, &mut host);
    only_relay_candidates(&joiner);
    // Closing one side closes the other.
    joiner.link.as_mut().unwrap().close();
    let t = Instant::now();
    while host.link.as_ref().unwrap().state() == LinkState::Open {
        assert!(
            t.elapsed() < Duration::from_secs(20),
            "the host never noticed"
        );
        exchange(&mut joiner, &mut host);
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A plain pairing (no relay only, no host candidates) also gathers and uses the relay
/// candidate, plus the free server-reflexive one from the Allocate response.
#[test]
fn turn_candidates_without_relay_only() {
    let (server, creds) = turn_server();
    let url = format!("turn:127.0.0.1:{}", server.local_addr().port());
    let mut joiner = Side::new(hidden());
    let mut host = Side::new(hidden());
    host.ep
        .open(PEER, false, &ice(vec![url.clone()], &creds, 1), false);
    joiner
        .ep
        .open(PEER, true, &ice(vec![url], &creds, 2), false);
    connect_and_echo(&mut joiner, &mut host);
    assert!(joiner.candidates.iter().any(|c| c.contains(" typ relay")));
}

#[test]
fn wrong_credentials_fail_the_pairing() {
    let (server, creds) = turn_server();
    let url = format!("turn:127.0.0.1:{}", server.local_addr().port());
    let mut bad = ice(vec![url], &creds, 1);
    bad[0].credential = Some("not it".into());
    let mut joiner = Side::new(hidden());
    joiner.ep.open(PEER, true, &bad, true);
    let t = Instant::now();
    while joiner.failed.is_none() {
        assert!(t.elapsed() < Duration::from_secs(10));
        joiner.pump();
        std::thread::sleep(Duration::from_millis(5));
    }
    let reason = joiner.failed.unwrap();
    assert!(
        reason.starts_with("could not reach the TURN relay") && reason.contains("credentials"),
        "{reason}"
    );
}

// ─── TURN over TCP and TLS ────────────────────────────────────────────────────────────

/// A stream → UDP bridge in front of the UDP TURN server: what a TCP/TLS TURN listener
/// does for a client, minus the server itself (each connection gets its own UDP 5-tuple).
fn bridge(tls: bool, turn: SocketAddr) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let config = tls.then(|| {
        let cert = CertificateDer::from_pem_file(fixture("server.pem")).unwrap();
        let key = PrivateKeyDer::from_pem_file(fixture("server.key")).unwrap();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        Arc::new(
            rustls::ServerConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(vec![cert], key)
                .unwrap(),
        )
    });
    std::thread::spawn(move || {
        for tcp in listener.incoming() {
            let Ok(tcp) = tcp else { return };
            let config = config.clone();
            std::thread::spawn(move || {
                tcp.set_read_timeout(Some(Duration::from_millis(2)))
                    .unwrap();
                let s: Box<dyn ReadWrite> = match config {
                    Some(c) => {
                        let conn = rustls::ServerConnection::new(c).unwrap();
                        Box::new(rustls::StreamOwned::new(conn, tcp))
                    }
                    None => Box::new(tcp),
                };
                serve_bridge(s, turn);
            });
        }
    });
    addr
}

trait ReadWrite: Read + Write + Send {}
impl<T: Read + Write + Send> ReadWrite for T {}

fn serve_bridge(mut s: Box<dyn ReadWrite>, turn: SocketAddr) {
    let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
    udp.connect(turn).unwrap();
    udp.set_nonblocking(true).unwrap();
    let mut pending = Vec::new();
    let mut buf = vec![0u8; 65536];
    loop {
        match s.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => {
                pending.extend_from_slice(&buf[..n]);
                for m in split_stream(&mut pending).expect("TURN framing") {
                    udp.send(&m).unwrap();
                }
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(_) => return,
        }
        while let Ok(n) = udp.recv(&mut buf) {
            // ChannelData is padded on streams (RFC 8656 §12.5).
            let m = match parse_channel_data(&buf[..n]) {
                Some((ch, data)) => channel_data(ch, data, true),
                None => buf[..n].to_vec(),
            };
            if s.write_all(&m).and_then(|()| s.flush()).is_err() {
                return;
            }
        }
    }
}

#[test]
fn tls_after_a_silent_udp_server_and_plain_tcp() {
    let (server, creds) = turn_server();
    let tls = bridge(true, server.local_addr());
    let tcp = bridge(false, server.local_addr());
    let silent = UdpSocket::bind("127.0.0.1:0").unwrap();
    // The joiner's UDP server never answers: after the Allocate times out it falls back
    // to `turns:`. The host uses plain TCP.
    let joiner_urls = vec![
        format!("turns:127.0.0.1:{}?transport=tcp", tls.port()),
        format!(
            "turn:127.0.0.1:{}?transport=udp",
            silent.local_addr().unwrap().port()
        ),
    ];
    let host_urls = vec![format!("turn:127.0.0.1:{}?transport=tcp", tcp.port())];
    let mut joiner = Side::new(hidden());
    let mut host = Side::new(hidden());
    host.ep.open(PEER, false, &ice(host_urls, &creds, 1), true);
    joiner
        .ep
        .open(PEER, true, &ice(joiner_urls, &creds, 2), true);
    connect_and_echo(&mut joiner, &mut host);
    only_relay_candidates(&joiner);
    let mut probe = [0u8; 4];
    silent.set_nonblocking(true).unwrap();
    assert!(silent.recv_from(&mut probe).is_ok(), "UDP was tried first");
}

#[test]
fn an_untrusted_tls_server_is_refused() {
    let (server, creds) = turn_server();
    let tls = bridge(true, server.local_addr());
    let mut joiner = Side::new(NativeConfig {
        extra_tls_roots: Vec::new(),
        ..hidden()
    });
    let url = format!("turns:127.0.0.1:{}?transport=tcp", tls.port());
    joiner.ep.open(PEER, true, &ice(vec![url], &creds, 2), true);
    let t = Instant::now();
    while joiner.failed.is_none() {
        assert!(t.elapsed() < Duration::from_secs(10));
        joiner.pump();
        std::thread::sleep(Duration::from_millis(5));
    }
    let reason = joiner.failed.unwrap();
    assert!(reason.contains("TLS"), "{reason}");
}

/// For `apps/web/e2e/p2p-turn.spec.ts`: a loopback TURN server (UDP, loopback peers
/// allowed). Prints `TURN {"urls":[..],"username":..,"credential":..}` and runs until stdin
/// closes.
#[test]
#[ignore = "driven by apps/web/e2e/p2p-turn.spec.ts"]
fn turn_server_stdio() {
    let (server, creds) = turn_server();
    let ice = ice(
        vec![format!(
            "turn:127.0.0.1:{}?transport=udp",
            server.local_addr().port()
        )],
        &creds,
        1,
    );
    println!("TURN {}", serde_json::to_string(&ice[0]).unwrap());
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    while std::io::stdin()
        .lock()
        .read_line(&mut line)
        .is_ok_and(|n| n > 0)
    {
        line.clear();
    }
    drop(server);
}

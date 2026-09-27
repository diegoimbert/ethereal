//! The TURN server replaces itself (fresh nonce map) at its nonce budgets
//! (docs/COLLAB.md §10). Loopback only, tiny budgets; the budgets' logic itself is tested
//! with injected time in `relay::ice::turn::gate`.
#![cfg(all(feature = "turn", not(target_arch = "wasm32")))]

use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use ether_collab::relay::ice::IceConfig;
use ether_collab::relay::ice::credentials::unix_now;
use ether_collab::relay::ice::turn::{REALM, TurnBudgets, TurnStats, TurnThread};
use ether_protocol::model::SiteId;
use stun::attributes::{ATTR_NONCE, ATTR_REALM, ATTR_USERNAME};
use stun::error_code::{CODE_STALE_NONCE, CODE_UNAUTHORIZED, ErrorCodeAttribute};
use stun::message::{
    CLASS_ERROR_RESPONSE, CLASS_REQUEST, CLASS_SUCCESS_RESPONSE, Getter, METHOD_ALLOCATE,
    Message, MessageType, Setter,
};
use stun::textattrs::TextAttribute;

fn budgets(soft: u64, hard: u64) -> TurnBudgets {
    TurnBudgets {
        nonce_soft: soft,
        nonce_hard: hard,
        unverified_per_second: 100.0,
        unverified_per_source_burst: 100.0,
        ..TurnBudgets::default()
    }
}

fn start(budgets: TurnBudgets) -> (TurnThread, Arc<ether_collab::relay::ice::TurnCredentials>) {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    let config = IceConfig {
        turn: true,
        turn_allow_private: true,
        ..IceConfig::default()
    };
    TurnThread::start_with(socket, &config, budgets).unwrap()
}

fn client() -> UdpSocket {
    let c = UdpSocket::bind("127.0.0.1:0").unwrap();
    c.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    c
}

fn allocate(attrs: Vec<Box<dyn Setter>>) -> Vec<u8> {
    let mut setters: Vec<Box<dyn Setter>> = vec![
        Box::new(stun::agent::TransactionId::new()),
        Box::new(MessageType::new(METHOD_ALLOCATE, CLASS_REQUEST)),
        Box::new(turn::proto::reqtrans::RequestedTransport {
            protocol: turn::proto::PROTO_UDP,
        }),
    ];
    setters.extend(attrs);
    let mut m = Message::new();
    m.build(&setters).unwrap();
    m.raw
}

fn exchange(c: &UdpSocket, server: SocketAddr, request: &[u8]) -> Message {
    c.send_to(request, server).unwrap();
    let mut buf = [0u8; 1500];
    let (n, _) = c.recv_from(&mut buf).expect("an answer");
    let mut m = Message::new();
    m.unmarshal_binary(&buf[..n]).unwrap();
    m
}

fn error_code(m: &Message) -> u16 {
    assert_eq!(m.typ.class, CLASS_ERROR_RESPONSE, "{m}");
    let mut code = ErrorCodeAttribute::default();
    code.get_from(m).unwrap();
    code.code.0
}

/// An unauthenticated Allocate: the server answers 401 and stores a nonce, returned.
fn first_allocate(c: &UdpSocket, server: SocketAddr) -> String {
    let m = exchange(c, server, &allocate(vec![]));
    assert_eq!(error_code(&m), CODE_UNAUTHORIZED.0);
    TextAttribute::get_from_as(&m, ATTR_NONCE).unwrap().text
}

/// An Allocate authenticated with a fresh credential of `site`, presenting `nonce`.
fn authenticated_allocate(
    c: &UdpSocket,
    server: SocketAddr,
    creds: &ether_collab::relay::ice::TurnCredentials,
    site: u64,
    nonce: &str,
) -> Message {
    let cred = creds.mint(SiteId(site), unix_now());
    let request = allocate(vec![
        Box::new(TextAttribute::new(ATTR_USERNAME, cred.username.clone())),
        Box::new(TextAttribute::new(ATTR_REALM, REALM.into())),
        Box::new(TextAttribute::new(ATTR_NONCE, nonce.into())),
        Box::new(stun::integrity::MessageIntegrity::new_long_term_integrity(
            cred.username,
            REALM.into(),
            cred.credential,
        )),
    ]);
    exchange(c, server, &request)
}

fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn load(stats: &TurnStats) -> (u64, u64, u64, usize) {
    (
        stats.rotations.load(Ordering::Relaxed),
        stats.forced_rotations.load(Ordering::Relaxed),
        stats.nonces.load(Ordering::Relaxed),
        stats.live.load(Ordering::Relaxed),
    )
}

#[test]
fn an_idle_server_is_rotated_at_the_soft_budget_and_forgets_its_nonces() {
    let (turn, creds) = start(budgets(3, 1000));
    let server = turn.local_addr();
    let c = client();
    let first = first_allocate(&c, server);
    // A nonce the server holds is accepted.
    let ok = authenticated_allocate(&c, server, &creds, 1, &first);
    assert_eq!(ok.typ.class, CLASS_SUCCESS_RESPONSE, "{ok}");
    // Its allocation is live: the soft budget does not rotate.
    let c2 = client();
    first_allocate(&c2, server);
    first_allocate(&c2, server);
    wait_until("the nonces to be counted", || load(turn.stats()).2 == 3);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(load(turn.stats()), (0, 0, 3, 1));
    drop(c);
    drop(turn);

    // Same again with nobody live: rotated, and the old nonces are unknown (438).
    let (turn, creds) = start(budgets(3, 1000));
    let server = turn.local_addr();
    let c = client();
    let old = first_allocate(&c, server);
    first_allocate(&c, server);
    first_allocate(&c, server);
    wait_until("a rotation", || load(turn.stats()).0 == 1);
    let m = authenticated_allocate(&c, server, &creds, 1, &old);
    assert_eq!(error_code(&m), CODE_STALE_NONCE.0, "forgotten by the new server");
    // The 438 carried a new nonce, which the new server accepts.
    let renewed = TextAttribute::get_from_as(&m, ATTR_NONCE).unwrap().text;
    let ok = authenticated_allocate(&c, server, &creds, 1, &renewed);
    assert_eq!(ok.typ.class, CLASS_SUCCESS_RESPONSE, "{ok}");
    assert_eq!(load(turn.stats()).1, 0, "not forced");
}

#[test]
fn the_hard_budget_rotates_even_with_live_allocations() {
    let (turn, creds) = start(budgets(3, 6));
    let server = turn.local_addr();
    let c = client();
    let nonce = first_allocate(&c, server);
    let ok = authenticated_allocate(&c, server, &creds, 1, &nonce);
    assert_eq!(ok.typ.class, CLASS_SUCCESS_RESPONSE, "{ok}");
    // Requests reusing a held nonce cost nothing: however many, no rotation.
    let other = client();
    for _ in 0..10 {
        // (a second allocation, then 437: this 5-tuple already has one)
        let m = authenticated_allocate(&other, server, &creds, 2, &nonce);
        assert!(m.typ.class == CLASS_SUCCESS_RESPONSE || error_code(&m) != CODE_STALE_NONCE.0);
    }
    let flood = client();
    for _ in 0..4 {
        first_allocate(&flood, server);
    }
    wait_until("the soft budget", || load(turn.stats()).2 == 5);
    assert_eq!(load(turn.stats()).0, 0, "live: kept past the soft budget");
    first_allocate(&flood, server);
    wait_until("a forced rotation", || load(turn.stats()).1 == 1);
    wait_until("the allocation to be dropped", || load(turn.stats()).3 == 0);
    let m = authenticated_allocate(&c, server, &creds, 1, &nonce);
    assert_eq!(error_code(&m), CODE_STALE_NONCE.0, "forgotten by the new server");
}

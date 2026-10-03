//! Native data channels end to end (docs/SHARING.md §6.1, node `p2p-transport`): two
//! str0m endpoints pair over loopback through an in-memory signaling path and exchange
//! fragmented frames in order, with backpressure; an unreachable peer fails in time.

#![cfg(not(target_arch = "wasm32"))]

use std::time::{Duration, Instant};

use ether_collab::LinkState;
use ether_collab::share::native::{NativeConfig, NativePeers};
use ether_collab::share::{BoxPeerLink, PeerEndpoint, PeerOutput};
use ether_collab::wire::WireFrame;
use ether_protocol::collab::{IceCandidate, StreamSignal};

const PEER: u32 = 7;

fn loopback(connect_timeout: Duration) -> NativePeers {
    NativePeers::new(NativeConfig {
        loopback: true,
        default_route: false,
        connect_timeout,
        ..NativeConfig::default()
    })
}

struct Side {
    ep: NativePeers,
    link: Option<BoxPeerLink>,
    fps: Option<(String, String)>,
    failed: Option<String>,
}

impl Side {
    fn new(ep: NativePeers) -> Self {
        Self {
            ep,
            link: None,
            fps: None,
            failed: None,
        }
    }

    /// Poll the endpoint; signals for the other side are returned.
    fn pump(&mut self) -> Vec<StreamSignal> {
        let mut out = Vec::new();
        self.ep.poll(&mut out);
        let mut signals = Vec::new();
        for o in out {
            match o {
                PeerOutput::Signal { peer, signal } => {
                    assert_eq!(peer, PEER);
                    signals.push(signal);
                }
                PeerOutput::Connected {
                    peer,
                    link,
                    local_fingerprint,
                    remote_fingerprint,
                } => {
                    assert_eq!(peer, PEER);
                    assert!(self.link.is_none(), "connected twice");
                    self.link = Some(link);
                    self.fps = Some((local_fingerprint, remote_fingerprint));
                }
                PeerOutput::Failed { peer, reason } => {
                    assert_eq!(peer, PEER);
                    self.failed = Some(reason);
                }
            }
        }
        signals
    }
}

/// The in-memory signaling service: everything one side emits reaches the other.
fn exchange(joiner: &mut Side, host: &mut Side) {
    for s in joiner.pump() {
        host.ep.signal(PEER, s);
    }
    for s in host.pump() {
        joiner.ep.signal(PEER, s);
    }
}

fn pair() -> (Side, Side) {
    let mut joiner = Side::new(loopback(Duration::from_secs(30)));
    let mut host = Side::new(loopback(Duration::from_secs(30)));
    host.ep.open(PEER, false, &[], false);
    joiner.ep.open(PEER, true, &[], false);
    let t = Instant::now();
    while joiner.link.is_none() || host.link.is_none() {
        assert!(
            t.elapsed() < Duration::from_secs(20),
            "no data channel (joiner: {:?}, host: {:?})",
            joiner.failed,
            host.failed
        );
        assert!(joiner.failed.is_none() && host.failed.is_none());
        exchange(&mut joiner, &mut host);
        std::thread::sleep(Duration::from_millis(1));
    }
    (joiner, host)
}

fn frame(i: usize, len: usize) -> WireFrame {
    if i % 3 == 2 {
        // Text frames too (JSON in real life), with multi-byte characters.
        WireFrame::Text(format!("{i}:{}", "é".repeat(len / 2)))
    } else {
        let mut b = vec![(i % 251) as u8; len];
        b[..8].copy_from_slice(&(i as u64).to_le_bytes());
        WireFrame::Binary(b)
    }
}

#[test]
fn twenty_mib_of_fragmented_frames_in_order_with_backpressure() {
    let (mut joiner, mut host) = pair();

    // Fingerprints: each side's local is the other's remote, in SDP form.
    let (jl, jr) = joiner.fps.clone().unwrap();
    let (hl, hr) = host.fps.clone().unwrap();
    assert_eq!(jl, hr);
    assert_eq!(jr, hl);
    assert_ne!(jl, hl);
    for fp in [&jl, &hl] {
        let hex = fp.strip_prefix("sha-256 ").expect("sha-256 fingerprint");
        assert_eq!(hex.split(':').count(), 32, "{fp}");
        assert!(
            hex.chars()
                .all(|c| c == ':' || c.is_ascii_hexdigit() && !c.is_ascii_lowercase())
        );
    }

    // 20 MiB joiner → host in frames from 0 bytes to 3 MiB (one past 16 KiB boundaries),
    // and some frames the other way at the same time.
    let sizes: Vec<usize> = (0..40)
        .map(|i| match i % 5 {
            0 => 3 * 1024 * 1024 + 1,
            1 => 16 * 1024 - 1,
            2 => 64 * 1024,
            3 => 1,
            _ => 300_000,
        })
        .collect();
    let mut total = 0;
    let mut n = 0;
    while total < 20 * 1024 * 1024 {
        total += sizes[n % sizes.len()];
        n += 1;
    }
    let expected: Vec<WireFrame> = (0..n)
        .map(|i| frame(i, sizes[i % sizes.len()].max(8)))
        .collect();
    let back: Vec<WireFrame> = (0..50)
        .map(|i| WireFrame::Text(format!("ack {i}")))
        .collect();

    const BUDGET: usize = 2 * 1024 * 1024;
    let mut sent = 0;
    let mut back_sent = 0;
    let mut got = Vec::new();
    let mut got_back = Vec::new();
    let mut max_buffered = 0;
    let t = Instant::now();
    while got.len() < n || got_back.len() < back.len() {
        assert!(
            t.elapsed() < Duration::from_secs(120),
            "stalled: {}/{n} frames, {} back",
            got.len(),
            got_back.len()
        );
        let jl = joiner.link.as_mut().unwrap();
        // Backpressure: only send while the link's buffer is under budget.
        while sent < n && jl.buffered() < BUDGET {
            jl.send(&expected[sent]);
            sent += 1;
            max_buffered = max_buffered.max(jl.buffered());
        }
        let hl = host.link.as_mut().unwrap();
        if back_sent < back.len() {
            hl.send(&back[back_sent]);
            back_sent += 1;
        }
        hl.poll(&mut got);
        jl.poll(&mut got_back);
        assert_eq!(jl.state(), LinkState::Open);
        assert_eq!(hl.state(), LinkState::Open);
        exchange(&mut joiner, &mut host);
        std::thread::sleep(Duration::from_micros(200));
    }
    assert!(got == expected, "frames arrive whole and in order");
    assert_eq!(got_back, back);
    assert!(max_buffered > 0, "buffered() reports queued bytes");
    assert!(
        max_buffered >= BUDGET,
        "the budget was reached: {max_buffered}"
    );
    // Everything drains.
    let t = Instant::now();
    while joiner.link.as_ref().unwrap().buffered() > 0 {
        assert!(
            t.elapsed() < Duration::from_secs(10),
            "buffer never drained"
        );
        exchange(&mut joiner, &mut host);
        std::thread::sleep(Duration::from_millis(1));
    }

    // Closing one side closes the other's link.
    joiner.link.as_mut().unwrap().close();
    let t = Instant::now();
    loop {
        exchange(&mut joiner, &mut host);
        if matches!(
            host.link.as_ref().unwrap().state(),
            LinkState::Closed { .. }
        ) {
            break;
        }
        assert!(
            t.elapsed() < Duration::from_secs(15),
            "host link stayed open"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn closing_the_endpoint_pairing_closes_the_link() {
    let (mut joiner, mut host) = pair();
    host.ep.close(PEER);
    let t = Instant::now();
    loop {
        exchange(&mut joiner, &mut host);
        let h = host.link.as_ref().unwrap().state();
        let j = joiner.link.as_ref().unwrap().state();
        if matches!(h, LinkState::Closed { .. }) && matches!(j, LinkState::Closed { .. }) {
            break;
        }
        assert!(
            t.elapsed() < Duration::from_secs(20),
            "links stayed open: {h:?} {j:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    // A closed link drops sends.
    host.link
        .as_mut()
        .unwrap()
        .send(&WireFrame::Text("late".into()));
    assert_eq!(host.link.as_ref().unwrap().buffered(), 0);
}

#[test]
fn unreachable_peer_fails_in_time() {
    // The joiner offers; the "host" answers nothing and its only candidate is a dead port.
    let timeout = Duration::from_secs(3);
    let mut joiner = Side::new(loopback(timeout));
    joiner.ep.open(PEER, true, &[], false);
    let t = Instant::now();
    let mut answered = false;
    while joiner.failed.is_none() {
        for s in joiner.pump() {
            if let (StreamSignal::Offer { .. }, false) = (&s, answered) {
                answered = true;
                // A candidate nobody listens on.
                joiner.ep.signal(
                    PEER,
                    StreamSignal::Ice {
                        candidate: IceCandidate {
                            candidate: "candidate:1 1 udp 2130706431 127.0.0.1 9 typ host".into(),
                            sdp_mid: Some("0".into()),
                            sdp_m_line_index: Some(0),
                            username_fragment: None,
                        },
                    },
                );
            }
        }
        assert!(
            t.elapsed() < timeout + Duration::from_secs(5),
            "never failed"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(t.elapsed() >= timeout - Duration::from_millis(100));
    assert!(joiner.link.is_none());
    // The default is the documented 30 s (ROADMAP acceptance: Failed within 30 s).
    assert_eq!(
        NativeConfig::default().connect_timeout,
        Duration::from_secs(30)
    );
}

#[test]
fn bye_fails_a_pending_pairing_and_unknown_peers_are_ignored() {
    let mut host = Side::new(loopback(Duration::from_secs(30)));
    host.ep.open(PEER, false, &[], false);
    host.ep.signal(PEER + 1, StreamSignal::Bye { reason: None });
    host.ep.signal(
        PEER,
        StreamSignal::Bye {
            reason: Some("refused".into()),
        },
    );
    let t = Instant::now();
    while host.failed.is_none() {
        host.pump();
        assert!(t.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(host.failed.as_deref(), Some("refused"));
}

//! `stream-host` candidate gathering (docs/COLLAB.md §10): host candidates in the offer, one
//! STUN Binding request per `stun:` URL on the sender's socket → a trickled srflx candidate,
//! then the end-of-candidates marker; unanswered STUN gives up silently.

#[path = "stream_support.rs"]
mod stream_util;

use std::net::UdpSocket;
use std::time::{Duration, Instant};

use ether_controller::streaming::StreamOutput;
use ether_core::protocol::collab::{IceServer, StreamSignal};
use ether_native::stream::stun;
use stream_util::*;

fn ice(urls: &[String]) -> Vec<IceServer> {
    vec![IceServer {
        urls: urls.to_vec(),
        username: None,
        credential: None,
    }]
}

/// Signals until the end-of-candidates marker (or `within` elapses).
fn gather(
    sender: &mut ether_native::stream::StreamSender,
    within: Duration,
) -> (Vec<StreamSignal>, Duration) {
    let start = Instant::now();
    let mut out = Outputs::default();
    loop {
        out.collect(sender);
        let signals = out.signals();
        let done = signals.iter().any(
            |s| matches!(s, StreamSignal::Ice { candidate } if candidate.candidate.is_empty()),
        );
        if done || start.elapsed() > within {
            assert!(
                !out.all
                    .iter()
                    .any(|o| matches!(o, StreamOutput::State { .. })),
                "{:?}",
                out.all
            );
            return (signals, start.elapsed());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn srflx_from_one_binding_request_then_end_of_candidates() {
    // A loopback STUN server that maps everyone to 203.0.113.7:4242.
    let server = UdpSocket::bind("127.0.0.1:0").unwrap();
    let url = format!("stun:{}", server.local_addr().unwrap());
    let responder = std::thread::spawn(move || {
        let mut buf = [0u8; 1500];
        server
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let (n, from) = server.recv_from(&mut buf).unwrap();
        assert_eq!(n, 20, "a minimal Binding request");
        let tid = stun::binding_request_tid(&buf[..n]).unwrap();
        let resp = stun::binding_response(&tid, "203.0.113.7:4242".parse().unwrap());
        server.send_to(&resp, from).unwrap();
    });
    let mut sender = loopback_sender();
    let urls = [url, "turn:127.0.0.1:1?transport=udp".to_string()];
    sender.open(LISTENER, STREAM, &ice(&urls)).unwrap();
    let (signals, _) = gather(&mut sender, Duration::from_secs(5));
    responder.join().unwrap();

    let StreamSignal::Offer { sdp } = &signals[0] else {
        panic!("offer first: {signals:?}");
    };
    assert!(
        sdp.contains(&format!("127.0.0.1 {} typ host", sender.port())),
        "{sdp}"
    );
    let ice: Vec<_> = signals[1..]
        .iter()
        .map(|s| match s {
            StreamSignal::Ice { candidate } => candidate.clone(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(ice.len(), 2, "{ice:?}");
    assert!(
        ice[0].candidate.starts_with("candidate:")
            && ice[0].candidate.contains("203.0.113.7 4242 typ srflx"),
        "{:?}",
        ice[0]
    );
    assert_eq!(ice[0].sdp_m_line_index, Some(0));
    assert!(ice[0].sdp_mid.is_some());
    assert!(ice[1].candidate.is_empty(), "end of candidates last");
}

#[test]
fn unanswered_stun_gives_up_silently() {
    let silent = UdpSocket::bind("127.0.0.1:0").unwrap();
    let mut sender = loopback_sender();
    let url = format!("stun:{}", silent.local_addr().unwrap());
    sender.open(LISTENER, STREAM, &ice(&[url])).unwrap();
    let (signals, took) = gather(&mut sender, Duration::from_secs(5));
    assert!(
        (Duration::from_millis(1500)..Duration::from_millis(4000)).contains(&took),
        "{took:?}"
    );
    // Offer, then only the end-of-candidates marker; the request was retransmitted once.
    assert_eq!(signals.len(), 2, "{signals:?}");
    let mut buf = [0u8; 64];
    silent.set_nonblocking(true).unwrap();
    let mut requests = 0;
    while silent.recv_from(&mut buf).is_ok() {
        requests += 1;
    }
    assert_eq!(requests, 2);
}

#[test]
fn without_stun_the_candidates_end_at_once() {
    let mut sender = loopback_sender();
    sender.open(LISTENER, STREAM, &[]).unwrap();
    let (signals, took) = gather(&mut sender, Duration::from_secs(2));
    assert!(took < Duration::from_millis(500));
    assert_eq!(signals.len(), 2, "{signals:?}");
    // Opening the same stream again is a no-op.
    sender.open(LISTENER, STREAM, &[]).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    let mut out = Vec::new();
    sender.poll(&mut out);
    assert!(out.is_empty(), "{out:?}");
}

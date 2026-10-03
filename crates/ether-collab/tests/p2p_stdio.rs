//! A native share peer driven over stdin/stdout, for the native ↔ browser interop e2e
//! (`apps/web/e2e/p2p.spec.ts`). Ignored in normal runs; the e2e starts it with
//! `cargo test -p ether-collab --test p2p_stdio -- --ignored --nocapture`.
//!
//! - `P2P_STDIO_OFFER=1`: this side is the joiner (it offers), else the host.
//! - stdout: one `P2P <json>` line per event: `{"type":"Signal","signal":...}`,
//!   `{"type":"Connected","local":"sha-256 ..","remote":".."}`, `{"type":"Failed","reason":..}`,
//!   `{"type":"Frame","text":..}` or `{"type":"Frame","len":..,"sum":..}` (binary).
//! - stdin: one JSON line per command: `{"type":"Signal","signal":...}` (a remote signal),
//!   `{"type":"Quit"}`. Every received frame is echoed back on the data channel.

#![cfg(not(target_arch = "wasm32"))]

use std::io::{BufRead, Write};
use std::time::{Duration, Instant};

use ether_collab::LinkState;
use ether_collab::share::native::{NativeConfig, NativePeers};
use ether_collab::share::{BoxPeerLink, PeerEndpoint, PeerOutput};
use ether_collab::wire::WireFrame;
use ether_protocol::collab::StreamSignal;
use serde::{Deserialize, Serialize};

const PEER: u32 = 1;

#[derive(Deserialize)]
#[serde(tag = "type")]
enum In {
    Signal { signal: StreamSignal },
    Quit,
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum Out {
    Signal {
        signal: StreamSignal,
    },
    Connected {
        local: String,
        remote: String,
    },
    Failed {
        reason: String,
    },
    Frame {
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        len: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        sum: Option<u32>,
    },
    Closed {
        reason: String,
    },
}

fn emit(o: &Out) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "P2P {}", serde_json::to_string(o).expect("serializes"));
    let _ = out.flush();
}

#[test]
#[ignore = "driven by apps/web/e2e/p2p.spec.ts"]
fn stdio_peer() {
    let offer = std::env::var("P2P_STDIO_OFFER").is_ok_and(|v| v == "1");
    let (tx, rx) = crossbeam_channel::unbounded::<In>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if let Ok(m) = serde_json::from_str::<In>(&line)
                && tx.send(m).is_err()
            {
                break;
            }
        }
        let _ = tx.send(In::Quit);
    });
    let mut ep = NativePeers::new(NativeConfig::default());
    ep.open(PEER, offer, &[], false);
    let mut link: Option<BoxPeerLink> = None;
    let started = Instant::now();
    let mut outputs = Vec::new();
    let mut frames = Vec::new();
    loop {
        for m in rx.try_iter() {
            match m {
                In::Signal { signal } => ep.signal(PEER, signal),
                In::Quit => return,
            }
        }
        ep.poll(&mut outputs);
        for o in outputs.drain(..) {
            match o {
                PeerOutput::Signal { signal, .. } => emit(&Out::Signal { signal }),
                PeerOutput::Connected {
                    link: l,
                    local_fingerprint,
                    remote_fingerprint,
                    ..
                } => {
                    link = Some(l);
                    emit(&Out::Connected {
                        local: local_fingerprint,
                        remote: remote_fingerprint,
                    });
                }
                PeerOutput::Failed { reason, .. } => {
                    emit(&Out::Failed { reason });
                    return;
                }
            }
        }
        if let Some(l) = link.as_mut() {
            l.poll(&mut frames);
            for f in frames.drain(..) {
                l.send(&f);
                emit(&match f {
                    WireFrame::Text(text) => Out::Frame {
                        text: Some(text),
                        len: None,
                        sum: None,
                    },
                    WireFrame::Binary(b) => Out::Frame {
                        text: None,
                        len: Some(b.len()),
                        sum: Some(b.iter().fold(0u32, |s, &x| s.wrapping_add(x as u32))),
                    },
                });
            }
            if let LinkState::Closed { reason, .. } = l.state() {
                emit(&Out::Closed { reason });
                return;
            }
        }
        assert!(
            started.elapsed() < Duration::from_secs(120),
            "stdio peer timed out"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

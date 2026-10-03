//! The share port (docs/SHARING.md §6.2, node `p2p-transport`): the `MessagePort` between
//! the UI thread (which owns the `RTCPeerConnection`s) and this controller Worker, where
//! `ether_collab::share::web::WebPeers` drives them.
//!
//! - [`install_share_port`]: called once by `controller.worker.ts` with the port the main
//!   thread transferred in `init`.
//! - [`ShareProbe`]: a bare [`PeerEndpoint`] for diagnostics and the browser e2e
//!   (`apps/web/e2e/p2p.spec.ts`): open pairings, pass signals, send and receive frames,
//!   without the controller's share session.

use std::collections::HashMap;

use ether_collab::LinkState;
use ether_collab::share::web::WebPeers;
use ether_collab::share::{BoxPeerLink, PeerEndpoint, PeerOutput};
use ether_collab::wire::WireFrame;
use ether_core::protocol::collab::{IceServer, StreamSignal};
use serde::Serialize;
use wasm_bindgen::prelude::*;
use web_sys::MessagePort;

/// Hand the share port to the sharing code (see the module docs).
#[wasm_bindgen]
pub fn install_share_port(port: MessagePort) {
    ether_collab::share::web::install_port(port);
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum ProbeOutput {
    Signal {
        peer: u32,
        signal: StreamSignal,
    },
    Connected {
        peer: u32,
        local: String,
        remote: String,
    },
    Failed {
        peer: u32,
        reason: String,
    },
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum ProbeFrame {
    Text {
        text: String,
    },
    /// Binary frames are summarised: length and a checksum (sum of bytes mod 2^32).
    Binary {
        len: usize,
        sum: u32,
    },
}

/// A [`WebPeers`] endpoint driven from JavaScript (JSON in, JSON out).
#[wasm_bindgen]
pub struct ShareProbe {
    peers: WebPeers,
    links: HashMap<u32, BoxPeerLink>,
}

#[wasm_bindgen]
impl ShareProbe {
    #[wasm_bindgen(constructor)]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            peers: WebPeers::new(),
            links: HashMap::new(),
        }
    }

    /// `ice_json`: `IceServer[]`.
    pub fn open(
        &mut self,
        peer: u32,
        offer: bool,
        ice_json: &str,
        relay_only: bool,
    ) -> Result<(), JsError> {
        let ice: Vec<IceServer> = serde_json::from_str(ice_json)?;
        self.links.remove(&peer);
        self.peers.open(peer, offer, &ice, relay_only);
        Ok(())
    }

    /// `signal_json`: a `StreamSignal`.
    pub fn signal(&mut self, peer: u32, signal_json: &str) -> Result<(), JsError> {
        self.peers.signal(peer, serde_json::from_str(signal_json)?);
        Ok(())
    }

    /// JSON `({type:"Signal",peer,signal} | {type:"Connected",peer,local,remote} |
    /// {type:"Failed",peer,reason})[]`.
    pub fn poll(&mut self) -> String {
        let mut out = Vec::new();
        self.peers.poll(&mut out);
        let mut json = Vec::with_capacity(out.len());
        for o in out {
            json.push(match o {
                PeerOutput::Signal { peer, signal } => ProbeOutput::Signal { peer, signal },
                PeerOutput::Connected {
                    peer,
                    link,
                    local_fingerprint,
                    remote_fingerprint,
                } => {
                    self.links.insert(peer, link);
                    ProbeOutput::Connected {
                        peer,
                        local: local_fingerprint,
                        remote: remote_fingerprint,
                    }
                }
                PeerOutput::Failed { peer, reason } => ProbeOutput::Failed { peer, reason },
            });
        }
        serde_json::to_string(&json).expect("probe output serializes")
    }

    pub fn send_text(&mut self, peer: u32, text: String) {
        if let Some(l) = self.links.get_mut(&peer) {
            l.send(&WireFrame::Text(text));
        }
    }

    /// A binary frame of `len` bytes: byte `i` = `(i * 31 + seed) mod 256`.
    pub fn send_pattern(&mut self, peer: u32, len: usize, seed: u8) {
        if let Some(l) = self.links.get_mut(&peer) {
            let bytes = (0..len)
                .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
                .collect();
            l.send(&WireFrame::Binary(bytes));
        }
    }

    /// Received frames since the last call, JSON `({type:"Text",text} |
    /// {type:"Binary",len,sum})[]`. With `echo`, each one is sent back first.
    pub fn recv(&mut self, peer: u32, echo: bool) -> String {
        let mut frames = Vec::new();
        if let Some(l) = self.links.get_mut(&peer) {
            l.poll(&mut frames);
            if echo {
                for f in &frames {
                    l.send(f);
                }
            }
        }
        let json: Vec<ProbeFrame> = frames
            .into_iter()
            .map(|f| match f {
                WireFrame::Text(text) => ProbeFrame::Text { text },
                WireFrame::Binary(b) => ProbeFrame::Binary {
                    len: b.len(),
                    sum: b.iter().fold(0u32, |s, &x| s.wrapping_add(x as u32)),
                },
            })
            .collect();
        serde_json::to_string(&json).expect("probe frames serialize")
    }

    pub fn buffered(&self, peer: u32) -> f64 {
        self.links.get(&peer).map_or(0.0, |l| l.buffered() as f64)
    }

    /// `"none"`, `"connecting"`, `"open"` or `"closed: <reason>"`.
    pub fn state(&self, peer: u32) -> String {
        match self.links.get(&peer).map(|l| l.state()) {
            None => "none".into(),
            Some(LinkState::Connecting) => "connecting".into(),
            Some(LinkState::Open) => "open".into(),
            Some(LinkState::Closed { reason, .. }) => format!("closed: {reason}"),
        }
    }

    pub fn close(&mut self, peer: u32) {
        if let Some(mut l) = self.links.remove(&peer) {
            l.close();
        }
        self.peers.close(peer);
    }
}

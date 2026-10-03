//! Sharing over WebRTC data channels with the sharer's app as the hub (base-115 contract;
//! design: docs/SHARING.md).
//!
//! - [`invite`]: the invite link format (parse/format; frozen, implemented).
//! - [`dc`]: how wire frames ([`crate::wire::WireFrame`]) travel on a data channel:
//!   fragments of at most [`dc::DC_FRAGMENT_BYTES`] (frozen, implemented).
//! - [`file`]: the host-local `share.json` next to a shared project (frozen shape; written
//!   by node `share-engine`, listed by `recents-shared`).
//! - The seams between the nodes (frozen traits): [`SignalLink`] (a socket to the signaling
//!   service), [`PeerEndpoint`] (creates peer connections, yields [`PeerLink`]s), bundled in
//!   [`ShareServices`]. Real implementations: node `p2p-transport` (native: tungstenite +
//!   rustls and str0m; web: the Worker's `WebSocket` and the UI's `RTCPeerConnection`
//!   behind a `MessagePort`). Until then [`default_services`] fails cleanly; tests inject
//!   in-memory fakes.
//!
//! The hub (node `share-engine`) is the existing [`crate::relay::Relay`] state machine run in
//! the host's process (one session), with the host's own site on a loopback link and every
//! joiner on a [`PeerLink`]: the op flow, ordering, catch-up, compaction and limits are
//! unchanged (docs/SHARING.md §2).

pub mod dc;
pub mod fake;
pub mod file;
pub mod handshake;
pub mod hub;
pub mod invite;
pub mod keys;

use ether_protocol::collab::{IceServer, StreamSignal};
use ether_protocol::share::{PeerId, SignalClientMessage, SignalServerMessage};

use crate::LinkState;
use crate::wire::WireFrame;

/// One ordered, reliable data channel to a peer. Non-blocking, like
/// [`crate::CollabTransport`]; a hub polls many. The first frames are the
/// `PeerHandshake` (text), then `CollabMessage` frames ([`crate::wire::encode_frame`]).
pub trait PeerLink {
    /// Queue a frame (cut with [`dc::fragment`]); dropped once closed.
    fn send(&mut self, frame: &WireFrame);
    /// Whole frames received since the last call (reassembled), in order.
    fn poll(&mut self, out: &mut Vec<WireFrame>);
    /// Bytes queued and not yet handed to the network (backpressure: the hub treats a
    /// link past its byte budget like the relay treats a slow reader).
    fn buffered(&self) -> usize;
    fn state(&self) -> LinkState;
    fn close(&mut self);
}

/// A socket to the signaling service (`wss://<signal>/v1/rooms/<room>/{host,join}`).
pub trait SignalLink {
    fn send(&mut self, message: &SignalClientMessage);
    fn poll(&mut self, out: &mut Vec<SignalServerMessage>);
    fn state(&self) -> LinkState;
    fn close(&mut self);
}

/// What a [`PeerEndpoint`] reports.
pub enum PeerOutput {
    /// A local signal for the other side of pairing `peer` (send it on the [`SignalLink`]).
    Signal { peer: PeerId, signal: StreamSignal },
    /// The data channel is open. Fingerprints are the DTLS certificate fingerprints as in
    /// SDP (`sha-256 AB:CD:...`), for the handshake proofs (docs/SHARING.md §4.3).
    Connected {
        peer: PeerId,
        link: BoxPeerLink,
        local_fingerprint: String,
        remote_fingerprint: String,
    },
    /// ICE/DTLS failed or timed out (no route: symmetric NAT without TURN, ...).
    Failed { peer: PeerId, reason: String },
}

/// Creates peer connections (one per pairing) and drives them. Non-blocking.
pub trait PeerEndpoint {
    /// Start a peer connection for `peer`. `offer`: create the data channel
    /// ([`dc::DC_LABEL`], ordered, reliable) and the offer (the joiner); else wait for the
    /// offer (the host). `relay_only` (`ShareCommand::SetPreferences`): gather and use
    /// relay (TURN) candidates only, hiding this device's IP; host/srflx candidates are
    /// dropped (web: `iceTransportPolicy: "relay"`).
    fn open(&mut self, peer: PeerId, offer: bool, ice_servers: &[IceServer], relay_only: bool);
    /// A remote signal for `peer`.
    fn signal(&mut self, peer: PeerId, signal: StreamSignal);
    fn poll(&mut self, out: &mut Vec<PeerOutput>);
    /// Close `peer` (its link too, if connected).
    fn close(&mut self, peer: PeerId);
}

#[cfg(not(target_arch = "wasm32"))]
pub type BoxPeerLink = Box<dyn PeerLink + Send>;
#[cfg(target_arch = "wasm32")]
pub type BoxPeerLink = Box<dyn PeerLink>;
#[cfg(not(target_arch = "wasm32"))]
pub type BoxSignalLink = Box<dyn SignalLink + Send>;
#[cfg(target_arch = "wasm32")]
pub type BoxSignalLink = Box<dyn SignalLink>;
#[cfg(not(target_arch = "wasm32"))]
pub type BoxPeerEndpoint = Box<dyn PeerEndpoint + Send>;
#[cfg(target_arch = "wasm32")]
pub type BoxPeerEndpoint = Box<dyn PeerEndpoint>;

/// Opens a signaling socket to a full URL (`wss://.../v1/rooms/<room>/host`).
#[cfg(not(target_arch = "wasm32"))]
pub type SignalConnector = Box<dyn FnMut(&str) -> BoxSignalLink + Send>;
#[cfg(target_arch = "wasm32")]
pub type SignalConnector = Box<dyn FnMut(&str) -> BoxSignalLink>;

/// Everything sharing needs from the platform (injected like the collab `Connector`).
pub struct ShareServices {
    pub signal: SignalConnector,
    pub peers: BoxPeerEndpoint,
}

/// The platform services (`p2p-transport`). Until that node lands: a signaling socket that
/// is closed at once (`fatal`) and an endpoint whose peers fail.
pub fn default_services() -> ShareServices {
    ShareServices {
        signal: Box::new(|_| Box::new(Unavailable)),
        peers: Box::new(FailedPeers::default()),
    }
}

struct Unavailable;

const UNAVAILABLE: &str = "peer-to-peer sharing is not available in this build yet";

impl SignalLink for Unavailable {
    fn send(&mut self, _: &SignalClientMessage) {}
    fn poll(&mut self, _: &mut Vec<SignalServerMessage>) {}
    fn state(&self) -> LinkState {
        LinkState::Closed {
            reason: UNAVAILABLE.into(),
            fatal: true,
        }
    }
    fn close(&mut self) {}
}

#[derive(Default)]
struct FailedPeers(Vec<PeerId>);

impl PeerEndpoint for FailedPeers {
    fn open(&mut self, peer: PeerId, _: bool, _: &[IceServer], _: bool) {
        self.0.push(peer);
    }
    fn signal(&mut self, _: PeerId, _: StreamSignal) {}
    fn poll(&mut self, out: &mut Vec<PeerOutput>) {
        out.extend(self.0.drain(..).map(|peer| PeerOutput::Failed {
            peer,
            reason: UNAVAILABLE.into(),
        }));
    }
    fn close(&mut self, _: PeerId) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_services_fail_cleanly() {
        let mut s = default_services();
        let link = (s.signal)("wss://x.test/v1/rooms/r/host");
        assert!(matches!(
            link.state(),
            LinkState::Closed { fatal: true, .. }
        ));
        let mut out = Vec::new();
        s.peers.open(3, true, &[], false);
        s.peers.poll(&mut out);
        assert!(matches!(
            out.as_slice(),
            [PeerOutput::Failed { peer: 3, .. }]
        ));
    }
}

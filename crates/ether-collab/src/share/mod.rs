//! Sharing over WebRTC data channels with the sharer's app as the hub (base-115 contract;
//! design: docs/SHARING.md).
//!
//! - [`invite`]: the invite link format (parse/format; frozen, implemented).
//! - [`dc`]: how wire frames ([`crate::wire::WireFrame`]) travel on a data channel:
//!   fragments of at most [`dc::DC_FRAGMENT_BYTES`] (frozen, implemented).
//! - [`PeerLink`]: one authenticated data channel to a peer, the seam between the WebRTC
//!   endpoints (native str0m, web UI `RTCPeerConnection`) and the hub. Implementations:
//!   node `p2p-transport`.
//! - [`file`]: the host-local `share.json` next to a shared project (frozen shape; read and
//!   written by node `share-engine`, listed by `recents-shared`).
//!
//! The hub itself is the existing [`crate::relay::Relay`] state machine run in the host's
//! process (one session), with the host's own site on a loopback link and every joiner on a
//! [`PeerLink`]: the op flow, ordering, catch-up, compaction and limits are unchanged
//! (docs/SHARING.md §2).

pub mod dc;
pub mod file;
pub mod invite;

use crate::LinkState;
use crate::wire::WireFrame;

/// One authenticated, ordered, reliable data channel to a peer (after the
/// `PeerHandshake`). Non-blocking, like [`crate::CollabTransport`]; a hub polls many.
pub trait PeerLink {
    /// Queue a frame (fragmented with [`dc::fragment`]); dropped once closed.
    fn send(&mut self, frame: &WireFrame);
    /// Whole frames received since the last call (reassembled), in order.
    fn poll(&mut self, out: &mut Vec<WireFrame>);
    /// Bytes queued and not yet handed to the network (backpressure: the hub treats a
    /// link past its byte budget like the relay treats a slow reader).
    fn buffered(&self) -> usize;
    fn state(&self) -> LinkState;
    fn close(&mut self);
}

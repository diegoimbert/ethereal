//! `ether-collab` (RESERVED; roadmap v2, owned by the `collab` node; see `docs/ROADMAP.md`).
//!
//! Keeps several replicas ("sites") of one document in sync and relays presence. The
//! frozen shapes are `ether_model::{SiteId, ActorId, OpOrigin, StampedTransaction}` and
//! `ether_protocol::collab::{CollabMessage, CollabCommand, CollabEvent, Presence}`.
//! Must stay wasm-safe (the web controller runs in a Worker).

use ether_protocol::collab::CollabMessage;
use ether_protocol::model::{SiteId, StampedTransaction};

/// How a site exchanges [`CollabMessage`]s with its peers (WebSocket relay, WebRTC, ...).
pub trait CollabTransport {
    fn send(&mut self, message: &CollabMessage);
    /// Non-blocking: messages received since the last call.
    fn poll(&mut self, out: &mut Vec<CollabMessage>);
}

/// The sync state machine of one site (reserved; implemented by the `collab` node).
pub trait CollabSession {
    fn site(&self) -> SiteId;
    /// A local transaction was committed: stamp and broadcast it.
    fn local(&mut self, transaction: StampedTransaction);
    /// Remote transactions ready to apply locally, in causal order.
    fn drain_remote(&mut self, out: &mut Vec<StampedTransaction>);
}

//! `ether-collab`: real-time collaboration between document replicas ("sites"). Design:
//! `docs/COLLAB.md`.
//!
//! - [`relay`]: the sequencer every site of a session connects to ([`relay::Relay`] state
//!   machine; [`relay::server`] WebSocket server and the `ether-collab-relay` binary, native).
//! - [`CollabTransport`]: how a site exchanges [`CollabMessage`]s with the relay. [`connect`]
//!   opens a WebSocket (a tungstenite thread natively, the browser's `WebSocket` on wasm, in
//!   the controller Worker); [`memory`] is an in-process hub for tests.
//! - [`wire`]: the opaque parts of the messages (snapshot, sync version), frames, media
//!   chunks.
//!
//! The document side (stamping, rebasing, per-site undo, applying remote transactions) is
//! in `ether-controller/src/collab`. Frozen shapes: `ether_model::{SiteId, ActorId,
//! OpOrigin, StampedTransaction}`, `ether_protocol::collab`.

pub mod memory;
pub mod relay;
pub mod wire;

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

pub use ether_protocol::collab::CollabMessage;

/// State of a site's link to the relay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkState {
    /// Connecting / handshaking.
    Connecting,
    /// Authenticated: messages flow.
    Open,
    /// Gone. `fatal` = refused (bad token, protocol version, session full): don't retry.
    Closed { reason: String, fatal: bool },
}

/// How a site exchanges [`CollabMessage`]s with the relay. Non-blocking.
pub trait CollabTransport {
    /// Queue a message (dropped if the link is closed).
    fn send(&mut self, message: &CollabMessage);
    /// Messages received since the last call, in relay order.
    fn poll(&mut self, out: &mut Vec<CollabMessage>);
    fn state(&self) -> LinkState;
    /// Close the link (after flushing what was queued, best effort).
    fn close(&mut self);
}

/// A boxed transport (`Send` natively, where controllers live on host threads).
#[cfg(not(target_arch = "wasm32"))]
pub type BoxTransport = Box<dyn CollabTransport + Send>;
#[cfg(target_arch = "wasm32")]
pub type BoxTransport = Box<dyn CollabTransport>;

/// Where and how to connect.
#[derive(Clone, Debug, PartialEq)]
pub struct ConnectRequest {
    /// Relay URL (`ws://host:port`).
    pub server: String,
    /// Session name ([`wire::valid_session_name`]).
    pub session: String,
    pub token: Option<String>,
    /// Free-form client description for the relay log.
    pub client: String,
}

/// Opens transports (the controller's default is [`connect`]; tests inject hubs).
#[cfg(not(target_arch = "wasm32"))]
pub type Connector = Box<dyn FnMut(&ConnectRequest) -> BoxTransport + Send>;
#[cfg(target_arch = "wasm32")]
pub type Connector = Box<dyn FnMut(&ConnectRequest) -> BoxTransport>;

/// Open a WebSocket to the relay (see [`CollabTransport`]). Never blocks: failures show up
/// as [`LinkState::Closed`].
pub fn connect(request: &ConnectRequest) -> BoxTransport {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Box::new(native::WsClient::connect(request))
    }
    #[cfg(target_arch = "wasm32")]
    {
        Box::new(web::WebClient::connect(request))
    }
}

/// The default [`Connector`].
pub fn default_connector() -> Connector {
    Box::new(connect)
}

//! `ether-server`: a headless Ethereal engine reachable over WebSocket (roadmap v2, owned by
//! the `remote-engine` node; see `docs/ROADMAP.md`).
//!
//! It runs the same native host as the desktop app (`ether-native`: controller, audio
//! backend, disk store, plugins) without a window, and serves the UI protocol described in
//! `ether_protocol::remote` (hello/auth handshake, JSON text frames, binary frames for bulk
//! bytes). The browser UI connects with `ui/src/transport/ws/WsTransport`.
//!
//! Ports: never hardcoded. `ServerConfig::bind` defaults to loopback on port 0 unless
//! configured; dev instances use the `+4` offset (`PORT_OFFSETS.remote`) of the instance base port
//! (`scripts/dev-env.mjs`, README "Running multiple dev instances").

use std::net::SocketAddr;

pub use ether_protocol::remote::{PROTOCOL_VERSION, ServerCapabilities, ServerInfo};

#[derive(Clone, Debug, PartialEq)]
pub struct ServerConfig {
    /// Listen address. Default: `127.0.0.1:0` (loopback, OS-chosen port).
    pub bind: SocketAddr,
    /// Shared secret clients must send in `ClientHello::token`. `None` = no auth, only
    /// allowed on loopback addresses.
    pub token: Option<String>,
    /// Display name in `ServerInfo` (default: host name).
    pub name: Option<String>,
    /// Allow a second client while one is connected (default: false = `Busy`).
    pub allow_multiple_clients: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: SocketAddr::from(([127, 0, 0, 1], 0)),
            token: None,
            name: None,
            allow_multiple_clients: false,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("refusing to serve without a token on non-loopback address {0}")]
    InsecureBind(SocketAddr),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("not implemented yet (remote-engine node)")]
    NotImplemented,
}

/// Run the server until the process is terminated. Prints the bound address (and the
/// token, if generated) on stdout.
pub fn serve(config: ServerConfig) -> Result<(), ServerError> {
    if config.token.is_none() && !config.bind.ip().is_loopback() {
        return Err(ServerError::InsecureBind(config.bind));
    }
    Err(ServerError::NotImplemented)
}

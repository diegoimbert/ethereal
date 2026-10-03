//! [`SignalLink`](super::SignalLink)s: the socket to the signaling service (docs/SHARING.md §3, §6).
//!
//! - Native ([`native::WsSignal`]): tungstenite on its own thread, `ws://` or `wss://`
//!   (rustls with the `ring` provider and the Mozilla root store).
//! - Web ([`web::WebSignal`]): the controller Worker's `WebSocket`.
//!
//! Frames are the JSON text of [`SignalClientMessage`]/[`SignalServerMessage`]. A frame the
//! client cannot parse is dropped (logged); the service closes the socket on its side for
//! malformed client frames. The link never interprets messages: `Refused` and friends are
//! the controller's business. A link is `Open` once the WebSocket upgrade succeeded.

use ether_protocol::share::{SignalClientMessage, SignalServerMessage};

use super::BoxSignalLink;

#[cfg(not(target_arch = "wasm32"))]
pub mod native;
#[cfg(target_arch = "wasm32")]
pub mod web;

/// Largest frame accepted from the service (its own frames are ≤ 64 KiB, §3.4).
pub const MAX_SIGNAL_FRAME_BYTES: usize = 256 * 1024;

/// The WebSocket URL of `url`: `http(s)://` becomes `ws(s)://`, `ws(s)://` is kept.
/// Anything else is an error (the link closes, `fatal`).
pub fn socket_url(url: &str) -> Result<String, String> {
    let url = url.trim();
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| format!("not a URL: {url}"))?;
    let scheme = match scheme.to_ascii_lowercase().as_str() {
        "ws" | "http" => "ws",
        "wss" | "https" => "wss",
        other => return Err(format!("unsupported signaling URL scheme `{other}`")),
    };
    if rest.is_empty() || rest.starts_with('/') {
        return Err(format!("no host in {url}"));
    }
    Ok(format!("{scheme}://{rest}"))
}

/// The room socket URL: `<signal>/v1/rooms/<room>/{host,join}` (`host`: the sharer's
/// socket, else a joiner's). `signal` is the service base (`https://.../signal`).
pub fn room_url(signal: &str, room: &str, host: bool) -> String {
    format!(
        "{}/v1/rooms/{room}/{}",
        signal.trim_end_matches('/'),
        if host { "host" } else { "join" }
    )
}

/// Open a signaling socket (see [`super::SignalConnector`]). Never blocks: failures show
/// up as [`LinkState::Closed`](crate::LinkState::Closed).
pub fn connect(url: &str) -> BoxSignalLink {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Box::new(native::WsSignal::connect(url))
    }
    #[cfg(target_arch = "wasm32")]
    {
        Box::new(web::WebSignal::connect(url))
    }
}

pub(crate) fn encode(message: &SignalClientMessage) -> String {
    serde_json::to_string(message).expect("signal messages serialize")
}

pub(crate) fn decode(text: &str) -> Option<SignalServerMessage> {
    match serde_json::from_str(text) {
        Ok(m) => Some(m),
        Err(e) => {
            tracing::debug!(%e, "malformed frame from the signaling service");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(
            socket_url("https://etherealws.pages.dev/signal/v1/rooms/r/host").unwrap(),
            "wss://etherealws.pages.dev/signal/v1/rooms/r/host"
        );
        assert_eq!(
            socket_url("http://127.0.0.1:8787/v1/health").unwrap(),
            "ws://127.0.0.1:8787/v1/health"
        );
        assert_eq!(socket_url("WSS://x.test/a").unwrap(), "wss://x.test/a");
        assert!(socket_url("ftp://x.test").is_err());
        assert!(socket_url("x.test/a").is_err());
        assert!(socket_url("wss:///a").is_err());
        assert_eq!(
            room_url("https://etherealws.pages.dev/signal/", "abc", true),
            "https://etherealws.pages.dev/signal/v1/rooms/abc/host"
        );
        assert_eq!(
            room_url("https://s.test", "abc", false),
            "https://s.test/v1/rooms/abc/join"
        );
    }
}

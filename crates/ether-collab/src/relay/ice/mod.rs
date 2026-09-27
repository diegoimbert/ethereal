//! NAT traversal served by the relay itself (native only; docs/COLLAB.md §10).
//!
//! - The relay binds **UDP on the same IP and port number** as its WebSocket listener
//!   ([`IceService::start`]); `--no-stun` turns it off, a failed bind only logs a warning.
//! - [`stun`]: the STUN Binding responder (pure [`StunResponder`] with injected time,
//!   rate-capped per source IP and globally; [`StunThread`] runs it on the socket).
//! - [`credentials`]: TURN REST credentials (`"<expiry>:<site id>"`, HMAC-SHA1 with a
//!   random per-run secret, never derived from the token), minted per site, 12 h TTL.
//! - `turn` (cargo feature `turn`, `--turn`): the webrtc-rs TURN server in its own tokio
//!   thread; it then owns the UDP port (Binding requests still go through the capped
//!   responder). Only for relays with a token: a tokenless relay serves STUN only.
//! - [`IceAdvertiser`] builds the per-site `IceServers` ([`crate::relay::IceProvider`]):
//!   `stun:<host>:<port>`, plus `turn:<host>:<port>?transport=udp` with credentials when
//!   TURN runs. `<host>` is `--public-host`, else the host name the site used to reach the
//!   relay (its WebSocket `Host` header, [`SiteHosts`]), else the listen address.

pub mod credentials;
pub mod peers;
pub mod stun;
#[cfg(feature = "turn")]
pub mod turn;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};

use ether_protocol::collab::IceServer;
use ether_protocol::model::SiteId;

pub use self::stun::{StunResponder, StunThread};
pub use credentials::{TurnCredential, TurnCredentials};
pub use peers::PeerFilter;

use super::{ConnId, IceProvider};

/// `true` when this build has the TURN server (cargo feature `turn`).
pub const TURN_SUPPORTED: bool = cfg!(feature = "turn");

/// Default relayed port range of `--turn` (IANA dynamic ports).
pub const DEFAULT_TURN_PORTS: (u16, u16) = (49152, 65535);

/// ICE options of the relay server (`RelayServerConfig::ice`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IceConfig {
    /// Answer STUN Binding requests on the UDP port (default on; `--no-stun` clears it).
    pub stun: bool,
    /// Run the TURN server (`--turn`; needs the `turn` feature and a relay token).
    pub turn: bool,
    /// Host name in the advertised URLs (`--public-host`).
    pub public_host: Option<String>,
    /// The TURN relayed address (`--public-ip`; default: the listen address).
    pub public_ip: Option<IpAddr>,
    /// Relayed port range, inclusive (`--turn-ports`; default [`DEFAULT_TURN_PORTS`]).
    pub turn_ports: Option<(u16, u16)>,
    /// Allow relaying to loopback/link-local/private peers (`--turn-allow-private`).
    pub turn_allow_private: bool,
}

impl Default for IceConfig {
    fn default() -> Self {
        Self {
            stun: true,
            turn: false,
            public_host: None,
            public_ip: None,
            turn_ports: None,
            turn_allow_private: false,
        }
    }
}

/// The host name each site used to reach the relay (from the WebSocket `Host` header),
/// keyed by the site id its connection claimed in `Hello`. Claims are kept per connection
/// in arrival order and the oldest live one wins, so a later claim of the same site id (a
/// contest, or a collision in another session) never redirects the current holder.
#[derive(Debug, Default)]
pub struct SiteHosts {
    claims: Mutex<HashMap<SiteId, Vec<(ConnId, String)>>>,
}

impl SiteHosts {
    pub fn claim(&self, site: SiteId, conn: ConnId, host: String) {
        let mut claims = self.claims.lock().expect("site hosts lock");
        let list = claims.entry(site).or_default();
        if !list.iter().any(|(c, _)| *c == conn) {
            list.push((conn, host));
        }
    }

    /// Forget `conn`'s claim (it disconnected).
    pub fn release(&self, site: SiteId, conn: ConnId) {
        let mut claims = self.claims.lock().expect("site hosts lock");
        if let Some(list) = claims.get_mut(&site) {
            list.retain(|(c, _)| *c != conn);
            if list.is_empty() {
                claims.remove(&site);
            }
        }
    }

    pub fn host(&self, site: SiteId) -> Option<String> {
        let claims = self.claims.lock().expect("site hosts lock");
        claims.get(&site)?.first().map(|(_, h)| h.clone())
    }

    pub fn len(&self) -> usize {
        self.claims.lock().expect("site hosts lock").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The host part of a `Host` header (`name[:port]`, `[v6][:port]`), if it is a plausible
/// DNS name or IP literal (it ends up in URLs sent back to the site).
pub fn host_name(header: &str) -> Option<String> {
    let header = header.trim();
    let name = if let Some(rest) = header.strip_prefix('[') {
        let (inner, tail) = rest.split_once(']')?;
        if !(tail.is_empty() || tail.starts_with(':')) {
            return None;
        }
        return inner
            .parse::<std::net::Ipv6Addr>()
            .ok()
            .map(|ip| format!("[{ip}]"));
    } else {
        match header.split_once(':') {
            Some((name, port)) if port.bytes().all(|b| b.is_ascii_digit()) => name,
            Some(_) => return None,
            None => header,
        }
    };
    let valid = !name.is_empty()
        && name.len() <= 253
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.');
    valid.then(|| name.to_ascii_lowercase())
}

/// A URL host for an IP (IPv6 in brackets).
pub fn ip_host(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => format!("[{v6}]"),
    }
}

/// Builds the `IceServers` advertised to each site.
#[derive(Debug, Clone)]
pub struct IceAdvertiser {
    /// `--public-host` (wins over the per-site host).
    pub public_host: Option<String>,
    /// Used when the site's host is unknown (the listen address; `None` if unspecified).
    pub fallback_host: Option<String>,
    /// The UDP port (same number as the WebSocket listener).
    pub port: u16,
    /// Present when TURN runs (and the relay has a token).
    pub turn: Option<Arc<TurnCredentials>>,
    pub hosts: Arc<SiteHosts>,
}

impl IceAdvertiser {
    /// The servers for `site` at wall-clock `now_unix` (seconds). Empty when no host is
    /// known (nothing is sent).
    pub fn servers(&self, site: SiteId, now_unix: u64) -> Vec<IceServer> {
        let host = self
            .public_host
            .as_deref()
            .map(|h| match h.parse::<std::net::Ipv6Addr>() {
                Ok(v6) => format!("[{v6}]"),
                Err(_) => h.to_string(),
            })
            .or_else(|| self.hosts.host(site))
            .or_else(|| self.fallback_host.clone());
        let Some(host) = host else {
            return Vec::new();
        };
        let port = self.port;
        let mut servers = vec![IceServer {
            urls: vec![format!("stun:{host}:{port}")],
            username: None,
            credential: None,
        }];
        if let Some(turn) = &self.turn {
            let c = turn.mint(site, now_unix);
            servers.push(IceServer {
                urls: vec![format!("turn:{host}:{port}?transport=udp")],
                username: Some(c.username),
                credential: Some(c.credential),
            });
        }
        servers
    }

    /// The [`IceProvider`] for [`crate::relay::Relay::set_ice_provider`] (the relay's clock
    /// argument is ignored: credential expiries are wall-clock unix seconds).
    pub fn into_provider(self) -> IceProvider {
        Box::new(move |site, _relay_ms| self.servers(site, credentials::unix_now()))
    }
}

/// What runs on the relay's UDP port.
#[derive(Debug)]
pub enum IceService {
    Stun(StunThread),
    #[cfg(feature = "turn")]
    Turn(turn::TurnThread),
}

/// Result of [`IceService::start`].
#[derive(Debug)]
pub struct IceStart {
    pub service: Option<IceService>,
    /// Install with `Relay::set_ice_provider` (`None`: nothing to advertise).
    pub advertiser: Option<IceAdvertiser>,
    /// One human-readable status line (`stun: ...` / `turn: ...`).
    pub status: String,
}

impl IceService {
    pub fn local_addr(&self) -> SocketAddr {
        match self {
            Self::Stun(s) => s.local_addr(),
            #[cfg(feature = "turn")]
            Self::Turn(t) => t.local_addr(),
        }
    }

    /// Bind UDP on `tcp_addr` (the WebSocket listener's actual address) and run STUN or
    /// TURN per `config`. Never fails: problems are logged and reported in `status`.
    pub fn start(
        config: &IceConfig,
        tcp_addr: SocketAddr,
        has_token: bool,
        hosts: Arc<SiteHosts>,
    ) -> IceStart {
        let off = |status: String| IceStart {
            service: None,
            advertiser: None,
            status,
        };
        if !config.stun && !config.turn {
            return off("stun: off (--no-stun)".into());
        }
        let socket = match UdpSocket::bind(tcp_addr) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(%tcp_addr, %e, "could not bind the STUN/TURN UDP port; running without it");
                return off(format!("stun: off (udp {tcp_addr}: {e})"));
            }
        };
        let fallback_host = (!tcp_addr.ip().is_unspecified()).then(|| ip_host(tcp_addr.ip()));
        let advertiser = |turn: Option<Arc<TurnCredentials>>| IceAdvertiser {
            public_host: config.public_host.clone(),
            fallback_host: fallback_host.clone(),
            port: tcp_addr.port(),
            turn,
            hosts: hosts.clone(),
        };
        let mut note = String::new();
        if config.turn && !has_token {
            tracing::warn!("--turn ignored: a relay without a token serves STUN only");
            note = " (--turn ignored: no token)".into();
        } else if config.turn && !TURN_SUPPORTED {
            tracing::warn!("--turn ignored: built without the `turn` feature");
            note = " (--turn ignored: built without the `turn` feature)".into();
        } else if config.turn {
            #[cfg(feature = "turn")]
            match turn::TurnThread::start(socket, config) {
                Ok((thread, creds)) => {
                    let status =
                        format!("turn: udp {} ({})", thread.local_addr(), thread.describe());
                    return IceStart {
                        service: Some(IceService::Turn(thread)),
                        advertiser: Some(advertiser(Some(creds))),
                        status,
                    };
                }
                Err(e) => {
                    tracing::warn!(%e, "could not start TURN; running without STUN/TURN");
                    return off(format!("turn: off ({e})"));
                }
            }
            #[cfg(not(feature = "turn"))]
            unreachable!("TURN_SUPPORTED is false");
        }
        if !config.stun {
            return off(format!("stun: off (--no-stun){note}"));
        }
        match StunThread::start(socket) {
            Ok(t) => {
                let status = format!("stun: udp {}{note}", t.local_addr());
                IceStart {
                    service: Some(IceService::Stun(t)),
                    advertiser: Some(advertiser(None)),
                    status,
                }
            }
            Err(e) => {
                tracing::warn!(%e, "could not start the STUN thread");
                off(format!("stun: off ({e})"))
            }
        }
    }
}

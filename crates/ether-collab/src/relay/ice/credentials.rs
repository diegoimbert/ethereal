//! TURN REST API credentials (docs/COLLAB.md §10).
//!
//! `username = "<expiry unix seconds>:<site id>"`, `credential = base64(HMAC-SHA1(secret,
//! username))`, with `secret` 32 random bytes from the OS generated when the relay starts,
//! kept in memory only and **never derived from the relay token** (every site holds the
//! token). A restart invalidates outstanding credentials.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ether_protocol::model::SiteId;
use hmac::{Hmac, Mac};
use sha1::Sha1;

/// Lifetime of a minted credential (the relay re-advertises every 6 h).
pub const CREDENTIAL_TTL_SECS: u64 = 12 * 3600;
/// Clock skew accepted on the expiry side: a username may expire at most
/// `now + CREDENTIAL_TTL_SECS + MAX_SKEW_SECS`.
pub const MAX_SKEW_SECS: u64 = 60;

/// Mints and checks the TURN credentials of one relay run.
pub struct TurnCredentials {
    secret: [u8; 32],
}

impl std::fmt::Debug for TurnCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TurnCredentials { secret: <hidden> }")
    }
}

/// A minted credential for one site.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnCredential {
    pub username: String,
    pub credential: String,
    /// Unix seconds.
    pub expiry: u64,
}

impl TurnCredentials {
    /// A fresh secret from the OS.
    pub fn new() -> Result<Self, String> {
        let mut secret = [0u8; 32];
        getrandom::fill(&mut secret).map_err(|e| format!("no OS entropy: {e}"))?;
        Ok(Self { secret })
    }

    /// Credentials for `site`, valid until `now_unix + CREDENTIAL_TTL_SECS`.
    pub fn mint(&self, site: SiteId, now_unix: u64) -> TurnCredential {
        let expiry = now_unix.saturating_add(CREDENTIAL_TTL_SECS);
        let username = format!("{expiry}:{}", site.0);
        let credential = STANDARD.encode(self.mac(&username));
        TurnCredential {
            username,
            credential,
            expiry,
        }
    }

    /// The credential (TURN password) for `username` if the username is acceptable at
    /// `now_unix` ([`parse_username`] and its expiry window), else `None`. The TURN auth
    /// handler derives the long-term key from it.
    pub fn password_for(&self, username: &str, now_unix: u64) -> Option<String> {
        check_username(username, now_unix)?;
        Some(STANDARD.encode(self.mac(username)))
    }

    /// `true` if `credential` is the one this relay minted for `username` and the username
    /// is acceptable at `now_unix`. Constant-time comparison.
    pub fn verify(&self, username: &str, credential: &str, now_unix: u64) -> bool {
        let Some(expected) = self.password_for(username, now_unix) else {
            return false;
        };
        let (a, b) = (expected.as_bytes(), credential.as_bytes());
        a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
    }

    fn mac(&self, username: &str) -> [u8; 20] {
        let mut mac =
            <Hmac<Sha1> as Mac>::new_from_slice(&self.secret).expect("HMAC accepts any key length");
        mac.update(username.as_bytes());
        mac.finalize().into_bytes().into()
    }
}

/// `(expiry unix seconds, site)` of a well-formed username (`"<expiry>:<site id>"`, both
/// plain decimal u64).
pub fn parse_username(username: &str) -> Option<(u64, SiteId)> {
    let (expiry, site) = username.split_once(':')?;
    let decimal = |s: &str| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit());
    if !decimal(expiry) || !decimal(site) {
        return None;
    }
    Some((expiry.parse().ok()?, SiteId(site.parse().ok()?)))
}

/// The username's `(expiry, site)` if it parses and its expiry is neither past nor later
/// than `now + CREDENTIAL_TTL_SECS + MAX_SKEW_SECS`.
pub fn check_username(username: &str, now_unix: u64) -> Option<(u64, SiteId)> {
    let (expiry, site) = parse_username(username)?;
    let latest = now_unix.saturating_add(CREDENTIAL_TTL_SECS + MAX_SKEW_SECS);
    (expiry > now_unix && expiry <= latest).then_some((expiry, site))
}

/// Unix seconds now (wall clock).
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

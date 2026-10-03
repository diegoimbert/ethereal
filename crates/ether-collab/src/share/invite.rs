//! Invite links (docs/SHARING.md §4.1). Frozen format, mirrored in
//! `ui/src/features/share/invite.ts`:
//!
//! ```text
//! https://etherealws.pages.dev/join/<room>[?s=<signal url, percent-encoded>]#<key>
//! ethereal://join/<room>[?s=...]#<key>
//! ```
//!
//! - `room`: 16 random bytes, base64url without padding (22 chars).
//! - `key`: a version character, then the secret. `1` = a v1 link key: 16 random bytes,
//!   base64url (22 chars). The key is in the **fragment**, so it never reaches the web
//!   server (Pages) nor the signaling service; only derived values do (§4.2).
//! - `s`: the signaling service when it is not the default (self-hosting).
//! - A link without a key is a bare room reference: refused today (`BadLink`), reserved for
//!   account-based joins later (the account proves membership instead of a key).

use std::fmt;

/// Length of a room id and of a v1 key secret (16 bytes in base64url, no padding).
pub const ID_CHARS: usize = 22;
/// Version character of v1 link keys.
pub const KEY_V1: char = '1';

/// A parsed invite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invite {
    pub room: String,
    /// `None`: no key (reserved for accounts).
    pub key: Option<LinkKey>,
    /// Non-default signaling service (`?s=`).
    pub signal_url: Option<String>,
}

/// A link (or member) key: version + secret (base64url).
#[derive(Clone, PartialEq, Eq)]
pub struct LinkKey {
    pub version: char,
    pub secret: String,
}

impl fmt::Debug for LinkKey {
    // Never log a secret.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LinkKey({}, ..)", self.version)
    }
}

impl fmt::Display for LinkKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.version, self.secret)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum InviteError {
    #[error("not an Ethereal invite link")]
    NotAnInvite,
    #[error("the invite link is damaged (bad room id)")]
    BadRoom,
    #[error("the invite link is damaged (bad key)")]
    BadKey,
    #[error("this invite needs a newer version of Ethereal")]
    UnknownKeyVersion,
}

fn is_b64url(s: &str) -> bool {
    s.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// A valid room id (or member id).
pub fn valid_id(s: &str) -> bool {
    s.len() == ID_CHARS && is_b64url(s)
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' => {
                let hex = s.get(i + 1..i + 3)?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

impl Invite {
    /// Parse a pasted or clicked link (`https://<any origin>/join/...` or
    /// `ethereal://join/...`; surrounding whitespace ignored).
    pub fn parse(link: &str) -> Result<Self, InviteError> {
        let link = link.trim();
        let rest = if let Some(r) = link.strip_prefix("ethereal://join/") {
            r
        } else {
            let after_scheme = link
                .strip_prefix("https://")
                .or_else(|| link.strip_prefix("http://"))
                .ok_or(InviteError::NotAnInvite)?;
            let path = &after_scheme[after_scheme.find('/').ok_or(InviteError::NotAnInvite)?..];
            path.strip_prefix("/join/")
                .ok_or(InviteError::NotAnInvite)?
        };
        let (rest, fragment) = match rest.split_once('#') {
            Some((r, f)) => (r, Some(f)),
            None => (rest, None),
        };
        let (path, query) = match rest.split_once('?') {
            Some((p, q)) => (p, Some(q)),
            None => (rest, None),
        };
        let room = path.trim_end_matches('/');
        if !valid_id(room) {
            return Err(InviteError::BadRoom);
        }
        let signal_url = query
            .into_iter()
            .flat_map(|q| q.split('&'))
            .find_map(|kv| kv.strip_prefix("s="))
            .map(|v| percent_decode(v).ok_or(InviteError::NotAnInvite))
            .transpose()?
            .filter(|s| s.starts_with("https://") || s.starts_with("http://"));
        let key = match fragment.filter(|f| !f.is_empty()) {
            None => None,
            Some(f) => {
                let mut chars = f.chars();
                let version = chars.next().ok_or(InviteError::BadKey)?;
                if version != KEY_V1 {
                    return Err(InviteError::UnknownKeyVersion);
                }
                let secret = chars.as_str();
                if !valid_id(secret) {
                    return Err(InviteError::BadKey);
                }
                Some(LinkKey {
                    version,
                    secret: secret.to_string(),
                })
            }
        };
        Ok(Invite {
            room: room.to_string(),
            key,
            signal_url,
        })
    }

    fn tail(&self) -> String {
        let mut s = self.room.clone();
        if let Some(u) = &self.signal_url {
            s.push_str("?s=");
            s.push_str(&percent_encode(u));
        }
        if let Some(k) = &self.key {
            s.push('#');
            s.push_str(&k.to_string());
        }
        s
    }

    /// The web link (`origin` without a trailing slash, e.g. `https://etherealws.pages.dev`).
    pub fn web_url(&self, origin: &str) -> String {
        format!("{}/join/{}", origin.trim_end_matches('/'), self.tail())
    }

    /// The desktop deep link.
    pub fn deep_link(&self) -> String {
        format!("ethereal://join/{}", self.tail())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOM: &str = "AbCdEfGhIjKlMnOpQrStUv";
    const SECRET: &str = "0123456789_-abcdefghij";

    #[test]
    fn web_and_deep_links_roundtrip() {
        let inv = Invite {
            room: ROOM.into(),
            key: Some(LinkKey {
                version: KEY_V1,
                secret: SECRET.into(),
            }),
            signal_url: None,
        };
        let web = inv.web_url("https://etherealws.pages.dev/");
        assert_eq!(
            web,
            format!("https://etherealws.pages.dev/join/{ROOM}#1{SECRET}")
        );
        assert_eq!(Invite::parse(&web), Ok(inv.clone()));
        let deep = inv.deep_link();
        assert_eq!(deep, format!("ethereal://join/{ROOM}#1{SECRET}"));
        assert_eq!(Invite::parse(&format!("  {deep}\n")), Ok(inv.clone()));

        let custom = Invite {
            signal_url: Some("https://signal.example.org/v?x=1".into()),
            ..inv
        };
        let web = custom.web_url("http://localhost:5173");
        assert!(web.contains("?s=https%3A%2F%2Fsignal.example.org%2Fv%3Fx%3D1#1"));
        assert_eq!(Invite::parse(&web), Ok(custom));
    }

    #[test]
    fn keyless_links_are_room_references() {
        let inv = Invite::parse(&format!("https://x.test/join/{ROOM}/")).unwrap();
        assert_eq!(inv.key, None);
    }

    #[test]
    fn damaged_links_are_refused() {
        use InviteError::*;
        assert_eq!(Invite::parse("https://x.test/song"), Err(NotAnInvite));
        assert_eq!(Invite::parse("ftp://x.test/join/a"), Err(NotAnInvite));
        assert_eq!(
            Invite::parse("https://x.test/join/short#1abc"),
            Err(BadRoom)
        );
        assert_eq!(
            Invite::parse(&format!("https://x.test/join/{ROOM}#1short")),
            Err(BadKey)
        );
        assert_eq!(
            Invite::parse(&format!("https://x.test/join/{ROOM}#2{SECRET}")),
            Err(UnknownKeyVersion)
        );
    }

    #[test]
    fn keys_never_print() {
        let k = LinkKey {
            version: KEY_V1,
            secret: SECRET.into(),
        };
        assert!(!format!("{k:?}").contains(SECRET));
    }
}

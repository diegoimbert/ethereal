//! Remote engine over WebSocket (roadmap v2, `remote-engine` node): the UI in a browser
//! talks to a headless engine (`ether-server`) or a desktop app on another machine.
//!
//! # Connection
//! 1. The client opens `ws(s)://host:port/` (no token in the URL: URLs end up in logs).
//! 2. Client → server, first text frame: a [`ClientHello`].
//! 3. Server → client: one [`ServerHello`] text frame. `Rejected` is followed by a close
//!    frame (code 4001 auth, 4002 version, 4003 server busy). A connected client that stays
//!    silent past the server's idle deadline is closed with 4004. After `Welcome` the server sends what every
//!    transport sends on connect (`Transport`, `Recording::ArmChanged`, project list, ...).
//! 4. Then every **text frame** is exactly one JSON [`ClientMessage`] (client → server) or
//!    [`ServerMessage`] (server → client), with the usual ordering rules (patches before the
//!    reply). The client's `connect()` then sends `Project::Get` for the document.
//! 5. **Binary frames** carry bulk bytes without base64 ([`encode_binary_frame`]): uploads
//!    (`Media::UploadChunk`), export downloads (`Reply` with `Bytes`) and peaks (`Reply`
//!    with `Peaks`). Either side may always use the plain JSON form instead; receivers
//!    must accept both.
//!
//! # Auth
//! The server has a shared secret token (generated at first start, shown by the desktop
//! app / printed by `ether-server`). Hellos with a missing or wrong token are rejected
//! (compared in constant time). `ServerInfo::auth_required == false` only on loopback-only
//! servers started with auth disabled.
//!
//! [`ClientMessage`]: crate::ClientMessage
//! [`ServerMessage`]: crate::ServerMessage

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Wire protocol version. Bumped on any breaking change of this crate's JSON shapes.
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ClientHello {
    pub protocol_version: u32,
    pub token: Option<String>,
    /// Free-form client description for the server log ("Ethereal web 0.2 / Firefox").
    pub client: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ServerHello {
    Welcome {
        server: ServerInfo,
        /// Opaque per-connection id (logs, future reconnect).
        session: String,
    },
    Rejected {
        reason: HelloRejection,
        message: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum HelloRejection {
    BadToken,
    UnsupportedVersion,
    /// Another client is connected and the server allows only one.
    Busy,
}

/// What a remote engine is and can do.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ServerInfo {
    /// Display name (host name or configured).
    pub name: String,
    pub app_version: String,
    pub protocol_version: u32,
    /// Dev instance id (`ETHER_INSTANCE`).
    pub instance: String,
    pub auth_required: bool,
    pub capabilities: ServerCapabilities,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ServerCapabilities {
    pub plugins: bool,
    pub recording: bool,
    pub upload: bool,
    pub export: bool,
    pub collab: bool,
}

/// First byte of a binary frame: how the payload maps into the JSON header message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[repr(u8)]
pub enum BinaryKind {
    /// The header message's single base64 byte field (`data`: `Media::UploadChunk`, the
    /// `Bytes` reply) is sent as `""`; the payload is its raw bytes.
    Bytes = 1,
    /// The header is a `Reply` with `ReplyValue::Peaks` whose `min`/`max` are one empty
    /// array per channel; the payload is, per channel in order, `n` little-endian `f32`
    /// mins then `n` maxes (`n = payload_len / (8 · channels)`).
    Peaks = 2,
}

/// Binary frame layout: `[kind: u8][header_len: u32 LE][header: UTF-8 JSON][payload]`.
pub fn encode_binary_frame(kind: BinaryKind, header_json: &str, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(5 + header_json.len() + payload.len());
    out.push(kind as u8);
    out.extend_from_slice(&(header_json.len() as u32).to_le_bytes());
    out.extend_from_slice(header_json.as_bytes());
    out.extend_from_slice(payload);
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinaryFrameError {
    TooShort,
    UnknownKind(u8),
    BadHeader,
}

/// Split a binary frame into `(kind, header JSON, payload)`.
pub fn decode_binary_frame(frame: &[u8]) -> Result<(BinaryKind, &str, &[u8]), BinaryFrameError> {
    if frame.len() < 5 {
        return Err(BinaryFrameError::TooShort);
    }
    let kind = match frame[0] {
        1 => BinaryKind::Bytes,
        2 => BinaryKind::Peaks,
        k => return Err(BinaryFrameError::UnknownKind(k)),
    };
    let len = u32::from_le_bytes([frame[1], frame[2], frame[3], frame[4]]) as usize;
    let rest = &frame[5..];
    if rest.len() < len {
        return Err(BinaryFrameError::TooShort);
    }
    let header = std::str::from_utf8(&rest[..len]).map_err(|_| BinaryFrameError::BadHeader)?;
    Ok((kind, header, &rest[len..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_frame_roundtrip() {
        let f = encode_binary_frame(BinaryKind::Bytes, r#"{"id":1}"#, &[9, 8, 7]);
        assert_eq!(&f[..5], &[1, 8, 0, 0, 0]);
        let (k, h, p) = decode_binary_frame(&f).unwrap();
        assert_eq!(
            (k, h, p),
            (BinaryKind::Bytes, r#"{"id":1}"#, &[9u8, 8, 7][..])
        );
        assert_eq!(
            decode_binary_frame(&f[..7]),
            Err(BinaryFrameError::TooShort)
        );
        assert_eq!(
            decode_binary_frame(&[3, 0, 0, 0, 0]),
            Err(BinaryFrameError::UnknownKind(3))
        );
    }
}

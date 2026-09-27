//! What `ether-collab` puts inside the opaque parts of [`CollabMessage`], and how messages
//! map to WebSocket frames.
//!
//! - `Snapshot.data` = JSON [`SnapshotData`] (bytes of the UTF-8 JSON).
//! - `SyncRequest.version` = the requester's confirmed log index as 8 little-endian bytes
//!   ([`encode_version`]); empty = "send everything" (relay → site: "send your state").
//! - Frames: every message is one JSON text frame; `Media` chunks go as remote-engine binary
//!   frames (`BinaryKind::Bytes`, header = the message with `data: ""`).

use std::collections::BTreeMap;

use ether_protocol::collab::CollabMessage;
use ether_protocol::model::{Base64Bytes, Color, SiteId};
use serde::{Deserialize, Serialize};

/// Version of the engine ↔ relay collaboration protocol (`CollabMessage::Hello`).
pub const COLLAB_PROTOCOL_VERSION: u32 = 1;

/// Largest message a relay or site accepts after the hello (snapshots of big projects).
pub const MAX_COLLAB_MESSAGE_BYTES: usize = 16 << 20;

/// Media files travel in chunks of this size.
pub const MEDIA_CHUNK_BYTES: usize = 1 << 20;

/// Presence colors, assigned by the relay in join order (first free). A subset of the
/// track palette (`ui/src/theme/tokens.ts` `TRACK_COLORS`) that reads well on dark themes.
pub const PEER_COLORS: [Color; 8] = [
    Color(0xff94a6),
    Color(0x5cffe8),
    Color(0xffa529),
    Color(0xbffb00),
    Color(0x92a7ff),
    Color(0xd86ce4),
    Color(0x25ffa8),
    Color(0xf7f47c),
];

/// Full state of a session at log position `index` (the number of transactions it
/// contains).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapshotData {
    pub index: u64,
    /// Last sequenced `seq` of every site whose transactions are included.
    pub sites: BTreeMap<SiteId, u64>,
    /// The project as an `.ether` file (JSON text; `ether_model::file`).
    pub ether: String,
}

impl SnapshotData {
    pub fn encode(&self) -> Base64Bytes {
        Base64Bytes(serde_json::to_vec(self).expect("snapshot serializes"))
    }

    pub fn decode(data: &Base64Bytes) -> Result<Self, String> {
        serde_json::from_slice(&data.0).map_err(|e| format!("bad snapshot: {e}"))
    }
}

/// `SyncRequest.version` for "I have applied `index` transactions".
pub fn encode_version(index: u64) -> Base64Bytes {
    Base64Bytes(index.to_le_bytes().to_vec())
}

/// `None` = empty version (full state wanted); `Err` = malformed.
pub fn decode_version(v: &Base64Bytes) -> Result<Option<u64>, String> {
    match v.0.len() {
        0 => Ok(None),
        8 => {
            let mut b = [0u8; 8];
            b.copy_from_slice(&v.0);
            Ok(Some(u64::from_le_bytes(b)))
        }
        n => Err(format!("bad sync version ({n} bytes)")),
    }
}

/// A session name is 1-64 chars of `[A-Za-z0-9._-]` (it is the URL path, and shows in UIs
/// and logs).
pub fn valid_session_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// The WebSocket URL of `session` on relay `server` (`ws://host:port[/]`).
pub fn session_url(server: &str, session: &str) -> String {
    format!("{}/{session}", server.trim_end_matches('/'))
}

/// The session name from a request path (`/<session>`), if valid.
pub fn session_from_path(path: &str) -> Option<&str> {
    let s = path.strip_prefix('/')?;
    let s = s.split(['?', '#']).next().unwrap_or_default();
    valid_session_name(s).then_some(s)
}

/// One outgoing WebSocket frame.
#[derive(Clone, Debug, PartialEq)]
pub enum WireFrame {
    Text(String),
    Binary(Vec<u8>),
}

/// Encode a message as a frame (media bytes as a binary frame).
pub fn encode_frame(message: &CollabMessage) -> WireFrame {
    use ether_protocol::remote::{BinaryKind, encode_binary_frame};
    if let Some((header, payload)) = media::split_payload(message) {
        let json = serde_json::to_string(&header).expect("collab message serializes");
        return WireFrame::Binary(encode_binary_frame(BinaryKind::Bytes, &json, payload));
    }
    WireFrame::Text(serde_json::to_string(message).expect("collab message serializes"))
}

pub fn decode_text(text: &str) -> Result<CollabMessage, String> {
    serde_json::from_str(text).map_err(|e| format!("bad collab message: {e}"))
}

pub fn decode_binary(frame: &[u8]) -> Result<CollabMessage, String> {
    use ether_protocol::remote::{BinaryKind, decode_binary_frame};
    let (kind, header, payload) =
        decode_binary_frame(frame).map_err(|e| format!("bad binary frame: {e:?}"))?;
    if kind != BinaryKind::Bytes {
        return Err("unexpected binary frame kind".into());
    }
    let message = decode_text(header)?;
    media::join_payload(message, payload)
}

pub mod media {
    //! The binary payload of `Media` chunks (kept apart so the rest of the wire code does
    //! not depend on that variant's shape).
    use super::*;

    /// `(header with empty data, payload)` for messages that carry bulk bytes.
    pub fn split_payload(message: &CollabMessage) -> Option<(CollabMessage, &[u8])> {
        match message {
            CollabMessage::Media {
                file,
                hash,
                offset,
                total,
                data,
            } => Some((
                CollabMessage::Media {
                    file: file.clone(),
                    hash: hash.clone(),
                    offset: *offset,
                    total: *total,
                    data: Base64Bytes(Vec::new()),
                },
                &data.0,
            )),
            _ => None,
        }
    }

    pub fn join_payload(message: CollabMessage, payload: &[u8]) -> Result<CollabMessage, String> {
        match message {
            CollabMessage::Media {
                file,
                hash,
                offset,
                total,
                data,
            } if data.0.is_empty() => Ok(CollabMessage::Media {
                file,
                hash,
                offset,
                total,
                data: Base64Bytes(payload.to_vec()),
            }),
            _ => Err("binary frame header is not a media chunk".into()),
        }
    }

    /// Split a file into `Media` chunk messages.
    pub fn chunks(file: &str, hash: &str, bytes: &[u8]) -> Vec<CollabMessage> {
        let total = bytes.len() as u64;
        let mut out = Vec::new();
        let mut offset = 0usize;
        loop {
            let end = (offset + MEDIA_CHUNK_BYTES).min(bytes.len());
            out.push(CollabMessage::Media {
                file: file.to_string(),
                hash: hash.to_string(),
                offset: offset as u64,
                total,
                data: Base64Bytes(bytes[offset..end].to_vec()),
            });
            offset = end;
            if offset >= bytes.len() {
                break;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_roundtrip() {
        assert_eq!(decode_version(&encode_version(42)), Ok(Some(42)));
        assert_eq!(decode_version(&Base64Bytes(vec![])), Ok(None));
        assert!(decode_version(&Base64Bytes(vec![1, 2])).is_err());
    }

    #[test]
    fn session_names() {
        assert_eq!(session_from_path("/jam-1"), Some("jam-1"));
        assert_eq!(session_from_path("/jam?x=1"), Some("jam"));
        assert_eq!(session_from_path("/"), None);
        assert_eq!(session_from_path("/a/b"), None);
        assert_eq!(session_from_path("/../x"), None);
        assert_eq!(session_url("ws://h:1/", "s"), "ws://h:1/s");
    }

    #[test]
    fn media_frames_roundtrip() {
        let bytes: Vec<u8> = (0..(MEDIA_CHUNK_BYTES + 10)).map(|i| i as u8).collect();
        let chunks = media::chunks("media/a.wav", "abc", &bytes);
        assert_eq!(chunks.len(), 2);
        for c in &chunks {
            let WireFrame::Binary(b) = encode_frame(c) else {
                panic!("media must be binary")
            };
            assert_eq!(&decode_binary(&b).unwrap(), c);
        }
        let hello = CollabMessage::Leave { site: SiteId(7) };
        let WireFrame::Text(t) = encode_frame(&hello) else {
            panic!("text")
        };
        assert_eq!(decode_text(&t).unwrap(), hello);
        // Empty file: one empty chunk.
        assert_eq!(media::chunks("media/e.wav", "h", &[]).len(), 1);
    }
}

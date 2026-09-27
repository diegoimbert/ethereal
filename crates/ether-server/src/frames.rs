//! Message ↔ WebSocket frame encoding (`ether_protocol::remote`).
//!
//! - Text frames: one JSON `ClientMessage` / `ServerMessage`.
//! - Binary frames (`encode_binary_frame`): bulk bytes without base64.
//!   - client → server: `Bytes` with a `Media::UploadChunk` header whose `data` is `""`.
//!   - server → client: `Bytes` for `Reply(Ok(Bytes { chunk }))` (export downloads) and
//!     `Peaks` for `Reply(Ok(Peaks))`, above a small size (tiny payloads stay JSON).

use ether_protocol::media::MediaCommand;
use ether_protocol::model::Base64Bytes;
use ether_protocol::remote::{BinaryKind, decode_binary_frame, encode_binary_frame};
use ether_protocol::{ClientMessage, Command, Reply, ReplyResult, ReplyValue, ServerMessage};

/// Payloads smaller than this stay in the JSON form.
pub const BINARY_THRESHOLD: usize = 1024;

/// An encoded outgoing frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    Text(String),
    Binary(Vec<u8>),
}

/// Encode a server message, as a binary frame when it carries bulk bytes.
pub fn encode_server(m: &ServerMessage) -> Frame {
    if let ServerMessage::Reply(Reply {
        id,
        result: ReplyResult::Ok { value },
    }) = m
    {
        match value {
            ReplyValue::Bytes { chunk } if chunk.data.0.len() >= BINARY_THRESHOLD => {
                let mut header = chunk.clone();
                let payload = std::mem::take(&mut header.data.0);
                let header = ServerMessage::Reply(Reply {
                    id: *id,
                    result: ReplyResult::Ok {
                        value: ReplyValue::Bytes { chunk: header },
                    },
                });
                return Frame::Binary(encode_binary_frame(
                    BinaryKind::Bytes,
                    &json(&header),
                    &payload,
                ));
            }
            ReplyValue::Peaks { peaks }
                if peaks.min.iter().map(Vec::len).sum::<usize>() * 8 >= BINARY_THRESHOLD
                    && peaks.min.len() == peaks.max.len()
                    && peaks
                        .min
                        .iter()
                        .zip(&peaks.max)
                        .all(|(a, b)| a.len() == b.len())
                    && peaks.min.windows(2).all(|w| w[0].len() == w[1].len()) =>
            {
                let n: usize = peaks.min.first().map_or(0, Vec::len);
                let mut payload = Vec::with_capacity(peaks.min.len() * n * 8);
                for (mins, maxs) in peaks.min.iter().zip(&peaks.max) {
                    for v in mins.iter().chain(maxs) {
                        payload.extend_from_slice(&v.to_le_bytes());
                    }
                }
                let mut header = peaks.clone();
                header.min = vec![Vec::new(); peaks.min.len()];
                header.max = vec![Vec::new(); peaks.max.len()];
                let header = ServerMessage::Reply(Reply {
                    id: *id,
                    result: ReplyResult::Ok {
                        value: ReplyValue::Peaks { peaks: header },
                    },
                });
                return Frame::Binary(encode_binary_frame(
                    BinaryKind::Peaks,
                    &json(&header),
                    &payload,
                ));
            }
            _ => {}
        }
    }
    Frame::Text(json(m))
}

/// Decode a server frame (what `ui/src/transport/ws/WsTransport.ts` does; used by tests and
/// Rust clients).
pub fn decode_server(f: &Frame) -> Result<ServerMessage, String> {
    match f {
        Frame::Text(t) => serde_json::from_str(t).map_err(|e| e.to_string()),
        Frame::Binary(b) => {
            let (kind, header, payload) = decode_binary_frame(b).map_err(|e| format!("{e:?}"))?;
            let mut m: ServerMessage = serde_json::from_str(header).map_err(|e| e.to_string())?;
            let ServerMessage::Reply(Reply {
                result: ReplyResult::Ok { value },
                ..
            }) = &mut m
            else {
                return Err("binary frames carry Ok replies".into());
            };
            match (kind, value) {
                (BinaryKind::Bytes, ReplyValue::Bytes { chunk }) => {
                    chunk.data = Base64Bytes(payload.to_vec());
                }
                (BinaryKind::Peaks, ReplyValue::Peaks { peaks }) => {
                    let ch = peaks.min.len();
                    if ch == 0 || peaks.max.len() != ch || payload.len() % (8 * ch) != 0 {
                        return Err("bad peaks payload".into());
                    }
                    let n = payload.len() / (8 * ch);
                    let f: Vec<f32> = payload
                        .chunks_exact(4)
                        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                        .collect();
                    for c in 0..ch {
                        peaks.min[c] = f[c * 2 * n..c * 2 * n + n].to_vec();
                        peaks.max[c] = f[c * 2 * n + n..(c + 1) * 2 * n].to_vec();
                    }
                }
                _ => return Err("binary kind does not match the reply".into()),
            }
            Ok(m)
        }
    }
}

fn json(m: &ServerMessage) -> String {
    serde_json::to_string(m).expect("server messages always serialize")
}

/// Why an incoming frame could not be turned into a message. `id` is the request id when
/// the header could be read (so the error can be answered with a reply).
#[derive(Debug, Clone, PartialEq)]
pub struct DecodeError {
    pub id: Option<u32>,
    pub message: String,
}

/// Parse a text frame into a `ClientMessage`.
pub fn decode_client_text(text: &str) -> Result<ClientMessage, DecodeError> {
    serde_json::from_str(text).map_err(|e| DecodeError {
        id: serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .and_then(|v| v.get("id")?.as_u64())
            .and_then(|id| u32::try_from(id).ok()),
        message: format!("invalid message: {e}"),
    })
}

/// Parse a binary frame into a `ClientMessage` (only `Bytes` + `Media::UploadChunk`).
pub fn decode_client_binary(frame: &[u8]) -> Result<ClientMessage, DecodeError> {
    let (kind, header, payload) = decode_binary_frame(frame).map_err(|e| DecodeError {
        id: None,
        message: format!("invalid binary frame: {e:?}"),
    })?;
    let mut msg = decode_client_text(header)?;
    let bad = |message: &str| DecodeError {
        id: Some(msg.id),
        message: message.to_string(),
    };
    if kind != BinaryKind::Bytes {
        return Err(bad("clients may only send Bytes binary frames"));
    }
    match &mut msg.command {
        Command::Media(MediaCommand::UploadChunk { data, .. }) => {
            if !data.0.is_empty() {
                return Err(bad("binary UploadChunk header must have empty data"));
            }
            *data = Base64Bytes(payload.to_vec());
        }
        _ => return Err(bad("binary frames only carry Media::UploadChunk")),
    }
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ether_protocol::export::ByteChunk;
    use ether_protocol::media::PeakData;

    fn reply(value: ReplyValue) -> ServerMessage {
        ServerMessage::Reply(Reply {
            id: 7,
            result: ReplyResult::Ok { value },
        })
    }

    #[test]
    fn bulk_replies_round_trip_through_binary_frames() {
        let small = reply(ReplyValue::Bytes {
            chunk: ByteChunk {
                offset: 0.0,
                data: Base64Bytes(vec![1, 2, 3]),
                eof: true,
            },
        });
        assert!(matches!(encode_server(&small), Frame::Text(_)));
        let big = reply(ReplyValue::Bytes {
            chunk: ByteChunk {
                offset: 4096.0,
                data: Base64Bytes((0..5000u32).map(|i| i as u8).collect()),
                eof: false,
            },
        });
        let f = encode_server(&big);
        assert!(matches!(f, Frame::Binary(ref b) if b[0] == 1));
        assert_eq!(decode_server(&f).unwrap(), big);
        let peaks = reply(ReplyValue::Peaks {
            peaks: PeakData {
                media: ether_protocol::model::IdGen::new(1).next(0),
                samples_per_peak: 256,
                start_frame: 0.0,
                min: vec![vec![-0.5; 100], vec![-0.25; 100]],
                max: vec![vec![0.5; 100], (0..100).map(|i| i as f32 / 100.0).collect()],
            },
        });
        let f = encode_server(&peaks);
        assert!(matches!(f, Frame::Binary(ref b) if b[0] == 2));
        assert_eq!(decode_server(&f).unwrap(), peaks);
    }

    #[test]
    fn upload_chunks_arrive_as_binary() {
        let header = r#"{"id":3,"gesture":null,"command":{"domain":"Media","command":{"type":"UploadChunk","upload":"u","offset":10,"data":""}}}"#;
        let frame = encode_binary_frame(BinaryKind::Bytes, header, &[9, 9, 9]);
        let m = decode_client_binary(&frame).unwrap();
        assert_eq!(m.id, 3);
        let Command::Media(MediaCommand::UploadChunk { data, offset, .. }) = m.command else {
            panic!()
        };
        assert_eq!((data.0, offset), (vec![9, 9, 9], 10.0));
        // Other commands, or Peaks frames, are refused with the request id.
        let other =
            r#"{"id":4,"gesture":null,"command":{"domain":"Transport","command":{"type":"Play"}}}"#;
        let e =
            decode_client_binary(&encode_binary_frame(BinaryKind::Bytes, other, &[])).unwrap_err();
        assert_eq!(e.id, Some(4));
        let e =
            decode_client_binary(&encode_binary_frame(BinaryKind::Peaks, header, &[])).unwrap_err();
        assert_eq!(e.id, Some(3));
        assert_eq!(decode_client_binary(&[1, 0]).unwrap_err().id, None);
        assert_eq!(decode_client_text(r#"{"id":5}"#).unwrap_err().id, Some(5));
    }
}

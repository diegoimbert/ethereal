//! Binary frame codec: the shared Rust ↔ TS vectors in
//! `ui/src/transport/ws/binaryFrame.vectors.json` (the TS side checks the same file in
//! `binaryFrame.test.ts`).

use ether_protocol::media::MediaCommand;
use ether_protocol::remote::{
    BinaryKind, PROTOCOL_VERSION, decode_binary_frame, encode_binary_frame,
};
use ether_protocol::{Command, ReplyResult, ReplyValue, ServerMessage};
use ether_server::frames::{Frame, decode_client_binary, decode_server};
use serde_json::Value;

fn vectors() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../ui/src/transport/ws/binaryFrame.vectors.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).expect("vector file")).unwrap()
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn rust_matches_the_shared_vectors() {
    let v = vectors();
    assert_eq!(
        v["protocol_version"], PROTOCOL_VERSION,
        "TS mirrors PROTOCOL_VERSION"
    );
    for t in v["vectors"].as_array().unwrap() {
        let name = t["name"].as_str().unwrap();
        let kind = match t["kind"].as_str().unwrap() {
            "Bytes" => BinaryKind::Bytes,
            "Peaks" => BinaryKind::Peaks,
            k => panic!("{k}"),
        };
        let header = t["header"].as_str().unwrap();
        let payload = hex(t["payload_hex"].as_str().unwrap());
        let frame = hex(t["frame_hex"].as_str().unwrap());
        assert_eq!(encode_binary_frame(kind, header, &payload), frame, "{name}");
        let (k, h, p) = decode_binary_frame(&frame).unwrap();
        assert_eq!((k, h, p), (kind, header, &payload[..]), "{name}");

        // Message level: what the server/clients rebuild from the frame.
        if header.contains(r#""domain":"Media""#) {
            let m = decode_client_binary(&frame).unwrap();
            let Command::Media(MediaCommand::UploadChunk { data, .. }) = m.command else {
                panic!("{name}")
            };
            assert_eq!(data.0, payload, "{name}");
        } else if header.starts_with(r#"{"kind""#) {
            let m = decode_server(&Frame::Binary(frame.clone())).unwrap();
            let ServerMessage::Reply(r) = m else { panic!() };
            let ReplyResult::Ok { value } = r.result else {
                panic!()
            };
            match value {
                ReplyValue::Bytes { chunk } => assert_eq!(chunk.data.0, payload, "{name}"),
                ReplyValue::Peaks { peaks } => {
                    let want = &t["peaks"];
                    assert_eq!(serde_json::to_value(&peaks.min).unwrap(), want["min"]);
                    assert_eq!(serde_json::to_value(&peaks.max).unwrap(), want["max"]);
                }
                other => panic!("{name}: {other:?}"),
            }
        }
    }
    for t in v["invalid"].as_array().unwrap() {
        let frame = hex(t["frame_hex"].as_str().unwrap());
        assert!(decode_binary_frame(&frame).is_err(), "{}", t["name"]);
    }
}

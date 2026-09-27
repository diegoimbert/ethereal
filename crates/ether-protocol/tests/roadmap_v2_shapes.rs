//! Roadmap v2 (contracts-2) wire shapes: pins the JSON the TS side relies on and exercises
//! every new internally-tagged enum that wraps data.

use ether_protocol::clips::ClipCommand;
use ether_protocol::collab::{CollabMessage, PresenceState};
use ether_protocol::drum_rack::{AutoSlice, SliceCommand};
use ether_protocol::export::*;
use ether_protocol::midi_map::{MidiInputEvent, MidiMapCommand, MidiMapEvent};
use ether_protocol::model::*;
use ether_protocol::remote::{self, ClientHello, HelloRejection, ServerHello};
use ether_protocol::tempo::TempoCommand;
use ether_protocol::*;
use serde_json::json;

fn roundtrip<T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(
    v: &T,
) -> serde_json::Value {
    let json = serde_json::to_value(v).expect("serialize");
    let back: T = serde_json::from_value(json.clone()).expect("deserialize");
    assert_eq!(&back, v);
    json
}

fn id<I: Id>(n: u128) -> I {
    I::from_ulid(Ulid(n))
}

#[test]
fn export_shapes() {
    let render = Command::Export(ExportCommand::Render {
        job: "J1".into(),
        request: ExportRequest {
            range: ExportRange::Custom {
                start: Beats(0.0),
                end: Beats(16.0),
            },
            format: ExportFormat {
                container: AudioContainer::Wav,
                bit_depth: BitDepth::Int24,
                sample_rate: None,
            },
            mode: ExportMode::Stems {
                tracks: vec![id(1)],
            },
            normalize: false,
            tail_seconds: 2.0,
            name: None,
        },
    });
    let v = roundtrip(&render);
    assert_eq!(v["domain"], "Export");
    assert_eq!(v["command"]["type"], "Render");
    assert_eq!(
        v["command"]["request"]["range"],
        json!({"type": "Custom", "start": 0.0, "end": 16.0})
    );
    assert_eq!(v["command"]["request"]["format"]["bit_depth"], "Int24");
    assert_eq!(v["command"]["request"]["mode"]["type"], "Stems");

    let done = Event::Export {
        event: ExportEvent::Done {
            job: "J1".into(),
            result: ExportResult::Download {
                downloads: vec![ExportDownload {
                    token: "t".into(),
                    name: "Song.wav".into(),
                    mime: "audio/wav".into(),
                    size: 44.0,
                }],
            },
        },
    };
    assert_eq!(roundtrip(&done)["event"]["result"]["type"], "Download");
    let bytes = ReplyValue::Bytes {
        chunk: ByteChunk {
            offset: 0.0,
            data: Base64Bytes(vec![1, 2, 3]),
            eof: true,
        },
    };
    assert_eq!(roundtrip(&bytes)["chunk"]["data"], "AQID");
}

#[test]
fn tempo_midi_slice_clip_shapes() {
    let tempo = Command::Tempo(TempoCommand::EditTempoPoint {
        id: id(2),
        time: None,
        bpm: Some(128.0),
        curve: Some(TempoCurve::Linear),
    });
    let v = roundtrip(&tempo);
    assert_eq!(v["command"]["curve"], "Linear");
    assert_eq!(v["command"]["time"], serde_json::Value::Null);

    let learn = Command::MidiMap(MidiMapCommand::Learn {
        target: Some(MidiMapTarget::Transport {
            action: TransportAction::TogglePlay,
        }),
    });
    assert_eq!(
        roundtrip(&learn)["command"]["target"],
        json!({"type": "Transport", "action": "TogglePlay"})
    );
    roundtrip(&Event::MidiMap {
        event: MidiMapEvent::Activity {
            source: MidiSource {
                port: None,
                channel: Some(1),
                control: MidiControl::PitchBend,
            },
        },
    });
    let input = MidiInputEvent {
        port: "p".into(),
        data: [0xb0, 7, 100],
        time_ms: 5.0,
    };
    assert_eq!(roundtrip(&input)["data"], json!([176, 7, 100]));

    let slice = Command::Slice(SliceCommand::Auto {
        device: id(3),
        mode: AutoSlice::Equal { count: 8 },
    });
    assert_eq!(
        roundtrip(&slice)["command"]["mode"],
        json!({"type": "Equal", "count": 8})
    );

    let fades = Command::Clip(ClipCommand::SetFadeCurves {
        id: id(4),
        fade_in: Some(FadeCurve::Curve { tension: 0.5 }),
        fade_out: None,
    });
    assert_eq!(
        roundtrip(&fades)["command"]["fade_in"],
        json!({"type": "Curve", "tension": 0.5})
    );
    let sampler = BuiltinDevice::new(BuiltinDeviceType::Sampler);
    assert_eq!(
        roundtrip(&sampler),
        json!({"type": "Sampler", "sample": null, "slices": {"enabled": false, "base_note": 36, "markers": []}})
    );
    assert_eq!(
        roundtrip(&BuiltinDevice::DrumRack),
        json!({"type": "DrumRack"})
    );
}

#[test]
fn remote_and_collab_shapes() {
    let hello = ClientHello {
        protocol_version: remote::PROTOCOL_VERSION,
        token: Some("s3cret".into()),
        client: "test".into(),
    };
    roundtrip(&hello);
    let rejected = ServerHello::Rejected {
        reason: HelloRejection::BadToken,
        message: "no".into(),
    };
    let v = roundtrip(&rejected);
    assert_eq!(
        (v["type"].clone(), v["reason"].clone()),
        (json!("Rejected"), json!("BadToken"))
    );

    // SiteId is a decimal string (u64 does not fit a JS number).
    let msg = CollabMessage::Leave {
        site: SiteId(u64::MAX),
    };
    assert_eq!(roundtrip(&msg)["site"], "18446744073709551615");
    roundtrip(&PresenceState::default());
}

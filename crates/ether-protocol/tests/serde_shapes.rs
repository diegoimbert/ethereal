//! Pins the JSON shape of representative messages (the TS side relies on it) and checks
//! that tagged enums round-trip (serde internally-tagged enums fail at *runtime* when a
//! variant wraps a non-map value, so every such shape must be exercised somewhere).

use ether_protocol::clips::ClipCommand;
use ether_protocol::mixer::MixerCommand;
use ether_protocol::model::*;
use ether_protocol::transport::TransportCommand;
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
fn client_message_shape() {
    let track: TrackId = id(1);
    let msg = ClientMessage {
        id: 7,
        gesture: Some(GestureId(3)),
        command: Command::Mixer(MixerCommand::SetVolume {
            track,
            volume: Decibels(-6.0),
        }),
    };
    let json = roundtrip(&msg);
    assert_eq!(
        json,
        json!({
            "id": 7,
            "gesture": 3,
            "command": {
                "domain": "Mixer",
                "command": { "type": "SetVolume", "track": track.to_string(), "volume": -6.0 }
            }
        })
    );

    roundtrip(&Command::Transport(TransportCommand::Play));
    roundtrip(&Command::Clip(ClipCommand::CreateMidi {
        id: id(2),
        track,
        start: Beats(0.0),
        length: Beats(4.0),
        name: None,
    }));
}

fn sample_track(id_: TrackId) -> Track {
    Track {
        id: id_,
        kind: TrackKind::Midi,
        name: "Bass".into(),
        color: Color(0xff8800),
        order: OrderKey("a0".into()),
        parent: None,
        mixer: TrackMixer::default(),
        input: TrackInput::Midi {
            port: None,
            channel: None,
        },
        output: TrackOutput::Default,
        monitor: MonitorMode::Auto,
        scale: Default::default(),
    }
}

#[test]
fn patch_and_entities_roundtrip() {
    let track = sample_track(id(10));
    let clip = Clip {
        id: id(11),
        track: track.id,
        start: Beats(8.0),
        name: "Loop".into(),
        color: None,
        muted: false,
        length: Beats(4.0),
        offset: Beats(0.0),
        looping: ClipLoop {
            enabled: true,
            start: Beats(0.0),
            end: Beats(4.0),
        },
        content: ClipContent::Audio(AudioContent {
            media: id(12),
            gain: Decibels(0.0),
            transpose: 0.0,
            fade_in: Beats(0.0),
            fade_out: Beats(0.0),
            warp: WarpSettings::default(),
        }),
    };
    let device = Device {
        id: id(13),
        track: track.id,
        order: OrderKey("a0".into()),
        name: "Sampler".into(),
        enabled: true,
        kind: DeviceKind::Builtin {
            device: BuiltinDevice::Sampler {
                sample: Some(id(12)),
            },
        },
        params: [(ParamId(1), 0.5)].into_iter().collect(),
    };
    let patch = Patch {
        revision: 5,
        changes: vec![
            PatchChange::Upsert {
                entity: Entity::Track(track.clone()),
            },
            PatchChange::Upsert {
                entity: Entity::Clip(clip),
            },
            PatchChange::Upsert {
                entity: Entity::Device(device),
            },
            PatchChange::Remove {
                key: EntityKey::Note(id(14)),
            },
        ],
        history: HistoryState::default(),
    };
    let json = roundtrip(&ServerMessage::Event(Event::Patch { patch }));
    assert_eq!(json["kind"], "Event");
    assert_eq!(json["body"]["type"], "Patch");
    let first = &json["body"]["patch"]["changes"][0];
    assert_eq!(first["type"], "Upsert");
    assert_eq!(first["entity"]["type"], "Track");
    assert_eq!(first["entity"]["value"]["name"], "Bass");

    let ops = vec![
        Op::Update {
            update: EntityUpdate::Track {
                id: track.id,
                change: TrackChange::Volume(Decibels(-3.0)),
            },
        },
        Op::Update {
            update: EntityUpdate::Device {
                id: id(13),
                change: DeviceChange::Param {
                    param: ParamId(1),
                    value: None,
                },
            },
        },
        Op::Settings {
            change: SettingsChange::CountInBars(2),
        },
        Op::Remove {
            key: EntityKey::Track(track.id),
        },
        Op::Insert {
            entity: Entity::Track(track),
        },
    ];
    for op in &ops {
        roundtrip(op);
    }
}

#[test]
fn replies_roundtrip() {
    roundtrip(&ServerMessage::Reply(Reply {
        id: 1,
        result: ReplyResult::Err {
            error: CommandError {
                code: ErrorCode::NotFound,
                message: "nope".into(),
            },
        },
    }));
    roundtrip(&ServerMessage::Reply(Reply {
        id: 2,
        result: ReplyResult::Ok {
            value: ReplyValue::Unit,
        },
    }));
    roundtrip(&Base64Bytes(vec![1, 2, 3, 255]));
}

#[test]
fn project_store_and_browser_shapes() {
    use ether_protocol::media::{BrowseLocation, MediaCommand, MediaSource};
    use ether_protocol::project::{ProjectCommand, ProjectSummary};

    let pid = IdGen::new(7).next_project_id(1_700_000_000_000);
    let json = roundtrip(&Command::Project(ProjectCommand::Open { id: pid }));
    assert_eq!(json["command"]["id"], pid.to_string());
    assert_eq!(pid.to_string().len(), 36);

    roundtrip(&ReplyValue::Projects {
        projects: vec![ProjectSummary {
            id: pid,
            name: "Song".into(),
            modified_ms: 1.0,
        }],
    });
    let json = roundtrip(&Command::Media(MediaCommand::Import {
        id: id(5),
        source: MediaSource::Location {
            location: BrowseLocation::Library { id: "lib".into() },
            path: "Drums/kick.wav".into(),
        },
    }));
    assert_eq!(json["command"]["source"]["location"]["type"], "Library");
    roundtrip(&MediaCommand::ListDirectory {
        location: BrowseLocation::ProjectMedia,
        path: String::new(),
    });
}

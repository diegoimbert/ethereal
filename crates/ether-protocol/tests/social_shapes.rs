//! base-62 wire shapes (docs/COLLAB.md §12): chat, pinned notes, peers' transports. Pins
//! the JSON the TS side relies on, and that the additions are backward compatible (older
//! presence JSON parses; an unset `transport` is omitted; older notes load unresolved).

use ether_protocol::collab::*;
use ether_protocol::model::*;
use ether_protocol::social::*;
use ether_protocol::{Command, Event};
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
fn chat_and_note_commands() {
    let chat: ChatMessageId = id(1);
    let v = roundtrip(&Command::Chat(ChatCommand::Send {
        id: chat,
        text: "hi".into(),
    }));
    assert_eq!(
        v,
        json!({"domain": "Chat", "command": {"type": "Send", "id": chat.to_string(), "text": "hi"}})
    );

    let note: PinnedNoteId = id(2);
    let track: TrackId = id(3);
    let position = NotePosition {
        beats: Beats(8.5),
        track: Some(track),
        y: 0.25,
        editor: None,
    };
    let v = roundtrip(&Command::PinnedNote(PinnedNoteCommand::Add {
        id: note,
        position: position.clone(),
        text: "louder here".into(),
        author_name: Some("Ada".into()),
    }));
    assert_eq!(v["domain"], "PinnedNote");
    assert_eq!(
        v["command"],
        json!({
            "type": "Add",
            "id": note.to_string(),
            "position": {"beats": 8.5, "track": track.to_string(), "y": 0.25},
            "text": "louder here",
            "author_name": "Ada"
        })
    );
    let v = roundtrip(&Command::PinnedNote(PinnedNoteCommand::Edit {
        id: note,
        text: None,
        position: None,
        resolved: Some(true),
    }));
    assert_eq!(v["command"]["resolved"], true);
    assert_eq!(v["command"]["text"], serde_json::Value::Null);
    roundtrip(&Command::PinnedNote(PinnedNoteCommand::Delete {
        ids: vec![note],
    }));
}

#[test]
fn entities_and_updates() {
    let author = Author {
        name: "Ada".into(),
        site: Some(SiteId(u64::MAX)),
        actor: None,
        color: Some(Color(0x5c_ffe8)),
    };
    let msg = ChatMessage {
        id: id(1),
        seq: 7,
        author: author.clone(),
        text: "hi".into(),
        sent_at: 1_700_000_000_000,
    };
    let v = roundtrip(&Entity::ChatMessage(msg));
    assert_eq!(v["type"], "ChatMessage");
    assert_eq!(v["value"]["seq"], 7);
    assert_eq!(v["value"]["author"]["site"], u64::MAX.to_string());
    assert_eq!(v["value"]["sent_at"], 1_700_000_000_000_u64);

    let note = PinnedNote {
        id: id(2),
        position: NotePosition {
            beats: Beats(0.0),
            track: None,
            y: 0.0,
            editor: None,
        },
        text: "intro".into(),
        author,
        created_at: 5,
        resolved: false,
    };
    let mut v = roundtrip(&Entity::PinnedNote(note.clone()));
    // Older notes without `resolved` load unresolved.
    v["value"].as_object_mut().unwrap().remove("resolved");
    assert_eq!(
        serde_json::from_value::<Entity>(v).unwrap(),
        Entity::PinnedNote(note.clone())
    );
    let v = roundtrip(&Op::Update {
        update: EntityUpdate::PinnedNote {
            id: note.id,
            change: PinnedNoteChange::Resolved(true),
        },
    });
    assert_eq!(
        v["update"],
        json!({"type": "PinnedNote", "id": note.id.to_string(), "change": {"field": "Resolved", "value": true}})
    );
    assert_eq!(
        serde_json::to_value(EntityKey::ChatMessage(id(1))).unwrap()["type"],
        "ChatMessage"
    );
}

#[test]
fn peer_transport_is_additive() {
    let old = json!({
        "cursor": null,
        "selected_tracks": [],
        "selected_clips": [],
        "selected_notes": [],
        "selected_devices": [],
        "view": null
    });
    let state: PresenceState = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(state.transport, None);
    assert_eq!(
        serde_json::to_value(&state).unwrap(),
        old,
        "unset is omitted"
    );

    let state = PresenceState {
        transport: Some(PeerTransport {
            position: Beats(-2.0),
            playing: true,
            sent_at_ms: 1_700_000_000_123,
            loop_region: Some(BeatRange {
                start: Beats(0.0),
                end: Beats(16.0),
            }),
        }),
        ..PresenceState::default()
    };
    let v = roundtrip(&state);
    assert_eq!(
        v["transport"],
        json!({
            "position": -2.0,
            "playing": true,
            "sent_at_ms": 1_700_000_000_123_u64,
            "loop_region": {"start": 0.0, "end": 16.0}
        })
    );
    const { assert!(PEER_TRANSPORT_REFRESH_MS <= 1000) };
}

#[test]
fn chat_received_event() {
    let a: ChatMessageId = id(1);
    let v = roundtrip(&Event::Collab {
        event: CollabEvent::ChatReceived { ids: vec![a] },
    });
    assert_eq!(
        v["event"],
        json!({"type": "ChatReceived", "ids": [a.to_string()]})
    );
}

#[test]
fn caps() {
    assert_eq!(CHAT_TEXT_MAX_CHARS, 2000);
    assert_eq!(CHAT_MAX_MESSAGES, 2000);
    assert_eq!(NOTE_TEXT_MAX_CHARS, 2000);
    assert_eq!(AUTHOR_NAME_MAX_CHARS, 64);
}

#[test]
fn piano_roll_note_position() {
    let clip: ClipId = id(9);
    let arranger = NotePosition {
        beats: Beats(1.0),
        track: None,
        y: 0.5,
        editor: None,
    };
    let v = roundtrip(&arranger);
    assert!(v.get("editor").is_none(), "unset editor is omitted");
    let editor = NotePosition {
        beats: Beats(0.0),
        track: None,
        y: 0.0,
        editor: Some(EditorNotePosition {
            clip,
            beats: Beats(2.5),
            pitch: 60.5,
        }),
    };
    let v = roundtrip(&editor);
    assert_eq!(
        v["editor"],
        json!({"clip": clip.to_string(), "beats": 2.5, "pitch": 60.5})
    );
}

//! base-62 pre-wiring (docs/COLLAB.md §12): what replies `Unsupported` until the
//! `collab-social` node lands. The node replaces these assertions with its own tests
//! (`tests/social*.rs`).

#[path = "collab_support.rs"]
mod support;

use ether_collab::memory::Hub;
use ether_core::protocol::model::*;
use ether_core::protocol::social::*;
use ether_core::protocol::*;
use support::*;

fn error_code(out: &[ServerMessage]) -> ErrorCode {
    match out.last() {
        Some(ServerMessage::Reply(Reply {
            result: ReplyResult::Err { error },
            ..
        })) => error.code,
        other => panic!("expected an error reply, got {other:#?}"),
    }
}

#[test]
fn social_commands_reply_unsupported_until_implemented() {
    let hub = Hub::default();
    let mut sites = session(&hub, 2);
    let s = &mut sites[0];
    let chat = ChatMessageId::NIL;
    let note: PinnedNoteId = s.id();
    let position = NotePosition {
        beats: Beats(4.0),
        track: None,
        y: 0.0,
    };
    for c in [
        Command::Chat(ChatCommand::Send {
            id: chat,
            text: "hi".into(),
        }),
        Command::PinnedNote(PinnedNoteCommand::Add {
            id: note,
            position: position.clone(),
            text: "look".into(),
            author_name: None,
        }),
        Command::PinnedNote(PinnedNoteCommand::Edit {
            id: note,
            text: None,
            position: Some(position.clone()),
            resolved: Some(true),
        }),
        Command::PinnedNote(PinnedNoteCommand::Delete { ids: vec![note] }),
    ] {
        let out = s.send(c.clone());
        assert_eq!(error_code(&out), ErrorCode::Unsupported, "{c:?}");
    }
    assert!(s.project().chat.is_empty() && s.project().pinned_notes.is_empty());
    // The peer sees no transport in our presence until the node publishes it.
    s.ok(Command::Collab(
        ether_core::protocol::collab::CollabCommand::SetPresence {
            presence: Default::default(),
        },
    ));
    settle(&mut sites.iter_mut().collect::<Vec<_>>(), &hub);
    assert!(sites[1].peers().iter().all(|p| p.state.transport.is_none()));
}

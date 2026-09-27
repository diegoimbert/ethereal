//! base-62 (docs/COLLAB.md §12): chat, pinned notes and peers' playheads, end to end over the
//! in-memory relay.

#[path = "collab_support.rs"]
mod support;

use ether_collab::memory::Hub;
use ether_core::protocol::collab::{CollabCommand, CollabEvent, PEER_TRANSPORT_REFRESH_MS};
use ether_core::protocol::model::*;
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::social::*;
use ether_core::protocol::transport::TransportCommand;
use ether_core::protocol::*;
use support::*;

fn send_chat(s: &mut Site, text: &str) -> ChatMessageId {
    let id = s.id();
    s.ok(Command::Chat(ChatCommand::Send {
        id,
        text: text.into(),
    }));
    id
}

fn error_code(out: &[ServerMessage]) -> ErrorCode {
    match out.last() {
        Some(ServerMessage::Reply(Reply {
            result: ReplyResult::Err { error },
            ..
        })) => error.code,
        other => panic!("expected an error reply, got {other:#?}"),
    }
}

/// Every `ChatReceived` id a site got so far.
fn received(s: &Site) -> Vec<ChatMessageId> {
    s.log
        .iter()
        .filter_map(|m| match m {
            ServerMessage::Event(Event::Collab {
                event: CollabEvent::ChatReceived { ids },
            }) => Some(ids.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}

fn texts(s: &Site) -> Vec<String> {
    s.project()
        .chat_ordered()
        .iter()
        .map(|m| m.text.clone())
        .collect()
}

fn pos(beats: f64) -> NotePosition {
    NotePosition {
        beats: Beats(beats),
        track: None,
        y: 0.0,
        editor: None,
    }
}

#[test]
fn chat_replicates_in_log_order_with_the_senders_identity() {
    let hub = Hub::default();
    let mut sites = session(&hub, 2);
    let site_a = sites[0].ctl.collab_site();
    let a1 = send_chat(&mut sites[0], "hello");
    let b1 = send_chat(&mut sites[1], "hi A");
    let a2 = send_chat(&mut sites[0], "let's go");
    settle(&mut sites.iter_mut().collect::<Vec<_>>(), &hub);
    assert_converged(&[&sites[0], &sites[1]]);
    let order = texts(&sites[0]);
    assert_eq!(order.len(), 3);
    assert_eq!(
        order,
        texts(&sites[1]),
        "every replica numbers in log order"
    );
    // Author snapshot: session name, site and the colour peers see for us.
    let m = &sites[1].project().chat[&a1];
    assert_eq!(m.author.name, "Site 0");
    assert_eq!(m.author.site, Some(site_a));
    let colour_seen_by_b = sites[1]
        .peers()
        .iter()
        .find(|p| p.site == site_a)
        .map(|p| p.color);
    assert!(m.author.color.is_some());
    assert_eq!(m.author.color, colour_seen_by_b);
    assert!(m.sent_at >= T0);
    // ChatReceived: peers' live messages only, never one's own.
    assert_eq!(received(&sites[1]), [a1, a2]);
    assert_eq!(received(&sites[0]), [b1]);
}

#[test]
fn chat_is_never_an_undo_step() {
    let hub = Hub::default();
    let mut sites = session(&hub, 2);
    let a = &mut sites[0];
    let marker: MarkerId = a.id();
    a.ok(Command::Marker(
        ether_core::protocol::markers::MarkerCommand::Add {
            id: marker,
            position: Beats(2.0),
            name: None,
            color: None,
        },
    ));
    send_chat(a, "keep me");
    a.ok(Command::Edit(EditCommand::Undo));
    assert!(
        a.project().markers.is_empty(),
        "undo skips the chat message"
    );
    assert_eq!(texts(a), ["keep me"]);
    settle(&mut sites.iter_mut().collect::<Vec<_>>(), &hub);
    assert_converged(&[&sites[0], &sites[1]]);
    assert_eq!(texts(&sites[1]), ["keep me"]);
}

#[test]
fn chat_is_validated_idempotent_and_needs_a_session() {
    let hub = Hub::default();
    let mut solo = Site::on_hub(77, &hub);
    solo.create_project("Solo");
    let out = solo.send(Command::Chat(ChatCommand::Send {
        id: ChatMessageId::NIL,
        text: "hi".into(),
    }));
    assert_eq!(error_code(&out), ErrorCode::InvalidState);

    let mut sites = session(&hub, 1);
    let a = &mut sites[0];
    for text in ["", "   \n", &"x".repeat(CHAT_TEXT_MAX_CHARS + 1)] {
        let out = a.send(Command::Chat(ChatCommand::Send {
            id: ChatMessageId::NIL,
            text: text.into(),
        }));
        assert_eq!(error_code(&out), ErrorCode::InvalidArgument, "{text:?}");
    }
    // 2000 chars but more than 4096 bytes.
    let wide = "😀".repeat(1100);
    let out = a.send(Command::Chat(ChatCommand::Send {
        id: ChatMessageId::NIL,
        text: wide,
    }));
    assert_eq!(error_code(&out), ErrorCode::InvalidArgument);
    let id = send_chat(a, "once");
    a.ok(Command::Chat(ChatCommand::Send {
        id,
        text: "twice".into(),
    }));
    assert_eq!(texts(a), ["once"], "an existing id is a no-op");
    // Not a document command: refused inside a batch.
    let batched = a.id();
    let out = a.send(Command::Edit(EditCommand::Batch {
        label: "b".into(),
        commands: vec![Command::Chat(ChatCommand::Send {
            id: batched,
            text: "in a batch".into(),
        })],
    }));
    assert!(matches!(
        out.last(),
        Some(ServerMessage::Reply(Reply {
            result: ReplyResult::Err { .. },
            ..
        }))
    ));
}

#[test]
fn chat_prunes_the_oldest_past_the_cap() {
    let hub = Hub::default();
    let mut sites = session(&hub, 2);
    for i in 0..CHAT_MAX_MESSAGES + 2 {
        send_chat(&mut sites[0], &format!("m{i}"));
        if i % 200 == 0 {
            settle(&mut sites.iter_mut().collect::<Vec<_>>(), &hub);
        }
    }
    settle(&mut sites.iter_mut().collect::<Vec<_>>(), &hub);
    assert_converged(&[&sites[0], &sites[1]]);
    let t = texts(&sites[1]);
    assert_eq!(t.len(), CHAT_MAX_MESSAGES);
    assert_eq!(t[0], "m2", "the two oldest were pruned");
    assert_eq!(t.last().map(String::as_str), Some("m2001"));
}

#[test]
fn chat_received_is_live_only_not_for_the_join_catch_up() {
    let hub = Hub::default();
    let mut sites = session(&hub, 1);
    let old = send_chat(&mut sites[0], "before B joined");
    settle(&mut sites.iter_mut().collect::<Vec<_>>(), &hub);
    let mut b = Site::on_hub(0x5151, &hub);
    b.join("ws://hub", "jam", "B", None);
    sites.push(b);
    settle(&mut sites.iter_mut().collect::<Vec<_>>(), &hub);
    assert!(sites[1].project().chat.contains_key(&old));
    assert!(
        received(&sites[1]).is_empty(),
        "the catch-up is not toasted"
    );
    let live = send_chat(&mut sites[0], "welcome");
    settle(&mut sites.iter_mut().collect::<Vec<_>>(), &hub);
    assert_eq!(received(&sites[1]), [live]);
    // A leave and re-join: the log B catches up on is not toasted either.
    sites[1].ok(Command::Collab(CollabCommand::Leave));
    let missed = send_chat(&mut sites[0], "while you were away");
    settle(&mut sites.iter_mut().collect::<Vec<_>>(), &hub);
    sites[1].join("ws://hub", "jam", "B", None);
    settle(&mut sites.iter_mut().collect::<Vec<_>>(), &hub);
    assert!(sites[1].project().chat.contains_key(&missed));
    assert_eq!(received(&sites[1]), [live]);
}

#[test]
fn pinned_notes_are_undoable_replicated_and_anyone_can_change_them() {
    let hub = Hub::default();
    let mut sites = session(&hub, 2);
    let site_a = sites[0].ctl.collab_site();
    let id: PinnedNoteId = sites[0].id();
    sites[0].ok(Command::PinnedNote(PinnedNoteCommand::Add {
        id,
        position: pos(8.0),
        text: "the bass is late here".into(),
        author_name: Some("ignored in a session".into()),
    }));
    settle(&mut sites.iter_mut().collect::<Vec<_>>(), &hub);
    let n = sites[1].project().pinned_notes[&id].clone();
    assert_eq!(n.author.name, "Site 0");
    assert_eq!(n.author.site, Some(site_a));
    assert!(n.author.color.is_some());
    assert!(!n.resolved);
    // B resolves and moves it (one gesture = one step), then discards it.
    let editor = EditorNotePosition {
        clip: sites[1].id(),
        beats: Beats(1.5),
        pitch: 60.5,
    };
    let moved = NotePosition {
        beats: Beats(0.0),
        track: None,
        y: 0.0,
        editor: Some(editor),
    };
    sites[1].ok(Command::PinnedNote(PinnedNoteCommand::Edit {
        id,
        text: Some("fixed?".into()),
        position: Some(moved.clone()),
        resolved: Some(true),
    }));
    sites[1].ok(Command::PinnedNote(PinnedNoteCommand::Delete {
        ids: vec![id],
    }));
    settle(&mut sites.iter_mut().collect::<Vec<_>>(), &hub);
    assert!(sites[0].project().pinned_notes.is_empty());
    // B's undo brings it back, as edited, for everyone.
    sites[1].ok(Command::Edit(EditCommand::Undo));
    settle(&mut sites.iter_mut().collect::<Vec<_>>(), &hub);
    assert_converged(&[&sites[0], &sites[1]]);
    let n = &sites[0].project().pinned_notes[&id];
    assert_eq!((n.text.as_str(), n.resolved), ("fixed?", true));
    assert_eq!(n.position, moved);
    // Validation comes from the model.
    let fresh: PinnedNoteId = sites[0].id();
    let out = sites[0].send(Command::PinnedNote(PinnedNoteCommand::Add {
        id: fresh,
        position: pos(-1.0),
        text: "x".into(),
        author_name: None,
    }));
    assert_eq!(error_code(&out), ErrorCode::InvalidArgument);
    let missing: PinnedNoteId = sites[0].id();
    let out = sites[0].send(Command::PinnedNote(PinnedNoteCommand::Edit {
        id: missing,
        text: None,
        position: None,
        resolved: Some(true),
    }));
    assert_eq!(error_code(&out), ErrorCode::NotFound);
}

/// The session identity authors a note only for the command that adds it (scoped): undo and
/// redo of the add, by anyone, keep the original author; other commands see no identity.
#[test]
fn undo_and_redo_of_an_add_keep_the_original_author() {
    let hub = Hub::default();
    let mut sites = session(&hub, 2);
    let site_a = sites[0].ctl.collab_site();
    let id: PinnedNoteId = sites[0].id();
    sites[0].ok(Command::PinnedNote(PinnedNoteCommand::Add {
        id,
        position: pos(2.0),
        text: "A's note".into(),
        author_name: None,
    }));
    let original = sites[0].project().pinned_notes[&id].author.clone();
    assert_eq!(original.site, Some(site_a));
    sites[0].ok(Command::Edit(EditCommand::Undo));
    assert!(sites[0].project().pinned_notes.is_empty());
    sites[0].ok(Command::Edit(EditCommand::Redo));
    assert_eq!(sites[0].project().pinned_notes[&id].author, original);
    settle(&mut sites.iter_mut().collect::<Vec<_>>(), &hub);
    // B discards it and undoes: back with A as the author, on both sites.
    sites[1].ok(Command::PinnedNote(PinnedNoteCommand::Delete { ids: vec![id] }));
    sites[1].ok(Command::Edit(EditCommand::Undo));
    settle(&mut sites.iter_mut().collect::<Vec<_>>(), &hub);
    for s in &sites {
        assert_eq!(s.project().pinned_notes[&id].author, original);
    }
    // After leaving, a new note uses `author_name` (the scope never outlives a command).
    sites[0].ok(Command::Collab(CollabCommand::Leave));
    let solo: PinnedNoteId = sites[0].id();
    sites[0].ok(Command::PinnedNote(PinnedNoteCommand::Add {
        id: solo,
        position: pos(3.0),
        text: "offline".into(),
        author_name: Some("Ada".into()),
    }));
    let a = &sites[0].project().pinned_notes[&solo].author;
    assert_eq!((a.name.as_str(), a.site, a.color), ("Ada", None, None));
}

#[test]
fn pinned_notes_work_outside_a_session() {
    let hub = Hub::default();
    let mut solo = Site::on_hub(78, &hub);
    solo.create_project("Solo");
    let id: PinnedNoteId = solo.id();
    let add = |text: &str| {
        Command::PinnedNote(PinnedNoteCommand::Add {
            id,
            position: pos(4.0),
            text: text.into(),
            author_name: Some("  Diego ".into()),
        })
    };
    solo.ok(add("solo note"));
    solo.ok(add("again"));
    let n = &solo.project().pinned_notes[&id];
    assert_eq!(n.text, "solo note", "an existing id is a no-op");
    assert_eq!(n.author.name, "Diego");
    assert_eq!((n.author.site, n.author.color), (None, None));
    assert_eq!(n.created_at, T0);
    // Inside a batch too, then undone as one step.
    let other: PinnedNoteId = solo.id();
    solo.ok(Command::Edit(EditCommand::Batch {
        label: "notes".into(),
        commands: vec![
            Command::PinnedNote(PinnedNoteCommand::Add {
                id: other,
                position: pos(1.0),
                text: "b".into(),
                author_name: None,
            }),
            Command::PinnedNote(PinnedNoteCommand::Delete { ids: vec![id] }),
        ],
    }));
    assert_eq!(solo.project().pinned_notes.len(), 1);
    assert_eq!(solo.project().pinned_notes[&other].author.name, "");
    solo.ok(Command::Edit(EditCommand::Undo));
    assert!(solo.project().pinned_notes.contains_key(&id));
    assert!(!solo.project().pinned_notes.contains_key(&other));
}

fn peer_transport(s: &Site, of: SiteId) -> Option<ether_core::protocol::collab::PeerTransport> {
    s.peers()
        .into_iter()
        .find(|p| p.site == of)
        .and_then(|p| p.state.transport)
}

/// Let the presence throttle (100 ms) pass, then settle.
fn publish(sites: &mut [Site], hub: &Hub) {
    for s in sites.iter_mut() {
        s.advance(200);
    }
    settle(&mut sites.iter_mut().collect::<Vec<_>>(), hub);
}

#[test]
fn peers_see_our_transport() {
    let hub = Hub::default();
    let mut sites = session(&hub, 2);
    let site_a = sites[0].ctl.collab_site();
    publish(&mut sites, &hub);
    let t = peer_transport(&sites[1], site_a).expect("published once synced");
    assert!(!t.playing && t.loop_region.is_none());
    // Locate while stopped: at once.
    sites[0].ok(Command::Transport(TransportCommand::Locate {
        position: Beats(16.0),
    }));
    publish(&mut sites, &hub);
    let t = peer_transport(&sites[1], site_a).unwrap();
    assert_eq!(t.position, Beats(16.0));
    // Loop and play: at once.
    let region = BeatRange {
        start: Beats(16.0),
        end: Beats(24.0),
    };
    sites[0].ok(Command::Transport(TransportCommand::SetLoopRegion {
        region,
    }));
    sites[0].ok(Command::Transport(TransportCommand::SetLoopEnabled {
        enabled: true,
    }));
    sites[0].ok(Command::Transport(TransportCommand::Play));
    publish(&mut sites, &hub);
    let t = peer_transport(&sites[1], site_a).unwrap();
    assert!(t.playing);
    assert_eq!(t.loop_region, Some(region));
    // Refreshed while playing (a new sample = a new `sent_at_ms`).
    let first = t.sent_at_ms;
    for _ in 0..(PEER_TRANSPORT_REFRESH_MS / 20 + 5) {
        sites[0].tick();
        sites[1].tick();
        hub.deliver();
    }
    sites[1].tick();
    let t = peer_transport(&sites[1], site_a).unwrap();
    assert!(
        t.sent_at_ms >= first + PEER_TRANSPORT_REFRESH_MS,
        "refreshed"
    );
    // Stop: at once; then no refresh while stopped.
    sites[0].ok(Command::Transport(TransportCommand::Stop));
    publish(&mut sites, &hub);
    let stopped = peer_transport(&sites[1], site_a).unwrap();
    assert!(!stopped.playing);
    for _ in 0..(PEER_TRANSPORT_REFRESH_MS / 20 + 5) {
        sites[0].tick();
        sites[1].tick();
        hub.deliver();
    }
    sites[1].tick();
    assert_eq!(peer_transport(&sites[1], site_a), Some(stopped));
    // Whatever the UI sends is overwritten.
    let mut presence = ether_core::protocol::collab::PresenceState::default();
    presence.transport = Some(ether_core::protocol::collab::PeerTransport {
        position: Beats(999.0),
        playing: true,
        sent_at_ms: 1,
        loop_region: None,
    });
    sites[0].ok(Command::Collab(CollabCommand::SetPresence { presence }));
    publish(&mut sites, &hub);
    let t = peer_transport(&sites[1], site_a).unwrap();
    assert!(!t.playing && t.position != Beats(999.0));
}

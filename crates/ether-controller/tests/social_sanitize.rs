//! base-62 (docs/COLLAB.md §12.1): chat is a journal. A peer's chat insert is dropped unless
//! its author is the relay-verified origin site and its `seq` is 0, and its chat removes are
//! dropped unless they are a prune (oldest messages, with its own new message, not below the
//! cap). The peer is simulated with a wire tap that appends forged ops to its next
//! transaction.

#[path = "collab_support.rs"]
mod support;

use ether_collab::memory::Hub;
use ether_controller::memory::MemoryLibrary;
use ether_controller::store::ProjectStore;
use ether_core::protocol::collab::CollabCommand;
use ether_core::protocol::markers::MarkerCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::project::ProjectCommand;
use ether_core::protocol::*;
use support::*;

fn chat(id: ChatMessageId, site: Option<SiteId>, seq: u64, text: &str) -> Op {
    Op::Insert {
        entity: Entity::ChatMessage(ChatMessage {
            id,
            seq,
            author: Author {
                name: "B".into(),
                site,
                actor: None,
                color: None,
            },
            text: text.into(),
            sent_at: 1,
        }),
    }
}

#[test]
fn forged_chat_inserts_from_a_peer_are_dropped() {
    let hub = Hub::default();
    let (mut sites, taps) = tapped_session(&hub, 2);
    let [a, b] = sites.as_mut_slice() else {
        unreachable!()
    };
    let (site_a, site_b) = (a.ctl.collab_site(), b.ctl.collab_site());
    let (spoofed, reordered, anonymous) = (b.id(), b.id(), b.id());
    // (A well-formed message can't be injected here: B's own echo check would see an op B
    // never applied. Acceptance is covered by the unit test in `social/mod.rs`.)
    taps[1].inject(vec![
        chat(spoofed, Some(site_a), 0, "posing as A"),
        chat(reordered, Some(site_b), 1, "forged order"),
        chat(anonymous, None, 0, "no author site"),
    ]);
    // Any edit carries the injected ops.
    let marker: MarkerId = b.id();
    b.ok(Command::Marker(MarkerCommand::Add {
        id: marker,
        position: Beats(4.0),
        name: None,
        color: None,
    }));
    settle(&mut [a, b], &hub);
    let p = a.project();
    assert!(
        p.markers.contains_key(&marker),
        "the rest of the transaction applies"
    );
    assert!(
        p.chat.is_empty(),
        "every forged insert is dropped: {:?}",
        p.chat
    );
    assert_converged(&[a, b]);
}

fn add_marker(s: &mut Site) -> MarkerId {
    let marker: MarkerId = s.id();
    s.ok(Command::Marker(MarkerCommand::Add {
        id: marker,
        position: Beats(4.0),
        name: None,
        color: None,
    }));
    marker
}

/// A tapped two-site session whose project already holds `n` chat messages by site A (the
/// journal of an earlier session, loaded with the project).
fn session_with_chat(hub: &Hub, n: usize) -> (Vec<Site>, Vec<Tap>, Vec<ChatMessageId>) {
    let taps: Vec<Tap> = (0..2).map(|_| Tap::default()).collect();
    let mut sites: Vec<Site> = taps
        .iter()
        .enumerate()
        .map(|(i, t)| {
            Site::new(
                0x2000 + i as u64 * 0x9e37,
                t.connector(hub),
                MemoryLibrary::new(),
            )
        })
        .collect();
    let a = &mut sites[0];
    let pid = a.create_project("Jam");
    let mut p = a.project().clone();
    let mut ids = Vec::new();
    for i in 0..n {
        let id: ChatMessageId = a.id();
        let Op::Insert { entity } = chat(id, Some(SiteId(1)), 0, &format!("old {i}")) else {
            unreachable!()
        };
        p.apply(&Op::Insert { entity }).unwrap();
        ids.push(id);
    }
    a.ctl
        .store
        .save(pid, &file::save(&p, "0.0.0").unwrap())
        .unwrap();
    a.ok(Command::Project(ProjectCommand::Open { id: pid }));
    assert_eq!(a.project().chat.len(), n);
    for i in 0..sites.len() {
        sites[i].ok(Command::Collab(CollabCommand::Join {
            server: "ws://hub".into(),
            session: "jam".into(),
            token: None,
            name: format!("Site {i}"),
        }));
        let mut refs: Vec<&mut Site> = sites.iter_mut().collect();
        settle(&mut refs, hub);
    }
    assert_eq!(
        sites[1].project().chat.len(),
        n,
        "the journal came with the snapshot"
    );
    (sites, taps, ids)
}

#[test]
fn a_peer_cannot_delete_chat_messages() {
    let hub = Hub::default();
    let (mut sites, taps, ids) = session_with_chat(&hub, 3);
    let [a, b] = sites.as_mut_slice() else {
        unreachable!()
    };
    // B deletes someone's newest message, then the oldest one: neither is a prune (no new
    // message of B's in the transaction, and far below the cap).
    for victim in [ids[2], ids[0]] {
        taps[1].inject(vec![Op::Remove {
            key: EntityKey::ChatMessage(victim),
        }]);
        let marker = add_marker(b);
        settle(&mut [a, b], &hub);
        assert!(a.project().markers.contains_key(&marker));
        assert_eq!(a.project().chat.len(), 3, "the journal is intact");
    }
    assert_converged(&[a, b]);
}

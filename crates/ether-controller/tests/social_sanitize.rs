//! base-62 (docs/COLLAB.md §12.1): a peer's chat insert is dropped unless its author is the
//! relay-verified origin site and its `seq` is 0. The peer is simulated with a wire tap that
//! appends forged ops to its next transaction.

#[path = "collab_support.rs"]
mod support;

use ether_collab::memory::Hub;
use ether_core::protocol::markers::MarkerCommand;
use ether_core::protocol::model::*;
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

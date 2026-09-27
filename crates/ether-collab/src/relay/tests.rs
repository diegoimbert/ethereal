//! Relay state machine tests.

use std::collections::BTreeMap;

use ether_protocol::collab::{CollabMessage, Presence, PresenceState};
use ether_protocol::model::{
    Base64Bytes, Color, OpOrigin, SiteId, StampedTransaction, Transaction,
};

use super::*;
use crate::wire::{COLLAB_PROTOCOL_VERSION, SnapshotData, encode_version};

fn hello(site: u64) -> CollabMessage {
    CollabMessage::Hello {
        site: SiteId(site),
        actor: None,
        name: format!("site {site}"),
        protocol_version: COLLAB_PROTOCOL_VERSION,
    }
}

fn sync(site: u64, from: Option<u64>) -> CollabMessage {
    CollabMessage::SyncRequest {
        site: SiteId(site),
        version: from.map_or(Base64Bytes(vec![]), |i| encode_version(0, i)),
    }
}

fn snapshot(index: u64) -> CollabMessage {
    CollabMessage::Snapshot {
        data: SnapshotData {
            epoch: 0,
            index,
            sites: BTreeMap::new(),
            ether: "{}".into(),
        }
        .encode(),
    }
}

fn tx(site: u64, seq: u64) -> CollabMessage {
    CollabMessage::Transaction {
        transaction: StampedTransaction {
            origin: OpOrigin {
                site: SiteId(site),
                actor: None,
                seq,
            },
            transaction: Transaction {
                label: format!("{site}/{seq}"),
                ops: vec![],
            },
        },
    }
}

fn to(out: &[Outgoing], conn: ConnId) -> Vec<CollabMessage> {
    out.iter()
        .filter(|(c, _)| *c == conn)
        .map(|(_, m)| (**m).clone())
        .collect()
}

fn kinds(ms: &[CollabMessage]) -> Vec<&'static str> {
    ms.iter()
        .map(|m| match m {
            CollabMessage::Hello { .. } => "Hello",
            CollabMessage::Transaction { .. } => "Transaction",
            CollabMessage::Update { .. } => "Update",
            CollabMessage::SyncRequest { .. } => "SyncRequest",
            CollabMessage::Snapshot { .. } => "Snapshot",
            CollabMessage::Presence { .. } => "Presence",
            CollabMessage::Media { .. } => "Media",
            CollabMessage::Leave { .. } => "Leave",
        })
        .collect()
}

/// A session created by site 1 (conn 1) with a snapshot at index 0.
fn created() -> Relay {
    let mut r = Relay::default();
    let mut out = Vec::new();
    r.connect(1, "jam").unwrap();
    r.message(1, hello(1), &mut out).unwrap();
    r.message(1, sync(1, None), &mut out).unwrap();
    assert_eq!(
        kinds(&to(&out, 1)),
        ["SyncRequest"],
        "the first site creates"
    );
    r.message(1, snapshot(0), &mut out).unwrap();
    r
}

#[test]
fn creator_then_joiner_gets_snapshot_log_and_peers() {
    let mut r = created();
    let mut out = Vec::new();
    r.message(1, tx(1, 1), &mut out).unwrap();
    assert_eq!(kinds(&to(&out, 1)), ["Transaction"], "echo to the author");
    out.clear();
    r.connect(2, "jam").unwrap();
    r.message(2, hello(2), &mut out).unwrap();
    assert!(out.is_empty(), "nothing before the sync request");
    r.message(2, sync(2, None), &mut out).unwrap();
    assert_eq!(
        kinds(&to(&out, 2)),
        ["Snapshot", "Transaction", "Hello", "Presence"]
    );
    assert_eq!(kinds(&to(&out, 1)), ["Hello", "Presence"]);
    // Colors differ.
    let color = |ms: Vec<CollabMessage>| match ms.last() {
        Some(CollabMessage::Presence { presence }) => presence.color,
        _ => panic!("presence"),
    };
    assert_ne!(color(to(&out, 1)), color(to(&out, 2)));
}

#[test]
fn joiners_wait_for_the_first_snapshot_and_creator_is_reelected() {
    let mut r = Relay::default();
    let mut out = Vec::new();
    for c in 1..=3 {
        r.connect(c, "s").unwrap();
        r.message(c, hello(c), &mut out).unwrap();
        r.message(c, sync(c, None), &mut out).unwrap();
    }
    assert_eq!(kinds(&to(&out, 1)), ["SyncRequest"]);
    assert!(to(&out, 2).is_empty() && to(&out, 3).is_empty());
    out.clear();
    // The creator leaves before sending its snapshot: the next waiting site is asked.
    r.disconnect(1, &mut out);
    assert_eq!(kinds(&to(&out, 2)), ["SyncRequest"]);
    assert!(
        r.message(3, snapshot(0), &mut out).is_err(),
        "only the creator may create"
    );
    out.clear();
    r.message(2, snapshot(0), &mut out).unwrap();
    assert_eq!(kinds(&to(&out, 3)), ["Snapshot", "Hello", "Presence"]);
}

#[test]
fn identity_and_duplicates_are_enforced() {
    let mut r = created();
    let mut out = Vec::new();
    assert!(r.message(1, tx(9, 1), &mut out).is_err(), "spoofed site");
    r.message(1, tx(1, 1), &mut out).unwrap();
    out.clear();
    r.message(1, tx(1, 1), &mut out).unwrap();
    assert!(out.is_empty(), "resend of a sequenced seq is dropped");
    let p = CollabMessage::Presence {
        presence: Presence {
            site: SiteId(42),
            actor: None,
            name: "x".repeat(200),
            color: Color(0),
            state: PresenceState::default(),
        },
    };
    r.connect(2, "jam").unwrap();
    r.message(2, hello(2), &mut out).unwrap();
    r.message(2, sync(2, None), &mut out).unwrap();
    out.clear();
    r.message(2, p, &mut out).unwrap();
    match &to(&out, 1)[..] {
        [CollabMessage::Presence { presence }] => {
            assert_eq!(presence.site, SiteId(2), "stamped with the sender's site");
            assert_ne!(presence.color, Color(0));
            assert_eq!(presence.name.len(), 64);
        }
        other => panic!("{other:?}"),
    }
    assert!(
        r.message(2, hello(2), &mut out)
            .is_err_and(|e| e.disconnect)
    );
}

#[test]
fn a_site_id_held_by_a_live_connection_cannot_be_claimed() {
    let mut r = created();
    let mut out = Vec::new();
    r.connect(2, "jam").unwrap();
    // The claim is held while the holder is probed; the newcomer's messages wait.
    r.message(2, hello(1), &mut out).unwrap();
    r.message(2, sync(1, None), &mut out).unwrap();
    assert_eq!(r.take_pings(), [1]);
    assert!(out.is_empty());
    // The holder answers (pong): the newcomer is refused, the holder stays.
    r.heard(1);
    assert_eq!(
        r.take_closing().iter().map(|c| c.0).collect::<Vec<_>>(),
        [2]
    );
    assert_eq!(r.peer_count("jam"), 1);
    // Even much later, the holder is not dropped (the contest is over).
    r.tick(60_000, &mut out);
    assert!(r.take_closing().is_empty());
    // A message from the holder is a sign of life too.
    r.connect(3, "jam").unwrap();
    r.message(3, hello(1), &mut out).unwrap();
    assert_eq!(r.take_pings(), [1]);
    r.message(1, tx(1, 1), &mut out).unwrap();
    assert_eq!(
        r.take_closing().iter().map(|c| c.0).collect::<Vec<_>>(),
        [3]
    );
    assert!(
        to(&out, 3).is_empty(),
        "nothing reached the refused newcomer"
    );
    // Once the old connection is gone, the site may come back (reconnect).
    r.disconnect(1, &mut out);
    r.connect(4, "jam").unwrap();
    r.message(4, hello(1), &mut out).unwrap();
    assert!(r.take_pings().is_empty());
}

#[test]
fn a_half_open_holder_is_dropped_after_its_probe() {
    let mut r = created();
    let mut out = Vec::new();
    r.message(1, tx(1, 1), &mut out).unwrap();
    r.tick(1_000, &mut out);
    r.connect(2, "jam").unwrap();
    r.message(2, hello(1), &mut out).unwrap();
    r.message(2, sync(1, Some(0)), &mut out).unwrap();
    assert_eq!(r.take_pings(), [1]);
    // No answer yet: still waiting.
    r.tick(1_000 + RelayConfig::default().site_probe_ms - 1, &mut out);
    assert!(r.take_closing().is_empty());
    assert!(to(&out, 2).is_empty());
    out.clear();
    // No answer in time: the holder is dropped, the newcomer's hello + sync proceed.
    r.tick(1_000 + RelayConfig::default().site_probe_ms, &mut out);
    assert_eq!(
        r.take_closing().iter().map(|c| c.0).collect::<Vec<_>>(),
        [1]
    );
    assert_eq!(r.peer_count("jam"), 1);
    assert_eq!(kinds(&to(&out, 2)), ["Transaction"], "the newcomer resumes");
    r.message(2, tx(1, 2), &mut out).unwrap();
    assert_eq!(r.log_len("jam"), (0, 2));
}

#[test]
fn a_recreated_session_is_never_resumed_at_an_old_index() {
    let mut r = created();
    let mut out = Vec::new();
    r.message(1, tx(1, 1), &mut out).unwrap();
    // Everybody leaves: the relay forgets the session; site 2 creates it again from its
    // own replica, with another epoch.
    r.disconnect(1, &mut out);
    r.connect(2, "jam").unwrap();
    r.message(2, hello(2), &mut out).unwrap();
    r.message(2, sync(2, None), &mut out).unwrap();
    let snap = CollabMessage::Snapshot {
        data: SnapshotData {
            epoch: 5,
            index: 0,
            sites: BTreeMap::new(),
            ether: "{}".into(),
        }
        .encode(),
    };
    r.message(2, snap, &mut out).unwrap();
    out.clear();
    // Site 1 comes back with "epoch 0, index 0": it gets the new snapshot, not a resume.
    r.connect(3, "jam").unwrap();
    r.message(3, hello(1), &mut out).unwrap();
    r.message(3, sync(1, Some(0)), &mut out).unwrap();
    assert_eq!(kinds(&to(&out, 3))[0], "Snapshot");
}

#[test]
fn conn_queue_holds_a_full_catch_up() {
    let c = RelayConfig::default();
    let q = server::conn_queue(&c);
    assert!(q >= c.max_log + c.max_media_bytes / crate::wire::MEDIA_CHUNK_BYTES);
    assert!(q >= server::CONN_QUEUE);
}

#[test]
fn resume_sends_only_what_is_missing() {
    let mut r = created();
    let mut out = Vec::new();
    for seq in 1..=3 {
        r.message(1, tx(1, seq), &mut out).unwrap();
    }
    r.connect(2, "jam").unwrap();
    r.message(2, hello(2), &mut out).unwrap();
    out.clear();
    r.message(2, sync(2, Some(2)), &mut out).unwrap();
    assert_eq!(
        kinds(&to(&out, 2)),
        ["Transaction", "Hello", "Presence"],
        "only the 3rd transaction"
    );
    // An index past the log falls back to the full state.
    r.connect(3, "jam").unwrap();
    r.message(3, hello(3), &mut out).unwrap();
    out.clear();
    r.message(3, sync(3, Some(99)), &mut out).unwrap();
    assert_eq!(
        kinds(&to(&out, 3))[..4],
        ["Snapshot", "Transaction", "Transaction", "Transaction"]
    );
}

#[test]
fn compaction_truncates_the_log() {
    let mut r = Relay::new(RelayConfig {
        compact_after: 3,
        ..RelayConfig::default()
    });
    let mut out = Vec::new();
    r.connect(1, "jam").unwrap();
    r.message(1, hello(1), &mut out).unwrap();
    r.message(1, sync(1, None), &mut out).unwrap();
    r.message(1, snapshot(0), &mut out).unwrap();
    out.clear();
    for seq in 1..=4 {
        r.message(1, tx(1, seq), &mut out).unwrap();
    }
    assert_eq!(
        kinds(&to(&out, 1)).last(),
        Some(&"SyncRequest"),
        "asked for a snapshot"
    );
    assert!(
        r.message(1, snapshot(9), &mut out).is_err(),
        "index past the log"
    );
    r.message(1, snapshot(4), &mut out).unwrap();
    assert_eq!(r.log_len("jam"), (4, 0));
    r.connect(2, "jam").unwrap();
    r.message(2, hello(2), &mut out).unwrap();
    out.clear();
    r.message(2, sync(2, Some(1)), &mut out).unwrap();
    assert_eq!(kinds(&to(&out, 2))[0], "Snapshot", "compacted: full state");
}

#[test]
fn hard_log_cap_disconnects() {
    let mut r = Relay::new(RelayConfig {
        compact_after: 100,
        max_log: 2,
        ..RelayConfig::default()
    });
    let mut out = Vec::new();
    r.connect(1, "jam").unwrap();
    r.message(1, hello(1), &mut out).unwrap();
    r.message(1, sync(1, None), &mut out).unwrap();
    r.message(1, snapshot(0), &mut out).unwrap();
    r.message(1, tx(1, 1), &mut out).unwrap();
    r.message(1, tx(1, 2), &mut out).unwrap();
    assert!(
        r.message(1, tx(1, 3), &mut out)
            .is_err_and(|e| e.disconnect)
    );
}

#[test]
fn media_is_cached_for_late_joiners_within_limits() {
    let mut r = Relay::new(RelayConfig {
        max_media_bytes: 10,
        ..RelayConfig::default()
    });
    let mut out = Vec::new();
    r.connect(1, "jam").unwrap();
    r.message(1, hello(1), &mut out).unwrap();
    r.message(1, sync(1, None), &mut out).unwrap();
    let chunk = |file: &str, n: usize| CollabMessage::Media {
        file: file.into(),
        hash: "h".into(),
        offset: 0,
        total: n as u64,
        data: Base64Bytes(vec![1; n]),
    };
    // The creator uploads media before its snapshot.
    r.message(1, chunk("media/a.wav", 8), &mut out).unwrap();
    r.message(1, snapshot(0), &mut out).unwrap();
    r.message(1, chunk("media/b.wav", 8), &mut out).unwrap();
    let bad = CollabMessage::Media {
        file: "media/c.wav".into(),
        hash: "h".into(),
        offset: 5,
        total: 6,
        data: Base64Bytes(vec![1; 4]),
    };
    assert!(r.message(1, bad, &mut out).is_err());
    r.connect(2, "jam").unwrap();
    r.message(2, hello(2), &mut out).unwrap();
    out.clear();
    r.message(2, sync(2, None), &mut out).unwrap();
    assert_eq!(
        kinds(&to(&out, 2)),
        ["Media", "Snapshot", "Hello", "Presence"],
        "b.wav is over the cache limit"
    );
}

#[test]
fn leave_is_broadcast_and_empty_sessions_are_dropped() {
    let mut r = created();
    let mut out = Vec::new();
    r.connect(2, "jam").unwrap();
    r.message(2, hello(2), &mut out).unwrap();
    r.message(2, sync(2, None), &mut out).unwrap();
    out.clear();
    r.message(2, CollabMessage::Leave { site: SiteId(2) }, &mut out)
        .unwrap();
    assert_eq!(kinds(&to(&out, 1)), ["Leave"]);
    assert_eq!(r.peer_count("jam"), 1);
    r.disconnect(1, &mut out);
    assert_eq!(r.session_count(), 0);
}

#[test]
fn limits_on_sessions_and_sites() {
    let mut r = Relay::new(RelayConfig {
        max_sessions: 1,
        max_sites_per_session: 1,
        ..RelayConfig::default()
    });
    r.connect(1, "a").unwrap();
    assert_eq!(r.connect(2, "b"), Err(Refusal::TooManySessions));
    assert_eq!(r.connect(3, "a"), Err(Refusal::SessionFull));
    assert_eq!(r.connect(4, "../x"), Err(Refusal::BadSession));
}

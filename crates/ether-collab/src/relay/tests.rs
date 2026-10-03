//! Relay state machine tests.

use std::collections::BTreeMap;

use ether_protocol::collab::{
    CollabMessage, IceServer, Presence, PresenceState, StreamSignal, TransportRequest,
};
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
            CollabMessage::Pointer { .. } => "Pointer",
            CollabMessage::Signal { .. } => "Signal",
            CollabMessage::Listen { .. } => "Listen",
            CollabMessage::Unlisten { .. } => "Unlisten",
            CollabMessage::TransportRequest { .. } => "TransportRequest",
            CollabMessage::StreamClock { .. } => "StreamClock",
            CollabMessage::IceServers { .. } => "IceServers",
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
        ["Snapshot", "Transaction", "Presence", "Hello", "Presence"]
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
    assert_eq!(
        kinds(&to(&out, 3)),
        ["Snapshot", "Presence", "Hello", "Presence"]
    );
}

/// The seqs of the transactions sent to `conn`, in order.
fn seqs_to(out: &[Outgoing], conn: ConnId) -> Vec<u64> {
    to(out, conn)
        .into_iter()
        .filter_map(|m| match m {
            CollabMessage::Transaction { transaction } => Some(transaction.origin.seq),
            _ => None,
        })
        .collect()
}

#[test]
fn transactions_sent_while_waiting_for_the_first_snapshot_are_sequenced_in_order() {
    // collab-converge: site 2 comes back to an emptied session and resends its pending
    // edit (seq 1) right after its `SyncRequest`, before site 1 re-created the session.
    // Dropping it and then sequencing its next edit (seq 2) would make seq 1 a "resend"
    // forever: site 2 would keep it pending and never converge.
    let mut r = Relay::default();
    let mut out = Vec::new();
    for c in 1..=2 {
        r.connect(c, "s").unwrap();
        r.message(c, hello(c), &mut out).unwrap();
        r.message(c, sync(c, Some(7)), &mut out).unwrap();
    }
    assert_eq!(kinds(&to(&out, 1)), ["SyncRequest"], "site 1 re-creates");
    r.message(2, tx(2, 1), &mut out).unwrap();
    // The creator resends its own pending edit too: its snapshot includes it.
    r.message(1, tx(1, 5), &mut out).unwrap();
    assert!(seqs_to(&out, 2).is_empty(), "nothing sequenced yet");
    out.clear();
    let snap = CollabMessage::Snapshot {
        data: SnapshotData {
            epoch: 0,
            index: 0,
            sites: BTreeMap::from([(SiteId(1), 5)]),
            ether: "{}".into(),
        }
        .encode(),
    };
    r.message(1, snap, &mut out).unwrap();
    // Site 2 gets the snapshot, then its held edit is sequenced (and echoed); the
    // creator's is a duplicate of its snapshot.
    assert_eq!(kinds(&to(&out, 2))[0], "Snapshot");
    assert_eq!(seqs_to(&out, 2), [1]);
    assert_eq!(seqs_to(&out, 1), [1]);
    out.clear();
    r.message(2, tx(2, 2), &mut out).unwrap();
    r.message(2, tx(2, 1), &mut out).unwrap();
    assert_eq!(
        seqs_to(&out, 2),
        [2],
        "seq 2 after seq 1, the resend dropped"
    );
    assert_eq!(r.log_len("s"), (0, 2));
    // A transaction before any `SyncRequest` is a protocol error: the link is closed (the
    // site resends everything, in order, on its next link).
    r.connect(3, "s").unwrap();
    r.message(3, hello(3), &mut out).unwrap();
    assert!(
        r.message(3, tx(3, 1), &mut out)
            .is_err_and(|e| e.disconnect)
    );
}

#[test]
fn other_protocol_versions_are_refused_at_the_hello() {
    let mut r = created();
    let mut out = Vec::new();
    r.connect(2, "jam").unwrap();
    let old = CollabMessage::Hello {
        site: SiteId(2),
        actor: None,
        name: "old build".into(),
        protocol_version: COLLAB_PROTOCOL_VERSION - 1,
    };
    assert!(
        r.message(2, old, &mut out)
            .is_err_and(|e| e.disconnect && e.reason.contains("protocol"))
    );
    assert_eq!(
        COLLAB_PROTOCOL_VERSION, 2,
        "base-62 bumped it (COLLAB.md §12)"
    );
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
    assert_eq!(
        kinds(&to(&out, 2)),
        ["Transaction", "Presence"],
        "the newcomer resumes (then its own presence)"
    );
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
        ["Transaction", "Presence", "Hello", "Presence"],
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
        ["Media", "Snapshot", "Presence", "Hello", "Presence"],
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

// ─── base-53: pointer, site-to-site routing, ICE advertisement ───────────────────────────

/// `created()` plus sites 2 and 3 (conns 2 and 3) synced.
fn three() -> Relay {
    let mut r = created();
    let mut out = Vec::new();
    for c in [2, 3] {
        r.connect(c, "jam").unwrap();
        r.message(c, hello(c), &mut out).unwrap();
        r.message(c, sync(c, None), &mut out).unwrap();
    }
    r
}

fn signal(from: u64, to: u64) -> CollabMessage {
    CollabMessage::Signal {
        from: SiteId(from),
        to: SiteId(to),
        stream: 7,
        signal: StreamSignal::Offer { sdp: "v=0".into() },
    }
}

#[test]
fn pointer_is_stamped_and_broadcast_to_others() {
    let mut r = three();
    let mut out = Vec::new();
    let p = CollabMessage::Pointer {
        site: SiteId(99),
        pointer: None,
    };
    r.message(2, p, &mut out).unwrap();
    assert!(to(&out, 2).is_empty(), "not echoed");
    for c in [1, 3] {
        assert_eq!(
            to(&out, c),
            [CollabMessage::Pointer {
                site: SiteId(2),
                pointer: None
            }],
            "stamped with the sender's site"
        );
    }
    // Not cached: a late joiner gets no pointers.
    out.clear();
    r.connect(4, "jam").unwrap();
    r.message(4, hello(4), &mut out).unwrap();
    r.message(4, sync(4, None), &mut out).unwrap();
    assert!(!kinds(&to(&out, 4)).contains(&"Pointer"));
}

#[test]
fn site_to_site_messages_reach_only_their_target() {
    let mut r = three();
    let mut out = Vec::new();
    r.message(2, signal(2, 3), &mut out).unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(to(&out, 3), [signal(2, 3)]);
    out.clear();
    for m in [
        CollabMessage::Listen {
            from: SiteId(2),
            to: SiteId(1),
            stream: 7,
        },
        CollabMessage::TransportRequest {
            from: SiteId(2),
            to: SiteId(1),
            stream: 7,
            request: TransportRequest::Play,
        },
    ] {
        r.message(2, m.clone(), &mut out).unwrap();
        assert_eq!(to(&out, 1), [m]);
        assert_eq!(out.len(), 1);
        out.clear();
    }
    // Spoofed sender, self-target, unknown target, relay-only messages: dropped.
    assert!(r.message(2, signal(3, 1), &mut out).is_err());
    assert!(r.message(2, signal(2, 2), &mut out).is_err());
    assert!(r.message(2, signal(2, 42), &mut out).is_err());
    assert!(
        r.message(2, CollabMessage::IceServers { servers: vec![] }, &mut out)
            .is_err()
    );
    // Oversized SDP.
    let big = CollabMessage::Signal {
        from: SiteId(2),
        to: SiteId(3),
        stream: 7,
        signal: StreamSignal::Answer {
            sdp: "x".repeat(MAX_SDP_BYTES + 1),
        },
    };
    assert!(r.message(2, big, &mut out).is_err());
    assert!(out.is_empty());
    // Other sessions are out of reach, and a site that has not synced cannot signal or
    // be signaled.
    r.connect(10, "other").unwrap();
    r.message(10, hello(10), &mut out).unwrap();
    assert!(r.message(10, signal(10, 1), &mut out).is_err());
    r.connect(11, "jam").unwrap();
    r.message(11, hello(11), &mut out).unwrap();
    assert!(r.message(11, signal(11, 1), &mut out).is_err());
    assert!(r.message(1, signal(1, 11), &mut out).is_err(), "not ready");
    assert!(out.is_empty());
}

#[test]
fn ice_servers_are_advertised_per_site_and_refreshed() {
    let mut r = Relay::new(RelayConfig {
        ice_refresh_ms: 1_000,
        ..RelayConfig::default()
    });
    r.set_ice_provider(Box::new(|site, now| {
        vec![IceServer {
            urls: vec!["stun:relay.test:3478".into()],
            username: Some(format!("{now}:{}", site.0)),
            credential: None,
        }]
    }));
    let mut out = Vec::new();
    r.connect(1, "jam").unwrap();
    r.message(1, hello(1), &mut out).unwrap();
    r.message(1, sync(1, None), &mut out).unwrap();
    assert_eq!(kinds(&to(&out, 1)), ["SyncRequest"], "not before sync");
    out.clear();
    r.message(1, snapshot(0), &mut out).unwrap();
    let ice = to(&out, 1);
    assert_eq!(
        kinds(&ice),
        ["Presence", "IceServers"],
        "the creator gets them once ready (after its own presence)"
    );
    let CollabMessage::IceServers { servers } = &ice[1] else {
        unreachable!()
    };
    assert_eq!(servers[0].username.as_deref(), Some("0:1"));
    out.clear();
    r.connect(2, "jam").unwrap();
    r.message(2, hello(2), &mut out).unwrap();
    r.message(2, sync(2, None), &mut out).unwrap();
    assert_eq!(kinds(&to(&out, 2)).last(), Some(&"IceServers"));
    assert!(
        !kinds(&to(&out, 1)).contains(&"IceServers"),
        "once per site"
    );
    out.clear();
    r.tick(500, &mut out);
    assert!(out.is_empty());
    r.tick(1_000, &mut out);
    assert_eq!(kinds(&to(&out, 1)), ["IceServers"], "refreshed");
    assert_eq!(kinds(&to(&out, 2)), ["IceServers"]);
}

/// docs/COLLAB.md §12.1: once synced, a site gets its own stamped presence (its colour),
/// after its catch-up and before the peers'; the creator too.
#[test]
fn a_synced_site_learns_its_own_colour() {
    let mut r = Relay::default();
    let mut out = Vec::new();
    r.connect(1, "jam").unwrap();
    r.message(1, hello(1), &mut out).unwrap();
    r.message(1, sync(1, None), &mut out).unwrap();
    out.clear();
    r.message(1, snapshot(0), &mut out).unwrap();
    let own = |ms: Vec<CollabMessage>, site: u64| {
        ms.into_iter()
            .find_map(|m| match m {
                CollabMessage::Presence { presence } if presence.site == SiteId(site) => {
                    Some(presence)
                }
                _ => None,
            })
            .expect("own presence")
    };
    let first = own(to(&out, 1), 1);
    out.clear();
    r.message(1, tx(1, 1), &mut out).unwrap();
    r.connect(2, "jam").unwrap();
    r.message(2, hello(2), &mut out).unwrap();
    out.clear();
    r.message(2, sync(2, None), &mut out).unwrap();
    let second = own(to(&out, 2), 2);
    assert_ne!(first.color, second.color);
    assert_eq!(second.state, PresenceState::default());
    // Only to itself: the peer gets the newcomer's presence once, as before.
    assert_eq!(kinds(&to(&out, 1)), ["Hello", "Presence"]);
    // After the log.
    assert_eq!(
        kinds(&to(&out, 2))[..3],
        ["Snapshot", "Transaction", "Presence"]
    );
}

// ─── base-115 hub hooks (docs/SHARING.md §2.3) ──────────────────────────────────────────

#[test]
fn only_the_snapshot_source_creates_and_compacts() {
    let mut r = Relay::new(RelayConfig {
        compact_after: 2,
        ..RelayConfig::default()
    });
    let mut out = Vec::new();
    // A view-only joiner says hello first: it is never asked to create the session.
    r.connect(1, "share").unwrap();
    r.connect(2, "share").unwrap();
    r.set_snapshot_source(2);
    r.message(1, hello(1), &mut out).unwrap();
    r.message(1, sync(1, None), &mut out).unwrap();
    assert!(to(&out, 1).is_empty(), "not the snapshot source");
    r.message(2, hello(2), &mut out).unwrap();
    r.message(2, sync(2, None), &mut out).unwrap();
    assert_eq!(kinds(&to(&out, 2)), ["SyncRequest"]);
    r.message(2, snapshot(0), &mut out).unwrap();
    assert_eq!(kinds(&to(&out, 1))[0], "Snapshot");
    out.clear();
    // Compaction asks the source even though conn 1 is older.
    for seq in 1..=3 {
        r.message(1, tx(1, seq), &mut out).unwrap();
    }
    let asked = |out: &[Outgoing], c| {
        to(out, c)
            .iter()
            .any(|m| matches!(m, CollabMessage::SyncRequest { .. }))
    };
    assert!(asked(&out, 2) && !asked(&out, 1));
    // The source left before creating: nobody else is asked.
    out.clear();
    r.disconnect(2, &mut out);
    r.disconnect(1, &mut out);
    r.connect(3, "share").unwrap();
    r.connect(4, "share").unwrap();
    r.set_snapshot_source(4);
    r.message(3, hello(3), &mut out).unwrap();
    r.message(3, sync(3, None), &mut out).unwrap();
    r.disconnect(4, &mut out);
    assert!(to(&out, 3).is_empty());
}

#[test]
fn set_color_takes_a_free_colour_only() {
    let mut r = Relay::default();
    r.connect(1, "share").unwrap();
    r.connect(2, "share").unwrap();
    let c1 = r.color(1).unwrap();
    assert!(!r.set_color(2, c1), "taken");
    assert!(r.set_color(2, Color(0x123456)));
    assert_eq!(r.color(2), Some(Color(0x123456)));
    assert!(r.set_color(1, c1), "its own colour");
    assert!(!r.set_color(9, c1), "unknown connection");
    let mut out = Vec::new();
    r.message(2, hello(2), &mut out).unwrap();
    r.message(1, hello(1), &mut out).unwrap();
    r.message(1, sync(1, None), &mut out).unwrap();
    r.message(1, snapshot(0), &mut out).unwrap();
    out.clear();
    r.message(2, sync(2, None), &mut out).unwrap();
    let own = to(&out, 2).into_iter().find_map(|m| match m {
        CollabMessage::Presence { presence } if presence.site == SiteId(2) => Some(presence.color),
        _ => None,
    });
    assert_eq!(own, Some(Color(0x123456)), "stamped with the set colour");
}

//! Undo history panel in a collab session (CONTRACTS.md §13.8, docs/COLLAB.md): only this
//! site's own steps are listed and jumped; peers' later edits survive a jump.

#[path = "collab_support.rs"]
mod support;

use ether_collab::memory::Hub;
use ether_core::protocol::mixer::MixerCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::undo_history::{HistoryCommand, HistoryList};
use ether_core::protocol::*;
use support::*;

fn add_track(s: &mut Site) -> TrackId {
    let id = s.id();
    s.ok(Command::Track(TrackCommand::Create {
        id,
        kind: TrackKind::Midi,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    id
}

fn set_volume(s: &mut Site, track: TrackId, db: f32) {
    s.ok(Command::Mixer(MixerCommand::SetVolume {
        track,
        volume: Decibels(db),
    }));
}

fn volume(s: &Site, track: TrackId) -> f32 {
    s.project().tracks[&track].mixer.volume.0
}

fn list(s: &mut Site) -> HistoryList {
    match s.ok(Command::History(HistoryCommand::List)) {
        ReplyValue::History { history } => history,
        other => panic!("expected History, got {other:?}"),
    }
}

#[test]
fn only_own_steps_are_listed_and_peers_later_edits_survive_a_jump() {
    let hub = Hub::default();
    let mut sites = session(&hub, 2);
    let [a, b] = sites.as_mut_slice() else {
        unreachable!()
    };
    let t = add_track(a);
    let u = add_track(a);
    settle(&mut [a, b], &hub);
    let a_before = list(a).steps.len();
    let b_before = list(b).steps.len();

    // A: T -6, U -6. B then: T -1 and a track of its own.
    set_volume(a, t, -6.0);
    set_volume(a, u, -6.0);
    settle(&mut [a, b], &hub);
    set_volume(b, t, -1.0);
    let w = add_track(b);
    settle(&mut [a, b], &hub);

    let la = list(a);
    assert_eq!(
        la.steps.len(),
        a_before + 2,
        "B's edits are not in A's list"
    );
    let lb = list(b);
    assert_eq!(
        lb.steps.len(),
        b_before + 2,
        "A's edits are not in B's list"
    );

    // A jumps back before its two volume edits: U is restored, T keeps B's later value,
    // B's track stays.
    let target = la.steps[a_before - 1].id;
    let out = a.send(Command::History(HistoryCommand::JumpTo {
        step: Some(target),
    }));
    assert_eq!(patches(&out).len(), 1, "one patch for the jump");
    assert_eq!(volume(a, u), 0.0);
    assert_eq!(volume(a, t), -1.0, "the peer's later edit survives");
    assert!(a.project().tracks.contains_key(&w));
    let after = list(a);
    assert_eq!(after.current, Some(target));
    assert_eq!(after.steps.iter().filter(|s| s.undone).count(), 2);
    settle(&mut [a, b], &hub);
    assert_eq!(volume(b, u), 0.0, "the jump reached B");
    assert_eq!(volume(b, t), -1.0);
    assert_converged(&[a, b]);
    // B's list is untouched by A's jump.
    assert_eq!(list(b).steps, lb.steps);

    // And forward again.
    a.ok(Command::History(HistoryCommand::JumpTo {
        step: la.current,
    }));
    settle(&mut [a, b], &hub);
    assert_eq!(volume(b, u), -6.0);
    assert_converged(&[a, b]);
}

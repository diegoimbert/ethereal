//! Undo history panel (CONTRACTS.md §13.8): the list reads as the edit timeline, jumping is
//! N undos/redos in one patch batch, checkpoints are named, `HistoryEvent::Changed` is
//! pushed (throttled) once a client listed the history.

mod common;

use common::*;
use ether_controller::ControllerConfig;
use ether_controller::memory::MemoryLibrary;
use ether_core::protocol::mixer::MixerCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::undo_history::{HistoryCommand, HistoryEvent, HistoryList};
use ether_core::protocol::{Command, ErrorCode, Event, ReplyValue};

fn track(h: &mut Harness) -> TrackId {
    let id: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id,
        kind: TrackKind::Midi,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    id
}

fn volume(h: &mut Harness, track: TrackId, db: f32) {
    h.ok(Command::Mixer(MixerCommand::SetVolume {
        track,
        volume: Decibels(db),
    }));
}

fn list(h: &mut Harness) -> HistoryList {
    match h.ok(Command::History(HistoryCommand::List)) {
        ReplyValue::History { history } => history,
        other => panic!("expected History, got {other:?}"),
    }
}

fn jump(h: &mut Harness, step: Option<u32>) -> Vec<ether_core::protocol::ServerMessage> {
    h.send(Command::History(HistoryCommand::JumpTo { step }))
}

fn checkpoint(
    h: &mut Harness,
    step: u32,
    name: Option<&str>,
) -> Vec<ether_core::protocol::ServerMessage> {
    h.send(Command::History(HistoryCommand::SetCheckpoint {
        step,
        name: name.map(str::to_string),
    }))
}

fn changed(out: &[ether_core::protocol::ServerMessage]) -> Vec<HistoryList> {
    events(out)
        .into_iter()
        .filter_map(|e| match e {
            Event::History {
                event: HistoryEvent::Changed { history },
            } => Some(history),
            _ => None,
        })
        .collect()
}

/// A project with four edits, `advance`d 1 s apart: create T, T -3 dB, T -6 dB, T -9 dB.
fn four_steps(h: &mut Harness) -> TrackId {
    let t = track(h);
    for db in [-3.0, -6.0, -9.0] {
        h.advance(1_000);
        volume(h, t, db);
    }
    t
}

#[test]
fn the_list_reads_as_the_edit_timeline() {
    let mut h = Harness::with_project();
    assert_eq!(list(&mut h), HistoryList::default());
    four_steps(&mut h);
    let l = list(&mut h);
    assert_eq!(l.steps.len(), 4);
    assert!(!l.truncated);
    let ids: Vec<u32> = l.steps.iter().map(|s| s.id).collect();
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "monotonic: {ids:?}");
    assert_eq!(l.current, Some(ids[3]));
    let times: Vec<u64> = l.steps.iter().map(|s| s.time_ms).collect();
    assert_eq!(times, vec![T0, T0 + 1_000, T0 + 2_000, T0 + 3_000]);
    assert!(l.steps.iter().all(|s| !s.undone && s.checkpoint.is_none()));
    // Labelled like the Undo menu.
    assert_eq!(l.steps[3].label, l.steps[2].label);
    assert!(!l.steps[0].label.is_empty());

    // Undo twice: the undone steps follow in redo order, ids and times unchanged.
    h.ok(Command::Edit(EditCommand::Undo));
    h.ok(Command::Edit(EditCommand::Undo));
    let after = list(&mut h);
    assert_eq!(
        after
            .steps
            .iter()
            .map(|s| (s.id, s.time_ms, s.undone))
            .collect::<Vec<_>>(),
        vec![
            (ids[0], times[0], false),
            (ids[1], times[1], false),
            (ids[2], times[2], true),
            (ids[3], times[3], true),
        ]
    );
    assert_eq!(after.current, Some(ids[1]));
}

#[test]
fn a_gesture_is_one_step_with_its_first_commit_time() {
    let mut h = Harness::with_project();
    let t = track(&mut h);
    let g = Some(GestureId(5));
    for db in [-1.0, -2.0, -3.0] {
        h.advance(50);
        h.send_with(
            Command::Mixer(MixerCommand::SetVolume {
                track: t,
                volume: Decibels(db),
            }),
            g,
        );
    }
    let l = list(&mut h);
    assert_eq!(l.steps.len(), 2);
    assert_eq!(l.steps[1].time_ms, T0 + 50);
}

#[test]
fn jumping_is_equivalent_to_n_undos_and_redos_in_one_patch() {
    let mut h = Harness::with_project();
    let mut twin = Harness::with_project();
    four_steps(&mut h);
    four_steps(&mut twin);
    let ids: Vec<u32> = list(&mut h).steps.iter().map(|s| s.id).collect();

    // Back three steps.
    let out = jump(&mut h, Some(ids[0]));
    for _ in 0..3 {
        twin.ok(Command::Edit(EditCommand::Undo));
    }
    assert_eq!(h.project().tracks, twin.project().tracks);
    let p = patches(&out);
    assert_eq!(p.len(), 1, "one patch for the whole jump");
    assert!(p[0].history.can_undo && p[0].history.can_redo);
    let ReplyValue::History { history } = ok(&out) else {
        panic!("expected History")
    };
    assert_eq!(history.current, Some(ids[0]));
    assert_eq!(history.steps.iter().filter(|s| s.undone).count(), 3);

    // Forward two.
    let out = jump(&mut h, Some(ids[2]));
    twin.ok(Command::Edit(EditCommand::Redo));
    twin.ok(Command::Edit(EditCommand::Redo));
    assert_eq!(h.project().tracks, twin.project().tracks);
    assert_eq!(patches(&out).len(), 1);
    assert_eq!(list(&mut h).current, Some(ids[2]));

    // Before everything: the track is gone.
    let out = jump(&mut h, None);
    for _ in 0..3 {
        twin.ok(Command::Edit(EditCommand::Undo));
    }
    assert_eq!(h.project().tracks, twin.project().tracks);
    assert!(
        h.project()
            .tracks
            .values()
            .all(|t| t.kind != TrackKind::Midi)
    );
    assert_eq!(patches(&out).len(), 1);
    assert!(!patches(&out)[0].history.can_undo);
    assert_eq!(list(&mut h).current, None);

    // All the way forward again; the steps are still the same ones.
    jump(&mut h, Some(ids[3]));
    for _ in 0..4 {
        twin.ok(Command::Edit(EditCommand::Redo));
    }
    assert_eq!(h.project().tracks, twin.project().tracks);
    let l = list(&mut h);
    assert_eq!(l.steps.iter().map(|s| s.id).collect::<Vec<_>>(), ids);
    assert!(l.steps.iter().all(|s| !s.undone));

    // Jumping to where we are changes nothing.
    let out = jump(&mut h, Some(ids[3]));
    assert!(patches(&out).is_empty());

    // A jump is not itself a step; the document is dirty like after an undo.
    assert_eq!(list(&mut h).steps.len(), 4);
    assert!(h.ctl.is_dirty());
}

#[test]
fn a_new_edit_after_a_jump_drops_the_undone_steps() {
    let mut h = Harness::with_project();
    let t = four_steps(&mut h);
    let ids: Vec<u32> = list(&mut h).steps.iter().map(|s| s.id).collect();
    jump(&mut h, Some(ids[1]));
    volume(&mut h, t, -12.0);
    let l = list(&mut h);
    assert_eq!(l.steps.len(), 3);
    assert!(l.steps[2].id > ids[3], "ids are never reused");
    let out = jump(&mut h, Some(ids[3]));
    assert_eq!(err(&out).code, ErrorCode::NotFound);
}

#[test]
fn dropped_and_unknown_steps_are_not_found() {
    let config = ControllerConfig {
        history_depth: 2,
        ..ControllerConfig::default()
    };
    let mut h = Harness::with(FakeBridge::default(), MemoryLibrary::new(), config);
    h.create_project("Short");
    four_steps(&mut h);
    let l = list(&mut h);
    assert!(l.truncated);
    assert_eq!(l.steps.len(), 2);
    let oldest = l.steps[0].id;
    let before = h.project().clone();
    for step in [oldest - 1, 999] {
        let out = jump(&mut h, Some(step));
        assert_eq!(err(&out).code, ErrorCode::NotFound);
        assert!(patches(&out).is_empty());
        assert_eq!(h.project(), &before);
    }
    // `None` = before the oldest kept step.
    jump(&mut h, None);
    assert_eq!(list(&mut h).current, None);
    let vol = h
        .project()
        .tracks
        .values()
        .find(|t| t.kind == TrackKind::Midi)
        .unwrap();
    assert_eq!(vol.mixer.volume.0, -3.0);
}

#[test]
fn checkpoints_are_named_for_the_session() {
    let mut h = Harness::with_project();
    four_steps(&mut h);
    let ids: Vec<u32> = list(&mut h).steps.iter().map(|s| s.id).collect();
    let before = h.project().clone();
    let dirty = h.ctl.is_dirty();
    let out = checkpoint(&mut h, ids[1], Some("  Before mix "));
    assert!(matches!(ok(&out), ReplyValue::Unit));
    assert!(patches(&out).is_empty(), "not undoable, no document change");
    assert_eq!(h.project(), &before);
    assert_eq!(h.ctl.is_dirty(), dirty);
    let l = list(&mut h);
    assert_eq!(l.steps[1].checkpoint.as_deref(), Some("Before mix"));
    assert_eq!(l.steps.len(), 4, "naming is not a step");

    // Kept across jumps (undone steps keep their name).
    jump(&mut h, Some(ids[0]));
    assert_eq!(
        list(&mut h).steps[1].checkpoint.as_deref(),
        Some("Before mix")
    );
    jump(&mut h, Some(ids[3]));
    assert_eq!(
        list(&mut h).steps[1].checkpoint.as_deref(),
        Some("Before mix")
    );

    // Cleared by `None` or a blank name.
    checkpoint(&mut h, ids[2], Some("x"));
    checkpoint(&mut h, ids[1], None);
    checkpoint(&mut h, ids[2], Some("   "));
    assert!(list(&mut h).steps.iter().all(|s| s.checkpoint.is_none()));

    assert_eq!(
        err(&checkpoint(&mut h, 999, Some("x"))).code,
        ErrorCode::NotFound
    );
    let long = "a".repeat(500);
    assert_eq!(
        err(&checkpoint(&mut h, ids[0], Some(&long))).code,
        ErrorCode::InvalidArgument
    );

    // A new project starts a new history.
    checkpoint(&mut h, ids[0], Some("Old"));
    h.create_project("Other");
    assert_eq!(list(&mut h), HistoryList::default());
}

#[test]
fn without_a_project() {
    let mut h = Harness::new();
    assert_eq!(list(&mut h), HistoryList::default());
    assert_eq!(err(&jump(&mut h, None)).code, ErrorCode::InvalidState);
}

#[test]
fn changes_are_pushed_throttled_once_listed() {
    let mut h = Harness::with_project();
    let t = track(&mut h);
    // Nobody listed the history: nothing is pushed.
    h.advance(500);
    assert!(changed(&h.tick()).is_empty());

    list(&mut h);
    h.advance(500);
    assert!(changed(&h.tick()).is_empty(), "unchanged: nothing pushed");

    // Two edits within 100 ms of each other: one event after the throttle.
    volume(&mut h, t, -1.0);
    let first = changed(&h.tick());
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].steps.len(), 2);
    h.advance(30);
    volume(&mut h, t, -2.0);
    assert!(changed(&h.tick()).is_empty(), "throttled");
    h.advance(69);
    assert!(changed(&h.tick()).is_empty(), "throttled");
    h.advance(1);
    let second = changed(&h.tick());
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].steps.len(), 3);
    assert_eq!(second[0], list(&mut h));

    // Checkpoints and undo are pushed too.
    h.advance(200);
    checkpoint(&mut h, second[0].steps[0].id, Some("Start"));
    let c = changed(&h.tick());
    assert_eq!(c[0].steps[0].checkpoint.as_deref(), Some("Start"));
    h.advance(200);
    h.ok(Command::Edit(EditCommand::Undo));
    let c = changed(&h.tick());
    assert!(c[0].steps[2].undone);
}

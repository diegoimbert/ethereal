//! No edit leaves a clip shorter than a tick behind (`doc::drop_slivers`): the leftovers of
//! overlap trims, splits and time edits whose edges missed each other by a rounding error
//! are deleted in the same undo step.

use crate::common;

use common::*;
use ether_controller::doc::MIN_CLIP_SLIVER;
use ether_core::protocol::Command;
use ether_core::protocol::clips::{ClipCommand, ClipMove};
use ether_core::protocol::model::*;
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::time_edit::{TimeEditCommand, TimeSelection};
use ether_core::protocol::tracks::TrackCommand;

fn setup() -> (Harness, TrackId) {
    let mut h = Harness::new();
    h.create_project("Slivers");
    let track: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id: track,
        kind: TrackKind::Midi,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    (h, track)
}

fn midi(h: &mut Harness, track: TrackId, start: f64, length: f64) -> ClipId {
    let id: ClipId = h.id();
    h.ok(Command::Clip(ClipCommand::CreateMidi {
        id,
        track,
        start: Beats(start),
        length: Beats(length),
        name: None,
    }));
    id
}

fn lengths(h: &Harness, track: TrackId) -> Vec<f64> {
    let mut v: Vec<(f64, f64)> = h
        .project()
        .clips
        .values()
        .filter(|c| c.track == track)
        .map(|c| (c.start.0, c.length.0))
        .collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    v.into_iter().map(|(_, l)| l).collect()
}

#[test]
fn a_move_that_almost_covers_a_clip_deletes_it_instead_of_leaving_a_sliver() {
    let (mut h, track) = setup();
    let a = midi(&mut h, track, 0.0, 4.0);
    let b = midi(&mut h, track, 8.0, 4.0);
    // B dropped a rounding error after A's start: A's leftover would be 2e-5 beats long.
    h.ok(Command::Clip(ClipCommand::Move {
        moves: vec![ClipMove {
            id: b,
            track,
            start: Beats(2e-5),
        }],
    }));
    assert!(
        !h.project().clips.contains_key(&a),
        "{:?}",
        lengths(&h, track)
    );
    assert_eq!(lengths(&h, track), vec![4.0]);
    // One undo step puts both back.
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(lengths(&h, track), vec![4.0, 4.0]);
}

#[test]
fn a_time_edit_split_next_to_a_clip_edge_leaves_no_sliver() {
    let (mut h, track) = setup();
    midi(&mut h, track, 0.0, 4.0);
    // Two cuts: a real one and one a hair (above the split tolerance) before the clip end.
    for at in [2.0, 4.0 - 5e-5] {
        let seed: ClipId = h.id();
        h.ok(Command::TimeEdit(TimeEditCommand::Split {
            tracks: vec![track],
            at: Beats(at),
            seed,
        }));
    }
    let l = lengths(&h, track);
    assert!(l.iter().all(|&x| x >= MIN_CLIP_SLIVER), "{l:?}");
    assert_eq!(l.len(), 2, "{l:?}");
    // Delete time that stops a hair before a clip end.
    let selection = TimeSelection {
        start: Beats(0.5),
        end: Beats(2.0 - 3e-5),
        tracks: vec![track],
        global: false,
    };
    let seed: ClipId = h.id();
    h.ok(Command::TimeEdit(TimeEditCommand::DeleteTime {
        selection,
        seed,
    }));
    let l = lengths(&h, track);
    assert!(l.iter().all(|&x| x >= MIN_CLIP_SLIVER), "{l:?}");
}

#[test]
fn short_clips_an_edit_does_not_touch_stay() {
    let (mut h, track) = setup();
    let short = midi(&mut h, track, 0.0, 0.25);
    midi(&mut h, track, 4.0, 4.0);
    h.ok(Command::Clip(ClipCommand::Rename {
        id: short,
        name: "s".into(),
    }));
    assert_eq!(lengths(&h, track), vec![0.25, 4.0]);
}

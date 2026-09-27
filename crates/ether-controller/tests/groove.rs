//! Groove (roadmap v2, `groove` node): quantize strength + swing, humanize, project swing.

mod common;

use common::*;
use ether_core::graph::{ClipContentDesc, NoteDesc};
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::groove::GrooveCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteSpec};
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::{Command, ErrorCode};

fn midi_clip(h: &mut Harness, start: f64, length: f64) -> ClipId {
    let track: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id: track,
        kind: TrackKind::Midi,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    let id = h.id();
    h.ok(Command::Clip(ClipCommand::CreateMidi {
        id,
        track,
        start: Beats(start),
        length: Beats(length),
        name: None,
    }));
    id
}

/// Adds one note per `(start, velocity)`, duration 0.25, in id order.
fn add_notes(h: &mut Harness, clip: ClipId, notes: &[(f64, f32)]) -> Vec<NoteId> {
    let ids: Vec<NoteId> = notes.iter().map(|_| h.id()).collect();
    h.ok(Command::Note(NoteCommand::Add {
        clip,
        notes: ids
            .iter()
            .zip(notes)
            .map(|(id, (start, velocity))| NoteSpec {
                id: *id,
                pitch: 60,
                velocity: *velocity,
                start: Beats(*start),
                duration: Beats(0.25),
            })
            .collect(),
    }));
    ids
}

fn start(h: &Harness, n: NoteId) -> f64 {
    h.project().notes[&n].start.0
}

fn quantize(clip: ClipId, grid: f64, strength: f32, swing: f32) -> Command {
    Command::Note(NoteCommand::Quantize {
        clip,
        notes: None,
        grid: Beats(grid),
        strength,
        ends: false,
        swing,
    })
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn undo(h: &mut Harness) {
    h.ok(Command::Edit(EditCommand::Undo));
}

fn compiled_notes(h: &mut Harness, clip: ClipId) -> Vec<NoteDesc> {
    h.tick();
    let g = h.ctl.bridge.last_graph();
    let c = g
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .find(|c| c.id == clip)
        .expect("clip compiled");
    match &c.content {
        ClipContentDesc::Midi { notes } => notes.clone(),
        other => panic!("not a MIDI clip: {other:?}"),
    }
}

#[test]
fn quantize_strength_half_moves_halfway() {
    let mut h = Harness::with_project();
    let c = midi_clip(&mut h, 0.0, 8.0);
    let n = add_notes(&mut h, c, &[(0.1, 0.8), (1.8, 0.8), (3.0, 0.8)]);
    h.ok(quantize(c, 1.0, 0.5, 0.0));
    assert!(close(start(&h, n[0]), 0.05));
    assert!(close(start(&h, n[1]), 1.9));
    assert!(close(start(&h, n[2]), 3.0));
    // Durations are kept when `ends` is false.
    assert!(close(h.project().notes[&n[1]].duration.0, 0.25));
    // One undo step restores everything.
    undo(&mut h);
    assert!(close(start(&h, n[0]), 0.1));
    assert!(close(start(&h, n[1]), 1.8));
}

#[test]
fn quantize_swing_delays_odd_grid_positions() {
    let mut h = Harness::with_project();
    let c = midi_clip(&mut h, 0.0, 8.0);
    // Eighth-note grid: 0.5 and 1.5 are off-beats (odd positions), 0 and 1 are not.
    let n = add_notes(
        &mut h,
        c,
        &[(0.02, 0.8), (0.48, 0.8), (1.0, 0.8), (1.53, 0.8)],
    );
    h.ok(quantize(c, 0.5, 1.0, 1.0));
    let d = 0.5 / 3.0;
    assert!(close(start(&h, n[0]), 0.0));
    assert!(close(start(&h, n[1]), 0.5 + d));
    assert!(close(start(&h, n[2]), 1.0));
    assert!(close(start(&h, n[3]), 1.5 + d));
    undo(&mut h);

    // Half swing at half strength: halfway to the half-swung target.
    h.ok(quantize(c, 0.5, 0.5, 0.5));
    let target = 0.5 + 0.5 * 0.5 / 3.0;
    assert!(close(start(&h, n[1]), 0.48 + (target - 0.48) * 0.5));
    assert!(close(start(&h, n[0]), 0.01));
}

#[test]
fn quantize_swing_with_ends_swings_note_ends() {
    let mut h = Harness::with_project();
    let c = midi_clip(&mut h, 0.0, 8.0);
    let n = add_notes(&mut h, c, &[(0.0, 0.8)]);
    // End at 0.25 snaps to 0.5 (odd) → delayed by 0.5 / 3.
    h.ok(Command::Note(NoteCommand::Quantize {
        clip: c,
        notes: Some(n.clone()),
        grid: Beats(0.5),
        strength: 1.0,
        ends: true,
        swing: 1.0,
    }));
    assert!(close(start(&h, n[0]), 0.0));
    assert!(close(h.project().notes[&n[0]].duration.0, 0.5 + 0.5 / 3.0));
}

#[test]
fn humanize_is_deterministic_from_seed() {
    let mut h = Harness::with_project();
    let c = midi_clip(&mut h, 0.0, 8.0);
    let n = add_notes(&mut h, c, &[(1.0, 0.5), (2.0, 0.5)]);
    let cmd = Command::Groove(GrooveCommand::Humanize {
        clip: c,
        notes: None,
        timing: Beats(0.1),
        velocity: 0.2,
        seed: 7,
    });
    h.ok(cmd.clone());
    // Same values as `mulberry32(7)` in the MockTransport (notes in id order).
    let expected = [
        (0.9023409506306053, 0.3247833030298352),
        (2.095381526555866, 0.5796114822849632),
    ];
    for (id, (s, v)) in n.iter().zip(expected) {
        let note = &h.project().notes[id];
        assert!(close(note.start.0, s), "{} vs {s}", note.start.0);
        assert!((f64::from(note.velocity) - v).abs() < 1e-6);
    }
    // One undo step; re-applying with the same seed gives the same result.
    undo(&mut h);
    assert!(close(start(&h, n[0]), 1.0));
    assert_eq!(h.project().notes[&n[0]].velocity, 0.5);
    h.ok(cmd);
    assert!(close(start(&h, n[0]), expected[0].0));
    assert!(close(start(&h, n[1]), expected[1].0));
    let humanized: Vec<Note> = n.iter().map(|id| h.project().notes[id].clone()).collect();

    // Undo → Redo restores exactly the humanized notes (no re-randomization).
    undo(&mut h);
    assert!(close(start(&h, n[1]), 2.0));
    h.ok(Command::Edit(EditCommand::Redo));
    for (id, want) in n.iter().zip(&humanized) {
        assert_eq!(&h.project().notes[id], want);
    }
}

#[test]
fn humanize_subset_clamps_and_validates() {
    let mut h = Harness::with_project();
    let c = midi_clip(&mut h, 0.0, 8.0);
    let n = add_notes(&mut h, c, &[(0.0, 1.0), (2.0, 0.5)]);
    h.ok(Command::Groove(GrooveCommand::Humanize {
        clip: c,
        notes: Some(vec![n[0]]),
        timing: Beats(4.0),
        velocity: 1.0,
        seed: 1,
    }));
    let a = &h.project().notes[&n[0]];
    assert!(a.start.0 >= 0.0 && (0.0..=1.0).contains(&a.velocity));
    // Unselected note untouched.
    assert!(close(start(&h, n[1]), 2.0));
    assert_eq!(h.project().notes[&n[1]].velocity, 0.5);

    let bad = h.send(Command::Groove(GrooveCommand::Humanize {
        clip: c,
        notes: None,
        timing: Beats(f64::NAN),
        velocity: 0.1,
        seed: 1,
    }));
    assert_eq!(err(&bad).code, ErrorCode::InvalidArgument);
    let unknown: ClipId = h.id();
    let missing = h.send(Command::Groove(GrooveCommand::Humanize {
        clip: unknown,
        notes: None,
        timing: Beats(0.1),
        velocity: 0.1,
        seed: 1,
    }));
    assert_eq!(err(&missing).code, ErrorCode::NotFound);
}

#[test]
fn set_swing_updates_settings_and_is_one_undo_step() {
    let mut h = Harness::with_project();
    h.ok(Command::Groove(GrooveCommand::SetSwing {
        amount: 1.5,
        grid: Beats(0.5),
    }));
    assert_eq!(h.project().settings.swing, 1.0);
    assert_eq!(h.project().settings.swing_grid, Beats(0.5));
    undo(&mut h);
    assert_eq!(h.project().settings.swing, 0.0);
    assert_eq!(h.project().settings.swing_grid, Beats(0.25));
    let bad = h.send(Command::Groove(GrooveCommand::SetSwing {
        amount: 0.5,
        grid: Beats(0.0),
    }));
    assert_eq!(err(&bad).code, ErrorCode::InvalidArgument);
    assert!(patches(&bad).is_empty());
}

#[test]
fn project_swing_is_applied_at_compile_time_only() {
    let mut h = Harness::with_project();
    let c = midi_clip(&mut h, 0.0, 8.0);
    let n = add_notes(
        &mut h,
        c,
        &[(0.0, 0.8), (0.25, 0.8), (0.5, 0.8), (0.75, 0.8), (0.6, 0.8)],
    );
    h.ok(Command::Groove(GrooveCommand::SetSwing {
        amount: 0.6,
        grid: Beats(0.25),
    }));
    let d = f64::from(0.6f32) * 0.25 / 3.0;
    let starts: Vec<f64> = compiled_notes(&mut h, c).iter().map(|n| n.start).collect();
    // 0.25 and 0.75 are odd sixteenths; 0.6 is off the grid and stays; order is by start.
    let expected = [0.0, 0.25 + d, 0.5, 0.6, 0.75 + d];
    assert_eq!(starts.len(), expected.len());
    for (s, e) in starts.iter().zip(expected) {
        assert!(close(*s, e), "{starts:?}");
    }
    // Non-destructive: the document is unchanged.
    assert!(close(start(&h, n[1]), 0.25));

    // The grid follows the content offset: with offset 0.25 the note at content 0.25 is
    // on the clip's first (even) grid line.
    h.ok(Command::Clip(ClipCommand::SetBounds {
        id: c,
        start: Beats(0.0),
        length: Beats(4.0),
        offset: Beats(0.25),
    }));
    let notes = compiled_notes(&mut h, c);
    let at = |content: f64| notes.iter().any(|n| close(n.start, content));
    assert!(at(0.25) && at(0.5 + d) && at(0.0 + d));

    // Swing 0 = straight.
    h.ok(Command::Groove(GrooveCommand::SetSwing {
        amount: 0.0,
        grid: Beats(0.25),
    }));
    let starts: Vec<f64> = compiled_notes(&mut h, c).iter().map(|n| n.start).collect();
    assert!(close(starts[1], 0.25));
}

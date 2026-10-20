//! MIDI reverse (`NoteCommand::Reverse`): mirrors a clip's notes, lanes, per-note
//! expressions and envelopes in time (whole clip) or a selection of notes; one undo step;
//! a double reverse restores the input.

use crate::common;

use common::*;
use ether_core::protocol::automation::{AutomationCommand, PointSpec};
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::expression::ExpressionCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteSpec};
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::{Command, ErrorCode};

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

fn clip(h: &mut Harness, track: TrackId, length: f64) -> ClipId {
    let id: ClipId = h.id();
    h.ok(Command::Clip(ClipCommand::CreateMidi {
        id,
        track,
        start: Beats(0.0),
        length: Beats(length),
        name: None,
    }));
    id
}

fn note(h: &mut Harness, clip: ClipId, pitch: u8, start: f64, duration: f64) -> NoteId {
    let id: NoteId = h.id();
    h.ok(Command::Note(NoteCommand::Add {
        clip,
        notes: vec![NoteSpec {
            id,
            pitch,
            velocity: 0.8,
            start: Beats(start),
            duration: Beats(duration),
        }],
    }));
    id
}

fn pt(time: f64, value: f32) -> ExpressionPoint {
    ExpressionPoint {
        time: Beats(time),
        value,
        curve: CurveShape::Linear,
    }
}

fn reverse(h: &mut Harness, clip: ClipId, notes: Option<Vec<NoteId>>) {
    h.ok(Command::Note(NoteCommand::Reverse { clip, notes }));
}

fn undo(h: &mut Harness) {
    h.ok(Command::Edit(EditCommand::Undo));
}

/// `(start, duration)` of a note.
fn span(h: &Harness, id: NoteId) -> (f64, f64) {
    let n = &h.project().notes[&id];
    (n.start.0, n.duration.0)
}

fn near(a: (f64, f64), b: (f64, f64)) {
    assert!(
        (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9,
        "{a:?} != {b:?}"
    );
}

#[test]
fn reverses_a_clip_and_keeps_note_properties() {
    let mut h = Harness::with_project();
    let t = track(&mut h);
    let c = clip(&mut h, t, 4.0);
    let a = note(&mut h, c, 60, 0.0, 1.0);
    let b = note(&mut h, c, 64, 1.0, 0.5);
    let d = note(&mut h, c, 67, 3.0, 1.0);
    h.ok(Command::Note(NoteCommand::Edit {
        edits: vec![ether_core::protocol::notes::NoteEdit {
            id: b,
            pitch: None,
            velocity: None,
            start: None,
            duration: None,
            muted: Some(true),
        }],
    }));
    let before = h.project().clone();
    reverse(&mut h, c, None);
    near(span(&h, a), (3.0, 1.0));
    near(span(&h, b), (2.5, 0.5));
    near(span(&h, d), (0.0, 1.0));
    let nb = &h.project().notes[&b];
    assert!(nb.muted);
    assert_eq!(nb.pitch, 64);
    assert_eq!(nb.velocity, before.notes[&b].velocity);
    assert_eq!(nb.release_velocity, before.notes[&b].release_velocity);
    // One undo step.
    undo(&mut h);
    assert_eq!(h.project(), &before);
}

#[test]
fn double_reverse_restores_the_clip() {
    let mut h = Harness::with_project();
    let t = track(&mut h);
    let c = clip(&mut h, t, 8.0);
    let n = note(&mut h, c, 60, 0.25, 0.75);
    note(&mut h, c, 62, 1.5, 2.0);
    let lane: ExpressionLaneId = h.id();
    h.ok(Command::Expression(ExpressionCommand::CreateLane {
        id: lane,
        clip: c,
        kind: ExpressionKind::Cc { controller: 1 },
    }));
    h.ok(Command::Expression(ExpressionCommand::SetPoints {
        lane,
        points: vec![pt(1.0, 0.1), pt(3.0, 0.9), pt(6.0, 0.4)],
    }));
    let e: NoteExpressionId = h.id();
    h.ok(Command::Expression(ExpressionCommand::SetNoteExpression {
        id: e,
        note: n,
        kind: NoteExpressionKind::Pressure,
        points: vec![pt(0.0, 0.2), pt(0.5, 0.8)],
    }));
    let before = h.project().clone();
    reverse(&mut h, c, None);
    assert_ne!(h.project().notes, before.notes);
    reverse(&mut h, c, None);
    let after = h.project();
    for (id, n0) in &before.notes {
        near(
            (after.notes[id].start.0, after.notes[id].duration.0),
            (n0.start.0, n0.duration.0),
        );
    }
    assert_eq!(after.expression_lanes, before.expression_lanes);
    assert_eq!(after.note_expressions, before.note_expressions);
}

#[test]
fn looped_clip_mirrors_inside_the_loop_region() {
    let mut h = Harness::with_project();
    let t = track(&mut h);
    let c = clip(&mut h, t, 8.0);
    // Loop region [1, 3): the note at 1..1.5 goes to 2.5..3; the note past the loop and
    // the one crossing its end stay.
    let inside = note(&mut h, c, 60, 1.0, 0.5);
    let past = note(&mut h, c, 62, 5.0, 1.0);
    let crossing = note(&mut h, c, 64, 2.5, 1.0);
    h.ok(Command::Clip(ClipCommand::SetLoop {
        id: c,
        looping: ClipLoop {
            enabled: true,
            start: Beats(1.0),
            end: Beats(3.0),
        },
    }));
    reverse(&mut h, c, None);
    near(span(&h, inside), (2.5, 0.5));
    near(span(&h, past), (5.0, 1.0));
    near(span(&h, crossing), (2.5, 1.0));
}

#[test]
fn trimmed_clip_mirrors_the_content_it_plays() {
    let mut h = Harness::with_project();
    let t = track(&mut h);
    let c = clip(&mut h, t, 4.0);
    let n = note(&mut h, c, 60, 2.0, 1.0);
    // Plays content [1, 5).
    h.ok(Command::Clip(ClipCommand::SetBounds {
        id: c,
        start: Beats(0.0),
        length: Beats(4.0),
        offset: Beats(1.0),
    }));
    reverse(&mut h, c, None);
    near(span(&h, n), (3.0, 1.0));
    let m = note(&mut h, c, 62, 1.0, 1.0);
    reverse(&mut h, c, None);
    near(span(&h, m), (4.0, 1.0));
}

#[test]
fn mirrors_cc_lanes_and_per_note_expressions() {
    let mut h = Harness::with_project();
    let t = track(&mut h);
    let c = clip(&mut h, t, 4.0);
    let n = note(&mut h, c, 60, 0.0, 2.0);
    let lane: ExpressionLaneId = h.id();
    h.ok(Command::Expression(ExpressionCommand::CreateLane {
        id: lane,
        clip: c,
        kind: ExpressionKind::PitchBend,
    }));
    h.ok(Command::Expression(ExpressionCommand::SetPoints {
        lane,
        points: vec![pt(0.0, 0.0), pt(1.0, 0.5), pt(4.0, -1.0)],
    }));
    let e: NoteExpressionId = h.id();
    h.ok(Command::Expression(ExpressionCommand::SetNoteExpression {
        id: e,
        note: n,
        kind: NoteExpressionKind::Pressure,
        points: vec![pt(0.0, 0.0), pt(0.5, 0.25), pt(2.0, 1.0)],
    }));
    reverse(&mut h, c, None);
    let got: Vec<(f64, f32)> = h.project().expression_lanes[&lane]
        .points
        .iter()
        .map(|p| (p.time.0, p.value))
        .collect();
    assert_eq!(got, vec![(0.0, -1.0), (3.0, 0.5), (4.0, 0.0)]);
    let got: Vec<(f64, f32)> = h.project().note_expressions[&e]
        .points
        .iter()
        .map(|p| (p.time.0, p.value))
        .collect();
    assert_eq!(got, vec![(0.0, 1.0), (1.5, 0.25), (2.0, 0.0)]);
}

#[test]
fn mirrors_clip_envelopes() {
    let mut h = Harness::with_project();
    let t = track(&mut h);
    let c = clip(&mut h, t, 4.0);
    note(&mut h, c, 60, 0.0, 1.0);
    let lane: AutomationLaneId = h.id();
    h.ok(Command::Automation(AutomationCommand::CreateLane {
        id: lane,
        owner: AutomationOwner::Clip { clip: c },
        target: AutomationTarget::TrackVolume { track: t },
    }));
    let ids: Vec<AutomationPointId> = vec![h.id(), h.id()];
    h.ok(Command::Automation(AutomationCommand::AddPoints {
        lane,
        points: vec![
            PointSpec {
                id: ids[0],
                time: Beats(0.0),
                value: 0.2,
                curve: CurveShape::Linear,
            },
            PointSpec {
                id: ids[1],
                time: Beats(1.0),
                value: 0.8,
                curve: CurveShape::Linear,
            },
        ],
    }));
    let before = h.project().clone();
    reverse(&mut h, c, None);
    let pts: Vec<(f64, f64)> = h
        .project()
        .points_of(lane)
        .iter()
        .map(|p| (p.time.0, p.value))
        .collect();
    // Mirrored over the clip's 4 beats: the ramp 0.2 -> 0.8 on [0, 1] becomes 0.8 -> 0.2 on
    // [3, 4], holding 0.8 before.
    assert_eq!(pts.first().map(|p| p.1), Some(0.8));
    assert_eq!(pts.last(), Some(&(4.0, 0.2)));
    undo(&mut h);
    assert_eq!(h.project(), &before);
    reverse(&mut h, c, None);
    reverse(&mut h, c, None);
    let pts: Vec<(f64, f64)> = h
        .project()
        .points_of(lane)
        .iter()
        .map(|p| (p.time.0, p.value))
        .collect();
    assert_eq!(pts, vec![(0.0, 0.2), (1.0, 0.8)]);
}

#[test]
fn selection_reverse_only_touches_the_selected_span() {
    let mut h = Harness::with_project();
    let t = track(&mut h);
    let c = clip(&mut h, t, 8.0);
    let a = note(&mut h, c, 60, 1.0, 1.0);
    let b = note(&mut h, c, 62, 2.0, 0.5);
    let outside = note(&mut h, c, 64, 4.0, 1.0);
    let between = note(&mut h, c, 65, 2.5, 0.5);
    let lane: ExpressionLaneId = h.id();
    h.ok(Command::Expression(ExpressionCommand::CreateLane {
        id: lane,
        clip: c,
        kind: ExpressionKind::Cc { controller: 1 },
    }));
    h.ok(Command::Expression(ExpressionCommand::SetPoints {
        lane,
        points: vec![pt(0.0, 0.0), pt(2.0, 1.0)],
    }));
    let lanes = h.project().expression_lanes.clone();
    // Selected span 1.0 .. 2.5.
    reverse(&mut h, c, Some(vec![a, b]));
    near(span(&h, a), (1.5, 1.0));
    near(span(&h, b), (1.0, 0.5));
    // Unselected notes stay, even inside the span.
    near(span(&h, outside), (4.0, 1.0));
    near(span(&h, between), (2.5, 0.5));
    assert_eq!(h.project().expression_lanes, lanes);
    // Empty selection: no-op, no undo step spent.
    let before = h.project().clone();
    reverse(&mut h, c, Some(vec![]));
    assert_eq!(h.project(), &before);
    // Twice = identity.
    reverse(&mut h, c, Some(vec![a, b]));
    near(span(&h, a), (1.0, 1.0));
    near(span(&h, b), (2.0, 0.5));
}

#[test]
fn rejects_an_unknown_clip() {
    let mut h = Harness::with_project();
    let clip: ClipId = h.id();
    let out = h.send(Command::Note(NoteCommand::Reverse { clip, notes: None }));
    assert_eq!(err(&out).code, ErrorCode::NotFound);
}

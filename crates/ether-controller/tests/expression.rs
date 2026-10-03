//! MIDI expression (`midi-expression`, CONTRACTS.md §13.2): lane and note-expression
//! commands (validation, idempotency, one undo step each, gestures), the compile hook, the
//! copies (note duplicate, paste, consolidate, comp flatten) and an offline render through
//! the Poly Synth identical for block sizes 64 and 512.

mod common;

use common::*;
use ether_core::config::EngineConfig;
use ether_core::expression::TrackExpressionDesc;
use ether_core::offline::OfflineRenderer;
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::devices::{DeviceCommand, DeviceSpec};
use ether_core::protocol::expression::ExpressionCommand;
use ether_core::protocol::freeze::FreezeCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteCopy, NoteSpec};
use ether_core::protocol::groove::GrooveCommand;
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::takes::TakeCommand;
use ether_core::protocol::time_edit::{TimeEditCommand, TimeSelection};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::{Command, ErrorCode};
use ether_core::{Node, RenderGraphDesc};

fn track(h: &mut Harness, kind: TrackKind) -> TrackId {
    let id: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id,
        kind,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    id
}

fn clip(h: &mut Harness, track: TrackId, start: f64, length: f64) -> ClipId {
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

fn x(c: ExpressionCommand) -> Command {
    Command::Expression(c)
}

fn new_lane(h: &mut Harness, clip: ClipId, kind: ExpressionKind) -> ExpressionLaneId {
    let id: ExpressionLaneId = h.id();
    h.ok(x(ExpressionCommand::CreateLane { id, clip, kind }));
    id
}

fn undo(h: &mut Harness) {
    h.ok(Command::Edit(EditCommand::Undo));
}

fn points(h: &Harness, lane: ExpressionLaneId) -> Vec<(f64, f32)> {
    h.project().expression_lanes[&lane]
        .points
        .iter()
        .map(|p| (p.time.0, p.value))
        .collect()
}

fn expr_of(h: &Harness, track: TrackId) -> TrackExpressionDesc {
    h.ctl
        .bridge
        .last_graph()
        .tracks
        .iter()
        .find(|t| t.id == track)
        .unwrap()
        .expression
        .clone()
}

#[test]
fn lanes_are_created_once_per_kind_on_midi_clips_only() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
    let c = clip(&mut h, t, 0.0, 4.0);
    let bend = new_lane(&mut h, c, ExpressionKind::PitchBend);
    // Retried create (same id) and another id for the same kind: no change.
    let before = h.project().clone();
    h.ok(x(ExpressionCommand::CreateLane {
        id: bend,
        clip: c,
        kind: ExpressionKind::PitchBend,
    }));
    let other: ExpressionLaneId = h.id();
    let out = h.send(x(ExpressionCommand::CreateLane {
        id: other,
        clip: c,
        kind: ExpressionKind::PitchBend,
    }));
    ok(&out);
    assert!(patches(&out).is_empty());
    assert_eq!(h.project(), &before);
    // CC range and clip kind.
    let id: ExpressionLaneId = h.id();
    let out = h.send(x(ExpressionCommand::CreateLane {
        id,
        clip: c,
        kind: ExpressionKind::Cc { controller: 120 },
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    let id: ExpressionLaneId = h.id();
    let missing: ClipId = h.id();
    let out = h.send(x(ExpressionCommand::CreateLane {
        id,
        clip: missing,
        kind: ExpressionKind::ChannelPressure,
    }));
    assert_eq!(err(&out).code, ErrorCode::NotFound);
    // Remove, one undo step brings it back.
    h.ok(x(ExpressionCommand::RemoveLane { id: bend }));
    assert!(h.project().expression_lanes.is_empty());
    undo(&mut h);
    assert!(h.project().expression_lanes.contains_key(&bend));
    let gone: ExpressionLaneId = h.id();
    let out = h.send(x(ExpressionCommand::RemoveLane { id: gone }));
    assert_eq!(err(&out).code, ErrorCode::NotFound);
}

#[test]
fn set_points_and_replace_range_validate_and_undo_in_one_step() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
    let c = clip(&mut h, t, 0.0, 4.0);
    let lane = new_lane(&mut h, c, ExpressionKind::Cc { controller: 1 });
    h.ok(x(ExpressionCommand::SetPoints {
        lane,
        points: vec![pt(0.0, 0.0), pt(1.0, 0.5), pt(2.0, 1.0), pt(3.0, 0.25)],
    }));
    // Out of range, unsorted, non-finite: rejected, nothing changes.
    for bad in [
        vec![pt(0.0, 1.5)],
        vec![pt(1.0, 0.0), pt(0.5, 0.0)],
        vec![pt(f64::NAN, 0.0)],
        vec![pt(-1.0, 0.0)],
    ] {
        let out = h.send(x(ExpressionCommand::SetPoints { lane, points: bad }));
        assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    }
    assert_eq!(points(&h, lane).len(), 4);
    // A pencil stroke: three range replacements in one gesture = one undo step.
    let gesture = GestureId(7);
    for (s, e, v) in [(1.0, 1.5, 0.1), (1.0, 2.0, 0.2), (1.0, 2.5, 0.3)] {
        let out = h.send_with(
            x(ExpressionCommand::ReplaceRange {
                lane,
                start: Beats(s),
                end: Beats(e),
                points: vec![pt(s, v), pt(e - 0.25, v)],
            }),
            Some(gesture),
        );
        ok(&out);
    }
    h.ok(Command::Edit(EditCommand::EndGesture { gesture }));
    assert_eq!(
        points(&h, lane),
        vec![(0.0, 0.0), (1.0, 0.3), (2.25, 0.3), (3.0, 0.25)]
    );
    // Points outside the range are rejected.
    let out = h.send(x(ExpressionCommand::ReplaceRange {
        lane,
        start: Beats(0.0),
        end: Beats(1.0),
        points: vec![pt(1.0, 0.0)],
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    undo(&mut h);
    assert_eq!(
        points(&h, lane),
        vec![(0.0, 0.0), (1.0, 0.5), (2.0, 1.0), (3.0, 0.25)]
    );
    // Bend accepts -1..=1.
    let bend = new_lane(&mut h, c, ExpressionKind::PitchBend);
    h.ok(x(ExpressionCommand::SetPoints {
        lane: bend,
        points: vec![pt(0.0, -1.0), pt(1.0, 1.0)],
    }));
}

#[test]
fn note_expressions_create_replace_remove_and_clear() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
    let c = clip(&mut h, t, 0.0, 4.0);
    let a = note(&mut h, c, 60, 0.0, 1.0);
    let b = note(&mut h, c, 64, 1.0, 1.0);
    let id: NoteExpressionId = h.id();
    let set = |id, note, points| {
        x(ExpressionCommand::SetNoteExpression {
            id,
            note,
            kind: NoteExpressionKind::Pressure,
            points,
        })
    };
    h.ok(set(id, a, vec![pt(0.0, 0.2)]));
    // Same kind again (another id): replaces the points of the existing curve.
    let other: NoteExpressionId = h.id();
    h.ok(set(other, a, vec![pt(0.0, 0.4), pt(0.5, 1.0)]));
    let e = h.project().note_expressions_of(a);
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].id, id);
    assert_eq!(e[0].points.len(), 2);
    // Out of range.
    let out = h.send(set(other, a, vec![pt(0.0, 2.0)]));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    // Pitch takes ±96 semitones.
    let pitch: NoteExpressionId = h.id();
    h.ok(x(ExpressionCommand::SetNoteExpression {
        id: pitch,
        note: a,
        kind: NoteExpressionKind::Pitch,
        points: vec![pt(0.0, -12.0)],
    }));
    let pb: NoteExpressionId = h.id();
    h.ok(set(pb, b, vec![pt(0.0, 0.5)]));
    // Empty points remove the curve; on a note without one it's a no-op.
    h.ok(set(other, a, vec![]));
    assert_eq!(h.project().note_expressions_of(a).len(), 1);
    undo(&mut h);
    assert_eq!(h.project().note_expressions_of(a).len(), 2);
    // Clear one kind, then everything.
    h.ok(x(ExpressionCommand::ClearNoteExpressions {
        notes: vec![a, b],
        kind: Some(NoteExpressionKind::Pressure),
    }));
    assert_eq!(h.project().note_expressions.len(), 1);
    h.ok(x(ExpressionCommand::ClearNoteExpressions {
        notes: vec![a, b],
        kind: None,
    }));
    assert!(h.project().note_expressions.is_empty());
    // Unknown note.
    let missing: NoteId = h.id();
    let out = h.send(set(other, missing, vec![pt(0.0, 0.5)]));
    assert_eq!(err(&out).code, ErrorCode::NotFound);
}

#[test]
fn compile_maps_note_indices_through_mute_and_swing() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
    let c = clip(&mut h, t, 0.0, 4.0);
    // Notes at 0, 0.5 (swung later than the muted 0.55 one), 0.55 (muted), 1.0.
    let n0 = note(&mut h, c, 60, 0.0, 0.25);
    let n1 = note(&mut h, c, 62, 0.5, 0.25);
    let n2 = note(&mut h, c, 64, 0.55, 0.25);
    let n3 = note(&mut h, c, 65, 0.52, 0.25);
    h.ok(Command::Note(NoteCommand::Edit {
        edits: vec![ether_core::protocol::notes::NoteEdit {
            id: n2,
            pitch: None,
            velocity: None,
            start: None,
            duration: None,
            muted: Some(true),
        }],
    }));
    h.ok(Command::Groove(GrooveCommand::SetSwing {
        amount: 1.0,
        grid: Beats(0.5),
    }));
    for (n, v) in [(n0, 0.0), (n1, 0.1), (n2, 0.2), (n3, 0.3)] {
        let id: NoteExpressionId = h.id();
        h.ok(x(ExpressionCommand::SetNoteExpression {
            id,
            note: n,
            kind: NoteExpressionKind::Pressure,
            points: vec![pt(0.0, v)],
        }));
    }
    let lane = new_lane(&mut h, c, ExpressionKind::Cc { controller: 74 });
    // An empty lane compiles to nothing; with points it does.
    h.tick();
    assert!(expr_of(&h, t).clips[0].lanes.is_empty());
    h.ok(x(ExpressionCommand::SetPoints {
        lane,
        points: vec![pt(0.0, 0.5)],
    }));
    h.tick();
    let g = h.ctl.bridge.last_graph().clone();
    let td = g.tracks.iter().find(|d| d.id == t).unwrap();
    let ether_core::graph::ClipContentDesc::Midi { notes } = &td.clips[0].content else {
        panic!()
    };
    let cx = &td.expression.clips[0];
    assert_eq!(cx.clip, c);
    assert_eq!(cx.lanes.len(), 1);
    // Every compiled note expression points at the note with its key.
    let by_key = |k: u8| match k {
        60 => 0.0,
        62 => 0.1,
        65 => 0.3,
        _ => panic!("muted note compiled"),
    };
    assert_eq!(cx.notes.len(), 3);
    for e in &cx.notes {
        let key = notes[e.note as usize].key;
        assert_eq!(e.points[0].1, by_key(key), "note {}", e.note);
    }
    assert!(cx.notes.windows(2).all(|w| w[0].note < w[1].note));
    // Swing really reordered: the 0.5 note now starts after the 0.52 one.
    let keys: Vec<u8> = notes.iter().map(|n| n.key).collect();
    assert_eq!(keys, vec![60, 65, 62]);
}

#[test]
fn copies_keep_expression() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
    let c = clip(&mut h, t, 0.0, 4.0);
    let n = note(&mut h, c, 60, 1.0, 1.0);
    let id: NoteExpressionId = h.id();
    h.ok(x(ExpressionCommand::SetNoteExpression {
        id,
        note: n,
        kind: NoteExpressionKind::Pressure,
        points: vec![pt(0.0, 0.25), pt(0.5, 0.75)],
    }));
    let lane = new_lane(&mut h, c, ExpressionKind::PitchBend);
    h.ok(x(ExpressionCommand::SetPoints {
        lane,
        points: vec![pt(0.0, -0.5), pt(2.0, 0.5)],
    }));
    let curve = h.project().note_expressions[&id].points.clone();

    // Note::Duplicate.
    let copy: NoteId = h.id();
    h.ok(Command::Note(NoteCommand::Duplicate {
        copies: vec![NoteCopy {
            from: n,
            new_id: copy,
        }],
        offset: Beats(1.0),
        transpose: 0,
    }));
    let e = h.project().note_expressions_of(copy);
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].points, curve);
    assert_ne!(e[0].id, id);

    // Time-edit copy/paste onto another track.
    let t2 = track(&mut h, TrackKind::Midi);
    h.ok(Command::TimeEdit(TimeEditCommand::Copy {
        selection: TimeSelection {
            start: Beats(0.0),
            end: Beats(4.0),
            tracks: vec![t],
            global: false,
        },
    }));
    let seed: ClipId = h.id();
    h.ok(Command::TimeEdit(TimeEditCommand::Paste {
        at: Beats(8.0),
        tracks: vec![t2],
        insert: false,
        seed,
    }));
    let pasted = h.project().arrangement_clips_of(t2)[0].id;
    let lanes = h.project().expression_lanes_of(pasted);
    assert_eq!(lanes.len(), 1);
    assert_eq!(lanes[0].points.len(), 2);
    let pasted_notes = h.project().notes_of(pasted);
    assert_eq!(pasted_notes.len(), 2);
    for pn in &pasted_notes {
        assert_eq!(h.project().note_expressions_of(pn.id)[0].points, curve);
    }

    // Consolidate a looped, offset clip: lanes are unrolled into the new clip's time.
    let t3 = track(&mut h, TrackKind::Midi);
    let c3 = clip(&mut h, t3, 0.0, 4.0);
    let n3 = note(&mut h, c3, 67, 0.5, 0.5);
    let e3: NoteExpressionId = h.id();
    h.ok(x(ExpressionCommand::SetNoteExpression {
        id: e3,
        note: n3,
        kind: NoteExpressionKind::Pressure,
        points: vec![pt(0.0, 1.0)],
    }));
    let l3 = new_lane(&mut h, c3, ExpressionKind::Cc { controller: 1 });
    h.ok(x(ExpressionCommand::SetPoints {
        lane: l3,
        points: vec![pt(0.0, 0.0), pt(1.0, 1.0)],
    }));
    h.ok(Command::Clip(ClipCommand::SetLoop {
        id: c3,
        looping: ClipLoop {
            enabled: true,
            start: Beats(0.0),
            end: Beats(1.0),
        },
    }));
    let (seed_clips, seed_notes, seed_media): (ClipId, ClipId, MediaId) = (h.id(), h.id(), h.id());
    h.ok(Command::Freeze(FreezeCommand::Consolidate {
        job: "x".into(),
        tracks: vec![t3],
        start: Beats(0.0),
        end: Beats(3.0),
        seed_clips,
        seed_notes,
        seed_media,
    }));
    let merged = ether_model::derive_id::<_, ClipId>(seed_clips, 0);
    let p = h.project();
    assert_eq!(p.notes_of(merged).len(), 3);
    for mn in p.notes_of(merged) {
        assert_eq!(p.note_expressions_of(mn.id).len(), 1);
    }
    let cc = p.expression_lanes_of(merged);
    assert_eq!(cc.len(), 1);
    let eval = |t: f64| {
        let pts = &cc[0].points;
        let i = pts.partition_point(|q| q.time.0 <= t);
        let (a, b) = (pts[i - 1], pts.get(i).copied().unwrap_or(pts[i - 1]));
        if b.time.0 <= a.time.0 || a.curve == CurveShape::Step {
            a.value
        } else {
            a.value + (b.value - a.value) * ((t - a.time.0) / (b.time.0 - a.time.0)) as f32
        }
    };
    // Each loop pass ramps 0 → 1 again.
    for pass in 0..3 {
        let base = pass as f64;
        assert!((eval(base + 0.0) - 0.0).abs() < 1e-6, "pass {pass}");
        assert!((eval(base + 0.5) - 0.5).abs() < 1e-6, "pass {pass}");
    }
}

#[test]
fn comp_flatten_keeps_expression() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
    let lane: TakeLaneId = h.id();
    h.ok(Command::Take(TakeCommand::CreateLane {
        id: lane,
        track: t,
        name: None,
        before: None,
    }));
    let c = clip(&mut h, t, 0.0, 4.0);
    h.ok(Command::Take(TakeCommand::MoveToLane {
        clips: vec![c],
        lane: Some(lane),
    }));
    let n = note(&mut h, c, 60, 0.0, 1.0);
    let e: NoteExpressionId = h.id();
    h.ok(x(ExpressionCommand::SetNoteExpression {
        id: e,
        note: n,
        kind: NoteExpressionKind::Pressure,
        points: vec![pt(0.0, 0.5)],
    }));
    let l = new_lane(&mut h, c, ExpressionKind::ChannelPressure);
    h.ok(x(ExpressionCommand::SetPoints {
        lane: l,
        points: vec![pt(0.0, 0.5)],
    }));
    let (id, split_id): (CompRegionId, CompRegionId) = (h.id(), h.id());
    h.ok(Command::Take(TakeCommand::SetComp {
        id,
        split_id,
        track: t,
        lane,
        start: Beats(0.0),
        end: Beats(4.0),
    }));
    let (seed, seed_notes): (ClipId, ClipId) = (h.id(), h.id());
    h.ok(Command::Take(TakeCommand::Flatten {
        track: t,
        seed,
        seed_notes,
        keep_lanes: false,
    }));
    let p = h.project();
    let flat = p.arrangement_clips_of(t)[0].id;
    assert_eq!(p.expression_lanes_of(flat).len(), 1);
    let fnote = p.notes_of(flat)[0].id;
    assert_eq!(p.note_expressions_of(fnote).len(), 1);
}

/// A MIDI track with a Poly Synth playing a note under a bend lane, a CC 1 lane and note
/// pressure, compiled by the controller and rendered offline with blocks of `block`.
fn render_poly(h: &mut Harness, t: TrackId, block: usize) -> Vec<f32> {
    let mut g: RenderGraphDesc = h.ctl.bridge.last_graph().clone();
    let config = EngineConfig {
        sample_rate: 48_000,
        max_block_size: block,
        max_nodes: 16,
        max_events_per_block: 1024,
        ..EngineConfig::default()
    };
    let mut r = OfflineRenderer::new(config);
    let synth: Box<dyn Node> =
        ether_devices::create(&BuiltinDevice::PolySynth, &ether_devices::NoSamples);
    let key = r.handle().add_node(synth).unwrap();
    for td in &mut g.tracks {
        if td.id == t {
            assert_eq!(td.chain.len(), 1);
            td.chain[0].node = key;
        } else {
            td.chain.clear();
        }
    }
    r.publish(g).unwrap();
    r.start(Beats(0.0)).unwrap();
    let frames = 3 * 24_000;
    let mut l = vec![0.0f32; frames];
    let mut rr = vec![0.0f32; frames];
    r.render(frames, &mut [&mut l, &mut rr]);
    l
}

#[test]
fn offline_render_through_the_poly_synth_is_block_size_independent() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi);
    let dev: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id: dev,
        track: t,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::PolySynth,
        },
        before: None,
    }));
    let c = clip(&mut h, t, 0.0, 4.0);
    let n = note(&mut h, c, 57, 0.25, 2.0);
    let bend = new_lane(&mut h, c, ExpressionKind::PitchBend);
    h.ok(x(ExpressionCommand::SetPoints {
        lane: bend,
        points: vec![
            pt(0.0, 0.0),
            pt(1.0, 1.0),
            ExpressionPoint {
                time: Beats(1.5),
                value: -0.5,
                curve: CurveShape::Step,
            },
            pt(2.0, 0.25),
        ],
    }));
    let cc = new_lane(&mut h, c, ExpressionKind::Cc { controller: 1 });
    h.ok(x(ExpressionCommand::SetPoints {
        lane: cc,
        points: vec![pt(0.0, 0.0), pt(2.0, 1.0)],
    }));
    let e: NoteExpressionId = h.id();
    h.ok(x(ExpressionCommand::SetNoteExpression {
        id: e,
        note: n,
        kind: NoteExpressionKind::Pressure,
        points: vec![pt(0.0, 0.0), pt(1.0, 1.0)],
    }));
    h.tick();
    let small = render_poly(&mut h, t, 64);
    let large = render_poly(&mut h, t, 512);
    assert!(small.iter().any(|s| s.abs() > 1e-3), "the synth plays");
    assert_eq!(small, large, "offline render identical for blocks of 64 and 512");
    // And the bend is audible: without the lane the render differs.
    h.ok(x(ExpressionCommand::RemoveLane { id: bend }));
    h.tick();
    let flat = render_poly(&mut h, t, 512);
    assert_ne!(flat, large);
}

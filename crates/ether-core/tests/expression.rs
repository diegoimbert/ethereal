//! MIDI expression playback (`midi-expression`, CONTRACTS.md §13.2): clip lanes as raw MIDI
//! and note expressions as `NoteExpression`, sample-accurate and identical for every block
//! size, only on value changes, re-sent after jumps, bend centred when its clip stops, and
//! note expressions following their note across loop wraps and clip moves.

mod common;

use common::*;
use ether_core::expression::{
    ClipExpressionDesc, ExpressionLaneDesc, NoteExpressionDesc, TrackExpressionDesc,
};
use ether_core::graph::{ClipDesc, RenderGraphDesc, TrackDesc};
use ether_core::protocol::model::{
    BeatRange, Beats, CurveShape, ExpressionKind, NoteExpressionKind, TrackKind,
};
use ether_core::{EngineParts, EventKind, NodeKey, TransportControl, create};

/// 120 BPM at 48 kHz.
const SPB: u64 = 24_000;

fn lane(kind: ExpressionKind, points: &[(f64, f32, CurveShape)]) -> ExpressionLaneDesc {
    ExpressionLaneDesc {
        kind,
        points: points.to_vec(),
    }
}

fn note_curve(note: u32, kind: NoteExpressionKind, points: &[(f64, f32)]) -> NoteExpressionDesc {
    NoteExpressionDesc {
        note,
        kind,
        points: points
            .iter()
            .map(|&(t, v)| (t, v, CurveShape::Linear))
            .collect(),
    }
}

/// A MIDI track (id 2) playing `clips` into a [`Recorder`].
fn setup(clips: Vec<ClipDesc>, expression: TrackExpressionDesc) -> (EngineParts, NodeKey, Seen0) {
    let mut p = create(config());
    let (rec, rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    p.handle
        .publish(desc(rec, clips, expression, None))
        .unwrap();
    (p, rec, rx)
}

type Seen0 = rtrb::Consumer<Seen>;

fn desc(
    rec: NodeKey,
    clips: Vec<ClipDesc>,
    expression: TrackExpressionDesc,
    looping: Option<(f64, f64)>,
) -> RenderGraphDesc {
    let mut t: TrackDesc = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[rec]);
    t.clips = clips;
    t.expression = expression;
    let (loop_enabled, (loop_start, loop_end)) = match looping {
        Some(l) => (true, l),
        None => (false, (0.0, 4.0)),
    };
    RenderGraphDesc {
        version: 1,
        tracks: vec![master(), t],
        loop_enabled,
        loop_start,
        loop_end,
        ..Default::default()
    }
}

fn midi(seen: &[Seen]) -> Vec<(u64, [u8; 3])> {
    events(seen)
        .into_iter()
        .filter_map(|(t, k)| match k {
            EventKind::Midi { data } => Some((t, data)),
            _ => None,
        })
        .collect()
}

/// `(sample, note_id, key, kind, value)` of the note expressions, and the note on/offs.
#[allow(clippy::type_complexity)]
fn notes(seen: &[Seen]) -> Vec<(u64, EventKind)> {
    events(seen)
        .into_iter()
        .filter(|(_, k)| {
            matches!(
                k,
                EventKind::NoteOn { .. }
                    | EventKind::NoteOff { .. }
                    | EventKind::NoteExpression { .. }
            )
        })
        .collect()
}

fn one_clip_expression(
    clip: ClipDesc,
    lanes: Vec<ExpressionLaneDesc>,
    n: Vec<NoteExpressionDesc>,
) -> (Vec<ClipDesc>, TrackExpressionDesc) {
    let x = TrackExpressionDesc {
        clips: vec![ClipExpressionDesc {
            clip: clip.id,
            lanes,
            notes: n,
        }],
        mpe: None,
    };
    (vec![clip], x)
}

fn run(clips: Vec<ClipDesc>, x: TrackExpressionDesc, frames: usize, block: usize) -> Vec<Seen> {
    let (mut p, _, mut rx) = setup(clips, x);
    p.handle.transport(TransportControl::Play).unwrap();
    render(&mut p.engine, frames, block);
    drain(&mut rx)
}

fn ramps() -> (Vec<ClipDesc>, TrackExpressionDesc) {
    let clip = midi_clip(cid(7), 0.0, 4.0, &[(0.0, 1.0, 60), (1.0, 2.0, 64)]);
    one_clip_expression(
        clip,
        vec![
            lane(
                ExpressionKind::Cc { controller: 1 },
                &[
                    (0.0, 0.0, CurveShape::Linear),
                    (2.0, 1.0, CurveShape::Curve { tension: 0.4 }),
                    (3.0, 0.2, CurveShape::Linear),
                ],
            ),
            lane(
                ExpressionKind::PitchBend,
                &[
                    (0.0, 0.0, CurveShape::Linear),
                    (1.5, -1.0, CurveShape::Step),
                    (2.25, 0.5, CurveShape::Linear),
                ],
            ),
        ],
        vec![
            note_curve(0, NoteExpressionKind::Pressure, &[(0.0, 0.0), (1.0, 1.0)]),
            note_curve(1, NoteExpressionKind::Pressure, &[(0.5, 1.0), (1.5, 0.0)]),
        ],
    )
}

#[test]
fn lanes_and_note_expressions_are_block_size_independent() {
    let frames = 4 * SPB as usize + 1000;
    let (clips, x) = ramps();
    let reference = run(clips.clone(), x.clone(), frames, 512);
    for block in [64, 100, 333] {
        let got = run(clips.clone(), x.clone(), frames, block);
        assert_eq!(midi(&got), midi(&reference), "block {block}");
        let (a, b) = (notes(&got), notes(&reference));
        if let Some(i) = (0..a.len().min(b.len())).find(|&i| a[i] != b[i]) {
            panic!(
                "block {block}: event {i}: {:?} vs {:?} (prev {:?})",
                a[i],
                b[i],
                &b[i.saturating_sub(2)..i]
            );
        }
        assert_eq!(a.len(), b.len(), "block {block}");
    }
    assert!(!midi(&reference).is_empty());
}

#[test]
fn lanes_send_only_changes_on_the_grid_and_breakpoints() {
    let (clips, x) = ramps();
    let seen = run(clips, x, 4 * SPB as usize + 1000, 512);
    let cc: Vec<(u64, u8)> = midi(&seen)
        .into_iter()
        .filter(|(_, d)| d[0] == 0xB0 && d[1] == 1)
        .map(|(t, d)| (t, d[2]))
        .collect();
    // Starts at 0 on the first sample, rises to 127 at beat 2, lands on 0.2 at beat 3.
    assert_eq!(cc[0], (0, 0));
    assert!(cc.windows(2).all(|w| w[0].1 != w[1].1), "only changes");
    assert!(
        cc.iter()
            .all(|(t, _)| t % 32 == 0 || *t == 2 * SPB || *t == 3 * SPB)
    );
    let top = cc.iter().find(|&&(_, v)| v == 127).unwrap();
    assert!(top.0 <= 2 * SPB && top.0 > 2 * SPB - 64 * 32, "{top:?}");
    assert_eq!(cc.last().unwrap().1, (0.2f32 * 127.0).round() as u8);
    // The bend's step lands exactly on beat 2.25 (14-bit +0.5 = 8192 + 4096).
    let bend: Vec<(u64, u16)> = midi(&seen)
        .into_iter()
        .filter(|(_, d)| d[0] == 0xE0)
        .map(|(t, d)| (t, u16::from(d[1]) | (u16::from(d[2]) << 7)))
        .collect();
    assert_eq!(bend[0], (0, 8192));
    let step = bend.iter().position(|&(_, v)| v == 8192 + 4096).unwrap();
    assert_eq!(bend[step].0, 9 * SPB / 4);
    assert!(bend[..step].iter().all(|&(t, _)| t < 9 * SPB / 4));
    // -1 holds from 1.5 to 2.25: one event for it.
    assert_eq!(bend.iter().filter(|&&(_, v)| v == 1).count(), 1);
    // The clip ends at beat 4: the bend is centred there.
    assert_eq!(*bend.last().unwrap(), (4 * SPB, 8192));
}

#[test]
fn lanes_go_out_before_the_notes_of_their_sample() {
    let (clips, x) = ramps();
    let seen = run(clips, x, 1000, 512);
    let ev = events(&seen);
    let first_note = ev
        .iter()
        .position(|(_, k)| matches!(k, EventKind::NoteOn { .. }))
        .unwrap();
    let first_cc = ev
        .iter()
        .position(|(_, k)| matches!(k, EventKind::Midi { .. }))
        .unwrap();
    assert!(first_cc < first_note);
}

#[test]
fn note_expressions_track_their_voice() {
    let (clips, x) = ramps();
    let seen = run(clips, x, 4 * SPB as usize, 512);
    let ev = notes(&seen);
    let mut on: Vec<(u32, u8, u64)> = Vec::new();
    for (t, k) in &ev {
        match *k {
            EventKind::NoteOn { note_id, key, .. } => on.push((note_id, key, *t)),
            EventKind::NoteOff { note_id, .. } => on.retain(|n| n.0 != note_id),
            EventKind::NoteExpression {
                note_id,
                key,
                expression,
                value,
                ..
            } => {
                let n = on.iter().find(|n| n.0 == note_id).expect("sounding note");
                assert_eq!(n.1, key);
                assert_eq!(expression, NoteExpressionKind::Pressure);
                assert!((0.0..=1.0).contains(&value));
            }
            _ => {}
        }
    }
    // The first value follows the note-on on its sample; the note at beat 1 starts its
    // curve at 1.0 (held before its first point at 0.5).
    let first_64 = ev
        .iter()
        .find(|(_, k)| matches!(k, EventKind::NoteExpression { key: 64, .. }))
        .unwrap();
    assert_eq!(first_64.0, SPB);
    assert!(matches!(first_64.1, EventKind::NoteExpression { value, .. } if value == 1.0));
    // Pressure of the first note rises to (almost) 1 before its note-off at beat 1.
    let last_60 = ev
        .iter()
        .rfind(|(_, k)| matches!(k, EventKind::NoteExpression { key: 60, .. }))
        .unwrap();
    assert!(last_60.0 < SPB);
    assert!(matches!(last_60.1, EventKind::NoteExpression { value, .. } if value > 0.99));
}

#[test]
fn note_expressions_follow_their_note_across_loop_wraps_and_clip_moves() {
    // A 2-beat transport loop over a clip with one note at 0.5..1.5 (pressure 0 → 1).
    let clip = midi_clip(cid(7), 0.0, 2.0, &[(0.5, 1.0, 62)]);
    let (clips, x) = one_clip_expression(
        clip,
        vec![],
        vec![note_curve(
            0,
            NoteExpressionKind::Pressure,
            &[(0.0, 0.0), (1.0, 1.0)],
        )],
    );
    let mut p = create(config());
    let (rec, mut rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    p.handle
        .publish(desc(rec, clips.clone(), x.clone(), Some((0.0, 2.0))))
        .unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    render(&mut p.engine, 5 * SPB as usize, 512);
    let ev = notes(&drain(&mut rx));
    // Two passes (and a half): each pass's voice gets its own curve from 0, starting on its
    // note-on.
    let ons: Vec<(u64, u32)> = ev
        .iter()
        .filter_map(|(t, k)| match k {
            EventKind::NoteOn { note_id, .. } => Some((*t, *note_id)),
            _ => None,
        })
        .collect();
    assert_eq!(ons.len(), 3);
    for (i, &(t, id)) in ons.iter().enumerate() {
        assert_eq!(t, SPB / 2 + i as u64 * 2 * SPB);
        let first = ev
            .iter()
            .find(|(_, k)| matches!(k, EventKind::NoteExpression { note_id, .. } if *note_id == id))
            .unwrap();
        assert_eq!(first.0, t);
        assert!(matches!(first.1, EventKind::NoteExpression { value, .. } if value == 0.0));
    }

    // Move the clip by one beat while playing: the curve stays on the note.
    let mut moved = clips[0].clone();
    moved.start = 1.0;
    p.handle
        .publish(desc(rec, vec![moved], x, Some((0.0, 4.0))))
        .unwrap();
    p.handle
        .transport(TransportControl::Locate {
            position: Beats(0.0),
        })
        .unwrap();
    drain(&mut rx);
    render(&mut p.engine, 3 * SPB as usize, 512);
    let ev = notes(&drain(&mut rx));
    let on = ev
        .iter()
        .find(|(_, k)| matches!(k, EventKind::NoteOn { .. }))
        .unwrap();
    let id = match on.1 {
        EventKind::NoteOn { note_id, .. } => note_id,
        _ => unreachable!(),
    };
    // The block's sample clock continues: compare relative to the note-on.
    let exprs: Vec<(u64, f32)> = ev
        .iter()
        .filter_map(|(t, k)| match k {
            EventKind::NoteExpression { note_id, value, .. } if *note_id == id => {
                Some((*t, *value))
            }
            _ => None,
        })
        .collect();
    assert_eq!(exprs[0], (on.0, 0.0));
    // Half way through the note (half a beat in): about 0.5.
    let mid = exprs.iter().find(|(t, _)| *t >= on.0 + SPB / 2).unwrap();
    assert!((mid.1 - 0.5).abs() < 0.01, "{mid:?}");
}

#[test]
fn locate_resends_current_values_and_stop_centres_the_bend() {
    let clip = midi_clip(cid(7), 0.0, 8.0, &[]);
    let (clips, x) = one_clip_expression(
        clip,
        vec![
            lane(
                ExpressionKind::Cc { controller: 11 },
                &[(0.0, 0.5, CurveShape::Step)],
            ),
            lane(ExpressionKind::PitchBend, &[(0.0, 0.25, CurveShape::Step)]),
            lane(
                ExpressionKind::ChannelPressure,
                &[(0.0, 1.0, CurveShape::Step)],
            ),
        ],
        vec![],
    );
    let (mut p, _, mut rx) = setup(clips, x);
    p.handle.transport(TransportControl::Play).unwrap();
    render(&mut p.engine, 4096, 512);
    let first = midi(&drain(&mut rx));
    // Each constant lane sent once.
    assert_eq!(first.len(), 3, "{first:?}");
    assert!(first.iter().all(|(t, _)| *t == 0));
    assert!(first.contains(&(0, [0xB0, 11, 64])));
    assert!(first.contains(&(0, [0xD0, 127, 0])));
    p.handle
        .transport(TransportControl::Locate {
            position: Beats(2.0),
        })
        .unwrap();
    render(&mut p.engine, 4096, 512);
    let again = midi(&drain(&mut rx));
    assert_eq!(again.len(), 3, "re-sent once after the locate: {again:?}");
    let at = again[0].0;
    assert!(again.iter().all(|(t, _)| *t == at));
    // A loop override wraps: re-sent after each wrap.
    p.handle
        .transport(TransportControl::SetLoop {
            enabled: true,
            region: BeatRange {
                start: Beats(2.0),
                end: Beats(2.5),
            },
        })
        .unwrap();
    render(&mut p.engine, SPB as usize, 512);
    let wraps = midi(&drain(&mut rx));
    // Two wraps in a beat (at 2.5 after the locate's ~2.17, then every half beat).
    assert_eq!(wraps.len(), 2 * 3, "{wraps:?}");
    // Stop: the bend goes back to the centre.
    p.handle.transport(TransportControl::Stop).unwrap();
    render(&mut p.engine, 1024, 512);
    let stop = midi(&drain(&mut rx));
    assert_eq!(stop.len(), 1, "{stop:?}");
    assert_eq!(stop[0].1, [0xE0, 0, 0x40]);
}

#[test]
fn muted_clips_and_missing_curves_send_nothing() {
    let mut clip = midi_clip(cid(7), 0.0, 4.0, &[(0.0, 1.0, 60)]);
    clip.muted = true;
    let (clips, x) = one_clip_expression(
        clip,
        vec![lane(
            ExpressionKind::Cc { controller: 1 },
            &[(0.0, 0.5, CurveShape::Step)],
        )],
        vec![note_curve(0, NoteExpressionKind::Pressure, &[(0.0, 0.5)])],
    );
    let seen = run(clips, x, 2 * SPB as usize, 512);
    assert!(midi(&seen).is_empty());
    assert!(notes(&seen).is_empty());
    // Expression of another clip id: nothing either.
    let clip = midi_clip(cid(8), 0.0, 4.0, &[(0.0, 1.0, 60)]);
    let (clips, mut x) = one_clip_expression(
        clip,
        vec![lane(
            ExpressionKind::Cc { controller: 1 },
            &[(0.0, 0.5, CurveShape::Step)],
        )],
        vec![note_curve(0, NoteExpressionKind::Pressure, &[(0.0, 0.5)])],
    );
    x.clips[0].clip = cid(9);
    let seen = run(clips, x, 2 * SPB as usize, 512);
    assert!(midi(&seen).is_empty());
    assert!(
        !notes(&seen)
            .iter()
            .any(|(_, k)| matches!(k, EventKind::NoteExpression { .. }))
    );
}

/// CPU cost (run with `cargo test --release -p ether-core --test expression -- --ignored
/// --nocapture`): a 512-frame block of one track with 4 dense lanes and 16 sounding notes
/// with pressure curves vs the same track without expression.
#[test]
#[ignore]
fn cpu_cost() {
    let dense = |n: usize| -> Vec<(f64, f32, CurveShape)> {
        (0..n)
            .map(|i| (i as f64 * 0.01, (i % 13) as f32 / 13.0, CurveShape::Linear))
            .collect()
    };
    let notes: Vec<(f64, f64, u8)> = (0..16).map(|i| (0.0, 8.0, 48 + i as u8)).collect();
    let clip = midi_clip(cid(7), 0.0, 8.0, &notes);
    let with = TrackExpressionDesc {
        clips: vec![ClipExpressionDesc {
            clip: cid(7),
            lanes: [
                ExpressionKind::Cc { controller: 1 },
                ExpressionKind::Cc { controller: 11 },
                ExpressionKind::PitchBend,
                ExpressionKind::ChannelPressure,
            ]
            .into_iter()
            .map(|kind| ExpressionLaneDesc {
                kind,
                points: dense(800),
            })
            .collect(),
            notes: (0..16)
                .map(|note| NoteExpressionDesc {
                    note,
                    kind: NoteExpressionKind::Pressure,
                    points: dense(800),
                })
                .collect(),
        }],
        mpe: None,
    };
    for (label, x) in [("without", TrackExpressionDesc::default()), ("with", with)] {
        let (mut p, _, mut rx) = setup(vec![clip.clone()], x);
        p.handle.transport(TransportControl::Play).unwrap();
        let blocks = 4 * SPB as usize / 512;
        let t = std::time::Instant::now();
        let mut l = vec![0.0f32; 512];
        let mut r = vec![0.0f32; 512];
        for _ in 0..blocks {
            let mut outs: [&mut [f32]; 2] = [&mut l, &mut r];
            p.engine.process(&[], &mut outs, 512);
            drain(&mut rx);
        }
        let per = t.elapsed().as_nanos() as f64 / blocks as f64;
        println!(
            "{label}: {per:.0} ns per 512-frame block ({:.3}% of real time)",
            per / (512.0 / 48_000.0 * 1e9) * 100.0
        );
    }
}

//! Loop joins (CONTRACTS.md §2, Clips, "loop join"; #272): in a looping MIDI clip, a
//! note held across the content loop end joins the note of the same key that starts before
//! the loop start and still sounds there: no note-off/note-on at the wrap, one voice that
//! ends where the loop start note ends. Exact event output, every block size, chasing,
//! repeated wraps and note expressions.

use crate::common;

use common::*;
use ether_core::expression::{ClipExpressionDesc, NoteExpressionDesc, TrackExpressionDesc};
use ether_core::graph::ClipDesc;
use ether_core::protocol::model::{Beats, CurveShape, NoteExpressionKind, TrackKind};
use ether_core::{EventKind, RenderGraphDesc, TransportControl, create};

/// Block sizes: dividing the boundaries (on a block edge) and not (mid-block).
const BLOCKS: [usize; 5] = [7, 64, 500, 509, 512];

/// 120 BPM at 48 kHz.
const SPB: f64 = 24_000.0;

fn s(beats: f64) -> u64 {
    (beats * SPB).round() as u64
}

/// An 8-beat clip looping content `[1, 3)` (wraps at timeline 3, 5 and 7).
fn looped(notes: &[(f64, f64, u8)]) -> ClipDesc {
    let mut c = midi_clip(cid(1), 0.0, 8.0, notes);
    c.looping = Some((1.0, 3.0));
    c
}

/// Every event the track's instrument saw, played from `from` (beats) for `frames`.
fn run(
    clips: Vec<ClipDesc>,
    expression: TrackExpressionDesc,
    from: f64,
    frames: usize,
    block: usize,
) -> Vec<(u64, EventKind)> {
    let mut p = create(config());
    let (rec, mut rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    let mut t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[rec]);
    t.clips = clips;
    t.expression = expression;
    p.handle
        .publish(RenderGraphDesc {
            version: 1,
            tracks: vec![master(), t],
            ..Default::default()
        })
        .unwrap();
    if from > 0.0 {
        p.handle
            .transport(TransportControl::Locate {
                position: Beats(from),
            })
            .unwrap();
    }
    p.handle.transport(TransportControl::Play).unwrap();
    render(&mut p.engine, frames, block);
    events(&drain(&mut rx))
}

/// `(sample, key, is_on)` of the note events.
fn note_events(clip: ClipDesc, from: f64, frames: usize, block: usize) -> Vec<(u64, u8, bool)> {
    run(
        vec![clip],
        TrackExpressionDesc::default(),
        from,
        frames,
        block,
    )
    .into_iter()
    .filter_map(|(t, k)| match k {
        EventKind::NoteOn { key, .. } => Some((t, key, true)),
        EventKind::NoteOff { key, .. } => Some((t, key, false)),
        _ => None,
    })
    .collect()
}

fn check(clip: impl Fn() -> ClipDesc, from: f64, frames: usize, want: &[(f64, u8, bool)]) {
    let want: Vec<(u64, u8, bool)> = want.iter().map(|&(b, k, on)| (s(b), k, on)).collect();
    for block in BLOCKS {
        assert_eq!(
            note_events(clip(), from, frames, block),
            want,
            "block {block}"
        );
    }
}

#[test]
fn a_note_held_across_the_loop_end_joins_the_loop_start_note() {
    // A (60) at 0.5..1.5 sounds at the loop start; B (60) at 2.5..3.5 crosses the loop end.
    // The intro plays A, then B, which joins A at every wrap: it ends half a beat after
    // each wrap (where A ends), with no off/on pair at the wrap.
    check(
        || looped(&[(0.5, 1.0, 60), (2.5, 1.0, 60)]),
        0.0,
        s(8.5) as usize,
        &[
            (0.5, 60, true),
            (1.5, 60, false),
            (2.5, 60, true),
            (3.5, 60, false),
            (4.5, 60, true),
            (5.5, 60, false),
            (6.5, 60, true),
            (7.5, 60, false),
        ],
    );
}

#[test]
fn different_keys_or_notes_not_crossing_the_boundaries_do_not_join() {
    // Different key: B is cut at each wrap, A never plays again after the intro.
    check(
        || looped(&[(0.5, 1.0, 62), (2.5, 1.0, 60)]),
        0.0,
        s(8.5) as usize,
        &[
            (0.5, 62, true),
            (1.5, 62, false),
            (2.5, 60, true),
            (3.0, 60, false),
            (4.5, 60, true),
            (5.0, 60, false),
            (6.5, 60, true),
            (7.0, 60, false),
        ],
    );
    // B ends exactly at the loop end: cut there as before.
    check(
        || looped(&[(0.5, 1.0, 60), (2.5, 0.5, 60)]),
        0.0,
        s(5.5) as usize,
        &[
            (0.5, 60, true),
            (1.5, 60, false),
            (2.5, 60, true),
            (3.0, 60, false),
            (4.5, 60, true),
            (5.0, 60, false),
        ],
    );
    // A ends exactly at the loop start (doesn't sound there): B is cut.
    check(
        || looped(&[(0.5, 0.5, 60), (2.5, 1.0, 60)]),
        0.0,
        s(5.5) as usize,
        &[
            (0.5, 60, true),
            (1.0, 60, false),
            (2.5, 60, true),
            (3.0, 60, false),
            (4.5, 60, true),
            (5.0, 60, false),
        ],
    );
    // A note starting exactly at the loop start is an ordinary loop note: it retriggers.
    check(
        || looped(&[(1.0, 0.5, 60), (2.5, 1.0, 60)]),
        0.0,
        s(5.6) as usize,
        &[
            (1.0, 60, true),
            (1.5, 60, false),
            (2.5, 60, true),
            (3.0, 60, false),
            (3.0, 60, true),
            (3.5, 60, false),
            (4.5, 60, true),
            (5.0, 60, false),
            (5.0, 60, true),
            (5.5, 60, false),
        ],
    );
}

#[test]
fn a_loop_start_note_longer_than_the_loop_joins_itself_to_the_clip_end() {
    // A (60) at 0.5..3.5 starts before the loop start and crosses the loop end: once held
    // across the first wrap it joins itself on every wrap, one voice to the clip end.
    check(
        || looped(&[(0.5, 3.0, 60)]),
        0.0,
        s(8.5) as usize,
        &[(0.5, 60, true), (8.0, 60, false)],
    );
    // The clip ending at the wrap cuts the voice there (no pass to join).
    check(
        || {
            let mut c = looped(&[(0.5, 1.0, 60), (2.5, 1.0, 60)]);
            c.length = 5.0;
            c
        },
        0.0,
        s(6.0) as usize,
        &[
            (0.5, 60, true),
            (1.5, 60, false),
            (2.5, 60, true),
            (3.5, 60, false),
            (4.5, 60, true),
            (5.0, 60, false),
        ],
    );
}

#[test]
fn chasing_into_a_joined_voice_plays_its_rest() {
    // Locate to 5.2: inside pass 1's joined part (B of pass 0 joined A until 5.5).
    let clip = || looped(&[(0.5, 1.0, 60), (2.5, 1.0, 60)]);
    check(
        clip,
        5.2,
        s(3.0) as usize,
        &[
            (0.0, 60, true),
            (0.3, 60, false),
            (1.3, 60, true),
            (2.3, 60, false),
        ],
    );
    // In pass 0 the joined voice comes from the intro's B too.
    check(
        clip,
        3.25,
        s(0.5) as usize,
        &[(0.0, 60, true), (0.25, 60, false)],
    );
    // No join (different keys): A stays silent after the intro, as before.
    check(
        || looped(&[(0.5, 1.0, 62), (2.5, 1.0, 60)]),
        5.2,
        s(1.0) as usize,
        &[],
    );
    // The self-joining loop start note: chased anywhere after the first wrap.
    check(
        || looped(&[(0.5, 3.0, 60)]),
        6.0,
        s(3.0) as usize,
        &[(0.0, 60, true), (2.0, 60, false)],
    );
}

#[test]
fn a_joined_voice_follows_the_loop_start_notes_expression_from_the_wrap() {
    // B's pressure is 1, A's 0: the joined voice keeps B's note-on (no retrigger) and
    // switches to A's curve at the wrap.
    let clip = looped(&[(0.5, 1.0, 60), (2.5, 1.0, 60)]);
    let curve = |note: u32, v: f32| NoteExpressionDesc {
        note,
        kind: NoteExpressionKind::Pressure,
        points: vec![(0.0, v, CurveShape::Linear)],
    };
    let x = TrackExpressionDesc {
        clips: vec![ClipExpressionDesc {
            clip: clip.id,
            lanes: vec![],
            notes: vec![curve(0, 0.0), curve(1, 1.0)],
        }],
        mpe: None,
    };
    for block in BLOCKS {
        let ev = run(vec![clip.clone()], x.clone(), 0.0, s(5.6) as usize, block);
        let seen: Vec<(u64, &str, f32)> = ev
            .iter()
            .filter_map(|(t, k)| match *k {
                EventKind::NoteOn { .. } => Some((*t, "on", 0.0)),
                EventKind::NoteOff { .. } => Some((*t, "off", 0.0)),
                EventKind::NoteExpression { value, .. } => Some((*t, "x", value)),
                _ => None,
            })
            .collect();
        assert_eq!(
            seen,
            [
                (s(0.5), "on", 0.0),
                (s(0.5), "x", 0.0),
                (s(1.5), "off", 0.0),
                (s(2.5), "on", 0.0),
                (s(2.5), "x", 1.0),
                (s(3.0), "x", 0.0),
                (s(3.5), "off", 0.0),
                (s(4.5), "on", 0.0),
                (s(4.5), "x", 1.0),
                (s(5.0), "x", 0.0),
                (s(5.5), "off", 0.0),
            ],
            "block {block}"
        );
    }
}

#[test]
fn offline_renders_join_like_playback() {
    // The export path (`OfflineRenderer`, render-sized chunks): the same joined notes.
    for chunk in [100, 4096] {
        let mut r = ether_core::offline::OfflineRenderer::new(config());
        let (rec, mut rx) = Recorder::new();
        let rec = r.handle().add_node(Box::new(rec)).unwrap();
        let mut t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[rec]);
        t.clips = vec![looped(&[(0.5, 1.0, 60), (2.5, 1.0, 60)])];
        r.publish(RenderGraphDesc {
            version: 1,
            tracks: vec![master(), t],
            ..Default::default()
        })
        .unwrap();
        r.start(Beats(0.0)).unwrap();
        let (mut l, mut rr) = (vec![0.0; chunk], vec![0.0; chunk]);
        let mut done = 0;
        while done < s(6.0) as usize {
            r.render(chunk, &mut [&mut l, &mut rr]);
            done += chunk;
        }
        let ev: Vec<(u64, bool)> = events(&drain(&mut rx))
            .into_iter()
            .filter_map(|(t, k)| match k {
                EventKind::NoteOn { .. } => Some((t, true)),
                EventKind::NoteOff { .. } => Some((t, false)),
                _ => None,
            })
            .collect();
        let want: Vec<(u64, bool)> = [0.5, 1.5, 2.5, 3.5, 4.5, 5.5]
            .iter()
            .enumerate()
            .map(|(i, &b)| (s(b), i % 2 == 0))
            .collect();
        assert_eq!(ev, want, "chunk {chunk}");
    }
}

//! MIDI expression playback never allocates on the audio thread: lanes, note expressions,
//! loop wraps, locates, snapshot swaps (the per-note state carried over), stop.
//!
//! `assert_no_alloc` aborts the test process on any allocation inside `process` (debug
//! builds, which is how `cargo test` runs).

mod common;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use common::*;
use ether_core::expression::{
    ClipExpressionDesc, ExpressionLaneDesc, NoteExpressionDesc, TrackExpressionDesc,
};
use ether_core::graph::RenderGraphDesc;
use ether_core::protocol::model::{
    Beats, CurveShape, ExpressionKind, NoteExpressionKind, TrackKind,
};
use ether_core::{Engine, TransportControl, create};

#[cfg(debug_assertions)]
#[global_allocator]
static A: AllocDisabler = AllocDisabler;

fn run(engine: &mut Engine, blocks: usize, frames: usize) {
    let mut l = [0.0f32; BLOCK];
    let mut r = [0.0f32; BLOCK];
    for _ in 0..blocks {
        let mut outs: [&mut [f32]; 2] = [&mut l[..frames], &mut r[..frames]];
        assert_no_alloc(|| engine.process(&[], &mut outs, frames));
    }
}

fn desc(rec: ether_core::NodeKey, shift: f64) -> RenderGraphDesc {
    let mut t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[rec]);
    let notes: Vec<(f64, f64, u8)> = (0..32)
        .map(|i| (i as f64 * 0.125, 0.4, 60 + (i % 12) as u8))
        .collect();
    let mut clip = midi_clip(cid(3), shift, 6.0, &notes);
    clip.looping = Some((0.0, 4.0));
    let curve = |n: usize| -> Vec<(f64, f32, CurveShape)> {
        (0..n)
            .map(|i| (i as f64 * 0.01, (i % 7) as f32 / 7.0, CurveShape::Linear))
            .collect()
    };
    t.expression = TrackExpressionDesc {
        clips: vec![ClipExpressionDesc {
            clip: cid(3),
            lanes: vec![
                ExpressionLaneDesc {
                    kind: ExpressionKind::Cc { controller: 74 },
                    points: curve(400),
                },
                ExpressionLaneDesc {
                    kind: ExpressionKind::PitchBend,
                    points: vec![
                        (0.0, -1.0, CurveShape::Curve { tension: 0.3 }),
                        (3.0, 1.0, CurveShape::Step),
                    ],
                },
            ],
            notes: (0..32)
                .map(|i| NoteExpressionDesc {
                    note: i,
                    kind: NoteExpressionKind::Pressure,
                    points: curve(40),
                })
                .collect(),
        }],
        mpe: None,
    };
    t.clips = vec![clip];
    RenderGraphDesc {
        version: 1,
        tracks: vec![master(), t],
        loop_enabled: true,
        loop_start: 0.5,
        loop_end: 3.25,
        ..Default::default()
    }
}

#[test]
fn expression_playback_never_allocates() {
    let mut p = create(config());
    let (rec, _rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    p.handle.publish(desc(rec, 0.0)).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    run(&mut p.engine, 200, BLOCK);
    run(&mut p.engine, 300, 100);
    // Snapshot swap while notes sound (per-note state carried over).
    p.handle.publish(desc(rec, 0.25)).unwrap();
    run(&mut p.engine, 100, BLOCK);
    p.handle
        .transport(TransportControl::Locate {
            position: Beats(1.3),
        })
        .unwrap();
    run(&mut p.engine, 100, 64);
    p.handle.transport(TransportControl::Stop).unwrap();
    run(&mut p.engine, 4, BLOCK);
    p.handle.transport(TransportControl::Play).unwrap();
    run(&mut p.engine, 50, BLOCK);
    p.gc.collect();
}

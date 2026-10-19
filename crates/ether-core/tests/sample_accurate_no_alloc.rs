//! Sample-accurate automation and exact tempo-ramp timing never allocate on the audio
//! thread: node-param events on the grid and at breakpoints (lanes and clip envelopes,
//! precedence handovers), per-sample volume/pan/send ramps, ramped tempo maps with loop
//! wraps, locates and snapshot swaps, blocks that are not grid multiples.
//!
//! `assert_no_alloc` aborts the test process on any allocation inside `process` (debug
//! builds, which is how `cargo test` runs).

mod common;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use common::*;
use ether_core::graph::{AutomationDesc, ParamMapping, RenderGraphDesc, ResolvedTarget};
use ether_core::protocol::devices::ParamScale;
use ether_core::protocol::model::{
    AutomationTarget, Beats, CurveShape, ParamId, TempoCurve, TrackKind,
};
use ether_core::tempo::TempoPointDesc;
use ether_core::{Engine, ParamChange, ParamTarget, TransportControl, create};

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

fn lane(resolved: ResolvedTarget, points: Vec<(f64, f64, CurveShape)>) -> AutomationDesc {
    AutomationDesc {
        target: AutomationTarget::TrackVolume { track: tid(2) },
        resolved,
        points,
        mapping: ParamMapping {
            min: 0.0,
            max: 1.0,
            scale: ParamScale::Linear,
            steps: None,
        },
    }
}

#[test]
fn sample_accurate_automation_never_allocates() {
    let mut p = create(config());
    let (rec, _rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    let dc = p.handle.add_node(Box::new(Dc(0.3))).unwrap();
    let node = ResolvedTarget::Node {
        node: rec,
        param: ParamId(2),
    };
    let points = |shift: f64| {
        vec![
            (0.0 + shift, 0.0, CurveShape::Linear),
            (0.73 + shift, 1.0, CurveShape::Step),
            (1.1 + shift, 0.4, CurveShape::Curve { tension: 0.5 }),
            (2.9 + shift, 0.8, CurveShape::Linear),
        ]
    };
    let mut t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[dc, rec]);
    t.sends = vec![send(sid(1), tid(3), 0.5, true)];
    t.automation = vec![
        lane(node, points(0.0)),
        lane(ResolvedTarget::TrackVolume, points(0.1)),
        lane(ResolvedTarget::TrackPan, points(0.2)),
        lane(ResolvedTarget::Send { send: sid(1) }, points(0.3)),
    ];
    let mut clip = midi_clip(cid(1), 0.61, 1.37, &[(0.1, 0.5, 60)]);
    clip.looping = Some((0.0, 0.5));
    clip.envelopes = vec![
        lane(node, points(0.05)),
        lane(ResolvedTarget::TrackVolume, points(0.0)),
    ];
    t.clips = vec![clip];
    let ret = track(tid(3), TrackKind::Return, Some(tid(1)));
    let desc = |tempo: Vec<TempoPointDesc>| {
        let mut d = RenderGraphDesc {
            version: 1,
            tracks: vec![master(), t.clone(), ret.clone()],
            tempo,
            ..Default::default()
        };
        d.loop_enabled = true;
        d.loop_start = 0.25;
        d.loop_end = 3.3;
        d
    };
    let ramp = vec![
        TempoPointDesc {
            beat: 0.0,
            bpm: 90.0,
            curve: TempoCurve::Linear,
        },
        TempoPointDesc {
            beat: 1.7,
            bpm: 170.0,
            curve: TempoCurve::Linear,
        },
        TempoPointDesc {
            beat: 2.9,
            bpm: 110.0,
            curve: TempoCurve::Step,
        },
    ];
    p.handle.publish(desc(ramp.clone())).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    // Several loop passes, in grid-multiple and odd blocks.
    run(&mut p.engine, 400, BLOCK);
    run(&mut p.engine, 400, 100);
    // Live change of the automated param, locate, a new tempo map (re-anchor), stop/play.
    p.handle
        .set_param(ParamChange {
            target: ParamTarget::Node {
                node: rec,
                param: ParamId(2),
            },
            value: 0.5,
        })
        .unwrap();
    p.handle
        .transport(TransportControl::Locate {
            position: Beats(1.9),
        })
        .unwrap();
    run(&mut p.engine, 100, 64);
    p.handle.publish(desc(vec![])).unwrap();
    run(&mut p.engine, 100, BLOCK);
    p.handle.publish(desc(ramp)).unwrap();
    run(&mut p.engine, 100, 37);
    p.handle.transport(TransportControl::Stop).unwrap();
    run(&mut p.engine, 50, BLOCK);
    p.handle.transport(TransportControl::Play).unwrap();
    run(&mut p.engine, 100, BLOCK);
    while p.gc.collect() > 0 {}
}

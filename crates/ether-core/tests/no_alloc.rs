//! Proves `Engine::process` never allocates or frees, across every feature the engine
//! exercises: snapshot swaps, node insert/remove, sources, notes, loops, locate, tempo
//! ramps, sends, groups, solo/mute, automation, live params, meters and playhead.
//!
//! `assert_no_alloc` aborts the test process on any allocation inside `process` (debug
//! builds, which is how `cargo test` runs).

mod common;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use common::*;
use ether_core::graph::{AutomationDesc, ClipContentDesc, ClipDesc, ParamMapping, ResolvedTarget};
use ether_core::protocol::devices::ParamScale;
use ether_core::protocol::model::{
    AutomationTarget, BeatRange, Beats, CurveShape, MediaId, ParamId, TempoCurve, TrackKind, Ulid,
};
use ether_core::tempo::TempoPointDesc;
use ether_core::{
    Engine, EngineOutputs, ParamChange, ParamTarget, RenderGraphDesc, TransportControl, create,
};

#[cfg(debug_assertions)]
#[global_allocator]
static A: AllocDisabler = AllocDisabler;

fn run(engine: &mut Engine, blocks: usize, frames: usize, out_l: &mut [f32], out_r: &mut [f32]) {
    let input = [0.1f32; BLOCK];
    let inputs: [&[f32]; 2] = [&input, &input];
    for _ in 0..blocks {
        let mut outs: [&mut [f32]; 2] = [&mut out_l[..frames], &mut out_r[..frames]];
        assert_no_alloc(|| engine.process(&inputs, &mut outs, frames));
    }
}

#[test]
fn process_never_allocates() {
    let mut p = create(config());
    let media = MediaId(Ulid(7));
    p.handle.add_source(media, mem_source(200_000)).unwrap();

    let (rec, _rx) = Recorder::new();
    let rec = p.handle.add_node(Box::new(rec)).unwrap();
    let dc = p.handle.add_node(Box::new(Dc(0.2))).unwrap();
    let delay = p.handle.add_node(Box::new(Delay::new(300))).unwrap();
    let delay2 = p.handle.add_node(Box::new(Delay::new(50))).unwrap();

    let group = track(tid(10), TrackKind::Group, Some(tid(1)));
    let mut midi = with_chain(track(tid(11), TrackKind::Midi, Some(tid(10))), &[rec, dc]);
    midi.group = Some(tid(10));
    let mut clip = midi_clip(
        cid(1),
        0.0,
        16.0,
        &[(0.0, 0.5, 60), (0.5, 0.25, 64), (1.75, 3.0, 67)],
    );
    clip.looping = Some((0.0, 2.0));
    clip.envelopes = vec![AutomationDesc {
        target: AutomationTarget::TrackPan { track: tid(11) },
        resolved: ResolvedTarget::TrackPan,
        points: vec![
            (0.0, 0.0, CurveShape::Curve { tension: 0.5 }),
            (2.0, 1.0, CurveShape::Linear),
        ],
        mapping: ParamMapping {
            min: -1.0,
            max: 1.0,
            scale: ParamScale::Linear,
            steps: None,
        },
    }];
    midi.clips = vec![clip];
    midi.sends = vec![send(sid(1), tid(20), 0.5, true)];
    midi.automation = vec![
        AutomationDesc {
            target: AutomationTarget::TrackVolume { track: tid(11) },
            resolved: ResolvedTarget::TrackVolume,
            points: vec![(0.0, 0.2, CurveShape::Linear), (8.0, 0.9, CurveShape::Step)],
            mapping: ParamMapping {
                min: -70.0,
                max: 6.0,
                scale: ParamScale::Fader,
                steps: None,
            },
        },
        AutomationDesc {
            target: AutomationTarget::TrackPan { track: tid(11) },
            resolved: ResolvedTarget::Node {
                node: rec,
                param: ParamId(1),
            },
            points: vec![
                (0.0, 0.0, CurveShape::Linear),
                (4.0, 1.0, CurveShape::Linear),
            ],
            mapping: ParamMapping {
                min: 20.0,
                max: 20_000.0,
                scale: ParamScale::Log,
                steps: None,
            },
        },
    ];

    let mut audio = with_chain(track(tid(12), TrackKind::Audio, Some(tid(1))), &[delay]);
    audio.audio_input = Some((0, 1));
    audio.monitor = true;
    audio.sends = vec![send(sid(2), tid(20), 0.3, false)];
    audio.clips = vec![ClipDesc {
        id: cid(2),
        start: 1.0,
        length: 6.0,
        offset: 0.5,
        looping: Some((0.0, 3.0)),
        muted: false,
        content: ClipContentDesc::Audio {
            media,
            gain: 0.8,
            transpose: 0.0,
            fade_in: 0.25,
            fade_out: 0.25,
            warp: Some(ether_core::graph::WarpDesc {
                mode: ether_core::protocol::model::WarpMode::Repitch,
                markers: vec![(0.0, 0.0), (2.0, 1.3), (6.0, 2.9)],
            }),
        },
        envelopes: vec![],
    }];
    let ret = with_chain(track(tid(20), TrackKind::Return, Some(tid(1))), &[delay2]);

    let mut desc = RenderGraphDesc {
        version: 1,
        tempo: vec![
            TempoPointDesc {
                beat: 0.0,
                bpm: 128.0,
                curve: TempoCurve::Linear,
            },
            TempoPointDesc {
                beat: 3.0,
                bpm: 90.0,
                curve: TempoCurve::Step,
            },
        ],
        loop_enabled: true,
        loop_start: 0.0,
        loop_end: 6.0,
        tracks: vec![master(), group, midi, audio, ret],
        ..Default::default()
    };
    p.handle.publish(desc.clone()).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();

    let mut l = vec![0.0; BLOCK];
    let mut r = vec![0.0; BLOCK];
    run(&mut p.engine, 200, BLOCK, &mut l, &mut r);
    run(&mut p.engine, 300, 37, &mut l, &mut r);

    // Live changes, swaps, transport while running.
    for i in 0..20 {
        p.handle
            .set_param(ParamChange {
                target: ParamTarget::TrackVolume { track: tid(12) },
                value: 0.1 * i as f64,
            })
            .unwrap();
        p.handle
            .set_param(ParamChange {
                target: ParamTarget::SendLevel { send: sid(2) },
                value: 0.5,
            })
            .unwrap();
        p.handle
            .set_param(ParamChange {
                target: ParamTarget::TrackMute { track: tid(10) },
                value: (i % 2) as f64,
            })
            .unwrap();
        p.handle
            .set_param(ParamChange {
                target: ParamTarget::Node {
                    node: dc,
                    param: ParamId(0),
                },
                value: 1.0,
            })
            .unwrap();
        desc.version += 1;
        desc.tracks[2].solo = i % 3 == 0;
        desc.tracks[3].volume = 0.5;
        p.handle.publish(desc.clone()).unwrap();
        if i % 5 == 0 {
            p.handle
                .transport(TransportControl::Locate {
                    position: Beats(i as f64 * 0.37),
                })
                .unwrap();
        }
        if i == 7 {
            p.handle
                .transport(TransportControl::SetLoop {
                    enabled: true,
                    region: BeatRange {
                        start: Beats(1.0),
                        end: Beats(1.5),
                    },
                })
                .unwrap();
            p.handle.transport(TransportControl::Stop).unwrap();
        }
        if i == 9 {
            p.handle.transport(TransportControl::Play).unwrap();
            p.handle.add_source(media, mem_source(1000)).unwrap();
        }
        if i == 12 {
            let extra = p.handle.add_node(Box::new(Dc(0.1))).unwrap();
            p.handle.remove_node(extra).unwrap();
            p.handle.remove_source(media).unwrap();
        }
        run(&mut p.engine, 40, BLOCK, &mut l, &mut r);
        // GC and polling on "other threads" in between.
        p.gc.collect();
        let mut out = EngineOutputs::default();
        p.handle.poll(&mut out);
    }
    assert_eq!(p.engine.leaked(), 0);
    assert!(l.iter().chain(r.iter()).all(|v| v.is_finite()));
}

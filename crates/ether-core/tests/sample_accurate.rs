//! Sample-accurate automation and exact tempo ramps (CONTRACTS.md §12.7, node
//! `sample-accurate-automation`):
//! - an offline render with automation (lanes, clip envelopes, volume/pan/send/node params)
//!   and tempo ramps is the same (±1e-6) with blocks of 64 and 512; with neither it is
//!   bit-identical;
//! - a `Step` point lands on the exact sample, also on a tempo ramp (null test);
//! - positions on a 10-minute tempo ramp match the closed form within 1e-9 beats, at every
//!   sub-block start and at every automation grid point.

mod common;

use common::*;
use ether_core::graph::{
    AutomationDesc, ClipContentDesc, ClipDesc, ParamMapping, RenderGraphDesc, ResolvedTarget,
};
use ether_core::protocol::devices::ParamScale;
use ether_core::protocol::model::{
    AutomationTarget, CurveShape, MediaId, ParamId, TempoCurve, TrackKind, Ulid,
};
use ether_core::tempo::TempoPointDesc;
use ether_core::{
    AudioBuffers, EngineConfig, EventKind, Node, NodeKey, PrepareConfig, ProcessContext,
    ProcessStatus, TransportControl, create,
};

/// Constant generator whose level is param 0, applied at each event's offset (like the
/// built-in devices' `split_at_events`): the output shows exactly where a change lands.
struct Level(f32);

impl Node for Level {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let mut pos = 0;
        let fill = |level: f32, from: usize, to: usize, audio: &mut AudioBuffers<'_, '_>| {
            for ch in audio.outputs.iter_mut() {
                ch[from..to].fill(level);
            }
        };
        for e in ctx.events {
            if let EventKind::Param { param, value } = e.kind
                && param == ParamId(0)
            {
                let o = (e.offset as usize).min(ctx.frames);
                fill(self.0, pos, o, audio);
                pos = o;
                self.0 = value as f32;
            }
        }
        fill(self.0, pos, ctx.frames, audio);
        ProcessStatus::Continue
    }
    fn channels(&self) -> (u16, u16) {
        (0, 2)
    }
}

fn cfg() -> EngineConfig {
    EngineConfig {
        max_events_per_block: 1024,
        ..config()
    }
}

fn linear(min: f64, max: f64) -> ParamMapping {
    ParamMapping {
        min,
        max,
        scale: ParamScale::Linear,
        steps: None,
    }
}

fn lane(
    resolved: ResolvedTarget,
    points: Vec<(f64, f64, CurveShape)>,
    mapping: ParamMapping,
) -> AutomationDesc {
    AutomationDesc {
        // Diagnostics only: the engine drives `resolved`.
        target: AutomationTarget::TrackVolume { track: tid(2) },
        resolved,
        points,
        mapping,
    }
}

fn tp(beat: f64, bpm: f64, curve: TempoCurve) -> TempoPointDesc {
    TempoPointDesc { beat, bpm, curve }
}

fn ramps() -> Vec<TempoPointDesc> {
    vec![
        tp(0.0, 100.0, TempoCurve::Linear),
        tp(6.0, 173.0, TempoCurve::Step),
        tp(9.3, 91.0, TempoCurve::Linear),
        tp(15.0, 140.0, TempoCurve::Linear),
        tp(19.0, 77.0, TempoCurve::Step),
    ]
}

const MEDIA: MediaId = MediaId(Ulid(42));

fn audio_clip(start: f64, length: f64) -> ClipDesc {
    ClipDesc {
        id: cid(90),
        start,
        length,
        offset: 0.25,
        looping: Some((0.0, 1.5)),
        muted: false,
        content: ClipContentDesc::Audio {
            media: MEDIA,
            gain: 0.7,
            transpose: 0.0,
            fade_in: 0.1,
            fade_out: 0.3,
            fade_in_curve: Default::default(),
            fade_out_curve: Default::default(),
            reversed: false,
            warp: None,
        },
        envelopes: vec![],
    }
}

/// A project using everything that is automated per sample: node-param lane with linear,
/// curved and step segments; volume, pan and send-level lanes; a clip envelope taking a
/// node param over from its lane mid-block and handing it back; a clip volume envelope on
/// a looped audio clip; notes. `automate = false` leaves the same project unautomated.
fn project(nodes: [NodeKey; 3], tempo: Vec<TempoPointDesc>, automate: bool) -> RenderGraphDesc {
    let [a, b, c] = nodes;
    let mut ta = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[a]);
    ta.sends = vec![send(sid(1), tid(9), 0.5, false)];
    let mut tb = with_chain(track(tid(3), TrackKind::Midi, Some(tid(1))), &[b]);
    tb.clips = vec![midi_clip(cid(1), 2.37, 3.11, &[(0.5, 1.0, 60)])];
    let mut tc = with_chain(track(tid(4), TrackKind::Audio, Some(tid(1))), &[c]);
    tc.clips = vec![audio_clip(1.13, 12.4)];
    let ret = track(tid(9), TrackKind::Return, Some(tid(1)));
    if automate {
        let node = |node| ResolvedTarget::Node {
            node,
            param: ParamId(0),
        };
        ta.automation = vec![
            lane(
                node(a),
                vec![
                    (0.0, 0.1, CurveShape::Linear),
                    (3.3, 0.9, CurveShape::Curve { tension: 0.6 }),
                    (7.77, 0.2, CurveShape::Step),
                    (11.123, 0.6, CurveShape::Linear),
                    (18.0, 0.3, CurveShape::Linear),
                ],
                linear(0.0, 1.0),
            ),
            lane(
                ResolvedTarget::TrackVolume,
                vec![
                    (0.5, 0.2, CurveShape::Linear),
                    (5.01, 1.0, CurveShape::Step),
                    (8.4, 0.4, CurveShape::Curve { tension: -0.4 }),
                    (16.0, 0.9, CurveShape::Linear),
                ],
                linear(0.0, 1.0),
            ),
            lane(
                ResolvedTarget::TrackPan,
                vec![
                    (0.0, 0.0, CurveShape::Linear),
                    (10.0, 1.0, CurveShape::Step),
                    (12.2, 0.3, CurveShape::Linear),
                ],
                linear(-1.0, 1.0),
            ),
            lane(
                ResolvedTarget::Send { send: sid(1) },
                vec![
                    (1.0, 1.0, CurveShape::Linear),
                    (9.9, 0.0, CurveShape::Linear),
                    (14.7, 0.8, CurveShape::Step),
                ],
                linear(0.0, 1.0),
            ),
        ];
        tb.automation = vec![lane(
            node(b),
            vec![
                (0.0, 0.3, CurveShape::Linear),
                (20.0, 0.9, CurveShape::Linear),
            ],
            linear(0.0, 1.0),
        )];
        tb.clips[0].envelopes = vec![lane(
            node(b),
            vec![
                (0.0, 1.0, CurveShape::Linear),
                (1.7, 0.1, CurveShape::Step),
                (2.2, 0.5, CurveShape::Linear),
            ],
            linear(0.0, 1.0),
        )];
        tc.clips[0].envelopes = vec![lane(
            ResolvedTarget::TrackVolume,
            vec![
                (0.0, 0.0, CurveShape::Linear),
                (0.9, 1.0, CurveShape::Linear),
                (1.45, 0.3, CurveShape::Linear),
            ],
            linear(0.0, 1.0),
        )];
    }
    RenderGraphDesc {
        version: 1,
        tracks: vec![master(), ta, tb, tc, ret],
        tempo,
        ..Default::default()
    }
}

/// Render 11 s of [`project`] in blocks of `block` (interleaved). `audio = false` leaves
/// the audio clip's media unregistered (it plays silence).
fn render_project(
    tempo: Vec<TempoPointDesc>,
    automate: bool,
    audio: bool,
    block: usize,
) -> Vec<f32> {
    let mut p = create(cfg());
    if audio {
        p.handle.add_source(MEDIA, mem_source(400_000)).unwrap();
    }
    let a = p.handle.add_node(Box::new(Level(0.5))).unwrap();
    let b = p.handle.add_node(Box::new(Level(0.5))).unwrap();
    let c = p.handle.add_node(Box::new(Pass)).unwrap();
    p.handle
        .publish(project([a, b, c], tempo, automate))
        .unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    let (l, r) = render(&mut p.engine, 11 * 48_000, block);
    l.into_iter().zip(r).flat_map(|(l, r)| [l, r]).collect()
}

/// Audio pass-through (the audio clip's track).
struct Pass;

impl Node for Pass {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        _: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        audio.pass_through();
        ProcessStatus::Continue
    }
}

fn max_diff(a: &[f32], b: &[f32]) -> (f32, usize) {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .enumerate()
        .map(|(i, (x, y))| ((x - y).abs(), i))
        .fold((0.0, 0), |m, d| if d.0 > m.0 { d } else { m })
}

#[test]
fn block_size_independent_with_automation_and_tempo_ramps() {
    let small = render_project(ramps(), true, true, 64);
    let large = render_project(ramps(), true, true, 512);
    assert!(small.iter().any(|s| *s != 0.0));
    let (d, i) = max_diff(&small, &large);
    assert!(d <= 1e-6, "64 vs 512: {d} at frame {}", i / 2);
    // Host blocks that are not grid multiples: mixer ramps are re-targeted mid-interval,
    // which only changes f32 rounding of the ramp.
    let odd = render_project(ramps(), true, true, 100);
    let (d, i) = max_diff(&small, &odd);
    assert!(d <= 1e-5, "64 vs 100: {d} at frame {}", i / 2);
}

#[test]
fn block_size_independent_with_automation_at_constant_tempo() {
    let tempo = vec![
        tp(0.0, 128.0, TempoCurve::Step),
        tp(7.5, 97.0, TempoCurve::Step),
    ];
    let small = render_project(tempo.clone(), true, true, 64);
    let large = render_project(tempo, true, true, 512);
    let (d, i) = max_diff(&small, &large);
    assert!(d <= 1e-6, "{d} at frame {}", i / 2);
}

#[test]
fn unautomated_constant_tempo_is_bit_identical() {
    let tempo = vec![
        tp(0.0, 128.0, TempoCurve::Step),
        tp(7.5, 97.0, TempoCurve::Step),
    ];
    let small = render_project(tempo.clone(), false, false, 64);
    let large = render_project(tempo.clone(), false, false, 512);
    assert!(small.iter().any(|s| *s != 0.0));
    assert!(small == large, "{:?}", max_diff(&small, &large));
    // Audio clips keep the v0.1 resampling math (beats linear within each sub-block), so
    // they differ by float rounding only; a clip boundary on a sample boundary lands on
    // the same sample for every block size.
    let small = render_project(tempo.clone(), false, true, 64);
    let large = render_project(tempo, false, true, 512);
    let (d, i) = max_diff(&small, &large);
    assert!(d <= 2.5e-7, "{d} at frame {}", i / 2);
}

/// First sample whose closed-form beat is at or after `beat`, on a single ramp from
/// `bpm0` to `bpm1` over `len` beats starting at beat 0 and sample 0.
fn ramp_sample(bpm0: f64, bpm1: f64, len: f64, beat: f64) -> u64 {
    let slope = (bpm1 - bpm0) / len;
    let seconds = 60.0 / slope * (beat * slope / bpm0).ln_1p();
    (seconds * f64::from(SR)).ceil() as u64
}

/// Closed-form beat of `seconds` on that ramp.
fn ramp_beat(bpm0: f64, bpm1: f64, len: f64, seconds: f64) -> f64 {
    let slope = (bpm1 - bpm0) / len;
    bpm0 / slope * (seconds * slope / 60.0).exp_m1()
}

#[test]
fn step_lands_on_the_exact_sample() {
    // Constant tempo and a ramp; steps between samples and exactly on one.
    for (tempo, beat) in [
        (vec![tp(0.0, 120.0, TempoCurve::Step)], 1.234_567),
        (vec![tp(0.0, 120.0, TempoCurve::Step)], 2.0),
        (
            vec![
                tp(0.0, 70.0, TempoCurve::Linear),
                tp(16.0, 190.0, TempoCurve::Step),
            ],
            3.217_59,
        ),
    ] {
        let expected = if tempo.len() == 1 {
            (beat * 24_000.0f64 - 1e-6).ceil() as usize
        } else {
            ramp_sample(70.0, 190.0, 16.0, beat) as usize
        };
        for block in [64, 512, 100] {
            let mut p = create(cfg());
            let lvl = p.handle.add_node(Box::new(Level(0.0))).unwrap();
            let (rec, mut rx) = Recorder::new();
            let rec = p.handle.add_node(Box::new(rec)).unwrap();
            let mut t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[lvl, rec]);
            let points = vec![
                (0.0, 0.25, CurveShape::Step),
                (beat, 0.75, CurveShape::Step),
            ];
            let node = |node, param| ResolvedTarget::Node { node, param };
            t.automation = vec![
                lane(node(lvl, ParamId(0)), points.clone(), linear(0.0, 1.0)),
                lane(node(rec, ParamId(5)), points, linear(0.0, 1.0)),
            ];
            p.handle
                .publish(RenderGraphDesc {
                    version: 1,
                    tracks: vec![master(), t],
                    tempo: tempo.clone(),
                    ..Default::default()
                })
                .unwrap();
            p.handle.transport(TransportControl::Play).unwrap();
            let (l, _) = render(&mut p.engine, expected + 2_000, block);
            // Null test: the level is 0.25 up to the sample before, 0.75 from it on (after
            // the tracks' initial 10 ms un-mute ramp).
            assert!(
                l[1_000..expected].iter().all(|v| *v == 0.25),
                "block {block}: early change"
            );
            assert!(
                l[expected..].iter().all(|v| *v == 0.75),
                "block {block}: late change ({} at {expected})",
                l[expected]
            );
            // The node sees exactly two events: the start value and the step, on its sample.
            let params: Vec<_> = events(&drain(&mut rx))
                .into_iter()
                .filter(|(_, k)| {
                    matches!(
                        k,
                        EventKind::Param {
                            param: ParamId(5),
                            ..
                        }
                    )
                })
                .collect();
            let step = |value| EventKind::Param {
                param: ParamId(5),
                value,
            };
            assert_eq!(
                params,
                vec![(0, step(0.25)), (expected as u64, step(0.75))],
                "block {block}"
            );
        }
    }
}

#[test]
fn ten_minute_tempo_ramp_matches_the_closed_form() {
    // 90 -> 200 BPM over 1400 beats: ~610 s.
    let (bpm0, bpm1, len) = (90.0, 200.0, 1400.0);
    let total = 600 * SR as u64;
    assert!(ramp_sample(bpm0, bpm1, len, len) > total);
    let sr = f64::from(SR);
    let mut positions = Vec::new();
    for block in [64, 512] {
        let mut p = create(cfg());
        let (rec, mut rx) = Recorder::new();
        let rec = p.handle.add_node(Box::new(rec)).unwrap();
        let mut t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[rec]);
        // A lane whose plain value is the beat: grid events carry the beat of their sample.
        t.automation = vec![lane(
            ResolvedTarget::Node {
                node: rec,
                param: ParamId(1),
            },
            vec![
                (0.0, 0.0, CurveShape::Linear),
                (len, 1.0, CurveShape::Linear),
            ],
            linear(0.0, len),
        )];
        p.handle
            .publish(RenderGraphDesc {
                version: 1,
                tracks: vec![master(), t],
                tempo: vec![
                    tp(0.0, bpm0, TempoCurve::Linear),
                    tp(len, bpm1, TempoCurve::Step),
                ],
                ..Default::default()
            })
            .unwrap();
        p.handle.transport(TransportControl::Play).unwrap();
        let (mut blocks, mut worst_block, mut worst_event, mut n_events) =
            (vec![], 0.0f64, 0.0f64, 0);
        let chunk = 94 * 512; // ~1 s, a multiple of both block sizes
        let mut done = 0u64;
        while done < total {
            render(&mut p.engine, chunk, block);
            done += chunk as u64;
            for s in drain(&mut rx) {
                match s {
                    Seen::Block(at, _, pos, _, _) => {
                        let err = (pos - ramp_beat(bpm0, bpm1, len, at as f64 / sr)).abs();
                        worst_block = worst_block.max(err);
                        if at % 4096 == 0 {
                            blocks.push((at, pos));
                        }
                    }
                    Seen::Event(
                        at,
                        EventKind::Param {
                            param: ParamId(1),
                            value,
                        },
                    ) => {
                        let err = (value - ramp_beat(bpm0, bpm1, len, at as f64 / sr)).abs();
                        worst_event = worst_event.max(err);
                        n_events += 1;
                        assert_eq!(at % 32, 0, "grid events only (no breakpoint inside)");
                    }
                    _ => {}
                }
            }
        }
        assert!(
            worst_block < 1e-9,
            "block {block}: sub-block starts off by {worst_block}"
        );
        assert!(
            worst_event < 1e-9,
            "block {block}: grid values off by {worst_event}"
        );
        assert!(n_events as u64 >= total / 32 - 1, "{n_events}");
        positions.push(blocks);
    }
    // Same positions for both block sizes, bit for bit.
    assert_eq!(positions[0], positions[1]);
}

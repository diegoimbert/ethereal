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
    // Mono-output (2 in, 1 out) node, MIDI-emitting node, a bypassed device, and a node
    // that overflows the event buffers.
    let mono = p.handle.add_node(Box::new(MonoSum)).unwrap();
    let emitter = p
        .handle
        .add_node(Box::new(NoteEmitter { per_block: 4 }))
        .unwrap();
    let bypassed = p.handle.add_node(Box::new(Delay::new(32))).unwrap();
    let flood = p
        .handle
        .add_node(Box::new(NoteEmitter { per_block: 2000 }))
        .unwrap();
    let flood_sink = p.handle.add_node(Box::new(Dc(0.01))).unwrap();

    let mut group = with_chain(track(tid(10), TrackKind::Group, Some(tid(1))), &[bypassed]);
    group.chain[0].enabled = false;
    let mut midi = with_chain(
        track(tid(11), TrackKind::Midi, Some(tid(10))),
        &[emitter, rec, dc],
    );
    let overflow = with_chain(
        track(tid(13), TrackKind::Midi, Some(tid(1))),
        &[flood, flood_sink],
    );
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
    audio.automation = vec![AutomationDesc {
        target: AutomationTarget::SendLevel { send: sid(2) },
        resolved: ResolvedTarget::Send { send: sid(2) },
        points: vec![
            (0.0, 0.1, CurveShape::Linear),
            (5.0, 0.9, CurveShape::Linear),
        ],
        mapping: ParamMapping {
            min: -70.0,
            max: 6.0,
            scale: ParamScale::Fader,
            steps: None,
        },
    }];
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
            fade_in_curve: Default::default(),
            fade_out_curve: Default::default(),
            reversed: false,
            warp: Some(ether_core::graph::WarpDesc {
                mode: ether_core::protocol::model::WarpMode::Repitch,
                markers: vec![(0.0, 0.0), (2.0, 1.3), (6.0, 2.9)],
            }),
        },
        envelopes: vec![],
    }];
    let ret = with_chain(
        track(tid(20), TrackKind::Return, Some(tid(1))),
        &[delay2, mono],
    );

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
        tracks: vec![master(), group, midi, audio, ret, overflow],
        ..Default::default()
    };
    p.handle.publish(desc.clone()).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();

    let mut l = vec![0.0; BLOCK];
    let mut r = vec![0.0; BLOCK];
    run(&mut p.engine, 200, BLOCK, &mut l, &mut r);
    run(&mut p.engine, 300, 37, &mut l, &mut r);

    // Live changes, swaps, transport while running.
    let mut saw_overflow = false;
    for i in 0..20 {
        p.handle
            .set_param(ParamChange {
                target: ParamTarget::TrackPan { track: tid(12) },
                value: 0.1 * i as f64 - 1.0,
            })
            .unwrap();
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
        saw_overflow |= out.event_overflow;
    }
    assert!(
        saw_overflow,
        "the flooding node must overflow and be reported"
    );
    assert_eq!(p.engine.leaked(), 0);
    assert!(l.iter().chain(r.iter()).all(|v| v.is_finite()));
}

// ─── Roadmap v2 hooks (base-24) ─────────────────────────────────────────────────────────

/// Adds its sidechain into its output (no allocation).
struct ScSum;

impl ether_core::Node for ScSum {
    fn prepare(&mut self, _: &ether_core::PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        _: &mut ether_core::ProcessContext<'_>,
        audio: &mut ether_core::AudioBuffers<'_, '_>,
    ) -> ether_core::ProcessStatus {
        audio.pass_through();
        ether_core::ProcessStatus::Continue
    }
    fn sidechain_inputs(&self) -> u16 {
        2
    }
    fn process_sidechain(
        &mut self,
        ctx: &mut ether_core::ProcessContext<'_>,
        audio: &mut ether_core::AudioBuffers<'_, '_>,
        sidechain: &[&[f32]],
    ) -> ether_core::ProcessStatus {
        self.process(ctx, audio);
        for (out, sc) in audio.outputs.iter_mut().zip(sidechain) {
            for (o, s) in out.iter_mut().zip(sc.iter()) {
                *o += s * 0.1;
            }
        }
        ether_core::ProcessStatus::Continue
    }
}

/// Passes its input through (a drum rack stand-in).
struct Pass;

impl ether_core::Node for Pass {
    fn prepare(&mut self, _: &ether_core::PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        _: &mut ether_core::ProcessContext<'_>,
        audio: &mut ether_core::AudioBuffers<'_, '_>,
    ) -> ether_core::ProcessStatus {
        audio.pass_through();
        ether_core::ProcessStatus::Continue
    }
}

#[test]
fn roadmap_hooks_never_allocate() {
    use ether_core::graph::{ChainEntry, PadDesc, RackDesc};
    use ether_core::preview::PreviewControl;
    use ether_core::protocol::model::DrumPadId;

    let mut p = create(config());
    let key = |p: &mut ether_core::EngineParts, n: Box<dyn ether_core::Node>| {
        p.handle.add_node(n).unwrap()
    };
    // Sidechain, both directions: S (latency 100) feeds early (0) and late (250) consumers.
    let s_dc = key(&mut p, Box::new(Dc(0.3)));
    let s_del = key(&mut p, Box::new(Delay::new(100)));
    let early_dc = key(&mut p, Box::new(Dc(0.1)));
    let early_sc = key(&mut p, Box::new(ScSum));
    let late_dc = key(&mut p, Box::new(Dc(0.1)));
    let late_del = key(&mut p, Box::new(Delay::new(250)));
    let late_sc = key(&mut p, Box::new(ScSum));
    // A rack with two pads fed by notes (key 60 = pad A), with params and automation.
    let emitter = key(&mut p, Box::new(NoteEmitter { per_block: 4 }));
    let rack = key(&mut p, Box::new(Pass));
    let (rec, _rx) = Recorder::new();
    let pad_a = key(&mut p, Box::new(rec));
    let pad_a_del = key(&mut p, Box::new(Delay::new(20)));
    let pad_b = key(&mut p, Box::new(Dc(0.05)));

    let source = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[s_dc, s_del]);
    let mut early = with_chain(
        track(tid(3), TrackKind::Midi, Some(tid(1))),
        &[early_dc, early_sc],
    );
    early.chain[1].sidechain = Some(tid(2));
    let mut late = with_chain(
        track(tid(4), TrackKind::Midi, Some(tid(1))),
        &[late_dc, late_del, late_sc],
    );
    late.chain[2].sidechain = Some(tid(2));
    let entry = |node| ChainEntry {
        node,
        enabled: true,
        sidechain: None,
    };
    let mut drums = with_chain(
        track(tid(5), TrackKind::Midi, Some(tid(1))),
        &[emitter, rack],
    );
    drums.racks = vec![RackDesc {
        rack,
        pads: vec![
            PadDesc {
                pad: DrumPadId(Ulid(1)),
                note: 60,
                choke_group: Some(1),
                chain: vec![entry(pad_a), entry(pad_a_del)],
                volume: 0.8,
                pan: -0.5,
                mute: false,
            },
            PadDesc {
                pad: DrumPadId(Ulid(2)),
                note: 62,
                choke_group: Some(1),
                chain: vec![entry(pad_b)],
                volume: 1.0,
                pan: 0.0,
                mute: false,
            },
        ],
    }];
    drums.automation = vec![AutomationDesc {
        target: AutomationTarget::DeviceParam {
            device: ether_core::protocol::model::DeviceId(Ulid(3)),
            param: ParamId(1),
        },
        resolved: ResolvedTarget::Node {
            node: pad_a,
            param: ParamId(1),
        },
        points: vec![
            (0.0, 0.0, CurveShape::Linear),
            (8.0, 1.0, CurveShape::Linear),
        ],
        mapping: ParamMapping {
            min: 0.0,
            max: 1.0,
            scale: ParamScale::Linear,
            steps: None,
        },
    }];
    p.handle
        .publish(RenderGraphDesc {
            version: 1,
            metronome: true,
            tracks: vec![master(), source, early, late, drums],
            ..Default::default()
        })
        .unwrap();
    p.handle.transport(TransportControl::Play).unwrap();

    let mut l = vec![0.0; BLOCK];
    let mut r = vec![0.0; BLOCK];
    run(&mut p.engine, 4, BLOCK, &mut l, &mut r);

    // Live pad params.
    for v in [0.2, 0.4] {
        p.handle
            .set_param(ParamChange {
                target: ParamTarget::Node {
                    node: pad_a,
                    param: ParamId(2),
                },
                value: v,
            })
            .unwrap();
        run(&mut p.engine, 2, BLOCK, &mut l, &mut r);
    }

    // Preview: play, replace, stop, then a short one that ends by itself.
    p.handle
        .preview(PreviewControl::Play {
            id: 1,
            source: mem_source(10 * BLOCK),
            gain: 0.5,
        })
        .unwrap();
    run(&mut p.engine, 2, BLOCK, &mut l, &mut r);
    p.handle
        .preview(PreviewControl::Play {
            id: 2,
            source: mem_source(10 * BLOCK),
            gain: 0.5,
        })
        .unwrap();
    run(&mut p.engine, 2, BLOCK, &mut l, &mut r);
    p.handle.preview(PreviewControl::Stop).unwrap();
    run(&mut p.engine, 1, BLOCK, &mut l, &mut r);
    p.handle
        .preview(PreviewControl::Play {
            id: 3,
            source: mem_source(BLOCK / 2),
            gain: 0.5,
        })
        .unwrap();
    run(&mut p.engine, 3, BLOCK, &mut l, &mut r);
    // Locate (resets pad nodes, AllNotesOff to pad chains) while everything runs.
    p.handle
        .transport(TransportControl::Locate {
            position: Beats(2.0),
        })
        .unwrap();
    run(&mut p.engine, 3, BLOCK, &mut l, &mut r);

    let mut out = EngineOutputs::default();
    p.handle.poll(&mut out);
    assert_eq!(out.preview_ended, Some(3));
    p.gc.collect();
}

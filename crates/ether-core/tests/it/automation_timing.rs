//! Automation timing against the audio it affects (CONTRACTS.md §12.7, issue #270): a step
//! 0 → 1 lands on the exact sample of the audio that was at the step's timeline position,
//! through plugin delay compensation, groups, the fader, sends, VCAs, device switches,
//! anticipative processing and every block size. Never early, never late.

use crate::common;

use common::*;
use ether_core::graph::{AutomationDesc, ParamMapping, RenderGraphDesc, ResolvedTarget};
use ether_core::parallel::{JobGraph, ParallelExecutor};
use ether_core::protocol::devices::ParamScale;
use ether_core::protocol::model::{AutomationTarget, CurveShape, ParamId, TrackKind};
use ether_core::{
    AudioBuffers, EngineConfig, EventKind, Node, NodeKey, PrepareConfig, ProcessContext,
    ProcessStatus, TransportControl, create,
};

/// Multiplies its input by param 0, applied at each event's offset (no smoothing): the
/// output shows the exact sample a change lands on.
struct Gain(f32);

impl Node for Gain {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let mut pos = 0;
        let run = |g: f32, from: usize, to: usize, audio: &mut AudioBuffers<'_, '_>| {
            for ch in 0..2 {
                for i in from..to {
                    audio.outputs[ch][i] = audio.inputs[ch][i] * g;
                }
            }
        };
        for e in ctx.events {
            if let EventKind::Param { param, value } = e.kind
                && param == ParamId(0)
            {
                let o = (e.offset as usize).min(ctx.frames);
                run(self.0, pos, o, audio);
                pos = o;
                self.0 = value as f32;
            }
        }
        run(self.0, pos, ctx.frames, audio);
        ProcessStatus::Continue
    }
}

/// 120 BPM at 48 kHz: 24 000 samples per beat. The step is at beat 1.5 + a third of a
/// sample's worth, between samples, so it lands on sample 36 001.
const STEP_BEAT: f64 = 1.5 + 0.7 / 24_000.0;
const STEP: usize = 36_001;

fn cfg(anticipative: bool) -> EngineConfig {
    EngineConfig {
        max_events_per_block: 1024,
        anticipative,
        ..config()
    }
}

fn unit() -> ParamMapping {
    ParamMapping {
        min: 0.0,
        max: 1.0,
        scale: ParamScale::Linear,
        steps: None,
    }
}

fn step_lane(resolved: ResolvedTarget) -> AutomationDesc {
    AutomationDesc {
        target: AutomationTarget::TrackVolume { track: tid(2) },
        resolved,
        points: vec![
            (0.0, 0.0, CurveShape::Step),
            (STEP_BEAT, 1.0, CurveShape::Step),
        ],
        mapping: unit(),
    }
}

fn gain_of(node: NodeKey) -> ResolvedTarget {
    ResolvedTarget::Node {
        node,
        param: ParamId(0),
    }
}

/// Runs the graph in order on the calling thread, but says it has workers (so the engine
/// anticipates).
struct InOrder;

impl ParallelExecutor for InOrder {
    fn execute(&mut self, jobs: usize, job: &(dyn Fn(usize) + Sync)) {
        for i in 0..jobs {
            job(i);
        }
    }
    fn execute_graph(&mut self, graph: &JobGraph, job: &(dyn Fn(usize) + Sync)) {
        for id in graph.order() {
            job(id);
        }
    }
    fn workers(&self) -> usize {
        2
    }
}

/// Builds a project with `build(handle)` and renders 56 000 samples (at least) of the
/// left channel in blocks of `block`.
fn run(
    block: usize,
    anticipative: bool,
    build: &dyn Fn(&mut ether_core::EngineHandle) -> RenderGraphDesc,
) -> Vec<f32> {
    let mut p = create(cfg(anticipative));
    if anticipative {
        p.engine.set_executor(Box::new(InOrder));
    }
    let desc = build(&mut p.handle);
    p.handle.publish(desc).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    let mut left = Vec::new();
    let (mut l, mut r) = (vec![0.0f32; block], vec![0.0f32; block]);
    while left.len() < 56_000 {
        let mut outs: [&mut [f32]; 2] = [&mut l[..], &mut r[..]];
        p.engine.process_with_hint(&[], &mut outs, block, block);
        left.extend_from_slice(&l);
        p.gc.collect();
    }
    left
}

/// The output is `before` up to sample `at` (exclusive) and `after` from it on, within
/// `ramp` samples starting at `at` (a ramp never starts before `at`).
#[track_caller]
fn assert_step(out: &[f32], at: usize, before: f32, after: f32, ramp: usize, what: &str) {
    let lo = at.saturating_sub(3_000);
    let early = (lo..at).find(|&i| (out[i] - before).abs() > 1e-6);
    let late = (at + ramp..at + 3_000).find(|&i| (out[i] - after).abs() > 1e-6);
    let first_moved = (lo..at + 3_000).find(|&i| (out[i] - before).abs() > 1e-6);
    assert!(
        early.is_none() && late.is_none(),
        "{what}: expected the step at sample {at}, the output first moves at {first_moved:?} \
         ({} samples off; early {early:?}, settled late {late:?})",
        first_moved.map_or(0, |f| f as i64 - at as i64),
    );
}

const L: usize = 1_000;

fn latent_track(
    handle: &mut ether_core::EngineHandle,
    tail: &[NodeKey],
) -> ether_core::graph::TrackDesc {
    let dc = handle.add_node(Box::new(Dc(1.0))).unwrap();
    let lat = handle.add_node(Box::new(Delay::new(L))).unwrap();
    let mut chain = vec![dc, lat];
    chain.extend_from_slice(tail);
    with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &chain)
}

fn desc(tracks: Vec<ether_core::graph::TrackDesc>) -> RenderGraphDesc {
    RenderGraphDesc {
        version: 1,
        tracks,
        ..Default::default()
    }
}

/// Every configuration a step is checked with.
fn configs() -> Vec<(usize, bool)> {
    vec![
        (64, false),
        (512, false),
        (100, false),
        (512, true),
        (128, true),
    ]
}

#[test]
fn device_after_a_latent_device() {
    for (block, antic) in configs() {
        let out = run(block, antic, &|h| {
            let g = h.add_node(Box::new(Gain(0.0))).unwrap();
            let mut t = latent_track(h, &[g]);
            t.automation = vec![step_lane(gain_of(g))];
            desc(vec![master(), t])
        });
        assert_step(
            &out,
            STEP + L,
            0.0,
            1.0,
            0,
            &format!("block {block} antic {antic}"),
        );
    }
}

#[test]
fn device_before_a_latent_device() {
    for (block, antic) in configs() {
        let out = run(block, antic, &|h| {
            let dc = h.add_node(Box::new(Dc(1.0))).unwrap();
            let g = h.add_node(Box::new(Gain(0.0))).unwrap();
            let lat = h.add_node(Box::new(Delay::new(L))).unwrap();
            let mut t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[dc, g, lat]);
            t.automation = vec![step_lane(gain_of(g))];
            desc(vec![master(), t])
        });
        assert_step(
            &out,
            STEP + L,
            0.0,
            1.0,
            0,
            &format!("block {block} antic {antic}"),
        );
    }
}

#[test]
fn volume_without_latency() {
    for (block, antic) in configs() {
        let out = run(block, antic, &|h| {
            let dc = h.add_node(Box::new(Dc(1.0))).unwrap();
            let mut t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[dc]);
            t.automation = vec![step_lane(ResolvedTarget::TrackVolume)];
            desc(vec![master(), t])
        });
        assert_step(
            &out,
            STEP,
            0.0,
            1.0,
            32,
            &format!("block {block} antic {antic}"),
        );
    }
}

#[test]
fn volume_after_a_latent_device() {
    for (block, antic) in configs() {
        let out = run(block, antic, &|h| {
            let mut t = latent_track(h, &[]);
            t.automation = vec![step_lane(ResolvedTarget::TrackVolume)];
            desc(vec![master(), t])
        });
        assert_step(
            &out,
            STEP + L,
            0.0,
            1.0,
            32,
            &format!("block {block} antic {antic}"),
        );
    }
}

#[test]
fn group_device_after_a_latent_child() {
    for (block, antic) in configs() {
        let out = run(block, antic, &|h| {
            let g = h.add_node(Box::new(Gain(0.0))).unwrap();
            let mut child = latent_track(h, &[]);
            child.output = Some(tid(10));
            child.group = Some(tid(10));
            let mut group = with_chain(track(tid(10), TrackKind::Group, Some(tid(1))), &[g]);
            group.automation = vec![step_lane(gain_of(g))];
            desc(vec![master(), group, child])
        });
        assert_step(
            &out,
            STEP + L,
            0.0,
            1.0,
            0,
            &format!("block {block} antic {antic}"),
        );
    }
}

#[test]
fn master_volume_after_a_latent_track() {
    for (block, antic) in configs() {
        let out = run(block, antic, &|h| {
            let t = latent_track(h, &[]);
            let mut m = master();
            m.automation = vec![step_lane(ResolvedTarget::TrackVolume)];
            desc(vec![m, t])
        });
        assert_step(
            &out,
            STEP + L,
            0.0,
            1.0,
            32,
            &format!("block {block} antic {antic}"),
        );
    }
}

#[test]
fn send_level_after_a_latent_device() {
    for (block, antic) in configs() {
        let out = run(block, antic, &|h| {
            let mut t = latent_track(h, &[]);
            t.volume = 0.0;
            t.sends = vec![send(sid(1), tid(9), 0.0, true)];
            t.automation = vec![step_lane(ResolvedTarget::Send { send: sid(1) })];
            let ret = track(tid(9), TrackKind::Return, Some(tid(1)));
            desc(vec![master(), t, ret])
        });
        assert_step(
            &out,
            STEP + L,
            0.0,
            1.0,
            32,
            &format!("block {block} antic {antic}"),
        );
    }
}

#[test]
fn loop_wrap_after_a_latent_device() {
    // Loop over beats 0..2 (48 000 samples): the gain steps up at STEP_BEAT and back to 0 at
    // the wrap. The audio after the latent device crosses the wrap `L` samples later, so
    // must the automation.
    for (block, antic) in configs() {
        let out = run(block, antic, &|h| {
            let g = h.add_node(Box::new(Gain(0.0))).unwrap();
            let mut t = latent_track(h, &[g]);
            t.automation = vec![step_lane(gain_of(g))];
            RenderGraphDesc {
                loop_enabled: true,
                loop_start: 0.0,
                loop_end: 2.0,
                ..desc(vec![master(), t])
            }
        });
        let what = format!("block {block} antic {antic}");
        assert_step(&out, STEP + L, 0.0, 1.0, 0, &format!("first pass, {what}"));
        assert_step(&out, 48_000 + L, 1.0, 0.0, 0, &format!("wrap, {what}"));
    }
}

#[test]
fn tempo_ramp_after_a_latent_device() {
    use ether_core::protocol::model::TempoCurve;
    use ether_core::tempo::TempoPointDesc;
    // 70 -> 190 BPM over 16 beats; the step's sample from the closed form.
    let (bpm0, bpm1, len, beat) = (70.0f64, 190.0f64, 16.0f64, 3.217_59f64);
    let slope = (bpm1 - bpm0) / len;
    let seconds = 60.0 / slope * (beat * slope / bpm0).ln_1p();
    let at = (seconds * f64::from(SR)).ceil() as usize;
    for (block, antic) in configs() {
        let mut p = create(cfg(antic));
        if antic {
            p.engine.set_executor(Box::new(InOrder));
        }
        let g = p.handle.add_node(Box::new(Gain(0.0))).unwrap();
        let mut t = latent_track(&mut p.handle, &[g]);
        let mut lane = step_lane(gain_of(g));
        lane.points[1].0 = beat;
        t.automation = vec![lane];
        p.handle
            .publish(RenderGraphDesc {
                tempo: vec![
                    TempoPointDesc {
                        beat: 0.0,
                        bpm: bpm0,
                        curve: TempoCurve::Linear,
                    },
                    TempoPointDesc {
                        beat: len,
                        bpm: bpm1,
                        curve: TempoCurve::Step,
                    },
                ],
                ..desc(vec![master(), t])
            })
            .unwrap();
        p.handle.transport(TransportControl::Play).unwrap();
        let (l, _) = render(&mut p.engine, at + L + 4_000, block);
        assert_step(
            &l,
            at + L,
            0.0,
            1.0,
            0,
            &format!("block {block} antic {antic}"),
        );
    }
}

#[test]
fn clip_envelope_after_a_latent_device() {
    for (block, antic) in configs() {
        let out = run(block, antic, &|h| {
            let g = h.add_node(Box::new(Gain(0.0))).unwrap();
            let mut t = latent_track(h, &[g]);
            // A clip from beat 1 to 3 whose envelope steps at timeline STEP_BEAT.
            let mut clip = midi_clip(cid(1), 1.0, 2.0, &[]);
            let mut env = step_lane(gain_of(g));
            env.points[1].0 = STEP_BEAT - 1.0;
            clip.envelopes = vec![env];
            t.clips = vec![clip];
            desc(vec![master(), t])
        });
        assert_step(
            &out,
            STEP + L,
            0.0,
            1.0,
            0,
            &format!("block {block} antic {antic}"),
        );
    }
}

#[test]
fn device_switch_after_a_latent_device() {
    // Gain 0 switched on by automation: the output (1 through the bypass path) starts its
    // crossfade towards 0 on the step's sample of the delayed audio.
    for (block, antic) in configs() {
        let out = run(block, antic, &|h| {
            let g = h.add_node(Box::new(Gain(0.0))).unwrap();
            let mut t = latent_track(h, &[g]);
            t.chain[2].enabled = false;
            t.automation = vec![step_lane(ResolvedTarget::NodeEnabled { node: g })];
            desc(vec![master(), t])
        });
        // The 10 ms crossfade (480 samples) starts at the step.
        assert_step(
            &out,
            STEP + L,
            1.0,
            0.0,
            480,
            &format!("block {block} antic {antic}"),
        );
    }
}

#[test]
fn vca_after_a_latent_device() {
    use ether_core::vca::VcaDesc;
    for (block, antic) in configs() {
        let out = run(block, antic, &|h| {
            let mut t = latent_track(h, &[]);
            t.vca = Some(tid(20));
            RenderGraphDesc {
                vcas: vec![VcaDesc {
                    id: tid(20),
                    volume: 1.0,
                    mute: false,
                    parent: None,
                    automation: vec![step_lane(ResolvedTarget::TrackVolume)],
                }],
                ..desc(vec![master(), t])
            }
        });
        assert_step(
            &out,
            STEP + L,
            0.0,
            1.0,
            32,
            &format!("block {block} antic {antic}"),
        );
    }
}

#[test]
fn param_changed_while_off_applies_when_back_on() {
    // The device is off until beat 1 (sample 24 000); its gain steps to 1 at beat 0.5 while
    // it is off. Back on, it must play with gain 1, not with its stale initial 0.5.
    for (block, antic) in configs() {
        let out = run(block, antic, &|h| {
            let dc = h.add_node(Box::new(Dc(1.0))).unwrap();
            let g = h.add_node(Box::new(Gain(0.5))).unwrap();
            let mut t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[dc, g]);
            let mut on = step_lane(ResolvedTarget::NodeEnabled { node: g });
            on.points[1].0 = 1.0;
            let mut gain = step_lane(gain_of(g));
            gain.points = vec![(0.0, 0.25, CurveShape::Step), (0.5, 1.0, CurveShape::Step)];
            t.automation = vec![on, gain];
            desc(vec![master(), t])
        });
        // Through the bypass (1.0) to the device's output at gain 1: no dip to 0.5.
        let what = format!("block {block} antic {antic}");
        assert!(
            out[24_000..30_000].iter().all(|v| (v - 1.0).abs() < 1e-6),
            "{what}: {:?}",
            &out[24_000 + 470..24_000 + 490]
        );
    }
}

#[test]
fn volume_ramp_has_no_sample_skew() {
    // A linear volume lane from 0 at beat 1 to 1 at beat 2: sample `i` plays the lane's
    // value at its own beat (not the next sample's).
    for (block, antic) in configs() {
        let out = run(block, antic, &|h| {
            let dc = h.add_node(Box::new(Dc(1.0))).unwrap();
            let mut t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[dc]);
            let mut lane = step_lane(ResolvedTarget::TrackVolume);
            lane.points = vec![
                (1.0, 0.0, CurveShape::Linear),
                (2.0, 1.0, CurveShape::Linear),
            ];
            t.automation = vec![lane];
            desc(vec![master(), t])
        });
        assert_eq!(out[23_999], 0.0, "block {block} antic {antic}: early");
        // (The first grid interval after the kink at beat 1 is interpolated across it.)
        for i in (24_032..48_000).step_by(7) {
            let want = (i - 24_000) as f32 / 24_000.0;
            assert!(
                (out[i] - want).abs() < 2e-5,
                "block {block} antic {antic}: sample {i} is {} (want {want})",
                out[i]
            );
        }
    }
}

/// A rack node passing its input (the chains' mix) through.
struct RackNode;

impl Node for RackNode {
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

#[test]
fn rack_chain_devices_around_a_latent_device() {
    use ether_core::graph::ChainEntry;
    use ether_core::protocol::model::{RackChainId, Ulid};
    use ether_core::rack_chains::{ChainRackDesc, ChainRackKind, RackChainDesc};
    // An audio effect rack after a latent device, whose one chain is gain → latent → gain:
    // the first gain's automation is late by the outer latency, the second's by both.
    for second in [false, true] {
        for (block, antic) in configs() {
            let out = run(block, antic, &|h| {
                let g1 = h
                    .add_node(Box::new(Gain(if second { 1.0 } else { 0.0 })))
                    .unwrap();
                let lat = h.add_node(Box::new(Delay::new(300))).unwrap();
                let g2 = h
                    .add_node(Box::new(Gain(if second { 0.0 } else { 1.0 })))
                    .unwrap();
                let rack = h.add_node(Box::new(RackNode)).unwrap();
                let mut t = latent_track(h, &[rack]);
                let entry = |node| ChainEntry {
                    node,
                    enabled: true,
                    sidechain: None,
                };
                t.chain_racks = vec![ChainRackDesc {
                    rack,
                    kind: ChainRackKind::AudioEffect,
                    chains: vec![RackChainDesc {
                        id: RackChainId(Ulid(1)),
                        chain: vec![entry(g1), entry(lat), entry(g2)],
                        volume: 1.0,
                        pan: 0.0,
                        mute: false,
                        keys: (0, 127),
                        velocities: (0, 127),
                        select: (0, 127),
                    }],
                    selector: 0,
                }];
                t.automation = vec![step_lane(gain_of(if second { g2 } else { g1 }))];
                desc(vec![master(), t])
            });
            assert_step(
                &out,
                STEP + L + 300,
                0.0,
                1.0,
                0,
                &format!("second {second} block {block} antic {antic}"),
            );
        }
    }
}

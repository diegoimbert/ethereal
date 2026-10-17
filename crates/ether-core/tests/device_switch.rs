//! Device on/off switching (static or automated, `ResolvedTarget::NodeEnabled`): the audio
//! switch lands on the breakpoint's sample, crossfades with the bypass path (no click), and
//! the bypass path carries the device's latency (no timing jump), on track chains, rack
//! chains and drum pads.

mod common;

use common::*;
use ether_core::graph::{
    AutomationDesc, PadDesc, ParamMapping, RackDesc, ResolvedTarget, TrackDesc,
};
use ether_core::protocol::devices::ParamScale;
use ether_core::protocol::model::{
    AutomationTarget, CurveShape, DrumPadId, RackChainId, TrackKind, Ulid,
};
use ether_core::rack_chains::{ChainRackDesc, ChainRackKind, RackChainDesc};
use ether_core::{
    AudioBuffers, Node, NodeKey, PrepareConfig, ProcessContext, ProcessStatus, RenderGraphDesc,
    TransportControl, create,
};

const BEAT: usize = 24_000; // 120 BPM @ 48 kHz
/// `SWITCH_FADE_MS` (10 ms) at 48 kHz.
const FADE: usize = 480;
/// Not a divisor of a beat: switches land inside blocks.
const ODD_BLOCK: usize = 384;

fn desc(tracks: Vec<ether_core::graph::TrackDesc>) -> RenderGraphDesc {
    RenderGraphDesc {
        version: 1,
        tracks,
        ..Default::default()
    }
}

/// On, off from beat `off`, on again from beat `on`.
fn switch(node: NodeKey, off: f64, on: f64) -> AutomationDesc {
    AutomationDesc {
        target: AutomationTarget::TrackVolume { track: tid(2) },
        resolved: ResolvedTarget::NodeEnabled { node },
        points: vec![
            (0.0, 1.0, CurveShape::Step),
            (off, 0.0, CurveShape::Step),
            (on, 1.0, CurveShape::Step),
        ],
        mapping: ParamMapping {
            min: 0.0,
            max: 1.0,
            scale: ParamScale::Linear,
            steps: Some(2),
        },
    }
}

/// A deterministic non-constant signal of the engine clock (instrument-like).
struct Ramp;

fn ramp(t: usize) -> f32 {
    (t % 977) as f32 * 0.001
}

impl Node for Ramp {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let t0 = ctx.transport.sample_time as usize;
        for ch in audio.outputs.iter_mut() {
            for (i, s) in ch.iter_mut().enumerate() {
                *s = ramp(t0 + i);
            }
        }
        ProcessStatus::Continue
    }
    fn channels(&self) -> (u16, u16) {
        (0, 2)
    }
}

/// A rack's node: passes its (chain-mixed) input through.
struct Thru;

impl Node for Thru {
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
fn automated_switch_is_sample_accurate_and_click_free() {
    // Dc(0.5) → Dc(0.25): the second one on outputs 0.25, off lets 0.5 through.
    let mut p = create(config());
    let a = p.handle.add_node(Box::new(Dc(0.5))).unwrap();
    let b = p.handle.add_node(Box::new(Dc(0.25))).unwrap();
    let mut t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[a, b]);
    t.automation = vec![switch(b, 1.0, 2.0)];
    p.handle.publish(desc(vec![master(), t])).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    let (l, _) = render(&mut p.engine, 3 * BEAT, ODD_BLOCK);
    // Off at beat 1, exactly on its sample (inside a block), ramping over FADE samples.
    assert_eq!(l[BEAT - 1], 0.25);
    assert!(l[BEAT] > 0.25 && l[BEAT] < 0.26, "{}", l[BEAT]);
    assert!((l[BEAT + FADE] - 0.5).abs() < 1e-6, "{}", l[BEAT + FADE]);
    // On again at beat 2.
    assert!((l[2 * BEAT - 1] - 0.5).abs() < 1e-6);
    assert!(l[2 * BEAT] < 0.5 && l[2 * BEAT] > 0.49, "{}", l[2 * BEAT]);
    assert!((l[2 * BEAT + FADE] - 0.25).abs() < 1e-6);
    // Bounded slope: never a step bigger than one fade increment.
    let max_step = l
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(max_step <= 0.25 / FADE as f32 * 1.01, "click: {max_step}");
}

#[test]
fn static_switch_crossfades_too() {
    let mut p = create(config());
    let a = p.handle.add_node(Box::new(Dc(0.5))).unwrap();
    let b = p.handle.add_node(Box::new(Dc(0.25))).unwrap();
    let mut t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[a, b]);
    p.handle.publish(desc(vec![master(), t.clone()])).unwrap();
    let (l, _) = render(&mut p.engine, 1024, BLOCK);
    assert_eq!(l[1023], 0.25);
    t.chain[1].enabled = false;
    p.handle.publish(desc(vec![master(), t])).unwrap();
    let (l, _) = render(&mut p.engine, 2048, BLOCK);
    assert!(l[0] < 0.26, "starts from where it was: {}", l[0]);
    assert!((l[FADE] - 0.5).abs() < 1e-6, "{}", l[FADE]);
    let max_step = l
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(max_step <= 0.25 / FADE as f32 * 1.01, "click: {max_step}");
}

/// Renders 3 beats of the track `build` makes (given its engine's node adder), with and
/// without the device switch lanes it returns: the bypass path carries the device's latency,
/// so switching pure delays changes nothing at all.
fn same_with_and_without(
    build: impl Fn(&mut dyn FnMut(Box<dyn Node>) -> NodeKey) -> (TrackDesc, Vec<AutomationDesc>),
) {
    let run = |switched: bool| {
        let mut p = create(config());
        let (mut t, lanes) = build(&mut |n| p.handle.add_node(n).unwrap());
        if switched {
            t.automation = lanes;
        }
        p.handle.publish(desc(vec![master(), t])).unwrap();
        p.handle.transport(TransportControl::Play).unwrap();
        render(&mut p.engine, 3 * BEAT, ODD_BLOCK).0
    };
    let want = run(false);
    assert!(want.iter().any(|&s| s > 0.1), "the signal arrives");
    let got = run(true);
    for (i, (g, w)) in got.iter().zip(&want).enumerate() {
        assert!((g - w).abs() < 1e-6, "sample {i}: {g} != {w}");
    }
}

#[test]
fn switching_a_latency_device_keeps_time_on_a_track_chain() {
    same_with_and_without(|add| {
        let src = add(Box::new(Ramp));
        let delay = add(Box::new(Delay::new(64)));
        let t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[src, delay]);
        (t, vec![switch(delay, 1.0, 2.0)])
    });
    // Off in the document, automated on for beat 1 only.
    same_with_and_without(|add| {
        let src = add(Box::new(Ramp));
        let delay = add(Box::new(Delay::new(64)));
        let mut t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[src, delay]);
        t.chain[1].enabled = false;
        let mut lane = switch(delay, 0.0, 1.0);
        lane.points = vec![
            (0.0, 0.0, CurveShape::Step),
            (1.0, 1.0, CurveShape::Step),
            (2.0, 0.0, CurveShape::Step),
        ];
        (t, vec![lane])
    });
}

fn rack_chain(id: u128, nodes: &[NodeKey]) -> RackChainDesc {
    RackChainDesc {
        id: RackChainId(Ulid(id)),
        chain: nodes
            .iter()
            .map(|&node| ether_core::graph::ChainEntry {
                node,
                enabled: true,
                sidechain: None,
            })
            .collect(),
        volume: 1.0,
        pan: 0.0,
        mute: false,
        keys: (0, 127),
        velocities: (0, 127),
        select: (0, 127),
    }
}

#[test]
fn switching_a_latency_device_keeps_time_in_racks() {
    // Audio effect rack: a delay on one chain (aligned with a muted empty one, so the rack
    // is a pure delay too), and the rack itself switched later.
    same_with_and_without(|add| {
        let src = add(Box::new(Ramp));
        let rack = add(Box::new(Thru));
        let delay = add(Box::new(Delay::new(64)));
        let mut t = with_chain(track(tid(2), TrackKind::Audio, Some(tid(1))), &[src, rack]);
        t.chain_racks = vec![ChainRackDesc {
            rack,
            kind: ChainRackKind::AudioEffect,
            chains: vec![
                rack_chain(1, &[delay]),
                RackChainDesc {
                    mute: true,
                    ..rack_chain(2, &[])
                },
            ],
            selector: 0,
        }];
        (t, vec![switch(delay, 0.5, 1.0), switch(rack, 1.5, 2.5)])
    });
    // A drum pad's chain: the pad's instrument, then a delay.
    same_with_and_without(|add| {
        let drums = add(Box::new(Thru));
        let src = add(Box::new(Ramp));
        let delay = add(Box::new(Delay::new(64)));
        let mut t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[drums]);
        let pad = PadDesc {
            pad: DrumPadId(Ulid(7)),
            note: 36,
            choke_group: None,
            chain: rack_chain(0, &[src, delay]).chain,
            volume: 1.0,
            pan: 0.0,
            mute: false,
        };
        t.racks = vec![RackDesc {
            rack: drums,
            pads: vec![pad],
        }];
        (t, vec![switch(delay, 1.0, 2.0)])
    });
}

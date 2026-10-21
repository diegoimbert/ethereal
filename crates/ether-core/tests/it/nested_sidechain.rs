//! v0.4 sidechains at any depth (`ether_core::sidechain`): devices inside drum pads and
//! (nested) rack chains read a sidechain, tapped at a point of another track or of a chain,
//! aligned for PDC; the main-signal delays inside chains count in the rack's latency; own
//! track points are read only when produced earlier; cycles are rejected.

use crate::common;

use std::sync::{Arc, Mutex};

use common::*;
use ether_core::graph::{
    ChainEntry, CompileError, NodeInfo, PadDesc, RackDesc, RenderGraphDesc, TrackDesc, compile_with,
};
use ether_core::protocol::model::{DrumPadId, InputTap, RackChainId, TrackId, TrackKind, Ulid};
use ether_core::rack_chains::{ChainRackDesc, ChainRackKind, RackChainDesc};
use ether_core::{
    AudioBuffers, Node, NodeKey, PrepareConfig, ProcessContext, ProcessStatus, ScPoint,
    SidechainTapDesc, create,
};

fn desc(tracks: Vec<TrackDesc>) -> RenderGraphDesc {
    RenderGraphDesc {
        version: 1,
        tracks,
        ..Default::default()
    }
}

#[derive(Default, Debug, Clone, Copy, PartialEq)]
struct Seen {
    main: Option<u64>,
    sidechain: Option<u64>,
    sidechain_calls: u32,
}

/// Passes its main input through; records the engine sample time of the first non-zero
/// main and sidechain samples.
struct Probe(Arc<Mutex<Seen>>);

fn first_nonzero(ctx: &ProcessContext<'_>, buf: &[f32]) -> Option<u64> {
    buf.iter()
        .position(|&s| s != 0.0)
        .map(|i| ctx.transport.sample_time + i as u64)
}

impl Node for Probe {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let mut seen = self.0.lock().unwrap();
        if seen.main.is_none() {
            seen.main = first_nonzero(ctx, audio.inputs[0]);
        }
        audio.pass_through();
        ProcessStatus::Continue
    }
    fn sidechain_inputs(&self) -> u16 {
        2
    }
    fn process_sidechain(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        sidechain: &[&[f32]],
    ) -> ProcessStatus {
        {
            let mut seen = self.0.lock().unwrap();
            seen.sidechain_calls += 1;
            if seen.sidechain.is_none() {
                seen.sidechain = first_nonzero(ctx, sidechain[0]);
            }
        }
        self.process(ctx, audio)
    }
}

/// Passes its input through (a rack device: its chains feed its input).
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

fn entries(nodes: &[NodeKey]) -> Vec<ChainEntry> {
    nodes
        .iter()
        .map(|&node| ChainEntry {
            node,
            enabled: true,
            sidechain: None,
        })
        .collect()
}

fn rack(rack: NodeKey, kind: ChainRackKind, chains: Vec<(u128, Vec<NodeKey>)>) -> ChainRackDesc {
    ChainRackDesc {
        rack,
        kind,
        selector: 0,
        chains: chains
            .into_iter()
            .map(|(id, nodes)| RackChainDesc {
                id: RackChainId(Ulid(id)),
                chain: entries(&nodes),
                volume: 1.0,
                pan: 0.0,
                mute: false,
                keys: (0, 127),
                velocities: (0, 127),
                select: (0, 127),
            })
            .collect(),
    }
}

/// Marks the entry of `node` (any depth) as sidechained from `source` at `point`.
fn key(t: &mut TrackDesc, node: NodeKey, source: TrackId, point: ScPoint) {
    let all = t
        .chain
        .iter_mut()
        .chain(
            t.racks
                .iter_mut()
                .flat_map(|r| &mut r.pads)
                .flat_map(|p| &mut p.chain),
        )
        .chain(
            t.chain_racks
                .iter_mut()
                .flat_map(|r| &mut r.chains)
                .flat_map(|c| &mut c.chain),
        );
    for e in all.filter(|e| e.node == node) {
        e.sidechain = Some(source);
    }
    if point != ScPoint::POST_FADER {
        t.sidechain_taps.push(SidechainTapDesc { node, point });
    }
}

struct Rig {
    p: ether_core::EngineParts,
}

impl Rig {
    fn new() -> Self {
        Self {
            p: create(config()),
        }
    }
    fn node(&mut self, n: impl Node + 'static) -> NodeKey {
        self.p.handle.add_node(Box::new(n)).unwrap()
    }
    fn probe(&mut self) -> (NodeKey, Arc<Mutex<Seen>>) {
        let seen = Arc::new(Mutex::new(Seen::default()));
        (self.node(Probe(seen.clone())), seen)
    }
    fn run(&mut self, tracks: Vec<TrackDesc>) -> Result<(), ether_core::EngineError> {
        self.p.handle.publish(desc(tracks))?;
        render(&mut self.p.engine, 4 * BLOCK, BLOCK);
        Ok(())
    }
}

/// Source S (tid 2): an impulse through `src_delay` samples of latency.
fn source(r: &mut Rig, src_delay: usize) -> TrackDesc {
    let mut chain = vec![r.node(Impulse)];
    if src_delay > 0 {
        chain.push(r.node(Delay::new(src_delay)));
    }
    with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &chain)
}

/// Consumer T (tid 3): instrument rack → chain [impulse, (delay), audio effect rack →
/// chain [probe]]: the probe two racks deep.
fn nested_consumer(r: &mut Rig, main_delay: usize) -> (TrackDesc, NodeKey, Arc<Mutex<Seen>>) {
    let (probe, seen) = r.probe();
    let outer = r.node(Pass);
    let inner = r.node(Pass);
    let imp = r.node(Impulse);
    let mut outer_chain = vec![imp];
    if main_delay > 0 {
        outer_chain.push(r.node(Delay::new(main_delay)));
    }
    outer_chain.push(inner);
    let mut t = with_chain(track(tid(3), TrackKind::Midi, Some(tid(1))), &[outer]);
    t.chain_racks = vec![
        rack(outer, ChainRackKind::Instrument, vec![(10, outer_chain)]),
        rack(inner, ChainRackKind::AudioEffect, vec![(11, vec![probe])]),
    ];
    (t, probe, seen)
}

#[test]
fn a_device_two_racks_deep_hears_a_later_sidechain_aligned() {
    // L_sc = 100 > L_main = 0: the main signal is delayed 100 before the probe, inside the
    // chain, and the delay counts in the consumer's latency.
    let mut r = Rig::new();
    let s = source(&mut r, 100);
    let (mut t, probe, seen) = nested_consumer(&mut r, 0);
    key(&mut t, probe, tid(2), ScPoint::POST_FADER);
    // The consumer is listed first: the compiler must still process the source first.
    r.run(vec![t.clone(), master(), s.clone()]).unwrap();
    let seen = *seen.lock().unwrap();
    assert_eq!(seen.sidechain, Some(100));
    assert_eq!(seen.main, Some(100));
    // PDC: the rack entry carries the 100 samples.
    let delay = s.chain[1].node;
    let lat = move |k: NodeKey| {
        Some(NodeInfo {
            latency: if delay == k { 100 } else { 0 },
            channels: (2, 2),
        })
    };
    let snap = compile_with(desc(vec![master(), s, t]), &config(), &lat).unwrap();
    assert_eq!(snap.track_latency(tid(3)), Some(100));
    assert_eq!(snap.output_compensation(tid(3)), Some(0));
}

#[test]
fn a_device_two_racks_deep_delays_an_earlier_sidechain() {
    // L_sc = 100 < L_main = 250: the sidechain is delayed 150.
    let mut r = Rig::new();
    let s = source(&mut r, 100);
    let (mut t, probe, seen) = nested_consumer(&mut r, 250);
    key(&mut t, probe, tid(2), ScPoint::POST_FADER);
    r.run(vec![master(), s, t]).unwrap();
    let seen = *seen.lock().unwrap();
    assert_eq!(seen.main, Some(250));
    assert_eq!(seen.sidechain, Some(250));
}

#[test]
fn a_pad_device_hears_its_sidechain() {
    let mut r = Rig::new();
    let s = source(&mut r, 0);
    let (probe, seen) = r.probe();
    let drums = r.node(Pass);
    let imp = r.node(Impulse);
    let mut t = with_chain(track(tid(3), TrackKind::Midi, Some(tid(1))), &[drums]);
    t.racks = vec![RackDesc {
        rack: drums,
        pads: vec![PadDesc {
            pad: DrumPadId(Ulid(20)),
            note: 36,
            choke_group: None,
            chain: entries(&[imp, probe]),
            volume: 1.0,
            pan: 0.0,
            mute: false,
        }],
    }];
    key(&mut t, probe, tid(2), ScPoint::Track(InputTap::PostFx));
    r.run(vec![master(), s, t]).unwrap();
    let seen = *seen.lock().unwrap();
    assert_eq!(seen.sidechain, Some(0));
    assert!(seen.sidechain_calls > 0);
}

/// Source S (tid 2) with an instrument rack: chain 30 = [impulse, delay 100], chain 31 =
/// [delay 200] (the longest).
fn chain_source(r: &mut Rig) -> (TrackDesc, NodeKey) {
    let rack_node = r.node(Pass);
    let imp = r.node(Impulse);
    let d100 = r.node(Delay::new(100));
    let d200 = r.node(Delay::new(200));
    let mut s = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[rack_node]);
    s.chain_racks = vec![rack(
        rack_node,
        ChainRackKind::Instrument,
        vec![(30, vec![imp, d100]), (31, vec![d200])],
    )];
    (s, imp)
}

fn chain_tap_case(point: ScPoint) -> Seen {
    let mut r = Rig::new();
    let (s, _) = chain_source(&mut r);
    let (probe, seen) = r.probe();
    let imp = r.node(Impulse);
    let mut t = with_chain(track(tid(3), TrackKind::Midi, Some(tid(1))), &[imp, probe]);
    key(&mut t, probe, tid(2), point);
    r.run(vec![master(), s, t]).unwrap();
    *seen.lock().unwrap()
}

#[test]
fn chain_taps_are_aligned_to_their_point() {
    // The chain's output is aligned with the rack's longest chain (200).
    let seen = chain_tap_case(ScPoint::ChainOut(30));
    assert_eq!((seen.sidechain, seen.main), (Some(200), Some(200)));
    let seen = chain_tap_case(ScPoint::ChainMix(30));
    assert_eq!((seen.sidechain, seen.main), (Some(200), Some(200)));
    // A device's output inside the chain (the instrument, "pre FX"): latency 0.
    let mut r = Rig::new();
    let (s, imp_src) = chain_source(&mut r);
    let (probe, seen) = r.probe();
    let imp = r.node(Impulse);
    let mut t = with_chain(track(tid(3), TrackKind::Midi, Some(tid(1))), &[imp, probe]);
    key(&mut t, probe, tid(2), ScPoint::After(imp_src));
    r.run(vec![master(), s, t]).unwrap();
    let seen = *seen.lock().unwrap();
    assert_eq!((seen.sidechain, seen.main), (Some(0), Some(0)));
    // An instrument rack chain's input is silent.
    let seen = chain_tap_case(ScPoint::ChainIn(30));
    assert_eq!(seen.sidechain, None);
    assert!(seen.sidechain_calls > 0);
}

#[test]
fn own_track_points_produced_earlier_are_heard() {
    // A device after the rack keyed from the rack's own chain (same track, same job).
    let mut r = Rig::new();
    let (mut s, _) = chain_source(&mut r);
    let (probe, seen) = r.probe();
    s.chain.extend(entries(&[probe]));
    key(&mut s, probe, tid(2), ScPoint::ChainOut(30));
    r.run(vec![master(), s]).unwrap();
    let seen = *seen.lock().unwrap();
    assert_eq!(seen.sidechain, Some(200));
    assert_eq!(seen.main, Some(200));
}

#[test]
fn a_chain_hears_an_earlier_chain_of_its_rack_before_the_alignment() {
    // Chain 30 = [impulse, delay 100] runs first; chain 32 = [impulse, probe] keys its
    // output before the rack's alignment (`ChainRaw`, latency 100): the probe's main signal
    // is delayed 100 to match, and the rack's longest chain is then 100 (not 100 + 100).
    let mut r = Rig::new();
    let rack_node = r.node(Pass);
    let (imp, d100, imp2) = (r.node(Impulse), r.node(Delay::new(100)), r.node(Impulse));
    let (probe, seen) = r.probe();
    let mut s = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[rack_node]);
    s.chain_racks = vec![rack(
        rack_node,
        ChainRackKind::Instrument,
        vec![(30, vec![imp, d100]), (32, vec![imp2, probe])],
    )];
    key(&mut s, probe, tid(2), ScPoint::ChainRaw(30));
    r.run(vec![master(), s.clone()]).unwrap();
    let seen = *seen.lock().unwrap();
    assert_eq!((seen.sidechain, seen.main), (Some(100), Some(100)));
    let lat = move |k: NodeKey| {
        Some(NodeInfo {
            latency: if k == d100 { 100 } else { 0 },
            channels: (2, 2),
        })
    };
    let snap = compile_with(desc(vec![master(), s]), &config(), &lat).unwrap();
    assert_eq!(snap.track_latency(tid(2)), Some(100));
}

#[test]
fn own_track_points_not_produced_yet_are_ignored() {
    // The probe inside chain 30 keyed from its own rack's output: not produced yet (the
    // model rejects it); the engine plays it without a sidechain rather than misreading.
    let mut r = Rig::new();
    let (mut s, _) = chain_source(&mut r);
    let (probe, seen) = r.probe();
    s.chain_racks[0].chains[0].chain.extend(entries(&[probe]));
    key(&mut s, probe, tid(2), ScPoint::ChainMix(31));
    // Its own track's post-fader output: a cycle, ignored too.
    let (probe2, seen2) = r.probe();
    s.chain.extend(entries(&[probe2]));
    key(&mut s, probe2, tid(2), ScPoint::POST_FADER);
    r.run(vec![master(), s]).unwrap();
    assert_eq!(seen.lock().unwrap().sidechain_calls, 0);
    assert_eq!(seen2.lock().unwrap().sidechain_calls, 0);
}

#[test]
fn nested_sidechains_that_loop_are_rejected() {
    // S's pad device keyed from T, T's nested device keyed from S's chain: a cycle.
    let mut r = Rig::new();
    let (mut s, _) = chain_source(&mut r);
    let (mut t, probe, _) = nested_consumer(&mut r, 0);
    key(&mut t, probe, tid(2), ScPoint::ChainOut(30));
    let (probe_s, _) = r.probe();
    s.chain_racks[0].chains[1].chain.extend(entries(&[probe_s]));
    key(&mut s, probe_s, tid(3), ScPoint::Track(InputTap::PreFx));
    let info = |_: NodeKey| {
        Some(NodeInfo {
            latency: 0,
            channels: (2, 2),
        })
    };
    assert!(matches!(
        compile_with(desc(vec![master(), s, t]), &config(), &info),
        Err(CompileError::Cycle(_))
    ));
}

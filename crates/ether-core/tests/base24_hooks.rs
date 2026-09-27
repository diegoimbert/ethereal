//! base-24 engine hooks: sidechain PDC alignment (both directions), drum-rack pad chains
//! (live params, automation and audio reach pad devices; pad latency counts in PDC).

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use ether_core::graph::{
    AutomationDesc, ChainEntry, NodeInfo, PadDesc, ParamMapping, RackDesc, RenderGraphDesc,
    ResolvedTarget, compile_with,
};
use ether_core::protocol::devices::ParamScale;
use ether_core::protocol::model::{
    AutomationTarget, CurveShape, DeviceId, DrumPadId, ParamId, TrackKind, Ulid,
};
use ether_core::{
    AudioBuffers, EventKind, Node, NodeKey, ParamChange, ParamTarget, PrepareConfig,
    ProcessContext, ProcessStatus, create,
};

fn desc(tracks: Vec<ether_core::graph::TrackDesc>) -> RenderGraphDesc {
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
            if seen.sidechain.is_none() {
                seen.sidechain = first_nonzero(ctx, sidechain[0]);
            }
        }
        self.process(ctx, audio)
    }
}

/// Source S: impulse through `src_delay` samples of latency. Consumer T: impulse through
/// `main_delay` samples, then the probe sidechained from S. Returns what the probe saw.
fn sidechain_case(src_delay: usize, main_delay: usize) -> Seen {
    let mut p = create(config());
    let seen = Arc::new(Mutex::new(Seen::default()));
    let s_imp = p.handle.add_node(Box::new(Impulse)).unwrap();
    let s_del = p.handle.add_node(Box::new(Delay::new(src_delay))).unwrap();
    let t_imp = p.handle.add_node(Box::new(Impulse)).unwrap();
    let probe = p.handle.add_node(Box::new(Probe(seen.clone()))).unwrap();
    let mut main = vec![t_imp];
    if main_delay > 0 {
        main.push(p.handle.add_node(Box::new(Delay::new(main_delay))).unwrap());
    }
    main.push(probe);
    let source = with_chain(
        track(tid(2), TrackKind::Midi, Some(tid(1))),
        &[s_imp, s_del],
    );
    let mut consumer = with_chain(track(tid(3), TrackKind::Midi, Some(tid(1))), &main);
    let last = consumer.chain.len() - 1;
    consumer.chain[last].sidechain = Some(tid(2));
    // The consumer is listed first: the compiler must still process the source first.
    p.handle
        .publish(desc(vec![consumer, master(), source]))
        .unwrap();
    render(&mut p.engine, 4 * BLOCK, BLOCK);
    *seen.lock().unwrap()
}

#[test]
fn sidechain_later_than_main_delays_the_main_signal() {
    // L_sc = 100 > L_main = 0: the main signal is delayed 100 before the probe.
    let seen = sidechain_case(100, 0);
    assert_eq!(seen.sidechain, Some(100));
    assert_eq!(seen.main, Some(100));
}

#[test]
fn sidechain_earlier_than_main_is_delayed() {
    // L_sc = 100 < L_main = 250: the sidechain is delayed 150.
    let seen = sidechain_case(100, 250);
    assert_eq!(seen.main, Some(250));
    assert_eq!(seen.sidechain, Some(250));
}

#[test]
fn sidechain_main_delay_counts_in_track_latency() {
    let mut p = create(config());
    let s = p.handle.add_node(Box::new(Delay::new(100))).unwrap();
    let probe = p
        .handle
        .add_node(Box::new(Probe(Arc::new(Mutex::new(Seen::default())))))
        .unwrap();
    let source = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[s]);
    let mut consumer = with_chain(track(tid(3), TrackKind::Midi, Some(tid(1))), &[probe]);
    consumer.chain[0].sidechain = Some(tid(2));
    let info = |k: NodeKey| {
        Some(NodeInfo {
            latency: if k == s { 100 } else { 0 },
            channels: (2, 2),
        })
    };
    let snap = compile_with(desc(vec![master(), source, consumer]), &config(), &info).unwrap();
    assert_eq!(snap.track_latency(tid(3)), Some(100));
    // Both reach master aligned: no extra output compensation for either.
    assert_eq!(snap.output_compensation(tid(2)), Some(0));
    assert_eq!(snap.output_compensation(tid(3)), Some(0));
}

// ─── Drum rack pads ─────────────────────────────────────────────────────────────────────

/// Records the Param events it receives and outputs a constant 0.25.
struct ParamProbe(Arc<Mutex<Vec<(u32, f64)>>>);

impl Node for ParamProbe {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let mut log = self.0.lock().unwrap();
        for e in ctx.events {
            if let EventKind::Param { param, value } = e.kind {
                log.push((param.0, value));
            }
        }
        for ch in audio.outputs.iter_mut() {
            ch.fill(0.25);
        }
        ProcessStatus::Continue
    }
}

/// Passes its input (the pads' mix) through, like the drum rack device.
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

fn rack_track(rack: NodeKey, pad_nodes: &[NodeKey]) -> ether_core::graph::TrackDesc {
    let mut t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[rack]);
    t.racks = vec![RackDesc {
        rack,
        pads: vec![PadDesc {
            pad: DrumPadId(Ulid(7)),
            note: 36,
            choke_group: None,
            chain: pad_nodes
                .iter()
                .map(|&node| ChainEntry {
                    node,
                    enabled: true,
                    sidechain: None,
                })
                .collect(),
            volume: 1.0,
            pan: 0.0,
            mute: false,
        }],
    }];
    t
}

#[test]
fn live_params_automation_and_audio_reach_pad_devices() {
    let mut p = create(config());
    let log = Arc::new(Mutex::new(Vec::new()));
    let rack = p.handle.add_node(Box::new(Pass)).unwrap();
    let pad_dev = p
        .handle
        .add_node(Box::new(ParamProbe(log.clone())))
        .unwrap();
    let mut t = rack_track(rack, &[pad_dev]);
    t.automation = vec![AutomationDesc {
        target: AutomationTarget::DeviceParam {
            device: DeviceId(Ulid(9)),
            param: ParamId(4),
        },
        resolved: ResolvedTarget::Node {
            node: pad_dev,
            param: ParamId(4),
        },
        points: vec![(0.0, 0.5, CurveShape::Linear)],
        mapping: ParamMapping {
            min: 0.0,
            max: 1.0,
            scale: ParamScale::Linear,
            steps: None,
        },
    }];
    p.handle.publish(desc(vec![master(), t])).unwrap();
    render(&mut p.engine, BLOCK, BLOCK);
    p.handle
        .set_param(ParamChange {
            target: ParamTarget::Node {
                node: pad_dev,
                param: ParamId(3),
            },
            value: 0.75,
        })
        .unwrap();
    let (l, r) = render(&mut p.engine, 2 * BLOCK, BLOCK);
    let log = log.lock().unwrap();
    assert!(log.contains(&(3, 0.75)), "live param: {log:?}");
    assert!(log.contains(&(4, 0.5)), "automation: {log:?}");
    // The pad's audio reaches the master through the rack.
    assert!(l[BLOCK..].iter().chain(&r[BLOCK..]).any(|&s| s > 0.1));
}

#[test]
fn pad_latency_counts_in_pdc() {
    let rack = NodeKey {
        index: 1,
        generation: 1,
    };
    let pad_a = NodeKey {
        index: 2,
        generation: 1,
    };
    let pad_b = NodeKey {
        index: 3,
        generation: 1,
    };
    let mut t = rack_track(rack, &[pad_a, pad_b]);
    t.racks[0].pads[0].chain.truncate(1);
    let mut second = t.racks[0].pads[0].clone();
    second.pad = DrumPadId(Ulid(8));
    second.note = 38;
    second.chain[0].node = pad_b;
    t.racks[0].pads.push(second);
    let info = |k: NodeKey| {
        Some(NodeInfo {
            latency: match k.index {
                1 => 10,
                2 => 50,
                3 => 20,
                _ => 0,
            },
            channels: (2, 2),
        })
    };
    let snap = compile_with(desc(vec![master(), t]), &config(), &info).unwrap();
    // Rack node 10 + longest pad chain 50.
    assert_eq!(snap.track_latency(tid(2)), Some(60));
    // A pad node used twice is rejected like any node.
    let dup = rack_track(rack, &[pad_a, pad_a]);
    assert!(compile_with(desc(vec![master(), dup]), &config(), &info).is_err());
}

// ─── Media preview ──────────────────────────────────────────────────────────────────────

#[test]
fn preview_plays_outside_the_transport_and_auto_stops() {
    use ether_core::EngineOutputs;
    use ether_core::preview::PreviewControl;
    let mut p = create(config());
    p.handle.publish(desc(vec![master()])).unwrap();
    let len = BLOCK + 100;
    p.handle
        .preview(PreviewControl::Play {
            source: Arc::new(MemSource(vec![0.5; len])),
            gain: 0.5,
        })
        .unwrap();
    // Stopped transport: the preview still plays, on both channels (mono source).
    let (l, r) = render(&mut p.engine, 3 * BLOCK, BLOCK);
    assert!((l[0] - 0.25).abs() < 1e-6 && (r[0] - 0.25).abs() < 1e-6);
    assert!((l[len - 1] - 0.25).abs() < 1e-6);
    assert!(l[len + 50..].iter().all(|&s| s == 0.0), "auto-stop");
    let mut out = EngineOutputs::default();
    p.handle.poll(&mut out);
    assert!(out.preview_ended);
    p.handle.poll(&mut out);
    assert!(!out.preview_ended, "reported once");
    // The finished source was retired to the GC, not dropped on the audio thread.
    assert!(p.gc.collect() >= 1);

    // Stop cuts a playing preview and reports its end.
    p.handle
        .preview(PreviewControl::Play {
            source: Arc::new(MemSource(vec![0.5; 10 * BLOCK])),
            gain: 1.0,
        })
        .unwrap();
    render(&mut p.engine, BLOCK, BLOCK);
    p.handle.preview(PreviewControl::Stop).unwrap();
    let (l, _) = render(&mut p.engine, BLOCK, BLOCK);
    assert!(l.iter().all(|&s| s == 0.0));
    p.handle.poll(&mut out);
    assert!(out.preview_ended);
}

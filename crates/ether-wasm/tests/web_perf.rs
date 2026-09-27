//! Graph snapshot cost on the Worklet (web-perf): the large fixture (64 tracks, 500 clips,
//! automation) goes through the real `Publish` path (`WebBridge` encode → ring →
//! `EngineHost` decode + remap + compile), and the old JSON decode is measured next to the
//! binary one.
//!
//! Numbers for the PR: `cargo test -p ether-wasm --release --test web_perf -- --nocapture`.

#[path = "../../ether-core/tests/codec_fixture.rs"]
mod fixture;

use std::time::{Duration, Instant};

use ether_controller::EngineBridge;
use ether_core::codec::{BinaryCodec, GraphCodec};
use ether_core::protocol::model::BuiltinDevice;
use ether_core::{EngineOutputs, NodeKey, RenderGraphDesc};
use ether_wasm::bridge::{self, WebBridge};
use ether_wasm::proto::{EngineMsg, Frame};
use ether_wasm::ring::HeapMemory;
use ether_wasm::worklet::{EngineHost, RENDER_QUANTUM};

/// Every node key the desc references (chains and pad chains).
fn node_keys(d: &RenderGraphDesc) -> Vec<NodeKey> {
    let mut keys: Vec<NodeKey> = d
        .tracks
        .iter()
        .flat_map(|t| {
            t.chain.iter().map(|e| e.node).chain(
                t.racks
                    .iter()
                    .flat_map(|r| r.pads.iter().flat_map(|p| p.chain.iter().map(|e| e.node))),
            )
        })
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

/// Fastest of `n` runs.
fn best<T>(n: usize, mut f: impl FnMut() -> T) -> Duration {
    (0..n)
        .map(|_| {
            let t = Instant::now();
            let out = f();
            let e = t.elapsed();
            drop(std::hint::black_box(out));
            e
        })
        .min()
        .unwrap()
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

struct Setup {
    bridge: WebBridge<HeapMemory>,
    engine: EngineHost<HeapMemory>,
    shared: bridge::Shared<HeapMemory>,
}

/// A Worklet host with every fixture node created (under the fixture's virtual keys).
fn setup(d: &RenderGraphDesc) -> Setup {
    let control = HeapMemory::new(1 << 20);
    let reports = HeapMemory::new(1 << 14);
    let shared = bridge::shared(control.clone(), reports.clone());
    let bridge = WebBridge::new(shared.clone());
    let mut engine = EngineHost::new(48_000, control, reports);
    for key in node_keys(d) {
        for frame in (EngineMsg::CreateBuiltin {
            key,
            device: BuiltinDevice::Utility,
            params: vec![],
        })
        .encode()
        {
            shared.borrow_mut().control.send(&frame);
        }
    }
    drain(&mut engine, &shared);
    Setup {
        bridge,
        engine,
        shared,
    }
}

/// Render until the control ring is empty; returns the slowest quantum.
fn drain(engine: &mut EngineHost<HeapMemory>, shared: &bridge::Shared<HeapMemory>) -> Duration {
    let mut worst = Duration::ZERO;
    for _ in 0..10_000 {
        shared.borrow_mut().control.flush();
        if engine.control_backlog() == 0 && shared.borrow().control.pending_bytes() == 0 {
            return worst;
        }
        let t = Instant::now();
        engine.render(RENDER_QUANTUM);
        worst = worst.max(t.elapsed());
    }
    panic!("control ring never drained");
}

fn errors(s: &mut Setup) -> Vec<String> {
    let mut out = EngineOutputs::default();
    for _ in 0..8 {
        s.engine.render(RENDER_QUANTUM);
    }
    s.bridge.poll(&mut out);
    std::mem::take(&mut s.shared.borrow_mut().errors)
}

#[test]
fn large_fixture_publishes_through_the_worklet() {
    let desc = fixture::large_project();
    let mut s = setup(&desc);
    assert_eq!(errors(&mut s), Vec::<String>::new());
    s.bridge.publish(desc.clone()).unwrap();
    drain(&mut s.engine, &s.shared);
    assert_eq!(errors(&mut s), Vec::<String>::new());
}

#[test]
fn graph_frame_decodes_within_bound_on_the_worklet_path() {
    // Same bound as `crates/ether-core/tests/codec_alloc.rs` (`DECODE_BOUND`).
    const BOUND: Duration = Duration::from_millis(5);
    let desc = fixture::large_project();
    let frame = EngineMsg::Publish {
        graph: Box::new(desc),
    }
    .encode()
    .remove(0);
    let t = best(15, || match Frame::decode(&frame) {
        Ok(Frame::Msg(EngineMsg::Publish { graph })) => graph,
        other => panic!("{other:?}"),
    });
    assert!(t < BOUND, "graph frame decode took {t:?}");
}

/// Before/after numbers for the PR (`--nocapture`; run with `--release`).
#[test]
fn report_snapshot_costs() {
    let desc = fixture::large_project();
    let runs = 11;

    let json = serde_json::to_vec(&desc).unwrap();
    let mut bin = Vec::new();
    BinaryCodec.encode(&desc, &mut bin);

    let json_enc = best(runs, || serde_json::to_vec(&desc).unwrap());
    let json_dec = best(runs, || {
        serde_json::from_slice::<RenderGraphDesc>(&json).unwrap()
    });
    let bin_enc = best(runs, || {
        let mut out = Vec::new();
        BinaryCodec.encode(&desc, &mut out);
        out
    });
    let bin_dec = best(runs, || BinaryCodec.decode(&bin).unwrap());

    // Worklet quantum that applies the publish (decode + remap + compile + one block).
    let mut s = setup(&desc);
    let mut quantum = Duration::MAX;
    for _ in 0..runs {
        s.bridge.publish(desc.clone()).unwrap();
        quantum = quantum.min(drain(&mut s.engine, &s.shared));
    }
    // Compile alone: publish straight into a core engine (no codec).
    let parts = ether_core::create(ether_wasm::worklet::web_engine_config(48_000));
    let mut handle = parts.handle;
    let mut gc = parts.gc;
    let mut remapped = desc.clone();
    let mut map = std::collections::BTreeMap::new();
    for key in node_keys(&desc) {
        let node = ether_devices::create(&BuiltinDevice::Utility, &NoSamples);
        map.insert(key, handle.add_node(node).unwrap());
    }
    for t in &mut remapped.tracks {
        for e in t.chain.iter_mut() {
            e.node = map[&e.node];
        }
        for r in &mut t.racks {
            r.rack = map[&r.rack];
            for p in &mut r.pads {
                for e in &mut p.chain {
                    e.node = map[&e.node];
                }
            }
        }
        for a in t
            .automation
            .iter_mut()
            .chain(t.clips.iter_mut().flat_map(|c| c.envelopes.iter_mut()))
        {
            if let ether_core::graph::ResolvedTarget::Node { node, .. } = &mut a.resolved {
                *node = map[node];
            }
        }
    }
    let mut engine = parts.engine;
    let compile = best(runs, || {
        handle.publish(remapped.clone()).unwrap();
        // Let the audio side swap and retire, so the control queue never fills.
        let mut l = [0.0; RENDER_QUANTUM];
        let mut r = [0.0; RENDER_QUANTUM];
        engine.process(&[], &mut [&mut l, &mut r], RENDER_QUANTUM);
        gc.collect();
    });
    let budget = RENDER_QUANTUM as f64 / 48.0;
    eprintln!(
        "\nweb-perf: large fixture ({} tracks, {} clips)\n\
         | step | JSON (before) | binary (after) |\n\
         |---|---|---|\n\
         | size | {} B | {} B |\n\
         | encode (Worker) | {:.3} ms | {:.3} ms |\n\
         | decode (Worklet) | {:.3} ms | {:.3} ms |\n\
         compile (unchanged): {:.3} ms; worklet publish quantum (after): {:.3} ms; \
         quantum budget at 48 kHz: {:.3} ms",
        fixture::LARGE_TRACKS,
        fixture::LARGE_CLIPS,
        json.len(),
        bin.len(),
        ms(json_enc),
        ms(bin_enc),
        ms(json_dec),
        ms(bin_dec),
        ms(compile),
        ms(quantum),
        budget,
    );
}

struct NoSamples;

impl ether_devices::SampleResolver for NoSamples {
    fn resolve(
        &self,
        _media: ether_core::protocol::model::MediaId,
    ) -> Option<std::sync::Arc<dyn ether_core::AudioSource>> {
        None
    }
}

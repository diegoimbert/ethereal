//! Graph snapshot cost measurements (web-perf), shared by the native test
//! (`tests/web_perf.rs`) and the wasm export [`crate::web::bench_graph_snapshot`] (run
//! in V8 by `apps/web/src/engine/graphSnapshot.bench.test.ts`), so the numbers come from
//! the real `Publish` path: `EngineMsg::encode` (Worker) → ring → `EngineHost` (Worklet:
//! frame decode, key remap, compile, one render quantum).

use std::collections::BTreeMap;

use ether_core::codec::{BinaryCodec, GraphCodec};
use ether_core::graph::ResolvedTarget;
use ether_core::protocol::model::BuiltinDevice;
use ether_core::{NodeKey, RenderGraphDesc};
use serde::Serialize;

use crate::bridge::{self, Shared, WebBridge};
use crate::proto::EngineMsg;
use crate::ring::HeapMemory;
use crate::worklet::{EngineHost, RENDER_QUANTUM, web_engine_config};

/// Test fixtures (the large project: 64 tracks, 500 clips, automation). Shared with the
/// codec tests in `ether-core`.
#[path = "../../ether-core/tests/codec_fixture.rs"]
pub mod fixture;

/// Fastest-of-N timings in milliseconds.
#[derive(Clone, Debug, Serialize)]
pub struct SnapshotCosts {
    pub tracks: usize,
    pub clips: usize,
    pub json_bytes: usize,
    pub binary_bytes: usize,
    pub json_encode_ms: f64,
    pub json_decode_ms: f64,
    pub binary_encode_ms: f64,
    pub binary_decode_ms: f64,
    /// `EngineHandle::publish` alone (compile + swap send), no codec.
    pub compile_ms: f64,
    /// Slowest render quantum while a binary `Publish` is applied by the Worklet host
    /// (ring read, decode, remap, compile, render one block, GC).
    pub publish_quantum_ms: f64,
    /// One render quantum's real-time budget at 48 kHz.
    pub quantum_budget_ms: f64,
}

/// Every node key the desc references (chains and pad chains).
pub fn node_keys(d: &RenderGraphDesc) -> Vec<NodeKey> {
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

/// A Worklet host (heap rings) with a `Utility` node created for every key `d` references,
/// under the desc's (virtual) keys.
pub struct Pipeline {
    pub bridge: WebBridge<HeapMemory>,
    pub engine: EngineHost<HeapMemory>,
    pub shared: Shared<HeapMemory>,
}

impl Pipeline {
    pub fn new(d: &RenderGraphDesc) -> Self {
        let control = HeapMemory::new(1 << 20);
        let reports = HeapMemory::new(1 << 14);
        let shared = bridge::shared(control.clone(), reports.clone());
        let mut p = Self {
            bridge: WebBridge::new(shared.clone()),
            engine: EngineHost::new(48_000, control, reports),
            shared,
        };
        for key in node_keys(d) {
            p.send(&EngineMsg::CreateBuiltin {
                key,
                device: BuiltinDevice::Utility,
                params: vec![],
            });
        }
        p.drain(|| 0.0);
        p
    }

    pub fn send(&mut self, msg: &EngineMsg) {
        let mut shared = self.shared.borrow_mut();
        for frame in msg.encode() {
            shared.control.send(&frame);
        }
    }

    /// Render until the control ring is empty; returns the slowest quantum (per `now`).
    pub fn drain(&mut self, now: impl Fn() -> f64) -> f64 {
        let mut worst: f64 = 0.0;
        for _ in 0..100_000 {
            self.shared.borrow_mut().control.flush();
            if self.engine.control_backlog() == 0
                && self.shared.borrow().control.pending_bytes() == 0
            {
                return worst;
            }
            let t = now();
            self.engine.render(RENDER_QUANTUM);
            worst = worst.max(now() - t);
        }
        panic!("control ring never drained");
    }
}

fn best<T>(runs: usize, now: &impl Fn() -> f64, mut f: impl FnMut() -> T) -> f64 {
    (0..runs.max(1))
        .map(|_| {
            let t = now();
            let out = f();
            let e = now() - t;
            drop(std::hint::black_box(out));
            e
        })
        .fold(f64::INFINITY, f64::min)
}

/// Measure the large fixture's snapshot costs; `now` returns milliseconds.
pub fn measure(runs: usize, now: impl Fn() -> f64) -> SnapshotCosts {
    let desc = fixture::large_project();
    let json = serde_json::to_vec(&desc).expect("desc serializes");
    let mut bin = Vec::new();
    BinaryCodec.encode(&desc, &mut bin);

    let json_encode_ms = best(runs, &now, || serde_json::to_vec(&desc).unwrap());
    let json_decode_ms = best(runs, &now, || {
        serde_json::from_slice::<RenderGraphDesc>(&json).unwrap()
    });
    let binary_encode_ms = best(runs, &now, || {
        let mut out = Vec::new();
        BinaryCodec.encode(&desc, &mut out);
        out
    });
    let binary_decode_ms = best(runs, &now, || BinaryCodec.decode(&bin).unwrap());

    let mut p = Pipeline::new(&desc);
    let msg = EngineMsg::Publish {
        graph: Box::new(desc.clone()),
    };
    let mut publish_quantum_ms = f64::INFINITY;
    for _ in 0..runs.max(1) {
        p.send(&msg);
        publish_quantum_ms = publish_quantum_ms.min(p.drain(&now));
    }

    // Compile alone: publish straight into a core engine, keys mapped like the Worklet does.
    let parts = ether_core::create(web_engine_config(48_000));
    let (mut handle, mut engine, mut gc) = (parts.handle, parts.engine, parts.gc);
    let mut map = BTreeMap::new();
    for key in node_keys(&desc) {
        let node = ether_devices::create(&BuiltinDevice::Utility, &NoSamples);
        map.insert(key, handle.add_node(node).expect("node slot"));
    }
    let mut remapped = desc.clone();
    for t in &mut remapped.tracks {
        let pads = t.racks.iter_mut().flat_map(|r| r.pads.iter_mut());
        for e in t
            .chain
            .iter_mut()
            .chain(pads.flat_map(|p| p.chain.iter_mut()))
        {
            e.node = map[&e.node];
        }
        for r in &mut t.racks {
            r.rack = map[&r.rack];
        }
        let lanes = t
            .automation
            .iter_mut()
            .chain(t.clips.iter_mut().flat_map(|c| c.envelopes.iter_mut()));
        for a in lanes {
            if let ResolvedTarget::Node { node, .. } = &mut a.resolved {
                *node = map[node];
            }
        }
    }
    let mut compile_ms = f64::INFINITY;
    let mut l = [0.0; RENDER_QUANTUM];
    let mut r = [0.0; RENDER_QUANTUM];
    for _ in 0..runs.max(1) {
        let d = remapped.clone();
        let t = now();
        handle.publish(d).expect("large fixture compiles");
        compile_ms = compile_ms.min(now() - t);
        // Swap and retire, so the control queue never fills.
        engine.process(&[], &mut [&mut l, &mut r], RENDER_QUANTUM);
        gc.collect();
    }

    SnapshotCosts {
        tracks: desc.tracks.len(),
        clips: desc.tracks.iter().map(|t| t.clips.len()).sum(),
        json_bytes: json.len(),
        binary_bytes: bin.len(),
        json_encode_ms,
        json_decode_ms,
        binary_encode_ms,
        binary_decode_ms,
        compile_ms,
        publish_quantum_ms,
        quantum_budget_ms: RENDER_QUANTUM as f64 / 48.0,
    }
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

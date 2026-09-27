//! Graph snapshot cost on the Worklet (web-perf): the large fixture (64 tracks, 500 clips,
//! automation) goes through the real `Publish` path (`WebBridge` encode → ring →
//! `EngineHost` decode + remap + compile), and the old JSON decode is measured next to the
//! binary one ([`ether_wasm::perf`]; the same code runs in V8 via `bench_graph_snapshot`).
//!
//! Native numbers: `cargo test -p ether-wasm --release --test web_perf -- --nocapture`.

use std::sync::LazyLock;
use std::time::{Duration, Instant};

use ether_controller::EngineBridge;
use ether_core::EngineOutputs;
use ether_wasm::perf::{self, Pipeline, fixture};
use ether_wasm::proto::{EngineMsg, Frame};
use ether_wasm::worklet::RENDER_QUANTUM;

static START: LazyLock<Instant> = LazyLock::new(Instant::now);

fn now_ms() -> f64 {
    START.elapsed().as_secs_f64() * 1e3
}

fn errors(p: &mut Pipeline) -> Vec<String> {
    let mut out = EngineOutputs::default();
    for _ in 0..8 {
        p.engine.render(RENDER_QUANTUM);
    }
    p.bridge.poll(&mut out);
    std::mem::take(&mut p.shared.borrow_mut().errors)
}

#[test]
fn large_fixture_publishes_through_the_worklet() {
    let desc = fixture::large_project();
    let mut p = Pipeline::new(&desc);
    assert_eq!(errors(&mut p), Vec::<String>::new());
    p.bridge.publish(desc).unwrap();
    p.drain(now_ms);
    assert_eq!(errors(&mut p), Vec::<String>::new());
}

#[test]
fn graph_frame_decodes_within_bound_on_the_worklet_path() {
    // Same bound as `crates/ether-core/tests/codec_alloc.rs` (`DECODE_BOUND`).
    const BOUND: Duration = Duration::from_millis(5);
    let frame = EngineMsg::Publish {
        graph: Box::new(fixture::large_project()),
    }
    .encode()
    .remove(0);
    let best = (0..15)
        .map(|_| {
            let t = Instant::now();
            let graph = match Frame::decode(&frame) {
                Ok(Frame::Msg(EngineMsg::Publish { graph })) => graph,
                other => panic!("{other:?}"),
            };
            let e = t.elapsed();
            drop(graph);
            e
        })
        .min()
        .unwrap();
    assert!(best < BOUND, "graph frame decode took {best:?}");
}

/// Before/after numbers for the PR (`--nocapture`; run with `--release`).
#[test]
fn report_snapshot_costs() {
    let c = perf::measure(11, now_ms);
    eprintln!(
        "\nweb-perf (native): large fixture ({} tracks, {} clips)\n\
         | step | JSON (before) | binary (after) |\n\
         |---|---|---|\n\
         | size | {} B | {} B |\n\
         | encode (Worker) | {:.3} ms | {:.3} ms |\n\
         | decode (Worklet) | {:.3} ms | {:.3} ms |\n\
         compile: {:.3} ms; worst publish quantum (after): {:.3} ms; budget {:.3} ms",
        c.tracks,
        c.clips,
        c.json_bytes,
        c.binary_bytes,
        c.json_encode_ms,
        c.binary_encode_ms,
        c.json_decode_ms,
        c.binary_decode_ms,
        c.compile_ms,
        c.publish_quantum_ms,
        c.quantum_budget_ms,
    );
    assert!(c.binary_decode_ms < c.json_decode_ms);
}

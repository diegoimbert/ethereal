//! Silence skipping (perf audit #5): the engine skips a node that returned
//! `ProcessStatus::Silent` while its input stays digital silence and no event reaches it,
//! and processes it again on the first event, non-silent input, a reset, a watch of its
//! analysis or its own wake request. Allocation-free on every path.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use common::*;
use ether_core::protocol::model::{ParamId, TrackKind};
use ether_core::{
    AudioBuffers, Engine, Node, NodeKey, ParamChange, ParamTarget, PrepareConfig, ProcessContext,
    ProcessStatus, RenderGraphDesc,
};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: AllocDisabler = AllocDisabler;

/// Pass-through that reports `Silent` whenever its input was silent, counting calls; can
/// ask to be woken.
struct Sleepy {
    calls: Arc<AtomicUsize>,
    wake: Arc<AtomicBool>,
}

impl Node for Sleepy {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        _: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        self.calls.fetch_add(1, Ordering::Relaxed);
        audio.pass_through();
        if audio.inputs.iter().all(|c| c.iter().all(|s| *s == 0.0)) {
            ProcessStatus::Silent
        } else {
            ProcessStatus::Continue
        }
    }
    fn wake_requested(&mut self) -> bool {
        self.wake.swap(false, Ordering::Relaxed)
    }
}

struct Rig {
    engine: Engine,
    handle: ether_core::EngineHandle,
    sleepy: NodeKey,
    calls: Arc<AtomicUsize>,
    wake: Arc<AtomicBool>,
}

/// A track whose chain is [`Sleepy`] (silent input: no clips, nothing before it).
fn rig() -> Rig {
    let mut p = ether_core::create(config());
    let calls = Arc::new(AtomicUsize::new(0));
    let wake = Arc::new(AtomicBool::new(false));
    let sleepy = p
        .handle
        .add_node(Box::new(Sleepy {
            calls: calls.clone(),
            wake: wake.clone(),
        }))
        .unwrap();
    let t = with_chain(track(tid(2), TrackKind::Audio, Some(tid(1))), &[sleepy]);
    p.handle
        .publish(RenderGraphDesc {
            version: 1,
            tracks: vec![master(), t],
            ..Default::default()
        })
        .unwrap();
    Rig {
        engine: p.engine,
        handle: p.handle,
        sleepy,
        calls,
        wake,
    }
}

fn blocks(r: &mut Rig, n: usize) -> usize {
    let before = r.calls.load(Ordering::Relaxed);
    let (mut l, mut rr) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    for _ in 0..n {
        let mut outs: [&mut [f32]; 2] = [&mut l, &mut rr];
        assert_no_alloc(|| r.engine.process(&[], &mut outs, BLOCK));
    }
    r.calls.load(Ordering::Relaxed) - before
}

#[test]
fn asleep_node_is_skipped_until_something_reaches_it() {
    let mut r = rig();
    // Processed once (reports Silent), then skipped.
    assert_eq!(blocks(&mut r, 1), 1);
    assert_eq!(blocks(&mut r, 10), 0);
    // An event wakes it for that block; silent input puts it back to sleep.
    r.handle
        .set_param(ParamChange {
            target: ParamTarget::Node {
                node: r.sleepy,
                param: ParamId(0),
            },
            value: 0.5,
        })
        .unwrap();
    assert_eq!(blocks(&mut r, 1), 1);
    assert_eq!(blocks(&mut r, 5), 0);
    // Its own wake request.
    r.wake.store(true, Ordering::Relaxed);
    assert_eq!(blocks(&mut r, 1), 1);
    assert_eq!(blocks(&mut r, 5), 0);
    // A watched analysis keeps it processed.
    r.handle.watch_analysis(r.sleepy, true).unwrap();
    assert_eq!(blocks(&mut r, 5), 5);
    r.handle.watch_analysis(r.sleepy, false).unwrap();
    assert_eq!(blocks(&mut r, 5), 0);
    // A locate resets every node and processes it again.
    r.handle
        .transport(ether_core::TransportControl::Locate {
            position: ether_core::protocol::model::Beats(4.0),
        })
        .unwrap();
    assert_eq!(blocks(&mut r, 1), 1);
    assert_eq!(blocks(&mut r, 5), 0);
}

#[test]
fn skipping_can_be_turned_off() {
    let mut r = rig();
    r.engine.set_silence_skipping(false);
    assert_eq!(blocks(&mut r, 10), 10);
    // It kept reporting Silent: skipped right away.
    r.engine.set_silence_skipping(true);
    assert_eq!(blocks(&mut r, 10), 0);
}

/// A sound source before the asleep node wakes it (non-silent input), and its output is
/// then the pass-through, sample for sample.
#[test]
fn signal_wakes_the_node() {
    let mut p = ether_core::create(config());
    let calls = Arc::new(AtomicUsize::new(0));
    let sleepy = p
        .handle
        .add_node(Box::new(Sleepy {
            calls: calls.clone(),
            wake: Arc::new(AtomicBool::new(false)),
        }))
        .unwrap();
    let dc = p.handle.add_node(Box::new(Dc(0.25))).unwrap();
    let silent = p.handle.add_node(Box::new(Dc(0.0))).unwrap();
    let publish = |h: &mut ether_core::EngineHandle, src: NodeKey| {
        let t = with_chain(
            track(tid(2), TrackKind::Audio, Some(tid(1))),
            &[src, sleepy],
        );
        h.publish(RenderGraphDesc {
            version: 1,
            tracks: vec![master(), t],
            ..Default::default()
        })
        .unwrap();
    };
    publish(&mut p.handle, silent);
    let (l, _) = render(&mut p.engine, 4 * BLOCK, BLOCK);
    assert!(l.iter().all(|s| *s == 0.0));
    assert_eq!(
        calls.load(Ordering::Relaxed),
        1,
        "asleep after its first block"
    );
    publish(&mut p.handle, dc);
    let (l, _) = render(&mut p.engine, 4 * BLOCK, BLOCK);
    assert_eq!(calls.load(Ordering::Relaxed), 5);
    // After the track's un-mute ramp of the new snapshot: the DC passes through.
    assert!(
        l[3 * BLOCK..].iter().all(|s| (*s - 0.25).abs() < 1e-6),
        "{:?}",
        &l[3 * BLOCK..3 * BLOCK + 4]
    );
}

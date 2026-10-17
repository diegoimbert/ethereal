//! Multicore (`multicore`, CONTRACTS.md §11.7): parallel processing is bit-identical
//! to sequential processing on randomized projects (groups, returns, pre/post sends,
//! sidechains, drum racks with choke groups, latent devices, audio clips incl.
//! Complex-warped ones pinned to the audio thread, automation, a snapshot swap, live
//! params, a locate), across worker counts and block sizes, whatever the job order.
//!
//! The real-time worker pool is tested the same way in `ether-native/tests/multicore*.rs`.

mod parallel_graphs;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ether_core::parallel::{ParallelExecutor, SequentialExecutor};
use parallel_graphs::*;

const FRAMES: usize = SR as usize / 2;

fn stretch() -> Option<Arc<dyn ether_stretch::StretcherFactory>> {
    Some(Arc::new(ether_stretch::SignalsmithFactory::default()))
}

/// Records how the engine dispatched (levels, jobs, pinned) and delegates.
struct Spy<E> {
    inner: E,
    calls: Arc<AtomicUsize>,
    max_jobs: Arc<AtomicUsize>,
    pinned: Arc<AtomicUsize>,
}

impl<E: ParallelExecutor> ParallelExecutor for Spy<E> {
    fn execute(&mut self, jobs: usize, job: &(dyn Fn(usize) + Sync)) {
        self.inner.execute(jobs, job);
    }
    fn execute_pinned(&mut self, jobs: usize, pinned: usize, job: &(dyn Fn(usize) + Sync)) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.max_jobs.fetch_max(jobs, Ordering::Relaxed);
        self.pinned.fetch_add(pinned, Ordering::Relaxed);
        self.inner.execute_pinned(jobs, pinned, job);
    }
    fn workers(&self) -> usize {
        self.inner.workers()
    }
}

#[test]
fn parallel_equals_sequential_bit_exact() {
    let mut audible = 0;
    for seed in 0..10u64 {
        for block in [64, 333, MAX_BLOCK] {
            let reference = render(seed, block, FRAMES, None, None, &mut |_| {});
            audible += reference.iter().any(|s| *s != 0.0) as usize;
            for workers in [0usize, 1, 2, 4, 8] {
                let exec: Box<dyn ParallelExecutor> = if workers == 0 {
                    Box::new(SequentialExecutor)
                } else {
                    Box::new(Threaded(workers))
                };
                let out = render(seed, block, FRAMES, Some(exec), None, &mut |_| {});
                if let Some(i) = first_diff(&reference, &out) {
                    panic!(
                        "seed {seed}, block {block}, {workers} workers: sample {i} differs \
                         ({} vs {})",
                        reference[i], out[i]
                    );
                }
            }
        }
    }
    assert!(audible >= 20, "only {audible}/30 renders make sound");
}

/// Complex-warped clips: their tracks own their stretcher voices, run on the workers like
/// the others (nothing is pinned to the calling thread any more, perf audit #8) and still
/// render bit-identically.
#[test]
fn complex_warp_tracks_run_on_workers_bit_exact() {
    use ether_core::graph::ClipContentDesc;
    use ether_core::protocol::model::WarpMode;
    let mut complex_seen = 0;
    for seed in 20..26u64 {
        let mut p = ether_core::create(config());
        p.handle.set_stretcher_factory(stretch().unwrap());
        complex_seen += build(seed, &mut p.handle, true)
            .desc
            .tracks
            .iter()
            .flat_map(|t| &t.clips)
            .filter(|c| {
                matches!(&c.content, ClipContentDesc::Audio { warp: Some(w), .. }
                    if w.mode == WarpMode::Complex)
            })
            .count();
        let reference = render(seed, 256, FRAMES, None, stretch(), &mut |_| {});
        let calls = Arc::new(AtomicUsize::new(0));
        let pinned = Arc::new(AtomicUsize::new(0));
        let spy = Spy {
            inner: Threaded(4),
            calls: calls.clone(),
            max_jobs: Arc::new(AtomicUsize::new(0)),
            pinned: pinned.clone(),
        };
        let out = render(
            seed,
            256,
            FRAMES,
            Some(Box::new(spy)),
            stretch(),
            &mut |_| {},
        );
        assert_eq!(first_diff(&reference, &out), None, "seed {seed}");
        assert_eq!(
            pinned.load(Ordering::Relaxed),
            0,
            "seed {seed}: pinned jobs"
        );
    }
    assert!(
        complex_seen > 0,
        "no generated project had a Complex-warped clip"
    );
}

/// The engine really dispatches levels with several jobs through the executor.
#[test]
fn levels_are_dispatched_in_parallel() {
    let calls = Arc::new(AtomicUsize::new(0));
    let max_jobs = Arc::new(AtomicUsize::new(0));
    let spy = Spy {
        inner: Threaded(2),
        calls: calls.clone(),
        max_jobs: max_jobs.clone(),
        pinned: Arc::new(AtomicUsize::new(0)),
    };
    render(
        3,
        MAX_BLOCK,
        4 * MAX_BLOCK,
        Some(Box::new(spy)),
        None,
        &mut |_| {},
    );
    assert!(calls.load(Ordering::Relaxed) > 0);
    assert!(max_jobs.load(Ordering::Relaxed) >= 4);
}

/// Jobs run in any order: reversing the claim order changes nothing.
#[test]
fn job_order_does_not_matter() {
    struct Reversed;
    impl ParallelExecutor for Reversed {
        fn execute(&mut self, jobs: usize, job: &(dyn Fn(usize) + Sync)) {
            for i in (0..jobs).rev() {
                job(i);
            }
        }
        fn workers(&self) -> usize {
            1
        }
    }
    for seed in 40..46u64 {
        let reference = render(seed, 128, FRAMES / 2, None, None, &mut |_| {});
        let out = render(
            seed,
            128,
            FRAMES / 2,
            Some(Box::new(Reversed)),
            None,
            &mut |_| {},
        );
        assert_eq!(first_diff(&reference, &out), None, "seed {seed}");
    }
}

/// Engine with `tracks` instrument tracks (a voiceless saw, asleep after its first block),
/// with a filter after it when `filters` (effects that never report silence), dispatching
/// through a spy that counts executor calls.
fn stopped_rig(
    filters: bool,
) -> (
    ether_core::Engine,
    ether_core::EngineHandle,
    Arc<AtomicUsize>,
) {
    use ether_core::protocol::model::TrackKind;
    let calls = Arc::new(AtomicUsize::new(0));
    let spy = Spy {
        inner: Threaded(2),
        calls: calls.clone(),
        max_jobs: Arc::new(AtomicUsize::new(0)),
        pinned: Arc::new(AtomicUsize::new(0)),
    };
    let mut p = ether_core::create(config());
    let mut tracks = vec![track(tid(1), TrackKind::Master, None)];
    for i in 0..4u128 {
        let mut t = track(tid(100 + i), TrackKind::Midi, Some(tid(1)));
        t.chain
            .push(entry(p.handle.add_node(Box::new(Saw::new(1.0))).unwrap()));
        if filters {
            t.chain.push(entry(
                p.handle.add_node(Box::new(Filter::new(0.5, 0))).unwrap(),
            ));
        }
        tracks.push(t);
    }
    p.handle
        .publish(ether_core::RenderGraphDesc {
            version: 1,
            tracks,
            ..Default::default()
        })
        .unwrap();
    p.engine.set_executor(Box::new(spy));
    (p.engine, p.handle, calls)
}

fn block(e: &mut ether_core::Engine) {
    let (mut l, mut r) = (vec![0.0f32; 256], vec![0.0f32; 256]);
    let mut outs: [&mut [f32]; 2] = [&mut l, &mut r];
    e.process(&[], &mut outs, 256);
}

/// Stopped, the engine still dispatches while any device works (tails, plugins that never
/// sleep: perf audit #2) ...
#[test]
fn stopped_engine_dispatches_while_devices_work() {
    let (mut engine, _h, calls) = stopped_rig(true);
    for _ in 0..8 {
        block(&mut engine);
    }
    assert!(calls.load(Ordering::Relaxed) >= 8);
}

/// ... and once every node is asleep (nothing monitored) runs the now trivial jobs on the
/// calling thread, so an idle engine wakes no worker; playing dispatches again.
#[test]
fn stopped_engine_parks_workers_once_everything_sleeps() {
    let (mut engine, h, calls) = stopped_rig(false);
    for _ in 0..2 {
        block(&mut engine);
    }
    let woken = calls.load(Ordering::Relaxed);
    for _ in 0..8 {
        block(&mut engine);
    }
    assert_eq!(
        calls.load(Ordering::Relaxed),
        woken,
        "an asleep engine woke the workers"
    );
    let mut h = h;
    h.transport(ether_core::TransportControl::Play).unwrap();
    for _ in 0..8 {
        block(&mut engine);
    }
    assert!(calls.load(Ordering::Relaxed) > woken);
}

/// A stale pad-node key (its slot reused by a live node of another track: the compiler
/// tolerates unknown pad nodes) next to the live key, processed in parallel: the stale key
/// only reads the slot's generation (`NodeTable::get`), the live node plays, and the
/// result is bit-identical to sequential processing.
#[test]
fn stale_pad_key_sharing_a_live_slot_is_harmless() {
    use ether_core::graph::{PadDesc, RackDesc, RenderGraphDesc};
    use ether_core::protocol::model::{DrumPadId, TrackKind, Ulid};

    let render_with = |exec: Option<Box<dyn ParallelExecutor>>| {
        let mut p = ether_core::create(config());
        let h = &mut p.handle;
        let stale = h.add_node(Box::new(Saw::new(1.0))).unwrap();
        h.remove_node(stale).unwrap();
        let live = h.add_node(Box::new(Saw::new(1.01))).unwrap();
        assert_eq!(live.index, stale.index);
        let rack = h.add_node(Box::new(Clip(2.0))).unwrap();
        let pad_saw = h.add_node(Box::new(Saw::new(0.99))).unwrap();
        let mut rng = Rng::new(5);
        let mut a = track(tid(10), TrackKind::Midi, Some(tid(1)));
        a.chain.push(entry(live));
        a.clips.push(midi_clip(&mut rng, 1, &[60, 64, 67]));
        let mut b = track(tid(11), TrackKind::Midi, Some(tid(1)));
        b.chain.push(entry(rack));
        b.racks.push(RackDesc {
            rack,
            pads: [(stale, 36u8), (pad_saw, 37)]
                .into_iter()
                .enumerate()
                .map(|(k, (node, note))| PadDesc {
                    pad: DrumPadId(Ulid(k as u128 + 1)),
                    note,
                    choke_group: None,
                    chain: vec![entry(node)],
                    volume: 1.0,
                    pan: 0.0,
                    mute: false,
                })
                .collect(),
        });
        b.clips.push(midi_clip(&mut rng, 2, &[36, 37]));
        let desc = RenderGraphDesc {
            tracks: vec![track(tid(1), TrackKind::Master, None), a, b],
            ..Default::default()
        };
        p.handle.publish(desc).unwrap();
        p.handle
            .transport(ether_core::TransportControl::Play)
            .unwrap();
        if let Some(e) = exec {
            p.engine.set_executor(e);
        }
        let mut out = Vec::new();
        let (mut l, mut r) = (vec![0.0f32; 128], vec![0.0f32; 128]);
        for _ in 0..1500 {
            let mut outs: [&mut [f32]; 2] = [&mut l, &mut r];
            p.engine.process(&[], &mut outs, 128);
            out.extend_from_slice(&l);
            out.extend_from_slice(&r);
            p.gc.collect();
        }
        out
    };
    let reference = render_with(None);
    assert!(reference.iter().any(|s| *s != 0.0));
    for _ in 0..5 {
        let out = render_with(Some(Box::new(Threaded(4))));
        assert_eq!(first_diff(&reference, &out), None);
    }
}

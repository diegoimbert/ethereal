//! Multicore (`multicore` node, CONTRACTS.md §11.7): parallel processing is bit-identical
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

/// Complex-warped clips: their tracks are pinned to the calling thread (the stretchers
/// live in the engine-wide warp state) and still render bit-identically.
#[test]
fn pinned_tracks_are_bit_exact_and_run_on_the_caller() {
    let mut pinned_seen = 0;
    for seed in 20..26u64 {
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
        pinned_seen += pinned.load(Ordering::Relaxed);
    }
    assert!(pinned_seen > 0, "no generated project had a pinned track");
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

//! Multicore with the real worker pool (`ether_native::workers`): bit-identical to
//! sequential processing on randomized projects and on a project of real built-in devices,
//! across worker counts 0/1/2/4/8 and block sizes, and on the #196 shape (1, 2, N workers);
//! more threads than cores never deadlock.

use crate::multicore_real;

use std::time::{Duration, Instant};

use ether_core::parallel::{ParallelExecutor, SequentialExecutor};
use ether_native::rt::enable_flush_denormals;
use ether_native::workers::physical_cores;
use multicore_real::graphs;
use multicore_real::*;

#[test]
fn pool_equals_sequential_on_random_projects() {
    // Workers run with flush-to-zero, like the host's audio thread: match it here.
    enable_flush_denormals();
    let frames = graphs::SR as usize / 2;
    for seed in 100..108u64 {
        for block in [64, 200, graphs::MAX_BLOCK] {
            let reference = graphs::render(seed, block, frames, None, stretch(), &mut |_| {});
            for workers in [0usize, 1, 2, 4, 8] {
                let out = graphs::render(
                    seed,
                    block,
                    frames,
                    Some(Box::new(pool(workers))),
                    stretch(),
                    &mut |_| {},
                );
                if let Some(i) = first_diff(&reference, &out) {
                    panic!("seed {seed}, block {block}, {workers} workers: sample {i} differs");
                }
            }
        }
    }
}

#[test]
fn pool_equals_sequential_with_real_devices() {
    enable_flush_denormals();
    let blocks = 120;
    let mut seq = engine(24, Some(Box::new(SequentialExecutor)));
    let (reference, _) = run(&mut seq.engine, blocks);
    assert!(reference.iter().any(|s| s.abs() > 1e-3), "silent project");
    for workers in [1usize, 2, 4, 8] {
        let mut par = engine(24, Some(Box::new(pool(workers))));
        let (out, _) = run(&mut par.engine, blocks);
        assert_eq!(first_diff(&reference, &out), None, "{workers} workers");
    }
}

/// More workers than cores (oversubscribed, preempted mid-job): every dispatch still
/// completes, results stay exact, and the whole run is bounded in time.
#[test]
fn oversubscribed_pool_never_deadlocks() {
    enable_flush_denormals();
    let workers = (physical_cores() * 2 + 2).min(32);
    let start = Instant::now();

    // Many tiny dispatches (the worst case for the spin/park hand-off).
    let mut p = pool(workers);
    let sum = std::sync::atomic::AtomicUsize::new(0);
    for round in 0..3000 {
        let jobs = 2 + round % 23;
        p.execute_pinned(jobs, round % 3, &|i| {
            sum.fetch_add(i + 1, std::sync::atomic::Ordering::Relaxed);
        });
    }
    let want: usize = (0..3000).map(|r| (2 + r % 23) * (3 + r % 23) / 2).sum();
    assert_eq!(sum.into_inner(), want);
    drop(p);

    // A real render.
    let blocks = 60;
    let (reference, _) = run(&mut engine(16, None).engine, blocks);
    let (out, _) = run(
        &mut engine(16, Some(Box::new(pool(workers)))).engine,
        blocks,
    );
    assert_eq!(first_diff(&reference, &out), None);

    // Pools come and go (threads joined, nothing left parked).
    for _ in 0..20 {
        let mut p = pool(workers);
        p.execute(8, &|_| {});
    }
    assert!(
        start.elapsed() < Duration::from_secs(120),
        "took {:?}",
        start.elapsed()
    );
}

/// The #196 shape (a light drum track keying the heaviest track, groups chained into a
/// serial master chain) on the real pool: 1, 2 and N workers render the sequential bits.
#[test]
fn pool_equals_sequential_on_the_196_shape() {
    enable_flush_denormals();
    let frames = graphs::SR as usize;
    let n = physical_cores().saturating_sub(1).clamp(3, 16);
    for block in [64, 256] {
        let reference = graphs::render_196(block, frames, None);
        assert!(reference.iter().any(|s| *s != 0.0), "silent project");
        for workers in [1usize, 2, n] {
            let out = graphs::render_196(block, frames, Some(Box::new(pool(workers))));
            assert_eq!(
                first_diff(&reference, &out),
                None,
                "{workers} workers, block {block}"
            );
        }
    }
}

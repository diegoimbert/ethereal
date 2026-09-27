//! The parallel path never allocates or frees, on **any** thread: the audio thread
//! dispatching levels, the workers running track jobs (drum racks, sidechains, latent
//! devices, sends, groups), pinned Complex-warp jobs, snapshot swaps and locates.
//!
//! `assert_no_alloc` alone checks only the calling thread (and only warns when the
//! workspace unifies its `warn_debug` feature), so the global allocator here also counts
//! every allocation process-wide while a block renders. This file holds a single test so
//! no other test thread allocates meanwhile.

mod multicore_real;

use std::alloc::{GlobalAlloc, Layout};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use ether_core::protocol::model::Beats;
use ether_core::{Engine, EngineParts, TransportControl, create};
use ether_native::rt::enable_flush_denormals;
use multicore_real::*;

static ARMED: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);

/// `AllocDisabler` (per-thread `assert_no_alloc`) + a process-wide counter while armed.
struct Counting;

// SAFETY: forwards every call unchanged to `AllocDisabler` (itself forwarding to
// `System`); only counts.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { AllocDisabler.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ARMED.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { AllocDisabler.dealloc(ptr, layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { AllocDisabler.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if ARMED.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { AllocDisabler.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static A: Counting = Counting;

fn block(engine: &mut Engine, l: &mut [f32], r: &mut [f32]) {
    let mut outs: [&mut [f32]; 2] = [l, r];
    ARMED.store(true, Ordering::SeqCst);
    assert_no_alloc(|| engine.process(&[], &mut outs, BLOCK));
    ARMED.store(false, Ordering::SeqCst);
}

#[test]
fn parallel_process_never_allocates_on_any_thread() {
    enable_flush_denormals();
    let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    // The counter sees allocations of other threads.
    ARMED.store(true, Ordering::SeqCst);
    std::thread::spawn(|| drop(std::hint::black_box(vec![1u8; 64])))
        .join()
        .unwrap();
    ARMED.store(false, Ordering::SeqCst);
    assert!(ALLOCS.load(Ordering::SeqCst) > 0);

    // Real devices: synths, EQ, compressors (some sidechained), reverb/delay, drum racks.
    let mut real = engine(32, Some(Box::new(pool(4))));
    for _ in 0..4 {
        block(&mut real.engine, &mut l, &mut r);
        real.gc.collect();
    }
    ALLOCS.store(0, Ordering::SeqCst);
    let mut heard = false;
    for i in 0..200 {
        if i == 100 {
            real.handle
                .transport(TransportControl::Locate {
                    position: Beats(2.0),
                })
                .unwrap();
        }
        block(&mut real.engine, &mut l, &mut r);
        real.gc.collect();
        heard |= l.iter().any(|s| s.abs() > 1e-3);
    }
    assert_eq!(ALLOCS.load(Ordering::SeqCst), 0, "real-device project");
    assert!(heard, "silent");

    // Randomized projects (pinned Complex-warp tracks, a snapshot swap, live params).
    for seed in [3u64, 21, 22, 23] {
        let EngineParts {
            mut engine,
            mut handle,
            mut gc,
        } = create(multicore_real::graphs::config());
        handle.set_stretcher_factory(stretch().unwrap());
        let project = multicore_real::graphs::build(seed, &mut handle, true);
        handle.publish(project.desc.clone()).unwrap();
        handle.transport(TransportControl::Play).unwrap();
        engine.set_executor(Box::new(pool(3)));
        for _ in 0..4 {
            block(&mut engine, &mut l, &mut r);
            gc.collect();
            handle.collect_stretchers();
        }
        ALLOCS.store(0, Ordering::SeqCst);
        for i in 0..150 {
            if i == 50 {
                handle.publish(project.swapped.clone()).unwrap();
            }
            if i == 100 {
                for c in &project.live {
                    handle.set_param(*c).unwrap();
                }
            }
            block(&mut engine, &mut l, &mut r);
            gc.collect();
            handle.collect_stretchers();
        }
        assert_eq!(ALLOCS.load(Ordering::SeqCst), 0, "seed {seed}");
    }
}

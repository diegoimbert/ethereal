//! Multicore benchmark (ignored by default; short and bounded):
//! `cargo test -p ether-native --release --test multicore_bench -- --ignored --nocapture`.
//!
//! 64 MIDI tracks (synth or sampler drum rack → EQ → compressor, some sidechained →
//! reverb/delay), 4 groups, 2 returns, limiter on master; 2 s of audio at 256-frame blocks
//! per worker count. Prints the time spent in `Engine::process` and the speedup over the
//! sequential engine, and checks every run is bit-identical to the sequential one.

mod multicore_real;

use ether_core::parallel::SequentialExecutor;
use ether_native::rt::enable_flush_denormals;
use ether_native::workers::physical_cores;
use multicore_real::*;

#[test]
#[ignore = "benchmark: run explicitly with --ignored --nocapture"]
fn bench_64_tracks() {
    enable_flush_denormals();
    let tracks = 64;
    let blocks = 2 * SR as usize / BLOCK;
    let audio = blocks as f64 * BLOCK as f64 / SR as f64;
    let (reference, seq) = {
        let mut p = engine(tracks, Some(Box::new(SequentialExecutor)));
        run(&mut p.engine, 8); // warm up
        let mut p = engine(tracks, Some(Box::new(SequentialExecutor)));
        run(&mut p.engine, blocks)
    };
    println!(
        "{tracks} tracks, {blocks} blocks of {BLOCK} ({audio:.1} s audio), {} physical cores",
        physical_cores()
    );
    println!(
        "workers 0: {:>7.1} ms  ({:.1}x realtime)",
        seq.as_secs_f64() * 1e3,
        audio / seq.as_secs_f64()
    );
    let max = physical_cores().saturating_sub(1).clamp(1, 8);
    let mut counts: Vec<usize> = [1, 2, 4, max].into_iter().filter(|&w| w <= max).collect();
    counts.dedup();
    for workers in counts {
        let mut p = engine(tracks, Some(Box::new(pool(workers))));
        let (out, t) = run(&mut p.engine, blocks);
        assert_eq!(first_diff(&reference, &out), None, "{workers} workers");
        println!(
            "workers {workers}: {:>7.1} ms  ({:.1}x realtime, speedup {:.2}x)",
            t.as_secs_f64() * 1e3,
            audio / t.as_secs_f64(),
            seq.as_secs_f64() / t.as_secs_f64()
        );
    }
}

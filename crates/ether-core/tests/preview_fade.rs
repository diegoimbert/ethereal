//! `media-preview`: the preview voice fades out on stop and replace (no click), keeps the new
//! preview's attack on replace, retires every source to the GC and never allocates.

mod common;

use std::sync::Arc;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use common::*;
use ether_core::preview::{FADE_FRAMES, PreviewControl};
use ether_core::{Engine, EngineOutputs, RenderGraphDesc, create};

#[cfg(debug_assertions)]
#[global_allocator]
static A: AllocDisabler = AllocDisabler;

/// Small blocks so controls land between blocks, like a web render quantum.
const Q: usize = 64;

fn setup() -> ether_core::EngineParts {
    let mut p = create(config());
    p.handle
        .publish(RenderGraphDesc {
            version: 1,
            tracks: vec![master()],
            ..Default::default()
        })
        .unwrap();
    p
}

fn dc(level: f32, frames: usize) -> Arc<dyn ether_core::AudioSource> {
    Arc::new(MemSource(vec![level; frames]))
}

fn play(id: u64, source: Arc<dyn ether_core::AudioSource>, gain: f32) -> PreviewControl {
    PreviewControl::Play { id, source, gain }
}

/// Render `frames` in `Q` blocks without allocating; returns the left channel.
fn run(engine: &mut Engine, frames: usize) -> Vec<f32> {
    let mut left = vec![0.0; frames];
    let mut right = vec![0.0; frames];
    for (l, r) in left.chunks_mut(Q).zip(right.chunks_mut(Q)) {
        let n = l.len();
        let mut outs: [&mut [f32]; 2] = [l, r];
        assert_no_alloc(|| engine.process(&[], &mut outs, n));
    }
    left
}

fn max_step(signal: &[f32]) -> f32 {
    signal
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

#[test]
fn stop_fades_out_without_a_click() {
    let mut p = setup();
    let level = 0.8;
    p.handle.preview(play(1, dc(level, 48_000), 1.0)).unwrap();
    let before = run(&mut p.engine, 1000);
    p.handle.preview(PreviewControl::Stop).unwrap();
    let after = run(&mut p.engine, 1000);
    let joined = [&before[1..], &after[..]].concat();
    // A hard cut would be a 0.8 step; the fade is `level / FADE_FRAMES` per sample.
    let bound = 1.5 * level / FADE_FRAMES as f32;
    assert!(
        max_step(&joined) <= bound,
        "max step {} > {bound}",
        max_step(&joined)
    );
    assert!(after[0] > 0.7, "the fade starts from the playing level");
    let fade = FADE_FRAMES as usize;
    assert!(after[fade..].iter().all(|&s| s == 0.0), "silent after the fade");
    let mut out = EngineOutputs::default();
    p.handle.poll(&mut out);
    assert_eq!(out.preview_ended, None, "a stop is never reported");
    assert!(p.gc.collect() >= 1, "the stopped source is retired");
}

#[test]
fn replace_crossfades_and_keeps_the_new_attack() {
    let mut p = setup();
    p.handle.preview(play(1, dc(0.8, 48_000), 1.0)).unwrap();
    run(&mut p.engine, 500);
    // Replaced by silence: only the old one's fade-out is heard, without a click.
    p.handle.preview(play(2, dc(0.0, 48_000), 1.0)).unwrap();
    let out = run(&mut p.engine, 1000);
    assert!(max_step(&out) <= 1.5 * 0.8 / FADE_FRAMES as f32);
    assert!(out[FADE_FRAMES as usize..].iter().all(|&s| s == 0.0));

    // Replaced by a 0.3 DC at gain 0.5: the new one starts at full level at once.
    p.handle.preview(play(3, dc(0.6, 48_000), 1.0)).unwrap();
    run(&mut p.engine, 500);
    p.handle.preview(play(4, dc(0.3, 48_000), 0.5)).unwrap();
    let out = run(&mut p.engine, 1000);
    assert!((out[0] - (0.15 + 0.6 * (1.0 - 1.0 / FADE_FRAMES as f32))).abs() < 1e-4);
    assert!(
        out[FADE_FRAMES as usize..]
            .iter()
            .all(|&s| (s - 0.15).abs() < 1e-6)
    );
    let mut o = EngineOutputs::default();
    p.handle.poll(&mut o);
    assert_eq!(o.preview_ended, None, "replaces are never reported");
}

#[test]
fn fades_with_the_preview_gain_and_short_sources() {
    let mut p = setup();
    // Gain 0.5: the fade ramps down from 0.5 * level.
    p.handle.preview(play(1, dc(1.0, 48_000), 0.5)).unwrap();
    run(&mut p.engine, 100);
    p.handle.preview(PreviewControl::Stop).unwrap();
    let out = run(&mut p.engine, 400);
    assert!(out[0] <= 0.5 && out[0] > 0.49);
    assert!(max_step(&out) <= 1.5 * 0.5 / FADE_FRAMES as f32);

    // A source that ends during its fade just stops there (and isn't reported: it was
    // stopped before its end).
    p.handle.preview(play(2, dc(0.5, 100), 1.0)).unwrap();
    run(&mut p.engine, 64);
    p.handle.preview(PreviewControl::Stop).unwrap();
    let out = run(&mut p.engine, 400);
    assert!(out[..36].iter().all(|&s| s > 0.0));
    assert!(out[36..].iter().all(|&s| s == 0.0));
    let mut o = EngineOutputs::default();
    p.handle.poll(&mut o);
    assert_eq!(o.preview_ended, None);
}

#[test]
fn rapid_replaces_retire_every_source_without_allocating() {
    let mut p = setup();
    // Replace faster than the fade: each control evicts the previous fading source.
    for id in 1..=20u64 {
        p.handle.preview(play(id, dc(0.1, 10_000), 1.0)).unwrap();
        let out = run(&mut p.engine, Q);
        assert!(
            out.iter().all(|&s| s <= 0.2 + 1e-6),
            "at most the new and one fading source sound"
        );
    }
    // The last one plays to its end and is the only natural end reported.
    run(&mut p.engine, 10_000 + 2 * FADE_FRAMES as usize);
    let mut o = EngineOutputs::default();
    p.handle.poll(&mut o);
    assert_eq!(o.preview_ended, Some(20));
    // Everything is handed to the GC eventually (the retire slots drain one per block).
    run(&mut p.engine, 8 * Q);
    assert!(p.gc.collect() >= 20);
    // Stopping an idle voice is a no-op.
    p.handle.preview(PreviewControl::Stop).unwrap();
    assert!(run(&mut p.engine, 256).iter().all(|&s| s == 0.0));
}

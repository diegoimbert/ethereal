//! base-53 stream tap ("listen on <peer>", docs/COLLAB.md §9.1): the tap carries master +
//! metronome/count-in and never the browser preview voice; it writes block headers that
//! tile the engine sample clock, and never allocates.

mod common;

use std::sync::Arc;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use common::*;
use ether_core::graph::MetronomeDesc;
use ether_core::preview::PreviewControl;
use ether_core::protocol::model::MetronomeSound;
use ether_core::stream_tap::{StreamTapReader, stream_tap_ring};
use ether_core::{Engine, EngineParts, RenderGraphDesc, TransportControl, create};

#[cfg(debug_assertions)]
#[global_allocator]
static A: AllocDisabler = AllocDisabler;

const Q: usize = 64;
const FRAMES: usize = 48_000;

fn start() -> EngineParts {
    let mut p = create(config());
    p.handle
        .publish(RenderGraphDesc {
            version: 1,
            metronome: true,
            click: MetronomeDesc {
                volume: 0.5,
                accent: true,
                sound: MetronomeSound::Classic,
                count_in_end: None,
            },
            tracks: vec![master()],
            ..Default::default()
        })
        .unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    p
}

/// Render `frames` in `Q` blocks without allocating; returns the left output channel.
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

/// Drain the tap: (left channel, block headers).
fn drain(r: &mut StreamTapReader) -> (Vec<f32>, Vec<ether_core::stream_tap::StreamBlock>) {
    let mut blocks = Vec::new();
    while let Ok(b) = r.blocks.pop() {
        blocks.push(b);
    }
    let mut left = Vec::new();
    while let Ok(l) = r.audio.pop() {
        left.push(l);
        r.audio.pop().expect("interleaved stereo");
    }
    (left, blocks)
}

#[test]
fn the_tap_has_the_metronome_but_never_the_preview() {
    // Reference: the same engine without a preview.
    let mut reference = start();
    let expected = run(&mut reference.engine, FRAMES);
    assert!(expected.iter().any(|&s| s != 0.0), "clicks are audible");

    let mut p = start();
    let (writer, mut reader) = stream_tap_ring(2 * FRAMES);
    p.handle.set_stream_tap(Some(writer)).unwrap();
    p.handle
        .preview(PreviewControl::Play {
            id: 1,
            source: Arc::new(MemSource(vec![0.25; 2 * FRAMES])),
            gain: 1.0,
        })
        .unwrap();
    let heard = run(&mut p.engine, FRAMES);
    let (tapped, blocks) = drain(&mut reader);

    assert_ne!(heard, expected, "the host hears its preview");
    assert_eq!(tapped, expected, "the stream is master + metronome only");
    // Headers tile the sample clock with the transport state.
    assert_eq!(
        blocks.iter().map(|b| b.frames as usize).sum::<usize>(),
        FRAMES
    );
    let mut t = blocks[0].sample_time;
    for b in &blocks {
        assert_eq!(b.sample_time, t);
        assert!(b.playing && !b.gap);
        t += u64::from(b.frames);
    }
    assert!(blocks.windows(2).all(|w| w[1].position > w[0].position));

    // Removing the tap stops the copy (the writer is retired, not dropped in `process`).
    p.handle.set_stream_tap(None).unwrap();
    run(&mut p.engine, Q);
    assert!(drain(&mut reader).0.is_empty());
}

#[test]
fn a_full_ring_skips_whole_blocks_and_flags_the_gap() {
    let mut p = start();
    let (writer, mut reader) = stream_tap_ring(4 * Q);
    p.handle.set_stream_tap(Some(writer)).unwrap();
    run(&mut p.engine, 8 * Q);
    let (audio, blocks) = drain(&mut reader);
    assert_eq!(audio.len(), 4 * Q, "only what fits");
    assert!(blocks.iter().all(|b| !b.gap));
    run(&mut p.engine, Q);
    let (audio, blocks) = drain(&mut reader);
    assert_eq!(audio.len(), Q);
    assert!(blocks[0].gap, "the reader learns it missed blocks");
}

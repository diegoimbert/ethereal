//! v0.2 engine hooks (contracts-3): the analysis channel (implemented), and the pre-wired
//! placeholders (frozen tracks, rack chains, modulation, input taps, VCAs) keep v0.1
//! behaviour until their nodes land. Nodes extend these with real behaviour tests.

mod common;

use assert_no_alloc::assert_no_alloc;
use common::*;
use ether_core::analysis::{ANALYSIS_HZ, AnalysisFrame, AnalysisKind};
use ether_core::graph::RenderGraphDesc;
use ether_core::protocol::model::{InputTap, TrackKind};
use ether_core::{
    AudioBuffers, InputTapDesc, Node, PrepareConfig, ProcessContext, ProcessStatus, create,
};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

/// Pass-through node that reports how many blocks it processed as a `Levels` frame.
struct Analyzer {
    blocks: f32,
}

impl Node for Analyzer {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        self.blocks += 1.0;
        for (o, i) in audio.outputs.iter_mut().zip(audio.inputs.iter()) {
            o[..ctx.frames].copy_from_slice(&i[..ctx.frames]);
        }
        ProcessStatus::Continue
    }
    fn has_analysis(&self) -> bool {
        true
    }
    fn analysis(&mut self, out: &mut AnalysisFrame) -> bool {
        out.begin(AnalysisKind::Levels);
        out.push(self.blocks);
        true
    }
}

#[test]
fn analysis_frames_are_throttled_and_allocation_free() {
    let mut parts = create(config());
    let key = parts
        .handle
        .add_node(Box::new(Analyzer { blocks: 0.0 }))
        .unwrap();
    let t = with_chain(track(tid(2), TrackKind::Audio, Some(tid(1))), &[key]);
    parts
        .handle
        .publish(RenderGraphDesc {
            tracks: vec![master(), t],
            ..Default::default()
        })
        .unwrap();
    // One second of audio.
    let blocks = SR as usize / BLOCK;
    let mut out = [vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]];
    for _ in 0..blocks {
        let (l, r) = out.split_at_mut(1);
        let mut outs: [&mut [f32]; 2] = [&mut l[0], &mut r[0]];
        assert_no_alloc(|| parts.engine.process(&[], &mut outs, BLOCK));
    }
    let mut frames = Vec::new();
    parts.handle.poll_analysis(|f| frames.push(*f));
    // ~ANALYSIS_HZ frames per second from the node, in order, tagged with its key.
    assert!(
        frames.len() as u32 >= ANALYSIS_HZ - 2 && frames.len() as u32 <= ANALYSIS_HZ + 1,
        "{} frames",
        frames.len()
    );
    assert!(
        frames
            .iter()
            .all(|f| f.node == key && f.kind == AnalysisKind::Levels)
    );
    assert!(
        frames
            .windows(2)
            .all(|w| w[0].values()[0] < w[1].values()[0])
    );
    // Removing the node stops its frames.
    parts.handle.remove_node(key).unwrap();
    for _ in 0..blocks {
        let (l, r) = out.split_at_mut(1);
        let mut outs: [&mut [f32]; 2] = [&mut l[0], &mut r[0]];
        parts.engine.process(&[], &mut outs, BLOCK);
    }
    let mut after = 0;
    parts.handle.poll_analysis(|_| after += 1);
    assert!(after <= 1, "{after}");
}

#[test]
fn placeholder_hooks_keep_v01_behaviour() {
    // An input tap (placeholder: not mixed) and a VCA assignment (placeholder: unity) leave
    // the render unchanged; the tap orders its source first without a cycle error.
    let mut parts = create(config());
    let a = track(tid(2), TrackKind::Audio, Some(tid(1)));
    let mut b = track(tid(3), TrackKind::Audio, Some(tid(1)));
    b.input_tap = Some(InputTapDesc {
        track: tid(2),
        point: InputTap::PostFader,
    });
    b.monitor = true;
    b.vca = Some(tid(9));
    parts
        .handle
        .publish(RenderGraphDesc {
            tracks: vec![master(), a, b],
            ..Default::default()
        })
        .unwrap();
    let (l, r) = render(&mut parts.engine, 4 * BLOCK, BLOCK);
    assert!(l.iter().chain(&r).all(|s| *s == 0.0));
}

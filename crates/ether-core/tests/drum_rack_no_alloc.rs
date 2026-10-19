//! Drum rack audio paths never allocate: pad routing, choke groups (fades, chokes across
//! blocks), smoothed pad mix, snapshot swaps that inherit pad state, transport jumps.

mod common;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use common::*;
use ether_core::graph::{ChainEntry, PadDesc, RackDesc, RenderGraphDesc};
use ether_core::protocol::model::{Beats, DrumPadId, TrackKind, Ulid};
use ether_core::{
    AudioBuffers, Engine, Node, NodeKey, PrepareConfig, ProcessContext, ProcessStatus,
    TransportControl, create,
};

#[cfg(debug_assertions)]
#[global_allocator]
static A: AllocDisabler = AllocDisabler;

/// The rack node: passes the pads' mix through.
struct Pass;

impl Node for Pass {
    fn prepare(&mut self, _: &PrepareConfig) {}
    fn reset(&mut self) {}
    fn process(
        &mut self,
        _: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        audio.pass_through();
        ProcessStatus::Continue
    }
}

fn run(engine: &mut Engine, blocks: usize, frames: usize) {
    let mut l = vec![0.0f32; frames];
    let mut r = vec![0.0f32; frames];
    for _ in 0..blocks {
        let mut outs: [&mut [f32]; 2] = [&mut l, &mut r];
        assert_no_alloc(|| engine.process(&[], &mut outs, frames));
    }
}

fn pad(id: u128, note: u8, group: Option<u8>, chain: &[NodeKey], volume: f32) -> PadDesc {
    PadDesc {
        pad: DrumPadId(Ulid(id)),
        note,
        choke_group: group,
        chain: chain
            .iter()
            .map(|&node| ChainEntry {
                node,
                enabled: true,
                sidechain: None,
            })
            .collect(),
        volume,
        pan: 0.3,
        mute: false,
    }
}

#[test]
fn drum_rack_paths_never_allocate() {
    let mut p = create(config());
    let (rec_a, _ra) = Recorder::new();
    let (rec_b, _rb) = Recorder::new();
    let rec_a = p.handle.add_node(Box::new(rec_a)).unwrap();
    let rec_b = p.handle.add_node(Box::new(rec_b)).unwrap();
    let dc_a = p.handle.add_node(Box::new(Dc(0.3))).unwrap();
    let dc_b = p.handle.add_node(Box::new(Dc(0.2))).unwrap();
    let slow = p.handle.add_node(Box::new(Delay::new(77))).unwrap();
    let rack = p.handle.add_node(Box::new(Pass)).unwrap();
    let build = |version: u64, volume: f32, mute: bool| {
        let mut t = with_chain(track(tid(2), TrackKind::Midi, Some(tid(1))), &[rack]);
        let mut b = pad(2, 38, Some(1), &[rec_b, dc_b], volume);
        b.mute = mute;
        t.racks = vec![RackDesc {
            rack,
            pads: vec![pad(1, 36, Some(1), &[rec_a, dc_a, slow], 1.0), b],
        }];
        // Dense alternating hits (chokes land inside and across blocks).
        let notes: Vec<(f64, f64, u8)> = (0..64)
            .map(|i| (i as f64 * 0.013, 0.01, if i % 2 == 0 { 36 } else { 38 }))
            .collect();
        let mut clip = midi_clip(cid(1), 0.0, 4.0, &notes);
        clip.looping = Some((0.0, 1.0));
        t.clips = vec![clip];
        RenderGraphDesc {
            version,
            tracks: vec![master(), t],
            ..Default::default()
        }
    };
    p.handle.publish(build(1, 1.0, false)).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    run(&mut p.engine, 20, BLOCK);
    for v in 2..8u64 {
        // Swaps (inherit), mix changes (ramps) and a locate (reset) while playing.
        p.handle
            .publish(build(v, 0.5 + v as f32 * 0.05, v % 3 == 0))
            .unwrap();
        if v == 5 {
            p.handle
                .transport(TransportControl::Locate {
                    position: Beats(0.25),
                })
                .unwrap();
        }
        run(&mut p.engine, 10, BLOCK);
        run(&mut p.engine, 7, 97);
    }
    p.handle.transport(TransportControl::Stop).unwrap();
    run(&mut p.engine, 4, BLOCK);
}

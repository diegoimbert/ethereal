//! The metronome never allocates on the audio thread (`tempo-metronome`): clicks at a fast
//! tempo in 7/8 with ramps, loop wraps, locates, the count-in and sound/volume changes.

mod common;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use common::*;
use ether_core::graph::MetronomeDesc;
use ether_core::protocol::model::{Beats, MetronomeSound, TempoCurve, TimeSignature, TrackKind};
use ether_core::tempo::{TempoPointDesc, TimeSignatureDesc};
use ether_core::{Engine, RenderGraphDesc, TransportControl, create};

#[cfg(debug_assertions)]
#[global_allocator]
static A: AllocDisabler = AllocDisabler;

fn run(engine: &mut Engine, blocks: usize, frames: usize) {
    let mut l = [0.0f32; BLOCK];
    let mut r = [0.0f32; BLOCK];
    for _ in 0..blocks {
        let mut outs: [&mut [f32]; 2] = [&mut l[..frames], &mut r[..frames]];
        assert_no_alloc(|| engine.process(&[], &mut outs, frames));
    }
}

fn graph(version: u64, sound: MetronomeSound, count_in_end: Option<f64>) -> RenderGraphDesc {
    RenderGraphDesc {
        vcas: Default::default(),
        version,
        tempo: vec![
            TempoPointDesc {
                beat: 0.0,
                bpm: 999.0,
                curve: TempoCurve::Linear,
            },
            TempoPointDesc {
                beat: 6.0,
                bpm: 60.0,
                curve: TempoCurve::Step,
            },
        ],
        signatures: vec![TimeSignatureDesc {
            beat: 0.0,
            signature: TimeSignature {
                numerator: 7,
                denominator: 32,
            },
        }],
        loop_enabled: true,
        loop_start: 1.0,
        loop_end: 7.0,
        metronome: true,
        click: MetronomeDesc {
            volume: 0.7,
            accent: true,
            sound,
            count_in_end,
        },
        tracks: vec![master()],
    }
}

#[test]
fn metronome_never_allocates() {
    let mut p = create(config());
    // A latent track: clicks wait in the pending queue for the PDC latency.
    let delay = p.handle.add_node(Box::new(Delay::new(3000))).unwrap();
    let latent = with_chain(track(tid(2), TrackKind::Audio, Some(tid(1))), &[delay]);
    let mut g = graph(1, MetronomeSound::Classic, None);
    g.tracks.push(latent.clone());
    p.handle.publish(g).unwrap();
    p.handle.transport(TransportControl::Play).unwrap();
    run(&mut p.engine, 400, BLOCK);
    run(&mut p.engine, 400, 37);

    let mut g = graph(2, MetronomeSound::Wood, None);
    g.tracks.push(latent.clone());
    p.handle.publish(g).unwrap();
    p.handle
        .transport(TransportControl::Locate {
            position: Beats(3.3),
        })
        .unwrap();
    run(&mut p.engine, 200, BLOCK);

    // Count-in (metronome off) from before beat 0.
    let mut g = graph(3, MetronomeSound::Beep, Some(2.0));
    g.metronome = false;
    p.handle.publish(g).unwrap();
    p.handle
        .transport(TransportControl::SetRecording { enabled: true })
        .unwrap();
    p.handle
        .transport(TransportControl::Locate {
            position: Beats(-2.0),
        })
        .unwrap();
    run(&mut p.engine, 400, 128);
    p.handle.transport(TransportControl::Stop).unwrap();
    run(&mut p.engine, 50, BLOCK);
    // Keep the collector's garbage out of the audio thread's hands.
    p.gc.collect();
}

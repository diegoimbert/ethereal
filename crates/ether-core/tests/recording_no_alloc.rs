//! The recording hook on the audio thread never allocates or frees: input capture (also
//! with a full ring), live MIDI to monitored tracks, recorded MIDI, loop wraps.

mod common;

use assert_no_alloc::{AllocDisabler, assert_no_alloc};
use common::*;
use ether_core::protocol::model::{BeatRange, Beats, TrackKind};
use ether_core::recording::LiveMidi;
use ether_core::{Engine, RenderGraphDesc, TransportControl, create};

#[cfg(debug_assertions)]
#[global_allocator]
static A: AllocDisabler = AllocDisabler;

fn run(engine: &mut Engine, blocks: usize) {
    let input = [0.25f32; BLOCK];
    let inputs: [&[f32]; 2] = [&input, &input];
    let mut l = vec![0.0; BLOCK];
    let mut r = vec![0.0; BLOCK];
    for _ in 0..blocks {
        let mut outs: [&mut [f32]; 2] = [&mut l, &mut r];
        assert_no_alloc(|| engine.process(&inputs, &mut outs, BLOCK));
    }
}

#[test]
fn recording_hook_never_allocates() {
    let mut p = create(config());
    let mut io = p.handle.take_recording_io().expect("recording io");
    let synth = p.handle.add_node(Box::new(Dc(0.0))).unwrap();

    let mut audio = track(tid(2), TrackKind::Audio, Some(tid(1)));
    audio.armed = true;
    audio.monitor = true;
    audio.audio_input = Some((0, 2));
    let mut keys = with_chain(track(tid(3), TrackKind::Midi, Some(tid(1))), &[synth]);
    keys.armed = true;
    keys.monitor = true;
    p.handle
        .publish(RenderGraphDesc {
            loop_enabled: true,
            loop_start: 0.0,
            loop_end: 0.5,
            tracks: vec![master(), audio, keys],
            ..Default::default()
        })
        .unwrap();
    for c in [
        TransportControl::SetLoop {
            enabled: true,
            region: BeatRange {
                start: Beats(0.0),
                end: Beats(0.5),
            },
        },
        TransportControl::SetRecording { enabled: true },
        TransportControl::Play,
    ] {
        p.handle.transport(c).unwrap();
    }
    run(&mut p.engine, 2);

    // Live MIDI (due now, late, and in the future) while recording.
    for (i, data) in [[0x90, 60, 100], [0x80, 60, 0], [0xb0, 1, 64]]
        .into_iter()
        .enumerate()
    {
        io.midi_in
            .push(LiveMidi {
                sample_time: (i as u64) * 300,
                data,
            })
            .unwrap();
    }
    run(&mut p.engine, 4);
    assert!(io.midi_out.pop().is_ok(), "MIDI recorded");

    // Nobody drains the capture ring: it fills up (> 2 s) and blocks are dropped, still
    // without allocating.
    let blocks = 2 * 48_000 / BLOCK + 50;
    run(&mut p.engine, blocks);
    let mut buf = Vec::new();
    let mut dropped = false;
    while let Some(b) = io.capture.next_block(&mut buf) {
        dropped |= b.dropped;
        buf.clear();
    }
    assert!(dropped, "a full ring drops blocks instead of blocking");
    assert_eq!(p.engine.leaked(), 0);
}

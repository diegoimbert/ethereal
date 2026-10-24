//! Cost of every built-in device type, default settings, playing: ns per stereo sample at
//! 128-frame blocks, noise in and a note every half second (instruments get notes, effects
//! audio). A harness for before/after numbers of DSP optimizations:
//!
//! ```text
//! cargo test --release -p ether-devices --test it device_costs:: -- --ignored --nocapture
//! ```
//! `ETHER_DEVICE_COSTS=Compressor,Delay` limits it to those types (`Debug` names).

use std::time::Instant;

use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType};
use ether_core::{
    AudioBuffers, EventBuffer, EventKind, PrepareConfig, ProcessContext, ProcessEvent,
    TransportInfo,
};
use ether_devices::NoSamples;

const SR: f32 = 48_000.0;
const BLOCK: usize = 128;
const SECONDS: usize = 4;

/// Mean ns per sample over [`SECONDS`] of playback.
fn cost(ty: BuiltinDeviceType) -> f64 {
    let mut d = ether_devices::create(&BuiltinDevice::new(ty), &NoSamples);
    d.prepare(&PrepareConfig {
        sample_rate: SR,
        max_block_size: BLOCK,
        max_events_per_block: 64,
    });
    let frames = SECONDS * SR as usize;
    let mut seed = 5u32;
    let noise: Vec<f32> = (0..frames)
        .map(|_| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 0.5
        })
        .collect();
    let (mut ol, mut or) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    let mut out_events = EventBuffer::with_capacity(256);
    let mut events: Vec<ProcessEvent> = Vec::with_capacity(8);
    let sidechain = d.sidechain_inputs() > 0;
    let mut spent = 0.0;
    let mut pos = 0;
    let mut note = 0u32;
    while pos + BLOCK <= frames {
        events.clear();
        if pos % 24_000 < BLOCK {
            let key = 48 + (note % 12) as u8;
            events.push(ProcessEvent {
                offset: 0,
                kind: EventKind::NoteOn {
                    note_id: note,
                    channel: 0,
                    key,
                    velocity: 0.8,
                },
            });
            note += 1;
        } else if pos % 24_000 < 12_000 + BLOCK && pos % 24_000 >= 12_000 && note > 0 {
            events.push(ProcessEvent {
                offset: 0,
                kind: EventKind::NoteOff {
                    note_id: note - 1,
                    channel: 0,
                    key: 48 + ((note - 1) % 12) as u8,
                    velocity: 0.5,
                },
            });
        }
        let transport = TransportInfo {
            playing: true,
            sample_time: pos as u64,
            bpm: 120.0,
            beats_per_sample: 2.0 / SR as f64,
            position: pos as f64 * 2.0 / SR as f64,
            ..TransportInfo::STOPPED
        };
        let input = &noise[pos..pos + BLOCK];
        let inputs: [&[f32]; 2] = [input, input];
        let mut outputs: [&mut [f32]; 2] = [&mut ol, &mut or];
        let mut ctx = ProcessContext {
            sample_rate: SR,
            frames: BLOCK,
            transport: &transport,
            events: &events,
            out_events: &mut out_events,
        };
        let mut audio = AudioBuffers {
            inputs: &inputs,
            outputs: &mut outputs,
        };
        let t = Instant::now();
        if sidechain {
            d.process_sidechain(&mut ctx, &mut audio, &[input, input]);
        } else {
            d.process(&mut ctx, &mut audio);
        }
        spent += t.elapsed().as_secs_f64();
        std::hint::black_box((&ol, &or));
        out_events.clear();
        pos += BLOCK;
    }
    spent * 1e9 / pos as f64
}

#[test]
#[ignore = "perf harness: run in release with --ignored --nocapture"]
fn every_device_cost() {
    let only: Option<Vec<String>> = std::env::var("ETHER_DEVICE_COSTS")
        .ok()
        .map(|v| v.split(',').map(|s| s.trim().to_owned()).collect());
    let mut rows: Vec<(f64, String)> = BuiltinDeviceType::ALL
        .iter()
        .filter(|t| only.as_ref().is_none_or(|o| o.contains(&format!("{t:?}"))))
        .map(|&t| {
            let best = (0..3).map(|_| cost(t)).fold(f64::INFINITY, f64::min);
            (best, format!("{t:?}"))
        })
        .collect();
    rows.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (ns, name) in rows {
        println!("device cost {name:>22}: {ns:8.2} ns/sample");
    }
}

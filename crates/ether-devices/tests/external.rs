//! External Instrument / External Audio Effect nodes (`external-instrument`, CONTRACTS.md
//! §13.7): return gain, hardware send on outputs 2..4, dry/wet alignment by the `Latency`
//! param, PDC latency, and RT safety. The engine side (`ether_core::hw_io`) is tested in
//! `ether-core/tests/hw_io.rs`.

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType};
use ether_core::{AudioBuffers, Device, EventBuffer, EventKind, PrepareConfig, ProcessContext};
use ether_core::{ProcessEvent, TransportInfo};
use ether_devices::NoSamples;
use ether_devices::external::{
    external_audio_effect as fx, external_instrument as inst, latency_ms, latency_samples,
};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const SR: f32 = 48_000.0;
const BLOCK: usize = 128;

fn make(ty: BuiltinDeviceType) -> Box<dyn Device> {
    let mut d = ether_devices::create(&BuiltinDevice::new(ty), &NoSamples);
    d.prepare(&PrepareConfig {
        sample_rate: SR,
        max_block_size: BLOCK,
        max_events_per_block: 64,
    });
    d
}

/// One block through `process_sidechain` (when `ret` is `Some`) with `outs` output channels.
fn block(
    d: &mut dyn Device,
    input: &[Vec<f32>],
    ret: Option<&[Vec<f32>; 2]>,
    outs: usize,
    events: &[ProcessEvent],
) -> Vec<Vec<f32>> {
    let mut out = vec![vec![0.0f32; BLOCK]; outs];
    let mut out_events = EventBuffer::with_capacity(16);
    let transport = TransportInfo {
        playing: true,
        ..TransportInfo::STOPPED
    };
    let ins: Vec<&[f32]> = input.iter().map(|c| c.as_slice()).collect();
    let mut o: Vec<&mut [f32]> = out.iter_mut().map(|c| c.as_mut_slice()).collect();
    let mut ctx = ProcessContext {
        sample_rate: SR,
        frames: BLOCK,
        transport: &transport,
        events,
        out_events: &mut out_events,
    };
    let mut buffers = AudioBuffers {
        inputs: &ins,
        outputs: &mut o,
    };
    match ret {
        Some([l, r]) => {
            let sc: [&[f32]; 2] = [l, r];
            assert_no_alloc(|| d.process_sidechain(&mut ctx, &mut buffers, &sc));
        }
        None => {
            assert_no_alloc(|| d.process(&mut ctx, &mut buffers));
        }
    }
    out
}

fn ramp(offset: usize) -> Vec<f32> {
    (0..BLOCK)
        .map(|i| (((offset + i) as f32) * 0.013).sin() * 0.5)
        .collect()
}

#[test]
fn latency_param_is_the_node_latency() {
    assert_eq!(latency_samples(10.0, SR), 480);
    assert_eq!(latency_samples(900.0, SR), 24_000, "clamped to 500 ms");
    assert!((latency_ms(480, SR) - 10.0).abs() < 1e-9);
    for (ty, id) in [
        (BuiltinDeviceType::ExternalInstrument, inst::LATENCY),
        (BuiltinDeviceType::ExternalAudioEffect, fx::LATENCY),
    ] {
        let mut d = make(ty);
        assert_eq!(d.latency(), 0);
        d.set_param(id, 12.5);
        assert_eq!(d.latency(), 600, "{ty:?}");
        // Automation / live changes too.
        let ev = [ProcessEvent {
            offset: 0,
            kind: EventKind::Param {
                param: id,
                value: 1.0,
            },
        }];
        let input = vec![vec![0.0; BLOCK]; 2];
        let n_in = d.channels().0 as usize;
        block(d.as_mut(), &input[..n_in], None, 2, &ev);
        assert_eq!(d.latency(), 48);
    }
}

#[test]
fn instrument_plays_the_return_with_its_gain_and_is_silent_without_it() {
    let mut d = make(BuiltinDeviceType::ExternalInstrument);
    assert_eq!(d.channels(), (0, 2));
    let ret = [vec![0.25f32; BLOCK], vec![-0.5f32; BLOCK]];
    let out = block(d.as_mut(), &[], Some(&ret), 2, &[]);
    assert_eq!(out[0][5], 0.25);
    assert_eq!(out[1][5], -0.5);
    d.set_param(inst::GAIN, -6.0);
    let out = block(d.as_mut(), &[], Some(&ret), 2, &[]);
    assert!((out[0][5] - 0.25 * 0.501).abs() < 1e-3);
    // No hardware (web, offline render, unresolved routing): silence.
    let out = block(d.as_mut(), &[], None, 2, &[]);
    assert!(out.iter().flatten().all(|s| *s == 0.0));
}

#[test]
fn effect_writes_its_send_on_outputs_2_and_3() {
    let mut d = make(BuiltinDeviceType::ExternalAudioEffect);
    d.set_param(fx::SEND_GAIN, 6.0);
    let input = vec![vec![0.5f32; BLOCK], vec![-0.25f32; BLOCK]];
    let ret = [vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]];
    let out = block(d.as_mut(), &input, Some(&ret), 4, &[]);
    let g = 10f32.powf(6.0 / 20.0);
    assert!((out[2][3] - 0.5 * g).abs() < 1e-4);
    assert!((out[3][3] + 0.25 * g).abs() < 1e-4);
    // Mix 100 %: only the (silent) return is heard.
    assert!(out[0].iter().chain(&out[1]).all(|s| *s == 0.0));
    // Two outputs only (no engine send): still fine.
    let out = block(d.as_mut(), &input, Some(&ret), 2, &[]);
    assert_eq!(out.len(), 2);
}

/// The acceptance in miniature: the hardware returns the send `lat` samples later; with
/// `Latency` set to that round trip the dry signal lines up with the return, so a 50 % mix
/// is the input delayed by `lat` (no comb filtering) and an inverted return cancels it.
#[test]
fn dry_and_return_are_aligned_by_the_latency_param() {
    let lat = 300usize;
    for (invert, expect_gain) in [(false, 1.0f32), (true, 0.0)] {
        let mut d = make(BuiltinDeviceType::ExternalAudioEffect);
        d.set_param(fx::LATENCY, latency_ms(lat as u32, SR));
        d.set_param(fx::MIX, 50.0);
        d.set_param(fx::INVERT_PHASE, if invert { 1.0 } else { 0.0 });
        assert_eq!(d.latency() as usize, lat);
        // A loop of `lat` samples: the hardware.
        let mut wire: Vec<f32> = vec![0.0; lat];
        let mut played: Vec<f32> = Vec::new();
        let mut heard: Vec<f32> = Vec::new();
        for b in 0..20 {
            let x = ramp(b * BLOCK);
            let ret_l: Vec<f32> = wire[..BLOCK].to_vec();
            let ret = [ret_l.clone(), ret_l];
            let out = block(d.as_mut(), &[x.clone(), x.clone()], Some(&ret), 4, &[]);
            wire.drain(..BLOCK);
            wire.extend_from_slice(&out[2]);
            played.extend_from_slice(&x);
            heard.extend_from_slice(&out[0]);
        }
        for t in lat + 64..heard.len() {
            let want = played[t - lat] * expect_gain;
            assert!(
                (heard[t] - want).abs() < 1e-5,
                "invert={invert} t={t}: {} vs {want}",
                heard[t]
            );
        }
    }
}

#[test]
fn reset_clears_the_dry_line() {
    let mut d = make(BuiltinDeviceType::ExternalAudioEffect);
    d.set_param(fx::LATENCY, latency_ms(64, SR));
    d.set_param(fx::MIX, 0.0);
    let x = vec![vec![1.0f32; BLOCK], vec![1.0f32; BLOCK]];
    block(d.as_mut(), &x, None, 2, &[]);
    d.reset();
    let silence = vec![vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]];
    let out = block(d.as_mut(), &silence, None, 2, &[]);
    assert!(out[0].iter().all(|s| *s == 0.0));
}

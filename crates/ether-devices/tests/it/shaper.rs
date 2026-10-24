//! `Shaper`: lanes follow their curves locked to the song position, depth 0 is bit-exact,
//! the balance law, the cutoff sweep, curve swaps through `set_data`, the stopped clock,
//! allocation-free processing.

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::model::{
    BuiltinDevice, BuiltinDeviceType, LfoCurve, LfoPoint, ParamId, ShaperCurves, ShaperLane,
};
use ether_core::{AudioBuffers, Device, EventBuffer, PrepareConfig, ProcessContext, TransportInfo};
use ether_devices::NoSamples;
use ether_devices::shaper::{curve_data, params as p};

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;
/// 120 bpm: one beat = 24 000 samples.
const BEAT: usize = 24_000;

fn shaper(curves: ShaperCurves, params: &[(ParamId, f64)]) -> Box<dyn Device> {
    let mut d = ether_devices::create(&BuiltinDevice::Shaper { curves }, &NoSamples);
    for &(id, v) in params {
        d.set_param(id, v);
    }
    d.prepare(&PrepareConfig {
        sample_rate: SR,
        max_block_size: BLOCK,
        max_events_per_block: 16,
    });
    d.reset();
    d
}

fn flat(y: f32) -> LfoCurve {
    LfoCurve {
        points: vec![LfoPoint::linear(0.0, y)],
    }
}

/// Runs `x` (both channels) from song position `start_beat` (playing, 120 bpm, or stopped).
fn run(d: &mut dyn Device, x: &[f32], start_beat: f64, playing: bool) -> [Vec<f32>; 2] {
    let mut out = [vec![0.0f32; x.len()], vec![0.0f32; x.len()]];
    let mut out_events = EventBuffer::with_capacity(16);
    run_into(d, x, &mut out, &mut out_events, start_beat, playing);
    out
}

/// [`run`] into `out` (allocation-free).
fn run_into(
    d: &mut dyn Device,
    x: &[f32],
    out: &mut [Vec<f32>; 2],
    out_events: &mut EventBuffer,
    start_beat: f64,
    playing: bool,
) {
    let mut i = 0;
    while i < x.len() {
        let n = BLOCK.min(x.len() - i);
        let transport = TransportInfo {
            playing,
            bpm: 120.0,
            beats_per_sample: 2.0 / f64::from(SR),
            sample_time: i as u64,
            position: start_beat + i as f64 * 2.0 / f64::from(SR),
            ..TransportInfo::STOPPED
        };
        let ins: [&[f32]; 2] = [&x[i..i + n], &x[i..i + n]];
        let (l, r) = out.split_at_mut(1);
        let mut outs: [&mut [f32]; 2] = [&mut l[0][i..i + n], &mut r[0][i..i + n]];
        let mut ctx = ProcessContext {
            sample_rate: SR,
            frames: n,
            transport: &transport,
            events: &[],
            out_events,
        };
        d.process(
            &mut ctx,
            &mut AudioBuffers {
                inputs: &ins,
                outputs: &mut outs,
            },
        );
        i += n;
    }
}

fn tone(n: usize, hz: f32, amp: f32) -> Vec<f32> {
    (0..n)
        .map(|i| amp * (std::f32::consts::TAU * hz * i as f32 / SR).sin())
        .collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

#[test]
fn volume_follows_the_curve_on_the_song_grid() {
    // A ramp from 0 to 1 over each quarter note, no smoothing.
    let ramp = LfoCurve {
        points: vec![LfoPoint::linear(0.0, 0.0), LfoPoint::linear(1.0, 1.0)],
    };
    let curves = ShaperCurves {
        volume: ramp,
        ..ShaperCurves::default()
    };
    let mut d = shaper(curves, &[(p::SMOOTHING, 0.0)]);
    let dc = vec![0.5f32; 2 * BEAT];
    // Starting at beat 1.25: a quarter of the way into a cycle.
    let out = run(d.as_mut(), &dc, 1.25, true);
    for (i, want) in [(0, 0.25), (BEAT / 4, 0.5), (BEAT / 2, 0.75), (BEAT, 0.25)] {
        let got = out[0][i] / 0.5;
        assert!((got - want).abs() < 1e-3, "sample {i}: {got} vs {want}");
        assert_eq!(out[0][i], out[1][i]);
    }
}

#[test]
fn zero_depth_and_lanes_off_are_bit_exact() {
    let x = tone(20_000, 330.0, 0.6);
    let mut d = shaper(ShaperCurves::default(), &[(p::VOLUME_DEPTH, 0.0)]);
    let out = run(d.as_mut(), &x, 0.0, true);
    assert_eq!(out[0], x);
    assert_eq!(out[1], x);
    let mut d = shaper(ShaperCurves::default(), &[(p::VOLUME_ON, 0.0)]);
    let out = run(d.as_mut(), &x, 0.0, true);
    assert_eq!(out[0], x);
}

#[test]
fn pan_turns_the_other_side_down_without_boost() {
    let x = vec![0.5f32; 4800];
    let pan = |y: f32, depth: f64| {
        let curves = ShaperCurves {
            pan: flat(y),
            ..ShaperCurves::default()
        };
        let mut d = shaper(
            curves,
            &[(p::VOLUME_ON, 0.0), (p::PAN_ON, 1.0), (p::PAN_DEPTH, depth)],
        );
        let out = run(d.as_mut(), &x, 0.0, true);
        (out[0][4000] / 0.5, out[1][4000] / 0.5)
    };
    let (l, r) = pan(1.0, 100.0);
    assert!(l.abs() < 1e-4 && (r - 1.0).abs() < 1e-6, "{l} {r}");
    let (l, r) = pan(0.0, 100.0);
    assert!((l - 1.0).abs() < 1e-6 && r.abs() < 1e-4, "{l} {r}");
    let (l, r) = pan(0.5, 100.0);
    assert!((l - 1.0).abs() < 1e-6 && (r - 1.0).abs() < 1e-6);
    // Half depth: half way, cos(π/4) on the far side.
    let (l, _) = pan(1.0, 50.0);
    assert!((l - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3, "{l}");
}

#[test]
fn the_cutoff_lane_sweeps_between_low_and_high() {
    let x = tone(24_000, 4000.0, 0.5);
    let level = |y: f32, filter: f64| {
        let curves = ShaperCurves {
            cutoff: flat(y),
            ..ShaperCurves::default()
        };
        let mut d = shaper(
            curves,
            &[
                (p::VOLUME_ON, 0.0),
                (p::CUTOFF_ON, 1.0),
                (p::RESONANCE, 0.0),
                (p::FILTER, filter),
            ],
        );
        let out = run(d.as_mut(), &x, 0.0, true);
        20.0 * (rms(&out[0][12_000..]) / rms(&x[12_000..])).log10()
    };
    // Low-pass: at 200 Hz a 4 kHz tone is far down; at 12 kHz it passes.
    let (shut, open) = (level(0.0, 0.0), level(1.0, 0.0));
    eprintln!("low-pass: shut {shut:.1} dB, open {open:.1} dB");
    assert!(shut < -40.0 && open > -1.5);
    // High-pass: the other way round.
    let (lo, hi) = (level(0.0, 1.0), level(1.0, 1.0));
    eprintln!("high-pass: at 200 Hz {lo:.1} dB, at 12 kHz {hi:.1} dB");
    assert!(lo > -1.5 && hi < -15.0);
}

#[test]
fn curve_swaps_and_the_stopped_clock() {
    let mut d = shaper(ShaperCurves::default(), &[(p::SMOOTHING, 0.0)]);
    let dc = vec![1.0f32; 4800];
    // Stopped: the lanes run at the tempo (the pump dips within a beat).
    let out = run(d.as_mut(), &vec![1.0f32; BEAT], 0.0, false);
    assert!(out[0].iter().any(|&v| v < 0.1) && out[0].iter().any(|&v| v > 0.99));
    // A flat volume curve swapped in place: unity from then on.
    let data = curve_data(&BuiltinDevice::Shaper {
        curves: ShaperCurves {
            volume: flat(1.0),
            ..ShaperCurves::default()
        },
    })
    .unwrap();
    assert!(d.set_data(data).is_some());
    let out = run(d.as_mut(), &dc, 0.0, true);
    assert!(out[0].iter().all(|&v| (v - 1.0).abs() < 1e-6));
    assert!(curve_data(&BuiltinDevice::Compressor).is_none());
}

#[test]
fn processes_without_allocating() {
    let mut d = shaper(
        ShaperCurves::default(),
        &[(p::CUTOFF_ON, 1.0), (p::PAN_ON, 1.0), (p::RESONANCE, 80.0)],
    );
    let x = tone(BLOCK * 50, 220.0, 0.8);
    let mut out = [vec![0.0f32; x.len()], vec![0.0f32; x.len()]];
    let mut events = EventBuffer::with_capacity(16);
    assert_no_alloc(|| run_into(d.as_mut(), &x, &mut out, &mut events, 3.0, true));
    assert!(out.iter().flatten().all(|v| v.is_finite()));
    // `BuiltinDevice::new` gives the default curves.
    assert_eq!(
        BuiltinDevice::new(BuiltinDeviceType::Shaper),
        BuiltinDevice::Shaper {
            curves: ShaperCurves::default()
        }
    );
    assert_eq!(ShaperLane::ALL.len(), ether_devices::shaper::LANES.len());
}

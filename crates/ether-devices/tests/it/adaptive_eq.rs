//! `AdaptiveEq`: a resonance is tamed, a dull sound is opened up, a balanced one is left
//! about alone, recover/tame at 0 is bit-exact, auto gain holds the loudness, freeze holds
//! the curve, the frame layout, allocation-free processing.

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventBuffer, Node, PrepareConfig, ProcessContext, TransportInfo,
};
use ether_devices::NoSamples;
use ether_devices::adaptive_eq::{AdaptiveEq, BANDS, Frame, POINTS, params as p};
use ether_devices::fx_analysis::loudness_device::Frame as LoudnessFrame;

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;

fn eq(params: &[(ParamId, f64)]) -> AdaptiveEq {
    let mut d = AdaptiveEq::new();
    for &(id, v) in params {
        d.set_param(id, v);
    }
    d.prepare(&PrepareConfig {
        sample_rate: SR,
        max_block_size: BLOCK,
        max_events_per_block: 16,
    });
    d
}

fn run_into(d: &mut dyn Device, x: &[f32], out: &mut [Vec<f32>; 2], events: &mut EventBuffer) {
    let transport = TransportInfo {
        playing: true,
        ..TransportInfo::STOPPED
    };
    let mut i = 0;
    while i < x.len() {
        let n = BLOCK.min(x.len() - i);
        let ins: [&[f32]; 2] = [&x[i..i + n], &x[i..i + n]];
        let (l, r) = out.split_at_mut(1);
        let mut outs: [&mut [f32]; 2] = [&mut l[0][i..i + n], &mut r[0][i..i + n]];
        let mut ctx = ProcessContext {
            sample_rate: SR,
            frames: n,
            transport: &transport,
            events: &[],
            out_events: events,
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

fn run(d: &mut dyn Device, x: &[f32]) -> Vec<f32> {
    let mut out = [vec![0.0f32; x.len()], vec![0.0f32; x.len()]];
    let mut events = EventBuffer::with_capacity(16);
    run_into(d, x, &mut out, &mut events);
    let [l, _] = out;
    l
}

/// Pink noise (Paul Kellet's filter on white noise), about -20 dBFS RMS.
fn pink(n: usize, seed: u32) -> Vec<f32> {
    let mut s = seed;
    let mut b = [0.0f32; 7];
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            let w = s as f32 / u32::MAX as f32 * 2.0 - 1.0;
            b[0] = 0.99886 * b[0] + w * 0.055_517_9;
            b[1] = 0.99332 * b[1] + w * 0.075_075_9;
            b[2] = 0.969 * b[2] + w * 0.153_852;
            b[3] = 0.8665 * b[3] + w * 0.310_485_6;
            b[4] = 0.55 * b[4] + w * 0.532_952_2;
            b[5] = -0.7616 * b[5] - w * 0.016_898;
            let y = b[0] + b[1] + b[2] + b[3] + b[4] + b[5] + b[6] + w * 0.5362;
            b[6] = w * 0.115_926;
            y * 0.03
        })
        .collect()
}

/// Amplitude of the `hz` component of `x` (a single-bin DFT).
fn tone_level(x: &[f32], hz: f32) -> f32 {
    let w = std::f32::consts::TAU * hz / SR;
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (i, v) in x.iter().enumerate() {
        re += f64::from(v * (w * i as f32).cos());
        im += f64::from(v * (w * i as f32).sin());
    }
    (2.0 * (re * re + im * im).sqrt() / x.len() as f64) as f32
}

fn gain_at(d: &AdaptiveEq, hz: f32) -> f32 {
    let f = d.frequencies();
    let k = (0..BANDS)
        .min_by(|&a, &b| {
            (f[a] / hz)
                .log2()
                .abs()
                .total_cmp(&(f[b] / hz).log2().abs())
        })
        .unwrap();
    d.gains()[k]
}

#[test]
fn tames_a_resonance() {
    let n = 5 * SR as usize;
    let mut x = pink(n, 3);
    for (i, v) in x.iter_mut().enumerate() {
        *v += 0.1 * (std::f32::consts::TAU * 2000.0 * i as f32 / SR).sin();
    }
    let mut d = eq(&[(p::RECOVER, 0.0), (p::TAME, 100.0)]);
    let y = run(&mut d, &x);
    let at = gain_at(&d, 2000.0);
    let far = gain_at(&d, 300.0);
    let tail = 3 * SR as usize..n;
    let cut = 20.0 * (tone_level(&y[tail.clone()], 2000.0) / tone_level(&x[tail], 2000.0)).log10();
    eprintln!("gain at 2 kHz {at:.1} dB, at 300 Hz {far:.1} dB; the tone {cut:.1} dB");
    assert!(at < -3.0 && far.abs() < 1.5, "{at} {far}");
    assert!(cut < -2.0, "{cut}");
}

#[test]
fn opens_up_a_dull_sound_and_leaves_a_balanced_one() {
    let n = 5 * SR as usize;
    // Pink noise through two one-pole low-passes at 1.5 kHz.
    let a = (-std::f32::consts::TAU * 1500.0 / SR).exp();
    let (mut s1, mut s2) = (0.0f32, 0.0f32);
    let dull: Vec<f32> = pink(n, 5)
        .into_iter()
        .map(|v| {
            s1 = v + (s1 - v) * a;
            s2 = s1 + (s2 - s1) * a;
            s2 * 2.0
        })
        .collect();
    let mut d = eq(&[(p::RECOVER, 100.0), (p::TAME, 0.0)]);
    run(&mut d, &dull);
    let hi = gain_at(&d, 6000.0);
    eprintln!("dull: gain at 6 kHz {hi:.1} dB");
    assert!(hi > 1.0, "{hi}");
    // Tame only: never a boost.
    let mut d = eq(&[(p::RECOVER, 0.0), (p::TAME, 100.0)]);
    run(&mut d, &dull);
    assert!(d.gains().iter().all(|&g| g <= 1e-3));
    // Pink noise with the reference tilt is already balanced: small moves only.
    let tilted = {
        // -1.5 dB/oct more than pink: a gentle one-pole at 1 kHz is close enough.
        let a = (-std::f32::consts::TAU * 1000.0 / SR).exp();
        let mut s = 0.0f32;
        pink(n, 9)
            .into_iter()
            .map(|v| {
                s = v + (s - v) * a;
                0.5 * (v + s)
            })
            .collect::<Vec<_>>()
    };
    let mut d = eq(&[]);
    run(&mut d, &tilted);
    let worst = d.gains().iter().fold(0.0f32, |m, g| m.max(g.abs()));
    eprintln!("near-reference input: largest move {worst:.1} dB");
    assert!(worst < 3.0, "{worst}");
}

#[test]
fn recover_and_tame_off_is_bit_exact() {
    let x = pink(48_000, 1);
    let mut d = eq(&[(p::RECOVER, 0.0), (p::TAME, 0.0)]);
    let y = run(&mut d, &x);
    assert_eq!(y, x);
    assert_eq!(d.latency(), 0);
}

fn lufs(x: &[f32]) -> f32 {
    let mut m = ether_devices::create(&BuiltinDevice::new(BuiltinDeviceType::Loudness), &NoSamples);
    m.prepare(&PrepareConfig {
        sample_rate: SR,
        max_block_size: BLOCK,
        max_events_per_block: 16,
    });
    run(m.as_mut(), x);
    use ether_core::analysis::{ANALYSIS_FRAMES_PER_PASS, AnalysisFrame, AnalysisSink};
    let mut frames = Box::new([AnalysisFrame::EMPTY; ANALYSIS_FRAMES_PER_PASS]);
    let mut sink = AnalysisSink::new(&mut frames[..]);
    m.analysis(&mut sink);
    frames[0].values()[LoudnessFrame::INTEGRATED]
}

#[test]
fn auto_gain_holds_the_loudness_and_freeze_holds_the_curve() {
    let n = 6 * SR as usize;
    let mut x = pink(n, 11);
    for (i, v) in x.iter_mut().enumerate() {
        *v += 0.08 * (std::f32::consts::TAU * 150.0 * i as f32 / SR).sin();
    }
    let mut d = eq(&[(p::RECOVER, 100.0), (p::TAME, 100.0)]);
    let y = run(&mut d, &x);
    let (a, b) = (lufs(&x[2 * SR as usize..]), lufs(&y[2 * SR as usize..]));
    eprintln!("loudness in {a:.2} LUFS, out {b:.2} LUFS");
    assert!((a - b).abs() < 1.0, "{a} {b}");
    // Freeze, then a very different input: the curve stays.
    let before = d.gains();
    d.set_param(p::FREEZE, 1.0);
    run(&mut d, &pink(2 * SR as usize, 13));
    assert_eq!(d.gains(), before);
}

#[test]
fn frame_and_allocation_free_processing() {
    let mut d = eq(&[]);
    let x = pink(4 * BLOCK * 64, 2);
    let mut out = [vec![0.0f32; x.len()], vec![0.0f32; x.len()]];
    let mut events = EventBuffer::with_capacity(16);
    assert_no_alloc(|| run_into(&mut d, &x, &mut out, &mut events));
    assert!(out.iter().flatten().all(|v| v.is_finite()));
    use ether_core::analysis::{ANALYSIS_FRAMES_PER_PASS, AnalysisFrame, AnalysisSink};
    let mut frames = Box::new([AnalysisFrame::EMPTY; ANALYSIS_FRAMES_PER_PASS]);
    let mut sink = AnalysisSink::new(&mut frames[..]);
    d.analysis(&mut sink);
    let f = frames[0].values();
    assert_eq!(f[Frame::POINTS] as usize, POINTS);
    assert_eq!(f.len(), Frame::DATA + 2 * POINTS + 3 * BANDS);
    // The response points run from 20 Hz to 20 kHz.
    assert!((f[Frame::DATA] - 20.0).abs() < 1e-3);
    assert!((f[Frame::DATA + POINTS - 1] - 20_000.0).abs() < 1.0);
}

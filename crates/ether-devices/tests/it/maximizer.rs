//! `Maximizer`: the brickwall (sample and true peak, checked with the loudness meter's
//! independent true-peak detector), latency and transparency under the threshold, the
//! threshold boost and level match, auto release, stereo link, the clipper, the analysis
//! frame and allocation-free processing.

use assert_no_alloc::assert_no_alloc;
use ether_core::Node;
use ether_core::analysis::{ANALYSIS_FRAMES_PER_PASS, AnalysisFrame, AnalysisKind, AnalysisSink};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType};
use ether_core::{
    AudioBuffers, Device, EventBuffer, EventKind, PrepareConfig, ProcessContext, ProcessEvent,
    TransportInfo,
};
use ether_devices::NoSamples;
use ether_devices::fx_analysis::loudness_device::Frame as LoudnessFrame;
use ether_devices::maximizer::{Frame, Maximizer, params as p};

const BLOCK: usize = 256;
const SR: f32 = 48_000.0;

fn prepared(d: &mut dyn Device, sr: f32) {
    d.prepare(&PrepareConfig {
        sample_rate: sr,
        max_block_size: BLOCK,
        max_events_per_block: 64,
    });
}

fn maximizer(params: &[(ether_core::protocol::model::ParamId, f64)]) -> Maximizer {
    let mut m = Maximizer::new();
    prepared(&mut m, SR);
    for &(id, v) in params {
        m.set_param(id, v);
    }
    m
}

/// Runs `left`/`right` through `d` in blocks; returns the output.
fn run(d: &mut dyn Device, left: &[f32], right: &[f32]) -> [Vec<f32>; 2] {
    let mut out = [vec![0.0f32; left.len()], vec![0.0f32; left.len()]];
    let transport = TransportInfo {
        playing: true,
        ..TransportInfo::STOPPED
    };
    let mut out_events = EventBuffer::with_capacity(64);
    let mut i = 0;
    while i < left.len() {
        let n = BLOCK.min(left.len() - i);
        let ins: [&[f32]; 2] = [&left[i..i + n], &right[i..i + n]];
        let (l, r) = out.split_at_mut(1);
        let mut outs: [&mut [f32]; 2] = [&mut l[0][i..i + n], &mut r[0][i..i + n]];
        let mut ctx = ProcessContext {
            sample_rate: SR,
            frames: n,
            transport: &transport,
            events: &[],
            out_events: &mut out_events,
        };
        let mut buffers = AudioBuffers {
            inputs: &ins,
            outputs: &mut outs,
        };
        d.process(&mut ctx, &mut buffers);
        i += n;
    }
    out
}

fn frame(d: &mut dyn Device) -> Vec<f32> {
    let mut frames = Box::new([AnalysisFrame::EMPTY; ANALYSIS_FRAMES_PER_PASS]);
    let mut sink = AnalysisSink::new(&mut frames[..]);
    d.analysis(&mut sink);
    assert_eq!(sink.len(), 1);
    assert_eq!(frames[0].kind, AnalysisKind::Levels);
    frames[0].values().to_vec()
}

/// A drum-like test signal: noise bursts with fast decays every 125 ms over a bass
/// tone, peaking around `peak_db` dBFS, plus a few bright sine bursts near fs/4.
fn program(seconds: f32, peak_db: f32, seed: u32) -> [Vec<f32>; 2] {
    let n = (seconds * SR) as usize;
    let amp = 10f32.powf(peak_db / 20.0);
    let mut s = seed;
    let mut rnd = || {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        (s as f32 / u32::MAX as f32) * 2.0 - 1.0
    };
    let mut l = vec![0.0f32; n];
    let mut r = vec![0.0f32; n];
    for i in 0..n {
        let t = i as f32 / SR;
        let hit = (t * 8.0).fract();
        let env = (-hit * 30.0).exp();
        let bass = 0.35 * (std::f32::consts::TAU * 55.0 * t).sin();
        let bright = if (t * 2.0).fract() < 0.05 {
            0.4 * (std::f32::consts::TAU * 11_900.0 * t + 0.785).sin()
        } else {
            0.0
        };
        l[i] = amp * (0.6 * env * rnd() + bass + bright);
        r[i] = amp * (0.6 * env * rnd() + bass * 0.9 - bright);
    }
    [l, r]
}

fn peak_db(x: &[f32]) -> f32 {
    20.0 * x.iter().fold(0.0f32, |m, v| m.max(v.abs())).log10()
}

/// The output's true peak, from the loudness meter (an independent detector).
fn true_peak_db(out: &[Vec<f32>; 2]) -> f32 {
    let mut meter =
        ether_devices::create(&BuiltinDevice::new(BuiltinDeviceType::Loudness), &NoSamples);
    prepared(meter.as_mut(), SR);
    run(meter.as_mut(), &out[0], &out[1]);
    frame(meter.as_mut())[LoudnessFrame::TRUE_PEAK_MAX]
}

fn rms_db(x: &[f32]) -> f32 {
    let e = x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum::<f64>() / x.len() as f64;
    (10.0 * e.log10()) as f32
}

#[test]
fn holds_the_ceiling_in_sample_and_true_peak() {
    for (threshold, ceiling, attack, clip) in [
        (-18.0, -1.0, 2.0, 0.0),
        (-24.0, -0.3, 0.1, 0.0),
        (-12.0, -6.0, 5.0, 0.0),
        (-18.0, -1.0, 1.0, 80.0),
    ] {
        let mut m = maximizer(&[
            (p::THRESHOLD, threshold),
            (p::CEILING, ceiling),
            (p::ATTACK, attack),
            (p::CLIP, clip),
        ]);
        let [l, r] = program(4.0, -3.0, 7);
        let out = run(&mut m, &l, &r);
        let sp = peak_db(&out[0]).max(peak_db(&out[1]));
        let tp = true_peak_db(&out);
        eprintln!(
            "threshold {threshold}, ceiling {ceiling}, attack {attack}, clip {clip}: \
             sample peak {sp:.3}, true peak {tp:.3}, gr max {:.1}",
            -frame(&mut m)[Frame::GAIN_REDUCTION_MAX]
        );
        assert!(sp <= ceiling as f32 + 1e-4, "sample peak {sp}");
        assert!(tp <= ceiling as f32 + 0.2, "true peak {tp}");
        // The device's own output meter agrees.
        assert!((m.output_true_peak_max() - tp).abs() < 0.3);
    }
}

#[test]
fn under_the_threshold_it_is_a_pure_delay() {
    let mut m = maximizer(&[]);
    let latency = m.latency() as usize;
    // 5 ms of lookahead + the detector's 8 samples.
    assert_eq!(latency, 240 + 8);
    let [l, r] = program(0.5, -12.0, 3);
    let out = run(&mut m, &l, &r);
    assert_eq!(&out[0][latency..], &l[..l.len() - latency]);
    assert_eq!(&out[1][latency..], &r[..r.len() - latency]);
    assert!(out[0][..latency].iter().all(|&v| v == 0.0));
    assert!(m.gain_reduction() == 0.0);
}

#[test]
fn the_threshold_boosts_and_match_level_compensates() {
    let [l, r] = program(1.0, -30.0, 5);
    let mut m = maximizer(&[(p::THRESHOLD, -10.0)]);
    let out = run(&mut m, &l, &r);
    let lat = m.latency() as usize;
    // -30 dBFS peaks + 10 dB: no limiting, exactly +10 dB.
    let gain = rms_db(&out[0][lat..]) - rms_db(&l[..l.len() - lat]);
    assert!((gain - 10.0).abs() < 0.01, "{gain}");
    // Driven into the ceiling with Match Level: back near the input's loudness (a bit
    // under: the limiting took the peaks).
    let [l, r] = program(2.0, -6.0, 5);
    let mut m = maximizer(&[(p::THRESHOLD, -12.0), (p::MATCH_LEVEL, 1.0)]);
    let out = run(&mut m, &l, &r);
    let diff = rms_db(&out[0][lat..]) - rms_db(&l[..l.len() - lat]);
    assert!((-10.0..=0.1).contains(&diff), "{diff}");
    assert!(
        peak_db(&out[0]) < -12.0,
        "the limited peaks sit 12 dB under the ceiling"
    );
}

/// Gain reduction (dB) at the end of `quiet_ms` of quiet tone after `loud_ms` of loud one.
fn recovery(auto: bool, loud_ms: f32, quiet_ms: f32) -> f32 {
    let mut m = maximizer(&[
        (p::THRESHOLD, -12.0),
        (p::RELEASE, 50.0),
        (p::AUTO_RELEASE, if auto { 1.0 } else { 0.0 }),
    ]);
    let tone = |ms: f32, amp: f32, from: usize| -> Vec<f32> {
        (0..(ms * 48.0) as usize)
            .map(|i| amp * (std::f32::consts::TAU * 200.0 * (from + i) as f32 / SR).sin())
            .collect()
    };
    let loud = tone(loud_ms, 0.9, 0);
    run(&mut m, &loud, &loud);
    let quiet = tone(quiet_ms, 0.01, loud.len());
    run(&mut m, &quiet, &quiet);
    m.gain_reduction()
}

#[test]
fn auto_release_recovers_slowly_after_sustained_limiting_only() {
    // After 2 s of heavy limiting, 100 ms later: auto release still holds more.
    let fixed = recovery(false, 2000.0, 100.0);
    let auto = recovery(true, 2000.0, 100.0);
    eprintln!("sustained: fixed {fixed:.2} dB, auto {auto:.2} dB");
    assert!(auto > fixed + 1.0, "auto {auto}, fixed {fixed}");
    // After a 10 ms burst both have about recovered by then.
    let fixed = recovery(false, 10.0, 300.0);
    let auto = recovery(true, 10.0, 300.0);
    eprintln!("burst: fixed {fixed:.2} dB, auto {auto:.2} dB");
    assert!(auto < 1.0 && fixed < 0.5, "auto {auto}, fixed {fixed}");
}

#[test]
fn stereo_link_decides_whether_one_side_ducks_the_other() {
    let n = 24_000;
    let loud: Vec<f32> = (0..n)
        .map(|i| 0.9 * (std::f32::consts::TAU * 100.0 * i as f32 / SR).sin())
        .collect();
    let quiet: Vec<f32> = loud.iter().map(|v| v * 0.05).collect();
    let tail = |out: &[Vec<f32>; 2]| peak_db(&out[1][n / 2..]);
    let mut linked = maximizer(&[(p::THRESHOLD, -12.0)]);
    let a = run(&mut linked, &loud, &quiet);
    let mut free = maximizer(&[(p::THRESHOLD, -12.0), (p::STEREO_LINK, 0.0)]);
    let b = run(&mut free, &loud, &quiet);
    let input = peak_db(&quiet) + 12.0;
    eprintln!(
        "right side: input+12 {input:.2}, linked {:.2}, unlinked {:.2}",
        tail(&a),
        tail(&b)
    );
    // Unlinked, the quiet side only gets the boost; linked, it ducks with the loud one.
    assert!((tail(&b) - input).abs() < 0.05);
    assert!(tail(&a) < input - 6.0);
}

#[test]
fn the_clipper_takes_peaks_off_the_limiter() {
    let gr = |clip: f64| {
        let mut m = maximizer(&[(p::THRESHOLD, -12.0), (p::CLIP, clip)]);
        let [l, r] = program(2.0, -3.0, 11);
        run(&mut m, &l, &r);
        -frame(&mut m)[Frame::GAIN_REDUCTION_MAX]
    };
    let (off, on) = (gr(0.0), gr(100.0));
    eprintln!("deepest reduction: clip off {off:.2} dB, clip 100 % {on:.2} dB");
    assert!(on < off - 2.0, "{on} vs {off}");
}

#[test]
fn frame_carries_meters_and_history() {
    let mut m = maximizer(&[(p::THRESHOLD, -12.0)]);
    let [l, r] = program(1.0, -6.0, 9);
    run(&mut m, &l, &r);
    let f = frame(&mut m);
    let n = f[Frame::POINTS] as usize;
    // 1 s of 20 ms points.
    assert_eq!(n, 50);
    assert_eq!(f.len(), Frame::HISTORY + 3 * n);
    assert!((f[Frame::STEP] - 0.02).abs() < 1e-6);
    assert!(
        f[Frame::GAIN_REDUCTION] < -1.0 && f[Frame::GAIN_REDUCTION] >= f[Frame::GAIN_REDUCTION_MAX]
    );
    assert!(f[Frame::OUTPUT_MAX] <= -0.7);
    // The input meter reads before the threshold boost: about the program's peak.
    let input = &f[Frame::HISTORY..Frame::HISTORY + n];
    let peak = input.iter().copied().fold(f32::MIN, f32::max);
    assert!((-8.0..=-2.0).contains(&peak), "{peak}");
    let gr = &f[Frame::HISTORY + 2 * n..];
    assert!(gr.iter().all(|&v| v <= 0.0) && gr.iter().any(|&v| v < -1.0));
    // The per-frame meters start over; the history doesn't.
    let g = frame(&mut m);
    assert_eq!(g[Frame::GAIN_REDUCTION], 0.0);
    assert_eq!(g[Frame::POINTS] as usize, 50);
}

#[test]
fn processes_without_allocating_and_survives_bad_input() {
    let mut d = ether_devices::create(
        &BuiltinDevice::new(BuiltinDeviceType::Maximizer),
        &NoSamples,
    );
    prepared(d.as_mut(), SR);
    let mut x = vec![0.5f32; BLOCK];
    x[10] = f32::NAN;
    x[11] = f32::INFINITY;
    x[12] = 1e30;
    let events = [ProcessEvent {
        offset: 3,
        kind: EventKind::Param {
            param: p::ATTACK,
            value: 0.3,
            step: false,
        },
    }];
    let mut out = [vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]];
    let transport = TransportInfo {
        playing: true,
        ..TransportInfo::STOPPED
    };
    let mut out_events = EventBuffer::with_capacity(64);
    let mut frames = Box::new([AnalysisFrame::EMPTY; ANALYSIS_FRAMES_PER_PASS]);
    assert_no_alloc(|| {
        for k in 0..40 {
            let ins: [&[f32]; 2] = [&x, &x];
            let (l, r) = out.split_at_mut(1);
            let mut outs: [&mut [f32]; 2] = [&mut l[0], &mut r[0]];
            let mut ctx = ProcessContext {
                sample_rate: SR,
                frames: BLOCK,
                transport: &transport,
                events: if k == 0 { &events } else { &[] },
                out_events: &mut out_events,
            };
            let mut buffers = AudioBuffers {
                inputs: &ins,
                outputs: &mut outs,
            };
            d.process(&mut ctx, &mut buffers);
            let mut sink = AnalysisSink::new(&mut frames[..]);
            d.analysis(&mut sink);
        }
    });
    assert!(
        out.iter()
            .flatten()
            .all(|v| v.is_finite() && v.abs() <= 0.9)
    );
}

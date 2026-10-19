//! `fx-analysis`: SpectrumAnalyzer and Tuner behaviour (known inputs → known frames),
//! pass-through, sample-accurate mute, NaN safety and allocation-free processing/analysis.

use assert_no_alloc::assert_no_alloc;
use ether_core::analysis::{ANALYSIS_FRAMES_PER_PASS, AnalysisFrame, AnalysisKind, AnalysisSink};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventBuffer, EventKind, PrepareConfig, ProcessContext, ProcessEvent,
    TransportInfo,
};
use ether_devices::NoSamples;
use ether_devices::fx_analysis::{spectrum_analyzer as sp, tuner as tp};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;

fn device(ty: BuiltinDeviceType) -> Box<dyn Device> {
    let mut d = ether_devices::create(&BuiltinDevice::new(ty), &NoSamples);
    d.prepare(&PrepareConfig {
        sample_rate: SR,
        max_block_size: BLOCK,
        max_events_per_block: 64,
    });
    d
}

/// Runs `d` over `left`/`right` (same length, multiple of BLOCK), calling `analysis` after
/// every block like a watched engine does; returns the output and the last frame of `kind`.
struct Run {
    out: [Vec<f32>; 2],
    frame: Option<Vec<f32>>,
    frames: usize,
}

fn run(
    d: &mut dyn Device,
    left: &[f32],
    right: &[f32],
    events: &[(usize, ParamId, f64)],
    watch: bool,
    kind: AnalysisKind,
) -> Run {
    let n = left.len();
    let mut out = [vec![0.0f32; n], vec![0.0f32; n]];
    let mut last = None;
    let mut count = 0;
    let mut frames = Box::new([AnalysisFrame::EMPTY; ANALYSIS_FRAMES_PER_PASS]);
    let mut out_events = EventBuffer::with_capacity(64);
    let mut block_events: Vec<ProcessEvent> = Vec::with_capacity(64);
    let transport = TransportInfo {
        playing: true,
        ..TransportInfo::STOPPED
    };
    for start in (0..n).step_by(BLOCK) {
        block_events.clear();
        for &(at, param, value) in events {
            if at >= start && at < start + BLOCK {
                block_events.push(ProcessEvent {
                    offset: (at - start) as u32,
                    kind: EventKind::Param { param, value },
                });
            }
        }
        let ins: [&[f32]; 2] = [&left[start..start + BLOCK], &right[start..start + BLOCK]];
        let (l, r) = out.split_at_mut(1);
        let mut outs: [&mut [f32]; 2] = [
            &mut l[0][start..start + BLOCK],
            &mut r[0][start..start + BLOCK],
        ];
        let mut ctx = ProcessContext {
            sample_rate: SR,
            frames: BLOCK,
            transport: &transport,
            events: &block_events,
            out_events: &mut out_events,
        };
        let mut buffers = AudioBuffers {
            inputs: &ins,
            outputs: &mut outs,
        };
        let mut written = 0;
        assert_no_alloc(|| {
            d.process(&mut ctx, &mut buffers);
            if watch {
                let mut sink = AnalysisSink::new(&mut frames[..]);
                d.analysis(&mut sink);
                written = sink.len();
            }
        });
        for f in &frames[..written] {
            assert_eq!(f.kind, kind);
            last = Some(f.values().to_vec());
            count += 1;
        }
    }
    Run {
        out,
        frame: last,
        frames: count,
    }
}

fn sine(hz: f32, amp: f32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| amp * (std::f32::consts::TAU * hz * i as f32 / SR).sin())
        .collect()
}

fn noise(n: usize, amp: f32) -> Vec<f32> {
    let mut s = 0x1234_5678u32;
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            amp * ((s as f32 / u32::MAX as f32) * 2.0 - 1.0)
        })
        .collect()
}

/// A second of audio.
const N: usize = 48_128;

// ─── Spectrum ─────────────────────────────────────────────────────────────────────────────

fn peak(frame: &[f32]) -> (f32, f32) {
    let (min_hz, max_hz) = (frame[0], frame[1]);
    let bins = &frame[2..];
    let (i, db) = bins.iter().enumerate().fold(
        (0, f32::MIN),
        |b, (i, v)| if *v > b.1 { (i, *v) } else { b },
    );
    let hz = min_hz * (max_hz / min_hz).powf(i as f32 / (bins.len() - 1) as f32);
    (hz, db)
}

#[test]
fn spectrum_passes_audio_through_and_publishes_frames() {
    let mut d = device(BuiltinDeviceType::SpectrumAnalyzer);
    let l = noise(N, 0.5);
    let r = sine(300.0, 0.3, N);
    let res = run(d.as_mut(), &l, &r, &[], true, AnalysisKind::Spectrum);
    assert_eq!(res.out[0], l);
    assert_eq!(res.out[1], r);
    // ~30 frames per second.
    assert!((25..=31).contains(&res.frames), "{}", res.frames);
    let f = res.frame.unwrap();
    assert_eq!(f.len(), 2 + 256);
    assert_eq!(f[0], 20.0);
    assert_eq!(f[1], 20_000.0);
    assert!(f.iter().all(|v| v.is_finite()));
}

#[test]
fn full_scale_sine_reads_zero_db_at_its_frequency() {
    for block in 0..4 {
        let mut d = device(BuiltinDeviceType::SpectrumAnalyzer);
        d.set_param(sp::BLOCK_SIZE, block as f64);
        d.set_param(sp::SLOPE, 0.0);
        d.set_param(sp::AVERAGING, 0.0);
        let s = sine(1000.0, 1.0, N);
        let f = run(d.as_mut(), &s, &s, &[], true, AnalysisKind::Spectrum)
            .frame
            .unwrap();
        let (hz, db) = peak(&f);
        assert!(
            (hz / 1000.0 - 1.0).abs() < 0.03,
            "block {block}: peak at {hz}"
        );
        assert!(db.abs() < 1.6, "block {block}: {db} dB");
        // Far from the tone: well below.
        let (lo, hi) = (f[2 + 10], f[2 + 250]);
        assert!(lo < -60.0 && hi < -60.0, "block {block}: {lo} {hi}");
    }
}

#[test]
fn slope_tilts_around_one_khz() {
    let s = sine(4000.0, 0.5, N);
    let at = |slope: f64| {
        let mut d = device(BuiltinDeviceType::SpectrumAnalyzer);
        d.set_param(sp::SLOPE, slope);
        peak(
            &run(d.as_mut(), &s, &s, &[], true, AnalysisKind::Spectrum)
                .frame
                .unwrap(),
        )
        .1
    };
    // 4 kHz is two octaves above the pivot: +3 dB/oct → +6 dB.
    let diff = at(3.0) - at(0.0);
    assert!((diff - 6.0).abs() < 0.3, "{diff}");
}

#[test]
fn channel_selects_what_is_analysed() {
    let l = sine(500.0, 0.5, N);
    let silent = vec![0.0; N];
    let level = |channel: f64, left: &[f32], right: &[f32]| {
        let mut d = device(BuiltinDeviceType::SpectrumAnalyzer);
        d.set_param(sp::CHANNEL, channel);
        peak(
            &run(d.as_mut(), left, right, &[], true, AnalysisKind::Spectrum)
                .frame
                .unwrap(),
        )
        .1
    };
    assert!(level(1.0, &l, &silent) > -12.0, "left");
    assert!(level(2.0, &l, &silent) < -120.0, "right is silent");
    // Mid of identical channels is the signal, side is silent.
    assert!(level(3.0, &l, &l) > -12.0, "mid");
    assert!(level(4.0, &l, &l) < -120.0, "side");
    // Stereo averages the powers: one channel alone reads 3 dB lower.
    let both = level(0.0, &l, &l);
    let one = level(0.0, &l, &silent);
    assert!((both - one - 3.01).abs() < 0.2, "{both} {one}");
}

#[test]
fn averaging_smooths_changes() {
    let mut s = sine(1000.0, 1.0, N);
    s.extend(vec![0.0; 4 * 1024]);
    let tail = |avg: f64| {
        let mut d = device(BuiltinDeviceType::SpectrumAnalyzer);
        d.set_param(sp::AVERAGING, avg);
        d.set_param(sp::BLOCK_SIZE, 0.0);
        peak(
            &run(d.as_mut(), &s, &s, &[], true, AnalysisKind::Spectrum)
                .frame
                .unwrap(),
        )
        .1
    };
    // After the tone stops, the averaged display decays slower than the raw one.
    assert!(tail(100.0) > tail(0.0) + 20.0);
}

#[test]
fn spectrum_is_idle_while_unwatched_and_nan_safe() {
    let mut d = device(BuiltinDeviceType::SpectrumAnalyzer);
    let s = sine(1000.0, 0.5, N);
    let res = run(d.as_mut(), &s, &s, &[], false, AnalysisKind::Spectrum);
    assert_eq!(res.frames, 0);
    let mut bad = s.clone();
    bad[1000] = f32::NAN;
    bad[2000] = f32::INFINITY;
    let res = run(d.as_mut(), &bad, &s, &[], true, AnalysisKind::Spectrum);
    assert!(res.frame.unwrap().iter().all(|v| v.is_finite()));
}

// ─── Tuner ────────────────────────────────────────────────────────────────────────────────

struct Reading {
    hz: f32,
    note: f32,
    cents: f32,
    confidence: f32,
    level_db: f32,
}

fn tune(d: &mut dyn Device, left: &[f32], right: &[f32]) -> Reading {
    let f = run(d, left, right, &[], true, AnalysisKind::Tuner)
        .frame
        .unwrap();
    assert_eq!(f.len(), 5);
    Reading {
        hz: f[0],
        note: f[1],
        cents: f[2],
        confidence: f[3],
        level_db: f[4],
    }
}

/// A plucked-string-like tone: fundamental plus decaying harmonics.
fn tone(hz: f32, n: usize) -> Vec<f32> {
    let mut v = vec![0.0; n];
    for h in 1..=6 {
        let a = 0.4 / h as f32;
        for (i, s) in sine(hz * h as f32, a, n).into_iter().enumerate() {
            v[i] += s;
        }
    }
    v
}

#[test]
fn tuner_finds_notes_across_the_range() {
    // (Hz, MIDI note): low B (5-string bass), low E, A2, A4, C6.
    for (hz, note) in [
        (30.87, 23.0),
        (41.2, 28.0),
        (110.0, 45.0),
        (440.0, 69.0),
        (1046.5, 84.0),
    ] {
        let mut d = device(BuiltinDeviceType::Tuner);
        let s = tone(hz, N);
        let r = tune(d.as_mut(), &s, &s);
        assert_eq!(r.note, note, "{hz} Hz → {}", r.hz);
        assert!((r.hz / hz - 1.0).abs() < 0.003, "{hz} Hz read {}", r.hz);
        assert!(r.cents.abs() < 5.0, "{hz} Hz: {} ct", r.cents);
        assert!(r.confidence > 0.9, "{hz} Hz: {}", r.confidence);
    }
}

#[test]
fn tuner_cents_follow_the_reference() {
    let s = sine(445.0, 0.5, N);
    let mut d = device(BuiltinDeviceType::Tuner);
    let r = tune(d.as_mut(), &s, &s);
    // 1200·log2(445/440) = +19.56 ct.
    assert_eq!(r.note, 69.0);
    assert!((r.cents - 19.56).abs() < 1.0, "{}", r.cents);
    let s = sine(440.0, 0.5, N);
    let mut d = device(BuiltinDeviceType::Tuner);
    d.set_param(tp::REFERENCE, 442.0);
    let r = tune(d.as_mut(), &s, &s);
    // 1200·log2(440/442) = −7.85 ct.
    assert!((r.cents + 7.85).abs() < 1.0, "{}", r.cents);
}

#[test]
fn tuner_reports_no_pitch_for_silence_and_noise() {
    let mut d = device(BuiltinDeviceType::Tuner);
    let z = vec![0.0; N];
    let r = tune(d.as_mut(), &z, &z);
    assert_eq!((r.hz, r.note), (0.0, -1.0));
    assert!(r.level_db <= -150.0);
    let mut d = device(BuiltinDeviceType::Tuner);
    let n = noise(N, 0.5);
    let r = tune(d.as_mut(), &n, &n);
    assert_eq!((r.hz, r.note), (0.0, -1.0), "noise read as {} Hz", r.hz);
    assert!(r.level_db > -12.0);
}

#[test]
fn tuner_input_selects_the_channel() {
    let a = sine(220.0, 0.5, N);
    let e = sine(329.63, 0.5, N);
    let mut d = device(BuiltinDeviceType::Tuner);
    d.set_param(tp::INPUT, 1.0);
    assert_eq!(tune(d.as_mut(), &a, &e).note, 57.0);
    let mut d = device(BuiltinDeviceType::Tuner);
    d.set_param(tp::INPUT, 2.0);
    assert_eq!(tune(d.as_mut(), &a, &e).note, 64.0);
}

#[test]
fn mute_output_is_sample_accurate_and_ramped() {
    let mut d = device(BuiltinDeviceType::Tuner);
    let s = vec![0.5f32; 4 * BLOCK];
    let at = BLOCK + 100;
    let res = run(
        d.as_mut(),
        &s,
        &s,
        &[(at, tp::MUTE, 1.0)],
        false,
        AnalysisKind::Tuner,
    );
    let out = &res.out[0];
    assert!(
        out[..at].iter().all(|v| *v == 0.5),
        "untouched before the event"
    );
    assert!(out[at] < 0.5 && out[at] > 0.45, "ramp starts at the event");
    // 10 ms ramp = 480 samples.
    assert_eq!(out[at + 480], 0.0);
    assert!(out[at + 480..].iter().all(|v| *v == 0.0));
    // Analysis still sees the input while muted.
    let t = tone(110.0, N);
    let r = tune(d.as_mut(), &t, &t);
    assert_eq!(r.note, 45.0);
}

#[test]
fn tuner_is_nan_safe() {
    let mut d = device(BuiltinDeviceType::Tuner);
    let mut s = tone(220.0, N);
    s[500] = f32::NAN;
    let res = run(d.as_mut(), &s, &s, &[], true, AnalysisKind::Tuner);
    assert!(res.frame.unwrap().iter().all(|v| v.is_finite()));
}

#[test]
fn every_block_size_and_sample_rate_runs_without_allocating() {
    for sr in [44_100.0f32, 96_000.0, 192_000.0] {
        for ty in [
            BuiltinDeviceType::SpectrumAnalyzer,
            BuiltinDeviceType::Tuner,
        ] {
            let mut d = ether_devices::create(&BuiltinDevice::new(ty), &NoSamples);
            d.prepare(&PrepareConfig {
                sample_rate: sr,
                max_block_size: BLOCK,
                max_events_per_block: 64,
            });
            let s = sine(440.0, 0.5, 8 * BLOCK);
            // Parameter sweeps through every choice, applied at offsets.
            let events: Vec<(usize, ParamId, f64)> = (0..4)
                .map(|i| {
                    (
                        i * BLOCK + 7,
                        ParamId(0),
                        if ty == BuiltinDeviceType::Tuner {
                            415.0 + 10.0 * i as f64
                        } else {
                            i as f64
                        },
                    )
                })
                .collect();
            let kind = if ty == BuiltinDeviceType::Tuner {
                AnalysisKind::Tuner
            } else {
                AnalysisKind::Spectrum
            };
            run(d.as_mut(), &s, &s, &events, true, kind);
            d.reset();
        }
    }
}

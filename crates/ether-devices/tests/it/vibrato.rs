//! Vibrato render tests: the measured pitch deviation is the depth (in cents) at every rate
//! and sample rate, the pitch follows the shape, depth 0 is a pure delay of the reported
//! latency, changes don't click, renders are deterministic. Every `process` call runs under
//! `assert_no_alloc`.

use std::f64::consts::TAU;

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventBuffer, EventKind, PrepareConfig, ProcessContext, ProcessEvent,
    TransportInfo,
};
use ether_devices::NoSamples;
use ether_devices::fx_modulation::{VIBRATO_MAX_EXCURSION_MS, vibrato as p, vibrato_contour};

const BLOCK: usize = 256;

fn device(sr: f32, params: &[(ParamId, f64)]) -> Box<dyn Device> {
    let mut d = ether_devices::create(&BuiltinDevice::new(BuiltinDeviceType::Vibrato), &NoSamples);
    for &(id, v) in params {
        d.set_param(id, v);
    }
    d.prepare(&PrepareConfig {
        sample_rate: sr,
        max_block_size: BLOCK,
        max_events_per_block: 64,
    });
    d
}

/// Render `[left, right]` in blocks of `block`; `events` are `(frame, kind)`, sorted; the
/// transport starts playing at frame `play_at` (never with `usize::MAX`).
fn render_with(
    d: &mut dyn Device,
    sr: f32,
    input: [&[f32]; 2],
    events: &[(usize, EventKind)],
    block: usize,
    play_at: usize,
) -> [Vec<f32>; 2] {
    let frames = input[0].len();
    let mut out = [vec![0.0f32; frames], vec![0.0f32; frames]];
    let mut out_events = EventBuffer::with_capacity(64);
    let mut block_events: Vec<ProcessEvent> = Vec::with_capacity(64);
    let bps = 2.0 / f64::from(sr);
    let mut pos = 0;
    while pos < frames {
        let n = block.min(frames - pos);
        block_events.clear();
        for (at, kind) in events {
            if *at >= pos && *at < pos + n {
                block_events.push(ProcessEvent {
                    offset: (*at - pos) as u32,
                    kind: *kind,
                });
            }
        }
        let playing = pos >= play_at;
        let transport = TransportInfo {
            playing,
            bpm: 120.0,
            beats_per_sample: if playing { bps } else { 0.0 },
            position: if playing {
                bps * (pos - play_at) as f64
            } else {
                0.0
            },
            sample_time: pos as u64,
            ..TransportInfo::STOPPED
        };
        let inputs: [&[f32]; 2] = [&input[0][pos..pos + n], &input[1][pos..pos + n]];
        let (l, r) = out.split_at_mut(1);
        let mut outputs: [&mut [f32]; 2] = [&mut l[0][pos..pos + n], &mut r[0][pos..pos + n]];
        let mut ctx = ProcessContext {
            sample_rate: sr,
            frames: n,
            transport: &transport,
            events: &block_events,
            out_events: &mut out_events,
        };
        let mut audio = AudioBuffers {
            inputs: &inputs,
            outputs: &mut outputs,
        };
        assert_no_alloc(|| {
            d.process(&mut ctx, &mut audio);
        });
        pos += n;
    }
    out
}

fn render(d: &mut dyn Device, sr: f32, mono: &[f32]) -> [Vec<f32>; 2] {
    render_with(d, sr, [mono, mono], &[], BLOCK, usize::MAX)
}

fn sine(freq: f64, sr: f32, secs: f64) -> Vec<f32> {
    let sr = f64::from(sr);
    (0..(secs * sr) as usize)
        .map(|i| (0.5 * (TAU * freq * i as f64 / sr).sin()) as f32)
        .collect()
}

fn param(id: ParamId, value: f64) -> EventKind {
    EventKind::Param {
        param: id,
        value,
        step: false,
    }
}

/// Instantaneous frequency (Hz) between consecutive rising zero crossings, at their midpoint
/// (seconds): `(time, hz)`.
fn inst_freq(x: &[f32], sr: f32) -> Vec<(f64, f64)> {
    let mut crossings = Vec::new();
    for i in 1..x.len() {
        let (a, b) = (f64::from(x[i - 1]), f64::from(x[i]));
        if a < 0.0 && b >= 0.0 {
            crossings.push((i - 1) as f64 + a / (a - b));
        }
    }
    let sr = f64::from(sr);
    crossings
        .windows(2)
        .map(|w| (0.5 * (w[0] + w[1]) / sr, sr / (w[1] - w[0])))
        .collect()
}

fn cents(hz: f64, reference: f64) -> f64 {
    1200.0 * (hz / reference).log2()
}

/// Expected up / down peaks (cents) for `depth` cents: speed `1 ± sinh(depth · ln2 / 1200)`.
fn peaks(depth: f64) -> (f64, f64) {
    let k = (depth * std::f64::consts::LN_2 / 1200.0).sinh();
    (1200.0 * (1.0 + k).log2(), 1200.0 * (1.0 - k).log2())
}

fn latency_at(sr: f32) -> usize {
    (f64::from(VIBRATO_MAX_EXCURSION_MS) * 0.001 * f64::from(sr)).round() as usize + 4
}

#[test]
fn depth_zero_is_a_pure_delay_of_the_reported_latency() {
    for sr in [44_100.0, 48_000.0, 96_000.0] {
        let mut d = device(sr, &[(p::DEPTH, 0.0)]);
        let lat = latency_at(sr);
        assert_eq!(d.latency() as usize, lat, "{sr}");
        let x = sine(997.0, sr, 0.3);
        let [l, r] = render(d.as_mut(), sr, &x);
        for i in lat..x.len() {
            assert_eq!(l[i], x[i - lat], "{sr} {i}");
            assert_eq!(r[i], x[i - lat]);
        }
        assert!(l[..lat].iter().all(|v| *v == 0.0));
        // Mix with depth 0: dry and wet are aligned, so any mix is the same delay.
        let mut d = device(sr, &[(p::DEPTH, 0.0), (p::MIX, 37.0)]);
        let [l, _] = render(d.as_mut(), sr, &x);
        for i in lat..x.len() {
            assert!((l[i] - x[i - lat]).abs() < 1e-6, "{sr} {i}");
        }
    }
}

#[test]
fn peak_deviation_is_the_depth_at_every_rate_and_sample_rate() {
    for sr in [44_100.0, 48_000.0, 96_000.0] {
        for (rate, depth) in [
            (0.5, 50.0),
            (3.0, 30.0),
            (3.0, 100.0),
            (8.0, 10.0),
            (15.0, 200.0),
        ] {
            let mut d = device(sr, &[(p::RATE, rate), (p::DEPTH, depth)]);
            let f0 = 1500.0;
            let x = sine(f0, sr, 0.4 + 2.5 / rate);
            let [l, _] = render(d.as_mut(), sr, &x);
            let f = inst_freq(&l[latency_at(sr) + 64..], sr);
            let (mut hi, mut lo) = (f64::MIN, f64::MAX);
            for &(_, hz) in &f[20..] {
                hi = hi.max(cents(hz, f0));
                lo = lo.min(cents(hz, f0));
            }
            let (up, down) = peaks(depth);
            let tol = 0.03 * depth + 0.3;
            assert!(
                (hi - up).abs() < tol,
                "{sr} {rate} Hz {depth}c: up {hi} vs {up}"
            );
            assert!(
                (lo - down).abs() < tol,
                "{sr} {rate} Hz {depth}c: down {lo} vs {down}"
            );
            // The up and down peaks straddle the depth.
            assert!(
                ((hi - lo) / 2.0 - depth).abs() < 0.02 * depth + 0.3,
                "{hi} {lo}"
            );
        }
    }
}

#[test]
fn the_pitch_follows_each_periodic_shape() {
    let sr = 48_000.0;
    let (rate, depth, f0) = (2.0, 60.0, 2000.0);
    let k = (depth * std::f64::consts::LN_2 / 1200.0).sinh();
    for shape in 0..5 {
        let mut d = device(
            sr,
            &[(p::SHAPE, shape as f64), (p::RATE, rate), (p::DEPTH, depth)],
        );
        let x = sine(f0, sr, 2.0);
        let lat = latency_at(sr);
        let [l, _] = render(d.as_mut(), sr, &x);
        let f = inst_freq(&l[lat..], sr);
        let mut err2 = 0.0;
        let mut n = 0.0;
        for &(t, hz) in &f[10..] {
            // The output at `t` was read when the LFO stood at `t + latency` (phase 0 at
            // sample 0, one sample advanced before the first read).
            let ph = rate * (t + (lat as f64 + 1.0) / f64::from(sr));
            let expected = k * vibrato_contour(shape, ph);
            let e = hz / f0 - 1.0 - expected;
            // The period-long measurement window smears the saws' fast reset.
            if !(shape >= 3 && (ph.fract() - 0.98).abs() < 0.04) {
                err2 += e * e;
                n += 1.0;
            }
        }
        let rms = (err2 / n).sqrt();
        assert!(rms < 0.03 * k, "shape {shape}: rms {rms} vs k {k}");
    }
}

#[test]
fn random_shapes_wobble_within_the_depth() {
    let sr = 48_000.0;
    let (depth, f0) = (40.0, 1500.0);
    for (shape, rate) in [(5.0, 4.0), (6.0, 1.0), (6.0, 9.0)] {
        let mut d = device(sr, &[(p::SHAPE, shape), (p::RATE, rate), (p::DEPTH, depth)]);
        let x = sine(f0, sr, 6.0);
        let [l, _] = render(d.as_mut(), sr, &x);
        let f = inst_freq(&l[latency_at(sr)..], sr);
        let (up, down) = peaks(depth);
        let (mut hi, mut lo) = (f64::MIN, f64::MAX);
        for &(_, hz) in &f[10..] {
            hi = hi.max(cents(hz, f0));
            lo = lo.min(cents(hz, f0));
        }
        assert!(hi <= up + 1.0 && lo >= down - 1.0, "{shape}: {lo} .. {hi}");
        assert!(
            hi > 0.3 * depth && lo < -0.3 * depth,
            "{shape} barely moves: {lo} .. {hi}"
        );
    }
}

#[test]
fn stereo_phase_puts_the_right_channel_in_opposition() {
    let sr = 48_000.0;
    let f0 = 1000.0;
    let mut d = device(
        sr,
        &[(p::STEREO_PHASE, 180.0), (p::RATE, 2.0), (p::DEPTH, 50.0)],
    );
    let x = sine(f0, sr, 1.5);
    let [l, r] = render(d.as_mut(), sr, &x);
    let (fl, fr) = (inst_freq(&l[2000..], sr), inst_freq(&r[2000..], sr));
    let n = fl.len().min(fr.len());
    let corr: f64 = (0..n).map(|i| (fl[i].1 - f0) * (fr[i].1 - f0)).sum();
    assert!(corr < 0.0, "{corr}");
    // `Phase` offsets both: half a cycle = the default's pitch, inverted.
    let mut a = device(sr, &[(p::RATE, 2.0), (p::DEPTH, 50.0)]);
    let mut b = device(sr, &[(p::RATE, 2.0), (p::DEPTH, 50.0), (p::PHASE, 180.0)]);
    let [la, _] = render(a.as_mut(), sr, &x);
    let [lb, _] = render(b.as_mut(), sr, &x);
    let (fa, fb) = (inst_freq(&la[2000..], sr), inst_freq(&lb[2000..], sr));
    for &(t, hz) in &fa {
        // The other render's measurement nearest in time.
        let j = fb.partition_point(|m| m.0 < t).min(fb.len() - 1);
        let (tb, hb) = fb[j];
        if (tb - t).abs() < 0.3e-3 {
            assert!((hz - f0 + (hb - f0)).abs() < 1.0, "{t}: {hz} {hb}");
        }
    }
}

#[test]
fn changes_do_not_click() {
    let sr = 48_000.0;
    let f0 = 800.0;
    let x = sine(f0, sr, 4.0);
    // The steepest a 0.5-amplitude sine can be at the deepest pitch (+200 cents) is the
    // bound: a click or a jump of the read position would exceed it.
    let bound = 0.5 * TAU * f0 * 2f64.powf(210.0 / 1200.0) / f64::from(sr);
    let s = |secs: f64| (secs * f64::from(sr)) as usize;
    let mut events = vec![
        (s(0.3), param(p::DEPTH, 200.0)),
        (s(0.6), param(p::RATE, 15.0)),
        (s(0.9), param(p::PHASE, 270.0)),
        (s(1.1), param(p::STEREO_PHASE, 180.0)),
        (s(1.3), param(p::RATE, 0.3)),
        (s(1.5), param(p::DEPTH, 5.0)),
        (s(1.6), param(p::MIX, 0.0)),
        (s(1.7), param(p::MIX, 100.0)),
        (s(1.8), param(p::DEPTH, 120.0)),
        (s(1.9), param(p::RATE, 6.0)),
    ];
    for (i, shape) in [2.0, 3.0, 5.0, 4.0, 6.0, 1.0, 0.0].into_iter().enumerate() {
        events.push((s(2.0 + 0.2 * i as f64), param(p::SHAPE, shape)));
    }
    events.push((s(3.5), param(p::SYNC, 1.0)));
    let mut d = device(sr, &[]);
    // The transport starts at 3.7 s: a retrigger.
    let [l, r] = render_with(d.as_mut(), sr, [&x, &x], &events, BLOCK, s(3.7));
    for ch in [&l, &r] {
        assert!(ch.iter().all(|v| v.is_finite()));
        let (at, worst) = ch
            .windows(2)
            .map(|w| f64::from((w[1] - w[0]).abs()))
            .enumerate()
            .fold((0, 0.0), |m, (i, v)| if v > m.1 { (i, v) } else { m });
        assert!(worst < bound, "step {worst} > {bound} at {at}");
    }
}

#[test]
fn renders_are_deterministic_and_block_size_independent() {
    let sr = 48_000.0;
    let x = sine(440.0, sr, 2.0);
    let y: Vec<f32> = x.iter().map(|v| -0.7 * v).collect();
    let events = [
        (10_000, param(p::DEPTH, 80.0)),
        (30_000, param(p::SHAPE, 6.0)),
        (50_000, param(p::PHASE, 90.0)),
        (70_000, param(p::SHAPE, 5.0)),
    ];
    let go = |block: usize| {
        let mut d = device(sr, &[(p::STEREO_PHASE, 60.0), (p::RATE, 4.0)]);
        // The transport starts on a block boundary of both sizes (61 × 256).
        render_with(d.as_mut(), sr, [&x, &y], &events, block, 15_616)
    };
    let a = go(BLOCK);
    assert_eq!(a, go(BLOCK));
    assert_eq!(a, go(61));
}

#[test]
fn no_nan_denormals_or_runaway_on_extreme_input() {
    let sr = 48_000.0;
    let n = 48_000;
    let mut x = vec![0.0f32; n];
    x[100] = 1e6;
    x[200] = -1e6;
    for v in &mut x[1000..2000] {
        *v = 1e-30;
    }
    for shape in 0..7 {
        let mut d = device(
            sr,
            &[
                (p::SHAPE, f64::from(shape)),
                (p::DEPTH, 200.0),
                (p::RATE, 20.0),
            ],
        );
        let [l, r] = render(d.as_mut(), sr, &x);
        for ch in [&l, &r] {
            assert!(ch.iter().all(|v| v.is_finite() && v.abs() <= 1.1e6));
            // Silence after the line drained: exact zeros, no subnormals.
            assert!(ch[3000..].iter().all(|v| *v == 0.0), "shape {shape}");
        }
    }
}

//! Grain Delay render tests. Every `process` call runs under `assert_no_alloc`.
//!
//! CPU cost per instance: `cargo test --release -p ether-devices --test it grain_delay:: --
//! --ignored --nocapture cpu_cost`.

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventBuffer, EventKind, PrepareConfig, ProcessContext, ProcessEvent,
    TransportInfo,
};
use ether_devices::grain_delay::params as gd;

const SR: f32 = 48_000.0;

fn device(params: &[(ParamId, f64)]) -> Box<dyn Device> {
    let mut d = ether_devices::create(
        &BuiltinDevice::new(BuiltinDeviceType::GrainDelay),
        &ether_devices::NoSamples,
    );
    for &(p, v) in params {
        d.set_param(p, v);
    }
    d.prepare(&PrepareConfig {
        sample_rate: SR,
        max_block_size: 512,
        max_events_per_block: 64,
    });
    d
}

/// Render stereo `input` in blocks of `block`; `events` are `(absolute frame, kind)`.
fn run(
    d: &mut dyn Device,
    input: &[Vec<f32>; 2],
    events: &[(usize, EventKind)],
    block: usize,
) -> [Vec<f32>; 2] {
    let frames = input[0].len();
    let mut out = [vec![0.0f32; frames], vec![0.0f32; frames]];
    let mut out_events = EventBuffer::with_capacity(64);
    let mut block_events: Vec<ProcessEvent> = Vec::with_capacity(64);
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
        let transport = TransportInfo {
            playing: true,
            bpm: 120.0,
            sample_time: pos as u64,
            ..TransportInfo::STOPPED
        };
        let inputs: [&[f32]; 2] = [&input[0][pos..pos + n], &input[1][pos..pos + n]];
        let (l, r) = out.split_at_mut(1);
        let mut outputs: [&mut [f32]; 2] = [&mut l[0][pos..pos + n], &mut r[0][pos..pos + n]];
        let mut ctx = ProcessContext {
            sample_rate: SR,
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

fn render(d: &mut dyn Device, input: &[Vec<f32>; 2]) -> [Vec<f32>; 2] {
    run(d, input, &[], 256)
}

fn sine(freq: f32, amp: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| amp * (std::f32::consts::TAU * freq * i as f32 / SR).sin())
        .collect()
}

fn noise(frames: usize, amp: f32) -> Vec<f32> {
    let mut s = 0x2468_ace1u32;
    (0..frames)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            amp * ((s >> 8) as f32 / (1u32 << 23) as f32 - 1.0)
        })
        .collect()
}

fn stereo(x: Vec<f32>) -> [Vec<f32>; 2] {
    [x.clone(), x]
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-12).log10()
}

/// Magnitude of the `freq` component (Goertzel, Hann window).
fn tone(x: &[f32], freq: f32) -> f32 {
    let n = x.len();
    let w = std::f64::consts::TAU * f64::from(freq) / f64::from(SR);
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for (i, v) in x.iter().enumerate() {
        let hann = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
        let s = f64::from(*v) * hann + 2.0 * w.cos() * s1 - s2;
        s2 = s1;
        s1 = s;
    }
    let p = s1 * s1 + s2 * s2 - 2.0 * w.cos() * s1 * s2;
    (p.max(0.0).sqrt() / (n as f64 * 0.25)) as f32
}

fn wet() -> Vec<(ParamId, f64)> {
    vec![(gd::MIX, 100.0), (gd::SPRAY, 0.0)]
}

#[test]
fn extremes_are_finite_without_allocating() {
    let mut d = device(&[]);
    let desc = d.descriptor();
    let input = stereo(noise(96_000, 0.9));
    let mut events = Vec::new();
    let mut at = 7;
    for p in &desc.params {
        for v in [p.min, p.max, f64::NAN, p.default] {
            events.push((
                at,
                EventKind::Param {
                    param: p.id,
                    value: v,
                    step: false,
                },
            ));
            at += 1009;
        }
    }
    for p in &desc.params {
        events.push((
            at,
            EventKind::Param {
                param: p.id,
                value: p.max,
                step: false,
            },
        ));
    }
    let out = run(d.as_mut(), &input, &events, 256);
    for ch in &out {
        for v in ch {
            assert!(v.is_finite() && v.abs() < 8.0, "{v}");
        }
    }
    for p in &desc.params {
        assert_eq!(d.param(p.id), Some(p.max), "{}", p.name);
    }
}

#[test]
fn unpitched_grains_are_a_clean_delay() {
    let mut params = wet();
    params.push((gd::TIME, 50.0));
    let mut d = device(&params);
    let x = noise(48_000, 0.5);
    let out = render(d.as_mut(), &stereo(x.clone()));
    let lag = 2400;
    let start = 4800;
    let err: Vec<f32> = (start..x.len()).map(|i| out[0][i] - x[i - lag]).collect();
    let e = db(rms(&err)) - db(rms(&x));
    assert!(e < -40.0, "residual {e} dB");
}

#[test]
fn pitch_transposes_and_keeps_the_level() {
    let x = sine(440.0, 0.5, 48_000);
    for (semis, want) in [(12.0, 880.0), (-12.0, 220.0), (7.0, 659.25)] {
        let mut params = wet();
        params.push((gd::PITCH, semis));
        params.push((gd::FREQUENCY, 20.0));
        let mut d = device(&params);
        let out = render(d.as_mut(), &stereo(x.clone()));
        let y = &out[0][12_000..];
        let at_want = db(tone(y, want));
        let at_in = db(tone(y, 440.0));
        assert!(at_want > at_in + 15.0, "{semis}: {at_want} vs {at_in}");
        let g = db(rms(y)) - db(rms(&x[12_000..]));
        assert!(g.abs() < 3.0, "{semis}: level {g} dB");
    }
}

#[test]
fn sync_follows_the_tempo() {
    // 1/8 at 120 bpm = 250 ms.
    let mut params = wet();
    params.push((gd::SYNC, 1.0));
    params.push((gd::FREQUENCY, 5.0));
    let mut d = device(&params);
    let mut x = vec![0.0f32; 48_000];
    x[1000] = 1.0;
    let out = render(d.as_mut(), &stereo(x));
    let peak = out[0]
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .unwrap()
        .0;
    assert!((peak as i64 - (1000 + 12_000)).abs() <= 2, "{peak}");
}

#[test]
fn feedback_repeats_and_stays_bounded() {
    let mut params = wet();
    params.push((gd::TIME, 100.0));
    params.push((gd::FEEDBACK, 95.0));
    params.push((gd::PITCH, 12.0));
    let mut d = device(&params);
    let mut x = noise(4800, 0.9);
    x.extend(vec![0.0; 10 * 48_000]);
    let out = render(d.as_mut(), &stereo(x));
    assert!(out[0].iter().all(|v| v.is_finite() && v.abs() < 4.0));
    // Still ringing a second later.
    assert!(rms(&out[0][48_000..52_800]) > 1e-3);
}

#[test]
fn spray_decorrelates_the_channels() {
    let mut d = device(&[(gd::MIX, 100.0), (gd::SPRAY, 100.0)]);
    let out = render(d.as_mut(), &stereo(noise(24_000, 0.5)));
    let side: Vec<f32> = out[0].iter().zip(&out[1]).map(|(l, r)| l - r).collect();
    assert!(db(rms(&side[6000..])) > -20.0);
    let mut d = device(&wet());
    let out = render(d.as_mut(), &stereo(noise(24_000, 0.5)));
    assert_eq!(out[0], out[1]);
}

#[test]
fn silent_after_its_input_without_feedback() {
    let mut d = device(&[(gd::PITCH, -12.0), (gd::SPRAY, 500.0), (gd::TIME, 1000.0)]);
    let mut x = noise(4800, 0.5);
    x.extend(vec![0.0; 6 * 48_000]);
    let out = render(d.as_mut(), &stereo(x));
    assert!(out[0][5 * 48_000..].iter().all(|v| *v == 0.0));
}

#[test]
fn renders_are_block_size_independent() {
    let x = stereo(noise(48_000, 0.5));
    let events = vec![
        (
            700,
            EventKind::Param {
                param: gd::PITCH,
                value: 7.0,
                step: false,
            },
        ),
        (
            9000,
            EventKind::Param {
                param: gd::FEEDBACK,
                value: 60.0,
                step: false,
            },
        ),
        (
            20_001,
            EventKind::Param {
                param: gd::RANDOM_PITCH,
                value: 40.0,
                step: false,
            },
        ),
        (
            30_000,
            EventKind::Param {
                param: gd::FREQUENCY,
                value: 120.0,
                step: false,
            },
        ),
    ];
    let mut a = device(&[]);
    let mut b = device(&[]);
    let ra = run(a.as_mut(), &x, &events, 64);
    let rb = run(b.as_mut(), &x, &events, 512);
    assert_eq!(ra, rb);
}

/// CPU cost per instance (stereo, 48 kHz, 128-sample blocks). Run in release.
#[test]
#[ignore]
fn cpu_cost() {
    let seconds = 10;
    let x = stereo(noise(48_000 * seconds, 0.5));
    for (name, params) in [
        ("grain delay default", vec![]),
        (
            "grain delay 150 Hz grains, pitch, feedback",
            vec![
                (gd::FREQUENCY, 150.0),
                (gd::PITCH, 7.0),
                (gd::RANDOM_PITCH, 50.0),
                (gd::FEEDBACK, 60.0),
            ],
        ),
    ] {
        let mut d = device(&params);
        let t = std::time::Instant::now();
        run(d.as_mut(), &x, &[], 128);
        let el = t.elapsed().as_secs_f64();
        println!(
            "{name}: {:.2} % of a core ({:.1} µs per 128-frame block)",
            100.0 * el / seconds as f64,
            el * 1e6 / (48_000.0 * seconds as f64 / 128.0)
        );
    }
}

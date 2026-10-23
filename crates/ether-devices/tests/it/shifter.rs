//! Shifter render tests. Every `process` call runs under `assert_no_alloc`.
//!
//! CPU cost per instance: `cargo test --release -p ether-devices --test it shifter:: --
//! --ignored --nocapture cpu_cost`.

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventBuffer, EventKind, PrepareConfig, ProcessContext, ProcessEvent,
    TransportInfo,
};
use ether_devices::shifter::{mode, params as sh, waveform};

const SR: f32 = 48_000.0;

fn device(params: &[(ParamId, f64)]) -> Box<dyn Device> {
    let mut d = ether_devices::create(
        &BuiltinDevice::new(BuiltinDeviceType::Shifter),
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

fn set(id: ParamId, value: f64) -> EventKind {
    EventKind::Param {
        param: id,
        value,
        step: false,
    }
}

fn ring(carrier: f64, mix: f64) -> Vec<(ParamId, f64)> {
    vec![
        (sh::MODE, mode::RING as f64),
        (sh::CARRIER, carrier),
        (sh::MIX, mix),
    ]
}

#[test]
fn extremes_are_finite_without_allocating() {
    let mut d = device(&[]);
    let desc = d.descriptor();
    let input = stereo(noise(96_000, 0.9));
    let mut events = Vec::new();
    let mut at = 7;
    for m in 0..3 {
        events.push((at, set(sh::MODE, f64::from(m))));
        at += 101;
        for p in &desc.params {
            if p.id == sh::MODE {
                continue;
            }
            for v in [p.min, p.max, f64::NAN, p.default] {
                events.push((at, set(p.id, v)));
                at += 211;
            }
        }
    }
    for p in &desc.params {
        events.push((at, set(p.id, p.max)));
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
fn ring_mode_multiplies_by_a_cosine() {
    let x = noise(48_000, 0.5);
    let f = 5.5;
    let mut d = device(&ring(f, 100.0));
    let out = render(d.as_mut(), &stereo(x.clone()));
    for (i, (&y, &x)) in out[0].iter().zip(&x).enumerate() {
        let c = (std::f64::consts::TAU * f * i as f64 / f64::from(SR)).cos() as f32;
        assert!((y - x * c).abs() < 1e-5, "{i}: {y} vs {}", x * c);
    }
}

/// The owner's set: ring mode at 1.54 + 4 Hz under 19.8 % wet, driven by 3.56 dB (Live's
/// "Shifter Tremolo"): one dip of about 6.5 dB per carrier cycle.
#[test]
fn a_slow_driven_ring_under_dry_is_a_tremolo() {
    let f = 1.536_974_55 + 4.000_002_86;
    let mut params = ring(1.536_974_55, 19.841_270_15);
    params.push((sh::FINE, 4.000_002_86));
    params.push((sh::DRIVE, 1.0));
    params.push((sh::DRIVE_AMOUNT, 3.5625));
    let mut d = device(&params);
    let x = vec![0.25f32; 48_000];
    let out = render(d.as_mut(), &stereo(x));
    let g = 10f32.powf(3.5625 / 20.0);
    let m = 0.198_412_7f32;
    for (i, &y) in out[0].iter().enumerate() {
        let c = (std::f64::consts::TAU * f * i as f64 / f64::from(SR)).cos() as f32;
        let want = 0.25 * (1.0 - m) + m * (g * 0.25 * c).tanh();
        assert!((y - want).abs() < 1e-4, "{i}: {y} vs {want}");
    }
    let (lo, hi) = out[0]
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    assert!(db(hi / lo) > 5.0 && db(hi / lo) < 8.0, "{lo} .. {hi}");
}

#[test]
fn frequency_mode_moves_every_partial_by_hertz() {
    let x = sine(1000.0, 0.5, 48_000);
    for (shift, want, image) in [(100.0, 1100.0, 900.0), (-250.0, 750.0, 1250.0)] {
        let mut d = device(&[
            (sh::MODE, mode::FREQUENCY as f64),
            (sh::SHIFT, shift),
            (sh::MIX, 100.0),
        ]);
        let out = render(d.as_mut(), &stereo(x.clone()));
        let y = &out[0][4800..];
        let up = db(tone(y, want));
        let other = db(tone(y, image)).max(db(tone(y, 1000.0)));
        assert!(up > -7.0, "{shift}: {up} dB");
        assert!(up > other + 35.0, "{shift}: {up} vs {other}");
    }
}

#[test]
fn wide_mirrors_the_shift_on_the_right() {
    let x = sine(1000.0, 0.5, 48_000);
    let mut d = device(&[
        (sh::MODE, mode::FREQUENCY as f64),
        (sh::SHIFT, 100.0),
        (sh::WIDE, 1.0),
        (sh::MIX, 100.0),
    ]);
    let out = render(d.as_mut(), &stereo(x));
    let (l, r) = (&out[0][4800..], &out[1][4800..]);
    assert!(db(tone(l, 1100.0)) > db(tone(l, 900.0)) + 35.0);
    assert!(db(tone(r, 900.0)) > db(tone(r, 1100.0)) + 35.0);
}

#[test]
fn pitch_mode_transposes_and_keeps_the_level() {
    let x = sine(440.0, 0.5, 48_000);
    for (semis, want) in [(12.0, 880.0), (-12.0, 220.0), (7.0, 659.25), (-24.0, 110.0)] {
        let mut d = device(&[(sh::PITCH, semis), (sh::MIX, 100.0)]);
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
fn unshifted_pitch_mode_is_a_clean_half_window_delay() {
    let mut d = device(&[(sh::MIX, 100.0), (sh::WINDOW, 80.0)]);
    let x = noise(48_000, 0.5);
    let out = render(d.as_mut(), &stereo(x.clone()));
    // 3 guard samples + half of 80 ms.
    let lag = 3 + 1920;
    let err: Vec<f32> = (4800..x.len()).map(|i| out[0][i] - x[i - lag]).collect();
    let e = db(rms(&err)) - db(rms(&x));
    assert!(e < -60.0, "residual {e} dB");
}

#[test]
fn a_mode_change_crossfades() {
    let x = sine(220.0, 0.5, 24_000);
    let events = vec![(12_000, set(sh::MODE, mode::RING as f64))];
    let mut d = device(&[(sh::MODE, mode::FREQUENCY as f64), (sh::MIX, 100.0)]);
    let out = run(d.as_mut(), &stereo(x), &events, 128);
    // A 220 Hz sine at 0.5 moves by at most ~0.0144 per sample; a hard switch from the
    // shifter's (rotated) output to the ring's would jump.
    let jump = out[0][11_000..13_000]
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max);
    assert!(jump < 0.03, "{jump}");
}

#[test]
fn silent_after_its_input() {
    for m in 0..3 {
        let mut d = device(&[(sh::MODE, f64::from(m)), (sh::WINDOW, 350.0)]);
        let mut x = noise(4800, 0.5);
        x.extend(vec![0.0; 2 * 48_000]);
        let out = render(d.as_mut(), &stereo(x));
        assert!(out[0][48_000..].iter().all(|v| v.abs() < 1e-9), "mode {m}");
    }
}

#[test]
fn renders_are_block_size_independent() {
    let x = stereo(noise(48_000, 0.5));
    let events = vec![
        (700, set(sh::PITCH, 7.0)),
        (9000, set(sh::WINDOW, 200.0)),
        (15_000, set(sh::MODE, mode::FREQUENCY as f64)),
        (20_001, set(sh::SHIFT, -300.0)),
        (30_000, set(sh::MODE, mode::RING as f64)),
        (30_500, set(sh::DRIVE, 1.0)),
        (40_000, set(sh::WIDE, 1.0)),
        (2000, set(sh::LFO_PITCH, 5.0)),
        (16_000, set(sh::LFO_DEPTH, 200.0)),
        (25_000, set(sh::LFO_WAVEFORM, 9.0)),
        (26_000, set(sh::LFO_STEREO, 1.0)),
        (26_000, set(sh::LFO_SPIN, 20.0)),
        (33_000, set(sh::ENV_ON, 1.0)),
        (33_000, set(sh::ENV_DEPTH, -400.0)),
        (5_000, set(sh::DELAY_ON, 1.0)),
        (5_000, set(sh::FEEDBACK, 70.0)),
        (12_345, set(sh::DELAY_SYNC, 0.0)),
        (12_345, set(sh::DELAY_TIME, 37.0)),
        (22_222, set(sh::TONE, 2000.0)),
        (44_000, set(sh::DELAY_ON, 0.0)),
    ];
    let mut events = events;
    events.sort_by_key(|e| e.0);
    let mut a = device(&[]);
    let mut b = device(&[]);
    let ra = run(a.as_mut(), &x, &events, 64);
    let rb = run(b.as_mut(), &x, &events, 512);
    assert_eq!(ra, rb);
}

/// A sine LFO sweeps the ring's carrier: `x·cos(2π Σ hz / sr)` with `hz` = carrier +
/// depth · sin(2π · lfo phase), the LFO advancing before each sample.
#[test]
fn the_lfo_sweeps_the_carrier() {
    let (carrier, depth, rate) = (300.0, 200.0, 3.0);
    let mut params = ring(carrier, 100.0);
    params.push((sh::LFO_DEPTH, depth));
    params.push((sh::LFO_RATE, rate));
    let mut d = device(&params);
    let x = vec![0.5f32; 24_000];
    let out = render(d.as_mut(), &stereo(x));
    let sr = f64::from(SR);
    let mut phase = 0.0f64;
    for (i, &y) in out[0].iter().enumerate() {
        let want = (0.5 * (std::f64::consts::TAU * phase).cos()) as f32;
        assert!((y - want).abs() < 2e-3, "{i}: {y} vs {want}");
        let lfo = (std::f64::consts::TAU * rate * (i + 1) as f64 / sr).sin();
        phase = (phase + (carrier + depth * lfo) / sr).fract();
    }
    // Phase 0: both sides the same.
    assert_eq!(out[0], out[1]);
}

#[test]
fn the_stereo_phase_and_spin_split_the_sides() {
    for extra in [
        vec![(sh::LFO_PHASE, 180.0)],
        vec![(sh::LFO_STEREO, 1.0), (sh::LFO_SPIN, 25.0)],
    ] {
        let mut params = vec![
            (sh::MODE, mode::FREQUENCY as f64),
            (sh::SHIFT, 0.0),
            (sh::LFO_DEPTH, 50.0),
            (sh::MIX, 100.0),
        ];
        params.extend(extra);
        let mut d = device(&params);
        let out = render(d.as_mut(), &stereo(sine(1000.0, 0.5, 48_000)));
        let side: Vec<f32> = out[0].iter().zip(&out[1]).map(|(l, r)| l - r).collect();
        assert!(db(rms(&side[4800..])) > -20.0, "{params:?}");
    }
}

#[test]
fn sample_and_hold_width_decorrelates_the_sides() {
    let base = vec![
        (sh::MODE, mode::FREQUENCY as f64),
        (sh::LFO_WAVEFORM, waveform::RANDOM_SH as f64),
        (sh::LFO_RATE, 8.0),
        (sh::LFO_DEPTH, 300.0),
        (sh::MIX, 100.0),
    ];
    let x = stereo(sine(1000.0, 0.5, 48_000));
    let mut narrow = base.clone();
    narrow.push((sh::LFO_WIDTH, 0.0));
    let out = render(device(&narrow).as_mut(), &x);
    assert_eq!(out[0], out[1]);
    let mut wide = base;
    wide.push((sh::LFO_WIDTH, 100.0));
    let out = render(device(&wide).as_mut(), &x);
    assert_ne!(out[0], out[1]);
}

#[test]
fn every_waveform_stays_in_range() {
    for w in 0..10 {
        let mut params = ring(1000.0, 100.0);
        params.push((sh::LFO_WAVEFORM, f64::from(w)));
        params.push((sh::LFO_DEPTH, 5000.0));
        params.push((sh::LFO_RATE, 50.0));
        params.push((sh::LFO_DUTY, 80.0));
        let mut d = device(&params);
        let out = render(d.as_mut(), &stereo(noise(24_000, 0.5)));
        // |carrier ± depth| keeps it a ring modulation: never louder than the input.
        assert!(
            out[0].iter().all(|v| v.is_finite() && v.abs() <= 0.5),
            "{w}"
        );
    }
}

#[test]
fn the_envelope_follower_shifts_with_the_level() {
    let x = sine(1000.0, 0.5, 48_000);
    let mut d = device(&[
        (sh::MODE, mode::FREQUENCY as f64),
        (sh::ENV_ON, 1.0),
        (sh::ENV_ATTACK, 0.1),
        (sh::ENV_RELEASE, 400.0),
        (sh::ENV_DEPTH, 500.0),
        (sh::MIX, 100.0),
    ]);
    let out = render(d.as_mut(), &stereo(x));
    // The follower sits near the 0.5 peak: about +250 Hz.
    let y = &out[0][24_000..];
    let near = (1200..=1300)
        .step_by(10)
        .map(|f| db(tone(y, f as f32)))
        .fold(f32::MIN, f32::max);
    assert!(near > db(tone(y, 1000.0)) + 20.0, "{near}");
    // Off: no shift.
    let mut d = device(&[
        (sh::MODE, mode::FREQUENCY as f64),
        (sh::ENV_DEPTH, 500.0),
        (sh::MIX, 100.0),
    ]);
    let out = render(d.as_mut(), &stereo(sine(1000.0, 0.5, 48_000)));
    assert!(db(tone(&out[0][24_000..], 1000.0)) > -10.0);
}

#[test]
fn the_envelope_bends_the_pitch() {
    let mut d = device(&[
        (sh::ENV_ON, 1.0),
        (sh::ENV_ATTACK, 0.1),
        (sh::ENV_RELEASE, 400.0),
        (sh::ENV_PITCH, 24.0),
        (sh::MIX, 100.0),
    ]);
    // A 0.5 sine holds the follower near 0.5: +12 semitones.
    let out = render(d.as_mut(), &stereo(sine(440.0, 0.5, 48_000)));
    let y = &out[0][24_000..];
    let up = (860..=900)
        .step_by(5)
        .map(|f| db(tone(y, f as f32)))
        .fold(f32::MIN, f32::max);
    assert!(up > db(tone(y, 440.0)) + 10.0, "{up}");
}

/// Index of the largest sample.
fn peak_at(x: &[f32]) -> usize {
    x.iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .unwrap()
        .0
}

#[test]
fn the_delay_echoes_the_shifted_signal() {
    // Unity pitch: the shifter is a 3 + 1920-sample delay, then 100 ms.
    let mut d = device(&[
        (sh::MIX, 100.0),
        (sh::DELAY_ON, 1.0),
        (sh::DELAY_SYNC, 0.0),
        (sh::DELAY_TIME, 100.0),
    ]);
    let mut x = vec![0.0f32; 24_000];
    x[1000] = 1.0;
    let out = render(d.as_mut(), &stereo(x.clone()));
    let at = peak_at(&out[0]) as i64;
    assert!((at - (1000 + 1923 + 4800)).abs() <= 1, "{at}");
    // Synced: 1/8 at 120 bpm = 250 ms.
    let mut d = device(&[
        (sh::MIX, 100.0),
        (sh::MODE, mode::RING as f64),
        (sh::CARRIER, 1.0),
        (sh::DELAY_ON, 1.0),
    ]);
    let out = render(d.as_mut(), &stereo(x));
    let at = peak_at(&out[0]) as i64;
    assert!((at - (1000 + 12_000)).abs() <= 1, "{at}");
}

#[test]
fn feedback_shifts_every_repeat_again() {
    // A 1 kHz burst, shifted up 100 Hz into a 200 ms delay with feedback: the first echo is
    // at 1.1 kHz, the second at 1.2 kHz.
    let mut d = device(&[
        (sh::MODE, mode::FREQUENCY as f64),
        (sh::SHIFT, 100.0),
        (sh::MIX, 100.0),
        (sh::DELAY_ON, 1.0),
        (sh::DELAY_SYNC, 0.0),
        (sh::DELAY_TIME, 200.0),
        (sh::FEEDBACK, 70.0),
    ]);
    let mut x = sine(1000.0, 0.5, 4800);
    x.extend(vec![0.0; 48_000]);
    let out = render(d.as_mut(), &stereo(x));
    let first = &out[0][9600 + 500..9600 + 4300];
    let second = &out[0][19_200 + 500..19_200 + 4300];
    assert!(db(tone(first, 1100.0)) > db(tone(first, 1200.0)) + 20.0);
    assert!(db(tone(second, 1200.0)) > db(tone(second, 1100.0)) + 20.0);
}

#[test]
fn heavy_feedback_stays_bounded_and_tone_darkens_the_repeats() {
    let base = vec![
        (sh::MIX, 100.0),
        (sh::DELAY_ON, 1.0),
        (sh::DELAY_SYNC, 0.0),
        (sh::DELAY_TIME, 50.0),
        (sh::FEEDBACK, 150.0),
    ];
    let mut x = noise(4800, 0.9);
    x.extend(vec![0.0; 4 * 48_000]);
    let out = render(device(&base).as_mut(), &stereo(x.clone()));
    assert!(out[0].iter().all(|v| v.is_finite() && v.abs() < 4.0));
    let mut dark = base.clone();
    dark.push((sh::FEEDBACK, 80.0));
    let mut bright = dark.clone();
    dark.push((sh::TONE, 1000.0));
    bright.push((sh::TONE, 22_000.0));
    let late = |p: &[(ParamId, f64)]| {
        let out = render(device(p).as_mut(), &stereo(x.clone()));
        let y = &out[0][48_000..72_000];
        db(tone(y, 6000.0)) - db(rms(y))
    };
    assert!(late(&dark) < late(&bright) - 10.0);
}

#[test]
fn delay_off_is_bit_identical_and_silence_comes_back() {
    let x = stereo(noise(24_000, 0.5));
    let plain = render(device(&[(sh::PITCH, 5.0)]).as_mut(), &x);
    let off = render(
        device(&[
            (sh::PITCH, 5.0),
            (sh::FEEDBACK, 90.0),
            (sh::DELAY_TIME, 300.0),
        ])
        .as_mut(),
        &x,
    );
    assert_eq!(plain, off);
    // With feedback the tail rings out, then the output goes back to silence.
    let mut d = device(&[
        (sh::DELAY_ON, 1.0),
        (sh::DELAY_SYNC, 0.0),
        (sh::DELAY_TIME, 100.0),
        (sh::FEEDBACK, 50.0),
    ]);
    let mut x = noise(4800, 0.5);
    x.extend(vec![0.0; 10 * 48_000]);
    let out = render(d.as_mut(), &stereo(x));
    assert!(rms(&out[0][9600..14_400]) > 1e-3);
    assert!(out[0][9 * 48_000..].iter().all(|v| v.abs() < 1e-6));
}

/// CPU cost per instance (stereo, 48 kHz, 128-sample blocks). Run in release.
#[test]
#[ignore]
fn cpu_cost() {
    let seconds = 10;
    let x = stereo(noise(48_000 * seconds, 0.5));
    for (name, params) in [
        ("shifter pitch +7", vec![(sh::PITCH, 7.0)]),
        (
            "shifter frequency, lfo + envelope + delay",
            vec![
                (sh::MODE, mode::FREQUENCY as f64),
                (sh::LFO_DEPTH, 100.0),
                (sh::ENV_ON, 1.0),
                (sh::ENV_DEPTH, 200.0),
                (sh::DELAY_ON, 1.0),
                (sh::FEEDBACK, 60.0),
                (sh::TONE, 5000.0),
            ],
        ),
        (
            "shifter frequency +100 Hz",
            vec![(sh::MODE, mode::FREQUENCY as f64), (sh::SHIFT, 100.0)],
        ),
        (
            "shifter ring driven",
            vec![
                (sh::MODE, mode::RING as f64),
                (sh::DRIVE, 1.0),
                (sh::MIX, 20.0),
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

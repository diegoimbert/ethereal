//! `PitchCorrection`: snapping to the chromatic scale and to a key, retune speed and
//! humanize (vibrato kept or flattened), the MIDI target, formant preservation (and the
//! chipmunk shift without it), unvoiced input and the latency, allocation-free processing.
//! Pitches of the output are measured here with a plain autocorrelation (independent of
//! the device's detector).

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventBuffer, EventKind, PrepareConfig, ProcessContext, ProcessEvent,
    TransportInfo,
};
use ether_devices::NoSamples;
use ether_devices::pitch_correction::{Frame, params as p, target};

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;

fn device(params: &[(ParamId, f64)]) -> Box<dyn Device> {
    let mut d = ether_devices::create(
        &BuiltinDevice::new(BuiltinDeviceType::PitchCorrection),
        &NoSamples,
    );
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

/// Runs `x` (both channels); `events` at absolute sample offsets.
fn run(d: &mut dyn Device, x: &[f32], events: &[(usize, EventKind)]) -> Vec<f32> {
    let mut out = [vec![0.0f32; x.len()], vec![0.0f32; x.len()]];
    let mut out_events = EventBuffer::with_capacity(16);
    let transport = TransportInfo {
        playing: true,
        ..TransportInfo::STOPPED
    };
    let mut block_events = Vec::with_capacity(16);
    let mut i = 0;
    while i < x.len() {
        let n = BLOCK.min(x.len() - i);
        block_events.clear();
        for (at, kind) in events {
            if (i..i + n).contains(at) {
                block_events.push(ProcessEvent {
                    offset: (at - i) as u32,
                    kind: *kind,
                });
            }
        }
        let ins: [&[f32]; 2] = [&x[i..i + n], &x[i..i + n]];
        let (l, r) = out.split_at_mut(1);
        let mut outs: [&mut [f32]; 2] = [&mut l[0][i..i + n], &mut r[0][i..i + n]];
        let mut ctx = ProcessContext {
            sample_rate: SR,
            frames: n,
            transport: &transport,
            events: &block_events,
            out_events: &mut out_events,
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
    let [l, _] = out;
    l
}

/// A band-limited-ish voice: a sawtooth with an optional vibrato (`depth` cents at
/// `rate` Hz), from `hz`.
fn voice(hz: f32, seconds: f32, vibrato: (f32, f32)) -> Vec<f32> {
    let n = (seconds * SR) as usize;
    let mut phase = 0.0f64;
    (0..n)
        .map(|i| {
            let t = i as f32 / SR;
            let cents = vibrato.0 * (std::f32::consts::TAU * vibrato.1 * t).sin();
            let f = hz * 2f32.powf(cents / 1200.0);
            phase = (phase + f64::from(f / SR)).fract();
            // A softened saw (two-pole low-pass) so the measurement is clean.
            (0.4 * (2.0 * phase - 1.0)) as f32
        })
        .collect()
}

/// Pitch (Hz) of `x` by normalized autocorrelation (lag 60 Hz ..= 1.2 kHz, the first
/// peak above 0.9 of the best, parabolic refinement).
fn pitch(x: &[f32]) -> f32 {
    let (lo, hi) = ((SR / 1200.0) as usize, (SR / 60.0) as usize);
    let n = x.len() - hi - 2;
    let r = |lag: usize| -> f32 {
        let (mut ab, mut aa, mut bb) = (0.0f64, 0.0f64, 0.0f64);
        for j in 0..n {
            let (a, b) = (f64::from(x[j]), f64::from(x[j + lag]));
            ab += a * b;
            aa += a * a;
            bb += b * b;
        }
        (ab / (aa * bb).sqrt().max(1e-12)) as f32
    };
    let rs: Vec<f32> = (lo..=hi + 1).map(r).collect();
    let best = rs.iter().copied().fold(f32::MIN, f32::max);
    let mut k = 1;
    while k + 1 < rs.len() {
        if rs[k] >= 0.9 * best && rs[k] >= rs[k - 1] && rs[k] >= rs[k + 1] {
            break;
        }
        k += 1;
    }
    let (a, b, c) = (rs[k - 1], rs[k], rs[k + 1]);
    let den = a - 2.0 * b + c;
    let off = if den.abs() > 1e-9 {
        0.5 * (a - c) / den
    } else {
        0.0
    };
    SR / ((lo + k) as f32 + off)
}

fn cents(a: f32, b: f32) -> f32 {
    1200.0 * (a / b).log2()
}

/// Pitch every 20 ms over `x[from..]` (cents relative to `hz`).
fn track(x: &[f32], from: usize, hz: f32) -> Vec<f32> {
    let w = 2048;
    (from..x.len() - w)
        .step_by(960)
        .map(|i| cents(pitch(&x[i..i + w]), hz))
        .collect()
}

fn std_dev(v: &[f32]) -> f32 {
    let m = v.iter().sum::<f32>() / v.len() as f32;
    (v.iter().map(|x| (x - m) * (x - m)).sum::<f32>() / v.len() as f32).sqrt()
}

#[test]
fn snaps_to_the_nearest_note_chromatic_and_in_key() {
    for detector in [0.0, 1.0] {
        // 35 cents sharp of A3.
        let x = voice(220.0 * 2f32.powf(0.35 / 12.0), 1.5, (0.0, 0.0));
        let mut d = device(&[(p::RETUNE, 0.0), (p::DETECTOR, detector)]);
        let y = run(d.as_mut(), &x, &[]);
        let got = pitch(&y[48_000..56_000]);
        eprintln!("detector {detector}: {got:.2} Hz");
        assert!(cents(got, 220.0).abs() < 5.0, "{got}");
    }
    // 270 Hz in C major: C#4 isn't in the key, C4 is nearer than D4.
    let x = voice(270.0, 1.5, (0.0, 0.0));
    let mut d = device(&[(p::RETUNE, 0.0), (p::SCALE, 1.0)]);
    let y = run(d.as_mut(), &x, &[]);
    let got = pitch(&y[48_000..56_000]);
    assert!(cents(got, 261.63).abs() < 5.0, "{got}");
    // The analysis frame says so too.
    let f = frame(d.as_mut());
    assert_eq!(f[Frame::TARGET], 60.0);
    assert!(
        (f[Frame::DETECTED] - 60.54).abs() < 0.05,
        "{}",
        f[Frame::DETECTED]
    );
}

fn frame(d: &mut dyn Device) -> Vec<f32> {
    use ether_core::analysis::{ANALYSIS_FRAMES_PER_PASS, AnalysisFrame, AnalysisSink};
    let mut frames = Box::new([AnalysisFrame::EMPTY; ANALYSIS_FRAMES_PER_PASS]);
    let mut sink = AnalysisSink::new(&mut frames[..]);
    d.analysis(&mut sink);
    assert_eq!(sink.len(), 1);
    frames[0].values().to_vec()
}

#[test]
fn retune_speed_and_humanize_decide_how_much_vibrato_survives() {
    // A3 with a ±40-cent, 5.5 Hz vibrato.
    let x = voice(220.0, 3.0, (40.0, 5.5));
    let spread = |params: &[(ParamId, f64)]| {
        let mut d = device(params);
        let y = run(d.as_mut(), &x, &[]);
        std_dev(&track(&y, 72_000, 220.0))
    };
    let input = std_dev(&track(&x, 72_000, 220.0));
    let hard = spread(&[(p::RETUNE, 0.0)]);
    let slow = spread(&[(p::RETUNE, 400.0)]);
    let medium = spread(&[(p::RETUNE, 40.0)]);
    let humanized = spread(&[(p::RETUNE, 40.0), (p::HUMANIZE, 100.0)]);
    eprintln!(
        "vibrato spread (cents): input {input:.1}, retune 0 {hard:.1}, 40 {medium:.1}, \
         40 + humanize {humanized:.1}, 400 {slow:.1}"
    );
    assert!(hard < 5.0, "{hard}");
    assert!(slow > 0.6 * input, "{slow}");
    assert!(medium > hard && humanized > medium, "{medium} {humanized}");
}

#[test]
fn the_midi_target_follows_the_held_note() {
    let x = voice(300.0, 2.0, (0.0, 0.0));
    let mut d = device(&[(p::RETUNE, 0.0), (p::TARGET, target::MIDI as f64)]);
    let on = EventKind::NoteOn {
        note_id: 1,
        channel: 0,
        key: 64,
        velocity: 0.8,
    };
    let off = EventKind::NoteOff {
        note_id: 1,
        channel: 0,
        key: 64,
        velocity: 0.0,
    };
    let y = run(d.as_mut(), &x, &[(0, on), (48_000, off)]);
    // Held: E4. Released: the scale's nearest note to 300 Hz, D4.
    let held = pitch(&y[30_000..38_000]);
    let after = pitch(&y[84_000..92_000]);
    eprintln!("held {held:.2} Hz, released {after:.2} Hz");
    assert!(cents(held, 329.63).abs() < 5.0, "{held}");
    assert!(cents(after, 293.66).abs() < 5.0, "{after}");
}

/// A voice with one formant: a 150 Hz pulse train through a resonator at `formant` Hz.
fn formant_voice(seconds: f32, formant: f32) -> Vec<f32> {
    let n = (seconds * SR) as usize;
    let period = SR / 150.0;
    let r = 0.995f32;
    let w = std::f32::consts::TAU * formant / SR;
    let (a1, a2) = (-2.0 * r * w.cos(), r * r);
    let (mut y1, mut y2) = (0.0f32, 0.0f32);
    (0..n)
        .map(|i| {
            let pulse = if (i as f32 % period) < 1.0 { 1.0 } else { 0.0 };
            let y = 0.02 * pulse - a1 * y1 - a2 * y2;
            y2 = y1;
            y1 = y;
            y
        })
        .collect()
}

/// The frequency of the strongest harmonic of `x` (a DFT at multiples of `f0`).
fn strongest_harmonic(x: &[f32], f0: f32) -> f32 {
    let mut best = (0.0f32, 0.0f32);
    for h in 1..30 {
        let f = f0 * h as f32;
        let w = std::f32::consts::TAU * f / SR;
        let (mut re, mut im) = (0.0f32, 0.0f32);
        for (i, v) in x.iter().enumerate() {
            let hann = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / x.len() as f32).cos();
            re += v * hann * (w * i as f32).cos();
            im -= v * hann * (w * i as f32).sin();
        }
        let m = re.hypot(im);
        if m > best.1 {
            best = (f, m);
        }
    }
    best.0
}

#[test]
fn formants_stay_put_unless_asked() {
    let x = formant_voice(1.5, 1000.0);
    let up = 150.0 * 2f32.powf(4.0 / 12.0);
    let shifted = |preserve: f64, formant: f64| {
        let mut d = device(&[
            (p::AMOUNT, 0.0),
            (p::TRANSPOSE, 4.0),
            (p::PRESERVE_FORMANTS, preserve),
            (p::FORMANT, formant),
        ]);
        let y = run(d.as_mut(), &x, &[]);
        let seg = &y[40_000..56_000];
        (pitch(seg), strongest_harmonic(seg, pitch(seg)))
    };
    let input = strongest_harmonic(&x[40_000..56_000], 150.0);
    let (f0, kept) = shifted(1.0, 0.0);
    let (_, moved) = shifted(0.0, 0.0);
    let (_, darker) = shifted(1.0, -4.0);
    eprintln!(
        "pitch {f0:.1} Hz (want {up:.1}); strongest harmonic: input {input:.0}, \
         preserved {kept:.0}, not preserved {moved:.0}, formant -4 {darker:.0}"
    );
    assert!(cents(f0, up).abs() < 8.0, "{f0}");
    assert!((kept / input - 1.0).abs() < 0.13, "{kept}");
    assert!(moved / input > 1.15, "{moved}");
    assert!(darker / input < 0.87, "{darker}");
}

#[test]
fn passes_unvoiced_input_and_a_steady_voice_without_correction_as_a_delay() {
    let mut s = 7u32;
    let noise: Vec<f32> = (0..48_000)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s as f32 / u32::MAX as f32 - 0.5) * 0.5
        })
        .collect();
    let mut d = device(&[]);
    let l = d.latency() as usize;
    // About 34 ms at 48 kHz.
    assert!((1500..1800).contains(&l), "{l}");
    let y = run(d.as_mut(), &noise, &[]);
    let err = (10_000..noise.len() - l)
        .map(|i| (y[i + l] - noise[i]).abs())
        .fold(0.0f32, f32::max);
    assert!(err < 1e-3, "noise: {err}");
    // A voice with `Amount` 0 (and no transpose): a delay too.
    let x = voice(220.0, 1.0, (0.0, 0.0));
    let mut d = device(&[(p::AMOUNT, 0.0)]);
    let y = run(d.as_mut(), &x, &[]);
    let rms = |v: &[f32]| (v.iter().map(|a| a * a).sum::<f32>() / v.len() as f32).sqrt();
    let diff: Vec<f32> = (20_000..40_000).map(|i| y[i + l] - x[i]).collect();
    let ratio = 20.0 * (rms(&diff) / rms(&x[20_000..40_000])).log10();
    eprintln!("amount 0: difference {ratio:.1} dB");
    assert!(ratio < -60.0, "{ratio}");
    // In tune already: it stays there.
    let mut d = device(&[(p::RETUNE, 0.0)]);
    let y = run(d.as_mut(), &x, &[]);
    assert!(cents(pitch(&y[30_000..38_000]), 220.0).abs() < 3.0);
}

#[test]
fn processes_without_allocating() {
    let mut d = device(&[(p::TARGET, target::MIDI as f64), (p::FORMANT, 2.0)]);
    let mut x = voice(180.0, 0.5, (30.0, 5.0));
    x[1000] = f32::NAN;
    let mut out = [vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]];
    let mut out_events = EventBuffer::with_capacity(16);
    let transport = TransportInfo {
        playing: true,
        ..TransportInfo::STOPPED
    };
    let on = [ProcessEvent {
        offset: 0,
        kind: EventKind::NoteOn {
            note_id: 3,
            channel: 0,
            key: 55,
            velocity: 1.0,
        },
    }];
    assert_no_alloc(|| {
        for (k, chunk) in x.chunks(BLOCK).enumerate() {
            let n = chunk.len();
            let ins: [&[f32]; 2] = [chunk, chunk];
            let (l, r) = out.split_at_mut(1);
            let mut outs: [&mut [f32]; 2] = [&mut l[0][..n], &mut r[0][..n]];
            let mut ctx = ProcessContext {
                sample_rate: SR,
                frames: n,
                transport: &transport,
                events: if k == 0 { &on } else { &[] },
                out_events: &mut out_events,
            };
            d.process(
                &mut ctx,
                &mut AudioBuffers {
                    inputs: &ins,
                    outputs: &mut outs,
                },
            );
        }
    });
    assert!(out.iter().flatten().all(|v| v.is_finite()));
}

#[test]
#[ignore = "perf: run in release with --ignored --nocapture"]
fn cost_on_a_voice() {
    for (name, params) in [
        ("pyin", vec![]),
        ("yin", vec![(p::DETECTOR, 1.0)]),
        ("pyin, formant +3", vec![(p::FORMANT, 3.0)]),
    ] {
        for hz in [110.0, 220.0, 440.0] {
            let x = voice(hz, 4.0, (30.0, 5.5));
            let mut d = device(&params);
            run(d.as_mut(), &x, &[]);
            let t = std::time::Instant::now();
            run(d.as_mut(), &x, &[]);
            let ns = t.elapsed().as_nanos() as f64 / x.len() as f64;
            println!("{name:<18} {hz:>5} Hz: {ns:6.1} ns per stereo sample");
        }
    }
}

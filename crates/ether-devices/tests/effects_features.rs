//! The features the built-in effects gained, one DSP test each
//! (compressor knee / mix / lookahead / auto makeup / meters, delay filters / modulation /
//! freeze, reverb freeze / filters / diffusion, utility channel mode / mid-side / bass mono /
//! DC filter / mute / meters, EQ channel modes, phaser & flanger LFO shapes, chorus sync,
//! limiter & gate meters, arpeggiator distance). Every `process` runs under
//! `assert_no_alloc` (debug builds abort on an allocation on the audio path). That the new
//! params default to the former sound is `effects_defaults.rs`.

use std::f32::consts::PI;

use assert_no_alloc::assert_no_alloc;
use ether_core::analysis::{AnalysisFrame, AnalysisKind, AnalysisSink};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventBuffer, EventKind, PrepareConfig, ProcessContext, ProcessEvent,
    TransportInfo,
};
use ether_devices::{
    NoSamples, compressor, create, delay, eq, fx_modulation, limiter, midi_fx, reverb, utility,
};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;

fn make(ty: BuiltinDeviceType, params: &[(ParamId, f64)]) -> Box<dyn Device> {
    let mut d = create(&BuiltinDevice::new(ty), &NoSamples);
    d.prepare(&PrepareConfig {
        sample_rate: SR,
        max_block_size: BLOCK,
        max_events_per_block: 64,
    });
    for &(p, v) in params {
        d.set_param(p, v);
    }
    d
}

fn transport(pos: usize) -> TransportInfo {
    let bps = 2.0 / f64::from(SR);
    TransportInfo {
        playing: true,
        sample_time: pos as u64,
        position: pos as f64 * bps,
        seconds: pos as f64 / f64::from(SR),
        bpm: 120.0,
        beats_per_sample: bps,
        ..TransportInfo::STOPPED
    }
}

/// Render stereo `l`/`r` through `d`; returns the outputs.
fn run(d: &mut dyn Device, l: &[f32], r: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let frames = l.len();
    let mut ol = vec![0.0f32; frames];
    let mut or = vec![0.0f32; frames];
    let mut out_events = EventBuffer::with_capacity(64);
    let mut pos = 0;
    while pos < frames {
        let n = BLOCK.min(frames - pos);
        let t = transport(pos);
        let inputs: [&[f32]; 2] = [&l[pos..pos + n], &r[pos..pos + n]];
        let mut outputs: [&mut [f32]; 2] = [&mut ol[pos..pos + n], &mut or[pos..pos + n]];
        let mut ctx = ProcessContext {
            sample_rate: SR,
            frames: n,
            transport: &t,
            events: &[],
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
    (ol, or)
}

fn mono(d: &mut dyn Device, x: &[f32]) -> (Vec<f32>, Vec<f32>) {
    run(d, x, x)
}

fn levels(d: &mut dyn Device) -> Vec<f32> {
    assert!(d.has_analysis());
    let mut frames = [AnalysisFrame::EMPTY; 4];
    let n = {
        let mut sink = AnalysisSink::new(&mut frames);
        assert_no_alloc(|| d.analysis(&mut sink));
        sink.len()
    };
    assert_eq!(n, 1);
    assert_eq!(frames[0].kind, AnalysisKind::Levels);
    frames[0].values().to_vec()
}

fn sine(freq: f32, amp: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| amp * (2.0 * PI * freq * i as f32 / SR).sin())
        .collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

fn finite(x: &[f32]) -> bool {
    x.iter().all(|v| v.is_finite())
}

const S: usize = SR as usize;

// ---- Compressor ------------------------------------------------------------------------

#[test]
fn compressor_knee_mix_lookahead_auto_makeup_and_meters() {
    use compressor::params as p;
    let loud = sine(1000.0, 0.5, S); // ≈ -6 dBFS peak
    let comp = |extra: &[(ParamId, f64)]| {
        let mut all = vec![(p::THRESHOLD, -12.0), (p::RATIO, 8.0), (p::ATTACK, 0.1)];
        all.extend_from_slice(extra);
        make(BuiltinDeviceType::Compressor, &all)
    };
    let tail = |o: &[f32]| peak(&o[S / 2..]);
    // Knee: a level 3 dB under the threshold is untouched by a hard knee and already reduced
    // inside a 24 dB soft knee.
    let under = |knee: f64| {
        let all = [
            (p::THRESHOLD, -3.0),
            (p::RATIO, 8.0),
            (p::ATTACK, 0.1),
            (p::KNEE, knee),
        ];
        tail(&mono(&mut *make(BuiltinDeviceType::Compressor, &all), &loud).0)
    };
    assert!((under(0.0) - 0.5).abs() < 1e-4);
    assert!(under(24.0) < 0.5 * 0.9, "soft knee {}", under(24.0));
    let hard = tail(&mono(&mut *comp(&[]), &loud).0);
    // Mix 0 % = the dry signal (latency-aligned, here none).
    let (dry, _) = mono(&mut *comp(&[(p::MIX, 0.0)]), &loud);
    assert!((tail(&dry) - 0.5).abs() < 1e-3);
    // 50 % sits between.
    let half = tail(&mono(&mut *comp(&[(p::MIX, 50.0)]), &loud).0);
    assert!(half > hard && half < 0.5);
    // Lookahead: reported as latency, the audio is delayed by exactly that much.
    let mut la = comp(&[(p::LOOKAHEAD, 5.0), (p::THRESHOLD, 0.0)]);
    assert_eq!(la.latency(), 240);
    let mut click = vec![0.0f32; 2048];
    click[10] = 0.1;
    let (o, _) = mono(&mut *la, &click);
    assert_eq!(o[10 + 240], 0.1);
    assert_eq!(o[10], 0.0);
    // Auto makeup lifts the compressed output (half the reduction at 0 dBFS).
    let auto = tail(&mono(&mut *comp(&[(p::AUTO_MAKEUP, 1.0)]), &loud).0);
    assert!(auto > hard * 1.5, "auto {auto} hard {hard}");
    // Meters: [gain reduction <= 0, detector level ≈ -6 dBFS].
    let mut c = comp(&[]);
    mono(&mut *c, &loud);
    let v = levels(&mut *c);
    assert_eq!(v.len(), 2);
    assert!(v[0] < -3.0, "{v:?}");
    assert!((v[1] + 6.02).abs() < 0.2, "{v:?}");
    // Read resets them.
    let v = levels(&mut *c);
    assert_eq!(v[0], 0.0);
}

// ---- Delay -----------------------------------------------------------------------------

#[test]
fn delay_feedback_filters_modulation_and_freeze() {
    use delay::params as p;
    let tone = |hz| {
        let mut x = sine(hz, 0.5, S / 10);
        x.resize(S, 0.0);
        x
    };
    let wet = &[(p::MIX, 100.0), (p::TIME, 100.0), (p::FEEDBACK, 80.0)];
    let echo_energy = |extra: &[(ParamId, f64)], x: &[f32]| {
        let mut all = wet.to_vec();
        all.extend_from_slice(extra);
        let (o, _) = mono(&mut *make(BuiltinDeviceType::Delay, &all), x);
        assert!(finite(&o));
        rms(&o[S / 5..])
    };
    // Low cut at 2 kHz thins a 100 Hz tone's repeats out; high cut at 500 Hz darkens 5 kHz.
    let low = tone(100.0);
    assert!(echo_energy(&[(p::LOW_CUT, 2000.0)], &low) < 0.1 * echo_energy(&[], &low));
    let high = tone(5000.0);
    assert!(echo_energy(&[(p::HIGH_CUT, 500.0)], &high) < 0.1 * echo_energy(&[], &high));
    // Modulation changes the echoes but keeps them in range.
    let m = echo_energy(&[(p::MOD_DEPTH, 100.0), (p::MOD_RATE, 3.0)], &low);
    assert!(m > 0.0);
    // Freeze: the loop holds what it had at unity and ignores new input.
    let mut d = make(
        BuiltinDeviceType::Delay,
        &[(p::MIX, 100.0), (p::TIME, 100.0), (p::FEEDBACK, 10.0)],
    );
    let burst = sine(440.0, 0.5, S / 20);
    mono(&mut *d, &burst);
    d.set_param(p::FREEZE, 1.0);
    let (o, _) = mono(&mut *d, &vec![0.0; 3 * S]);
    let (a, b) = (rms(&o[S / 2..S]), rms(&o[2 * S..3 * S]));
    assert!(
        a > 0.01 && (a - b).abs() < 0.05 * a,
        "frozen loop {a} → {b}"
    );
    // Unfrozen with 10 % feedback, the same tail dies out.
    d.set_param(p::FREEZE, 0.0);
    let (o, _) = mono(&mut *d, &vec![0.0; S]);
    assert!(rms(&o[S / 2..]) < 1e-4);
}

// ---- Reverb ----------------------------------------------------------------------------

#[test]
fn reverb_freeze_filters_and_diffusion() {
    use reverb::params as p;
    let mut burst = sine(3000.0, 0.5, S / 20);
    burst.resize(S / 10, 0.0);
    let wet = [(p::MIX, 100.0), (p::DECAY, 1.0), (p::PRE_DELAY, 0.0)];
    // Freeze: the tail sustains instead of falling 60 dB per second.
    let mut r = make(BuiltinDeviceType::Reverb, &wet);
    mono(&mut *r, &burst);
    r.set_param(p::FREEZE, 1.0);
    let (o, _) = mono(&mut *r, &vec![0.0; 4 * S]);
    let (a, b) = (rms(&o[S / 2..S]), rms(&o[3 * S..4 * S]));
    assert!(finite(&o));
    assert!(a > 1e-3 && b > 0.5 * a, "frozen tail {a} → {b}");
    let mut plain = make(BuiltinDeviceType::Reverb, &wet);
    mono(&mut *plain, &burst);
    let (o, _) = mono(&mut *plain, &vec![0.0; 4 * S]);
    assert!(rms(&o[3 * S..4 * S]) < 1e-4);
    // Input high cut at 500 Hz removes most of a 3 kHz burst from the tail.
    let tail = |extra: &[(ParamId, f64)]| {
        let mut all = wet.to_vec();
        all.extend_from_slice(extra);
        let (o, _) = mono(&mut *make(BuiltinDeviceType::Reverb, &all), &burst);
        rms(&o)
    };
    assert!(tail(&[(p::HIGH_CUT, 500.0)]) < 0.2 * tail(&[]));
    // Diffusion changes the sound (allpass gain), stays stable at its maximum.
    let (d0, _) = mono(
        &mut *make(BuiltinDeviceType::Reverb, &[(p::DIFFUSION, 0.0)]),
        &burst,
    );
    let (d1, _) = mono(
        &mut *make(BuiltinDeviceType::Reverb, &[(p::DIFFUSION, 100.0)]),
        &burst,
    );
    assert!(finite(&d1) && d0 != d1);
}

#[test]
fn reverb_output_trims_only_the_wet() {
    use reverb::params as p;
    let l = sine(440.0, 0.5, S / 2);
    let r = sine(660.0, 0.4, S / 2);
    let render = |extra: &[(ParamId, f64)]| {
        let mut all = vec![(p::MIX, 100.0), (p::DECAY, 1.5)];
        all.extend_from_slice(extra);
        run(&mut *make(BuiltinDeviceType::Reverb, &all), &l, &r)
    };
    // 0 dB (the default) is bit-identical to not setting it (`effects_defaults` pins the
    // pre-`Output` sound).
    let (base_l, base_r) = render(&[]);
    assert_eq!(
        (base_l.clone(), base_r.clone()),
        render(&[(p::OUTPUT, 0.0)])
    );
    // -6 dB halves the wet amplitude (10^(-6/20) = 0.501), sample for sample.
    let (ol, or) = render(&[(p::OUTPUT, -6.0)]);
    let k = 10f32.powf(-6.0 / 20.0);
    for (a, b) in base_l.iter().chain(&base_r).zip(ol.iter().chain(&or)) {
        assert!(
            (a * k - b).abs() <= 1e-6 * a.abs().max(1.0),
            "{a} * {k} != {b}"
        );
    }
    assert!((rms(&ol) / rms(&base_l) - 0.501).abs() < 1e-3);
    // The dry path is untouched: at 0 % mix the trim changes nothing.
    let dry = |db: f64| {
        run(
            &mut *make(BuiltinDeviceType::Reverb, &[(p::MIX, 0.0), (p::OUTPUT, db)]),
            &l,
            &r,
        )
    };
    assert_eq!(dry(0.0), dry(-36.0));
}

// ---- Utility ---------------------------------------------------------------------------

#[test]
fn utility_channel_mode_mid_side_bass_mono_dc_mute_and_meters() {
    use utility::params as p;
    let l = sine(440.0, 0.5, S / 4);
    let r = sine(660.0, 0.25, S / 4);
    let run_u = |params: &[(ParamId, f64)], l: &[f32], r: &[f32]| {
        run(&mut *make(BuiltinDeviceType::Utility, params), l, r)
    };
    // Channel Left: both outputs are the left input; Swap exchanges them.
    let (ol, or) = run_u(&[(p::CHANNEL, 1.0)], &l, &r);
    assert_eq!(ol, l);
    assert_eq!(or, l);
    let same = |a: &[f32], b: &[f32]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);
    let (ol, or) = run_u(&[(p::CHANNEL, 3.0)], &l, &r);
    assert!(same(&ol, &r) && same(&or, &l));
    // Mid/Side mode at -100 %: mid only (L == R); at +100 %: side only (L == -R).
    let (ol, or) = run_u(&[(p::WIDTH_MODE, 1.0), (p::MID_SIDE, -100.0)], &l, &r);
    assert!(ol.iter().zip(&or).all(|(a, b)| (a - b).abs() < 1e-6));
    let (ol, or) = run_u(&[(p::WIDTH_MODE, 1.0), (p::MID_SIDE, 100.0)], &l, &r);
    assert!(ol.iter().zip(&or).all(|(a, b)| (a + b).abs() < 1e-6));
    // Bass mono: a pure-side 50 Hz tone (L = -R) is removed below 200 Hz; a 2 kHz one stays.
    let lo = sine(50.0, 0.5, S);
    let lo_neg: Vec<f32> = lo.iter().map(|v| -v).collect();
    let (ol, _) = run_u(
        &[(p::BASS_MONO, 1.0), (p::BASS_MONO_FREQ, 200.0)],
        &lo,
        &lo_neg,
    );
    // 12 dB/oct: two octaves under the crossover the side is down ≈ 24 dB.
    assert!(rms(&ol[S / 2..]) < 0.08 * rms(&lo), "{}", rms(&ol[S / 2..]));
    let hi = sine(2000.0, 0.5, S);
    let hi_neg: Vec<f32> = hi.iter().map(|v| -v).collect();
    let (ol, _) = run_u(
        &[(p::BASS_MONO, 1.0), (p::BASS_MONO_FREQ, 200.0)],
        &hi,
        &hi_neg,
    );
    assert!(rms(&ol[S / 2..]) > 0.9 * rms(&hi));
    // DC filter removes an offset.
    let dc = vec![0.3f32; S];
    let (ol, _) = run_u(&[(p::DC_FILTER, 1.0)], &dc, &dc);
    assert!(ol[S - 1].abs() < 1e-3);
    // Mute: silence once the ramp is done.
    let (ol, or) = run_u(&[(p::MUTE, 1.0)], &l, &r);
    assert!(peak(&ol) == 0.0 && peak(&or) == 0.0);
    // Meters: output peaks per side.
    let mut u = make(BuiltinDeviceType::Utility, &[]);
    run(&mut *u, &l, &r);
    let v = levels(&mut *u);
    assert!(
        (v[0] + 6.02).abs() < 0.1 && (v[1] + 12.04).abs() < 0.1,
        "{v:?}"
    );
}

// ---- EQ --------------------------------------------------------------------------------

#[test]
fn eq_channel_modes() {
    use eq::params as p;
    let boost = [(p::band(3, p::GAIN), 12.0), (p::band(3, p::FREQ), 1000.0)];
    let x = sine(1000.0, 0.25, S / 2);
    let with = |channel: f64| {
        let mut all = boost.to_vec();
        all.push((p::CHANNEL, channel));
        mono(&mut *make(BuiltinDeviceType::Eq, &all), &x)
    };
    // Left: the right side passes untouched, the left is boosted.
    let (ol, or) = with(1.0);
    assert!(or.iter().zip(&x).all(|(a, b)| (a - b).abs() < 1e-6));
    assert!(rms(&ol[S / 4..]) > 3.0 * rms(&x));
    // Side on a mono input (no side): unchanged; Mid: boosted on both sides.
    let (ol, or) = with(4.0);
    assert!(ol.iter().zip(&x).all(|(a, b)| (a - b).abs() < 1e-6));
    assert!(or.iter().zip(&x).all(|(a, b)| (a - b).abs() < 1e-6));
    let (ol, or) = with(3.0);
    assert!(rms(&ol[S / 4..]) > 3.0 * rms(&x) && rms(&or[S / 4..]) > 3.0 * rms(&x));
}

// ---- Modulation ------------------------------------------------------------------------

#[test]
fn phaser_and_flanger_lfo_shapes_and_chorus_sync() {
    let x = sine(800.0, 0.3, S);
    for (ty, shape) in [
        (BuiltinDeviceType::Phaser, fx_modulation::phaser::LFO_SHAPE),
        (
            BuiltinDeviceType::Flanger,
            fx_modulation::flanger::LFO_SHAPE,
        ),
    ] {
        let (sine_out, _) = mono(&mut *make(ty, &[]), &x);
        for s in 1..5 {
            let (o, _) = mono(&mut *make(ty, &[(shape, f64::from(s))]), &x);
            assert!(finite(&o));
            assert_ne!(o, sine_out, "{ty:?} shape {s}");
        }
    }
    use fx_modulation::chorus as c;
    let (free, _) = mono(&mut *make(BuiltinDeviceType::Chorus, &[]), &x);
    let (synced, _) = mono(
        &mut *make(
            BuiltinDeviceType::Chorus,
            &[(c::SYNC, 1.0), (c::SYNC_RATE, 8.0)],
        ),
        &x,
    );
    assert!(finite(&synced));
    assert_ne!(free, synced);
}

// ---- Limiter / gate meters ---------------------------------------------------------------

#[test]
fn limiter_and_gate_meters() {
    let x = sine(1000.0, 0.9, S / 2);
    let mut l = make(BuiltinDeviceType::Limiter, &[(limiter::params::GAIN, 12.0)]);
    mono(&mut *l, &x);
    let v = levels(&mut *l);
    assert_eq!(v.len(), 2);
    assert!(v[0] < -6.0, "gain reduction {v:?}");
    assert!(v[1] > 6.0, "input-gained detector peak {v:?}");
    let mut g = make(BuiltinDeviceType::Gate, &[]);
    mono(&mut *g, &x);
    let v = levels(&mut *g);
    assert_eq!(v.len(), 2);
    assert!((v[1] + 0.9).abs() < 0.2, "detector level {v:?}");
}

// ---- Arpeggiator -----------------------------------------------------------------------

#[test]
fn arpeggiator_distance_transposes_each_repeat() {
    use midi_fx::arpeggiator as arp;
    let mut d = midi_fx::create(&BuiltinDevice::new(BuiltinDeviceType::Arpeggiator));
    d.prepare(&PrepareConfig {
        sample_rate: SR,
        max_block_size: BLOCK,
        max_events_per_block: 64,
    });
    d.set_param(arp::OCTAVES, 3.0);
    d.set_param(arp::DISTANCE, 7.0);
    let mut keys = Vec::new();
    let mut out_events = EventBuffer::with_capacity(256);
    let note = [ProcessEvent {
        offset: 0,
        kind: EventKind::NoteOn {
            note_id: 1,
            channel: 0,
            key: 60,
            velocity: 0.8,
        },
    }];
    let mut pos = 0;
    while pos < S {
        let t = transport(pos);
        out_events.clear();
        let events: &[ProcessEvent] = if pos == 0 { &note } else { &[] };
        let mut ctx = ProcessContext {
            sample_rate: SR,
            frames: BLOCK,
            transport: &t,
            events,
            out_events: &mut out_events,
        };
        let mut audio = AudioBuffers {
            inputs: &[],
            outputs: &mut [],
        };
        assert_no_alloc(|| {
            d.process(&mut ctx, &mut audio);
        });
        for e in out_events.as_slice() {
            if let EventKind::NoteOn { key, .. } = e.kind {
                keys.push(key);
            }
        }
        pos += BLOCK;
    }
    // Sixteenths at 120 BPM for 1 s: C4, G4, D5, C4, ...
    assert_eq!(&keys[..4], &[60, 67, 74, 60]);
}

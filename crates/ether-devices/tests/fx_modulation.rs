//! `fx-modulation` render tests: Chorus, Phaser, Flanger, Tremolo/Auto-Pan. Every `process`
//! call runs under `assert_no_alloc` (debug builds abort on allocation on the audio path).

use std::f32::consts::PI;

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventBuffer, EventKind, PrepareConfig, ProcessContext, ProcessEvent,
    TransportInfo,
};
use ether_devices::NoSamples;
use ether_devices::fx_modulation::{chorus, flanger, phaser, tremolo};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;
const TYPES: [BuiltinDeviceType; 4] = [
    BuiltinDeviceType::Chorus,
    BuiltinDeviceType::Phaser,
    BuiltinDeviceType::Flanger,
    BuiltinDeviceType::Tremolo,
];

fn device(ty: BuiltinDeviceType, params: &[(ParamId, f64)]) -> Box<dyn Device> {
    let mut d = ether_devices::create(&BuiltinDevice::new(ty), &NoSamples);
    for &(id, v) in params {
        d.set_param(id, v);
    }
    d.prepare(&PrepareConfig {
        sample_rate: SR,
        max_block_size: BLOCK,
        max_events_per_block: 64,
    });
    d
}

/// How the transport runs during a render.
#[derive(Clone, Copy)]
struct Play {
    playing: bool,
    bpm: f64,
    /// Song position (beats) at frame 0.
    start: f64,
}

const STOPPED: Play = Play {
    playing: false,
    bpm: 120.0,
    start: 0.0,
};

/// Render stereo `[left, right]` through `d` in blocks of `block`; `events` are `(absolute
/// frame, kind)`, sorted. Returns `[left, right]`.
fn render_with(
    d: &mut dyn Device,
    input: [&[f32]; 2],
    events: &[(usize, EventKind)],
    block: usize,
    play: Play,
) -> [Vec<f32>; 2] {
    let frames = input[0].len();
    let mut out = [vec![0.0f32; frames], vec![0.0f32; frames]];
    let mut out_events = EventBuffer::with_capacity(64);
    let mut block_events: Vec<ProcessEvent> = Vec::with_capacity(64);
    let bps = play.bpm / 60.0 / f64::from(SR);
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
            playing: play.playing,
            bpm: play.bpm,
            beats_per_sample: if play.playing { bps } else { 0.0 },
            position: play.start + if play.playing { bps * pos as f64 } else { 0.0 },
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
        assert!(out_events.is_empty());
        pos += n;
    }
    out
}

fn render(d: &mut dyn Device, mono: &[f32], events: &[(usize, EventKind)]) -> [Vec<f32>; 2] {
    render_with(d, [mono, mono], events, BLOCK, STOPPED)
}

fn sine(freq: f32, amp: f32, secs: f32) -> Vec<f32> {
    (0..(secs * SR) as usize)
        .map(|i| amp * (2.0 * PI * freq * i as f32 / SR).sin())
        .collect()
}

/// Deterministic white noise in `-amp..amp`.
fn noise(amp: f32, secs: f32) -> Vec<f32> {
    let mut s: u32 = 0x1234_5678;
    (0..(secs * SR) as usize)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            amp * ((s as f32 / u32::MAX as f32) * 2.0 - 1.0)
        })
        .collect()
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// Largest second difference (a smooth signal's is ~ω²·amp; steps/zipper show up as spikes).
fn max_curvature(x: &[f32]) -> f32 {
    x.windows(3)
        .map(|w| (w[0] - 2.0 * w[1] + w[2]).abs())
        .fold(0.0, f32::max)
}

fn param(id: ParamId, value: f64) -> EventKind {
    EventKind::Param { param: id, value }
}

fn secs(s: f32) -> usize {
    (s * SR) as usize
}

// ─── all devices ───────────────────────────────────────────────────────────────────────

/// Extreme and NaN param values arriving as sample-offset events, stereo noise: output stays
/// finite and bounded, and nothing allocates.
#[test]
fn extreme_params_stay_finite_without_allocating() {
    for ty in TYPES {
        let desc = ether_devices::descriptor(ty);
        let mut events = Vec::new();
        for (k, value) in [(0, 0.0), (1, f64::NAN), (2, f64::INFINITY), (3, -1e9)] {
            for p in &desc.params {
                let v = match k {
                    0 => p.max,
                    3 => p.min,
                    _ => value,
                };
                events.push((k * 9_000 + p.id.0 as usize * 7, param(p.id, v)));
            }
        }
        events.sort_by_key(|e| e.0);
        let mut d = device(ty, &[]);
        let n = noise(0.5, 1.0);
        let mut n2 = n.clone();
        n2.reverse();
        let [l, r] = render_with(d.as_mut(), [&n, &n2], &events, BLOCK, STOPPED);
        for x in l.iter().chain(&r) {
            assert!(x.is_finite(), "{ty:?}");
        }
        // Output at +24 dB (x15.8) on a 0.5 input with resonant feedback: bounded by the
        // feedback soft clip.
        assert!(peak(&l).max(peak(&r)) < 80.0, "{ty:?}: {}", peak(&l));
        // NaN resets to the default, everything else was clamped.
        for p in &desc.params {
            let v = d.param(p.id).unwrap();
            assert!(v >= p.min && v <= p.max, "{ty:?} {}", p.name);
        }
    }
}

/// Maximum feedback on a loud input stays bounded, and the tail decays to exact zeros
/// (flushed states: no denormal tails).
#[test]
fn max_feedback_is_stable_and_tails_flush_to_zero() {
    let cases: [(BuiltinDeviceType, Vec<(ParamId, f64)>); 4] = [
        (
            BuiltinDeviceType::Chorus,
            vec![
                (chorus::FEEDBACK, 95.0),
                (chorus::VOICES, 4.0),
                (chorus::DEPTH, 100.0),
            ],
        ),
        (
            BuiltinDeviceType::Phaser,
            vec![
                (phaser::FEEDBACK, 95.0),
                (phaser::STAGES, 4.0),
                (phaser::DEPTH, 100.0),
            ],
        ),
        (
            BuiltinDeviceType::Flanger,
            vec![
                (flanger::FEEDBACK, -95.0),
                (flanger::DEPTH, 100.0),
                (flanger::DELAY, 0.1),
            ],
        ),
        (BuiltinDeviceType::Tremolo, vec![(tremolo::DEPTH, 100.0)]),
    ];
    for (ty, params) in cases {
        let mut d = device(ty, &params);
        let mut x = noise(1.0, 2.0);
        x.extend(std::iter::repeat_n(0.0, secs(8.0)));
        let [l, _] = render(d.as_mut(), &x, &[]);
        assert!(peak(&l) < 30.0, "{ty:?}: {}", peak(&l));
        let tail = &l[l.len() - secs(0.5)..];
        assert!(tail.iter().all(|v| *v == 0.0), "{ty:?}: {:e}", peak(tail));
    }
}

/// Output is independent of the block size (params land at their sample, LFOs advance per
/// sample), including synced LFOs following the song position.
#[test]
fn output_is_sample_accurate_and_block_size_independent() {
    let x = noise(0.5, 1.0);
    let play = Play {
        playing: true,
        bpm: 128.0,
        start: 3.3,
    };
    let cases: [(BuiltinDeviceType, Vec<(usize, EventKind)>); 4] = [
        (
            BuiltinDeviceType::Chorus,
            vec![
                (1_001, param(chorus::DELAY, 20.0)),
                (20_017, param(chorus::MIX, 90.0)),
            ],
        ),
        (
            BuiltinDeviceType::Phaser,
            vec![
                (0, param(phaser::SYNC, 1.0)),
                (7_777, param(phaser::CENTER, 3000.0)),
            ],
        ),
        (
            BuiltinDeviceType::Flanger,
            vec![
                (5, param(flanger::FEEDBACK, -60.0)),
                (12_345, param(flanger::DEPTH, 90.0)),
            ],
        ),
        (
            BuiltinDeviceType::Tremolo,
            vec![
                (0, param(tremolo::SYNC, 1.0)),
                (9_999, param(tremolo::SHAPE, 2.0)),
            ],
        ),
    ];
    for (ty, events) in cases {
        let mut a = device(ty, &[]);
        let mut b = device(ty, &[]);
        let [la, ra] = render_with(a.as_mut(), [&x, &x], &events, 256, play);
        let [lb, rb] = render_with(b.as_mut(), [&x, &x], &events, 37, play);
        let err = la
            .iter()
            .zip(&lb)
            .chain(ra.iter().zip(&rb))
            .map(|(p, q)| (p - q).abs())
            .fold(0.0, f32::max);
        assert!(err < 1e-4, "{ty:?}: {err}");
    }
}

/// `Mix` 0 % passes the input unchanged (bit-exact, no latency) on the delay-based effects.
#[test]
fn dry_mix_is_transparent() {
    let x = noise(0.5, 0.5);
    for (ty, mix) in [
        (BuiltinDeviceType::Chorus, chorus::MIX),
        (BuiltinDeviceType::Phaser, phaser::MIX),
        (BuiltinDeviceType::Flanger, flanger::MIX),
    ] {
        let mut d = device(ty, &[(mix, 0.0)]);
        let [l, r] = render(d.as_mut(), &x, &[]);
        assert_eq!(l, x, "{ty:?}");
        assert_eq!(r, x, "{ty:?}");
    }
    let mut d = device(BuiltinDeviceType::Tremolo, &[(tremolo::DEPTH, 0.0)]);
    assert_eq!(render(d.as_mut(), &x, &[])[0], x);
}

/// `Output` changes land at their sample and ramp over one automation grid interval.
#[test]
fn output_gain_is_sample_accurate() {
    let x = vec![1.0f32; 4_000];
    let mut d = device(BuiltinDeviceType::Tremolo, &[(tremolo::DEPTH, 0.0)]);
    let [l, _] = render(d.as_mut(), &x, &[(1_000, param(tremolo::OUTPUT, -6.0))]);
    assert_eq!(l[999], 1.0);
    assert!(l[1_000] < 1.0);
    let target = 10f32.powf(-6.0 / 20.0);
    assert!((l[1_031] - target).abs() < 1e-6, "{}", l[1_031]);
    assert!((l[1_016] - (1.0 + target) / 2.0).abs() < 0.02);
}

// ─── chorus ────────────────────────────────────────────────────────────────────────────

/// Modulated delay reads are interpolated: a vibrato at full depth and a large `Delay` jump
/// stay smooth (no zipper noise or clicks).
#[test]
fn chorus_modulation_and_delay_jumps_are_smooth() {
    let x = sine(1_000.0, 0.5, 2.0);
    let base = max_curvature(&x);
    let mut d = device(
        BuiltinDeviceType::Chorus,
        &[
            (chorus::MODE, 2.0),
            (chorus::DEPTH, 100.0),
            (chorus::RATE, 2.0),
        ],
    );
    let [l, _] = render(d.as_mut(), &x, &[(secs(1.0), param(chorus::DELAY, 40.0))]);
    let wet = &l[secs(0.05)..];
    // The 7 -> 40 ms jump glides with a slew limit (read speed within ±8 %): pitch bends
    // by a few percent, no click (a click is a curvature spike many times the input's).
    assert!(
        max_curvature(wet) < base * 1.35,
        "{} vs {base}",
        max_curvature(wet)
    );
    // Before the jump: only the vibrato (±5 % pitch at 2 Hz).
    assert!(max_curvature(&wet[..secs(0.9)]) < base * 1.15);
    // Vibrato is pitch modulation: the output differs from the input, at the same level.
    assert!((rms(wet) / rms(&x) - 1.0).abs() < 0.05);
    let diff: Vec<f32> = wet
        .iter()
        .zip(&x[secs(0.05)..])
        .map(|(a, b)| a - b)
        .collect();
    assert!(rms(&diff) > 0.1);
}

#[test]
fn chorus_spread_controls_stereo_width() {
    let x = noise(0.5, 1.0);
    let width = |spread: f64| {
        let mut d = device(
            BuiltinDeviceType::Chorus,
            &[(chorus::SPREAD, spread), (chorus::MIX, 100.0)],
        );
        let [l, r] = render(d.as_mut(), &x, &[]);
        let side: Vec<f32> = l.iter().zip(&r).map(|(a, b)| a - b).collect();
        rms(&side) / rms(&l)
    };
    assert_eq!(width(0.0), 0.0, "mono in, no spread: mono out");
    assert!(width(100.0) > 0.2, "{}", width(100.0));
}

#[test]
fn chorus_voices_and_modes_change_the_sound_at_constant_level() {
    let x = noise(0.5, 1.0);
    let mut outs = Vec::new();
    for params in [
        vec![(chorus::VOICES, 1.0)],
        vec![(chorus::VOICES, 4.0)],
        vec![(chorus::MODE, 1.0)],
    ] {
        let mut p = params.clone();
        p.push((chorus::MIX, 100.0));
        let mut d = device(BuiltinDeviceType::Chorus, &p);
        let [l, _] = render(d.as_mut(), &x, &[]);
        let level = rms(&l[secs(0.1)..]) / rms(&x);
        assert!((0.5..1.5).contains(&level), "{params:?}: {level}");
        outs.push(l);
    }
    assert_ne!(outs[0], outs[1]);
    assert_ne!(outs[1], outs[2]);
}

#[test]
fn chorus_high_cut_darkens_the_wet_signal() {
    let x = sine(10_000.0, 0.5, 0.5);
    let level = |cut: f64| {
        let mut d = device(
            BuiltinDeviceType::Chorus,
            &[
                (chorus::HIGH_CUT, cut),
                (chorus::MIX, 100.0),
                (chorus::DEPTH, 0.0),
                (chorus::VOICES, 1.0),
            ],
        );
        rms(&render(d.as_mut(), &x, &[])[0][secs(0.1)..])
    };
    assert!(level(20_000.0) > 0.25);
    assert!(level(1_000.0) < 0.06, "{}", level(1_000.0));
}

// ─── phaser ────────────────────────────────────────────────────────────────────────────

/// With the sweep frozen, `N` first-order allpass stages at break frequency `f` shift a sine
/// at `f·tan(90°/N)` by 180°: with `Mix` 50 % it cancels (the notch).
#[test]
fn phaser_notches_sit_where_the_stages_put_them() {
    for (stages_index, stages) in [(0.0, 2.0f32), (1.0, 4.0)] {
        let notch = 1_000.0 * (PI / (2.0 * stages)).tan();
        let run = |freq: f32| {
            let mut d = device(
                BuiltinDeviceType::Phaser,
                &[
                    (phaser::STAGES, stages_index),
                    (phaser::DEPTH, 0.0),
                    (phaser::CENTER, 1_000.0),
                    (phaser::FEEDBACK, 0.0),
                ],
            );
            let x = sine(freq, 0.5, 0.5);
            rms(&render(d.as_mut(), &x, &[])[0][secs(0.2)..])
        };
        assert!(run(notch) < 0.005, "{stages} stages: {}", run(notch));
        assert!(run(notch * 0.25) > 0.2);
    }
}

#[test]
fn phaser_sweeps_and_stereo_phase_decorrelates() {
    let x = noise(0.5, 2.0);
    let mut d = device(
        BuiltinDeviceType::Phaser,
        &[(phaser::RATE, 2.0), (phaser::STEREO_PHASE, 180.0)],
    );
    let [l, r] = render(d.as_mut(), &x, &[]);
    let side: Vec<f32> = l.iter().zip(&r).map(|(a, b)| a - b).collect();
    assert!(rms(&side) > 0.05);
    let mut d = device(BuiltinDeviceType::Phaser, &[(phaser::STEREO_PHASE, 0.0)]);
    let [l, r] = render(d.as_mut(), &x, &[]);
    assert_eq!(l, r);
}

// ─── flanger ───────────────────────────────────────────────────────────────────────────

/// A 1 ms delay mixed 50/50 with the dry signal cancels a 500 Hz sine (comb notch).
#[test]
fn flanger_is_a_comb_filter() {
    let x = sine(500.0, 0.5, 0.5);
    let mut d = device(
        BuiltinDeviceType::Flanger,
        &[
            (flanger::DEPTH, 0.0),
            (flanger::DELAY, 1.0),
            (flanger::FEEDBACK, 0.0),
        ],
    );
    let [l, _] = render(d.as_mut(), &x, &[]);
    assert!(rms(&l[secs(0.1)..]) < 0.002, "{}", rms(&l[secs(0.1)..]));
}

/// Through-zero delays the dry path by 10 ms and reports it as latency; with no sweep the
/// output is exactly the delayed input.
#[test]
fn flanger_through_zero_reports_latency_and_aligns() {
    let d = device(BuiltinDeviceType::Flanger, &[]);
    assert_eq!(d.latency(), 0);
    let mut d = device(
        BuiltinDeviceType::Flanger,
        &[
            (flanger::THROUGH_ZERO, 1.0),
            (flanger::DEPTH, 0.0),
            (flanger::FEEDBACK, 0.0),
        ],
    );
    let lat = d.latency() as usize;
    assert_eq!(lat, 480);
    let x = noise(0.5, 0.5);
    let [l, _] = render(d.as_mut(), &x, &[]);
    for i in lat..x.len() {
        assert!((l[i] - x[i - lat]).abs() < 1e-5, "{i}");
    }
    // Sweeping through zero: deep cancellation moments on a sine, full level elsewhere.
    let mut d = device(
        BuiltinDeviceType::Flanger,
        &[
            (flanger::THROUGH_ZERO, 1.0),
            (flanger::FEEDBACK, 0.0),
            (flanger::RATE, 1.0),
        ],
    );
    let x = sine(2_000.0, 0.5, 2.0);
    let [l, _] = render(d.as_mut(), &x, &[]);
    let windows: Vec<f32> = l[secs(0.1)..].chunks(480).map(rms).collect();
    let (lo, hi) = windows
        .iter()
        .fold((f32::MAX, 0.0f32), |(a, b), v| (a.min(*v), b.max(*v)));
    assert!(lo < 0.05 && hi > 0.3, "{lo} {hi}");
}

// ─── tremolo ───────────────────────────────────────────────────────────────────────────

#[test]
fn tremolo_depth_and_stereo_phase() {
    let x = vec![1.0f32; secs(1.0)];
    let mut d = device(
        BuiltinDeviceType::Tremolo,
        &[(tremolo::DEPTH, 100.0), (tremolo::STEREO_PHASE, 180.0)],
    );
    let [l, r] = render(d.as_mut(), &x, &[]);
    let l = &l[secs(0.05)..];
    let r = &r[secs(0.05)..];
    let (min, max) = l
        .iter()
        .fold((1.0f32, 0.0f32), |(a, b), v| (a.min(*v), b.max(*v)));
    assert!(min < 0.01 && max > 0.99, "{min} {max}");
    // 180°: one side up while the other is down (sine: they sum to 1).
    for (a, b) in l.iter().zip(r) {
        assert!((a + b - 1.0).abs() < 0.02, "{a} {b}");
    }
}

#[test]
fn square_and_saw_edges_do_not_click() {
    let x = vec![1.0f32; secs(1.0)];
    for shape in [2.0, 3.0, 4.0] {
        let mut d = device(
            BuiltinDeviceType::Tremolo,
            &[
                (tremolo::SHAPE, shape),
                (tremolo::DEPTH, 100.0),
                (tremolo::RATE, 10.0),
            ],
        );
        let [l, _] = render(d.as_mut(), &x, &[]);
        let jump = l
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max);
        assert!(jump < 0.03, "shape {shape}: {jump}");
        let (min, max) = l[secs(0.1)..]
            .iter()
            .fold((1.0f32, 0.0f32), |(a, b), v| (a.min(*v), b.max(*v)));
        assert!(min < 0.05 && max > 0.95, "shape {shape}: {min} {max}");
    }
}

/// Auto-pan: full left and full right swings, never louder than the input on either side.
#[test]
fn auto_pan_moves_between_the_sides_without_boost() {
    let x = vec![1.0f32; secs(1.0)];
    let mut d = device(
        BuiltinDeviceType::Tremolo,
        &[
            (tremolo::MODE, 1.0),
            (tremolo::DEPTH, 100.0),
            (tremolo::RATE, 2.0),
        ],
    );
    let [l, r] = render(d.as_mut(), &x, &[]);
    assert!(peak(&l) <= 1.0 && peak(&r) <= 1.0);
    let min_l = l.iter().copied().fold(1.0, f32::min);
    let min_r = r.iter().copied().fold(1.0, f32::min);
    assert!(min_l < 0.01 && min_r < 0.01, "{min_l} {min_r}");
    // Centre: both at unity.
    let centre = l
        .iter()
        .zip(&r)
        .position(|(a, b)| (a - b).abs() < 1e-3)
        .unwrap();
    assert!(l[centre] > 0.99);
}

/// Synced to 1/4 at 120 BPM while playing from beat 0.4: the tremolo's minimum (sine
/// phase 0.75) lands on beat n + 0.75 of the song, after the LFO has locked on.
#[test]
fn synced_tremolo_follows_the_song_position() {
    for bpm in [120.0, 93.0] {
        let x = vec![1.0f32; secs(3.0)];
        let mut d = device(
            BuiltinDeviceType::Tremolo,
            &[
                (tremolo::SYNC, 1.0),
                (tremolo::SYNC_RATE, 8.0),
                (tremolo::DEPTH, 100.0),
            ],
        );
        let play = Play {
            playing: true,
            bpm,
            start: 0.4,
        };
        let [l, _] = render_with(d.as_mut(), [&x, &x], &[], BLOCK, play);
        let spb = 60.0 / bpm * f64::from(SR);
        // Look at the last whole beat.
        let beat = ((l.len() as f64 / spb) + 0.4).floor() - 1.0;
        let from = ((beat - 0.4) * spb) as usize;
        let to = from + spb as usize;
        let argmin = (from..to)
            .min_by(|&a, &b| l[a].partial_cmp(&l[b]).unwrap())
            .unwrap();
        let phase = (argmin as f64 / spb + 0.4).rem_euclid(1.0);
        // The 1 ms edge smoothing delays the minimum by ~1 ms.
        assert!((phase - 0.75).abs() < 0.01, "{bpm} BPM: {phase}");
    }
}

// ─── CPU cost (run with `--release -- --ignored --nocapture cpu_cost`) ───────────────────

#[test]
#[ignore]
fn cpu_cost() {
    let x = noise(0.5, 10.0);
    for (ty, params) in [
        (BuiltinDeviceType::Chorus, vec![(chorus::VOICES, 2.0)]),
        (BuiltinDeviceType::Chorus, vec![(chorus::VOICES, 4.0)]),
        (BuiltinDeviceType::Chorus, vec![(chorus::MODE, 1.0)]),
        (BuiltinDeviceType::Phaser, vec![(phaser::STAGES, 1.0)]),
        (BuiltinDeviceType::Phaser, vec![(phaser::STAGES, 4.0)]),
        (BuiltinDeviceType::Flanger, vec![]),
        (
            BuiltinDeviceType::Flanger,
            vec![(flanger::THROUGH_ZERO, 1.0)],
        ),
        (BuiltinDeviceType::Tremolo, vec![]),
    ] {
        let mut d = device(ty, &params);
        let t = std::time::Instant::now();
        let [l, _] = render(d.as_mut(), &x, &[]);
        let el = t.elapsed().as_secs_f64();
        std::hint::black_box(l);
        println!(
            "{ty:?} {params:?}: {:.1} ns/stereo frame, {:.3} % of one core at 48 kHz",
            el * 1e9 / x.len() as f64,
            el / 10.0 * 100.0
        );
    }
}

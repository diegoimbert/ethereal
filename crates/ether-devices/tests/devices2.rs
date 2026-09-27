//! Offline render tests for the roadmap v2 effects (`devices-2`): EQ, reverb, limiter,
//! utility. Every `process` call runs under `assert_no_alloc` (debug builds abort on
//! allocation on the audio path).

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventBuffer, EventKind, Node, PrepareConfig, ProcessContext,
    ProcessEvent, TransportInfo,
};
use ether_devices::{Eq, Limiter, NoSamples, Reverb, Utility, eq, limiter, reverb, utility};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;

fn prepared<D: Device + ?Sized>(d: &mut D) {
    d.prepare(&PrepareConfig {
        sample_rate: SR,
        max_block_size: BLOCK,
        max_events_per_block: 64,
    });
}

/// Render stereo `input` through `d` in blocks. `events` are `(absolute frame, kind)`,
/// sorted.
fn render<D: Device + ?Sized>(
    d: &mut D,
    input: &[Vec<f32>; 2],
    events: &[(usize, EventKind)],
) -> [Vec<f32>; 2] {
    let frames = input[0].len();
    let mut out = [vec![0.0f32; frames], vec![0.0f32; frames]];
    let mut out_events = EventBuffer::with_capacity(64);
    let mut block_events: Vec<ProcessEvent> = Vec::with_capacity(64);
    let transport = TransportInfo {
        playing: true,
        ..TransportInfo::STOPPED
    };
    let mut pos = 0;
    while pos < frames {
        let n = BLOCK.min(frames - pos);
        block_events.clear();
        for (at, kind) in events {
            if *at >= pos && *at < pos + n {
                block_events.push(ProcessEvent {
                    offset: (*at - pos) as u32,
                    kind: *kind,
                });
            }
        }
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

fn stereo(x: Vec<f32>) -> [Vec<f32>; 2] {
    [x.clone(), x]
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| (v * v) as f64).sum::<f64>() / x.len().max(1) as f64).sqrt() as f32
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-12).log10()
}

fn sine(freq: f32, amp: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| amp * (std::f32::consts::TAU * freq * i as f32 / SR).sin())
        .collect()
}

/// Deterministic white noise in -amp..amp.
fn noise(amp: f32, frames: usize, seed: u32) -> Vec<f32> {
    let mut s = seed.max(1);
    (0..frames)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            amp * (s as f32 / u32::MAX as f32 * 2.0 - 1.0)
        })
        .collect()
}

fn param(id: ParamId, value: f64) -> EventKind {
    EventKind::Param { param: id, value }
}

// ---------------------------------------------------------------------------------------
// Shared

#[test]
fn new_builtins_are_real_devices() {
    for (ty, count) in [
        (BuiltinDeviceType::Eq, eq::BANDS * 5 + 1),
        (BuiltinDeviceType::Reverb, 6),
        (BuiltinDeviceType::Limiter, 3),
        (BuiltinDeviceType::Utility, 6),
    ] {
        let desc = ether_devices::descriptor(ty);
        assert_eq!(desc.params.len(), count, "{ty:?}");
        assert_eq!((desc.audio_inputs, desc.audio_outputs), (2, 2));
        assert_eq!(desc.sidechain_inputs, 0);
        let mut d = ether_devices::create(&BuiltinDevice::new(ty), &NoSamples);
        prepared(&mut *d);
        let expected_latency = if ty == BuiltinDeviceType::Limiter {
            240
        } else {
            0
        };
        assert_eq!(d.latency(), expected_latency, "{ty:?}");
        assert_eq!(d.sidechain_inputs(), 0);
    }
}

/// Sweep every param of every new device to random values mid-stream (sample-accurate
/// events), on noise: never allocates, output stays finite and bounded.
#[test]
fn random_automation_is_rt_safe_and_bounded() {
    for ty in [
        BuiltinDeviceType::Eq,
        BuiltinDeviceType::Reverb,
        BuiltinDeviceType::Limiter,
        BuiltinDeviceType::Utility,
    ] {
        let desc = ether_devices::descriptor(ty);
        let mut d = ether_devices::create(&BuiltinDevice::new(ty), &NoSamples);
        prepared(&mut *d);
        let frames = 96_000;
        let x = noise(0.5, frames, 7);
        let r = noise(1.0, 400, 99);
        let mut events = Vec::new();
        for (k, rk) in r.iter().enumerate().take(200) {
            let p = &desc.params[k % desc.params.len()];
            let n = (rk + 1.0) as f64 * 0.5;
            events.push((k * 450 + 3, param(p.id, p.to_plain(n))));
        }
        let out = render(&mut *d, &stereo(x), &events);
        for ch in &out {
            assert!(
                ch.iter().all(|v| v.is_finite() && v.abs() < 64.0),
                "{ty:?}: {}",
                peak(ch)
            );
        }
        // Every param reads back inside its range.
        for p in &desc.params {
            let v = d.param(p.id).unwrap();
            assert!((p.min..=p.max).contains(&v), "{ty:?} {}", p.name);
        }
    }
}

// ---------------------------------------------------------------------------------------
// EQ

/// Steady-state gain (dB) of `e` at `freq`, measured with a sine.
fn eq_gain_db(e: &mut Eq, freq: f32) -> f32 {
    e.reset();
    let frames = 24_000;
    let x = sine(freq, 0.25, frames);
    let out = render(e, &stereo(x.clone()), &[]);
    let tail = frames / 2;
    db(rms(&out[0][tail..]) / rms(&x[tail..]))
}

fn eq_with(setup: impl Fn(&mut Eq)) -> Eq {
    let mut e = Eq::new();
    prepared(&mut e);
    setup(&mut e);
    e
}

/// Turn off every band, then configure band `b`.
fn solo_band(e: &mut Eq, b: usize, ty: f64, freq: f64, gain: f64, q: f64) {
    for band in 0..eq::BANDS {
        e.set_param(eq::params::band(band, eq::params::ON), 0.0);
    }
    e.set_param(eq::params::band(b, eq::params::ON), 1.0);
    e.set_param(eq::params::band(b, eq::params::TYPE), ty);
    e.set_param(eq::params::band(b, eq::params::FREQ), freq);
    e.set_param(eq::params::band(b, eq::params::GAIN), gain);
    e.set_param(eq::params::band(b, eq::params::Q), q);
}

#[test]
fn eq_default_is_transparent() {
    let mut e = eq_with(|_| {});
    let x = noise(0.5, 9600, 3);
    let out = render(&mut e, &stereo(x.clone()), &[]);
    for (a, b) in out[0].iter().zip(&x) {
        assert!((a - b).abs() < 1e-6);
    }
    assert!((e.magnitude(1000.0) - 1.0).abs() < 1e-9);
}

#[test]
fn eq_bell_response() {
    let mut e = eq_with(|e| solo_band(e, 3, 2.0, 1000.0, 12.0, 1.0));
    let at = |e: &mut Eq, f: f32| eq_gain_db(e, f);
    assert!((at(&mut e, 1000.0) - 12.0).abs() < 0.2);
    assert!(at(&mut e, 50.0).abs() < 0.3);
    assert!(at(&mut e, 15_000.0).abs() < 0.5);
    // Rendered response matches the analytic curve at a few points.
    for f in [200.0, 700.0, 1400.0, 4000.0] {
        let want = db(e.magnitude(f as f64) as f32);
        let got = at(&mut e, f);
        assert!((got - want).abs() < 0.2, "{f} Hz: {got} vs {want}");
    }
    // Cut.
    let mut e = eq_with(|e| solo_band(e, 3, 2.0, 1000.0, -18.0, 2.0));
    assert!((at(&mut e, 1000.0) + 18.0).abs() < 0.3);
}

#[test]
fn eq_cuts_and_shelves() {
    let q = std::f64::consts::FRAC_1_SQRT_2;
    // Low cut 1 kHz, 12 dB/oct: -3 dB at the corner, ~-40 dB at 100 Hz.
    let mut e = eq_with(|e| solo_band(e, 0, 0.0, 1000.0, 0.0, q));
    assert!((eq_gain_db(&mut e, 1000.0) + 3.01).abs() < 0.2);
    assert!((eq_gain_db(&mut e, 100.0) + 40.0).abs() < 1.0);
    assert!(eq_gain_db(&mut e, 12_000.0).abs() < 0.2);
    // High cut 1 kHz.
    let mut e = eq_with(|e| solo_band(e, 7, 5.0, 1000.0, 0.0, q));
    assert!((eq_gain_db(&mut e, 1000.0) + 3.01).abs() < 0.2);
    assert!(eq_gain_db(&mut e, 10_000.0) < -38.0);
    assert!(eq_gain_db(&mut e, 60.0).abs() < 0.2);
    // Low shelf +6 dB at 200 Hz.
    let mut e = eq_with(|e| solo_band(e, 1, 1.0, 200.0, 6.0, q));
    assert!((eq_gain_db(&mut e, 30.0) - 6.0).abs() < 0.3);
    assert!(eq_gain_db(&mut e, 8000.0).abs() < 0.2);
    // High shelf -9 dB at 3 kHz.
    let mut e = eq_with(|e| solo_band(e, 6, 4.0, 3000.0, -9.0, q));
    assert!((eq_gain_db(&mut e, 16_000.0) + 9.0).abs() < 0.4);
    assert!(eq_gain_db(&mut e, 100.0).abs() < 0.2);
    // Notch.
    let mut e = eq_with(|e| solo_band(e, 4, 3.0, 2000.0, 0.0, 4.0));
    assert!(eq_gain_db(&mut e, 2000.0) < -40.0);
    // Bands combine in series; output gain applies on top.
    let mut e = eq_with(|e| {
        solo_band(e, 3, 2.0, 1000.0, 6.0, 1.0);
        e.set_param(eq::params::OUTPUT, -6.0);
    });
    assert!(eq_gain_db(&mut e, 1000.0).abs() < 0.2);
    assert!((eq_gain_db(&mut e, 40.0) + 6.0).abs() < 0.2);
}

/// Largest sample-to-sample step of `x`.
fn max_step(x: &[f32]) -> f32 {
    x.windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

/// Largest second difference of `x`: ~`A·ω²` for a smooth sine, but a discontinuity
/// (click, zipper step) shows up at the size of the jump itself.
fn max_curvature(x: &[f32]) -> f32 {
    x.windows(3)
        .map(|w| (w[2] - 2.0 * w[1] + w[0]).abs())
        .fold(0.0, f32::max)
}

#[test]
fn eq_param_changes_do_not_click() {
    // A low sine through a bell at the same frequency: gain jumps +6 → +24 dB, the band
    // toggles, changes type and sweeps 4 octaves.
    let mut e = eq_with(|e| solo_band(e, 2, 2.0, 100.0, 6.0, 1.0));
    let frames = 48_000;
    let x = sine(100.0, 0.05, frames);
    let g = eq::params::band(2, eq::params::GAIN);
    let on = eq::params::band(2, eq::params::ON);
    let ty = eq::params::band(2, eq::params::TYPE);
    let events = [
        (6000 + 60, param(g, 24.0)),
        (18_000, param(on, 0.0)),
        (24_000, param(on, 1.0)),
        (30_000, param(ty, 1.0)),
        (36_000, param(eq::params::band(2, eq::params::FREQ), 2000.0)),
    ];
    let out = render(&mut e, &stereo(x.clone()), &events);
    // Smooth 100 Hz sine at +24 dB (amp ~0.8): second difference ≈ A·ω² ≈ 1.4e-4.
    let w = std::f32::consts::TAU * 100.0 / SR;
    let smooth = 0.05 * 10f32.powf(24.0 / 20.0) * w * w;
    // (Sweeping a +24 dB shelf 4 octaves overshoots the level a little, hence 3x.)
    let worst = max_curvature(&out[0]);
    assert!(worst < smooth * 3.0, "{worst} vs {smooth}");
    // The change did land.
    assert!(peak(&out[0][12_000..18_000]) > 0.7);

    // Control: the same gain jump applied without smoothing is a click the metric sees.
    let mut e = eq_with(|e| solo_band(e, 2, 2.0, 100.0, 6.0, 1.0));
    let first = render(&mut e, &[x[..6060].to_vec(), x[..6060].to_vec()], &[]);
    e.set_param(g, 24.0);
    let second = render(&mut e, &[x[6060..].to_vec(), x[6060..].to_vec()], &[]);
    let joined: Vec<f32> = first[0].iter().chain(&second[0]).copied().collect();
    assert!(
        max_curvature(&joined) > smooth * 20.0,
        "{} {:?}",
        max_curvature(&joined),
        &joined[6055..6065]
    );
}

// ---------------------------------------------------------------------------------------
// Limiter

fn limiter_with(setup: impl Fn(&mut Limiter)) -> Limiter {
    let mut l = Limiter::new();
    prepared(&mut l);
    setup(&mut l);
    l
}

#[test]
fn limiter_reports_lookahead_latency_and_is_transparent_below_ceiling() {
    let mut l = limiter_with(|_| {});
    let lat = l.latency() as usize;
    assert_eq!(lat, limiter::lookahead_samples(SR) as usize);
    assert_eq!(lat, 240, "5 ms at 48 kHz");
    let x = noise(0.5, 9600, 11);
    let out = render(&mut l, &stereo(x.clone()), &[]);
    assert!(out[0][..lat].iter().all(|v| *v == 0.0));
    for i in lat..x.len() {
        assert_eq!(
            out[0][i],
            x[i - lat],
            "delayed exactly by the reported latency"
        );
    }
    assert_eq!(l.gain_reduction(), 0.0);
}

#[test]
fn limiter_never_exceeds_ceiling() {
    for (ceiling, gain) in [(-0.3, 0.0), (-1.0, 12.0), (-12.0, 24.0), (0.0, 18.0)] {
        let mut l = limiter_with(|l| {
            l.set_param(limiter::params::CEILING, ceiling);
            l.set_param(limiter::params::GAIN, gain);
            l.set_param(limiter::params::RELEASE, 5.0);
        });
        let ceil = 10f32.powf(ceiling as f32 / 20.0);
        // Noise, sines, isolated spikes and a hard step.
        let frames = 48_000;
        let mut x = noise(1.0, frames, 5);
        for (i, v) in sine(60.0, 1.0, frames).into_iter().enumerate().skip(12_000) {
            x[i] = v;
        }
        x[24_000..30_000].fill(0.0);
        x[26_000] = 1.0;
        x[27_000] = -1.0;
        x[36_000..].fill(0.9);
        let out = render(&mut l, &[x.clone(), noise(0.8, frames, 9)], &[]);
        for ch in &out {
            assert!(
                peak(ch) <= ceil,
                "ceiling {ceiling} gain {gain}: {}",
                peak(ch)
            );
        }
        // It's a limiter, not a mute: the level sits close to the ceiling on the step.
        assert!(peak(&out[0][40_000..]) > ceil * 0.9);
    }
}

#[test]
fn limiter_ceiling_automation_and_release() {
    let mut l = limiter_with(|l| l.set_param(limiter::params::RELEASE, 50.0));
    let frames = 48_000;
    let mut x = sine(200.0, 1.0, frames);
    // Loud burst, then quiet.
    for v in &mut x[12_000..] {
        *v *= 0.1;
    }
    let events = [(3000, param(limiter::params::CEILING, -6.0))];
    let out = render(&mut l, &stereo(x.clone()), &events);
    assert!(peak(&out[0]) <= 10f32.powf(-0.3 / 20.0));
    assert!(peak(&out[0][6000..12_000]) <= 10f32.powf(-6.0 / 20.0) + 1e-6);
    // Release: 0.5 s after the burst the quiet part is back to unity gain.
    let tail = &out[0][36_000..];
    assert!((peak(tail) - 0.1).abs() < 0.002, "{}", peak(tail));
    assert_eq!(l.gain_reduction(), 0.0);
}

// ---------------------------------------------------------------------------------------
// Reverb

fn reverb_with(setup: impl Fn(&mut Reverb)) -> Reverb {
    let mut r = Reverb::new();
    prepared(&mut r);
    setup(&mut r);
    r
}

fn impulse(frames: usize) -> [Vec<f32>; 2] {
    let mut x = vec![0.0; frames];
    x[0] = 1.0;
    stereo(x)
}

/// RMS (dB) of both channels over `[t0, t1)` seconds.
fn window_db(out: &[Vec<f32>; 2], t0: f32, t1: f32) -> f32 {
    let (a, b) = ((t0 * SR) as usize, (t1 * SR) as usize);
    db((rms(&out[0][a..b]).powi(2) + rms(&out[1][a..b]).powi(2)).sqrt())
}

#[test]
fn reverb_tail_decays_at_the_decay_time() {
    let mut r = reverb_with(|r| {
        r.set_param(reverb::params::MIX, 100.0);
        r.set_param(reverb::params::DECAY, 1.0);
        r.set_param(reverb::params::DAMPING, 0.0);
        r.set_param(reverb::params::PRE_DELAY, 0.0);
    });
    let out = render(&mut r, &impulse(3 * SR as usize), &[]);
    // A dense tail exists and decays monotonically, window after window.
    let levels: Vec<f32> = (0..10)
        .map(|k| window_db(&out, 0.1 + 0.2 * k as f32, 0.3 + 0.2 * k as f32))
        .collect();
    assert!(levels[0] > -60.0, "{levels:?}");
    assert!(levels.windows(2).all(|w| w[1] < w[0]), "{levels:?}");
    // ~60 dB per second (decay 1 s), damping off: slope within 25 %.
    let slope = (levels[0] - levels[5]) / 1.0;
    assert!((slope - 60.0).abs() < 15.0, "{slope} dB/s, {levels:?}");
    // Longer decay, longer tail.
    let mut long = reverb_with(|r| {
        r.set_param(reverb::params::MIX, 100.0);
        r.set_param(reverb::params::DECAY, 4.0);
    });
    let out_long = render(&mut long, &impulse(3 * SR as usize), &[]);
    assert!(window_db(&out_long, 2.0, 2.5) > window_db(&out, 2.0, 2.5) + 30.0);
}

#[test]
fn reverb_tail_ends_in_exact_silence() {
    // Denormal safety: a short tail flushes to exact zeros instead of subnormals.
    let mut r = reverb_with(|r| {
        r.set_param(reverb::params::MIX, 100.0);
        r.set_param(reverb::params::DECAY, 0.2);
    });
    let out = render(&mut r, &impulse(4 * SR as usize), &[]);
    assert!(out[0][3 * SR as usize..].iter().all(|v| *v == 0.0));
}

#[test]
fn reverb_mix_width_and_pre_delay() {
    // Mix 0 = dry.
    let mut r = reverb_with(|r| r.set_param(reverb::params::MIX, 0.0));
    let x = noise(0.5, 4800, 2);
    let out = render(&mut r, &stereo(x.clone()), &[]);
    assert!(out[0].iter().zip(&x).all(|(a, b)| (a - b).abs() < 1e-7));

    // Pre-delay: fully wet output starts no earlier than the pre-delay.
    let mut r = reverb_with(|r| {
        r.set_param(reverb::params::MIX, 100.0);
        r.set_param(reverb::params::PRE_DELAY, 100.0);
    });
    let out = render(&mut r, &impulse(SR as usize), &[]);
    let first = out[0].iter().position(|v| v.abs() > 1e-9).unwrap();
    assert!(first >= 4800, "{first}");

    // Width: 100 % is decorrelated stereo, 0 % is mono.
    let wide = reverb_with(|r| r.set_param(reverb::params::MIX, 100.0));
    let narrow = reverb_with(|r| {
        r.set_param(reverb::params::MIX, 100.0);
        r.set_param(reverb::params::WIDTH, 0.0);
    });
    for (mut r, stereo_out) in [(wide, true), (narrow, false)] {
        let out = render(&mut r, &impulse(SR as usize), &[]);
        let diff: Vec<f32> = out[0].iter().zip(&out[1]).map(|(a, b)| a - b).collect();
        if stereo_out {
            assert!(rms(&diff) > rms(&out[0]) * 0.5);
        } else {
            assert!(rms(&diff) < 1e-6);
        }
    }
}

#[test]
fn reverb_is_clean_at_extremes() {
    for (size, decay, damping, pre) in [
        (0.0, 0.2, 0.0, 0.0),
        (100.0, 20.0, 0.0, 250.0),
        (100.0, 20.0, 100.0, 0.0),
        (0.0, 20.0, 0.0, 0.0),
    ] {
        let mut r = reverb_with(|r| {
            r.set_param(reverb::params::SIZE, size);
            r.set_param(reverb::params::DECAY, decay);
            r.set_param(reverb::params::DAMPING, damping);
            r.set_param(reverb::params::PRE_DELAY, pre);
            r.set_param(reverb::params::MIX, 100.0);
        });
        let frames = 4 * SR as usize;
        let mut x = noise(0.7, frames, 21);
        x[SR as usize..].fill(0.0);
        // Size swept while the tail rings.
        let events = [(2 * SR as usize, param(reverb::params::SIZE, 100.0 - size))];
        let out = render(&mut r, &[x.clone(), noise(0.7, frames, 4)], &events);
        for ch in &out {
            assert!(
                // 1 s of loud noise into a 20 s tail builds up; it must stay finite and
                // bounded, not blow up.
                ch.iter().all(|v| v.is_finite()) && peak(ch) < 16.0,
                "size {size} decay {decay}: {}",
                peak(ch)
            );
        }
        // Energy never grows after the input stops.
        let a = window_db(&out, 1.2, 1.7);
        let b = window_db(&out, 3.4, 3.9);
        assert!(b <= a + 0.5, "size {size} decay {decay}: {a} -> {b}");
    }
}

// ---------------------------------------------------------------------------------------
// Utility

fn utility_with(setup: impl Fn(&mut Utility)) -> Utility {
    let mut u = Utility::new();
    prepared(&mut u);
    setup(&mut u);
    u
}

/// Output of one stereo frame `(l, r)` held long enough for the smoothers to settle.
fn utility_frame(u: &mut Utility, l: f32, r: f32) -> (f32, f32) {
    let out = render(u, &[vec![l; 2048], vec![r; 2048]], &[]);
    (out[0][2047], out[1][2047])
}

fn close(a: (f32, f32), b: (f32, f32)) -> bool {
    (a.0 - b.0).abs() < 1e-5 && (a.1 - b.1).abs() < 1e-5
}

#[test]
fn utility_math() {
    let (l, r) = (0.6, -0.2);
    let mid = (l + r) / 2.0;
    let side = (l - r) / 2.0;
    type Case = (Vec<(ParamId, f64)>, (f32, f32));
    let cases: Vec<Case> = vec![
        (vec![], (l, r)),
        (vec![(utility::params::INVERT_L, 1.0)], (-l, r)),
        (vec![(utility::params::INVERT_R, 1.0)], (l, -r)),
        (vec![(utility::params::WIDTH, 0.0)], (mid, mid)),
        (vec![(utility::params::MONO, 1.0)], (mid, mid)),
        (
            vec![
                (utility::params::MONO, 1.0),
                (utility::params::WIDTH, 200.0),
            ],
            (mid, mid),
        ),
        (
            vec![(utility::params::WIDTH, 200.0)],
            (mid + 2.0 * side, mid - 2.0 * side),
        ),
        (
            vec![(utility::params::WIDTH, 50.0)],
            (mid + 0.5 * side, mid - 0.5 * side),
        ),
        (vec![(utility::params::PAN, 1.0)], (0.0, r)),
        (vec![(utility::params::PAN, -0.5)], (l, r * 0.5)),
        (vec![(utility::params::GAIN, -6.0206)], (l * 0.5, r * 0.5)),
        (
            // Both inverted, then mono: polarity flips the sum.
            vec![
                (utility::params::INVERT_L, 1.0),
                (utility::params::INVERT_R, 1.0),
                (utility::params::MONO, 1.0),
            ],
            (-mid, -mid),
        ),
    ];
    for (params, want) in cases {
        let mut u = utility_with(|u| {
            for (id, v) in &params {
                u.set_param(*id, *v);
            }
        });
        let got = utility_frame(&mut u, l, r);
        assert!(close(got, want), "{params:?}: {got:?} vs {want:?}");
    }
    // Mono sums to zero for anti-phase input (the classic phase check).
    let mut u = utility_with(|u| u.set_param(utility::params::MONO, 1.0));
    assert!(close(utility_frame(&mut u, 0.5, -0.5), (0.0, 0.0)));
}

#[test]
fn utility_toggles_are_smoothed() {
    let mut u = utility_with(|_| {});
    let x = vec![0.5f32; 4800];
    let events = [
        (1000, param(utility::params::INVERT_L, 1.0)),
        (3000, param(utility::params::GAIN, 36.0)),
    ];
    let out = render(&mut u, &[x.clone(), x], &events);
    // Polarity flip ramps over ~20 ms instead of jumping 0.5 → -0.5.
    assert!(max_step(&out[0][..2900]) < 0.01, "{}", max_step(&out[0]));
    assert!((out[0][2900] + 0.5).abs() < 1e-5);
    assert!(out[1][3000..].windows(2).all(|w| w[1] >= w[0]));
}

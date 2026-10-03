//! fx-color devices (Saturator, Bitcrusher, Auto Filter): render tests. Every `process`
//! call runs under `assert_no_alloc` (debug builds abort on allocation on the audio path).
//!
//! CPU cost per instance: `cargo test --release -p ether-devices --test fx_color -- --ignored
//! --nocapture cpu_cost`.

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::model::ParamId;
use ether_core::{
    AudioBuffers, Device, EventBuffer, EventKind, Node, PrepareConfig, ProcessContext,
    ProcessEvent, TransportInfo,
};
use ether_devices::fx_color::{
    AutoFilter, Bitcrusher, Saturator, auto_filter as af, bitcrusher as bc, saturator as sat,
    saturator_device,
};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const SR: f32 = 48_000.0;
const MAX_BLOCK: usize = 512;

fn prepared<D: Device + ?Sized>(d: &mut D) {
    d.prepare(&PrepareConfig {
        sample_rate: SR,
        max_block_size: MAX_BLOCK,
        max_events_per_block: 64,
    });
}

struct Render<'a> {
    block: usize,
    transport: TransportInfo,
    sidechain: Option<&'a [Vec<f32>; 2]>,
}

impl Default for Render<'_> {
    fn default() -> Self {
        Self {
            block: 256,
            transport: TransportInfo {
                playing: true,
                beats_per_sample: 2.0 / f64::from(SR),
                ..TransportInfo::STOPPED
            },
            sidechain: None,
        }
    }
}

impl Render<'_> {
    /// Render stereo `input` through `d`. `events` are `(absolute frame, kind)`, sorted.
    fn run<D: Device + ?Sized>(
        &self,
        d: &mut D,
        input: &[Vec<f32>; 2],
        events: &[(usize, EventKind)],
    ) -> [Vec<f32>; 2] {
        let frames = input[0].len();
        let mut out = [vec![0.0f32; frames], vec![0.0f32; frames]];
        let mut out_events = EventBuffer::with_capacity(64);
        let mut block_events: Vec<ProcessEvent> = Vec::with_capacity(64);
        let mut pos = 0;
        while pos < frames {
            let n = self.block.min(frames - pos);
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
                position: self.transport.position + pos as f64 * self.transport.beats_per_sample,
                sample_time: pos as u64,
                ..self.transport
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
            match self.sidechain {
                Some(sc) => {
                    let key: [&[f32]; 2] = [&sc[0][pos..pos + n], &sc[1][pos..pos + n]];
                    assert_no_alloc(|| {
                        d.process_sidechain(&mut ctx, &mut audio, &key);
                    });
                }
                None => assert_no_alloc(|| {
                    d.process(&mut ctx, &mut audio);
                }),
            }
            pos += n;
        }
        out
    }
}

fn render<D: Device + ?Sized>(
    d: &mut D,
    input: &[Vec<f32>; 2],
    events: &[(usize, EventKind)],
) -> [Vec<f32>; 2] {
    Render::default().run(d, input, events)
}

fn sine(freq: f32, amp: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| amp * (2.0 * std::f32::consts::PI * freq * i as f32 / SR).sin())
        .collect()
}

fn stereo(x: Vec<f32>) -> [Vec<f32>; 2] {
    [x.clone(), x]
}

fn noise(frames: usize, amp: f32) -> Vec<f32> {
    let mut s = 0x1234_5678u32;
    (0..frames)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            amp * ((s >> 8) as f32 / (1u32 << 23) as f32 - 1.0)
        })
        .collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-12).log10()
}

fn set(d: &mut dyn Device, id: ParamId, v: f64) {
    d.set_param(id, v);
}

fn param(id: ParamId, value: f64) -> EventKind {
    EventKind::Param { param: id, value }
}

fn finite_and_bounded(out: &[Vec<f32>; 2], bound: f32, what: &str) {
    for ch in out {
        for (i, v) in ch.iter().enumerate() {
            assert!(v.is_finite(), "{what}: sample {i} = {v}");
            assert!(v.abs() <= bound, "{what}: sample {i} = {v} > {bound}");
        }
    }
}

/// Every param at min, max, default and NaN, at sample-accurate offsets: finite output and
/// no allocation.
fn extremes(d: &mut dyn Device, bound: f32, what: &str) {
    prepared(d);
    let desc = d.descriptor();
    let input = stereo(noise(48_000, 0.9));
    let mut events = Vec::new();
    let mut at = 7;
    for p in &desc.params {
        for v in [p.min, p.max, f64::NAN, p.default] {
            events.push((at, param(p.id, v)));
            at += 311;
        }
    }
    for p in &desc.params {
        events.push((at, param(p.id, p.max)));
    }
    let out = render(d, &input, &events);
    finite_and_bounded(&out, bound, what);
    for p in &desc.params {
        assert_eq!(d.param(p.id), Some(p.max), "{what}: {}", p.name);
    }
    // Silence after all that decays to exact zeros or tiny values (no denormal build-up).
    d.reset();
    let out = render(d, &stereo(vec![0.0; 4096]), &[]);
    finite_and_bounded(&out, bound, what);
}

// ─── Saturator ─────────────────────────────────────────────────────────────────────────

#[test]
fn saturator_extremes_are_finite_without_allocating() {
    for curve in 0..6 {
        for os in 0..3 {
            let mut d = Saturator::new();
            set(&mut d, sat::CURVE, f64::from(curve));
            set(&mut d, sat::OVERSAMPLING, f64::from(os));
            extremes(&mut d, 64.0, &format!("saturator curve {curve} os {os}"));
        }
    }
}

#[test]
fn saturator_latency_is_constant_and_the_dry_path_is_aligned() {
    assert_eq!(saturator_device::LATENCY, 29);
    for os in 0..3 {
        let mut d = Saturator::new();
        set(&mut d, sat::CURVE, 1.0); // Hard: linear below full scale.
        set(&mut d, sat::DRIVE, 0.0);
        set(&mut d, sat::AUTO_GAIN, 0.0);
        set(&mut d, sat::OVERSAMPLING, f64::from(os));
        prepared(&mut d);
        assert_eq!(d.latency(), 29);
        let x = sine(2000.0, 0.5, 9600);
        let out = render(&mut d, &stereo(x.clone()), &[]);
        // Past the DC blocker's settling, the output is the input delayed by the latency.
        let err = (4800..9600)
            .map(|i| (out[0][i] - x[i - 29]).abs())
            .fold(0.0f32, f32::max);
        assert!(err < 0.01, "os {os}: {err}");
        // Mix 50 %: no comb filtering (dry and wet aligned).
        let mut d = Saturator::new();
        set(&mut d, sat::CURVE, 1.0);
        set(&mut d, sat::DRIVE, 0.0);
        set(&mut d, sat::AUTO_GAIN, 0.0);
        set(&mut d, sat::OVERSAMPLING, f64::from(os));
        set(&mut d, sat::MIX, 50.0);
        prepared(&mut d);
        let hi = sine(15_000.0, 0.5, 9600);
        let out = render(&mut d, &stereo(hi.clone()), &[]);
        let r = db(rms(&out[0][4800..]) / rms(&hi[4800..]));
        assert!(r.abs() < 0.5, "os {os}: 15 kHz at 50 % mix = {r} dB");
    }
}

#[test]
fn saturator_is_dc_safe() {
    for curve in 0..6 {
        let mut d = Saturator::new();
        set(&mut d, sat::CURVE, f64::from(curve));
        set(&mut d, sat::BIAS, 100.0);
        set(&mut d, sat::DRIVE, 24.0);
        prepared(&mut d);
        let out = render(&mut d, &stereo(sine(220.0, 0.8, 96_000)), &[]);
        let tail = &out[0][48_000..];
        let mean = tail.iter().sum::<f32>() / tail.len() as f32;
        assert!(mean.abs() < 1e-3, "curve {curve}: DC {mean}");
        // Silence in, silence out (the bias offset is removed exactly).
        let out = render(&mut d, &stereo(vec![0.0; 48_000]), &[]);
        assert!(
            out[0][24_000..].iter().all(|v| v.abs() < 1e-4),
            "curve {curve}: silence"
        );
    }
}

#[test]
fn saturator_auto_gain_keeps_the_level() {
    for curve in [0.0, 2.0, 3.0] {
        let mut d = Saturator::new();
        set(&mut d, sat::CURVE, curve);
        set(&mut d, sat::DRIVE, 24.0);
        prepared(&mut d);
        let x = sine(200.0, 0.25, 48_000);
        let out = render(&mut d, &stereo(x.clone()), &[]);
        let r = db(rms(&out[0][24_000..]) / rms(&x[24_000..]));
        assert!(r.abs() < 1.0, "curve {curve}: auto gain off by {r} dB");
    }
}

/// Aliasing floor: the loudest spectral line below 20 kHz that is not a harmonic of the
/// input (a harmonic above Nyquist folded back), relative to the fundamental, for a 5 kHz
/// sine at -6 dBFS through `curve` with `drive_db` of drive. Printed by the test below as
/// the documented aliasing floor of each oversampling mode.
fn aliasing_floor_db(os: usize, curve: f64, drive_db: f64) -> f32 {
    let mut d = Saturator::new();
    set(&mut d, sat::CURVE, curve);
    set(&mut d, sat::DRIVE, drive_db);
    set(&mut d, sat::OVERSAMPLING, os as f64);
    set(&mut d, sat::AUTO_GAIN, 0.0);
    prepared(&mut d);
    // N = 4800 at 48 kHz: 10 Hz bins, 5 kHz = bin 500. Harmonics are multiples of 500
    // bins; their aliases (|n·5000 - k·48000| Hz) land on multiples of 100 bins.
    let n = 4800;
    let out = render(&mut d, &stereo(sine(5000.0, 0.5, n * 3)), &[]);
    let y = &out[0][n * 2..];
    // Exact-bin DFT (integer periods in the window, no leakage).
    let power = |k: usize| {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, v) in y.iter().enumerate() {
            let a = 2.0 * std::f64::consts::PI * ((k * i) % n) as f64 / n as f64;
            re += f64::from(*v) * a.cos();
            im -= f64::from(*v) * a.sin();
        }
        re * re + im * im
    };
    let fund = power(500);
    let worst = (100..2000)
        .step_by(100)
        .filter(|k| k % 500 != 0)
        .map(power)
        .fold(0.0f64, f64::max);
    (10.0 * (worst / fund).max(1e-30).log10()) as f32
}

#[test]
fn saturator_oversampling_lowers_the_aliasing_floor() {
    let mut report = Vec::new();
    for (name, curve, drive) in [("soft +12 dB", 0.0, 12.0), ("hard +12 dB", 1.0, 12.0)] {
        let f: Vec<f32> = (0..3)
            .map(|os| aliasing_floor_db(os, curve, drive))
            .collect();
        println!(
            "aliasing floor ({name}, 5 kHz at -6 dBFS, worst alias below 20 kHz): \
             off {:.1} dB, 2x {:.1} dB, 4x {:.1} dB",
            f[0], f[1], f[2]
        );
        report.push(f);
    }
    // Measured: soft off -41, 2x/4x -99 (numerical floor); hard off -32, 2x -47, 4x -58.
    for f in &report {
        assert!(f[1] < f[0] - 12.0, "{f:?}");
        assert!(f[2] <= f[1] + 1.0, "{f:?}");
    }
    assert!(report[0][1] < -80.0, "{:?}", report[0]);
    assert!(report[1][2] < report[1][1] - 6.0, "{:?}", report[1]);
    assert!(report[1][2] < -50.0, "{:?}", report[1]);
}

#[test]
fn saturator_mode_changes_are_click_free() {
    let mut d = Saturator::new();
    set(&mut d, sat::DRIVE, 12.0);
    prepared(&mut d);
    let x = sine(100.0, 0.5, 48_000);
    let events = [
        (10_007, param(sat::OVERSAMPLING, 2.0)),
        (20_011, param(sat::OVERSAMPLING, 0.0)),
        (30_013, param(sat::OVERSAMPLING, 1.0)),
    ];
    let out = render(&mut d, &stereo(x), &events);
    // A 100 Hz tone moves at most ~0.07 per sample: no jump bigger than that anywhere.
    let max_step = out[0][1000..]
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max);
    assert!(max_step < 0.1, "{max_step}");
}

// ─── Bitcrusher ────────────────────────────────────────────────────────────────────────

#[test]
fn bitcrusher_extremes_are_finite_without_allocating() {
    extremes(&mut Bitcrusher::new(), 64.0, "bitcrusher");
}

#[test]
fn bitcrusher_quantizes_and_holds() {
    let mut d = Bitcrusher::new();
    set(&mut d, bc::BITS, 1.0);
    set(&mut d, bc::RATE, 12_000.0);
    prepared(&mut d);
    let x = sine(300.0, 0.9, 4800);
    let out = render(&mut d, &stereo(x), &[]);
    for v in &out[0] {
        assert!([-1.0, 0.0, 1.0].contains(v), "{v}");
    }
    // 48 k / 12 k: every value is held 4 samples.
    for chunk in out[0].chunks(4) {
        assert!(chunk.iter().all(|v| *v == chunk[0]), "{chunk:?}");
    }
    // 24 bits at full rate is transparent.
    let mut d = Bitcrusher::new();
    set(&mut d, bc::BITS, 24.0);
    set(&mut d, bc::RATE, 48_000.0);
    prepared(&mut d);
    let x = sine(1000.0, 0.7, 4800);
    let out = render(&mut d, &stereo(x.clone()), &[]);
    let err = out[0]
        .iter()
        .zip(&x)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(err < 1e-6, "{err}");
    // 8 bits: the error is at most half a step.
    let mut d = Bitcrusher::new();
    set(&mut d, bc::RATE, 48_000.0);
    prepared(&mut d);
    let out = render(&mut d, &stereo(x.clone()), &[]);
    let err = out[0]
        .iter()
        .zip(&x)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(err <= 0.5 / 128.0 + 1e-6, "{err}");
}

#[test]
fn bitcrusher_jitter_varies_the_hold_and_mix_blends() {
    let mut d = Bitcrusher::new();
    set(&mut d, bc::BITS, 24.0);
    set(&mut d, bc::RATE, 6000.0);
    set(&mut d, bc::JITTER, 100.0);
    prepared(&mut d);
    let out = render(&mut d, &stereo(noise(9600, 0.5)), &[]);
    let mut runs = std::collections::BTreeSet::new();
    let mut len = 1;
    for w in out[0].windows(2) {
        if w[1] == w[0] {
            len += 1;
        } else {
            runs.insert(len);
            len = 1;
        }
    }
    assert!(runs.len() >= 4, "hold lengths {runs:?}");
    assert!(runs.iter().all(|r| (4..=13).contains(r)), "{runs:?}");
    // Mix 0 = dry.
    let mut d = Bitcrusher::new();
    set(&mut d, bc::MIX, 0.0);
    prepared(&mut d);
    let x = noise(4800, 0.5);
    let out = render(&mut d, &stereo(x.clone()), &[]);
    assert_eq!(out[0], x);
}

#[test]
fn bitcrusher_dither_decorrelates_the_error() {
    // A tiny sine under one 4-bit step vanishes without dither and survives with it.
    let x = sine(1000.0, 0.03, 48_000);
    let level = |dither: f64| {
        let mut d = Bitcrusher::new();
        set(&mut d, bc::BITS, 4.0);
        set(&mut d, bc::RATE, 48_000.0);
        set(&mut d, bc::DITHER, dither);
        prepared(&mut d);
        let out = render(&mut d, &stereo(x.clone()), &[]);
        // Correlation with the input sine.
        out[0].iter().zip(&x).map(|(a, b)| a * b).sum::<f32>()
    };
    assert_eq!(level(0.0), 0.0);
    assert!(level(1.0) > 0.0);
}

// ─── Auto Filter ───────────────────────────────────────────────────────────────────────

fn tone_gain_db(d: &mut AutoFilter, freq: f32) -> f32 {
    d.reset();
    let x = sine(freq, 0.1, 24_000);
    let out = render(d, &stereo(x.clone()), &[]);
    db(rms(&out[0][12_000..]) / rms(&x[12_000..]))
}

fn filter(ty: f64, cutoff: f64, res: f64) -> AutoFilter {
    let mut d = AutoFilter::new();
    set(&mut d, af::TYPE, ty);
    set(&mut d, af::CUTOFF, cutoff);
    set(&mut d, af::RESONANCE, res);
    prepared(&mut d);
    d
}

#[test]
fn auto_filter_responses() {
    // Low-pass 12 / 24 at 1 kHz.
    let mut lp12 = filter(0.0, 1000.0, 0.0);
    assert!(tone_gain_db(&mut lp12, 100.0).abs() < 0.5);
    let a12 = tone_gain_db(&mut lp12, 8000.0);
    assert!((-42.0..-30.0).contains(&a12), "LP12 at 3 octaves: {a12}");
    let mut lp24 = filter(1.0, 1000.0, 0.0);
    assert!(tone_gain_db(&mut lp24, 100.0).abs() < 1.0);
    let a24 = tone_gain_db(&mut lp24, 8000.0);
    assert!(a24 < -60.0, "LP24 at 3 octaves: {a24}");
    // High-pass 12 / 24.
    let mut hp12 = filter(2.0, 1000.0, 0.0);
    assert!(tone_gain_db(&mut hp12, 10_000.0).abs() < 0.5);
    assert!(tone_gain_db(&mut hp12, 125.0) < -30.0);
    let mut hp24 = filter(3.0, 1000.0, 0.0);
    assert!(tone_gain_db(&mut hp24, 10_000.0).abs() < 1.0);
    assert!(tone_gain_db(&mut hp24, 125.0) < -60.0);
    // Band-pass: unity at the cutoff, down on both sides.
    let mut bp = filter(4.0, 1000.0, 50.0);
    assert!(tone_gain_db(&mut bp, 1000.0).abs() < 0.5);
    assert!(tone_gain_db(&mut bp, 100.0) < -12.0);
    assert!(tone_gain_db(&mut bp, 10_000.0) < -12.0);
    // Notch: a hole at the cutoff.
    let mut notch = filter(5.0, 1000.0, 20.0);
    assert!(tone_gain_db(&mut notch, 1000.0) < -30.0);
    assert!(tone_gain_db(&mut notch, 100.0).abs() < 0.5);
    // Peak: 6 + 12·res dB bell.
    let mut peak = filter(6.0, 1000.0, 50.0);
    let g = tone_gain_db(&mut peak, 1000.0);
    assert!((g - 12.0).abs() < 0.5, "peak {g}");
    assert!(tone_gain_db(&mut peak, 50.0).abs() < 0.5);
    // Resonance makes a peak at the cutoff.
    let mut res = filter(0.0, 1000.0, 90.0);
    assert!(tone_gain_db(&mut res, 1000.0) > 12.0);
}

#[test]
fn auto_filter_extremes_are_finite_without_allocating() {
    for ty in 0..7 {
        let mut d = AutoFilter::new();
        set(&mut d, af::TYPE, f64::from(ty));
        extremes(&mut d, 64.0, &format!("auto filter type {ty}"));
    }
    // Self-oscillating ladder with full drive and a loud input stays bounded.
    let mut d = filter(1.0, 20_000.0, 100.0);
    set(&mut d, af::DRIVE, 24.0);
    set(&mut d, af::LFO_AMOUNT, 100.0);
    set(&mut d, af::LFO_RATE, 20.0);
    let out = render(&mut d, &stereo(noise(48_000, 1.0)), &[]);
    finite_and_bounded(&out, 16.0, "self-oscillation");
}

#[test]
fn auto_filter_envelope_opens_the_filter() {
    // Low-pass at 100 Hz; a loud 4 kHz tone opens it with Envelope +100 %.
    let x = sine(4000.0, 0.5, 24_000);
    let level = |amount: f64| {
        let mut d = filter(0.0, 100.0, 0.0);
        set(&mut d, af::ENV_AMOUNT, amount);
        prepared(&mut d);
        let out = render(&mut d, &stereo(x.clone()), &[]);
        db(rms(&out[0][12_000..]))
    };
    let closed = level(0.0);
    let open = level(100.0);
    assert!(open > closed + 30.0, "closed {closed}, open {open}");
}

#[test]
fn auto_filter_envelope_keys_from_the_sidechain() {
    // Quiet input, loud key: with the sidechain connected the follower follows the key.
    let x = sine(4000.0, 0.05, 24_000);
    let key = stereo(sine(100.0, 0.9, 24_000));
    let mut d = filter(0.0, 100.0, 0.0);
    set(&mut d, af::ENV_AMOUNT, 100.0);
    prepared(&mut d);
    let keyed = Render {
        sidechain: Some(&key),
        ..Render::default()
    }
    .run(&mut d, &stereo(x.clone()), &[]);
    assert!(d.envelope() > 0.8, "follower {}", d.envelope());
    let mut d = filter(0.0, 100.0, 0.0);
    set(&mut d, af::ENV_AMOUNT, 100.0);
    prepared(&mut d);
    let unkeyed = render(&mut d, &stereo(x), &[]);
    assert!(d.envelope() < 0.06, "follower {}", d.envelope());
    assert!(db(rms(&keyed[0][12_000..])) > db(rms(&unkeyed[0][12_000..])) + 20.0);
}

/// Loud/quiet windows of a 5 kHz tone through a 1 kHz low-pass swept ±4 octaves by a square
/// LFO (loud while the LFO is high).
fn lfo_windows(start_beats: f64, phase: f64) -> [Vec<bool>; 2] {
    let mut d = filter(0.0, 1000.0, 0.0);
    set(&mut d, af::LFO_AMOUNT, 100.0);
    set(&mut d, af::LFO_SHAPE, 4.0);
    set(&mut d, af::LFO_SYNC, 1.0);
    set(&mut d, af::LFO_SYNC_RATE, 8.0); // 1/4 = 0.5 s at 120 bpm = 24000 samples.
    set(&mut d, af::LFO_PHASE, phase);
    prepared(&mut d);
    let mut r = Render::default();
    r.transport.position = start_beats;
    r.transport.bpm = 120.0;
    let out = r.run(&mut d, &stereo(sine(5000.0, 0.5, 48_000)), &[]);
    // Middle 8000 samples of each half cycle.
    let loud = |ch: &[f32], k: usize| {
        let s = k * 12_000 + 2000;
        db(rms(&ch[s..s + 8000])) > -20.0
    };
    [
        (0..4).map(|k| loud(&out[0], k)).collect(),
        (0..4).map(|k| loud(&out[1], k)).collect(),
    ]
}

#[test]
fn auto_filter_lfo_syncs_to_the_song_position() {
    let [l, r] = lfo_windows(0.0, 0.0);
    assert_eq!(l, [true, false, true, false]);
    assert_eq!(r, l);
    // Half a cycle later in the song: the LFO is locked to the position, not the start.
    let [l, _] = lfo_windows(0.5, 0.0);
    assert_eq!(l, [false, true, false, true]);
    // Stereo phase 180°: channels alternate.
    let [l, r] = lfo_windows(0.0, 180.0);
    assert_eq!(l, [true, false, true, false]);
    assert_eq!(r, [false, true, false, true]);
}

/// Block-size independence (CONTRACTS §12.7): the same automation at the same absolute
/// frames renders identically with blocks of 64 and 512.
#[test]
fn renders_are_block_size_independent() {
    let x = stereo(noise(24_000, 0.7));
    type Case = (
        &'static str,
        fn() -> Box<dyn Device>,
        Vec<(usize, EventKind)>,
    );
    let devices: [Case; 3] = [
        (
            "saturator",
            || Box::new(Saturator::new()),
            vec![
                (1000, param(sat::DRIVE, 30.0)),
                (5003, param(sat::CURVE, 4.0)),
                (9001, param(sat::OVERSAMPLING, 2.0)),
                (13_000, param(sat::MIX, 40.0)),
            ],
        ),
        (
            "bitcrusher",
            || Box::new(Bitcrusher::new()),
            vec![
                (999, param(bc::RATE, 3000.0)),
                (4001, param(bc::JITTER, 60.0)),
                (8000, param(bc::DITHER, 1.0)),
                (12_345, param(bc::BITS, 3.5)),
            ],
        ),
        (
            "auto filter",
            || Box::new(AutoFilter::new()),
            vec![
                (100, param(af::LFO_AMOUNT, 70.0)),
                (2000, param(af::ENV_AMOUNT, -50.0)),
                (6000, param(af::TYPE, 1.0)),
                (10_000, param(af::CUTOFF, 300.0)),
                (15_000, param(af::LFO_SYNC, 1.0)),
            ],
        ),
    ];
    for (name, make, events) in devices {
        let mut a = make();
        prepared(a.as_mut());
        let mut b = make();
        prepared(b.as_mut());
        let ra = Render {
            block: 64,
            ..Render::default()
        }
        .run(a.as_mut(), &x, &events);
        let rb = Render {
            block: 512,
            ..Render::default()
        }
        .run(b.as_mut(), &x, &events);
        let err = ra[0]
            .iter()
            .zip(&rb[0])
            .map(|(p, q)| (p - q).abs())
            .fold(0.0f32, f32::max);
        assert!(err <= 1e-6, "{name}: {err}");
    }
}

/// CPU cost per instance (stereo, 48 kHz, 128-sample blocks). Run in release (see the module
/// docs); prints the share of one core that each device takes in real time.
#[test]
#[ignore]
fn cpu_cost() {
    let seconds = 10;
    let x = stereo(noise(48_000 * seconds, 0.5));
    let cases: Vec<(&str, Box<dyn Device>)> = {
        let mut v: Vec<(&str, Box<dyn Device>)> = Vec::new();
        for (name, os) in [
            ("saturator off", 0.0),
            ("saturator 2x", 1.0),
            ("saturator 4x", 2.0),
        ] {
            let mut d = Saturator::new();
            set(&mut d, sat::OVERSAMPLING, os);
            v.push((name, Box::new(d)));
        }
        v.push(("bitcrusher", Box::new(Bitcrusher::new())));
        let mut d = AutoFilter::new();
        set(&mut d, af::LFO_AMOUNT, 50.0);
        set(&mut d, af::ENV_AMOUNT, 50.0);
        v.push(("auto filter svf", Box::new(d)));
        let mut d = AutoFilter::new();
        set(&mut d, af::TYPE, 1.0);
        set(&mut d, af::DRIVE, 12.0);
        set(&mut d, af::LFO_AMOUNT, 50.0);
        v.push(("auto filter ladder+drive", Box::new(d)));
        v
    };
    for (name, mut d) in cases {
        prepared(d.as_mut());
        let t = std::time::Instant::now();
        Render {
            block: 128,
            ..Render::default()
        }
        .run(d.as_mut(), &x, &[]);
        let el = t.elapsed().as_secs_f64();
        println!(
            "{name}: {:.2} % of a core ({:.1} µs per 128-frame block)",
            100.0 * el / seconds as f64,
            el * 1e6 / (48_000.0 * seconds as f64 / 128.0)
        );
    }
}

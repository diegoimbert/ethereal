//! `fx-dynamics` render tests: Gate, Multiband Compressor, Transient Shaper. Every
//! `process*`/`analysis` call runs under `assert_no_alloc` (debug builds abort on allocation
//! on the audio path).

use std::f32::consts::PI;

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, ParamId};
use ether_core::{
    AnalysisFrame, AnalysisKind, AnalysisSink, AudioBuffers, Device, EventBuffer, EventKind, Node,
    PrepareConfig, ProcessContext, ProcessEvent, TransportInfo,
};
use ether_devices::NoSamples;
use ether_devices::fx_dynamics::{
    Gate, MultibandCompressor, TransientShaper, gate, multiband_compressor as mbc,
    transient_shaper as ts,
};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;

fn prepared<D: Device>(mut d: D) -> D {
    d.prepare(&PrepareConfig {
        sample_rate: SR,
        max_block_size: BLOCK,
        max_events_per_block: 64,
    });
    d
}

/// Render mono `main` (on both channels) through `d`, keyed by `sidechain` (both channels)
/// if given. `events` are `(absolute frame, kind)`, sorted. Returns the left output.
fn render<D: Device + ?Sized>(
    d: &mut D,
    main: &[f32],
    sidechain: Option<&[f32]>,
    events: &[(usize, EventKind)],
) -> Vec<f32> {
    let frames = main.len();
    let mut out_l = vec![0.0f32; frames];
    let mut out_r = vec![0.0f32; frames];
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
        let inputs: [&[f32]; 2] = [&main[pos..pos + n], &main[pos..pos + n]];
        let mut outputs: [&mut [f32]; 2] = [&mut out_l[pos..pos + n], &mut out_r[pos..pos + n]];
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
        assert_no_alloc(|| match sidechain {
            Some(sc) => {
                let key: [&[f32]; 2] = [&sc[pos..pos + n], &sc[pos..pos + n]];
                d.process_sidechain(&mut ctx, &mut audio, &key);
            }
            None => {
                d.process(&mut ctx, &mut audio);
            }
        });
        assert_eq!(out_l[pos..pos + n], out_r[pos..pos + n], "linked stereo");
        pos += n;
    }
    out_l
}

/// One analysis pass (no allocation): the `Levels` values.
fn levels(d: &mut dyn Node) -> Vec<f32> {
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

fn sine(freq: f32, amp: f32, secs: f32) -> Vec<f32> {
    (0..(secs * SR) as usize)
        .map(|i| amp * (2.0 * PI * freq * i as f32 / SR).sin())
        .collect()
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-12).log10()
}

fn amp(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn set<D: Device>(mut d: D, params: &[(ParamId, f64)]) -> D {
    for &(id, v) in params {
        d.set_param(id, v);
    }
    d
}

fn param(id: ParamId, value: f64) -> EventKind {
    EventKind::Param { param: id, value }
}

// ─── all devices ───────────────────────────────────────────────────────────────────────

const TYPES: [BuiltinDeviceType; 3] = [
    BuiltinDeviceType::Gate,
    BuiltinDeviceType::MultibandCompressor,
    BuiltinDeviceType::TransientShaper,
];

/// Every param at its min, then max, then NaN (sample-accurate events mid-block), on noise
/// with and without a sidechain: finite output, no allocation, params read back clamped.
#[test]
fn extreme_params_stay_finite_without_allocating() {
    let mut seed = 1u32;
    let noise: Vec<f32> = (0..SR as usize)
        .map(|_| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
        })
        .collect();
    for ty in TYPES {
        let desc = ether_devices::descriptor(ty);
        let mut d = ether_devices::create(&BuiltinDevice::new(ty), &NoSamples);
        d.prepare(&PrepareConfig {
            sample_rate: SR,
            max_block_size: BLOCK,
            max_events_per_block: 64,
        });
        for pick in [0usize, 1, 2] {
            let events: Vec<(usize, EventKind)> = desc
                .params
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let v = [p.min, p.max, f64::NAN][pick];
                    (100 + i, param(p.id, v))
                })
                .collect();
            for sc in [None, Some(&noise[..])] {
                let out = render(d.as_mut(), &noise, sc, &events);
                assert!(out.iter().all(|x| x.is_finite()), "{ty:?} pick {pick}");
                // +24 dB makeup and +24 dB output on full-scale noise: large but bounded.
                assert!(peak(&out) < 1000.0, "{ty:?} pick {pick}: {}", peak(&out));
            }
            for p in &desc.params {
                let v = d.param(p.id).unwrap();
                let expect = [p.min, p.max, p.default][pick];
                assert_eq!(v, expect, "{ty:?} {}", p.name);
            }
            let values = levels(d.as_mut());
            assert!(values.iter().all(|v| v.is_finite()), "{ty:?}");
        }
        // Silence after loud noise decays to exact zero state (no denormal tails).
        d.reset();
        let silent = vec![0.0f32; SR as usize / 2];
        let out = render(d.as_mut(), &silent, None, &[]);
        assert!(out.iter().all(|x| *x == 0.0), "{ty:?}");
    }
}

/// Tiny (subnormal-range) input never produces NaN/inf and keeps silent output silent-ish.
#[test]
fn subnormal_input_is_harmless() {
    let tiny: Vec<f32> = (0..20_000)
        .map(|i| if i % 2 == 0 { 1e-38 } else { -1e-39 })
        .collect();
    for ty in TYPES {
        let mut d = ether_devices::create(&BuiltinDevice::new(ty), &NoSamples);
        d.prepare(&PrepareConfig {
            sample_rate: SR,
            max_block_size: BLOCK,
            max_events_per_block: 64,
        });
        let out = render(d.as_mut(), &tiny, None, &[]);
        assert!(
            out.iter().all(|x| x.is_finite() && x.abs() < 1e-30),
            "{ty:?}"
        );
    }
}

// ─── Gate ──────────────────────────────────────────────────────────────────────────────

#[test]
fn gate_passes_loud_and_silences_quiet_input() {
    let mut g = prepared(Gate::new());
    // Loud (-10 dBFS) passes at unity (threshold -40).
    let loud = sine(440.0, amp(-10.0), 0.5);
    let out = render(&mut g, &loud, None, &[]);
    assert!((db(peak(&out[12_000..])) + 10.0).abs() < 0.05);
    assert_eq!(g.gain_reduction(), 0.0);
    // Quiet (-50 dBFS) is gated to silence after hold (10 ms) + release (100 ms).
    let quiet = sine(440.0, amp(-50.0), 0.5);
    let out = render(&mut g, &quiet, None, &[]);
    assert!(peak(&out[12_000..]) == 0.0, "{}", peak(&out[12_000..]));
    // The meter reports the closed gate at the bottom of its range.
    assert!(levels(&mut g)[0] <= -79.0);
    // Floor -20 dB: attenuated by 20 dB instead.
    g.set_param(gate::RANGE, -20.0);
    let out = render(&mut g, &quiet, None, &[]);
    assert!(
        (db(peak(&out[12_000..])) + 70.0).abs() < 0.1,
        "{}",
        db(peak(&out[12_000..]))
    );
}

#[test]
fn gate_hysteresis_keeps_it_open_between_close_and_open_levels() {
    // Threshold -40, Return 10: opens at -40, closes below -50.
    let mut g = prepared(set(Gate::new(), &[(gate::HYSTERESIS, 10.0)]));
    let mut x = sine(440.0, amp(-30.0), 0.2);
    x.extend(sine(440.0, amp(-45.0), 0.5));
    let out = render(&mut g, &x, None, &[]);
    let tail = &out[out.len() - 4800..];
    assert!(
        (db(peak(tail)) + 45.0).abs() < 0.1,
        "stayed open: {}",
        db(peak(tail))
    );
    // Starting at -45 from closed: stays closed.
    let mut g = prepared(set(Gate::new(), &[(gate::HYSTERESIS, 10.0)]));
    let quiet = sine(440.0, amp(-60.0), 0.3);
    render(&mut g, &quiet, None, &[]);
    let out = render(&mut g, &sine(440.0, amp(-45.0), 0.3), None, &[]);
    assert_eq!(peak(&out[4800..]), 0.0);
}

#[test]
fn gate_hold_and_release_timing() {
    // Hold 50 ms, release 100 ms, floor -80 (silent): after the input drops, the gain stays
    // at unity for ~50 ms, then falls to silence within 100 ms.
    let mut g = prepared(set(
        Gate::new(),
        &[(gate::HOLD, 50.0), (gate::RELEASE, 100.0)],
    ));
    let dc = |a: f32, secs: f32| vec![a; (secs * SR) as usize];
    render(&mut g, &dc(0.5, 0.1), None, &[]);
    let out = render(&mut g, &dc(0.001, 0.3), None, &[]); // -60 dBFS
    let ms = |t: f32| (t * 0.001 * SR) as usize;
    assert!((out[ms(45.0)] - 0.001).abs() < 1e-6, "held");
    assert!(out[ms(80.0)] < 0.001 && out[ms(80.0)] > 0.0, "releasing");
    assert_eq!(out[ms(155.0)], 0.0, "closed");
}

#[test]
fn gate_lookahead_is_latency() {
    let mut g = prepared(set(Gate::new(), &[(gate::LOOKAHEAD, 5.0)]));
    assert_eq!(g.latency(), 240);
    let mut x = vec![0.0f32; 4800];
    x[1000] = 1.0;
    let out = render(&mut g, &x, None, &[]);
    assert_eq!(out[1240], 1.0);
    assert!(out[..1240].iter().all(|v| *v == 0.0));
    // With lookahead the gate opens before the (delayed) onset: a quiet-to-loud step at
    // 1 ms attack is fully open when the step comes out.
    let mut g = prepared(set(
        Gate::new(),
        &[
            (gate::LOOKAHEAD, 5.0),
            (gate::ATTACK, 1.0),
            (gate::HOLD, 0.0),
        ],
    ));
    let mut x = vec![0.0f32; 24_000];
    for v in &mut x[12_000..] {
        *v = 0.5;
    }
    let out = render(&mut g, &x, None, &[]);
    assert_eq!(out[12_240], 0.5);
    g.set_param(gate::LOOKAHEAD, 0.0);
    assert_eq!(g.latency(), 0);
}

#[test]
fn gate_sidechain_keys_the_detector() {
    let main = sine(220.0, amp(-6.0), 0.5);
    // A silent key closes the gate on a loud main signal.
    let mut g = prepared(Gate::new());
    let silent = vec![0.0f32; main.len()];
    let out = render(&mut g, &main, Some(&silent), &[]);
    assert_eq!(peak(&out[12_000..]), 0.0);
    // A loud key opens it on a quiet one.
    let quiet = sine(220.0, amp(-60.0), 0.5);
    let key = sine(1000.0, amp(-6.0), 0.5);
    let out = render(&mut g, &quiet, Some(&key), &[]);
    assert!((db(peak(&out[12_000..])) + 60.0).abs() < 0.1);
    // The key high-pass (1 kHz) ignores a loud low key.
    let mut g = prepared(set(Gate::new(), &[(gate::SIDECHAIN_HPF, 1000.0)]));
    let low_key = sine(40.0, amp(-30.0), 0.5);
    let out = render(&mut g, &quiet, Some(&low_key), &[]);
    assert_eq!(peak(&out[12_000..]), 0.0);
}

#[test]
fn gate_flip_passes_only_the_quiet_part() {
    let mut g = prepared(set(Gate::new(), &[(gate::FLIP, 1.0)]));
    let out = render(&mut g, &sine(440.0, amp(-10.0), 0.3), None, &[]);
    assert_eq!(peak(&out[4800..]), 0.0, "loud is gated");
    let out = render(&mut g, &sine(440.0, amp(-50.0), 0.3), None, &[]);
    assert!((db(peak(&out[9600..])) + 50.0).abs() < 0.1, "quiet passes");
}

#[test]
fn expander_reduces_by_ratio_below_threshold() {
    // Expander 2:1, threshold -40: a -50 dB input is lowered by another 10 dB.
    let mut g = prepared(set(
        Gate::new(),
        &[(gate::MODE, 1.0), (gate::RATIO, 2.0), (gate::RANGE, -40.0)],
    ));
    let out = render(&mut g, &vec![amp(-50.0); 24_000], None, &[]);
    assert!((db(out[20_000]) + 60.0).abs() < 0.05, "{}", db(out[20_000]));
    // Capped by the floor (range): ratio 20 would be -190 dB, floor -40.
    g.set_param(gate::RATIO, 20.0);
    let out = render(&mut g, &vec![amp(-50.0); 48_000], None, &[]);
    assert!((db(out[40_000]) + 90.0).abs() < 0.05, "{}", db(out[40_000]));
    // Above threshold: unity.
    let out = render(&mut g, &vec![amp(-20.0); 48_000], None, &[]);
    assert!((db(out[40_000]) + 20.0).abs() < 1e-3);
}

#[test]
fn gate_output_param_is_sample_accurate() {
    let mut g = prepared(Gate::new());
    let x = vec![0.5f32; 4000];
    let out = render(&mut g, &x, None, &[(1000, param(gate::OUTPUT, -6.0206))]);
    assert_eq!(out[999], 0.5);
    assert!(out[1000] < 0.5 && out[1000] > 0.49, "{}", out[1000]);
    assert!((out[1031] - 0.25).abs() < 1e-4, "{}", out[1031]);
}

// ─── Multiband Compressor ─────────────────────────────────────────────────────────────

/// Neutral: every band at ratio 1, so only the crossovers act.
fn neutral_mbc() -> MultibandCompressor {
    let mut m = MultibandCompressor::new();
    for id in [mbc::LOW_RATIO, mbc::MID_RATIO, mbc::HIGH_RATIO] {
        m.set_param(id, 1.0);
    }
    prepared(m)
}

/// Steady-state gain (dB) of a sine at `freq` through `d`.
fn sine_gain<D: Device>(d: &mut D, freq: f32, level_db: f32) -> f32 {
    d.reset();
    let x = sine(freq, amp(level_db), 1.0);
    let out = render(d, &x, None, &[]);
    db(rms(&out[24_000..])) - db(rms(&x[24_000..]))
}

#[test]
fn multiband_neutral_sum_is_flat() {
    let mut m = neutral_mbc();
    for f in [30.0, 150.0, 200.0, 700.0, 2500.0, 6000.0, 15_000.0] {
        let g = sine_gain(&mut m, f, -6.0);
        assert!(g.abs() < 0.02, "{f} Hz: {g} dB");
    }
    // Moved crossovers (also crossed: Mid/High below Low/Mid is clamped) stay flat.
    m.set_param(mbc::LOW_MID_FREQ, 800.0);
    m.set_param(mbc::MID_HIGH_FREQ, 600.0);
    for f in [100.0, 800.0, 5000.0] {
        let g = sine_gain(&mut m, f, -6.0);
        assert!(g.abs() < 0.02, "{f} Hz: {g} dB");
    }
}

#[test]
fn multiband_compresses_each_band_independently() {
    // Only the low band compresses hard (threshold -40, 10:1).
    let mut m = neutral_mbc();
    m.set_param(mbc::LOW_THRESHOLD, -40.0);
    m.set_param(mbc::LOW_RATIO, 10.0);
    let low = sine_gain(&mut m, 60.0, -6.0);
    assert!(low < -20.0, "low band compressed: {low}");
    assert!(m.gain_reduction(0) > 20.0);
    assert_eq!(m.gain_reduction(2), 0.0);
    let high = sine_gain(&mut m, 8000.0, -6.0);
    assert!(high.abs() < 0.05, "high band untouched: {high}");
    // Meters: [low, mid, high] in dB (<= 0).
    sine_gain(&mut m, 60.0, -6.0);
    let v = levels(&mut m);
    assert_eq!(v.len(), 3);
    assert!(v[0] < -20.0, "{v:?}");
    assert!(v[2] > -0.5, "{v:?}");
    // Makeup gain applies to its band only.
    m.set_param(mbc::HIGH_MAKEUP, 6.0);
    let high = sine_gain(&mut m, 8000.0, -6.0);
    assert!((high - 6.0).abs() < 0.05, "{high}");
}

#[test]
fn multiband_solo_and_bypass() {
    let mut m = neutral_mbc();
    m.set_param(mbc::MID_SOLO, 1.0);
    assert!(
        sine_gain(&mut m, 60.0, -6.0) < -35.0,
        "low muted by mid solo"
    );
    // (LR4 skirts: -0.23 dB at 1 kHz between 200 Hz and 2.5 kHz)
    assert!(sine_gain(&mut m, 1000.0, -6.0).abs() < 0.5, "mid audible");
    m.set_param(mbc::MID_SOLO, 0.0);
    // Heavy low compression, bypassed: the low band passes unchanged.
    m.set_param(mbc::LOW_THRESHOLD, -60.0);
    m.set_param(mbc::LOW_RATIO, 20.0);
    m.set_param(mbc::LOW_MAKEUP, 12.0);
    m.set_param(mbc::LOW_BYPASS, 1.0);
    assert!(sine_gain(&mut m, 60.0, -6.0).abs() < 0.05);
    // Solo toggled mid-stream crossfades (no step larger than the signal allows).
    let x = sine(1000.0, 0.5, 0.2);
    let out = render(&mut m, &x, None, &[(4800, param(mbc::LOW_SOLO, 1.0))]);
    let max_step = out
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(max_step < 0.2, "{max_step}");
}

#[test]
fn multiband_mix_blends_with_a_phase_matched_dry() {
    let mut m = prepared(set(
        MultibandCompressor::new(),
        &[
            (mbc::LOW_THRESHOLD, -60.0),
            (mbc::MID_THRESHOLD, -60.0),
            (mbc::HIGH_THRESHOLD, -60.0),
            (mbc::LOW_RATIO, 20.0),
            (mbc::MID_RATIO, 20.0),
            (mbc::HIGH_RATIO, 20.0),
            (mbc::MIX, 0.0),
        ],
    ));
    // Mix 0 = dry through the crossover allpasses: flat at every frequency.
    for f in [100.0, 200.0, 1000.0, 2500.0, 9000.0] {
        let g = sine_gain(&mut m, f, -6.0);
        assert!(g.abs() < 0.02, "{f} Hz: {g}");
    }
    // Neutral bands at 50 % mix: still flat (wet and dry in phase, no comb filter).
    let mut n = neutral_mbc();
    n.set_param(mbc::MIX, 50.0);
    for f in [150.0, 200.0, 2500.0, 3000.0] {
        let g = sine_gain(&mut n, f, -6.0);
        assert!(g.abs() < 0.02, "{f} Hz: {g}");
    }
}

#[test]
fn multiband_sidechain_keys_every_band() {
    let mut m = prepared(set(
        MultibandCompressor::new(),
        &[
            (mbc::LOW_THRESHOLD, -30.0),
            (mbc::MID_THRESHOLD, -30.0),
            (mbc::HIGH_THRESHOLD, -30.0),
            (mbc::LOW_RATIO, 10.0),
            (mbc::MID_RATIO, 10.0),
            (mbc::HIGH_RATIO, 10.0),
        ],
    ));
    // Quiet main (below every threshold), loud 1 kHz key: all three bands reduce.
    let main = sine(100.0, amp(-40.0), 0.5);
    let key = sine(1000.0, amp(-6.0), 0.5);
    let out = render(&mut m, &main, Some(&key), &[]);
    let g = db(rms(&out[12_000..])) - db(rms(&main[12_000..]));
    assert!(g < -15.0, "{g}");
    for b in 0..3 {
        assert!(m.gain_reduction(b) > 15.0, "band {b}");
    }
    // Without a key: untouched.
    m.reset();
    let out = render(&mut m, &main, None, &[]);
    let g = db(rms(&out[12_000..])) - db(rms(&main[12_000..]));
    assert!(g.abs() < 0.05, "{g}");
}

#[test]
fn multiband_crossover_automation_glides_without_clicks() {
    let mut m = neutral_mbc();
    let x = sine(300.0, 0.5, 0.5);
    let events: Vec<(usize, EventKind)> = (0..50)
        .map(|i| {
            (
                i * 256,
                param(mbc::LOW_MID_FREQ, if i % 2 == 0 { 50.0 } else { 900.0 }),
            )
        })
        .collect();
    let out = render(&mut m, &x, None, &events);
    let max_step = out
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    // A 300 Hz sine at 0.5 moves at most ~0.02 per sample.
    assert!(max_step < 0.05, "{max_step}");
}

// ─── Transient Shaper ─────────────────────────────────────────────────────────────────

/// A decaying 200 Hz "hit" every 0.25 s.
fn hits(level: f32, secs: f32) -> Vec<f32> {
    (0..(secs * SR) as usize)
        .map(|i| {
            let t = (i % (SR as usize / 4)) as f32 / SR;
            level * (-t * 30.0).exp() * (2.0 * PI * 200.0 * t).sin()
        })
        .collect()
}

#[test]
fn shaper_neutral_is_transparent() {
    let mut s = prepared(TransientShaper::new());
    let x = hits(0.8, 1.0);
    let out = render(&mut s, &x, None, &[]);
    assert_eq!(out, x);
}

/// Peak of the first 5 ms and RMS of 60..200 ms after each hit, in dB relative to input.
fn attack_and_tail(x: &[f32], out: &[f32]) -> (f32, f32) {
    let hit = SR as usize / 4;
    let start = 2 * hit; // skip the first hits (followers settle)
    let a = |v: &[f32]| peak(&v[start..start + 240]);
    let t = |v: &[f32]| rms(&v[start + 2880..start + 9600]);
    (db(a(out)) - db(a(x)), db(t(out)) - db(t(x)))
}

#[test]
fn shaper_attack_and_sustain_shape_hits() {
    let x = hits(0.5, 1.0);
    let mut s = prepared(set(TransientShaper::new(), &[(ts::ATTACK, 100.0)]));
    let (a, t) = attack_and_tail(&x, &render(&mut s, &x, None, &[]));
    assert!(a > 6.0, "attack boosted: {a}");
    assert!(t.abs() < 1.0, "tail ~untouched: {t}");
    assert!(levels(&mut s)[0] > 6.0);

    let mut s = prepared(set(TransientShaper::new(), &[(ts::ATTACK, -100.0)]));
    let (a, _) = attack_and_tail(&x, &render(&mut s, &x, None, &[]));
    assert!(a < -3.0, "attack softened: {a}");

    let mut s = prepared(set(TransientShaper::new(), &[(ts::SUSTAIN, -100.0)]));
    let (a, t) = attack_and_tail(&x, &render(&mut s, &x, None, &[]));
    assert!(t < -6.0, "tail reduced: {t}");
    assert!(a.abs() < 1.0, "attack ~untouched: {a}");

    let mut s = prepared(set(TransientShaper::new(), &[(ts::SUSTAIN, 100.0)]));
    let (_, t) = attack_and_tail(&x, &render(&mut s, &x, None, &[]));
    assert!(t > 6.0, "tail boosted: {t}");
}

#[test]
fn shaper_is_level_independent_and_steady_signals_pass() {
    let shape = |level: f32| {
        let x = hits(level, 1.0);
        let mut s = prepared(set(TransientShaper::new(), &[(ts::ATTACK, 70.0)]));
        attack_and_tail(&x, &render(&mut s, &x, None, &[])).0
    };
    let (loud, quiet) = (shape(0.8), shape(0.05));
    assert!((loud - quiet).abs() < 0.5, "{loud} vs {quiet}");
    // A steady tone is unchanged once the followers settle.
    let mut s = prepared(set(
        TransientShaper::new(),
        &[(ts::ATTACK, 100.0), (ts::SUSTAIN, 100.0)],
    ));
    let x = sine(440.0, 0.3, 1.0);
    let out = render(&mut s, &x, None, &[]);
    let g = db(rms(&out[24_000..])) - db(rms(&x[24_000..]));
    assert!(g.abs() < 0.3, "{g}");
}

#[test]
fn shaper_clip_mix_and_output() {
    let x = hits(1.0, 1.0);
    let mut s = prepared(set(
        TransientShaper::new(),
        &[(ts::ATTACK, 100.0), (ts::CLIP, 1.0)],
    ));
    let out = render(&mut s, &x, None, &[]);
    assert!(peak(&out) <= 1.0);
    // Mix 0 = dry; Output ramps over one automation grid interval.
    let mut s = prepared(set(
        TransientShaper::new(),
        &[(ts::ATTACK, 100.0), (ts::MIX, 0.0)],
    ));
    let dc = vec![0.5f32; 4000];
    let out = render(&mut s, &dc, None, &[(2000, param(ts::OUTPUT, -6.0206))]);
    assert_eq!(out[1999], 0.5);
    assert!((out[2031] - 0.25).abs() < 1e-4, "{}", out[2031]);
}

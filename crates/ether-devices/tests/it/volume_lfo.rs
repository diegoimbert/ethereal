//! Volume LFO render tests: the gain is the drawn curve at the song-locked phase (several
//! tempos, sample rates and start positions), depth 0 is bit-exact unity, curve swaps and
//! param changes never step faster than the smoothing allows, renders don't depend on the
//! block size, no NaN or subnormals, and the factory curves round-trip. Every `process` and
//! `set_data` call runs under `assert_no_alloc`.

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::model::{
    BuiltinDevice, BuiltinDeviceType, CurveShape, LfoCurve, LfoPoint, ParamId,
    check_volume_shaper_bands, load_preset,
};
use ether_core::{
    AudioBuffers, Device, EventBuffer, EventKind, PrepareConfig, ProcessContext, ProcessEvent,
    ProcessStatus, TransportInfo,
};
use ether_devices::NoSamples;
use ether_devices::contract::SYNC_RATE_BEATS;
use ether_devices::fx_modulation::{volume_lfo as p, volume_lfo_curve};

const BLOCK: usize = 256;
const QUARTER: f64 = 8.0; // `SYNC_RATES` index of 1/4

fn ramp_curve() -> LfoCurve {
    LfoCurve {
        points: vec![
            LfoPoint::linear(0.0, 1.0),
            LfoPoint::curved(0.1, 0.0, -0.5),
            LfoPoint::linear(0.7, 0.8),
            LfoPoint {
                x: 0.85,
                y: 0.3,
                curve: CurveShape::Step,
            },
        ],
    }
}

fn device(sr: f32, curve: &LfoCurve, params: &[(ParamId, f64)]) -> Box<dyn Device> {
    banded(sr, curve, &[], params)
}

/// A device whose band 1 is `curve` and bands 2 and 3 `bands`.
fn banded(
    sr: f32,
    curve: &LfoCurve,
    bands: &[LfoCurve],
    params: &[(ParamId, f64)],
) -> Box<dyn Device> {
    let kind = BuiltinDevice::VolumeLfo {
        curve: curve.clone(),
        bands: bands.to_vec(),
    };
    let mut d = ether_devices::create(&kind, &NoSamples);
    for &(id, v) in params {
        d.set_param(id, v);
    }
    d.prepare(&PrepareConfig {
        sample_rate: sr,
        max_block_size: 1024,
        max_events_per_block: 64,
    });
    d
}

/// How a render plays.
#[derive(Clone, Copy)]
struct Play {
    sr: f32,
    bpm: f64,
    /// Song position (beats) when the transport starts.
    start: f64,
    /// Frame at which the transport starts (`usize::MAX` = never).
    at: usize,
    block: usize,
}

impl Play {
    fn new(sr: f32) -> Self {
        Self {
            sr,
            bpm: 120.0,
            start: 0.0,
            at: 0,
            block: BLOCK,
        }
    }

    fn beats_per_sample(&self) -> f64 {
        self.bpm / 60.0 / f64::from(self.sr)
    }

    /// Song position at `frame` (once playing).
    fn position(&self, frame: usize) -> f64 {
        self.start + self.beats_per_sample() * (frame - self.at) as f64
    }
}

/// Something to do at a frame.
enum Act {
    Event(EventKind),
    Curve(LfoCurve),
}

fn render(
    d: &mut dyn Device,
    play: Play,
    input: [&[f32]; 2],
    acts: &[(usize, Act)],
) -> [Vec<f32>; 2] {
    let frames = input[0].len();
    let mut out = [vec![0.0f32; frames], vec![0.0f32; frames]];
    let mut out_events = EventBuffer::with_capacity(64);
    let mut block_events: Vec<ProcessEvent> = Vec::with_capacity(64);
    let mut pos = 0;
    while pos < frames {
        let mut n = play.block.min(frames - pos);
        // Curve swaps land between blocks: cut the block there.
        if let Some(at) = acts
            .iter()
            .filter_map(|(at, a)| matches!(a, Act::Curve(_)).then_some(*at))
            .find(|&at| at > pos && at < pos + n)
        {
            n = at - pos;
        }
        for (at, act) in acts {
            if let Act::Curve(c) = act
                && *at == pos
            {
                let data = volume_lfo_curve(&BuiltinDevice::VolumeLfo {
                    curve: c.clone(),
                    bands: Vec::new(),
                })
                .expect("curve data");
                let mut back = None;
                assert_no_alloc(|| back = d.set_data(data));
                assert!(
                    back.is_some_and(|b| b.is::<ether_devices::fx_modulation::VolumeLfoCurves>())
                );
            }
        }
        block_events.clear();
        for (at, act) in acts {
            if let Act::Event(kind) = act
                && *at >= pos
                && *at < pos + n
            {
                block_events.push(ProcessEvent {
                    offset: (*at - pos) as u32,
                    kind: *kind,
                });
            }
        }
        let playing = pos >= play.at;
        let transport = TransportInfo {
            playing,
            bpm: play.bpm,
            beats_per_sample: if playing {
                play.beats_per_sample()
            } else {
                0.0
            },
            position: if playing { play.position(pos) } else { 0.0 },
            sample_time: pos as u64,
            ..TransportInfo::STOPPED
        };
        let inputs: [&[f32]; 2] = [&input[0][pos..pos + n], &input[1][pos..pos + n]];
        let (l, r) = out.split_at_mut(1);
        let mut outputs: [&mut [f32]; 2] = [&mut l[0][pos..pos + n], &mut r[0][pos..pos + n]];
        let mut ctx = ProcessContext {
            sample_rate: play.sr,
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

fn ones(frames: usize) -> Vec<f32> {
    vec![1.0; frames]
}

fn noise(frames: usize, seed: u32) -> Vec<f32> {
    let mut s = seed;
    (0..frames)
        .map(|_| {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((s >> 8) as f32 / (1 << 24) as f32 * 2.0 - 1.0) * 0.5
        })
        .collect()
}

/// `v` is `gain(curve(phase))` (within float noise), the phase rounded either way: on a
/// vertical edge the expected value depends on the last bits of the phase.
fn on_curve(curve: &LfoCurve, phase: f64, v: f32, gain: impl Fn(f64) -> f64) -> bool {
    [-1e-9, 0.0, 1e-9]
        .iter()
        .any(|e| (f64::from(v) - gain(curve.value_at(phase + e))).abs() < 2e-4)
}

fn max_step(x: &[f32]) -> f32 {
    x.windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

/// The gain is the curve at the song position, whatever the tempo, the sample rate and
/// where playback starts (no smoothing: exactly the drawn curve).
#[test]
fn gain_is_the_curve_at_the_song_position() {
    let curve = ramp_curve();
    for (sr, bpm, start, rate) in [
        (48_000.0, 120.0, 0.0, QUARTER),
        (44_100.0, 128.0, 13.37, QUARTER),
        (96_000.0, 174.0, 3.5, 6.0),
        (48_000.0, 90.0, 7.25, 10.0),
        (44_100.0, 140.0, 0.6, 4.0),
    ] {
        let play = Play {
            sr,
            bpm,
            start,
            at: 0,
            block: BLOCK,
        };
        let beats = SYNC_RATE_BEATS[rate as usize];
        let mut d = device(sr, &curve, &[(p::SMOOTHING, 0.0), (p::SYNC_RATE, rate)]);
        let frames = sr as usize;
        let x = ones(frames);
        let [l, r] = render(d.as_mut(), play, [&x, &x], &[]);
        for i in 0..frames {
            let phase = play.position(i) / beats;
            assert!(
                on_curve(&curve, phase, l[i], |g| g) && on_curve(&curve, phase, r[i], |g| g),
                "sr {sr} bpm {bpm} start {start} frame {i}: {} vs {}",
                l[i],
                curve.value_at(phase)
            );
        }
    }
}

/// A free LFO restarts at phase 0 when the transport starts; stopped, a synced one runs on
/// at the tempo.
#[test]
fn free_rate_restarts_with_the_transport_and_stopped_sync_runs_on() {
    let curve = ramp_curve();
    let sr = 48_000.0;
    let hz = 3.0;
    let frames = 24_000;
    let x = ones(frames);
    // A block boundary (the transport starts at a block's first frame).
    let at = 20 * BLOCK;
    let mut d = device(
        sr,
        &curve,
        &[(p::SMOOTHING, 0.0), (p::SYNC, 0.0), (p::RATE, hz)],
    );
    let play = Play {
        at,
        ..Play::new(sr)
    };
    let [l, _] = render(d.as_mut(), play, [&x, &x], &[]);
    for (i, &v) in l.iter().enumerate().skip(at) {
        let phase = hz * (i - at + 1) as f64 / f64::from(sr);
        assert!(on_curve(&curve, phase, v, |g| g), "frame {i}");
    }
    // Synced, never playing: one cycle per quarter at 120 BPM.
    let mut d = device(sr, &curve, &[(p::SMOOTHING, 0.0)]);
    let stopped = Play {
        at: usize::MAX,
        ..Play::new(sr)
    };
    let [l, _] = render(d.as_mut(), stopped, [&x, &x], &[]);
    for (i, &v) in l.iter().enumerate() {
        let phase = 2.0 * (i + 1) as f64 / f64::from(sr);
        assert!(on_curve(&curve, phase, v, |g| g), "frame {i}");
    }
}

/// Depth scales the dip, phase offsets the cycle, stereo phase delays the right channel.
#[test]
fn depth_phase_and_stereo_phase() {
    let curve = ramp_curve();
    let sr = 48_000.0;
    let frames = 48_000;
    let x = ones(frames);
    let mut d = device(
        sr,
        &curve,
        &[
            (p::SMOOTHING, 0.0),
            (p::DEPTH, 40.0),
            (p::PHASE, 90.0),
            (p::STEREO_PHASE, 180.0),
            (p::OUTPUT, -6.0),
        ],
    );
    let play = Play::new(sr);
    let [l, r] = render(d.as_mut(), play, [&x, &x], &[]);
    let out = 10f64.powf(-6.0 / 20.0);
    let gain = |c: f64| (1.0 - 0.4 * (1.0 - c)) * out;
    for i in 0..frames {
        let ph = play.position(i);
        assert!(on_curve(&curve, ph + 0.25, l[i], gain), "left {i}");
        assert!(on_curve(&curve, ph + 0.75, r[i], gain), "right {i}");
    }
}

/// Depth 0 is bit-exact unity, whatever the curve and the other params.
#[test]
fn depth_zero_is_transparent() {
    let sr = 44_100.0;
    let x = noise(30_000, 7);
    let y = noise(30_000, 8);
    let mut d = device(
        sr,
        &ramp_curve(),
        &[(p::DEPTH, 0.0), (p::PHASE, 45.0), (p::STEREO_PHASE, 90.0)],
    );
    let [l, r] = render(d.as_mut(), Play::new(sr), [&x, &y], &[]);
    assert_eq!(l, x);
    assert_eq!(r, y);
}

/// Curve swaps (square edges included) and jumps of every param never move the gain faster
/// than the smoothing allows: at most `1 - e^(-1/(ms·sr))` of a full swing per sample.
#[test]
fn edits_never_click() {
    let sr = 48_000.0;
    let frames = 96_000;
    let x = ones(frames);
    let square = LfoCurve {
        points: vec![
            LfoPoint {
                x: 0.0,
                y: 1.0,
                curve: CurveShape::Step,
            },
            LfoPoint {
                x: 0.5,
                y: 0.0,
                curve: CurveShape::Step,
            },
        ],
    };
    let flat_low = LfoCurve {
        points: vec![LfoPoint::linear(0.0, 0.0)],
    };
    let ev = |id: ParamId, value: f64| {
        Act::Event(EventKind::Param {
            param: id,
            value,
            step: false,
        })
    };
    let acts = vec![
        (4_000, ev(p::PHASE, 270.0)),
        (9_001, Act::Curve(flat_low)),
        (13_337, ev(p::DEPTH, 10.0)),
        (15_000, ev(p::DEPTH, 100.0)),
        (20_000, Act::Curve(LfoCurve::default())),
        (27_777, ev(p::SYNC_RATE, 3.0)),
        (33_000, ev(p::SYNC, 0.0)),
        (40_000, ev(p::RATE, 17.0)),
        (51_000, ev(p::STEREO_PHASE, 180.0)),
        (60_000, Act::Curve(square.clone())),
        (70_000, ev(p::SYNC, 1.0)),
    ];
    for ms in [1.0f32, 5.0] {
        let mut d = device(sr, &square, &[(p::SMOOTHING, f64::from(ms))]);
        let [l, r] = render(d.as_mut(), Play::new(sr), [&x, &x], &acts);
        let bound = 1.0 - (-1.0 / (ms * 0.001 * sr)).exp();
        for ch in [&l, &r] {
            let step = max_step(ch);
            assert!(step <= bound * 1.001, "{ms} ms: step {step} > {bound}");
        }
        // The square still reaches both of its levels.
        let tail = &l[frames - 24_000..];
        assert!(tail.iter().any(|&v| v > 0.99) && tail.iter().any(|&v| v < 0.01));
    }
}

/// The same render in blocks of 1, 37, 256 and 1024 frames.
#[test]
fn renders_do_not_depend_on_the_block_size() {
    let sr = 48_000.0;
    let x = noise(20_000, 3);
    let ev = |id: ParamId, value: f64| {
        Act::Event(EventKind::Param {
            param: id,
            value,
            step: false,
        })
    };
    let acts = vec![
        (1_234, ev(p::DEPTH, 60.0)),
        (5_000, ev(p::PHASE, 33.0)),
        (12_000, ev(p::SYNC_RATE, 6.0)),
    ];
    let run = |block: usize| {
        let mut d = device(sr, &ramp_curve(), &[(p::STEREO_PHASE, 30.0)]);
        let play = Play {
            block,
            start: 2.25,
            ..Play::new(sr)
        };
        render(d.as_mut(), play, [&x, &x], &acts)
    };
    let a = run(1024);
    for block in [1, 37, 256] {
        let b = run(block);
        for ch in 0..2 {
            let diff = a[ch]
                .iter()
                .zip(&b[ch])
                .map(|(u, v)| (u - v).abs())
                .fold(0.0f32, f32::max);
            assert!(diff < 1e-6, "block {block}: {diff}");
        }
    }
    // Deterministic: the same render twice is bit-identical.
    assert_eq!(run(256), run(256));
}

/// No NaN, no subnormals, silence in = silence out (and the node then sleeps).
#[test]
fn no_nan_no_subnormals_and_silence() {
    let sr = 48_000.0;
    let frames = 48_000;
    let tiny: Vec<f32> = (0..frames)
        .map(|i| 1e-20 * ((i % 7) as f32 - 3.0))
        .collect();
    let mut d = device(
        sr,
        &LfoCurve {
            points: vec![LfoPoint::linear(0.0, 0.0), LfoPoint::curved(0.9, 1.0, 1.0)],
        },
        &[(p::SMOOTHING, 20.0), (p::OUTPUT, 24.0), (p::SYNC_RATE, 0.0)],
    );
    let [l, r] = render(d.as_mut(), Play::new(sr), [&tiny, &tiny], &[]);
    for v in l.iter().chain(&r) {
        assert!(v.is_finite() && (*v == 0.0 || v.is_normal()), "{v:e}");
    }
    // Silent in, silent out, then asleep.
    let silence = vec![0.0f32; 4_096];
    let mut d = device(sr, &ramp_curve(), &[]);
    let mut out = [vec![1.0f32; 512], vec![1.0f32; 512]];
    let mut out_events = EventBuffer::with_capacity(8);
    let mut status = ProcessStatus::Continue;
    for b in 0..8 {
        let transport = TransportInfo {
            sample_time: b * 512,
            ..TransportInfo::STOPPED
        };
        let inputs: [&[f32]; 2] = [&silence[..512], &silence[..512]];
        let (l, r) = out.split_at_mut(1);
        let mut outputs: [&mut [f32]; 2] = [&mut l[0], &mut r[0]];
        let mut ctx = ProcessContext {
            sample_rate: sr,
            frames: 512,
            transport: &transport,
            events: &[],
            out_events: &mut out_events,
        };
        status = d.process(
            &mut ctx,
            &mut AudioBuffers {
                inputs: &inputs,
                outputs: &mut outputs,
            },
        );
        assert!(out.iter().all(|c| c.iter().all(|&v| v == 0.0)));
    }
    assert_eq!(status, ProcessStatus::Silent);
}

/// `set_data` takes curves only; anything else comes back unchanged.
#[test]
fn set_data_rejects_foreign_payloads() {
    let mut d = device(48_000.0, &ramp_curve(), &[]);
    let back = d.set_data(Box::new(42u32)).expect("handed back");
    assert_eq!(back.downcast_ref::<u32>(), Some(&42));
    assert!(volume_lfo_curve(&BuiltinDevice::Compressor).is_none());
}

/// Every factory preset: a valid curve that survives a JSON round trip, and params in range.
#[test]
fn factory_presets_are_valid_and_round_trip() {
    let presets = ether_devices::factory_presets(BuiltinDeviceType::VolumeLfo);
    assert_eq!(presets.len(), 15);
    let desc = ether_devices::descriptor(BuiltinDeviceType::VolumeLfo);
    for fp in presets {
        let preset = load_preset(fp.json).unwrap_or_else(|e| panic!("{}: {e}", fp.id));
        assert_eq!(preset.name, preset.name.to_lowercase(), "{}", fp.id);
        let kind = preset.kind.clone().expect("curve");
        let BuiltinDevice::VolumeLfo { curve, bands } = &kind else {
            panic!("{}: {kind:?}", fp.id)
        };
        assert!(curve.check().is_ok(), "{}", fp.id);
        assert_eq!(&curve.sanitized(), curve, "{}", fp.id);
        assert!(check_volume_shaper_bands(bands).is_ok(), "{}", fp.id);
        for b in bands {
            assert_eq!(&b.sanitized(), b, "{}", fp.id);
        }
        // A multiband preset has a curve for each of its bands.
        let count = preset.params.get(&p::BANDS).map_or(1, |v| *v as usize + 1);
        assert!(bands.len() + 1 >= count, "{}", fp.id);
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(serde_json::from_str::<BuiltinDevice>(&json).unwrap(), kind);
        for info in &desc.params {
            // Older presets leave the band params out (loaded at their defaults: one band).
            let Some(&v) = preset.params.get(&info.id) else {
                assert!(info.id.0 >= p::BANDS.0, "{} {}", fp.id, info.name);
                continue;
            };
            assert!(
                (info.min..=info.max).contains(&v),
                "{} {}",
                fp.id,
                info.name
            );
        }
    }
}

// ---- Bands ------------------------------------------------------------------------------

fn sine(frames: usize, hz: f64, sr: f32) -> Vec<f32> {
    (0..frames)
        .map(|i| (0.5 * (std::f64::consts::TAU * hz * i as f64 / f64::from(sr)).sin()) as f32)
        .collect()
}

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|&v| f64::from(v).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
}

/// Output / input level (dB) of a steady sine, measured after the filters settled.
fn level_db(d: &mut dyn Device, hz: f64, sr: f32) -> f64 {
    let frames = sr as usize;
    let x = sine(frames, hz, sr);
    let [l, r] = render(d, Play::new(sr), [&x, &x], &[]);
    let from = frames / 2;
    let db = |y: &[f32]| 20.0 * (rms(&y[from..]) / rms(&x[from..])).log10();
    let (a, b) = (db(&l), db(&r));
    assert!((a - b).abs() < 1e-6, "{hz} Hz: left {a} vs right {b}");
    a
}

fn flat_at(y: f32) -> LfoCurve {
    LfoCurve {
        points: vec![LfoPoint::linear(0.0, y)],
    }
}

/// Every band at unity: the split sums flat (an allpass), with 2 and 3 bands, at the
/// default and at moved crossovers (the upper one under the lower one too).
#[test]
fn unity_bands_sum_flat() {
    let sr = 48_000.0;
    for (bands, lo, hi) in [
        (1.0, 200.0, 15_000.0),
        (2.0, 200.0, 15_000.0),
        (2.0, 900.0, 4_000.0),
        (2.0, 3_000.0, 300.0),
    ] {
        let mut worst = 0.0f64;
        for hz in [
            30.0, 80.0, 200.0, 450.0, 1_000.0, 2_500.0, 6_000.0, 12_000.0, 15_000.0, 19_000.0,
        ] {
            let mut d = banded(
                sr,
                &LfoCurve::flat(),
                &[],
                &[
                    (p::BANDS, bands),
                    (p::CROSSOVER_1, lo),
                    (p::CROSSOVER_2, hi),
                ],
            );
            worst = worst.max(level_db(d.as_mut(), hz, sr).abs());
        }
        assert!(
            worst < 0.02,
            "{} bands at {lo}/{hi} Hz: {worst} dB off flat",
            bands + 1.0
        );
    }
}

/// A band's curve only ducks its band: band 1 at 0 removes a 50 Hz sine and leaves a
/// 2 kHz one alone; band 2 at 0 (of 3) does the opposite, and band 3 takes the highs.
#[test]
fn a_band_only_ducks_its_band() {
    let sr = 48_000.0;
    let level = |curve: &LfoCurve, bands: &[LfoCurve], count: f64, hz: f64| {
        let mut d = banded(sr, curve, bands, &[(p::BANDS, count)]);
        level_db(d.as_mut(), hz, sr)
    };
    let silent = flat_at(0.0);
    let unity = LfoCurve::flat();
    assert!(level(&silent, &[], 2.0, 50.0) < -40.0);
    assert!(level(&silent, &[], 2.0, 2_000.0).abs() < 0.05);
    assert!(level(&silent, &[], 2.0, 18_000.0).abs() < 0.05);
    let mid_off = [silent.clone(), unity.clone()];
    assert!(level(&unity, &mid_off, 2.0, 50.0).abs() < 0.05);
    assert!(level(&unity, &mid_off, 2.0, 2_000.0) < -40.0);
    let high_off = [unity.clone(), silent.clone()];
    assert!(level(&unity, &high_off, 2.0, 2_000.0).abs() < 0.05);
    assert!(level(&unity, &high_off, 2.0, 19_500.0) < -12.0);
    // Two bands: band 2 is everything above `Crossover 1`.
    let upper_off = [silent.clone()];
    assert!(level(&unity, &upper_off, 1.0, 50.0).abs() < 0.05);
    assert!(level(&unity, &upper_off, 1.0, 2_000.0) < -40.0);
    assert!(level(&unity, &upper_off, 1.0, 18_000.0) < -40.0);
    // A band's depth scales its dip: band 2 at 0 with 50 % depth = -6 dB on the mids.
    let mut d = banded(
        sr,
        &unity,
        &[silent],
        &[(p::BANDS, 2.0), (p::BAND_2_DEPTH, 50.0)],
    );
    assert!((level_db(d.as_mut(), 2_000.0, sr) - 20.0 * 0.5f64.log10()).abs() < 0.05);
}

/// Mean `|y|` over `a..b`.
fn envelope(y: &[f32], a: usize, b: usize) -> f32 {
    y[a..b].iter().map(|v| v.abs()).sum::<f32>() / (b - a) as f32
}

/// Each band plays its own curve at its own speed: band 3 at ×4 follows its ramp once per
/// sixteenth (at 1/4), band 2 at ÷2 spans two quarters (locked to the song position).
#[test]
fn bands_follow_their_own_curve_and_speed() {
    let sr = 48_000.0;
    let ramp = LfoCurve {
        points: vec![LfoPoint::linear(0.0, 0.0), LfoPoint::linear(1.0, 1.0)],
    };
    let mut d = banded(
        sr,
        &LfoCurve::flat(),
        &[LfoCurve::flat(), ramp.clone()],
        &[
            (p::BANDS, 2.0),
            (p::CROSSOVER_2, 3_000.0),
            (p::BAND_3_SPEED, 5.0),
            (p::SMOOTHING, 0.0),
        ],
    );
    let x = sine(48_000, 12_000.0, sr);
    let [l, _] = render(d.as_mut(), Play::new(sr), [&x, &x], &[]);
    // At 120 BPM a quarter is 24000 samples: a ×4 cycle is 6000.
    for c in 2..7 {
        let start = c * 6_000;
        let low = envelope(&l, start + 100, start + 1_500);
        let high = envelope(&l, start + 4_500, start + 5_900);
        assert!(low < 0.3 * high, "cycle {c}: {low} vs {high}");
    }
    let mut d = banded(
        sr,
        &LfoCurve::flat(),
        &[ramp],
        &[(p::BANDS, 1.0), (p::BAND_2_SPEED, 1.0), (p::SMOOTHING, 0.0)],
    );
    let x = sine(96_000, 3_000.0, sr);
    let [l, _] = render(d.as_mut(), Play::new(sr), [&x, &x], &[]);
    // Rising over 48000 samples: at a quarter (24000) it is half way.
    let half = envelope(&l, 23_000, 25_000);
    let full = envelope(&l, 46_000, 47_900);
    assert!((half / full - 0.5).abs() < 0.05, "{half} / {full}");
}

/// Changing the band count and the crossovers mid-render never clicks (on a steady low
/// sine the output never jumps), and renders don't depend on the block size.
#[test]
fn band_changes_are_smooth_and_block_size_independent() {
    let sr = 48_000.0;
    let frames = 60_000;
    let x = sine(frames, 110.0, sr);
    let ev = |id: ParamId, value: f64| {
        Act::Event(EventKind::Param {
            param: id,
            value,
            step: false,
        })
    };
    let acts = vec![
        (6_000, ev(p::BANDS, 2.0)),
        (15_000, ev(p::CROSSOVER_1, 60.0)),
        (24_000, ev(p::BANDS, 1.0)),
        (33_000, ev(p::CROSSOVER_1, 1_000.0)),
        (42_000, ev(p::BANDS, 0.0)),
        (51_000, ev(p::BANDS, 2.0)),
    ];
    let run = |block: usize| {
        let mut d = banded(sr, &flat_at(0.2), &[flat_at(1.0), flat_at(0.6)], &[]);
        let play = Play {
            block,
            ..Play::new(sr)
        };
        render(d.as_mut(), play, [&x, &x], &acts)
    };
    let a = run(256);
    // A 110 Hz sine at 0.5 moves at most ~0.0072 per sample.
    let step = max_step(&a[0]);
    assert!(step < 0.012, "step {step}");
    for block in [1, 64, 1024] {
        let b = run(block);
        let diff = a[0]
            .iter()
            .zip(&b[0])
            .map(|(u, v)| (u - v).abs())
            .fold(0.0f32, f32::max);
        assert!(diff < 1e-6, "block {block}: {diff}");
    }
    for v in a.iter().flatten() {
        assert!(v.is_finite() && (*v == 0.0 || v.is_normal()), "{v:e}");
    }
}

/// One band ignores the band params and curves: bit-identical to the plain shaper.
#[test]
fn one_band_ignores_the_band_params() {
    let sr = 44_100.0;
    let x = noise(20_000, 5);
    let plain = {
        let mut d = device(sr, &ramp_curve(), &[]);
        render(d.as_mut(), Play::new(sr), [&x, &x], &[])
    };
    let mut d = banded(
        sr,
        &ramp_curve(),
        &[flat_at(0.0), flat_at(0.0)],
        &[
            (p::CROSSOVER_1, 1_000.0),
            (p::BAND_2_DEPTH, 10.0),
            (p::BAND_3_SPEED, 6.0),
        ],
    );
    assert_eq!(render(d.as_mut(), Play::new(sr), [&x, &x], &[]), plain);
}

/// Split, silence in still ends in silence (the crossovers ring out) and the node sleeps.
#[test]
fn split_goes_silent() {
    let sr = 48_000.0;
    let mut d = banded(sr, &LfoCurve::default(), &[], &[(p::BANDS, 2.0)]);
    let x = noise(4_096, 9);
    render(d.as_mut(), Play::new(sr), [&x, &x], &[]);
    let silence = vec![0.0f32; 512];
    let mut out = [vec![1.0f32; 512], vec![1.0f32; 512]];
    let mut out_events = EventBuffer::with_capacity(8);
    let mut status = ProcessStatus::Continue;
    for b in 0..20u64 {
        let transport = TransportInfo {
            sample_time: 4_096 + b * 512,
            ..TransportInfo::STOPPED
        };
        let inputs: [&[f32]; 2] = [&silence, &silence];
        let (l, r) = out.split_at_mut(1);
        let mut outputs: [&mut [f32]; 2] = [&mut l[0], &mut r[0]];
        let mut ctx = ProcessContext {
            sample_rate: sr,
            frames: 512,
            transport: &transport,
            events: &[],
            out_events: &mut out_events,
        };
        status = d.process(
            &mut ctx,
            &mut AudioBuffers {
                inputs: &inputs,
                outputs: &mut outputs,
            },
        );
    }
    assert_eq!(status, ProcessStatus::Silent);
}

/// "sidechain lows 1/4" ducks the lows on the beat and leaves the mids alone.
#[test]
fn sidechain_lows_preset_ducks_only_the_lows() {
    let fp = ether_devices::factory_presets(BuiltinDeviceType::VolumeLfo)
        .iter()
        .find(|fp| fp.id == "volume-lfo/sidechain-lows-1-4")
        .expect("preset");
    let preset = load_preset(fp.json).unwrap();
    let kind = preset.kind.clone().unwrap();
    let sr = 48_000.0;
    let run = |hz: f64| {
        let mut d = ether_devices::create(&kind, &NoSamples);
        for (id, v) in &preset.params {
            d.set_param(*id, *v);
        }
        d.prepare(&PrepareConfig {
            sample_rate: sr,
            max_block_size: 1024,
            max_events_per_block: 64,
        });
        let x = sine(48_000, hz, sr);
        let [l, _] = render(d.as_mut(), Play::new(sr), [&x, &x], &[]);
        // The first tenth of beat 2 (frame 24000 at 120 BPM): the dip.
        (rms(&l[24_000..26_400]), rms(&x[24_000..26_400]))
    };
    let (lo, lo_in) = run(50.0);
    assert!(lo < 0.5 * lo_in, "{lo} vs {lo_in}");
    let (mid, mid_in) = run(2_000.0);
    assert!((mid / mid_in - 1.0).abs() < 0.01, "{mid} vs {mid_in}");
}

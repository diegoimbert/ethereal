//! Convolution Reverb (v0.3, `fx-space`): render checks (exact IR at zero latency,
//! pre-delay, shaping, mix, missing IR), click-free IR swaps and shaping changes, and RT
//! safety (`process` and `set_data` under `assert_no_alloc`, swaps and rebuilds included).
//!
//! `cargo test --release -p ether-devices --test fx_space -- --ignored --nocapture cpu`
//! prints the CPU cost for 1 s and 5 s IRs at 48 kHz.

use std::sync::Arc;

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::model::{BuiltinDevice, IrSource, MediaId, ParamId, Ulid};
use ether_core::{
    AudioBuffers, AudioSource, Device, EventBuffer, EventKind, PrepareConfig, ProcessContext,
    ProcessEvent, TransportInfo,
};
use ether_devices::fx_space::{self, convolution_reverb as p};
use ether_devices::{NoSamples, SampleResolver};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;

/// An in-memory IR file.
struct Mem(Vec<Vec<f32>>);

impl AudioSource for Mem {
    fn channels(&self) -> u16 {
        self.0.len() as u16
    }
    fn frames(&self) -> u64 {
        self.0[0].len() as u64
    }
    fn read(&self, channel: u16, start: u64, out: &mut [f32]) -> bool {
        let ch = &self.0[channel as usize];
        for (i, o) in out.iter_mut().enumerate() {
            *o = ch.get(start as usize + i).copied().unwrap_or(0.0);
        }
        true
    }
}

const IR_MEDIA: MediaId = MediaId(Ulid(7));

struct One(Arc<dyn AudioSource>);

impl SampleResolver for One {
    fn resolve(&self, media: MediaId) -> Option<Arc<dyn AudioSource>> {
        (media == IR_MEDIA).then(|| self.0.clone())
    }
}

fn media_reverb() -> BuiltinDevice {
    BuiltinDevice::ConvolutionReverb {
        ir: Some(IrSource::Media { media: IR_MEDIA }),
    }
}

fn factory(id: &str) -> BuiltinDevice {
    BuiltinDevice::ConvolutionReverb {
        ir: Some(IrSource::Factory { id: id.into() }),
    }
}

fn noise(n: usize, seed: u32) -> Vec<f32> {
    let mut s = seed;
    (0..n)
        .map(|_| {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (s >> 8) as f32 / (1 << 24) as f32 * 2.0 - 1.0
        })
        .collect()
}

fn decaying_noise(n: usize, seed: u32) -> Vec<f32> {
    noise(n, seed)
        .into_iter()
        .enumerate()
        .map(|(i, v)| v * (-(i as f32) * 5.0 / n as f32).exp())
        .collect()
}

fn prepared(device: &BuiltinDevice, samples: &dyn SampleResolver, block: usize) -> Box<dyn Device> {
    with_params(device, samples, block, &[])
}

/// Created with `params` set, then prepared (the engine's order: the IR is built with them).
fn with_params(
    device: &BuiltinDevice,
    samples: &dyn SampleResolver,
    block: usize,
    params: &[(ParamId, f64)],
) -> Box<dyn Device> {
    let mut d = fx_space::create(device, samples);
    for &(id, v) in params {
        d.set_param(id, v);
    }
    d.prepare(&PrepareConfig {
        sample_rate: SR,
        max_block_size: block,
        max_events_per_block: 64,
    });
    d
}

/// Process stereo `input` in blocks of `block`, with `events` (absolute frame, param,
/// value) applied at their offsets; returns the stereo output.
fn render(
    d: &mut dyn Device,
    input: [&[f32]; 2],
    block: usize,
    events: &[(usize, ParamId, f64)],
) -> [Vec<f32>; 2] {
    let n = input[0].len();
    let mut out = [vec![0.0f32; n], vec![0.0f32; n]];
    let transport = TransportInfo {
        playing: true,
        ..TransportInfo::STOPPED
    };
    let mut out_events = EventBuffer::with_capacity(64);
    let mut evs: Vec<ProcessEvent> = Vec::with_capacity(64);
    let mut i = 0;
    while i < n {
        let m = block.min(n - i);
        evs.clear();
        for &(at, param, value) in events {
            if at >= i && at < i + m {
                evs.push(ProcessEvent {
                    offset: (at - i) as u32,
                    kind: EventKind::Param { param, value },
                });
            }
        }
        let ins: [&[f32]; 2] = [&input[0][i..i + m], &input[1][i..i + m]];
        let (l, r) = out.split_at_mut(1);
        let mut outs: [&mut [f32]; 2] = [&mut l[0][i..i + m], &mut r[0][i..i + m]];
        let mut ctx = ProcessContext {
            sample_rate: SR,
            frames: m,
            transport: &transport,
            events: &evs,
            out_events: &mut out_events,
        };
        let mut buffers = AudioBuffers {
            inputs: &ins,
            outputs: &mut outs,
        };
        assert_no_alloc(|| {
            d.process(&mut ctx, &mut buffers);
        });
        i += m;
    }
    out
}

fn wet_only(d: &mut dyn Device) {
    d.set_param(p::MIX, 100.0);
}

fn energy(x: &[f32]) -> f64 {
    x.iter().map(|&v| v as f64 * v as f64).sum()
}

/// Largest |second difference| (clicks show up as spikes).
fn max_d2(x: &[f32]) -> f32 {
    x.windows(3)
        .fold(0.0f32, |m, w| m.max((w[2] - 2.0 * w[1] + w[0]).abs()))
}

#[test]
fn impulse_gives_the_normalized_ir_at_zero_latency() {
    let ir = decaying_noise(20_000, 1);
    let src = One(Arc::new(Mem(vec![ir.clone(), ir.clone()])));
    let mut d = prepared(&media_reverb(), &src, BLOCK);
    wet_only(&mut *d);
    let mut x = vec![0.0f32; 30_000];
    x[0] = 1.0;
    let [l, r] = render(&mut *d, [&x, &x], BLOCK, &[]);
    let g = 1.0 / energy(&ir).sqrt() as f32;
    for (i, &h) in ir.iter().enumerate() {
        assert!((l[i] - h * g).abs() < 1e-5, "sample {i}: {} vs {}", l[i], h * g);
        assert!((r[i] - h * g).abs() < 1e-5);
    }
    assert!(l[0].abs() > 0.0, "zero latency");
    assert!(l[20_000..].iter().all(|v| v.abs() < 1e-6));
    assert_eq!(d.latency(), 0);
}

#[test]
fn block_size_does_not_change_the_output() {
    let x = noise(40_000, 9);
    let mut outs = Vec::new();
    for block in [1usize, 64, 333, 1024] {
        let mut d = prepared(&factory("chamber"), &NoSamples, block);
        outs.push(render(&mut *d, [&x, &x], block, &[(5000, p::MIX, 70.0)])[0].clone());
    }
    // Same up to float reassociation and the event landing in different blocks.
    for o in &outs[1..] {
        let err = o.iter().zip(&outs[0]).fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(err < 1e-3, "err {err}");
    }
}

#[test]
fn pre_delay_shifts_the_wet_onset() {
    let ir = vec![1.0f32];
    let src = One(Arc::new(Mem(vec![ir])));
    let mut d = prepared(&media_reverb(), &src, BLOCK);
    wet_only(&mut *d);
    d.set_param(p::PRE_DELAY, 10.0);
    let mut x = vec![0.0f32; 2000];
    x[0] = 1.0;
    let [l, _] = render(&mut *d, [&x, &x], BLOCK, &[]);
    let peak = l
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .unwrap()
        .0;
    assert_eq!(peak, 480, "10 ms at 48 kHz");
}

#[test]
fn decay_shortens_and_reverse_flips_the_tail() {
    let x = {
        let mut x = vec![0.0f32; 4 * 48_000];
        x[0] = 1.0;
        x
    };
    let tail = |decay: f64, reverse: bool| {
        let mut d = with_params(
            &factory("hall"),
            &NoSamples,
            BLOCK,
            &[
                (p::MIX, 100.0),
                (p::DECAY, decay),
                (p::REVERSE, if reverse { 1.0 } else { 0.0 }),
            ],
        );
        render(&mut *d, [&x, &x], BLOCK, &[])[0].clone()
    };
    let full = tail(100.0, false);
    let short = tail(30.0, false);
    let rev = tail(100.0, true);
    let len = (2.8 * 48_000.0) as usize;
    // Decay 30 %: nothing after 30 % of the IR.
    assert!(energy(&short[len * 3 / 10 + 10..]) < 1e-9);
    assert!(energy(&full[len * 3 / 10 + 10..len]) > 1e-4);
    // Reverse: the energy builds up towards the end.
    let q = len / 4;
    assert!(energy(&rev[3 * q..len]) > 10.0 * energy(&rev[..q]));
    assert!(energy(&full[..q]) > 10.0 * energy(&full[3 * q..len]));
}

#[test]
fn missing_ir_passes_the_dry_signal_through() {
    let x = noise(4096, 3);
    for device in [
        media_reverb(),
        BuiltinDevice::ConvolutionReverb { ir: None },
        factory("not-a-factory-ir"),
    ] {
        // `NoSamples`: the media isn't loaded (or is missing).
        let mut d = prepared(&device, &NoSamples, BLOCK);
        let [l, r] = render(&mut *d, [&x, &x], BLOCK, &[]);
        assert_eq!(l, x, "{device:?}");
        assert_eq!(r, x);
    }
}

#[test]
fn extreme_params_stay_finite() {
    let x = noise(48_000, 4);
    for (id, lo, hi) in [
        (p::MIX, 0.0, 100.0),
        (p::PRE_DELAY, 0.0, 250.0),
        (p::DECAY, 10.0, 100.0),
        (p::SIZE, 50.0, 150.0),
        (p::LOW_CUT, 20.0, 2000.0),
        (p::HIGH_CUT, 1000.0, 20000.0),
        (p::WIDTH, 0.0, 200.0),
        (p::GAIN, -24.0, 24.0),
        (p::REVERSE, 0.0, 1.0),
    ] {
        for v in [lo, hi, f64::NAN, 1e9, -1e9] {
            let mut d = prepared(&factory("plate"), &NoSamples, BLOCK);
            let [l, r] = render(&mut *d, [&x, &x], BLOCK, &[(100, id, v), (30_000, id, lo)]);
            assert!(l.iter().chain(&r).all(|s| s.is_finite()), "{id:?} = {v}");
            let stored = d.param(id).unwrap();
            assert!(stored >= lo && stored <= hi, "{id:?} clamped: {stored}");
        }
    }
}

#[test]
fn mix_width_and_gain() {
    let x = noise(20_000, 6);
    let run = |events: &[(usize, ParamId, f64)]| {
        let mut d = prepared(&factory("room"), &NoSamples, BLOCK);
        for &(_, id, v) in events {
            d.set_param(id, v);
        }
        render(&mut *d, [&x, &x], BLOCK, &[])
    };
    // Mix 0: dry.
    let [l, _] = run(&[(0, p::MIX, 0.0)]);
    assert_eq!(l, x);
    // Width 0: mono wet.
    let [l, r] = run(&[(0, p::MIX, 100.0), (0, p::WIDTH, 0.0)]);
    assert!(l.iter().zip(&r).all(|(a, b)| (a - b).abs() < 1e-6));
    // Width 100: decorrelated stereo IR.
    let [l, r] = run(&[(0, p::MIX, 100.0)]);
    assert!(l.iter().zip(&r).any(|(a, b)| (a - b).abs() > 1e-3));
    // Gain +6 dB doubles the wet.
    let [a, _] = run(&[(0, p::MIX, 100.0)]);
    let [b, _] = run(&[(0, p::MIX, 100.0), (0, p::GAIN, 6.0206)]);
    let ratio = (energy(&b) / energy(&a)).sqrt();
    assert!((ratio - 2.0).abs() < 1e-3, "ratio {ratio}");
}

#[test]
fn low_cut_removes_lows_from_the_wet() {
    let sine = |hz: f32| -> Vec<f32> {
        (0..48_000)
            .map(|i| (std::f32::consts::TAU * hz * i as f32 / SR).sin() * 0.5)
            .collect()
    };
    let x = sine(60.0);
    let level = |low: f64| {
        let mut d = prepared(&factory("chamber"), &NoSamples, BLOCK);
        wet_only(&mut *d);
        d.set_param(p::LOW_CUT, low);
        let [l, _] = render(&mut *d, [&x, &x], BLOCK, &[]);
        energy(&l[24_000..])
    };
    assert!(level(1000.0) < 0.01 * level(20.0));
}

/// A steady sine through the reverb; at `at` a new IR (`set_data`) or a shaping change.
fn swap_click_ratio(swap: impl FnOnce(&mut dyn Device)) -> f32 {
    let n = 5 * 48_000;
    let x: Vec<f32> = (0..n)
        .map(|i| (std::f32::consts::TAU * 220.0 * i as f32 / SR).sin() * 0.5)
        .collect();
    let mut d = prepared(&factory("room"), &NoSamples, BLOCK);
    d.set_param(p::MIX, 50.0);
    let at = 2 * 48_000;
    let [before, _] = render(&mut *d, [&x[..at], &x[..at]], BLOCK, &[]);
    swap(&mut *d);
    let [after, _] = render(&mut *d, [&x[at..], &x[at..]], BLOCK, &[]);
    // Steady states before and after (a swap to no IR doubles the dry level at mix 50 %).
    let steady = max_d2(&before[48_000..]).max(max_d2(&after[after.len() - 48_000..]));
    max_d2(&after) / steady
}

#[test]
fn ir_swap_crossfades_without_a_click() {
    let next = fx_space::ir_swap(&factory("hall"), &NoSamples, SR).unwrap();
    let mut next = Some(next);
    let ratio = swap_click_ratio(|d| {
        let back = assert_no_alloc(|| d.set_data(next.take().unwrap()));
        drop(back);
    });
    assert!(ratio < 1.5, "swap click ratio {ratio}");
}

#[test]
fn ir_swap_to_none_fades_to_dry() {
    let next = fx_space::ir_swap(&BuiltinDevice::ConvolutionReverb { ir: None }, &NoSamples, SR);
    let mut next = next;
    let ratio = swap_click_ratio(|d| {
        drop(assert_no_alloc(|| d.set_data(next.take().unwrap())));
    });
    assert!(ratio < 1.5, "click ratio {ratio}");
}

#[test]
fn shaping_change_crossfades_without_a_click() {
    for (id, v) in [(p::DECAY, 40.0), (p::SIZE, 150.0), (p::REVERSE, 1.0)] {
        let ratio = swap_click_ratio(|d| d.set_param(id, v));
        assert!(ratio < 1.5, "{id:?}: click ratio {ratio}");
    }
}

#[test]
fn swapped_ir_matches_a_fresh_reverb_once_settled() {
    // After a swap (with non-default shaping) the reverb sounds like one built with that IR.
    let x = noise(6 * 48_000, 8);
    let mut d = prepared(&factory("room"), &NoSamples, BLOCK);
    wet_only(&mut *d);
    d.set_param(p::SIZE, 120.0);
    let quiet = vec![0.0f32; 48_000];
    render(&mut *d, [&quiet, &quiet], BLOCK, &[]);
    let swap = fx_space::ir_swap(&factory("plate"), &NoSamples, SR).unwrap();
    drop(d.set_data(swap));
    // Let the swap settle on silence, then feed both the same signal.
    render(&mut *d, [&quiet, &quiet], BLOCK, &[]);
    let mut fresh = with_params(
        &factory("plate"),
        &NoSamples,
        BLOCK,
        &[(p::MIX, 100.0), (p::SIZE, 120.0)],
    );
    let [a, _] = render(&mut *d, [&x, &x], BLOCK, &[]);
    let [b, _] = render(&mut *fresh, [&x, &x], BLOCK, &[]);
    let err = a.iter().zip(&b).fold(0.0f32, |m, (p, q)| m.max((p - q).abs()));
    assert!(err < 1e-3, "err {err}");
}

#[test]
fn set_data_rejects_foreign_payloads_and_hands_back_old_convolvers() {
    let mut d = prepared(&factory("room"), &NoSamples, BLOCK);
    let back = d.set_data(Box::new(42u32)).unwrap();
    assert_eq!(*back.downcast::<u32>().unwrap(), 42);
    let x = vec![0.0f32; 48_000];
    // Two swaps in a row: the first is fully replaced and comes back with the second.
    let a = fx_space::ir_swap(&factory("hall"), &NoSamples, SR).unwrap();
    drop(d.set_data(a));
    render(&mut *d, [&x, &x], BLOCK, &[]);
    let b = fx_space::ir_swap(&factory("plate"), &NoSamples, SR).unwrap();
    let back = d.set_data(b).unwrap();
    let back = back.downcast::<fx_space::IrSwap>().unwrap();
    assert_eq!(back.replaced_count(), 1, "the room convolver, retired after the first swap");
}

#[test]
fn every_factory_ir_builds_and_sounds() {
    let mut x = vec![0.0f32; 6 * 48_000];
    x[0] = 1.0;
    for spec in fx_space::FACTORY_IRS {
        for sr in [44_100.0f32, 48_000.0, 96_000.0] {
            let mut d = fx_space::create(&factory(spec.id), &NoSamples);
            d.prepare(&PrepareConfig {
                sample_rate: sr,
                max_block_size: 512,
                max_events_per_block: 64,
            });
            d.set_param(p::MIX, 100.0);
            let [l, r] = render(&mut *d, [&x, &x], 512, &[]);
            let len = (spec.length_seconds * sr as f64) as usize;
            assert!(energy(&l[..len.min(l.len())]) > 0.1, "{} @ {sr}", spec.id);
            assert!(l.iter().chain(&r).all(|v| v.is_finite()));
        }
    }
}

/// CPU of the reverb (wet path, stereo) for 1 s and 5 s IRs at 48 kHz.
#[test]
#[ignore = "benchmark: run in release with --ignored --nocapture"]
fn cpu() {
    for seconds in [1.0f64, 5.0] {
        for block in [64usize, 128, 512] {
            let ir = decaying_noise((seconds * 48_000.0) as usize, 11);
            let src = One(Arc::new(Mem(vec![ir.clone(), ir])));
            let mut d = prepared(&media_reverb(), &src, block);
            d.set_param(p::MIX, 50.0);
            let x = noise(48_000 * 20, 12);
            let mut times = Vec::with_capacity(x.len() / block);
            let mut i = 0;
            let transport = TransportInfo {
                playing: true,
                ..TransportInfo::STOPPED
            };
            let mut ev = EventBuffer::with_capacity(4);
            let mut out = [vec![0.0f32; block], vec![0.0f32; block]];
            while i + block <= x.len() {
                let ins: [&[f32]; 2] = [&x[i..i + block], &x[i..i + block]];
                let (l, r) = out.split_at_mut(1);
                let mut outs: [&mut [f32]; 2] = [&mut l[0], &mut r[0]];
                let mut ctx = ProcessContext {
                    sample_rate: SR,
                    frames: block,
                    transport: &transport,
                    events: &[],
                    out_events: &mut ev,
                };
                let b0 = std::time::Instant::now();
                d.process(
                    &mut ctx,
                    &mut AudioBuffers {
                        inputs: &ins,
                        outputs: &mut outs,
                    },
                );
                times.push(b0.elapsed().as_secs_f64());
                i += block;
            }
            let total: f64 = times.iter().sum();
            let budget = block as f64 / 48_000.0;
            times.sort_by(f64::total_cmp);
            let pct = |q: f64| times[((times.len() - 1) as f64 * q) as usize] / budget * 100.0;
            println!(
                "IR {seconds} s, block {block}: avg {:.2} % of one core; per block p99 {:.1} %, p99.9 {:.1} %, max {:.1} % of its budget",
                100.0 * total / (x.len() as f64 / 48_000.0),
                pct(0.99),
                pct(0.999),
                pct(1.0),
            );
        }
    }
    let conv = fx_space::Convolver::new(
        fx_space::IrInput::Factory(4),
        fx_space::IrBase::load(&fx_space::IrInput::Factory(4), 48_000.0).unwrap(),
        fx_space::Shaping::default(),
    );
    println!("5 s factory IR: {:.1} MB", conv.memory_bytes() as f64 / 1e6);
}

//! The Reverb's shimmer (pitch-shifted feedback): it shifts the tail by its pitch, stays
//! bounded and decays at every extreme (20 s decay, no damping, full amount, any pitch),
//! never produces NaN or subnormal output, never allocates, renders the same at every block
//! size, and switched off is the plain reverb (bit for bit, `effects_defaults.rs` too).

use std::f32::consts::PI;

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::model::ParamId;
use ether_core::{
    AudioBuffers, Device, EventBuffer, EventKind, Node, PrepareConfig, ProcessContext,
    ProcessEvent, TransportInfo,
};
use ether_devices::Reverb;
use ether_devices::reverb::params as p;

const SR: f32 = 48_000.0;
const S: usize = SR as usize;

fn reverb(params: &[(ParamId, f64)], max_block: usize) -> Reverb {
    let mut r = Reverb::new();
    r.prepare(&PrepareConfig {
        sample_rate: SR,
        max_block_size: max_block,
        max_events_per_block: 64,
    });
    r.set_param(p::MIX, 100.0);
    r.set_param(p::PRE_DELAY, 0.0);
    for &(id, v) in params {
        r.set_param(id, v);
    }
    r
}

/// Render `x` (mono into both inputs) in blocks of `block`, with `events` at absolute
/// sample positions; every `process` under `assert_no_alloc`.
fn render(
    r: &mut Reverb,
    x: &[f32],
    block: usize,
    events: &[(usize, ParamId, f64)],
) -> (Vec<f32>, Vec<f32>) {
    let frames = x.len();
    let (mut l, mut rr) = (vec![0.0f32; frames], vec![0.0f32; frames]);
    let mut out_events = EventBuffer::with_capacity(8);
    let mut evs: Vec<ProcessEvent> = Vec::with_capacity(events.len());
    let mut pos = 0;
    while pos < frames {
        let n = block.min(frames - pos);
        evs.clear();
        evs.extend(events.iter().filter(|e| e.0 >= pos && e.0 < pos + n).map(
            |&(at, param, value)| ProcessEvent {
                offset: (at - pos) as u32,
                kind: EventKind::Param { param, value },
            },
        ));
        let inputs: [&[f32]; 2] = [&x[pos..pos + n], &x[pos..pos + n]];
        let mut outputs: [&mut [f32]; 2] = [&mut l[pos..pos + n], &mut rr[pos..pos + n]];
        let transport = TransportInfo::STOPPED;
        let mut ctx = ProcessContext {
            sample_rate: SR,
            frames: n,
            transport: &transport,
            events: &evs,
            out_events: &mut out_events,
        };
        let mut audio = AudioBuffers {
            inputs: &inputs,
            outputs: &mut outputs,
        };
        assert_no_alloc(|| {
            r.process(&mut ctx, &mut audio);
        });
        pos += n;
    }
    (l, rr)
}

/// Deterministic white noise (-1..1).
fn noise(frames: usize, seed: u32) -> Vec<f32> {
    let mut s = seed.max(1);
    (0..frames)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s as f32 / u32::MAX as f32) * 2.0 - 1.0
        })
        .collect()
}

/// `burst` samples of `x` then silence, `total` samples in all.
fn burst(x: Vec<f32>, total: usize) -> Vec<f32> {
    let mut v = x;
    v.resize(total, 0.0);
    v
}

fn sine(freq: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| 0.5 * (2.0 * PI * freq * i as f32 / SR).sin())
        .collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

/// Power of `x` at `freq` (Goertzel, Hann window).
fn power_at(x: &[f32], freq: f32) -> f32 {
    let n = x.len() as f32;
    let w = 2.0 * PI * freq / SR;
    let coef = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f32, 0.0f32);
    for (i, &v) in x.iter().enumerate() {
        let win = 0.5 - 0.5 * (2.0 * PI * i as f32 / n).cos();
        let s0 = v * win + coef * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    (s1 * s1 + s2 * s2 - coef * s1 * s2) / (n * n)
}

/// The shimmer at its extremes.
fn max_feedback(pitch: f64) -> Vec<(ParamId, f64)> {
    vec![
        (p::DECAY, 20.0),
        (p::DAMPING, 0.0),
        (p::SIZE, 100.0),
        (p::DIFFUSION, 100.0),
        (p::SHIMMER, 1.0),
        (p::SHIMMER_AMOUNT, 100.0),
        (p::SHIMMER_PITCH, pitch),
        (p::SHIMMER_TONE, 20_000.0),
    ]
}

#[test]
fn shimmer_at_max_feedback_stays_bounded_and_decays() {
    for pitch in [12.0, 7.0, 19.0, -12.0, 24.0, -24.0, 0.0] {
        let mut r = reverb(&max_feedback(pitch), 256);
        // A second of full-scale noise, then 24 s of tail.
        let x = burst(noise(S, 7), 25 * S);
        let (l, rr) = render(&mut r, &x, 256, &[]);
        for ch in [&l, &rr] {
            assert!(ch.iter().all(|v| v.is_finite()), "{pitch}: not finite");
            assert!(peak(ch) < 4.0, "{pitch}: peak {}", peak(ch));
            let early = rms(&ch[S..3 * S]);
            let late = rms(&ch[23 * S..25 * S]);
            // A 20 s decay alone loses 66 dB over 22 s; the tail must at least fade.
            assert!(late < early * 0.5, "{pitch}: tail grows {early} → {late}");
            assert!(ch.iter().all(|v| *v == 0.0 || v.abs() >= f32::MIN_POSITIVE));
        }
    }
}

#[test]
fn shimmer_with_freeze_holds_and_never_runs_away() {
    let mut params = max_feedback(12.0);
    params.push((p::FREEZE, 0.0));
    let mut r = reverb(&params, 128);
    let x = burst(noise(S / 2, 3), 12 * S);
    let (l, _) = render(&mut r, &x, 128, &[(S, p::FREEZE, 1.0)]);
    assert!(l.iter().all(|v| v.is_finite()));
    assert!(peak(&l) < 4.0, "peak {}", peak(&l));
    // Frozen: the tail holds (neither dies nor grows).
    let (a, b) = (rms(&l[3 * S..4 * S]), rms(&l[11 * S..12 * S]));
    assert!(b > a * 0.25 && b < a * 2.0, "{a} → {b}");
}

#[test]
fn shimmer_shifts_the_tail_by_its_pitch() {
    // A 440 Hz tone into a long reverb: with shimmer +12 the tail gains 880 Hz (and 1760);
    // without, almost nothing there.
    let x = burst(sine(440.0, S / 2), 4 * S);
    let base = [(p::DECAY, 6.0), (p::DAMPING, 20.0)];
    let mut plain = reverb(&base, 256);
    let (pl, _) = render(&mut plain, &x, 256, &[]);
    let mut shim = base.to_vec();
    shim.extend([(p::SHIMMER, 1.0), (p::SHIMMER_AMOUNT, 80.0)]);
    let mut r = reverb(&shim, 256);
    let (sl, _) = render(&mut r, &x, 256, &[]);
    let tail = |v: &[f32]| v[2 * S..3 * S].to_vec();
    let ratio = |v: &[f32]| power_at(&tail(v), 880.0) / power_at(&tail(v), 440.0);
    assert!(
        ratio(&sl) > 100.0 * ratio(&pl),
        "octave {} vs {}",
        ratio(&sl),
        ratio(&pl)
    );
    assert!(ratio(&sl) > 0.05, "octave too quiet: {}", ratio(&sl));
    // -12: the octave below.
    let mut down = shim.clone();
    down.push((p::SHIMMER_PITCH, -12.0));
    let mut r = reverb(&down, 256);
    let (dl, _) = render(&mut r, &x, 256, &[]);
    let below = |v: &[f32]| power_at(&tail(v), 220.0) / power_at(&tail(v), 440.0);
    assert!(
        below(&dl) > 100.0 * below(&pl),
        "{} vs {}",
        below(&dl),
        below(&pl)
    );
}

#[test]
fn shimmer_renders_the_same_at_every_block_size() {
    let x = burst(noise(S / 4, 11), 2 * S);
    let events = [
        (1_000, p::SHIMMER, 1.0),
        (9_001, p::SHIMMER_PITCH, 7.0),
        (20_000, p::SHIMMER_TONE, 3000.0),
        (40_003, p::SHIMMER_AMOUNT, 90.0),
        (70_000, p::SHIMMER, 0.0),
        (80_000, p::SHIMMER, 1.0),
    ];
    let reference = render(&mut reverb(&[], 512), &x, 512, &events);
    for block in [1, 7, 64, 333] {
        let out = render(&mut reverb(&[], 512), &x, block, &events);
        assert!(out == reference, "block {block} differs");
    }
}

#[test]
fn shimmer_off_is_the_plain_reverb() {
    // 0.1 s of silence, then noise.
    let mut x = vec![0.0; S / 10];
    x.extend(burst(noise(S / 4, 5), 2 * S));
    let plain = render(&mut reverb(&[], 256), &x, 256, &[]);
    // Amount, pitch and tone without the switch change nothing.
    let set = [
        (p::SHIMMER_AMOUNT, 100.0),
        (p::SHIMMER_PITCH, 7.0),
        (p::SHIMMER_TONE, 1000.0),
    ];
    assert!(render(&mut reverb(&set, 256), &x, 256, &[]) == plain);
    // Switched on, then off before any signal (faded out within the silence): the plain
    // path again, bit for bit.
    let toggled = render(
        &mut reverb(&[], 256),
        &x,
        256,
        &[(0, p::SHIMMER, 1.0), (1, p::SHIMMER, 0.0)],
    );
    assert!(toggled == plain);
}

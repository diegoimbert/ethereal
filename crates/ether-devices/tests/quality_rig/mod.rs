//! Shared rig of the sound-quality tests: offline rendering of any built-in device
//! and the objective measurements (aliasing, clicks, zipper noise, DC, levels, reverb
//! density / smoothness, CPU). Used by `sound_quality.rs` (regression tests) and its
//! `audit` report (`cargo test --release -p ether-devices --test it sound_quality:: -- --ignored
//! --nocapture`).

#![allow(dead_code)]

use std::collections::HashMap;
use std::f64::consts::PI;
use std::sync::Arc;
use std::time::Instant;

use ether_core::protocol::model::{
    BuiltinDevice, BuiltinDeviceType, MediaId, ParamId, SampleZone, Ulid, Zone,
};
use ether_core::{
    AudioBuffers, AudioSource, Device, EventBuffer, EventKind, PrepareConfig, ProcessContext,
    ProcessEvent, TransportInfo,
};
use ether_devices::{SampleResolver, create};
use realfft::RealFftPlanner;

pub const SR: f32 = 48_000.0;
pub const BLOCK: usize = 256;
pub const S: usize = SR as usize;

// --- test samples -------------------------------------------------------------------------

/// Sample media of the sampler tests: a band-limited saw and a sine, both at C4.
pub const SAW_SAMPLE: u128 = 1;
pub const SINE_SAMPLE: u128 = 2;
pub const C4_HZ: f64 = 261.625_565_300_598_6;

struct VecSource(Vec<f32>);

impl AudioSource for VecSource {
    fn channels(&self) -> u16 {
        1
    }
    fn frames(&self) -> u64 {
        self.0.len() as u64
    }
    fn read(&self, _channel: u16, start: u64, out: &mut [f32]) -> bool {
        for (k, o) in out.iter_mut().enumerate() {
            *o = self.0.get(start as usize + k).copied().unwrap_or(0.0);
        }
        true
    }
}

/// An additive saw at `hz` with every harmonic below 20 kHz (no aliasing in the sample).
pub fn bl_saw(hz: f64, amp: f64, frames: usize) -> Vec<f32> {
    let harmonics = (20_000.0 / hz) as usize;
    let w = 2.0 * PI * hz / f64::from(SR);
    (0..frames)
        .map(|i| {
            let mut s = 0.0;
            for k in 1..=harmonics {
                s += (w * k as f64 * i as f64).sin() / k as f64;
            }
            (amp * s * 2.0 / PI) as f32
        })
        .collect()
}

pub fn sine(hz: f64, amp: f64, frames: usize) -> Vec<f32> {
    let w = 2.0 * PI * hz / f64::from(SR);
    (0..frames)
        .map(|i| (amp * (w * i as f64).sin()) as f32)
        .collect()
}

pub struct Samples(HashMap<MediaId, Arc<dyn AudioSource>>);

impl Samples {
    pub fn new() -> Self {
        let len = 8 * S;
        let mut m: HashMap<MediaId, Arc<dyn AudioSource>> = HashMap::new();
        m.insert(
            media(SAW_SAMPLE),
            Arc::new(VecSource(bl_saw(C4_HZ, 0.5, len))),
        );
        m.insert(
            media(SINE_SAMPLE),
            Arc::new(VecSource(sine(C4_HZ, 0.5, len))),
        );
        Self(m)
    }
}

impl SampleResolver for Samples {
    fn resolve(&self, media: MediaId) -> Option<Arc<dyn AudioSource>> {
        self.0.get(&media).cloned()
    }
}

pub fn media(n: u128) -> MediaId {
    MediaId(Ulid(n))
}

/// The document form of a built-in type; samplers get `sample` (multisampler: one zone over
/// the whole keyboard, root C4).
pub fn builtin(ty: BuiltinDeviceType, sample: u128) -> BuiltinDevice {
    let mut d = BuiltinDevice::new(ty);
    match &mut d {
        BuiltinDevice::Sampler { sample: s, .. } => *s = Some(media(sample)),
        BuiltinDevice::MultiSampler { zones } => {
            zones.push(SampleZone {
                media: Some(media(sample)),
                root_key: 60,
                keys: Zone { lo: 0, hi: 127 },
                velocities: Zone { lo: 1, hi: 127 },
                ..SampleZone::default()
            });
        }
        _ => {}
    }
    d
}

/// A prepared device of type `ty` (samplers play `sample`), `params` applied.
pub fn make_with(ty: BuiltinDeviceType, sample: u128, params: &[(u32, f64)]) -> Box<dyn Device> {
    let mut d = create(&builtin(ty, sample), &Samples::new());
    d.prepare(&PrepareConfig {
        sample_rate: SR,
        max_block_size: BLOCK,
        max_events_per_block: 64,
    });
    for &(p, v) in params {
        d.set_param(ParamId(p), v);
    }
    d
}

pub fn make(ty: BuiltinDeviceType, params: &[(u32, f64)]) -> Box<dyn Device> {
    make_with(ty, SAW_SAMPLE, params)
}

// --- rendering ----------------------------------------------------------------------------

pub fn on(key: u8, velocity: f32) -> EventKind {
    EventKind::NoteOn {
        note_id: u32::from(key),
        channel: 0,
        key,
        velocity,
    }
}

pub fn off(key: u8) -> EventKind {
    EventKind::NoteOff {
        note_id: u32::from(key),
        channel: 0,
        key,
        velocity: 0.0,
    }
}

pub fn param(id: u32, value: f64) -> EventKind {
    EventKind::Param {
        param: ParamId(id),
        value,
    }
}

pub struct Render {
    pub out: [Vec<f32>; 2],
    /// Wall-clock seconds spent in each block's `process`.
    pub block_secs: Vec<f64>,
}

impl Render {
    pub fn mono(&self) -> Vec<f32> {
        self.out[0]
            .iter()
            .zip(&self.out[1])
            .map(|(l, r)| 0.5 * (l + r))
            .collect()
    }

    /// Share of real time spent processing blocks in `[from, to)` seconds of output.
    pub fn cpu(&self, from: f64, to: f64) -> f64 {
        let b0 = (from * f64::from(SR)) as usize / BLOCK;
        let b1 = ((to * f64::from(SR)) as usize / BLOCK).min(self.block_secs.len());
        let spent: f64 = self.block_secs[b0..b1].iter().sum();
        spent / ((b1 - b0) * BLOCK) as f64 * f64::from(SR)
    }
}

/// Render `frames` of `d` from `input` (stereo, may be shorter: zero-padded) with timed
/// `events` (`(frame, kind)`, sorted).
pub fn render(
    d: &mut dyn Device,
    input: &[Vec<f32>; 2],
    events: &[(usize, EventKind)],
    frames: usize,
) -> Render {
    let mut out = [vec![0.0f32; frames], vec![0.0f32; frames]];
    let mut block_secs = Vec::with_capacity(frames / BLOCK + 1);
    let mut ins = [vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]];
    let mut out_events = EventBuffer::with_capacity(256);
    let mut block_events: Vec<ProcessEvent> = Vec::with_capacity(64);
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
        let bps = 2.0 / f64::from(SR);
        let transport = TransportInfo {
            playing: true,
            sample_time: pos as u64,
            position: pos as f64 * bps,
            seconds: pos as f64 / f64::from(SR),
            bpm: 120.0,
            beats_per_sample: bps,
            ..TransportInfo::STOPPED
        };
        for (buf, src) in ins.iter_mut().zip(input) {
            for (k, v) in buf[..n].iter_mut().enumerate() {
                *v = src.get(pos + k).copied().unwrap_or(0.0);
            }
        }
        let inputs: [&[f32]; 2] = [&ins[0][..n], &ins[1][..n]];
        let (l, r) = out.split_at_mut(1);
        let mut outputs: [&mut [f32]; 2] = [&mut l[0][pos..pos + n], &mut r[0][pos..pos + n]];
        out_events.clear();
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
        let t = Instant::now();
        d.process(&mut ctx, &mut audio);
        block_secs.push(t.elapsed().as_secs_f64());
        pos += n;
    }
    Render { out, block_secs }
}

pub fn silence() -> [Vec<f32>; 2] {
    [Vec::new(), Vec::new()]
}

pub fn stereo(x: Vec<f32>) -> [Vec<f32>; 2] {
    [x.clone(), x]
}

/// A mix-like test signal (`secs` long, peak about -6 dBFS): a band-limited saw chord, a
/// kick (decaying pitched sine) every half second and noise hats every quarter.
pub fn program(secs: f64) -> [Vec<f32>; 2] {
    let n = (secs * f64::from(SR)) as usize;
    let chord: Vec<Vec<f32>> = [110.0, 138.59, 164.81]
        .iter()
        .map(|&hz| bl_saw(hz, 0.08, n))
        .collect();
    let mut rng = 0x1234_5678u32;
    let mut l = vec![0.0f32; n];
    let mut r = vec![0.0f32; n];
    for i in 0..n {
        let t = i as f64 / f64::from(SR);
        let beat = t % 0.5;
        let kick = (2.0 * PI * (50.0 * beat + 40.0 * (1.0 - (-beat * 30.0).exp()))).sin()
            * (-beat * 12.0).exp()
            * 0.35;
        let q = t % 0.25;
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        let noise = (rng as f64 / f64::from(u32::MAX) * 2.0 - 1.0) * (-q * 60.0).exp() * 0.08;
        let c = f64::from(chord[0][i] + chord[1][i] + chord[2][i]);
        l[i] = (kick + noise + c) as f32;
        r[i] = (kick - 0.5 * noise + c * 0.9) as f32;
    }
    [l, r]
}

// --- measurements -------------------------------------------------------------------------

pub fn db(x: f64) -> f64 {
    20.0 * x.max(1e-12).log10()
}

pub fn pow_db(x: f64) -> f64 {
    10.0 * x.max(1e-24).log10()
}

pub fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|&v| f64::from(v) * f64::from(v)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

pub fn peak(x: &[f32]) -> f64 {
    x.iter().fold(0.0f64, |m, &v| m.max(f64::from(v).abs()))
}

pub fn mean(x: &[f32]) -> f64 {
    x.iter().map(|&v| f64::from(v)).sum::<f64>() / x.len().max(1) as f64
}

pub fn finite(x: &[f32]) -> bool {
    x.iter().all(|v| v.is_finite())
}

pub fn has_subnormals(x: &[f32]) -> bool {
    x.iter().any(|v| v.is_subnormal())
}

pub fn secs(t: f64) -> usize {
    (t * f64::from(SR)) as usize
}

/// Power spectrum (bins `0..=n/2`) of `x` under a 4-term Blackman-Harris window (sidelobes
/// at -92 dB), normalized so a full-scale sine's bins sum to about 0.5.
pub fn spectrum(x: &[f32]) -> Vec<f64> {
    let n = x.len();
    let mut planner = RealFftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(n);
    let mut input = fft.make_input_vec();
    let (a0, a1, a2, a3) = (0.35875, 0.48829, 0.14128, 0.01168);
    let mut wsum = 0.0;
    for (i, v) in input.iter_mut().enumerate() {
        let p = 2.0 * PI * i as f64 / n as f64;
        let w = a0 - a1 * p.cos() + a2 * (2.0 * p).cos() - a3 * (3.0 * p).cos();
        wsum += w * w;
        *v = f64::from(x[i]) * w;
    }
    let mut out = fft.make_output_vec();
    fft.process(&mut input, &mut out).unwrap();
    out.iter()
        .map(|c| c.norm_sqr() * 2.0 / (wsum * n as f64))
        .collect()
}

pub fn bin_hz(n: usize) -> f64 {
    f64::from(SR) / n as f64
}

/// Aliasing of a periodic signal of fundamental `f0`: power in the bins below `max_hz` that
/// are not near DC or a harmonic of `f0`, relative to the harmonic power (dB). `x.len()` is
/// a power of two.
pub fn alias_db(x: &[f32], f0: f64, max_hz: f64) -> f64 {
    let n = x.len();
    let spec = spectrum(x);
    let hz = bin_hz(n);
    // Blackman-Harris main lobe: ±4 bins; allow some pitch drift.
    let lobe = 6.0 * hz;
    let mut harm = 0.0;
    let mut alias = 0.0;
    for (b, p) in spec.iter().enumerate() {
        let f = b as f64 * hz;
        let k = (f / f0).round();
        let near_harmonic = k >= 1.0 && (f - k * f0).abs() <= lobe;
        if near_harmonic {
            harm += p;
        } else if f > lobe && f <= max_hz {
            alias += p;
        }
    }
    pow_db(alias / harm.max(1e-30))
}

/// Power above `from_hz` relative to the total (dB).
pub fn hf_db(x: &[f32], from_hz: f64) -> f64 {
    let spec = spectrum(x);
    let hz = bin_hz(x.len());
    let total: f64 = spec.iter().sum();
    let hf: f64 = spec
        .iter()
        .enumerate()
        .filter(|(b, _)| *b as f64 * hz >= from_hz)
        .map(|(_, p)| p)
        .sum();
    pow_db(hf / total.max(1e-30))
}

/// Largest burst of energy above `from_hz` in 1024-frame windows (hop 128) over
/// `[from, to)`, relative to `reference_power` (dB). A sine through a clean envelope or
/// gain change keeps this far below the signal; a click or zipper step is broadband.
pub fn click_db(x: &[f32], from: usize, to: usize, from_hz: f64, reference_power: f64) -> f64 {
    const W: usize = 1024;
    let hz = bin_hz(W);
    let mut worst = 0.0f64;
    let mut start = from;
    while start + W <= to.min(x.len()) {
        let spec = spectrum(&x[start..start + W]);
        let hf: f64 = spec
            .iter()
            .enumerate()
            .filter(|(b, _)| *b as f64 * hz >= from_hz)
            .map(|(_, p)| p)
            .sum();
        worst = worst.max(hf);
        start += 128;
    }
    pow_db(worst / reference_power.max(1e-30))
}

/// Mean power (the sum of the spectrum) of `x`.
pub fn power(x: &[f32]) -> f64 {
    x.iter().map(|&v| f64::from(v) * f64::from(v)).sum::<f64>() / x.len().max(1) as f64
}

/// Schroeder energy-decay curve of an impulse response (dB, 0 at the start).
pub fn edc_db(x: &[f32]) -> Vec<f64> {
    let mut acc = 0.0;
    let mut e: Vec<f64> = x
        .iter()
        .rev()
        .map(|&v| {
            acc += f64::from(v) * f64::from(v);
            acc
        })
        .collect();
    e.reverse();
    let e0 = e[0].max(1e-30);
    e.iter().map(|v| pow_db(v / e0)).collect()
}

/// RT60 (s) extrapolated from the -5..-35 dB part of the energy-decay curve (T30).
pub fn rt60(x: &[f32]) -> Option<f64> {
    let edc = edc_db(x);
    let a = edc.iter().position(|&d| d <= -5.0)?;
    let b = edc.iter().position(|&d| d <= -35.0)?;
    Some((b - a) as f64 / f64::from(SR) * 2.0)
}

/// Normalized echo density (Abel & Huang) of `x` over 20 ms windows starting at `t` s:
/// the share of samples beyond one standard deviation over that of a Gaussian (1 = dense).
pub fn echo_density(x: &[f32], t: f64) -> f64 {
    let w = secs(0.02);
    let s = secs(t);
    let seg = &x[s..(s + w).min(x.len())];
    let sd = rms(seg);
    if sd == 0.0 {
        return 0.0;
    }
    let over = seg.iter().filter(|&&v| f64::from(v).abs() > sd).count();
    over as f64 / seg.len() as f64 / 0.317_310_5
}

/// Roughness of a decaying tail: the standard deviation (dB) of its 10 ms energy envelope
/// around a straight-line fit, from `t0` until the envelope is 40 dB below its start
/// (lumps, flutter and audible modulation raise it; a smooth exponential tail is < 1 dB).
pub fn tail_roughness_db(x: &[f32], t0: f64) -> f64 {
    let w = secs(0.01);
    let mut env = Vec::new();
    let mut i = secs(t0);
    while i + w <= x.len() {
        env.push(pow_db(power(&x[i..i + w])));
        i += w;
    }
    if env.is_empty() {
        return 0.0;
    }
    let start = env[0];
    let end = env
        .iter()
        .position(|&e| e < start - 40.0)
        .unwrap_or(env.len());
    let env = &env[..end.max(4)];
    let n = env.len() as f64;
    let mx = (n - 1.0) / 2.0;
    let my = env.iter().sum::<f64>() / n;
    let mut sxy = 0.0;
    let mut sxx = 0.0;
    for (k, e) in env.iter().enumerate() {
        sxy += (k as f64 - mx) * (e - my);
        sxx += (k as f64 - mx).powi(2);
    }
    let slope = sxy / sxx;
    let var = env
        .iter()
        .enumerate()
        .map(|(k, e)| (e - (my + slope * (k as f64 - mx))).powi(2))
        .sum::<f64>()
        / n;
    var.sqrt()
}

/// Spectral peakiness of a tail segment (dB): how far the strongest narrow peaks stand
/// above the third-octave-smoothed spectrum, 200 Hz..8 kHz (metallic ringing / modes).
pub fn ringing_db(x: &[f32]) -> f64 {
    let spec: Vec<f64> = spectrum(x).iter().map(|p| pow_db(*p)).collect();
    let hz = bin_hz(x.len());
    let mut excess: Vec<f64> = Vec::new();
    let lo = (200.0 / hz) as usize;
    let hi = (8000.0 / hz) as usize;
    for b in lo..hi {
        let f = b as f64 * hz;
        let a = ((f / 2f64.powf(1.0 / 6.0)) / hz) as usize;
        let z = ((f * 2f64.powf(1.0 / 6.0)) / hz) as usize;
        let lin: f64 =
            (a..=z).map(|k| 10f64.powf(spec[k] / 10.0)).sum::<f64>() / (z - a + 1) as f64;
        excess.push(spec[b] - pow_db(lin));
    }
    excess.sort_by(|a, b| a.partial_cmp(b).unwrap());
    // 99.5th percentile: a few strong modes, not single-bin noise peaks.
    excess[(excess.len() as f64 * 0.995) as usize]
}

/// Pearson correlation of two channels.
pub fn correlation(a: &[f32], b: &[f32]) -> f64 {
    let (mut ab, mut aa, mut bb) = (0.0, 0.0, 0.0);
    for (x, y) in a.iter().zip(b) {
        let (x, y) = (f64::from(*x), f64::from(*y));
        ab += x * y;
        aa += x * x;
        bb += y * y;
    }
    ab / (aa * bb).sqrt().max(1e-30)
}

/// 4× oversampled (windowed-sinc) peak of `x`: the inter-sample ("true") peak.
pub fn true_peak(x: &[f32]) -> f64 {
    const TAPS: i64 = 16;
    let mut m = peak(x);
    for i in TAPS as usize..x.len() - TAPS as usize {
        for phase in 1..4 {
            let frac = phase as f64 / 4.0;
            let mut acc = 0.0;
            for k in -TAPS + 1..=TAPS {
                let t = k as f64 - frac;
                let sinc = (PI * t).sin() / (PI * t);
                let w = 0.5 + 0.5 * (PI * t / TAPS as f64).cos();
                acc += f64::from(x[(i as i64 + k) as usize]) * sinc * w;
            }
            m = m.max(acc.abs());
        }
    }
    m
}

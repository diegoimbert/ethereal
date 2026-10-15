//! Built-in effect harness (perf audit #5 / #17). Ignored by default; run in release:
//!
//! ```text
//! cargo test --release -p ether-devices --test perf_devices -- --ignored --nocapture
//! ```
//!
//! - `effect_costs`: ns per stereo sample of every built-in effect on a busy signal, with
//!   its modulation turned up (LFOs, envelope followers, gain reduction), and on silence.
//! - `effect_renders`: renders a fixed program (signal, silence, loud signal, a parameter
//!   change, silence) through every effect. With `ETHER_DEVICE_DUMP=<dir>` it writes them;
//!   with `ETHER_DEVICE_BASE=<dir>` it compares with renders written by another build (max
//!   abs difference per effect: bit-identical or not).

use std::time::Instant;

use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventBuffer, EventKind, PrepareConfig, ProcessContext, ProcessEvent,
    ProcessStatus, TransportInfo,
};
use ether_devices::NoSamples;

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;

/// Effects and the params (group/name = value) that make them work hard.
fn effects() -> Vec<(BuiltinDeviceType, Vec<(&'static str, f64)>)> {
    use BuiltinDeviceType::*;
    vec![
        (Eq, vec![]),
        (Compressor, vec![]),
        (Limiter, vec![]),
        (Delay, vec![]),
        (Reverb, vec![]),
        (Utility, vec![]),
        (Gate, vec![]),
        (MultibandCompressor, vec![]),
        (TransientShaper, vec![]),
        (Chorus, vec![]),
        (Flanger, vec![]),
        (Phaser, vec![]),
        (Tremolo, vec![]),
        (
            AutoFilter,
            vec![
                ("Envelope/Amount", 50.0),
                ("LFO/Amount", 60.0),
                ("Filter/Resonance", 40.0),
            ],
        ),
        (Saturator, vec![]),
        (Bitcrusher, vec![]),
        (ConvolutionReverb, vec![]),
        // devices-pass-effects: the new stages switched on (the defaults above skip them).
        (
            Compressor,
            vec![
                ("Compressor/Knee", 12.0),
                ("Output/Mix", 50.0),
                ("Compressor/Lookahead", 5.0),
            ],
        ),
        (
            Delay,
            vec![
                ("Filter/Low Cut", 200.0),
                ("Filter/High Cut", 5000.0),
                ("Modulation/Mod Depth", 50.0),
            ],
        ),
        (
            Reverb,
            vec![("Filter/Low Cut", 200.0), ("Filter/High Cut", 6000.0)],
        ),
        (
            Utility,
            vec![
                ("Stereo/Bass Mono", 1.0),
                ("Utility/DC Filter", 1.0),
                ("Utility/Channel", 3.0),
            ],
        ),
        (Eq, vec![("Output/Channel", 3.0)]),
        (Phaser, vec![("Phaser/LFO Shape", 1.0)]),
    ]
}

fn device(ty: BuiltinDeviceType, params: &[(&str, f64)]) -> Box<dyn Device> {
    let mut d = ether_devices::create(&BuiltinDevice::new(ty), &NoSamples);
    let desc = d.descriptor();
    for (key, v) in params {
        let (group, name) = key.split_once('/').unwrap();
        let p = desc
            .params
            .iter()
            .find(|p| p.name == name && p.group.as_deref() == Some(group))
            .unwrap_or_else(|| panic!("{ty:?}: no param {key}"));
        d.set_param(p.id, *v);
    }
    // EQ: turn a few bands on so it filters.
    if ty == BuiltinDeviceType::Eq {
        for p in &desc.params {
            if p.name.contains("Gain") && p.group.as_deref() != Some("Output") {
                d.set_param(p.id, 4.0);
            }
            if p.name == "On" || p.name.ends_with(" On") || p.name == "Enabled" {
                d.set_param(p.id, 1.0);
            }
        }
    }
    d.prepare(&PrepareConfig {
        sample_rate: SR,
        max_block_size: BLOCK,
        max_events_per_block: 64,
    });
    d
}

/// Deterministic test signal: noise + a sine at `level`.
fn signal(frames: usize, level: f32, seed: u32) -> [Vec<f32>; 2] {
    let mut x = seed.max(1);
    let mut noise = || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    };
    let mut l = vec![0.0; frames];
    let mut r = vec![0.0; frames];
    for i in 0..frames {
        let s = (i as f32 * 2.0 * std::f32::consts::PI * 220.0 / SR).sin();
        l[i] = level * (0.6 * s + 0.4 * noise());
        r[i] = level * (0.6 * s + 0.4 * noise());
    }
    [l, r]
}

/// Render `input` through `d`; `events`: (frame, param, value). Returns the output and the
/// first frame from which every block reported `Silent` (if any).
fn render(
    d: &mut dyn Device,
    input: &[Vec<f32>; 2],
    events: &[(usize, ParamId, f64)],
) -> ([Vec<f32>; 2], Option<usize>) {
    let frames = input[0].len();
    let mut out = [vec![0.0f32; frames], vec![0.0f32; frames]];
    let mut out_events = EventBuffer::with_capacity(64);
    let mut block_events: Vec<ProcessEvent> = Vec::with_capacity(64);
    let mut silent_from = None;
    let mut pos = 0;
    while pos < frames {
        let n = BLOCK.min(frames - pos);
        block_events.clear();
        for &(at, param, value) in events {
            if at >= pos && at < pos + n {
                block_events.push(ProcessEvent {
                    offset: (at - pos) as u32,
                    kind: EventKind::Param { param, value },
                });
            }
        }
        let transport = TransportInfo {
            playing: true,
            bpm: 120.0,
            beats_per_sample: 2.0 / SR as f64,
            sample_time: pos as u64,
            position: pos as f64 * 2.0 / SR as f64,
            ..TransportInfo::STOPPED
        };
        out_events.clear();
        let mut ctx = ProcessContext {
            sample_rate: SR,
            frames: n,
            transport: &transport,
            events: &block_events,
            out_events: &mut out_events,
        };
        let ins: [&[f32]; 2] = [&input[0][pos..pos + n], &input[1][pos..pos + n]];
        let [ol, or] = &mut out;
        let mut outs: [&mut [f32]; 2] = [&mut ol[pos..pos + n], &mut or[pos..pos + n]];
        let mut audio = AudioBuffers {
            inputs: &ins,
            outputs: &mut outs,
        };
        let st = d.process(&mut ctx, &mut audio);
        match st {
            ProcessStatus::Silent => {
                silent_from.get_or_insert(pos);
            }
            ProcessStatus::Continue => silent_from = None,
        }
        pos += n;
    }
    (out, silent_from)
}

#[test]
#[ignore = "perf harness: run in release with --ignored --nocapture"]
fn effect_costs() {
    let frames = SR as usize * 4;
    let busy = signal(frames, 0.5, 7);
    let silence = [vec![0.0; frames], vec![0.0; frames]];
    println!("ns per stereo sample (busy / silent input)");
    for (ty, params) in effects() {
        let mut d = device(ty, &params);
        let _ = render(d.as_mut(), &busy, &[]);
        let t = Instant::now();
        let _ = render(d.as_mut(), &busy, &[]);
        let busy_ns = t.elapsed().as_nanos() as f64 / frames as f64;
        let t = Instant::now();
        let (_, silent_from) = render(d.as_mut(), &silence, &[]);
        let silent_ns = t.elapsed().as_nanos() as f64 / frames as f64;
        let sleeps = silent_from.map_or("never".to_string(), |f| {
            format!("{:.0} ms", f as f64 * 1000.0 / SR as f64)
        });
        println!(
            "{:<22} {busy_ns:7.1} / {silent_ns:7.1}   reports Silent after {sleeps}",
            format!("{ty:?}")
        );
    }
}

/// signal 0.5 s, silence 0.5 s, loud 0.5 s (+ a param change), silence 1.5 s.
/// A param event: (frame, param, value).
type ParamEvent = (usize, ParamId, f64);

fn program(d: &dyn Device) -> ([Vec<f32>; 2], Vec<ParamEvent>) {
    let half = SR as usize / 2;
    let mut input = [vec![0.0; 6 * half], vec![0.0; 6 * half]];
    let a = signal(half, 0.25, 3);
    let b = signal(half, 0.9, 11);
    for c in 0..2 {
        input[c][..half].copy_from_slice(&a[c]);
        input[c][2 * half..3 * half].copy_from_slice(&b[c]);
    }
    // Move the first continuous param halfway (glides, coefficient changes).
    let desc = d.descriptor();
    let mut events = vec![];
    if let Some(p) = desc
        .params
        .iter()
        .find(|p| p.labels.is_none() && p.max > p.min)
    {
        let v = p.min + (p.max - p.min) * 0.37;
        events.push((2 * half + half / 3, p.id, v));
    }
    (input, events)
}

#[test]
#[ignore = "manual before/after comparison"]
fn effect_renders() {
    let dump = std::env::var("ETHER_DEVICE_DUMP").ok();
    let base = std::env::var("ETHER_DEVICE_BASE").ok();
    for (ty, params) in effects() {
        let mut d = device(ty, &params);
        let (input, events) = program(d.as_ref());
        let (out, _) = render(d.as_mut(), &input, &events);
        let flat: Vec<f32> = out[0]
            .iter()
            .zip(&out[1])
            .flat_map(|(l, r)| [*l, *r])
            .collect();
        let name = format!("{ty:?}");
        if let Some(dir) = &dump {
            let bytes: Vec<u8> = flat.iter().flat_map(|s| s.to_le_bytes()).collect();
            std::fs::write(format!("{dir}/{name}.f32"), bytes).unwrap();
        }
        if let Some(dir) = &base {
            let bytes = std::fs::read(format!("{dir}/{name}.f32")).unwrap();
            let old: Vec<f32> = bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| f32::from_le_bytes(*c))
                .collect();
            let peak = old.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            let (diff, at) = old
                .iter()
                .zip(&flat)
                .enumerate()
                .map(|(i, (a, b))| ((a - b).abs(), i / 2))
                .fold((0.0f32, 0), |m, d| if d.0 > m.0 { d } else { m });
            let same = old
                .iter()
                .zip(&flat)
                .all(|(a, b)| a.to_bits() == b.to_bits());
            // Away from the param change (a discontinuity either way).
            let near = |f: usize| events.iter().any(|e| f + 64 >= e.0 && f < e.0 + 64);
            let away = old
                .iter()
                .zip(&flat)
                .enumerate()
                .filter(|(i, _)| !near(i / 2))
                .fold(0.0f32, |m, (_, (a, b))| m.max((a - b).abs()));
            println!(
                "{name:<22} {}  max |diff| {diff:.3e} at frame {at}, {away:.3e} away from the \
                 param change (peak {peak:.3})",
                if same {
                    "bit-identical"
                } else {
                    "differs      "
                }
            );
        }
    }
}

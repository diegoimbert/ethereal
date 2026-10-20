//! FM Synth (`fm-synth`): descriptor ↔ mock parity, pitch and ratios, every algorithm's
//! routing, envelopes and velocity, feedback, filter, polyphony and stealing, extreme and
//! random params (finite, bounded, no denormal tails), RT safety, block-size invariance,
//! factory presets and the CPU budget.

use std::collections::BTreeMap;

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{BuiltinDeviceType, ParamId, load_preset};
use ether_core::{
    AudioBuffers, Device, EventBuffer, EventKind, Node, PrepareConfig, ProcessContext,
    ProcessEvent, ProcessStatus, TransportInfo,
};
use ether_devices::fm_synth::algorithms::{ALGORITHMS, OPS};
use ether_devices::fm_synth::{FmSynth, fm_synth as p};

const SR: f32 = 48_000.0;

use p::op;

fn opp(o: usize, field: u32) -> ParamId {
    p::op_param(o, field)
}

struct Rig {
    synth: FmSynth,
    time: u64,
    block: usize,
    out_events: EventBuffer,
    last_status: ProcessStatus,
}

/// An event at an absolute sample time.
#[derive(Clone, Copy)]
struct At(u64, EventKind);

fn on_vel(key: u8, id: u32, velocity: f32) -> EventKind {
    EventKind::NoteOn {
        note_id: id,
        channel: 0,
        key,
        velocity,
    }
}

fn on(key: u8, id: u32) -> EventKind {
    on_vel(key, id, 1.0)
}

fn off(key: u8, id: u32) -> EventKind {
    EventKind::NoteOff {
        note_id: id,
        channel: 0,
        key,
        velocity: 0.0,
    }
}

fn set(param: ParamId, value: f64) -> EventKind {
    EventKind::Param { param, value }
}

impl Rig {
    fn new() -> Self {
        Self::with_block(256)
    }

    fn with_block(block: usize) -> Self {
        let mut synth = FmSynth::new();
        synth.prepare(&PrepareConfig {
            sample_rate: SR,
            max_block_size: 512,
            max_events_per_block: 256,
        });
        Self {
            synth,
            time: 0,
            block,
            out_events: EventBuffer::with_capacity(64),
            last_status: ProcessStatus::Silent,
        }
    }

    fn params(&mut self, values: &[(ParamId, f64)]) {
        for &(id, v) in values {
            self.synth.set_param(id, v);
        }
    }

    /// Render `frames` samples with `events` (absolute times from now), in blocks.
    fn run(&mut self, frames: usize, events: &[At]) -> (Vec<f32>, Vec<f32>) {
        let start = self.time;
        let (mut l, mut r) = (vec![0.0f32; frames], vec![0.0f32; frames]);
        let mut done = 0;
        let mut evs = EventBuffer::with_capacity(256);
        while done < frames {
            let n = self.block.min(frames - done);
            let t0 = start + done as u64;
            evs.clear();
            for e in events {
                let at = start + e.0;
                if at >= t0 && at < t0 + n as u64 {
                    evs.push(ProcessEvent {
                        offset: (at - t0) as u32,
                        kind: e.1,
                    });
                }
            }
            assert_no_alloc(|| evs.sort());
            let transport = TransportInfo {
                playing: true,
                sample_time: t0,
                bpm: 120.0,
                beats_per_sample: 2.0 / f64::from(SR),
                position: t0 as f64 * 2.0 / f64::from(SR),
                ..TransportInfo::STOPPED
            };
            self.out_events.clear();
            let (lo, ro) = (&mut l[done..done + n], &mut r[done..done + n]);
            let mut outs: [&mut [f32]; 2] = [lo, ro];
            let mut ctx = ProcessContext {
                sample_rate: SR,
                frames: n,
                transport: &transport,
                events: evs.as_slice(),
                out_events: &mut self.out_events,
            };
            let mut buffers = AudioBuffers {
                inputs: &[],
                outputs: &mut outs,
            };
            let synth = &mut self.synth;
            self.last_status = assert_no_alloc(|| synth.process(&mut ctx, &mut buffers));
            done += n;
        }
        self.time += frames as u64;
        (l, r)
    }
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// Largest sample-to-sample jump (clicks).
fn max_step(x: &[f32]) -> f32 {
    x.windows(2).fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()))
}

/// Frequency from positive-going zero crossings (for near-sine signals).
fn frequency(x: &[f32]) -> f32 {
    let crossings: Vec<f32> = (1..x.len())
        .filter(|&i| x[i - 1] <= 0.0 && x[i] > 0.0)
        .map(|i| (i - 1) as f32 + x[i - 1] / (x[i - 1] - x[i]))
        .collect();
    assert!(crossings.len() > 2, "no periodic signal");
    let span = crossings[crossings.len() - 1] - crossings[0];
    (crossings.len() - 1) as f32 * SR / span
}

/// Energy of `x` outside the fundamental `hz` (1 − the share of a fitted sine): 0 for a
/// pure sine at `hz`, larger the more harmonics or sidebands.
fn impurity(x: &[f32], hz: f32) -> f32 {
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (i, v) in x.iter().enumerate() {
        let a = std::f64::consts::TAU * f64::from(hz) * i as f64 / f64::from(SR);
        re += f64::from(*v) * a.cos();
        im += f64::from(*v) * a.sin();
    }
    let n = x.len() as f64;
    let fund = 2.0 * (re * re + im * im) / (n * n);
    let total = x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / n;
    (1.0 - fund / total.max(1e-30)).max(0.0) as f32
}

/// Amplitude of the component of `x` at `hz` (single-bin DFT).
fn tone_level(x: &[f32], hz: f32) -> f32 {
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (i, v) in x.iter().enumerate() {
        let a = std::f64::consts::TAU * f64::from(hz) * i as f64 / f64::from(SR);
        re += f64::from(*v) * a.cos();
        im += f64::from(*v) * a.sin();
    }
    (2.0 * (re * re + im * im).sqrt() / x.len() as f64) as f32
}

// ---------------------------------------------------------------------------------------
// Descriptor

#[test]
fn mock_descriptor_matches_rust_and_ids_are_dense() {
    let d = ether_devices::descriptor(BuiltinDeviceType::FmSynth);
    assert_eq!(d.params.len(), p::COUNT);
    for (i, info) in d.params.iter().enumerate() {
        assert_eq!(info.id.0 as usize, i, "dense ids");
    }
    assert_eq!(d.name, "FM Synth");
    assert!(d.midi_input && d.audio_inputs == 0 && d.audio_outputs == 2);
    // The mock transport's copy (`ui/src/transport/mock/devices/fmSynth.json`).
    let map: BTreeMap<String, DeviceDescriptor> = [("FmSynth".to_owned(), d)].into();
    let expected = serde_json::to_value(map).unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../ui/src/transport/mock/devices/fmSynth.json");
    let text = format!("{}\n", serde_json::to_string_pretty(&expected).unwrap());
    if std::env::var_os("UPDATE_MOCK_DESCRIPTORS").is_some() {
        std::fs::write(&path, &text).unwrap();
        return;
    }
    let found: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "{}: {e} (run with UPDATE_MOCK_DESCRIPTORS=1)",
                path.display()
            )
        }))
        .unwrap();
    assert_eq!(
        found,
        expected,
        "{} is stale: run `UPDATE_MOCK_DESCRIPTORS=1 cargo test -p ether-devices --test fm_synth`",
        path.display()
    );
}

// ---------------------------------------------------------------------------------------
// Pitch

/// Only operator A, a sine at 0 dB (the default patch).
#[test]
fn default_patch_is_a_sine_at_the_note_and_goes_silent_after_release() {
    let mut rig = Rig::new();
    let (l, r) = rig.run(48_000, &[At(0, on(69, 1)), At(24_000, off(69, 1))]);
    assert_eq!(l, r, "mono voices: both channels equal");
    let hz = frequency(&l[4_800..20_000]);
    assert!((hz - 440.0).abs() < 0.5, "{hz}");
    assert!(impurity(&l[4_800..14_400], 440.0) < 1e-3);
    // Volume -14 dB.
    let pk = peak(&l[4_800..20_000]);
    assert!((pk - 0.2).abs() < 0.012, "peak {pk}");
    // Release (300 ms default) ends, then silence.
    assert!(rms(&l[24_000..26_000]) > 0.01);
    let (tail, _) = rig.run(48_000, &[]);
    assert!(tail[24_000..].iter().all(|v| *v == 0.0), "no denormal tail");
    assert_eq!(rig.last_status, ProcessStatus::Silent);
    assert_eq!(rig.synth.active_voices(), 0);
}

#[test]
fn ratios_fixed_frequency_transpose_and_bend() {
    let hz = |params: &[(ParamId, f64)], key: u8| {
        let mut rig = Rig::new();
        rig.params(params);
        let (l, _) = rig.run(19_200, &[At(0, on(key, 1))]);
        frequency(&l[4_800..])
    };
    let a = |field| opp(0, field);
    let close = |got: f32, want: f32| (got / want - 1.0).abs() < 2e-3;
    assert!(close(hz(&[(a(op::COARSE), 2.0)], 69), 880.0));
    assert!(
        close(hz(&[(a(op::COARSE), 0.0)], 69), 220.0),
        "coarse 0 = ½"
    );
    assert!(close(hz(&[(a(op::COARSE), 3.0)], 57), 660.0));
    // Fine: thousandths of the ratio.
    assert!(close(hz(&[(a(op::FINE), 500.0)], 69), 660.0));
    // Fixed frequency ignores the key.
    let fixed = [(a(op::FIXED), 1.0), (a(op::FREQUENCY), 1000.0)];
    assert!(close(hz(&fixed, 40), 1000.0));
    assert!(close(hz(&fixed, 90), 1000.0));
    assert!(close(hz(&[(p::TRANSPOSE, 12.0)], 69), 880.0));
    assert!(close(
        hz(&[(p::FINE, 100.0)], 69),
        440.0 * 2f32.powf(1.0 / 12.0)
    ));
    // Pitch bend: full up at the default 2 semitones.
    let mut rig = Rig::new();
    let (l, _) = rig.run(
        24_000,
        &[
            At(0, on(69, 1)),
            At(
                0,
                EventKind::Midi {
                    data: [0xE0, 0x7F, 0x7F],
                },
            ),
        ],
    );
    let bent = frequency(&l[9_600..]);
    assert!(
        (bent / (440.0 * 2f32.powf(2.0 / 12.0)) - 1.0).abs() < 3e-3,
        "{bent}"
    );
}

// ---------------------------------------------------------------------------------------
// Algorithms

/// Render one note (key 57, 220 Hz) with the given operators audible at 0 dB (the others
/// at -70 dB, still on), in `alg`. Ratios A 1, B 2, C 3, D 5 so every pair is distinct.
fn render_ops(alg: usize, audible: &[usize]) -> Vec<f32> {
    let mut rig = Rig::new();
    rig.synth.set_param(p::ALGORITHM, alg as f64);
    for (o, ratio) in [1.0, 2.0, 3.0, 5.0].into_iter().enumerate() {
        rig.synth.set_param(opp(o, op::COARSE), ratio);
        let level = if audible.contains(&o) { -6.0 } else { -70.0 };
        rig.synth.set_param(opp(o, op::LEVEL), level);
    }
    rig.run(9_600, &[At(0, on(57, 1))]).0
}

#[test]
fn every_algorithm_routes_its_operators_as_its_table_says() {
    for (a, alg) in ALGORITHMS.iter().enumerate() {
        let alone: Vec<Vec<f32>> = (0..OPS).map(|o| render_ops(a, &[o])).collect();
        for (o, solo) in alone.iter().enumerate() {
            // An operator is heard iff it is a carrier.
            let heard = rms(&solo[2_400..]) > 1e-3;
            assert_eq!(heard, alg.is_carrier(o), "algorithm {} op {o}", a + 1);
        }
        for i in (0..OPS).filter(|&i| alg.is_carrier(i)) {
            for j in (0..OPS).filter(|&j| j != i) {
                // Without modulation the pair is the sum of the two alone (linear mix);
                // `j` modulating `i` changes the carrier's spectrum.
                let both = render_ops(a, &[i, j]);
                let diff: Vec<f32> = (2_400..both.len())
                    .map(|k| both[k] - alone[i][k] - alone[j][k])
                    .collect();
                let ratio = rms(&diff) / rms(&alone[i][2_400..]);
                let modulates = alg.modulators[i] & (1 << j) != 0;
                if modulates {
                    assert!(ratio > 0.2, "algorithm {}: {j} → {i}: {ratio}", a + 1);
                } else {
                    assert!(ratio < 1e-3, "algorithm {}: {j} ↛ {i}: {ratio}", a + 1);
                }
            }
        }
    }
}

#[test]
fn modulation_and_feedback_add_harmonics() {
    // Stack: B (ratio 1) into A, more modulator level = more sidebands.
    let purity = |b_level: f64, feedback: f64| {
        let mut rig = Rig::new();
        rig.params(&[
            (opp(1, op::LEVEL), b_level),
            (opp(0, op::FEEDBACK), feedback),
        ]);
        let (l, _) = rig.run(14_400, &[At(0, on(57, 1))]);
        impurity(&l[4_800..], 220.0)
    };
    let pure = purity(-70.0, 0.0);
    let light = purity(-24.0, 0.0);
    let heavy = purity(-6.0, 0.0);
    let fed = purity(-70.0, 100.0);
    assert!(pure < 1e-3, "{pure}");
    assert!(
        light > pure + 0.005 && heavy > light,
        "{pure} {light} {heavy}"
    );
    assert!(fed > 0.05, "feedback: {fed}");
}

// ---------------------------------------------------------------------------------------
// Envelopes, velocity

#[test]
fn operator_envelope_follows_adsr() {
    let a = |field| opp(0, field);
    let mut rig = Rig::new();
    rig.params(&[
        (a(op::ATTACK), 100.0),
        (a(op::DECAY), 200.0),
        (a(op::SUSTAIN), 25.0),
        (a(op::RELEASE), 200.0),
    ]);
    let (l, _) = rig.run(48_000, &[At(0, on(69, 1)), At(24_000, off(69, 1))]);
    let win = |ms: f32| {
        let s = (ms * 48.0) as usize;
        peak(&l[s..s + 480])
    };
    // Rising through the attack, peak near its end, decaying to the sustain.
    assert!(win(10.0) < win(50.0) && win(50.0) < win(95.0), "attack");
    let top = win(95.0);
    assert!((top - 0.2).abs() < 0.02, "peak {top}");
    let sustain = win(450.0);
    assert!(
        (sustain / top - 0.25).abs() < 0.03,
        "sustain {}",
        sustain / top
    );
    // Release: -80 dB in 200 ms.
    assert!(win(560.0) < sustain * 0.2);
    assert!(peak(&l[36_000..]) < 1e-3);
    assert!(max_step(&l) < 0.06, "click {}", max_step(&l));
}

#[test]
fn velocity_sensitivity_scales_operator_levels() {
    let level = |sens: f64, velocity: f32| {
        let mut rig = Rig::new();
        rig.params(&[(opp(0, op::VELOCITY), sens)]);
        let (l, _) = rig.run(9_600, &[At(0, on_vel(69, 1, velocity))]);
        peak(&l[4_800..])
    };
    let full = level(100.0, 1.0);
    assert!(
        (level(100.0, 0.5) / full - 0.25).abs() < 0.02,
        "squared velocity"
    );
    assert!((level(0.0, 0.2) / full - 1.0).abs() < 1e-3, "insensitive");
    assert!((level(50.0, 0.001) / full - 0.5).abs() < 0.02, "half");
}

// ---------------------------------------------------------------------------------------
// Filter, LFO, tone

#[test]
fn filter_darkens_and_its_envelope_opens_it() {
    // A bright patch: B into A at -6 dB, ratio 3.
    let bright = [(opp(1, op::LEVEL), -6.0), (opp(1, op::COARSE), 3.0)];
    let render = |extra: &[(ParamId, f64)]| {
        let mut rig = Rig::new();
        rig.params(&bright);
        rig.params(extra);
        let (l, _) = rig.run(19_200, &[At(0, on(57, 1))]);
        l
    };
    let open = render(&[]);
    let closed = render(&[(p::FILTER_ON, 1.0), (p::CUTOFF, 300.0), (p::RESONANCE, 0.0)]);
    assert!(impurity(&closed[4_800..], 220.0) < impurity(&open[4_800..], 220.0) * 0.5);
    for ty in 0..6 {
        let x = render(&[
            (p::FILTER_ON, 1.0),
            (p::FILTER_TYPE, ty as f64),
            (p::CUTOFF, 900.0),
        ]);
        assert!(
            x.iter().all(|v| v.is_finite()) && rms(&x[4_800..]) > 1e-3,
            "type {ty}"
        );
    }
    // Envelope: closed cutoff, full positive amount, decaying: bright first, darker later.
    let env = render(&[
        (p::FILTER_ON, 1.0),
        (p::CUTOFF, 200.0),
        (p::FILTER_ENV_AMOUNT, 100.0),
        (p::FILTER_DECAY, 150.0),
    ]);
    assert!(impurity(&env[480..2_400], 220.0) > impurity(&env[14_400..], 220.0) + 0.01);
}

#[test]
fn lfo_modulates_pitch_and_volume_and_tone_darkens() {
    let mut rig = Rig::new();
    rig.params(&[(p::LFO_RATE, 4.0), (p::LFO_VOLUME, 100.0)]);
    let (l, _) = rig.run(48_000, &[At(0, on(69, 1))]);
    let windows: Vec<f32> = (0..20)
        .map(|w| peak(&l[w * 2_400..(w + 1) * 2_400]))
        .collect();
    let (lo, hi) = windows[2..]
        .iter()
        .fold((f32::MAX, 0.0f32), |(a, b), v| (a.min(*v), b.max(*v)));
    assert!(lo < hi * 0.2, "tremolo {lo} .. {hi}");

    let mut rig = Rig::new();
    rig.params(&[(p::LFO_RATE, 2.0), (p::LFO_PITCH, 50.0)]);
    let (l, _) = rig.run(48_000, &[At(0, on(69, 1))]);
    let f: Vec<f32> = (0..8)
        .map(|w| frequency(&l[w * 6_000..(w + 1) * 6_000]))
        .collect();
    let spread =
        f.iter().fold(0.0f32, |m, v| m.max(*v)) - f.iter().fold(f32::MAX, |m, v| m.min(*v));
    assert!(spread > 20.0, "vibrato {f:?}");

    // Tone: a saw through the 1 kHz corner loses its highs.
    let bright = |tone: f64| {
        let mut rig = Rig::new();
        rig.params(&[(opp(0, op::WAVE), 2.0), (p::TONE, tone)]);
        let (l, _) = rig.run(9_600, &[At(0, on(69, 1))]);
        impurity(&l[4_800..], 440.0)
    };
    assert!(bright(0.0) < bright(100.0) * 0.6);
}

// ---------------------------------------------------------------------------------------
// Voices

#[test]
fn polyphony_limit_steals_the_oldest_voice_without_clicks() {
    let mut rig = Rig::new();
    rig.params(&[(p::VOICES, 4.0), (opp(0, op::ATTACK), 5.0)]);
    let notes: Vec<At> = (0..6)
        .map(|i| At(i * 2_000, on(48 + (i as u8) * 4, i as u32)))
        .collect();
    let (l, _) = rig.run(24_000, &notes);
    assert_eq!(rig.synth.active_voices(), 4);
    assert!(max_step(&l) < 0.1, "click {}", max_step(&l));
    // Releasing everything ends every voice.
    let offs: Vec<At> = (0..6)
        .map(|i| At(0, off(48 + i * 4, u32::from(i))))
        .collect();
    rig.run(48_000, &offs);
    assert_eq!(rig.synth.active_voices(), 0);
}

#[test]
fn sustain_pedal_and_glide() {
    let mut rig = Rig::new();
    let pedal = |down: bool| EventKind::Midi {
        data: [0xB0, 64, if down { 127 } else { 0 }],
    };
    rig.run(
        4_800,
        &[At(0, pedal(true)), At(0, on(60, 1)), At(2_000, off(60, 1))],
    );
    let (l, _) = rig.run(24_000, &[]);
    assert!(rms(&l[20_000..]) > 0.1, "held by the pedal");
    rig.run(48_000, &[At(0, pedal(false))]);
    assert_eq!(rig.synth.active_voices(), 0);

    let mut rig = Rig::new();
    rig.params(&[(p::GLIDE, 200.0)]);
    let (l, _) = rig.run(
        48_000,
        &[
            At(0, on(57, 1)),
            At(9_600, off(57, 1)),
            At(9_600, on(69, 2)),
        ],
    );
    let mid = frequency(&l[10_000..12_000]);
    assert!(mid > 230.0 && mid < 420.0, "gliding: {mid}");
    assert!((frequency(&l[36_000..]) - 440.0).abs() < 1.0);
}

/// `Mode` (Mono / Legato): one voice, last-note priority, back to the held key on release.
#[test]
fn mono_and_legato_modes_use_one_voice_and_last_note_priority() {
    for mode in [1.0, 2.0] {
        let mut rig = Rig::new();
        rig.params(&[(p::VOICE_MODE, mode)]);
        let (l, _) = rig.run(
            48_000,
            &[
                At(0, on(57, 1)),
                At(12_000, on(69, 2)),
                At(24_000, off(69, 2)),
            ],
        );
        assert_eq!(rig.synth.active_voices(), 1, "mode {mode}");
        let f1 = frequency(&l[2_400..11_000]);
        let f2 = frequency(&l[14_400..23_000]);
        let f3 = frequency(&l[26_400..47_000]);
        assert!((f1 / 220.0 - 1.0).abs() < 2e-3, "{mode}: {f1}");
        assert!((f2 / 440.0 - 1.0).abs() < 2e-3, "{mode}: {f2}");
        assert!((f3 / 220.0 - 1.0).abs() < 2e-3, "{mode}: back to held {f3}");
        rig.run(4_800, &[At(0, off(57, 1))]);
        rig.run(48_000, &[]);
        assert_eq!(rig.synth.active_voices(), 0, "mode {mode}: released");
    }
}

#[test]
fn legato_does_not_retrigger_but_mono_does_and_only_overlaps_glide() {
    let run = |mode: f64, glide: f64, off_at: u64| {
        let mut rig = Rig::new();
        rig.params(&[
            (p::VOICE_MODE, mode),
            (p::GLIDE, glide),
            (opp(0, op::ATTACK), 200.0),
            (opp(0, op::DECAY), 50.0),
            (opp(0, op::SUSTAIN), 20.0),
            (opp(0, op::RELEASE), 1.0),
        ]);
        let (l, _) = rig.run(
            48_000,
            &[
                At(0, on(57, 1)),
                At(24_000, on(69, 2)),
                At(off_at, off(57, 1)),
                At(40_000, off(69, 2)),
            ],
        );
        l
    };
    // 40 ms into the second note: legato stays at sustain, mono is in its new attack.
    let level = |l: &[f32]| rms(&l[26_000..26_400]) / rms(&l[23_000..23_400]);
    let legato = run(2.0, 0.0, 30_000);
    assert!(
        (level(&legato) - 1.0).abs() < 0.2,
        "legato {}",
        level(&legato)
    );
    let f = frequency(&legato[24_400..29_000]);
    assert!((f / 440.0 - 1.0).abs() < 3e-3, "no glide at 0 ms: {f}");
    let mono = run(1.0, 0.0, 30_000);
    assert!(level(&mono) > 1.5, "mono retriggers {}", level(&mono));
    // Legato + glide: an overlap slides, a detached note (touching) jumps and retriggers.
    let slide = run(2.0, 300.0, 30_000);
    let mid = frequency(&slide[24_600..26_000]);
    assert!(mid > 240.0 && mid < 420.0, "gliding: {mid}");
    let touching = run(2.0, 300.0, 24_000);
    assert!(level(&touching) > 1.5, "retriggered: {}", level(&touching));
    let f = frequency(&touching[24_200..25_600]);
    assert!((f / 440.0 - 1.0).abs() < 0.01, "no glide: {f}");
}

// ---------------------------------------------------------------------------------------
// Robustness

#[test]
fn extreme_and_random_params_stay_finite_and_bounded() {
    let desc = ether_devices::descriptor(BuiltinDeviceType::FmSynth);
    let mut seed = 0xFACE_u32;
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        f64::from(seed) / f64::from(u32::MAX)
    };
    for round in 0..16 {
        let mut rig = Rig::new();
        for info in &desc.params {
            let v = match round {
                0 => info.min,
                1 => info.max,
                _ => info.min + (info.max - info.min) * rnd(),
            };
            rig.synth.set_param(info.id, v);
        }
        if round == 1 {
            // Every operator heard and fed back at full level, in each algorithm.
            rig.synth.set_param(p::ALGORITHM, 10.0);
        }
        let events: Vec<At> = (0..12)
            .map(|i| At(i * 400, on(12 + (i as u8) * 9, i as u32)))
            .collect();
        let (l, r) = rig.run(24_000, &events);
        assert!(
            l.iter().chain(&r).all(|v| v.is_finite()),
            "round {round}: NaN"
        );
        let pk = peak(&l).max(peak(&r));
        assert!(pk < 16.0, "round {round}: peak {pk}");
        // All notes off: everything ends, no denormals left in the output.
        let offs: Vec<At> = (0..12)
            .map(|i| At(0, off(12 + i * 9, u32::from(i))))
            .collect();
        let (l, _) = rig.run(24_000, &offs);
        assert!(
            l.iter().all(|v| *v == 0.0 || v.abs() > f32::MIN_POSITIVE),
            "round {round}: denormal output"
        );
    }
    // Feedback at 100 % on every operator, all algorithms, stays bounded.
    for a in 0..ALGORITHMS.len() {
        let mut rig = Rig::new();
        rig.synth.set_param(p::ALGORITHM, a as f64);
        for o in 0..OPS {
            rig.params(&[(opp(o, op::LEVEL), 0.0), (opp(o, op::FEEDBACK), 100.0)]);
        }
        let (l, _) = rig.run(9_600, &[At(0, on(100, 1)), At(0, on(20, 2))]);
        assert!(
            l.iter().all(|v| v.is_finite()) && peak(&l) < 4.0,
            "algorithm {}",
            a + 1
        );
    }
    let mut s = FmSynth::new();
    s.set_param(p::CUTOFF, f64::NAN);
    assert_eq!(s.param(p::CUTOFF), Some(8000.0));
    s.set_param(opp(2, op::COARSE), 1e9);
    assert_eq!(s.param(opp(2, op::COARSE)), Some(48.0));
}

/// Control chunks sit on the absolute sample clock: any block size that is a multiple of
/// the control chunk (16) renders identically.
#[test]
fn render_is_deterministic_and_independent_of_the_block_size() {
    let events = [
        At(0, set(opp(1, op::LEVEL), -12.0)),
        At(10, on(60, 1)),
        At(333, on(64, 2)),
        At(1_000, set(p::ALGORITHM, 4.0)),
        At(1_234, set(opp(3, op::WAVE), 4.0)),
        At(1_300, set(opp(3, op::LEVEL), -20.0)),
        At(2_000, set(p::FILTER_ON, 1.0)),
        At(5_000, off(60, 1)),
        At(7_777, off(64, 2)),
    ];
    let render = |block: usize| Rig::with_block(block).run(12_000, &events);
    let a = render(512);
    for block in [16, 64, 128, 256] {
        assert_eq!(render(block), a, "block {block}");
    }
    // Without oversampling too.
    let mut ev = events.to_vec();
    ev.insert(0, At(0, set(p::ANTIALIAS, 0.0)));
    let b = Rig::with_block(512).run(12_000, &ev);
    assert_eq!(Rig::with_block(64).run(12_000, &ev), b);
}

#[test]
fn antialias_removes_folded_sidebands() {
    // Key 96 (2093 Hz) modulated at ratio 10 (20.9 kHz): the second upper sideband (44 kHz)
    // folds to 4047 Hz at the base rate. Oversampled, the decimator removes it.
    let level_at = |aa: f64, hz: f32| {
        let mut rig = Rig::new();
        rig.params(&[
            (p::ANTIALIAS, aa),
            (opp(1, op::LEVEL), -12.0),
            (opp(1, op::COARSE), 10.0),
        ]);
        let (l, _) = rig.run(14_400, &[At(0, on(96, 1))]);
        tone_level(&l[4_800..], hz)
    };
    let fold = 48_000.0 - (2_093.0 + 2.0 * 20_930.0);
    let (with, without) = (level_at(1.0, fold), level_at(0.0, fold));
    let fundamental = level_at(1.0, 2_093.0);
    assert!(
        without > fundamental * 0.05,
        "the alias exists: {without} vs {fundamental}"
    );
    assert!(with < without * 0.05, "with {with} vs without {without}");
}

#[test]
fn factory_presets_load_and_sound() {
    let presets = ether_devices::factory_presets(BuiltinDeviceType::FmSynth);
    assert_eq!(presets.len(), 5);
    let mut bad = vec![];
    for fp in presets {
        let preset = load_preset(fp.json).unwrap();
        let mut rig = Rig::new();
        for (id, v) in &preset.params {
            rig.synth.set_param(*id, *v);
        }
        let mono = rig.synth.param(p::VOICES) == Some(1.0);
        let keys: &[u8] = if mono { &[36] } else { &[48, 55, 60, 64] };
        let mut events: Vec<At> = keys
            .iter()
            .enumerate()
            .map(|(i, k)| At(0, on(*k, i as u32)))
            .collect();
        events.extend(
            keys.iter()
                .enumerate()
                .map(|(i, k)| At(48_000, off(*k, i as u32))),
        );
        let (l, r) = rig.run(72_000, &events);
        let pk = peak(&l).max(peak(&r));
        println!(
            "{:20} peak {:6.1} dBFS, rms {:6.1} dBFS",
            fp.id,
            20.0 * pk.log10(),
            20.0 * rms(&l[..48_000]).log10()
        );
        if !(0.1..0.95).contains(&pk) {
            bad.push(format!("{}: peak {pk}", fp.id));
        }
        assert!(l.iter().chain(&r).all(|v| v.is_finite()), "{}", fp.id);
    }
    assert!(bad.is_empty(), "{bad:?}");
}

/// CPU budget: 16 voices, four operators each (one fed back), filter on, oversampled.
#[test]
fn cpu_budget_16_voices() {
    let mut rig = Rig::with_block(256);
    rig.params(&[
        (p::ALGORITHM, 4.0),
        (opp(1, op::LEVEL), -12.0),
        (opp(2, op::LEVEL), -12.0),
        (opp(3, op::LEVEL), -18.0),
        (opp(3, op::FEEDBACK), 40.0),
        (opp(2, op::WAVE), 2.0),
        (p::FILTER_ON, 1.0),
        (p::FILTER_TYPE, 1.0),
        (p::FILTER_ENV_AMOUNT, 40.0),
        (p::LFO_PITCH, 10.0),
        (p::VOICES, 16.0),
    ]);
    let notes: Vec<At> = (0..16)
        .map(|i| At(0, on(36 + i * 3, u32::from(i))))
        .collect();
    rig.run(4_800, &notes);
    assert_eq!(rig.synth.active_voices(), 16);
    let seconds = 5.0;
    let frames = (SR * seconds) as usize;
    let t = std::time::Instant::now();
    rig.run(frames, &[]);
    let load = t.elapsed().as_secs_f32() / seconds;
    println!("fm synth: 16 voices, {:.1} % of realtime", load * 100.0);
    // Generous: shared, loaded CI machines and the opt-level 1 dev profile.
    assert!(load < 0.5, "{:.1} % of realtime", load * 100.0);
}

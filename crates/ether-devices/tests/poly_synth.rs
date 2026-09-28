//! Poly Synth (`synth-2`): rendering, pitch, voice allocation (stealing, mono, legato),
//! glide, unison, pedal, pitch bend, extreme params, RT safety, block-size invariance,
//! factory presets and the CPU budget (16 voices × 4 unison).

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::model::{BuiltinDeviceType, ParamId, load_preset};
use ether_core::{
    AudioBuffers, Device, EventBuffer, EventKind, Node, PrepareConfig, ProcessContext,
    ProcessEvent, ProcessStatus, TransportInfo,
};
use ether_devices::poly_synth::{PolySynth, poly_synth as p};

#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

const SR: f32 = 48_000.0;

struct Rig {
    synth: PolySynth,
    time: u64,
    block: usize,
    out_events: EventBuffer,
    last_status: ProcessStatus,
}

/// An event at an absolute sample time.
#[derive(Clone, Copy)]
struct At(u64, EventKind);

fn on(key: u8, id: u32) -> EventKind {
    EventKind::NoteOn {
        note_id: id,
        channel: 0,
        key,
        velocity: 1.0,
    }
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
        let mut synth = PolySynth::new();
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
        let mut evs: Vec<ProcessEvent> = Vec::with_capacity(256);
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
                events: &evs,
                out_events: &mut self.out_events,
            };
            let mut buffers = AudioBuffers {
                inputs: &[],
                outputs: &mut outs,
            };
            let synth = &mut self.synth;
            let status = assert_no_alloc(|| synth.process(&mut ctx, &mut buffers));
            self.last_status = status;
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

/// A clean sine voice: open filter, no drive, instant-ish attack.
fn sine_patch(rig: &mut Rig) {
    rig.params(&[
        (p::OSC1_TYPE, 3.0),
        (p::CUTOFF, 20_000.0),
        (p::RESONANCE, 0.0),
        (p::FILTER_TYPE, 1.0),
        (p::AMP_SUSTAIN, 100.0),
    ]);
}

#[test]
fn default_patch_renders_a_note_and_goes_silent_after_release() {
    let mut rig = Rig::new();
    let (l, r) = rig.run(24_000, &[At(100, on(60, 1)), At(12_000, off(60, 1))]);
    assert!(l[..100].iter().all(|&v| v == 0.0));
    let body = &l[2_000..12_000];
    assert!(l.iter().chain(&r).all(|v| v.is_finite()));
    let pk = peak(body);
    assert!((0.05..0.9).contains(&pk), "peak {pk}");
    assert_eq!(l, r, "unison 1 is mono");
    // Release 300 ms: silent well before 1 s later.
    let (tail, _) = rig.run(48_000, &[]);
    assert!(peak(&tail[24_000..]) < 1e-4);
    rig.run(512, &[]);
    assert_eq!(rig.last_status, ProcessStatus::Silent);
    assert_eq!(rig.synth.active_voices(), 0);
}

#[test]
fn note_start_and_release_are_click_free() {
    for ty in 0..4 {
        let mut rig = Rig::new();
        rig.params(&[
            (p::OSC1_TYPE, f64::from(ty)),
            (p::AMP_ATTACK, 0.0),
            (p::AMP_RELEASE, 1.0),
            (p::CUTOFF, 20_000.0),
        ]);
        let (l, _) = rig.run(9_600, &[At(10, on(48, 1)), At(4_811, off(48, 1))]);
        // The oscillator itself moves by at most ~0.05/sample at 130 Hz; a click would be a
        // jump of the full output level.
        let pk = peak(&l);
        assert!(max_step(&l[..40]) < pk * 0.25, "type {ty}: start");
        assert!(max_step(&l[4_800..4_900]) < pk * 0.35, "type {ty}: release");
    }
}

#[test]
fn pitch_is_accurate_across_the_keyboard_and_oscillator_offsets() {
    for (key, extra, hz) in [
        (69u8, &[][..], 440.0f32),
        (57, &[][..], 220.0),
        (81, &[][..], 880.0),
        (69, &[(p::OSC1_OCTAVE, -1.0)][..], 220.0),
        (69, &[(p::OSC1_SEMITONES, 7.0)][..], 659.255),
        (69, &[(p::OSC1_FINE, 1.0)][..], 466.164),
    ] {
        let mut rig = Rig::new();
        sine_patch(&mut rig);
        rig.params(extra);
        let (l, _) = rig.run(48_000, &[At(0, on(key, 1))]);
        let f = frequency(&l[4_800..]);
        assert!(
            (f / hz - 1.0).abs() < 0.002,
            "key {key} {extra:?}: {f} vs {hz}"
        );
    }
}

#[test]
fn pitch_bend_follows_the_bend_range() {
    let mut rig = Rig::new();
    sine_patch(&mut rig);
    rig.params(&[(p::BEND_RANGE, 12.0)]);
    // Full bend up: +12 semitones.
    let bend = EventKind::Midi {
        data: [0xE0, 0x7F, 0x7F],
    };
    let (l, _) = rig.run(48_000, &[At(0, on(57, 1)), At(0, bend)]);
    let f = frequency(&l[4_800..]);
    assert!((f / 440.0 - 1.0).abs() < 0.003, "{f}");
}

#[test]
fn every_wavetable_sounds_across_its_positions_and_the_keyboard() {
    for table in 0..8 {
        for (pos, key) in [(0.0, 96), (37.0, 60), (100.0, 36), (100.0, 72)] {
            let mut rig = Rig::new();
            rig.params(&[
                (p::OSC1_TYPE, 4.0),
                (p::OSC1_TABLE, f64::from(table)),
                (p::OSC1_POSITION, pos),
                (p::CUTOFF, 20_000.0),
            ]);
            let (l, _) = rig.run(12_000, &[At(0, on(key, 1))]);
            assert!(l.iter().all(|v| v.is_finite()));
            let pk = peak(&l[2_000..]);
            assert!(
                pk > 0.02 && pk < 1.5,
                "table {table} pos {pos} key {key}: {pk}"
            );
        }
    }
}

#[test]
fn filter_cutoff_darkens_and_modes_differ() {
    let bright = |cutoff: f64, mode: f64| {
        let mut rig = Rig::new();
        rig.params(&[(p::CUTOFF, cutoff), (p::FILTER_TYPE, mode)]);
        let (l, _) = rig.run(9_600, &[At(0, on(45, 1))]);
        // Energy of the first difference ≈ high-frequency content.
        let d: Vec<f32> = l[2_400..].windows(2).map(|w| w[1] - w[0]).collect();
        (rms(&d), rms(&l[2_400..]))
    };
    let (open, _) = bright(20_000.0, 0.0);
    let (closed, level) = bright(300.0, 0.0);
    assert!(closed < open * 0.2, "{closed} vs {open}");
    assert!(level > 0.01, "low-pass keeps the fundamental");
    // High-pass at 2 kHz removes the body of a 110 Hz saw.
    let (_, hp_level) = bright(2_000.0, 2.0);
    let (_, lp_level) = bright(2_000.0, 1.0);
    assert!(hp_level < lp_level * 0.5, "{hp_level} vs {lp_level}");
}

#[test]
fn filter_envelope_opens_the_filter() {
    let run = |amount: f64| {
        let mut rig = Rig::new();
        rig.params(&[
            (p::CUTOFF, 200.0),
            (p::FILTER_ENV_AMOUNT, amount),
            (p::FILTER_SUSTAIN, 100.0),
        ]);
        let (l, _) = rig.run(9_600, &[At(0, on(45, 1))]);
        let d: Vec<f32> = l[4_800..].windows(2).map(|w| w[1] - w[0]).collect();
        rms(&d)
    };
    assert!(run(80.0) > run(0.0) * 4.0);
}

#[test]
fn polyphony_limit_steals_the_oldest_voice_without_clicks() {
    let mut rig = Rig::new();
    rig.params(&[(p::VOICES, 2.0), (p::AMP_RELEASE, 2_000.0)]);
    let (l, _) = rig.run(
        24_000,
        &[At(0, on(60, 1)), At(4_000, on(64, 2)), At(8_000, on(67, 3))],
    );
    assert_eq!(rig.synth.active_voices(), 2, "the stolen voice faded out");
    let pk = peak(&l);
    assert!(max_step(&l[7_990..8_200]) < pk * 0.3, "steal is click-free");
    // Releasing voices are stolen first.
    let mut rig = Rig::new();
    rig.params(&[(p::VOICES, 2.0), (p::AMP_RELEASE, 5_000.0)]);
    rig.run(
        9_600,
        &[
            At(0, on(60, 1)),
            At(100, on(64, 2)),
            At(200, off(64, 2)),
            At(4_000, on(67, 3)),
        ],
    );
    assert_eq!(rig.synth.active_voices(), 2);
    // Note 1 (held) survives: releasing it ends it; the other voice is note 3.
    let (l, _) = rig.run(4_800, &[At(0, off(67, 3)), At(0, off(60, 1))]);
    assert!(peak(&l) > 0.0);
}

#[test]
fn mono_and_legato_modes_use_one_voice_and_last_note_priority() {
    for mode in [1.0, 2.0] {
        let mut rig = Rig::new();
        sine_patch(&mut rig);
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
        assert!((f1 / 220.0 - 1.0).abs() < 0.003, "{mode}: {f1}");
        assert!((f2 / 440.0 - 1.0).abs() < 0.003, "{mode}: {f2}");
        assert!(
            (f3 / 220.0 - 1.0).abs() < 0.003,
            "{mode}: back to held key {f3}"
        );
    }
}

#[test]
fn legato_does_not_retrigger_but_mono_does() {
    let level_after_second_note = |mode: f64| {
        let mut rig = Rig::new();
        rig.params(&[
            (p::VOICE_MODE, mode),
            (p::AMP_ATTACK, 200.0),
            (p::AMP_SUSTAIN, 20.0),
            (p::AMP_DECAY, 50.0),
            (p::CUTOFF, 20_000.0),
        ]);
        let (l, _) = rig.run(48_000, &[At(0, on(57, 1)), At(24_000, on(60, 2))]);
        // 50 ms into the second note: legato stays at sustain, mono is in its new attack.
        rms(&l[26_000..26_400]) / rms(&l[23_000..23_400])
    };
    let legato = level_after_second_note(2.0);
    let mono = level_after_second_note(1.0);
    assert!((legato - 1.0).abs() < 0.2, "legato {legato}");
    assert!(mono > 1.5, "mono retriggers {mono}");
}

#[test]
fn glide_moves_between_pitches() {
    let mut rig = Rig::new();
    sine_patch(&mut rig);
    rig.params(&[(p::VOICE_MODE, 2.0), (p::GLIDE, 300.0)]);
    let (l, _) = rig.run(48_000, &[At(0, on(57, 1)), At(12_000, on(69, 2))]);
    let mid = frequency(&l[12_600..14_000]);
    assert!(mid > 240.0 && mid < 420.0, "gliding: {mid}");
    let end = frequency(&l[36_000..]);
    assert!((end / 440.0 - 1.0).abs() < 0.003, "arrived: {end}");
}

#[test]
fn unison_spreads_in_stereo_and_detunes() {
    let mut rig = Rig::new();
    rig.params(&[
        (p::UNISON_VOICES, 4.0),
        (p::UNISON_SPREAD, 100.0),
        (p::UNISON_DETUNE, 40.0),
    ]);
    let (l, r) = rig.run(24_000, &[At(0, on(57, 1))]);
    let diff: Vec<f32> = l.iter().zip(&r).map(|(a, b)| a - b).collect();
    assert!(rms(&diff) > rms(&l) * 0.2, "stereo");
    // Level stays comparable to a single voice (1/√n normalization).
    let mut mono = Rig::new();
    let (m, _) = mono.run(24_000, &[At(0, on(57, 1))]);
    let ratio = rms(&l[4_800..]) / rms(&m[4_800..]);
    assert!((0.5..2.0).contains(&ratio), "{ratio}");
}

#[test]
fn sustain_pedal_holds_released_notes() {
    let mut rig = Rig::new();
    rig.params(&[(p::AMP_RELEASE, 50.0)]);
    let pedal = |down: bool| EventKind::Midi {
        data: [0xB0, 64, if down { 127 } else { 0 }],
    };
    let (l, _) = rig.run(
        48_000,
        &[
            At(0, pedal(true)),
            At(0, on(60, 1)),
            At(4_800, off(60, 1)),
            At(24_000, pedal(false)),
        ],
    );
    assert!(rms(&l[20_000..23_000]) > 0.01, "held by the pedal");
    assert!(peak(&l[30_000..]) < 1e-3, "released with the pedal");
}

#[test]
fn lfo_and_mod_envelope_targets_modulate() {
    // Vibrato: pitch varies over time.
    let mut rig = Rig::new();
    sine_patch(&mut rig);
    rig.params(&[
        (p::LFO1_TARGET, 1.0),
        (p::LFO1_AMOUNT, 60.0),
        (p::LFO1_RATE, 4.0),
    ]);
    let (l, _) = rig.run(48_000, &[At(0, on(69, 1))]);
    let a = frequency(&l[0..6_000]);
    let b = frequency(&l[6_000..12_000]);
    assert!((a - b).abs() > 5.0, "{a} vs {b}");
    // Tremolo (amp) and pan.
    for (target, stereo) in [(3.0, false), (4.0, true)] {
        let mut rig = Rig::new();
        sine_patch(&mut rig);
        rig.params(&[
            (p::LFO2_TARGET, target),
            (p::LFO2_AMOUNT, 100.0),
            (p::LFO2_RATE, 5.0),
        ]);
        let (l, r) = rig.run(48_000, &[At(0, on(69, 1))]);
        let segs: Vec<f32> = l.chunks(2_400).skip(1).map(rms).collect();
        let (lo, hi) = segs
            .iter()
            .fold((f32::MAX, 0.0f32), |(a, b), &v| (a.min(v), b.max(v)));
        assert!(hi > lo * 2.0, "target {target}: {lo}..{hi}");
        assert_eq!(l != r, stereo);
    }
    // Mod envelope → pitch: starts high, settles.
    let mut rig = Rig::new();
    sine_patch(&mut rig);
    rig.params(&[
        (p::MOD_ENV_TARGET, 1.0),
        (p::MOD_ENV_AMOUNT, 50.0),
        (p::MOD_DECAY, 100.0),
    ]);
    let (l, _) = rig.run(48_000, &[At(0, on(57, 1))]);
    assert!(frequency(&l[0..1_200]) > 300.0);
    assert!((frequency(&l[24_000..]) / 220.0 - 1.0).abs() < 0.003);
}

#[test]
fn synced_lfo_locks_to_the_song_position() {
    // 1/4 at 120 bpm = 2 Hz, starting at beat 0.
    let mut rig = Rig::new();
    sine_patch(&mut rig);
    rig.params(&[
        (p::LFO1_SYNC, 1.0),
        (p::LFO1_SYNC_RATE, 8.0),
        (p::LFO1_TARGET, 3.0),
        (p::LFO1_AMOUNT, 100.0),
        (p::LFO1_SHAPE, 3.0),
    ]);
    let (l, _) = rig.run(48_000, &[At(0, on(69, 1))]);
    // Square LFO: loud in the first half of each beat (a beat is 24 000 samples), silent in
    // the second.
    let loud = rms(&l[25_000..35_000]);
    let quiet = rms(&l[37_000..47_000]);
    assert!(loud > quiet * 5.0, "{loud} vs {quiet}");
}

#[test]
fn extreme_and_random_params_stay_finite_and_bounded() {
    let desc = ether_devices::descriptor(BuiltinDeviceType::PolySynth);
    let mut seed = 12_345u32;
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        f64::from(seed) / f64::from(u32::MAX)
    };
    for round in 0..12 {
        let mut rig = Rig::new();
        for info in &desc.params {
            let v = match round {
                0 => info.min,
                1 => info.max,
                _ => info.min + (info.max - info.min) * rnd(),
            };
            rig.synth.set_param(info.id, v);
        }
        rig.synth.set_param(p::UNISON_VOICES, 8.0);
        let events: Vec<At> = (0..12)
            .map(|i| At(i * 400, on(24 + (i as u8) * 7, i as u32)))
            .collect();
        let (l, r) = rig.run(24_000, &events);
        assert!(
            l.iter().chain(&r).all(|v| v.is_finite()),
            "round {round}: NaN"
        );
        let pk = peak(&l).max(peak(&r));
        assert!(pk < 8.0, "round {round}: peak {pk}");
    }
    // NaN params fall back to the default.
    let mut s = PolySynth::new();
    s.set_param(p::CUTOFF, f64::NAN);
    assert_eq!(s.param(p::CUTOFF), Some(8000.0));
    s.set_param(p::CUTOFF, 1e9);
    assert_eq!(s.param(p::CUTOFF), Some(20_000.0));
}

/// Control chunks sit on the absolute sample clock, so any block size that is a multiple of
/// the control chunk (16) renders identically (other sizes differ only in the control-rate
/// interpolation phase).
#[test]
fn render_is_independent_of_the_block_size() {
    let events = [
        At(0, set(p::UNISON_VOICES, 3.0)),
        At(10, on(60, 1)),
        At(333, on(64, 2)),
        At(1_000, set(p::CUTOFF, 1_200.0)),
        At(1_234, set(p::OSC1_TYPE, 4.0)),
        At(5_000, off(60, 1)),
        At(7_777, off(64, 2)),
    ];
    let render = |block: usize| {
        let mut rig = Rig::with_block(block);
        rig.run(12_000, &events)
    };
    let a = render(512);
    for block in [16, 64, 128, 256] {
        assert_eq!(render(block), a, "block {block}");
    }
}

#[test]
fn param_events_apply_at_their_sample() {
    let mut rig = Rig::new();
    sine_patch(&mut rig);
    // Osc level to off at sample 5 000 (ramped over 5 ms).
    let (l, _) = rig.run(
        12_000,
        &[At(0, on(69, 1)), At(5_000, set(p::OSC1_LEVEL, -70.0))],
    );
    assert!(rms(&l[4_000..5_000]) > 0.05);
    assert!(peak(&l[5_300..]) < 1e-6);
}

#[test]
fn factory_presets_load_and_sound() {
    let presets = ether_devices::factory_presets(BuiltinDeviceType::PolySynth);
    assert!(
        (8..=12).contains(&presets.len()),
        "{} presets",
        presets.len()
    );
    for fp in presets {
        let preset = load_preset(fp.json).unwrap();
        let mut rig = Rig::new();
        for (id, v) in &preset.params {
            rig.synth.set_param(*id, *v);
        }
        // A chord, held 1 s.
        let (l, r) = rig.run(
            72_000,
            &[
                At(0, on(48, 1)),
                At(0, on(55, 2)),
                At(0, on(60, 3)),
                At(0, on(64, 4)),
                At(48_000, off(48, 1)),
                At(48_000, off(55, 2)),
                At(48_000, off(60, 3)),
                At(48_000, off(64, 4)),
            ],
        );
        let pk = peak(&l).max(peak(&r));
        assert!(
            (0.05..1.0).contains(&pk),
            "{}: chord peak {pk} ({:.1} dBFS)",
            fp.id,
            20.0 * pk.log10()
        );
        assert!(l.iter().chain(&r).all(|v| v.is_finite()), "{}", fp.id);
    }
}

/// CPU budget: 16 voices × 4 unison (two oscillators, one a wavetable, ladder filter with
/// drive, all modulation on) must render far faster than realtime.
#[test]
fn cpu_budget_16_voices_x4_unison() {
    let mut rig = Rig::with_block(256);
    rig.params(&[
        (p::OSC2_TYPE, 4.0),
        (p::OSC2_TABLE, 1.0),
        (p::OSC2_LEVEL, -3.0),
        (p::SUB_LEVEL, -12.0),
        (p::NOISE_LEVEL, -30.0),
        (p::UNISON_VOICES, 4.0),
        (p::DRIVE, 6.0),
        (p::RESONANCE, 40.0),
        (p::FILTER_ENV_AMOUNT, 40.0),
        (p::LFO1_TARGET, 2.0),
        (p::LFO1_AMOUNT, 30.0),
        (p::LFO2_TARGET, 4.0),
        (p::LFO2_AMOUNT, 30.0),
        (p::AMP_RELEASE, 5_000.0),
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
    let elapsed = t.elapsed().as_secs_f32();
    let load = elapsed / seconds;
    println!(
        "poly synth: 16 voices x 4 unison, {seconds} s rendered in {:.1} ms = {:.1} % of realtime ({:.0}x realtime)",
        elapsed * 1000.0,
        load * 100.0,
        1.0 / load
    );
    // Generous: shared, loaded CI machines and the opt-level 1 dev profile.
    assert!(load < 0.5, "{:.1} % of realtime", load * 100.0);
}

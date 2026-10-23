//! The Poly Synth's unison (1..=16 voices, detune in cents,
//! blend), drift, the legacy unison params of old documents, and that patches written before
//! this pass render as they did (fingerprints recorded on the engine before the change).
//!
//! `cargo test -p ether-devices --test it instruments_poly_synth:: -- --nocapture` also prints the
//! CPU cost of 16 voices × 16 unison.

use assert_no_alloc::assert_no_alloc;
use ether_core::protocol::model::ParamId;
use ether_core::{
    AudioBuffers, Device, EventBuffer, EventKind, Node, PrepareConfig, ProcessContext,
    ProcessEvent, TransportInfo,
};
use ether_devices::poly_synth::PolySynth;

const SR: f32 = 48_000.0;

/// Param ids by number, so the golden cases also compile against the engine before this
/// pass (when the fingerprints were recorded).
mod id {
    use ether_core::protocol::model::ParamId;
    pub const OSC1_TYPE: ParamId = ParamId(0);
    pub const OSC1_LEVEL: ParamId = ParamId(6);
    pub const OSC2_TYPE: ParamId = ParamId(7);
    pub const OSC2_LEVEL: ParamId = ParamId(13);
    pub const SUB_LEVEL: ParamId = ParamId(14);
    pub const NOISE_LEVEL: ParamId = ParamId(16);
    pub const LEGACY_UNISON: ParamId = ParamId(18);
    pub const LEGACY_DETUNE: ParamId = ParamId(19);
    pub const SPREAD: ParamId = ParamId(20);
    pub const VOICES: ParamId = ParamId(21);
    pub const FILTER_TYPE: ParamId = ParamId(25);
    pub const CUTOFF: ParamId = ParamId(26);
    pub const RESONANCE: ParamId = ParamId(27);
    pub const DRIVE: ParamId = ParamId(28);
    pub const AMP_SUSTAIN: ParamId = ParamId(33);
    pub const AMP_RELEASE: ParamId = ParamId(34);
    pub const FILTER_ENV_AMOUNT: ParamId = ParamId(30);
    pub const LFO1_TARGET: ParamId = ParamId(49);
    pub const LFO1_AMOUNT: ParamId = ParamId(50);
    pub const OSC2_TABLE: ParamId = ParamId(60);
    // Added by this pass.
    pub const UNISON_VOICES: ParamId = ParamId(61);
    pub const UNISON_DETUNE: ParamId = ParamId(62);
    pub const UNISON_BLEND: ParamId = ParamId(63);
    pub const DRIFT: ParamId = ParamId(64);
}

#[derive(Clone, Copy)]
struct At(u64, EventKind);

fn on(key: u8, note_id: u32) -> EventKind {
    EventKind::NoteOn {
        note_id,
        channel: 0,
        key,
        velocity: 1.0,
    }
}

fn off(key: u8, note_id: u32) -> EventKind {
    EventKind::NoteOff {
        note_id,
        channel: 0,
        key,
        velocity: 0.0,
    }
}

fn set(param: ParamId, value: f64) -> EventKind {
    EventKind::Param {
        param,
        value,
        step: false,
    }
}

struct Rig {
    synth: PolySynth,
    time: u64,
    block: usize,
    out_events: EventBuffer,
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
        }
    }

    fn params(&mut self, values: &[(ParamId, f64)]) -> &mut Self {
        for &(id, v) in values {
            self.synth.set_param(id, v);
        }
        self
    }

    /// Render `frames` samples with `events` (times from now), in blocks, never allocating.
    fn run(&mut self, frames: usize, events: &[At]) -> (Vec<f32>, Vec<f32>) {
        let start = self.time;
        let (mut l, mut r) = (vec![0.0f32; frames], vec![0.0f32; frames]);
        let mut evs = EventBuffer::with_capacity(256);
        let mut done = 0;
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
            evs.sort();
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
            assert_no_alloc(|| synth.process(&mut ctx, &mut buffers));
            done += n;
        }
        self.time += frames as u64;
        (l, r)
    }
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// Magnitude of `x` at `hz` (Goertzel over the whole slice, Hann window), normalised so a
/// full-scale sine reads about 0.5.
fn magnitude(x: &[f32], hz: f32) -> f32 {
    let n = x.len();
    let w = std::f64::consts::TAU * f64::from(hz) / f64::from(SR);
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (i, &s) in x.iter().enumerate() {
        let win = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / (n - 1) as f64).cos();
        let v = f64::from(s) * win;
        re += v * (w * i as f64).cos();
        im -= v * (w * i as f64).sin();
    }
    ((re * re + im * im).sqrt() * 2.0 / n as f64) as f32
}

fn cents(hz: f32, c: f32) -> f32 {
    hz * (c / 1200.0).exp2()
}

/// A clean sine voice: open low-pass, no resonance, full sustain.
fn sine(rig: &mut Rig) -> &mut Rig {
    rig.params(&[
        (id::OSC1_TYPE, 3.0),
        (id::CUTOFF, 20_000.0),
        (id::RESONANCE, 0.0),
        (id::FILTER_TYPE, 1.0),
        (id::AMP_SUSTAIN, 100.0),
    ])
}

/// One held A4 (440 Hz), 2 s; the steady part (after the attack).
fn held_a4(rig: &mut Rig) -> (Vec<f32>, Vec<f32>) {
    let (l, r) = rig.run(2 * SR as usize, &[At(0, on(69, 1))]);
    (l[4_800..].to_vec(), r[4_800..].to_vec())
}

// --- unison -----------------------------------------------------------------------------

#[test]
fn unison_voices_and_detune_show_on_the_spectrum() {
    // 3 copies at ±50 cents: three partials at 440·2^(-50/1200), 440, 440·2^(50/1200).
    let mut rig = Rig::new();
    sine(&mut rig).params(&[
        (id::UNISON_VOICES, 3.0),
        (id::UNISON_DETUNE, 50.0),
        (id::SPREAD, 0.0),
    ]);
    let (l, r) = held_a4(&mut rig);
    assert_eq!(l, r, "no spread: mono");
    let floor = magnitude(&l, 434.0).max(magnitude(&l, 446.0));
    for hz in [cents(440.0, -50.0), 440.0, cents(440.0, 50.0)] {
        let m = magnitude(&l, hz);
        assert!(m > 20.0 * floor, "{hz:.1} Hz: {m} (floor {floor})");
    }
    // Nothing beyond the outermost copies.
    assert!(magnitude(&l, cents(440.0, 90.0)) < 0.05 * magnitude(&l, 440.0));

    // 16 copies at ±100 cents: the outermost ones are a semitone away, evenly spaced between.
    let mut rig = Rig::new();
    sine(&mut rig).params(&[(id::UNISON_VOICES, 16.0), (id::UNISON_DETUNE, 100.0)]);
    let (l, _) = held_a4(&mut rig);
    let step = 200.0 / 15.0;
    let partials: Vec<f32> = (0..16)
        .map(|u| magnitude(&l, cents(440.0, -100.0 + step * u as f32)))
        .collect();
    let mean = partials.iter().sum::<f32>() / 16.0;
    for (u, m) in partials.iter().enumerate() {
        assert!(*m > 0.3 * mean, "copy {u}: {m} vs mean {mean}");
    }
    assert!(magnitude(&l, cents(440.0, 140.0)) < 0.1 * mean);
}

#[test]
fn unison_level_is_normalised_and_the_mono_sum_holds_up() {
    let level = |voices: f64, spread: f64| {
        let mut rig = Rig::new();
        rig.params(&[
            (id::UNISON_VOICES, voices),
            (id::UNISON_DETUNE, 30.0),
            (id::SPREAD, spread),
        ]);
        let (l, r) = held_a4(&mut rig);
        let mono: Vec<f32> = l.iter().zip(&r).map(|(a, b)| 0.5 * (a + b)).collect();
        let side: Vec<f32> = l.iter().zip(&r).map(|(a, b)| 0.5 * (a - b)).collect();
        (rms(&l).max(rms(&r)), rms(&mono), rms(&side))
    };
    let (one, ..) = level(1.0, 0.0);
    for voices in [2.0, 4.0, 8.0, 16.0] {
        let (stereo, mono, side) = level(voices, 100.0);
        // Random-phase copies sum in power: about the single voice's level.
        let ratio = stereo / one;
        assert!(
            (0.5..2.0).contains(&ratio),
            "{voices} voices: level ×{ratio}"
        );
        // Wide but mono-compatible: the mid stays well above the side, never cancels.
        assert!(
            mono > 0.5 * one,
            "{voices} voices: mono sum ×{}",
            mono / one
        );
        assert!(side > 0.05 * one, "{voices} voices: no width");
    }
}

#[test]
fn blend_fades_the_side_copies_against_the_centre() {
    let render = |blend: f64| {
        let mut rig = Rig::new();
        sine(&mut rig).params(&[
            (id::UNISON_VOICES, 3.0),
            (id::UNISON_DETUNE, 50.0),
            (id::SPREAD, 0.0),
            (id::UNISON_BLEND, blend),
        ]);
        held_a4(&mut rig).0
    };
    let full = render(100.0);
    let centre = render(0.0);
    let half = render(50.0);
    let side = |x: &[f32]| magnitude(x, cents(440.0, 50.0));
    // 0 %: the centre copy only.
    assert!(side(&centre) < 0.01 * magnitude(&centre, 440.0));
    // 50 %: the sides at half the centre's level.
    let ratio = side(&half) / magnitude(&half, 440.0);
    assert!((0.4..0.6).contains(&ratio), "{ratio}");
    // Level kept (RMS normalisation).
    let r = rms(&centre) / rms(&full);
    assert!((0.6..1.6).contains(&r), "{r}");
    // Even counts: the two middle copies are the centre. (Quieter: the filter's soft
    // saturation makes 3rd-order intermodulation of the two right where the outer ones sit.)
    let mut rig = Rig::new();
    sine(&mut rig).params(&[
        (id::OSC1_LEVEL, -24.0),
        (id::UNISON_VOICES, 4.0),
        (id::UNISON_DETUNE, 60.0),
        (id::SPREAD, 0.0),
        (id::UNISON_BLEND, 0.0),
    ]);
    let (l, _) = held_a4(&mut rig);
    let inner = magnitude(&l, cents(440.0, 20.0));
    assert!(inner > 0.003, "{inner}");
    let outer = magnitude(&l, cents(440.0, 60.0));
    assert!(outer < 0.01 * inner, "outer {outer} vs inner {inner}");
}

#[test]
fn legacy_unison_params_keep_old_documents_sounding() {
    // An old document: Unison (legacy) 4, Detune (legacy) 50 % = 25 cents at the outside.
    let legacy = |extra: &[(ParamId, f64)]| {
        let mut rig = Rig::new();
        sine(&mut rig).params(&[
            (id::LEGACY_UNISON, 4.0),
            (id::LEGACY_DETUNE, 50.0),
            (id::SPREAD, 0.0),
        ]);
        rig.params(extra);
        held_a4(&mut rig).0
    };
    let l = legacy(&[]);
    for c in [-25.0, -25.0 / 3.0, 25.0 / 3.0, 25.0] {
        assert!(magnitude(&l, cents(440.0, c)) > 0.05, "{c} cents");
    }
    // The new control takes over once above 1, with its own detune.
    let l = legacy(&[(id::UNISON_VOICES, 2.0), (id::UNISON_DETUNE, 80.0)]);
    assert!(magnitude(&l, cents(440.0, 80.0)) > 0.05);
    assert!(magnitude(&l, cents(440.0, 25.0)) < 0.01);
}

// --- drift ------------------------------------------------------------------------------

/// Pitch (Hz) over successive windows, from zero crossings.
fn pitch_track(x: &[f32], window: usize) -> Vec<f32> {
    x.chunks(window)
        .map(|w| {
            let c: Vec<f32> = (1..w.len())
                .filter(|&i| w[i - 1] <= 0.0 && w[i] > 0.0)
                .map(|i| (i - 1) as f32 + w[i - 1] / (w[i - 1] - w[i]))
                .collect();
            (c.len() - 1) as f32 * SR / (c[c.len() - 1] - c[0])
        })
        .collect()
}

#[test]
fn drift_wanders_within_its_range_and_off_is_exact() {
    let track = |drift: f64, note: u32| {
        let mut rig = Rig::new();
        sine(&mut rig).params(&[(id::DRIFT, drift)]);
        let (l, _) = rig.run(3 * SR as usize, &[At(0, on(69, note))]);
        pitch_track(&l[4_800..], 4_800)
    };
    for hz in track(0.0, 1) {
        assert!((hz / 440.0 - 1.0).abs() < 2e-4, "no drift: {hz}");
    }
    let t = track(100.0, 1);
    let dev: Vec<f32> = t.iter().map(|hz| 1200.0 * (hz / 440.0).log2()).collect();
    let (lo, hi) = dev
        .iter()
        .fold((f32::MAX, f32::MIN), |(a, b), &c| (a.min(c), b.max(c)));
    assert!(lo > -15.5 && hi < 15.5, "within ±15 cents: {lo}..{hi}");
    assert!(hi - lo > 1.0, "it moves: {lo}..{hi}");
}

#[test]
fn drift_never_changes_the_noise_or_other_random_sources() {
    // Noise only (osc 1 off): drift moves pitches, so the output must not change at all.
    let render = |drift: f64| {
        let mut rig = Rig::new();
        rig.params(&[
            (id::OSC1_LEVEL, -70.0),
            (id::NOISE_LEVEL, -6.0),
            (id::UNISON_VOICES, 4.0),
            (id::DRIFT, drift),
        ]);
        rig.run(24_000, &[At(0, on(60, 1)), At(100, on(67, 2))])
    };
    assert_eq!(render(0.0), render(100.0));
}

// --- engine contracts -------------------------------------------------------------------

#[test]
fn new_params_render_independently_of_the_block_size() {
    let events = [
        At(0, set(id::UNISON_VOICES, 16.0)),
        At(0, set(id::DRIFT, 60.0)),
        At(10, on(60, 1)),
        At(333, on(64, 2)),
        At(1_000, set(id::UNISON_DETUNE, 70.0)),
        At(2_000, set(id::UNISON_BLEND, 30.0)),
        At(5_000, off(60, 1)),
        At(7_777, off(64, 2)),
    ];
    let render = |block: usize| Rig::with_block(block).run(12_000, &events);
    let a = render(512);
    for block in [16, 64, 256] {
        assert_eq!(render(block), a, "block {block}");
    }
}

#[test]
fn extreme_unison_and_drift_stay_finite_and_click_free() {
    let mut rig = Rig::new();
    rig.params(&[
        (id::UNISON_VOICES, 16.0),
        (id::UNISON_DETUNE, 100.0),
        (id::UNISON_BLEND, 0.0),
        (id::DRIFT, 100.0),
        (id::SPREAD, 100.0),
        (id::RESONANCE, 90.0),
        (id::DRIVE, 24.0),
    ]);
    let events: Vec<At> = (0..8u8)
        .map(|i| At(u64::from(i) * 500, on(30 + i * 9, u32::from(i))))
        .chain([
            At(4_000, set(id::UNISON_BLEND, 100.0)),
            At(6_000, set(id::UNISON_DETUNE, 0.0)),
            At(8_000, set(id::UNISON_VOICES, 1.0)),
        ])
        .collect();
    let (l, r) = rig.run(24_000, &events);
    assert!(l.iter().chain(&r).all(|v| v.is_finite() && v.abs() < 4.0));
}

/// CPU per voice at 16 unison (2 oscillators, one a wavetable, ladder filter with drive,
/// modulation on): printed for the PR, bounded loosely (shared machines, dev profile).
#[test]
fn cpu_16_voices_x16_unison() {
    let mut rig = Rig::new();
    rig.params(&[
        (id::OSC2_TYPE, 4.0),
        (id::OSC2_TABLE, 1.0),
        (id::OSC2_LEVEL, -3.0),
        (id::SUB_LEVEL, -12.0),
        (id::UNISON_VOICES, 16.0),
        (id::UNISON_DETUNE, 25.0),
        (id::SPREAD, 80.0),
        (id::DRIFT, 30.0),
        (id::DRIVE, 6.0),
        (id::RESONANCE, 40.0),
        (id::FILTER_ENV_AMOUNT, 40.0),
        (id::LFO1_TARGET, 2.0),
        (id::LFO1_AMOUNT, 30.0),
        (id::AMP_RELEASE, 5_000.0),
    ]);
    let notes: Vec<At> = (0..16u8)
        .map(|i| At(0, on(36 + i * 3, u32::from(i))))
        .collect();
    rig.run(4_800, &notes);
    assert_eq!(rig.synth.active_voices(), 16);
    let seconds = 3.0;
    let t = std::time::Instant::now();
    rig.run((SR * seconds) as usize, &[]);
    let load = t.elapsed().as_secs_f32() / seconds;
    println!(
        "poly synth: 16 voices x 16 unison (32 oscillators per voice) = {:.1} % of one core in realtime, {:.2} % per voice",
        load * 100.0,
        load * 100.0 / 16.0
    );
    assert!(load < 2.0, "{:.1} % of realtime", load * 100.0);
}

// --- patches from before this pass render as before ----------------------------------------

/// Render fingerprint: RMS of each 1 000-sample block of both channels (robust across
/// platforms' libm), plus a hash of the exact sample bits.
fn fingerprint(rig: &mut Rig) -> (Vec<f32>, u64) {
    let events = [
        At(0, on(48, 1)),
        At(0, on(55, 2)),
        At(2_000, on(64, 3)),
        At(30_000, off(48, 1)),
        At(30_000, off(55, 2)),
        At(40_000, off(64, 3)),
    ];
    let (l, r) = rig.run(48_000, &events);
    let blocks = l
        .chunks(1_000)
        .zip(r.chunks(1_000))
        .map(|(a, b)| rms(a) + rms(b))
        .collect();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in l.iter().chain(&r) {
        h ^= u64::from(v.to_bits());
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    (blocks, h)
}

/// Patches using only the params that existed before this pass.
fn golden_cases() -> Vec<(&'static str, Vec<(ParamId, f64)>)> {
    vec![
        ("default", vec![]),
        (
            "legacy unison 4",
            vec![
                (id::LEGACY_UNISON, 4.0),
                (id::LEGACY_DETUNE, 40.0),
                (id::SPREAD, 100.0),
            ],
        ),
        (
            "legacy unison 8, wavetable, drive",
            vec![
                (id::LEGACY_UNISON, 8.0),
                (id::LEGACY_DETUNE, 70.0),
                (id::OSC2_TYPE, 4.0),
                (id::OSC2_TABLE, 3.0),
                (id::OSC2_LEVEL, -6.0),
                (id::SUB_LEVEL, -12.0),
                (id::NOISE_LEVEL, -30.0),
                (id::DRIVE, 12.0),
                (id::LFO1_TARGET, 2.0),
                (id::LFO1_AMOUNT, 40.0),
                (id::VOICES, 2.0),
            ],
        ),
    ]
}

/// Recorded on `origin/dev` before this pass (`print_golden_fingerprints`); re-recorded
/// for the 4-point polyBLEP oscillators and the 2x oversampled filter.
#[rustfmt::skip]
const GOLDEN: &[(&str, &[f32], u64)] = &[
    ("default", &[0.3002643, 0.26518026, 0.3389902, 0.31011173, 0.31515813, 0.31706822, 0.28983334, 0.28866914, 0.31129774, 0.26713875, 0.28671327, 0.321584, 0.27498728, 0.32821915, 0.2858537, 0.26539105, 0.30687112, 0.29097077, 0.2702137, 0.29318905, 0.28614786, 0.28948846, 0.3000582, 0.29284236, 0.2840309, 0.28651294, 0.272141, 0.30923036, 0.26799667, 0.25994703, 0.26218393, 0.18486744, 0.17156318, 0.16877808, 0.16692667, 0.16628906, 0.1656809, 0.16499479, 0.16315301, 0.16280055, 0.12491576, 0.06651669, 0.034982793, 0.018247554, 0.009499569, 0.0049574543, 0.0026168479, 0.0014020706], 0x8ae6d7df30e6b62d),
    ("legacy unison 4", &[0.362733, 0.29237786, 0.41869134, 0.38401037, 0.33543858, 0.3126809, 0.28338337, 0.25821248, 0.2546066, 0.22996426, 0.21333174, 0.2320509, 0.23708808, 0.22525902, 0.24823664, 0.22126403, 0.19680697, 0.204947, 0.20885688, 0.23495306, 0.23294029, 0.26025814, 0.28147775, 0.2973677, 0.36897272, 0.33159176, 0.3383096, 0.32226378, 0.3066861, 0.2821773, 0.21996503, 0.14805615, 0.12111424, 0.11192137, 0.114247605, 0.12626964, 0.14155713, 0.14145976, 0.123031005, 0.106719986, 0.07646805, 0.040387154, 0.023508724, 0.014232116, 0.008831913, 0.005361679, 0.003265657, 0.0019213168], 0x4d46b221eba81dba),
    ("legacy unison 8, wavetable, drive", &[0.22096568, 0.2130056, 0.20989168, 0.19752344, 0.15682055, 0.13768111, 0.14673117, 0.17005858, 0.20200565, 0.23088163, 0.18153661, 0.17834283, 0.17983612, 0.14290178, 0.1419018, 0.14076254, 0.14151804, 0.16127619, 0.1736159, 0.17946415, 0.18759838, 0.15204817, 0.14780529, 0.14949378, 0.17800239, 0.16395894, 0.18839431, 0.18747583, 0.1668625, 0.13360223, 0.119931504, 0.096393645, 0.101902016, 0.091026455, 0.1016261, 0.12346933, 0.124992445, 0.1128833, 0.13657291, 0.12700649, 0.07203835, 0.037517846, 0.025403403, 0.013304228, 0.008764112, 0.00439497, 0.0019611083, 0.0008222727], 0x87835f4e685ef089),
];

#[test]
#[ignore = "prints the fingerprints to paste into GOLDEN"]
fn print_golden_fingerprints() {
    for (name, params) in golden_cases() {
        let mut rig = Rig::new();
        rig.params(&params);
        let (blocks, hash) = fingerprint(&mut rig);
        println!("    ({name:?}, &{blocks:?}, {hash:#x}),");
    }
}

#[test]
fn patches_from_before_render_as_before() {
    assert_eq!(GOLDEN.len(), golden_cases().len());
    for ((name, params), (gname, blocks, hash)) in golden_cases().into_iter().zip(GOLDEN) {
        assert_eq!(name, *gname);
        let mut rig = Rig::new();
        rig.params(&params);
        let (now, now_hash) = fingerprint(&mut rig);
        for (i, (a, b)) in now.iter().zip(blocks.iter()).enumerate() {
            assert!(
                (a - b).abs() <= 1e-5 + 1e-4 * b.abs(),
                "{name}: block {i}: {a} vs {b}"
            );
        }
        // Bit-exact where the recording was made (same libm).
        if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            assert_eq!(now_hash, *hash, "{name}: not bit-exact");
        }
    }
}

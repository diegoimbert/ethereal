//! Audio → MIDI accuracy (`audio-to-midi`, CONTRACTS.md §13.5) on fixtures with a known
//! ground truth: note lists ("MIDI") rendered here by small synths, then detected, then
//! scored like `mir_eval`'s onset-only note metrics: a detected note matches an unmatched
//! reference note of the **same key** whose onset is within the tolerance (one-to-one,
//! closest first). Precision = matched / detected, recall = matched / reference.
//!
//! The acceptance fixtures (sine melody, sine chords, kick/snare/hat loop) must be exact at
//! 20 ms; the richer ones (sawtooth lead with vibrato, a low bass line, piano-like chords
//! with decaying inharmonic partials, a busier drum loop over noise) have floors. Run with
//! `--nocapture` for the table (`ACCURACY.md`-style rows are printed for the PR).

use std::f64::consts::PI;
use std::sync::Arc;
use std::time::Instant;

use ether_media::DecodedAudio;
use ether_media::to_midi::{DetectedNote, Detector, Mode, Options};

// ─── ground truth ───────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug)]
struct Ref {
    start: f64,
    duration: f64,
    pitch: u8,
    velocity: f32,
}

fn r(start: f64, duration: f64, pitch: u8, velocity: f32) -> Ref {
    Ref {
        start,
        duration,
        pitch,
        velocity,
    }
}

/// Notes on a beat grid: `(beat, beats, key)` at `bpm`.
fn grid(bpm: f64, notes: &[(f64, f64, u8)], velocity: f32) -> Vec<Ref> {
    let b = 60.0 / bpm;
    notes
        .iter()
        .map(|&(s, d, p)| r(0.1 + s * b, d * b, p, velocity))
        .collect()
}

#[derive(Debug, Default)]
struct Score {
    reference: usize,
    detected: usize,
    matched: usize,
    mean_err_ms: f64,
    max_err_ms: f64,
}

impl Score {
    fn precision(&self) -> f64 {
        if self.detected == 0 {
            1.0
        } else {
            self.matched as f64 / self.detected as f64
        }
    }
    fn recall(&self) -> f64 {
        if self.reference == 0 {
            1.0
        } else {
            self.matched as f64 / self.reference as f64
        }
    }
    fn f1(&self) -> f64 {
        let (p, r) = (self.precision(), self.recall());
        if p + r == 0.0 {
            0.0
        } else {
            2.0 * p * r / (p + r)
        }
    }
}

fn score(reference: &[Ref], detected: &[DetectedNote], tol: f64) -> Score {
    let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
    for (i, g) in reference.iter().enumerate() {
        for (j, d) in detected.iter().enumerate() {
            let e = (d.start - g.start).abs();
            if d.pitch == g.pitch && e <= tol {
                pairs.push((e, i, j));
            }
        }
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (mut gi, mut dj) = (vec![false; reference.len()], vec![false; detected.len()]);
    let mut errs = Vec::new();
    for (e, i, j) in pairs {
        if !gi[i] && !dj[j] {
            gi[i] = true;
            dj[j] = true;
            errs.push(e * 1000.0);
        }
    }
    Score {
        reference: reference.len(),
        detected: detected.len(),
        matched: errs.len(),
        mean_err_ms: if errs.is_empty() {
            0.0
        } else {
            errs.iter().sum::<f64>() / errs.len() as f64
        },
        max_err_ms: errs.iter().copied().fold(0.0, f64::max),
    }
}

// ─── renderers ──────────────────────────────────────────────────────────────────────────

/// Deterministic white noise in -1..1.
struct Noise(u64);
impl Noise {
    fn next(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 40) as f32 / (1u64 << 23) as f32) - 1.0
    }
}

#[derive(Clone, Copy)]
enum Voice {
    /// Pure sine, 3 ms ramps.
    Sine,
    /// Band-limited sawtooth with an ADSR and a 5.5 Hz ±`vibrato` semitone vibrato.
    Saw { vibrato: f64 },
    /// Struck string: partials decaying faster with their order, slightly sharp
    /// (stiffness), 50 ms release.
    Piano,
}

fn render(sr: u32, len: f64, notes: &[Ref], voice: Voice) -> Arc<DecodedAudio> {
    let n = (len * sr as f64) as usize;
    let mut x = vec![0.0f32; n];
    let srf = sr as f64;
    for g in notes {
        let f = 440.0 * 2f64.powf((g.pitch as f64 - 69.0) / 12.0);
        let a = (g.start * srf) as usize;
        let off = g.duration;
        let tail = match voice {
            Voice::Sine => 0.0,
            Voice::Saw { .. } => 0.06,
            Voice::Piano => 0.05,
        };
        let b = (((g.start + off + tail) * srf) as usize).min(n);
        let amp = 0.25 * g.velocity as f64;
        let mut phase = 0.0f64;
        for (i, out) in x.iter_mut().enumerate().take(b).skip(a) {
            let t = (i - a) as f64 / srf;
            let v = match voice {
                Voice::Sine => {
                    let ramp = (t / 0.003).min(1.0).min((off - t).max(0.0) / 0.003);
                    ramp * (2.0 * PI * f * t).sin()
                }
                Voice::Saw { vibrato } => {
                    let fv = f * 2f64.powf(vibrato * (2.0 * PI * 5.5 * t).sin() / 12.0);
                    phase += fv / srf;
                    let env = adsr(t, off, 0.005, 0.08, 0.7, 0.06);
                    let mut s = 0.0;
                    let mut h = 1.0;
                    while h * fv < 0.45 * srf && h <= 30.0 {
                        s += (2.0 * PI * h * phase).sin() / h;
                        h += 1.0;
                    }
                    env * 0.6 * s
                }
                Voice::Piano => {
                    let attack = (t / 0.002).min(1.0);
                    let release = if t > off {
                        (1.0 - (t - off) / 0.05).max(0.0)
                    } else {
                        1.0
                    };
                    let mut s = 0.0;
                    for h in 1..=12 {
                        let h = h as f64;
                        let fh = h * f * (1.0 + 1e-4 * h * h).sqrt();
                        if fh > 0.45 * srf {
                            break;
                        }
                        s += (2.0 * PI * fh * t).sin() * (-(1.2 + 0.7 * h) * t).exp() / h.powf(1.3);
                    }
                    attack * release * s
                }
            };
            *out += (amp * v) as f32;
        }
    }
    Arc::new(DecodedAudio {
        sample_rate: sr,
        channels: vec![x.clone(), x],
    })
}

fn adsr(t: f64, off: f64, a: f64, d: f64, s: f64, r: f64) -> f64 {
    let level = |t: f64| {
        if t < a {
            t / a
        } else if t < a + d {
            1.0 - (1.0 - s) * (t - a) / d
        } else {
            s
        }
    };
    if t < off {
        level(t)
    } else {
        level(off) * (1.0 - (t - off) / r).max(0.0)
    }
}

/// A one-pole-pair biquad (RBJ), for the drum noise.
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    z: [f64; 2],
}
impl Biquad {
    fn new(kind: &str, f: f64, q: f64, sr: f64) -> Self {
        let w = 2.0 * PI * f / sr;
        let (cw, sw) = (w.cos(), w.sin());
        let alpha = sw / (2.0 * q);
        let (b, a0, a1, a2) = match kind {
            "hp" => (
                [(1.0 + cw) / 2.0, -(1.0 + cw), (1.0 + cw) / 2.0],
                1.0 + alpha,
                -2.0 * cw,
                1.0 - alpha,
            ),
            "lp" => (
                [(1.0 - cw) / 2.0, 1.0 - cw, (1.0 - cw) / 2.0],
                1.0 + alpha,
                -2.0 * cw,
                1.0 - alpha,
            ),
            _ => ([alpha, 0.0, -alpha], 1.0 + alpha, -2.0 * cw, 1.0 - alpha),
        };
        Self {
            b: [b[0] / a0, b[1] / a0, b[2] / a0],
            a: [a1 / a0, a2 / a0],
            z: [0.0; 2],
        }
    }
    fn run(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// Synth drum kit keyed like General MIDI (36 kick, 38 snare, 42 closed hat), plus
/// optional background noise (dB below full scale).
fn render_drums(sr: u32, len: f64, hits: &[Ref], noise_db: Option<f64>) -> Arc<DecodedAudio> {
    let srf = sr as f64;
    let n = (len * srf) as usize;
    let mut x = vec![0.0f32; n];
    let mut rng = Noise(7);
    for g in hits {
        let a = (g.start * srf) as usize;
        let amp = 0.5 * g.velocity as f64;
        match g.pitch {
            36 => {
                let mut phase = 0.0;
                for (i, out) in x.iter_mut().enumerate().skip(a).take((0.5 * srf) as usize) {
                    let t = (i - a) as f64 / srf;
                    phase += (45.0 + 80.0 * (-t / 0.03).exp()) / srf;
                    let click = if t < 0.002 {
                        0.1 * rng.next() as f64
                    } else {
                        0.0
                    };
                    let v = (2.0 * PI * phase).sin() * (-t / 0.22).exp() * (t / 0.001).min(1.0);
                    *out += (amp * (v + click)) as f32;
                }
            }
            38 => {
                // Noise band-limited to ≈1–6 kHz (the wires), over a 185 Hz body.
                let mut bp = Biquad::new("bp", 2500.0, 0.6, srf);
                let mut lp = Biquad::new("lp", 6000.0, 0.7, srf);
                for (i, out) in x.iter_mut().enumerate().skip(a).take((0.4 * srf) as usize) {
                    let t = (i - a) as f64 / srf;
                    let tone = 0.5 * (2.0 * PI * 185.0 * t).sin() * (-t / 0.07).exp();
                    let noise = 1.4 * lp.run(bp.run(rng.next() as f64)) * (-t / 0.11).exp();
                    *out += (amp * (tone + noise) * (t / 0.001).min(1.0)) as f32;
                }
            }
            _ => {
                let mut hp1 = Biquad::new("hp", 7000.0, 0.7, srf);
                let mut hp2 = Biquad::new("hp", 7000.0, 0.7, srf);
                for (i, out) in x.iter_mut().enumerate().skip(a).take((0.25 * srf) as usize) {
                    let t = (i - a) as f64 / srf;
                    let v = hp2.run(hp1.run(rng.next() as f64)) * (-t / 0.035).exp();
                    *out += (amp * 0.8 * v) as f32;
                }
            }
        }
    }
    if let Some(db) = noise_db {
        let g = 10f64.powf(db / 20.0) as f32;
        for v in &mut x {
            *v += g * rng.next();
        }
    }
    Arc::new(DecodedAudio {
        sample_rate: sr,
        channels: vec![x],
    })
}

// ─── fixtures ───────────────────────────────────────────────────────────────────────────

fn sine_melody() -> Vec<Ref> {
    // A tune with steps, leaps, a repeated note, legato and detached notes, 120 BPM.
    grid(
        120.0,
        &[
            (0.0, 0.5, 60),
            (0.5, 0.5, 62),
            (1.0, 0.5, 64),
            (1.5, 0.5, 65),
            (2.0, 0.9, 67),
            (3.0, 0.9, 67),
            (4.0, 0.25, 72),
            (4.25, 0.25, 71),
            (4.5, 0.25, 69),
            (4.75, 0.25, 67),
            (5.0, 1.0, 64),
            (6.0, 0.4, 76),
            (6.5, 0.4, 55),
            (7.0, 0.75, 84),
            (8.0, 1.5, 60),
        ],
        0.8,
    )
}

fn saw_lead() -> Vec<Ref> {
    let mut v = grid(
        100.0,
        &[
            (0.0, 0.45, 69),
            (0.5, 0.45, 72),
            (1.0, 0.95, 76),
            (2.0, 0.2, 74),
            (2.25, 0.2, 72),
            (2.5, 0.45, 71),
            (3.0, 0.45, 69),
            (3.5, 0.45, 69),
            (4.0, 1.9, 64),
            (6.0, 0.45, 57),
            (6.5, 0.45, 60),
            (7.0, 0.95, 62),
        ],
        0.9,
    );
    // Dynamics.
    for (i, n) in v.iter_mut().enumerate() {
        n.velocity = [0.9, 0.6, 1.0, 0.5][i % 4];
    }
    v
}

fn bass_line() -> Vec<Ref> {
    grid(
        110.0,
        &[
            (0.0, 0.9, 28),
            (1.0, 0.4, 28),
            (1.5, 0.4, 31),
            (2.0, 0.9, 33),
            (3.0, 0.9, 35),
            (4.0, 0.9, 36),
            (5.0, 0.4, 40),
            (5.5, 0.4, 38),
            (6.0, 1.9, 33),
        ],
        0.9,
    )
}

/// `(beat, beats, keys)` chords.
fn chords(bpm: f64, chords: &[(f64, f64, &[u8])], velocity: f32) -> Vec<Ref> {
    let mut v = Vec::new();
    for &(s, d, keys) in chords {
        for &k in keys {
            v.extend(grid(bpm, &[(s, d, k)], velocity));
        }
    }
    v
}

fn sine_chords() -> Vec<Ref> {
    chords(
        90.0,
        &[
            (0.0, 1.8, &[60, 64, 67]),
            (2.0, 1.8, &[57, 60, 64]),
            (4.0, 1.8, &[53, 57, 60, 65]),
            (6.0, 1.8, &[55, 59, 62, 67]),
            (8.0, 0.9, &[48, 55, 64]),
            (9.0, 0.9, &[50, 57, 65]),
        ],
        0.6,
    )
}

fn piano_chords() -> Vec<Ref> {
    chords(
        96.0,
        &[
            (0.0, 1.9, &[48, 60, 64, 67]),
            (2.0, 1.9, &[45, 60, 64, 69]),
            (4.0, 0.9, &[41, 60, 65, 69]),
            (5.0, 0.9, &[43, 59, 62, 67]),
            (6.0, 1.9, &[48, 55, 64, 72]),
            (8.0, 0.45, &[62, 65, 69]),
            (8.5, 0.45, &[64, 67, 71]),
            (9.0, 1.4, &[65, 69, 72]),
        ],
        0.7,
    )
}

/// `steps` of 16ths per bar: `(step, key, velocity)`, `bars` times.
fn drum_pattern(bpm: f64, bars: usize, steps: &[(usize, u8, f32)]) -> Vec<Ref> {
    let step = 60.0 / bpm / 4.0;
    let mut v = Vec::new();
    for bar in 0..bars {
        for &(s, k, vel) in steps {
            v.push(r(0.1 + (bar * 16 + s) as f64 * step, step, k, vel));
        }
    }
    v
}

fn basic_beat() -> Vec<Ref> {
    let mut steps = vec![
        (0, 36, 1.0),
        (8, 36, 1.0),
        (10, 36, 0.8),
        (4, 38, 1.0),
        (12, 38, 1.0),
    ];
    for s in (0..16).step_by(2) {
        steps.push((s, 42, if s % 4 == 0 { 0.8 } else { 0.55 }));
    }
    drum_pattern(110.0, 2, &steps)
}

fn busy_beat() -> Vec<Ref> {
    let mut steps = vec![
        (0, 36, 1.0),
        (3, 36, 0.7),
        (7, 36, 0.8),
        (10, 36, 0.9),
        (4, 38, 1.0),
        (12, 38, 1.0),
        (15, 38, 0.5),
    ];
    // 16th hats, quiet off-beats, except under the lone kicks (3, 7) and snare (15).
    for s in (0..16).filter(|s| ![3, 7, 15].contains(s)) {
        steps.push((s, 42, if s % 2 == 0 { 0.7 } else { 0.4 }));
    }
    drum_pattern(124.0, 2, &steps)
}

// ─── runner ─────────────────────────────────────────────────────────────────────────────

struct Case {
    name: &'static str,
    mode: Mode,
    audio: Arc<DecodedAudio>,
    reference: Vec<Ref>,
    /// Required (precision, recall) at 20 ms and at 50 ms.
    floor20: (f64, f64),
    floor50: (f64, f64),
}

fn run(case: &Case) -> (Score, Score) {
    let secs = case.audio.frames() as f64 / case.audio.sample_rate as f64;
    let t = Instant::now();
    let notes = Detector::run(case.audio.clone(), case.mode, Options::default());
    let took = t.elapsed().as_secs_f64();
    let s20 = score(&case.reference, &notes, 0.020);
    let s50 = score(&case.reference, &notes, 0.050);
    eprintln!(
        "| {:<14} | {:>3} | {:>3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:>4.1} / {:>4.1} | {:>5.0}x |",
        case.name,
        s50.reference,
        s50.detected,
        s20.precision(),
        s20.recall(),
        s50.precision(),
        s50.recall(),
        s50.f1(),
        s50.mean_err_ms,
        s50.max_err_ms,
        secs / took.max(1e-9),
    );
    if s50.matched < s50.reference || s50.matched < s50.detected {
        let mut missed: Vec<String> = Vec::new();
        for g in &case.reference {
            if !notes
                .iter()
                .any(|d| d.pitch == g.pitch && (d.start - g.start).abs() <= 0.05)
            {
                missed.push(format!("{}@{:.3}", g.pitch, g.start));
            }
        }
        let extra: Vec<String> = notes
            .iter()
            .filter(|d| {
                !case
                    .reference
                    .iter()
                    .any(|g| d.pitch == g.pitch && (d.start - g.start).abs() <= 0.05)
            })
            .map(|d| format!("{}@{:.3}", d.pitch, d.start))
            .collect();
        eprintln!("    missed: {missed:?}\n    extra:  {extra:?}");
    }
    (s20, s50)
}

fn check(cases: &[Case]) {
    eprintln!(
        "| fixture        | ref | det | P@20  | R@20  | P@50  | R@50  | F1@50 | onset err ms (mean / max) | speed |"
    );
    let mut failures = Vec::new();
    for c in cases {
        let (s20, s50) = run(c);
        if s20.precision() < c.floor20.0 || s20.recall() < c.floor20.1 {
            failures.push(format!(
                "{}: P/R@20 {:.3}/{:.3} below {:?}",
                c.name,
                s20.precision(),
                s20.recall(),
                c.floor20
            ));
        }
        if s50.precision() < c.floor50.0 || s50.recall() < c.floor50.1 {
            failures.push(format!(
                "{}: P/R@50 {:.3}/{:.3} below {:?}",
                c.name,
                s50.precision(),
                s50.recall(),
                c.floor50
            ));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn melody_fixtures() {
    let exact = (1.0, 1.0);
    let m = sine_melody();
    let lead = saw_lead();
    let bass = bass_line();
    check(&[
        Case {
            name: "sine melody",
            mode: Mode::Melody,
            audio: render(44_100, 9.0, &m, Voice::Sine),
            reference: m,
            floor20: exact,
            floor50: exact,
        },
        Case {
            name: "saw lead (vib)",
            mode: Mode::Melody,
            audio: render(48_000, 6.0, &lead, Voice::Saw { vibrato: 0.15 }),
            reference: lead,
            floor20: (0.9, 0.9),
            floor50: (0.9, 0.9),
        },
        Case {
            name: "bass line",
            mode: Mode::Melody,
            audio: render(44_100, 5.0, &bass, Voice::Saw { vibrato: 0.0 }),
            reference: bass,
            floor20: (0.85, 0.85),
            floor50: (0.9, 0.9),
        },
    ]);
}

#[test]
fn harmony_fixtures() {
    let exact = (1.0, 1.0);
    let sc = sine_chords();
    let pc = piano_chords();
    check(&[
        Case {
            name: "sine chords",
            mode: Mode::Harmony,
            audio: render(44_100, 8.0, &sc, Voice::Sine),
            reference: sc,
            floor20: exact,
            floor50: exact,
        },
        Case {
            name: "piano chords",
            mode: Mode::Harmony,
            audio: render(48_000, 7.5, &pc, Voice::Piano),
            reference: pc,
            floor20: (0.85, 0.85),
            floor50: (0.85, 0.85),
        },
    ]);
}

#[test]
fn drum_fixtures() {
    let exact = (1.0, 1.0);
    let basic = basic_beat();
    let busy = busy_beat();
    check(&[
        Case {
            name: "drum loop",
            mode: Mode::Drums,
            audio: render_drums(44_100, 4.6, &basic, None),
            reference: basic,
            floor20: exact,
            floor50: exact,
        },
        Case {
            name: "busy loop+noise",
            mode: Mode::Drums,
            audio: render_drums(48_000, 4.2, &busy, Some(-50.0)),
            reference: busy,
            floor20: (0.85, 0.85),
            floor50: (0.9, 0.9),
        },
    ]);
}

/// Harder material: a sawtooth pad (rich, overlapping partials), a melody under noise,
/// and the acceptance signals at other sample rates.
#[test]
fn harder_fixtures_and_rates() {
    let exact = (1.0, 1.0);
    let pad = chords(
        100.0,
        &[
            (0.0, 1.9, &[48, 55, 60, 64]),
            (2.0, 1.9, &[50, 57, 62, 65]),
            (4.0, 1.9, &[43, 55, 59, 62]),
            (6.0, 1.9, &[48, 52, 60, 67]),
        ],
        0.5,
    );
    let m = sine_melody();
    let mut noisy = render(44_100, 9.0, &m, Voice::Saw { vibrato: 0.1 });
    {
        let a = Arc::make_mut(&mut noisy);
        let mut rng = Noise(3);
        for ch in &mut a.channels {
            for v in ch.iter_mut() {
                *v += 0.01 * rng.next(); // ≈ -40 dBFS white noise
            }
        }
    }
    let beat = basic_beat();
    check(&[
        Case {
            name: "saw pad chords",
            mode: Mode::Harmony,
            audio: render(44_100, 5.5, &pad, Voice::Saw { vibrato: 0.0 }),
            reference: pad,
            floor20: (0.85, 0.85),
            floor50: (0.85, 0.85),
        },
        Case {
            name: "melody + noise",
            mode: Mode::Melody,
            audio: noisy,
            reference: m.clone(),
            floor20: (0.9, 0.9),
            floor50: (0.9, 0.9),
        },
        Case {
            name: "melody 22.05k",
            mode: Mode::Melody,
            audio: render(22_050, 9.0, &m, Voice::Sine),
            reference: m.clone(),
            floor20: exact,
            floor50: exact,
        },
        Case {
            name: "melody 96k",
            mode: Mode::Melody,
            audio: render(96_000, 9.0, &m, Voice::Sine),
            reference: m,
            floor20: exact,
            floor50: exact,
        },
        Case {
            name: "drums 96k",
            mode: Mode::Drums,
            audio: render_drums(96_000, 4.6, &beat, None),
            reference: beat,
            floor20: exact,
            floor50: exact,
        },
    ]);
}

/// The modes respect the pitch range and the drum keys from the options.
#[test]
fn options_are_honoured() {
    let m = sine_melody();
    let audio = render(44_100, 9.0, &m, Voice::Sine);
    let opts = Options {
        min_pitch: 62,
        max_pitch: 72,
        ..Options::default()
    };
    let notes = Detector::run(audio, Mode::Melody, opts);
    assert!(!notes.is_empty());
    assert!(
        notes.iter().all(|n| (62..=72).contains(&n.pitch)),
        "{notes:?}"
    );

    let beat = basic_beat();
    let audio = render_drums(44_100, 4.6, &beat, None);
    let opts = Options {
        kick_key: 35,
        snare_key: 40,
        hihat_key: 44,
        ..Options::default()
    };
    let notes = Detector::run(audio, Mode::Drums, opts);
    let mut keys: Vec<u8> = notes.iter().map(|n| n.pitch).collect();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys, [35, 40, 44]);
    // Notes are sorted by (start, pitch).
    assert!(
        notes
            .windows(2)
            .all(|w| (w[0].start, w[0].pitch) <= (w[1].start, w[1].pitch))
    );
}

/// Louder notes get higher velocities.
#[test]
fn velocity_follows_the_level() {
    let notes = vec![r(0.1, 0.4, 60, 1.0), r(0.6, 0.4, 62, 0.25)];
    let audio = render(44_100, 1.2, &notes, Voice::Sine);
    let d = Detector::run(audio, Mode::Melody, Options::default());
    assert_eq!(d.len(), 2, "{d:?}");
    assert!(d[0].velocity > d[1].velocity + 0.2, "{d:?}");
}

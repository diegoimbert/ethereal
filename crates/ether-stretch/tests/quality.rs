//! Objective evaluation of the warp-mode stretchers: Complex+ ([`PlusStretcher`]), Complex
//! (Signalsmith Stretch, the C++ the desktop uses) and Repitch (linear-interpolation
//! resampling, as the engine plays it).
//!
//! Every test signal is generated here (no recordings), with a known *ideal* stretched or
//! transposed version where one exists (stationary chords, sweeps, vowels from a source-filter
//! model, click trains), so quality is measured against the ideal instead of by ear:
//!
//! - **lsd**: log-spectral distance (dB) of STFT magnitudes vs the ideal (phase-blind).
//! - **pre-echo**: energy in the 40 ms before each attack over the attack's (dB).
//! - **attack**: peak level around each attack vs the source attack's (1 = intact).
//! - **am**: amplitude modulation of the partials of a stretched chord (phasiness / beating,
//!   % of the partial's level; 0 for the ideal).
//! - **ic**: inter-channel phase error of a stereo chord's partials (degrees; stereo image).
//! - **formant**: mean error (dB) of a transposed vowel's harmonics vs the source envelope.
//! - **cpu**: time per second of stereo audio.
//!
//! `cargo test --release -p ether-stretch --test quality -- --ignored --nocapture` prints the
//! tables recorded in `docs/WARP.md`. `regression` (not ignored) holds Complex+ to
//! thresholds on a subset.

use std::f32::consts::{PI, TAU};

use ether_stretch::{Formants, PlusStretcher, Stretcher};
use realfft::RealFftPlanner;

const SR: f32 = 48_000.0;

// ---------------------------------------------------------------------------------------
// Stretchers.

/// Linear-interpolation resampling (the engine's Repitch) behind the trait: no latency, the
/// time ratio changes the pitch.
#[derive(Default)]
struct Repitch {
    hist: Vec<f32>,
    pos: f64,
    channels: usize,
}

impl Stretcher for Repitch {
    fn configure(&mut self, channels: usize, _sample_rate: f32, _max_block: usize) {
        self.channels = channels;
        self.hist = vec![0.0; channels];
        self.pos = 0.0;
    }
    fn reset(&mut self) {
        self.hist.fill(0.0);
        self.pos = 0.0;
    }
    fn input_latency(&self) -> usize {
        0
    }
    fn output_latency(&self) -> usize {
        0
    }
    fn set_transpose_semitones(&mut self, _semitones: f32) {}
    fn seek(&mut self, input: &[&[f32]], _playback_rate: f64) {
        for (h, c) in self.hist.iter_mut().zip(input) {
            *h = c.last().copied().unwrap_or(0.0);
        }
        self.pos = 0.0;
    }
    #[allow(clippy::needless_range_loop)]
    fn process(&mut self, input: &[&[f32]], n_in: usize, output: &mut [&mut [f32]], n_out: usize) {
        let step = n_in as f64 / n_out.max(1) as f64;
        for ch in 0..self.channels {
            let x = |i: i64| -> f32 {
                if i < 0 {
                    self.hist[ch]
                } else {
                    input[ch].get(i as usize).copied().unwrap_or(0.0)
                }
            };
            for o in 0..n_out {
                // Output o plays input position `pos + (o + 1)·step - 1` (the last sample of
                // the previous call is index -1).
                let t = self.pos + (o + 1) as f64 * step - 1.0;
                let i = t.floor();
                let f = (t - i) as f32;
                output[ch][o] = x(i as i64) * (1.0 - f) + x(i as i64 + 1) * f;
            }
            if n_in > 0 {
                self.hist[ch] = input[ch][n_in - 1];
            }
        }
        self.pos = self.pos + n_out as f64 * step - n_in as f64;
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Algo {
    Plus,
    Complex,
    Repitch,
}

impl Algo {
    fn name(self) -> &'static str {
        match self {
            Algo::Plus => "complex+",
            Algo::Complex => "complex",
            Algo::Repitch => "repitch",
        }
    }

    fn make(self, formants: Option<Formants>) -> Box<dyn Stretcher> {
        match self {
            Algo::Plus => {
                let mut s = PlusStretcher::new();
                s.set_formants(formants.unwrap_or(Formants {
                    amount: 0.0,
                    ..Formants::default()
                }));
                Box::new(s)
            }
            #[cfg(all(feature = "signalsmith", not(target_arch = "wasm32")))]
            Algo::Complex => Box::new(ether_stretch::SignalsmithStretcher::default()),
            #[cfg(not(all(feature = "signalsmith", not(target_arch = "wasm32"))))]
            Algo::Complex => Box::new(ether_stretch::RustStretcher::default()),
            Algo::Repitch => Box::new(Repitch::default()),
        }
    }
}

/// Plays `input` at time `ratio` (input/output) and `semitones` the way the engine does: a
/// seek with pre-roll, then blocks fed up to `src(o + out_lat) + in_lat`. Output `o` is
/// input `o·ratio`.
fn drive(
    st: &mut dyn Stretcher,
    input: &[Vec<f32>],
    ratio: f64,
    semitones: f32,
    block: usize,
) -> Vec<Vec<f32>> {
    let chans = input.len();
    st.configure(chans, SR, block);
    st.set_transpose_semitones(semitones);
    let (in_lat, out_lat) = (st.input_latency() as f64, st.output_latency() as f64);
    let len = input[0].len();
    let out_total = (len as f64 / ratio) as usize;
    let read = |from: i64, to: i64| -> Vec<Vec<f32>> {
        (0..chans)
            .map(|ch| {
                (from..to)
                    .map(|i| {
                        if i >= 0 && (i as usize) < len {
                            input[ch][i as usize]
                        } else {
                            0.0
                        }
                    })
                    .collect()
            })
            .collect()
    };
    let feed = |o: f64| ((o + out_lat) * ratio + in_lat).floor() as i64;
    let mut fed = feed(0.0);
    let pre = read(fed - SR as i64 / 2, fed);
    let refs: Vec<&[f32]> = pre.iter().map(|c| c.as_slice()).collect();
    st.seek(&refs, ratio);
    let mut out = vec![Vec::with_capacity(out_total); chans];
    let mut bufs = vec![vec![0.0f32; block]; chans];
    let mut o = 0;
    while o < out_total {
        let n = block.min(out_total - o);
        let target = feed((o + n) as f64);
        let src = read(fed, target);
        let refs: Vec<&[f32]> = src.iter().map(|c| c.as_slice()).collect();
        let mut outs: Vec<&mut [f32]> = bufs.iter_mut().map(|b| &mut b[..n]).collect();
        st.process(&refs, (target - fed) as usize, &mut outs, n);
        for (ch, b) in bufs.iter().enumerate() {
            out[ch].extend_from_slice(&b[..n]);
        }
        fed = target;
        o += n;
    }
    out
}

// ---------------------------------------------------------------------------------------
// Signals. `r` = time ratio (input/output) of the ideal, `p` = pitch ratio.

/// Deterministic pseudo-random in [0, 1).
fn rnd(seed: &mut u32) -> f32 {
    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    (*seed >> 8) as f32 / (1u32 << 24) as f32
}

struct Partial {
    freq: f32,
    amp: f32,
    phase: f32,
    /// Right channel's phase offset and gain.
    r_phase: f32,
    r_gain: f32,
}

/// A stationary "chord" of 40 partials from 110 Hz to ~8 kHz, spaced so that a 4096-point
/// analysis resolves every one (equal-tempered chords have near-coincident partials, which
/// beat and make phase-blind metrics depend on arbitrary phases). Each partial has its own
/// phase and stereo offset.
fn chord_partials() -> Vec<Partial> {
    let mut seed = 11;
    let mut v = Vec::new();
    let mut f = 110.0f32;
    while f < 8000.0 {
        v.push(Partial {
            freq: f,
            amp: 0.1 * (110.0 / f).sqrt(),
            phase: rnd(&mut seed) * TAU,
            r_phase: (rnd(&mut seed) - 0.5) * PI,
            r_gain: 0.6 + 0.4 * rnd(&mut seed),
        });
        f = f * 1.09 + 25.0;
    }
    v
}

fn chord(len: usize, p: f32) -> Vec<Vec<f32>> {
    let parts = chord_partials();
    let mut l = vec![0.0f32; len];
    let mut r = vec![0.0f32; len];
    for q in &parts {
        let w = TAU * q.freq * p / SR;
        for i in 0..len {
            let ph = w * i as f32 + q.phase;
            l[i] += q.amp * ph.sin();
            r[i] += q.amp * q.r_gain * (ph + q.r_phase).sin();
        }
    }
    vec![l, r]
}

/// Log sweep 60 Hz → 8 kHz over `dur` input seconds, played at time ratio `r`, pitch `p`.
fn sweep(dur: f32, r: f32, p: f32) -> Vec<Vec<f32>> {
    let len = (dur * SR / r) as usize;
    let k = (8000.0f32 / 60.0).ln();
    let mut ph = 0.0f32;
    let x: Vec<f32> = (0..len)
        .map(|i| {
            let t = i as f32 * r / SR;
            let f = 60.0 * (k * t / dur).exp() * p;
            ph = (ph + TAU * f / SR) % TAU;
            0.3 * ph.sin()
        })
        .collect();
    vec![x.clone(), x]
}

/// Vowel /a/ envelope (all-pole formant model, F1–F4, with a glottal tilt).
fn vowel_env(f: f32) -> f32 {
    let mut m = 1.0f32;
    for (fc, bw) in [
        (730.0f32, 90.0f32),
        (1090.0, 110.0),
        (2440.0, 170.0),
        (3400.0, 250.0),
    ] {
        let x = f / fc;
        m /= ((1.0 - x * x).powi(2) + (f * bw / (fc * fc)).powi(2)).sqrt();
    }
    m / (1.0 + (f / 150.0).powi(2)).sqrt()
}

/// A sung vowel (f0 140 Hz, `vibrato` depth), harmonics up to 10 kHz from [`vowel_env`].
/// `p`: pitch ratio; `keep`: formants stay (envelope at the new harmonic frequencies) or move
/// with the pitch. Played at time ratio `r` (vibrato rate follows).
#[allow(clippy::needless_range_loop)]
fn vowel(dur: f32, r: f32, p: f32, keep: bool, vibrato: f32) -> Vec<f32> {
    let len = (dur * SR / r) as usize;
    let f0 = 140.0f32;
    let hs = (10_000.0 / (f0 * p.max(1.0))) as usize;
    let norm = 0.25
        / (1..=hs)
            .map(|h| vowel_env(h as f32 * f0))
            .fold(0.0f32, f32::max);
    let mut phases = vec![0.0f32; hs + 1];
    let mut seed = 5;
    for ph in phases.iter_mut() {
        *ph = rnd(&mut seed) * TAU;
    }
    (0..len)
        .map(|i| {
            let t = i as f32 * r / SR;
            let f = f0 * (1.0 + vibrato * (TAU * 5.5 * t).sin());
            let mut s = 0.0;
            for h in 1..=hs {
                let fh = h as f32 * f * p;
                if fh > 0.45 * SR {
                    break;
                }
                let a = if keep {
                    vowel_env(fh)
                } else {
                    vowel_env(h as f32 * f)
                };
                phases[h] = (phases[h] + TAU * fh / SR) % TAU;
                s += a * norm * phases[h].sin();
            }
            s
        })
        .collect()
}

/// Click times (input seconds) of [`clicks`].
fn click_times(dur: f32) -> Vec<f32> {
    (0..)
        .map(|k| 0.3 + k as f32 * 0.25)
        .take_while(|t| *t < dur - 0.2)
        .collect()
}

/// A click: 3 ms of decaying noise (the same for every click).
fn click_shape() -> Vec<f32> {
    let mut seed = 99;
    (0..144)
        .map(|i| (rnd(&mut seed) * 2.0 - 1.0) * 0.8 * (-(i as f32) / 30.0).exp())
        .collect()
}

/// Clicks over a quiet pad (so the stretcher isn't silent between them), played at time
/// ratio `r`; the ideal keeps each click's shape.
fn clicks(dur: f32, r: f32, with_pad: bool) -> Vec<Vec<f32>> {
    let len = (dur * SR / r) as usize;
    let mut x = vec![0.0f32; len];
    if with_pad {
        for (i, v) in x.iter_mut().enumerate() {
            let t = i as f32 / SR;
            *v = 0.02 * (TAU * 220.0 * t).sin() + 0.015 * (TAU * 330.0 * t).sin();
        }
    }
    let shape = click_shape();
    for t in click_times(dur) {
        let at = (t * SR / r) as usize;
        for (k, s) in shape.iter().enumerate() {
            if at + k < len {
                x[at + k] += s;
            }
        }
    }
    vec![x.clone(), x]
}

/// A drum loop (kick, snare, hats; 120 BPM 16ths) and its onsets (input samples).
fn drums(dur: f32) -> (Vec<Vec<f32>>, Vec<usize>) {
    let len = (dur * SR) as usize;
    let mut x = vec![0.0f32; len];
    let mut seed = 3;
    let mut onsets = Vec::new();
    let step = (SR * 0.125) as usize;
    let mut s = 0;
    while s * step + 9600 < len {
        let at = s * step;
        let pos = s % 16;
        let mut hit = false;
        if pos == 0 || pos == 6 || pos == 10 {
            // Kick: pitch drop 150 → 50 Hz plus a click.
            let mut ph = 0.0f32;
            for i in 0..9600 {
                let t = i as f32 / SR;
                let f = 50.0 + 100.0 * (-t * 30.0).exp();
                ph += TAU * f / SR;
                x[at + i] += 0.7 * ph.sin() * (-t * 8.0).exp();
                if i < 48 {
                    x[at + i] += 0.3 * (1.0 - i as f32 / 48.0);
                }
            }
            hit = true;
        }
        if pos == 4 || pos == 12 {
            for i in 0..7200 {
                let t = i as f32 / SR;
                x[at + i] += (0.4 * (rnd(&mut seed) * 2.0 - 1.0) + 0.3 * (TAU * 190.0 * t).sin())
                    * (-t * 25.0).exp();
            }
            hit = true;
        }
        if pos % 2 == 0 {
            // Hat: high-passed noise.
            let mut prev = 0.0;
            for i in 0..2400 {
                let t = i as f32 / SR;
                let n = rnd(&mut seed) * 2.0 - 1.0;
                x[at + i] += 0.25 * (n - prev) * (-t * 90.0).exp();
                prev = n;
            }
            hit = true;
        }
        if hit {
            onsets.push(at);
        }
        s += 1;
    }
    (vec![x.clone(), x], onsets)
}

// ---------------------------------------------------------------------------------------
// Metrics.

fn stft_mag(x: &[f32], n: usize, hop: usize) -> Vec<Vec<f32>> {
    let fft = RealFftPlanner::<f32>::new().plan_fft_forward(n);
    let win: Vec<f32> = (0..n)
        .map(|i| 0.5 - 0.5 * (TAU * i as f32 / n as f32).cos())
        .collect();
    let mut buf = vec![0.0f32; n];
    let mut spec = fft.make_output_vec();
    let mut frames = Vec::new();
    let mut s = 0;
    while s + n <= x.len() {
        for i in 0..n {
            buf[i] = x[s + i] * win[i];
        }
        fft.process(&mut buf, &mut spec).unwrap();
        frames.push(spec.iter().map(|c| c.norm()).collect());
        s += hop;
    }
    frames
}

/// Log-spectral distance (dB) of `y` vs `ideal`, frames in the middle (edges skipped), bins
/// within 60 dB of the ideal frame's peak (or the output's: spurious energy counts).
fn lsd(y: &[f32], ideal: &[f32]) -> f32 {
    lsd_n(y, ideal, 2048)
}

fn lsd_n(y: &[f32], ideal: &[f32], n: usize) -> f32 {
    let hop = n / 4;
    let len = y.len().min(ideal.len());
    let a = stft_mag(&y[..len], n, hop);
    let b = stft_mag(&ideal[..len], n, hop);
    let global = b.iter().flatten().fold(0.0f32, |m, v| m.max(*v));
    let eps = global * 1e-4;
    let skip = (0.15 * SR) as usize / hop;
    let (mut sum, mut count) = (0.0f64, 0usize);
    for (fa, fb) in a
        .iter()
        .zip(&b)
        .skip(skip)
        .take(a.len().saturating_sub(2 * skip))
    {
        let peak = fb.iter().chain(fa.iter()).fold(0.0f32, |m, v| m.max(*v));
        let gate = peak * 1e-3;
        let (mut s, mut c) = (0.0f64, 0usize);
        for (va, vb) in fa.iter().zip(fb) {
            if va.max(*vb) < gate {
                continue;
            }
            let d = 20.0 * ((va + eps) / (vb + eps)).log10();
            s += (d * d) as f64;
            c += 1;
        }
        if c > 0 {
            sum += (s / c as f64).sqrt();
            count += 1;
        }
    }
    (sum / count.max(1) as f64) as f32
}

fn energy(x: &[f32], a: i64, b: i64) -> f64 {
    (a.max(0)..b.min(x.len() as i64))
        .map(|i| (x[i as usize] as f64).powi(2))
        .sum()
}

fn peak(x: &[f32], a: i64, b: i64) -> f32 {
    (a.max(0)..b.min(x.len() as i64))
        .map(|i| x[i as usize].abs())
        .fold(0.0, f32::max)
}

/// (pre-echo dB, attack level) averaged over attacks at output samples `at`, the source
/// attack peaking at `src_peak`.
fn transients(y: &[f32], at: &[i64], src_peak: f32) -> (f32, f32) {
    let ms = |m: f32| (m * SR / 1000.0) as i64;
    let (mut pre, mut att) = (0.0f64, 0.0f64);
    for &t in at {
        let e_pre = energy(y, t - ms(40.0), t - ms(3.0));
        let e_post = energy(y, t - ms(1.0), t + ms(15.0));
        pre += 10.0 * ((e_pre + 1e-12) / (e_post + 1e-12)).log10();
        att += (peak(y, t - ms(3.0), t + ms(6.0)) / src_peak) as f64;
    }
    let n = at.len().max(1) as f64;
    ((pre / n) as f32, (att / n) as f32)
}

/// Complex amplitude of `freq` in `x[a..a+n]` (Hann-weighted DFT, normalised to the sine
/// amplitude).
fn tone(x: &[f32], a: usize, n: usize, freq: f32) -> (f32, f32) {
    let w = TAU * freq / SR;
    let (mut re, mut im, mut ws) = (0.0f64, 0.0f64, 0.0f64);
    for i in 0..n {
        let win = 0.5 - 0.5 * (TAU * i as f32 / n as f32).cos();
        let v = x[a + i] * win;
        let ph = w * (a + i) as f32;
        re += (v * ph.cos()) as f64;
        im -= (v * ph.sin()) as f64;
        ws += win as f64;
    }
    let s = 2.0 / ws;
    ((re * s) as f32, (im * s) as f32)
}

/// Median amplitude modulation (% of the mean) of the chord's partials over the output's
/// middle, and the mean inter-channel phase error (degrees, amplitude-weighted).
fn chord_coherence(y: &[Vec<f32>], p: f32) -> (f32, f32) {
    let parts = chord_partials();
    let n = 8192;
    let hop = 2048;
    let len = y[0].len();
    let mut am = Vec::new();
    let (mut ic, mut icw) = (0.0f64, 0.0f64);
    for q in &parts {
        let f = q.freq * p;
        if f > 15_000.0 {
            continue;
        }
        let mut amps = Vec::new();
        let mut s = len / 5;
        while s + n < len * 4 / 5 {
            let (lr, li) = tone(&y[0], s, n, f);
            let (rr, ri) = tone(&y[1], s, n, f);
            amps.push((lr * lr + li * li).sqrt());
            // Inter-channel phase: arg(L · conj(R)) vs the source's -r_phase.
            let d = (li * rr - lr * ri).atan2(lr * rr + li * ri);
            let want = -q.r_phase;
            let mut e = (d - want).abs() % TAU;
            if e > PI {
                e = TAU - e;
            }
            ic += (e * q.amp) as f64;
            icw += q.amp as f64;
            s += hop;
        }
        let mean = amps.iter().sum::<f32>() / amps.len() as f32;
        let var = amps.iter().map(|a| (a - mean).powi(2)).sum::<f32>() / amps.len() as f32;
        am.push(var.sqrt() / mean.max(1e-9) * 100.0);
    }
    am.sort_by(f32::total_cmp);
    (am[am.len() / 2], (ic / icw.max(1e-9)) as f32 * 180.0 / PI)
}

/// Mean |dB| error of the audible harmonics (200 Hz – 5 kHz, within 40 dB of the strongest)
/// of a transposed constant-pitch vowel vs the source envelope (formants kept), after
/// removing the overall level offset (a median, robust to a few outliers).
fn formant_error(y: &[f32], p: f32) -> f32 {
    let f0 = 140.0 * p;
    let n = 16_384;
    let a = y.len() / 2 - n / 2;
    let mut d = Vec::new();
    let top = (1..)
        .map(|h| h as f32 * f0)
        .take_while(|f| *f <= 5000.0)
        .map(|f| 20.0 * vowel_env(f).log10())
        .fold(f32::MIN, f32::max);
    let mut h = 1;
    loop {
        let f = h as f32 * f0;
        if f > 5000.0 {
            break;
        }
        let want = 20.0 * vowel_env(f).log10();
        if f >= 200.0 && want > top - 40.0 {
            let (re, im) = tone(y, a, n, f);
            let got = 20.0 * (re * re + im * im).sqrt().max(1e-9).log10();
            d.push(got - want);
        }
        h += 1;
    }
    let mut sorted = d.clone();
    sorted.sort_by(f32::total_cmp);
    let off = sorted[sorted.len() / 2];
    d.iter().map(|v| (v - off).abs()).sum::<f32>() / d.len() as f32
}

fn mono(x: &[Vec<f32>]) -> Vec<f32> {
    x[0].iter().zip(&x[1]).map(|(a, b)| 0.5 * (a + b)).collect()
}

// ---------------------------------------------------------------------------------------
// Measurements.

const DUR: f32 = 4.0;

fn stretch_lsd(algo: Algo, sig: &str, t: f32) -> f32 {
    let r = 1.0 / t;
    match sig {
        "chord" => {
            let x = chord((DUR * SR) as usize, 1.0);
            let y = drive(algo.make(None).as_mut(), &x, r as f64, 0.0, 256);
            let ideal = chord(y[0].len(), 1.0);
            0.5 * (lsd_n(&y[0], &ideal[0], 4096) + lsd_n(&y[1], &ideal[1], 4096))
        }
        "sweep" => {
            let x = sweep(DUR, 1.0, 1.0);
            let y = drive(algo.make(None).as_mut(), &x, r as f64, 0.0, 256);
            let ideal = sweep(DUR, r, 1.0);
            lsd(&y[0], &ideal[0])
        }
        "vowel" => {
            let x = vowel(DUR, 1.0, 1.0, true, 0.012);
            let y = drive(
                algo.make(None).as_mut(),
                &[x.clone(), x],
                r as f64,
                0.0,
                256,
            );
            let ideal = vowel(DUR, r, 1.0, true, 0.012);
            lsd(&y[0], &ideal)
        }
        "clicks" => {
            let x = clicks(DUR, 1.0, true);
            let y = drive(algo.make(None).as_mut(), &x, r as f64, 0.0, 256);
            let ideal = clicks(DUR, r, true);
            lsd(&y[0], &ideal[0])
        }
        _ => unreachable!(),
    }
}

fn click_metrics(algo: Algo, t: f32) -> (f32, f32) {
    let r = 1.0 / t;
    let x = clicks(DUR, 1.0, false);
    let y = drive(algo.make(None).as_mut(), &x, r as f64, 0.0, 256);
    let at: Vec<i64> = click_times(DUR)
        .iter()
        .map(|s| (s * SR / r) as i64)
        .collect();
    let src_peak = click_shape().iter().fold(0.0f32, |m, v| m.max(v.abs()));
    transients(&mono(&y), &at, src_peak)
}

/// Attack level of the drum hits (the drums' own tails fill the pre-echo window, and a
/// stretcher rightly lengthens them, so pre-echo is measured on clicks).
fn drum_attack(algo: Algo, t: f32) -> f32 {
    let r = 1.0 / t;
    let (x, onsets) = drums(DUR);
    let y = drive(algo.make(None).as_mut(), &x, r as f64, 0.0, 256);
    let ms = |m: f32| (m * SR / 1000.0) as i64;
    let src_peak = onsets
        .iter()
        .map(|&o| peak(&x[0], o as i64 - ms(3.0), o as i64 + ms(6.0)))
        .sum::<f32>()
        / onsets.len() as f32;
    let at: Vec<i64> = onsets.iter().map(|&o| (o as f32 / r) as i64).collect();
    transients(&mono(&y), &at, src_peak).1
}

fn transpose_lsd(algo: Algo, st: f32) -> f32 {
    let p = (st / 12.0).exp2();
    let x = chord((DUR * SR) as usize, 1.0);
    let y = drive(algo.make(None).as_mut(), &x, 1.0, st, 256);
    let ideal = chord(y[0].len(), p);
    0.5 * (lsd_n(&y[0], &ideal[0], 4096) + lsd_n(&y[1], &ideal[1], 4096))
}

fn formant_metric(algo: Algo, st: f32) -> f32 {
    let p = (st / 12.0).exp2();
    let x = vowel(DUR, 1.0, 1.0, true, 0.0);
    let f = Formants::default();
    let y = drive(algo.make(Some(f)).as_mut(), &[x.clone(), x], 1.0, st, 256);
    formant_error(&y[0], p)
}

fn unity_lsd(algo: Algo, sig: &str) -> f32 {
    let x = match sig {
        "chord" => chord((DUR * SR) as usize, 1.0),
        "drums" => drums(DUR).0,
        "vowel" => {
            let v = vowel(DUR, 1.0, 1.0, true, 0.012);
            vec![v.clone(), v]
        }
        _ => unreachable!(),
    };
    let y = drive(algo.make(None).as_mut(), &x, 1.0, 0.0, 256);
    lsd(&y[0], &x[0])
}

/// (ms per second of stereo audio, worst block ms) at time ratio 0.8 (×1.25) and `st`
/// semitones (formants preserved), 128-frame blocks, on a drum loop over a chord.
fn cpu(algo: Algo, st_: f32) -> (f64, f64) {
    // Best of 5 (the machine may be busy).
    (0..5)
        .map(|_| cpu_once(algo, st_))
        .fold((f64::MAX, f64::MAX), |a, b| (a.0.min(b.0), a.1.min(b.1)))
}

fn cpu_once(algo: Algo, st_: f32) -> (f64, f64) {
    let secs = 20.0;
    let (d, _) = drums(secs);
    let c = chord((secs * SR) as usize, 1.0);
    let x: Vec<Vec<f32>> = (0..2)
        .map(|ch| d[ch].iter().zip(&c[ch]).map(|(a, b)| a + b).collect())
        .collect();
    let mut st = algo.make(Some(Formants::default()));
    st.configure(2, SR, 128);
    st.set_transpose_semitones(st_);
    let ratio = 0.8;
    let mut l = vec![0.0f32; 128];
    let mut r = vec![0.0f32; 128];
    let (mut pos, mut acc) = (0usize, 0.0f64);
    let mut worst = 0.0f64;
    let start = std::time::Instant::now();
    let mut o = 0usize;
    while pos + 256 < x[0].len() {
        let target = (((o + 128) as f64) * ratio) as usize;
        let n_in = target - (o as f64 * ratio) as usize;
        let t = std::time::Instant::now();
        st.process(
            &[&x[0][pos..pos + n_in], &x[1][pos..pos + n_in]],
            n_in,
            &mut [&mut l, &mut r],
            128,
        );
        worst = worst.max(t.elapsed().as_secs_f64());
        acc += l[0] as f64;
        pos += n_in;
        o += 128;
    }
    let elapsed = start.elapsed().as_secs_f64();
    std::hint::black_box(acc);
    let audio = o as f64 / SR as f64;
    (elapsed / audio * 1000.0, worst * 1000.0)
}

const ALGOS: [Algo; 3] = [Algo::Plus, Algo::Complex, Algo::Repitch];

fn row(label: &str, f: impl Fn(Algo) -> Option<String>) {
    let cells: Vec<String> = ALGOS
        .iter()
        .map(|a| f(*a).unwrap_or_else(|| "—".into()))
        .collect();
    println!("| {label} | {} |", cells.join(" | "));
}

fn header(title: &str, metric: &str) {
    println!("\n**{title}**\n");
    println!("| {metric} | {} |", ALGOS.map(|a| a.name()).join(" | "));
    println!("|---|{}", "---:|".repeat(ALGOS.len()));
}

#[test]
#[ignore = "evaluation tables (run with --release --ignored --nocapture)"]
fn quality_tables() {
    let times = [0.5f32, 0.8, 1.25, 2.0, 4.0];
    header(
        "Stretch: log-spectral distance to the ideal (dB, lower is better)",
        "signal × length",
    );
    for sig in ["chord", "vowel", "sweep", "clicks"] {
        for t in times {
            row(&format!("{sig} ×{t}"), |a| {
                Some(format!("{:.2}", stretch_lsd(a, sig, t)))
            });
        }
    }
    header(
        "Transients: click train (pre-echo dB / attack level)",
        "length",
    );
    for t in [1.25f32, 2.0, 4.0] {
        row(&format!("clicks ×{t}"), |a| {
            let (pre, att) = click_metrics(a, t);
            Some(format!("{pre:.1} / {att:.2}"))
        });
    }
    header("Transients: drum loop, attack level (1 = intact)", "length");
    for t in [0.8f32, 1.25, 2.0, 4.0] {
        row(&format!("drums ×{t}"), |a| {
            Some(format!("{:.2}", drum_attack(a, t)))
        });
    }
    header(
        "Phasiness: stereo chord (partial AM % / inter-channel phase error °)",
        "length",
    );
    for t in [1.25f32, 2.0, 4.0] {
        row(&format!("chord ×{t}"), |a| {
            let x = chord((DUR * SR) as usize, 1.0);
            let y = drive(a.make(None).as_mut(), &x, 1.0 / t as f64, 0.0, 256);
            let (am, ic) = chord_coherence(&y, if a == Algo::Repitch { 1.0 / t } else { 1.0 });
            Some(format!("{am:.1} / {ic:.1}"))
        });
    }
    header(
        "Transposition: chord, log-spectral distance to the ideal (dB)",
        "semitones",
    );
    for st in [-24.0f32, -12.0, -7.0, 7.0, 12.0, 24.0] {
        row(&format!("{st:+}"), |a| {
            (a != Algo::Repitch).then(|| format!("{:.2}", transpose_lsd(a, st)))
        });
    }
    header(
        "Formants: transposed vowel, audible harmonics' level error vs the source envelope (dB)",
        "semitones",
    );
    for st in [-12.0f32, -7.0, -3.0, 3.0, 7.0, 12.0] {
        row(&format!("{st:+}"), |a| {
            (a != Algo::Repitch).then(|| format!("{:.2}", formant_metric(a, st)))
        });
    }
    header(
        "Ratio 1, no transposition: log-spectral distance to the input (dB)",
        "signal",
    );
    for sig in ["chord", "vowel", "drums"] {
        row(sig, |a| Some(format!("{:.2}", unity_lsd(a, sig))));
    }
    header(
        "CPU: one stereo voice, 128-frame blocks (ms per second of audio / worst block ms)",
        "case",
    );
    row("×1.25", |a| {
        let (ms, worst) = cpu(a, 0.0);
        Some(format!("{ms:.1} / {worst:.3}"))
    });
    row("×1.25, +3 st (formants kept)", |a| {
        (a != Algo::Repitch).then(|| {
            let (ms, worst) = cpu(a, 3.0);
            format!("{ms:.1} / {worst:.3}")
        })
    });
}

/// Complex+ held to its measured quality (with margin), so regressions show.
#[test]
fn regression() {
    let lsd = |sig, t| stretch_lsd(Algo::Plus, sig, t);
    for (sig, t, max) in [
        ("chord", 2.0, 4.0),
        ("vowel", 4.0, 2.5),
        ("clicks", 2.0, 4.5),
    ] {
        let v = lsd(sig, t);
        assert!(v < max, "{sig} ×{t}: lsd {v} dB");
    }
    let (pre, att) = click_metrics(Algo::Plus, 2.0);
    assert!(
        pre < -30.0 && att > 0.9,
        "clicks ×2: pre-echo {pre} dB, attack {att}"
    );
    let att = drum_attack(Algo::Plus, 4.0);
    assert!(att > 0.85, "drums ×4: attack {att}");
    let f = formant_metric(Algo::Plus, 7.0);
    assert!(f < 3.0, "vowel +7: formant error {f} dB");
    let tr = transpose_lsd(Algo::Plus, -12.0);
    assert!(tr < 6.0, "chord -12 st: lsd {tr} dB");
    let u = unity_lsd(Algo::Plus, "drums");
    assert!(u < 0.1, "drums at ratio 1: lsd {u} dB");
}

//! Drums: onsets per band, classified into kick / snare / hi-hat.
//!
//! 1. Novelty per band, 3 ms hop: spectral flux (≈10 ms Hann) of the mid (1–5 kHz) and
//!    high (> 6 kHz) bands, and the level rise of a 120 Hz low-passed signal's peak
//!    envelope (a 10 ms spectrum can't follow a 50 Hz kick). Onsets are picked per band (so
//!    a quiet hat next to a loud kick is not masked), merged within 35 ms and sharpened on
//!    the envelope of the signal's first difference (attacks are bright).
//! 2. Each onset compares a ≈40 ms spectrum after it with one before it: the energy *gained*
//!    per band (kick 30–120 Hz, snare 1–5 kHz, hat 8–16 kHz), normalised by that band's
//!    typical gain over the file, says which drums struck (several can, e.g. kick + hat).
//!    A snare's own top end is not a hat: over a snare, the hat band must gain at least
//!    half as much (relative to its typical gain) as the snare band.

use super::fft::{Fft, hann};
use super::onset::pick_peaks;
use super::prep::Envelope;
use super::{DetectedNote, Options};

/// Novelty bands (Hz): mid, high.
const BANDS: [(f32, f32); 2] = [(1000.0, 5000.0), (6000.0, 16000.0)];
/// Low-band cutoff (Hz) and peak hold (s) of the kick novelty.
const LOW_CUTOFF: f64 = 120.0;
const LOW_HOLD: f64 = 0.02;
/// Classification bands (Hz): kick, snare, hat.
const CLASS_BANDS: [(f32, f32); 3] = [(30.0, 120.0), (1000.0, 5000.0), (8000.0, 16000.0)];
/// Length of a drum note (seconds).
const NOTE_SECONDS: f64 = 0.1;

pub(crate) struct Drums {
    rate: f64,
    hop: usize,
    fft: Fft,
    win: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    /// Level (dB) per frame of each novelty band.
    band_db: [Vec<f32>; 2],
    /// Low-passed signal: two biquad states, the next sample to filter, the peak per hop.
    low: [Biquad; 2],
    low_pos: usize,
    low_peak: Vec<f32>,
    level: Vec<f32>,
    next: usize,
    total: usize,
    /// Classification stage: onset times, then the energy gained at each.
    times: Option<Vec<f64>>,
    gains: Vec<[f32; 3]>,
    class_fft: Fft,
    class_win: Vec<f32>,
    re2: Vec<f32>,
    im2: Vec<f32>,
}

impl Drums {
    pub(crate) fn new(rate: f64, len: usize) -> Self {
        let n = ((rate * 0.01) as usize).next_power_of_two().max(64);
        let hop = ((rate * 0.003).round() as usize).max(1);
        let long = ((rate * 0.04) as usize).next_power_of_two().max(64);
        Self {
            re2: vec![0.0; long],
            im2: vec![0.0; long],
            rate,
            hop,
            fft: Fft::new(n),
            win: hann(n),
            re: vec![0.0; n],
            im: vec![0.0; n],
            band_db: Default::default(),
            low: [
                Biquad::lowpass(LOW_CUTOFF, rate),
                Biquad::lowpass(LOW_CUTOFF, rate),
            ],
            low_pos: 0,
            low_peak: Vec::new(),
            level: Vec::new(),
            next: 0,
            total: len.div_ceil(hop) + 1,
            times: None,
            gains: Vec::new(),
            class_fft: Fft::new(long),
            class_win: hann(long),
        }
    }

    pub(crate) fn done(&self) -> bool {
        self.times
            .as_ref()
            .is_some_and(|t| self.gains.len() >= t.len())
    }

    pub(crate) fn progress(&self) -> f32 {
        let frames = self.next as f32 / self.total.max(1) as f32;
        let class = match &self.times {
            Some(t) if !t.is_empty() => self.gains.len() as f32 / t.len() as f32,
            Some(_) => 1.0,
            None => 0.0,
        };
        0.8 * frames + 0.2 * class
    }

    pub(crate) fn step(&mut self, x: &[f32], opts: &Options, budget: usize) -> usize {
        let mut used = 0;
        while self.next < self.total && used < budget {
            self.frame(x);
            self.next += 1;
            used += self.hop;
        }
        if self.next >= self.total && self.times.is_none() {
            self.times = Some(self.onsets(x, opts));
        }
        let Some(times) = self.times.take() else {
            return used;
        };
        let n = self.class_fft.len();
        let lead = (0.005 * self.rate) as isize;
        while self.gains.len() < times.len() && used < budget {
            let at = (times[self.gains.len()] * self.rate) as isize;
            let pre = self.band_energy(x, at - lead - n as isize);
            let post = self.band_energy(x, at - lead);
            self.gains
                .push([0, 1, 2].map(|b| (post[b] - pre[b]).max(0.0)));
            used += n;
        }
        self.times = Some(times);
        used
    }

    /// Energy per class band of the `n`-point window starting at sample `from`.
    fn band_energy(&mut self, x: &[f32], from: isize) -> [f32; 3] {
        let n = self.class_fft.len();
        for j in 0..n {
            let i = from + j as isize;
            self.re2[j] = if i < 0 || i as usize >= x.len() {
                0.0
            } else {
                x[i as usize] * self.class_win[j]
            };
            self.im2[j] = 0.0;
        }
        self.class_fft.forward(&mut self.re2, &mut self.im2);
        let mut out = [0.0f32; 3];
        for (b, band) in CLASS_BANDS.iter().enumerate() {
            out[b] = self
                .bins(n, *band)
                .map(|k| self.re2[k] * self.re2[k] + self.im2[k] * self.im2[k])
                .sum();
        }
        out
    }

    fn bins(&self, n: usize, (lo, hi): (f32, f32)) -> std::ops::Range<usize> {
        let hi = hi.min(0.45 * self.rate as f32);
        let b = |f: f32| ((f as f64 / self.rate) * n as f64).round() as usize;
        b(lo)..(b(hi) + 1).min(n / 2)
    }

    fn frame(&mut self, x: &[f32]) {
        let n = self.fft.len();
        let start = (self.next * self.hop) as isize - (n / 2) as isize;
        let mut e = 0.0f32;
        for j in 0..n {
            let i = start + j as isize;
            let v = if i < 0 || i as usize >= x.len() {
                0.0
            } else {
                x[i as usize]
            };
            e += v * v;
            self.re[j] = v * self.win[j];
            self.im[j] = 0.0;
        }
        self.fft.forward(&mut self.re, &mut self.im);
        let scale = 4.0 / n as f32;
        for (b, band) in BANDS.iter().enumerate() {
            let e: f32 = self
                .bins(n, *band)
                .map(|k| (self.re[k] * self.re[k] + self.im[k] * self.im[k]) * scale * scale)
                .sum();
            self.band_db[b].push(10.0 * (e + 1e-10).log10());
        }
        self.level.push(10.0 * (e / n as f32 + 1e-12).log10());
        // Low band: filter up to this frame's time, keep the hop's peak.
        let end = ((self.next * self.hop) + 1).min(x.len());
        let mut peak = 0.0f32;
        while self.low_pos < end {
            let [l0, l1] = &mut self.low;
            let v = l1.run(l0.run(x[self.low_pos]));
            peak = peak.max(v.abs());
            self.low_pos += 1;
        }
        self.low_peak.push(peak);
    }

    /// Kick novelty: the dB rise of the held low-band peak over 6 ms.
    fn low_novelty(&self, frame_sec: f64) -> Vec<f32> {
        let hold = ((LOW_HOLD / frame_sec).ceil() as usize).max(1);
        let held: Vec<f32> = (0..self.low_peak.len())
            .map(|i| {
                let m = self.low_peak[i.saturating_sub(hold - 1)..=i]
                    .iter()
                    .copied()
                    .fold(0.0f32, f32::max);
                20.0 * (m + 1e-5).log10()
            })
            .collect();
        (0..held.len())
            .map(|i| (held[i] - held[i.saturating_sub(2)]).max(0.0))
            .collect()
    }

    /// Onset times (seconds), merged across bands.
    fn onsets(&self, x: &[f32], opts: &Options) -> Vec<f64> {
        let sens = opts.sensitivity.clamp(0.0, 1.0);
        let frame_sec = self.hop as f64 / self.rate;
        let gmax = self.level.iter().copied().fold(-120.0, f32::max);
        let floor = gmax - (25.0 + 35.0 * sens);
        let half = ((0.015 / frame_sec) as usize).max(1);
        let gap = ((0.04 / frame_sec) as usize).max(1);
        let delta = 0.06 + 0.16 * (1.0 - sens);
        let level = &self.level;
        let mut frames: Vec<usize> = Vec::new();
        let low = self.low_novelty(frame_sec);
        let bands: Vec<Vec<f32>> = self.band_db.iter().map(|d| rise(d, 3)).collect();
        for f in std::iter::once(&low).chain(&bands) {
            let p = pick_peaks(f, half, delta, gap, |i| {
                let j = (i + 5).min(level.len() - 1);
                level[i].max(level[j]) >= floor
            });
            frames.extend(p);
        }
        frames.sort_unstable();
        let merge = ((0.035 / frame_sec) as usize).max(1);
        let mut merged: Vec<usize> = Vec::new();
        for i in frames {
            if merged.last().is_none_or(|&l| i - l > merge) {
                merged.push(i);
            }
        }
        let env = Envelope::emphasized(x, self.rate);
        let mut times: Vec<f64> = Vec::with_capacity(merged.len());
        for i in merged {
            let t = env.refine_onset(i as f64 * frame_sec, 0.02, 0.03);
            if times.last().is_none_or(|&l| t - l >= 0.035) {
                times.push(t);
            }
        }

        times
    }

    pub(crate) fn finish(&self, opts: &Options) -> Vec<DetectedNote> {
        let sens = opts.sensitivity.clamp(0.0, 1.0);
        let empty = Vec::new();
        let times = self.times.as_ref().unwrap_or(&empty);
        let gains = &self.gains;
        let reference: [f32; 3] = [0, 1, 2].map(|b| {
            let mut v: Vec<f32> = gains.iter().map(|g| g[b]).filter(|&g| g > 0.0).collect();
            v.sort_by(f32::total_cmp);
            v.get((v.len() as f32 * 0.9) as usize)
                .or(v.last())
                .copied()
                .unwrap_or(1.0)
                .max(1e-12)
        });
        let tau = 0.3 - 0.2 * sens;
        let norm: Vec<[f32; 3]> = gains
            .iter()
            .map(|g| [0, 1, 2].map(|b| g[b] / reference[b]))
            .collect();
        let keys = [opts.kick_key, opts.snare_key, opts.hihat_key];
        let mut notes = Vec::new();
        for ((&t, g), nb) in times.iter().zip(gains).zip(&norm) {
            // Each band is judged against its own typical gain (`nb`), and must also be a
            // real share of what this onset gained: a kind of drum absent from the file
            // would otherwise be "typical" at the level of the others' leakage.
            let total = (g[0] + g[1] + g[2]).max(1e-12);
            let top = (g[1] + g[2]).max(1e-12);
            let share = [g[0] / total, g[1] / top, g[2] / top];
            let mut hit = [
                nb[0] >= tau && share[0] >= 0.1,
                nb[1] >= tau && share[1] >= 0.15,
                // Hats are often far quieter than the backbeat: a lower bar.
                nb[2] >= 0.5 * tau && share[2] >= 0.15,
            ];
            // A snare's own top end is not a hat.
            if hit[1] && hit[2] {
                hit[2] = nb[2] >= 0.5 * nb[1];
            }
            if !hit.iter().any(|h| *h) {
                let b = (0..3)
                    .max_by(|&a, &b| share[a].total_cmp(&share[b]))
                    .unwrap();
                if nb[b] >= tau * 0.5 {
                    hit[b] = true;
                }
            }
            for b in 0..3 {
                if hit[b] {
                    notes.push(DetectedNote {
                        start: t,
                        duration: NOTE_SECONDS,
                        pitch: keys[b],
                        velocity: nb[b].sqrt().clamp(0.1, 1.0),
                    });
                }
            }
        }
        // A drum note ends before the next hit of the same key.
        notes.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.pitch.cmp(&b.pitch)));
        for i in 0..notes.len() {
            let next = notes[i + 1..]
                .iter()
                .find(|n| n.pitch == notes[i].pitch)
                .map(|n| n.start);
            if let Some(s) = next {
                notes[i].duration = notes[i].duration.min((s - notes[i].start) * 0.9);
            }
        }
        notes.retain(|n| n.duration > 0.0);
        notes
    }
}

/// How far (dB) each frame rises above the loudest of the `back` frames before it.
fn rise(db: &[f32], back: usize) -> Vec<f32> {
    (0..db.len())
        .map(|i| {
            let prev = db[i.saturating_sub(back)..i]
                .iter()
                .copied()
                .fold(f32::MIN, f32::max);
            if prev == f32::MIN {
                0.0
            } else {
                (db[i] - prev).max(0.0)
            }
        })
        .collect()
}

/// A 2nd-order Butterworth low-pass (RBJ biquad, transposed direct form II).
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    z: [f32; 2],
}

impl Biquad {
    fn lowpass(f: f64, rate: f64) -> Self {
        let w = 2.0 * std::f64::consts::PI * f.min(0.45 * rate) / rate;
        let (cw, sw) = (w.cos(), w.sin());
        let alpha = sw / (2.0 * std::f64::consts::FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;
        let b1 = (1.0 - cw) / a0;
        Self {
            b: [(b1 / 2.0) as f32, b1 as f32, (b1 / 2.0) as f32],
            a: [(-2.0 * cw / a0) as f32, ((1.0 - alpha) / a0) as f32],
            z: [0.0; 2],
        }
    }

    #[inline]
    fn run(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

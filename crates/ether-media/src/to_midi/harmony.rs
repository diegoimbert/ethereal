//! Harmony: simple polyphonic transcription.
//!
//! 1. Onsets: spectral flux of the log-compressed short-time spectrum (≈40 ms Hann, 4 ms
//!    hop), peak-picked, sharpened on the envelope.
//! 2. Between consecutive onsets, one high-resolution spectrum (Blackman, up to 350 ms,
//!    zero-padded): interpolated peaks, then iterative harmonic-sum selection: the
//!    candidate key whose harmonics collect the most energy (its fundamental must be
//!    present) is kept and its partials removed, until the best remaining candidate is
//!    much weaker than the first.
//! 3. A key present in consecutive segments continues its note unless its partials were
//!    re-attacked at the onset (energy before vs after, by Goertzel); notes end where the
//!    level decays 35 dB below the segment's attack, or at the next onset.

use std::collections::BTreeMap;

use super::fft::{Fft, blackman, hann};
use super::onset::{compress, pick_peaks};
use super::prep::Envelope;
use super::{DetectedNote, Options, hz_of};

/// Pitch tolerance (semitones) when matching a partial to a peak.
const TOL: f32 = 0.4;
/// Most harmonics summed per candidate.
const HARMONICS: usize = 10;
/// Most notes per segment.
const MAX_POLY: usize = 8;

pub(crate) struct Harmony {
    rate: f64,
    hop: usize,
    fft: Fft,
    win: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    prev: Vec<f32>,
    flux: Vec<f32>,
    level: Vec<f32>,
    next: usize,
    total: usize,
    /// Segment stage (after the frames).
    long: Fft,
    lre: Vec<f32>,
    lim: Vec<f32>,
    onsets: Option<Vec<f64>>,
    seg: usize,
    found: Vec<Vec<(u8, f32)>>,
}

#[derive(Clone, Copy)]
struct Peak {
    freq: f32,
    amp: f32,
}

impl Harmony {
    pub(crate) fn new(rate: f64, len: usize) -> Self {
        let n = ((rate * 0.04) as usize).next_power_of_two().max(64);
        let hop = ((rate * 0.004).round() as usize).max(1);
        let long = ((rate * 0.7) as usize).next_power_of_two().max(256);
        Self {
            rate,
            hop,
            fft: Fft::new(n),
            win: hann(n),
            re: vec![0.0; n],
            im: vec![0.0; n],
            prev: vec![0.0; n / 2],
            flux: Vec::new(),
            level: Vec::new(),
            next: 0,
            total: len.div_ceil(hop) + 1,
            long: Fft::new(long),
            lre: vec![0.0; long],
            lim: vec![0.0; long],
            onsets: None,
            seg: 0,
            found: Vec::new(),
        }
    }

    pub(crate) fn done(&self) -> bool {
        self.onsets.as_ref().is_some_and(|o| self.seg >= o.len())
    }

    pub(crate) fn progress(&self) -> f32 {
        let frames = self.next as f32 / self.total.max(1) as f32;
        let segs = match &self.onsets {
            Some(o) if !o.is_empty() => self.seg as f32 / o.len() as f32,
            Some(_) => 1.0,
            None => 0.0,
        };
        0.7 * frames + 0.3 * segs
    }

    pub(crate) fn step(&mut self, x: &[f32], opts: &Options, budget: usize) -> usize {
        let mut used = 0;
        while self.next < self.total && used < budget {
            self.frame(x);
            self.next += 1;
            used += self.hop;
        }
        if self.next >= self.total && self.onsets.is_none() {
            self.onsets = Some(self.pick_onsets(x, opts));
        }
        let Some(onsets) = self.onsets.take() else {
            return used;
        };
        let end = x.len() as f64 / self.rate;
        while self.seg < onsets.len() && used < budget {
            let t0 = onsets[self.seg];
            let t1 = onsets.get(self.seg + 1).copied().unwrap_or(end);
            let notes = self.segment(x, t0, t1, opts);
            self.found.push(notes);
            self.seg += 1;
            used += ((t1 - t0) * self.rate) as usize + 1;
        }
        self.onsets = Some(onsets);
        used
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
        let scale = 2.0 / (n as f32 * 0.5);
        let k0 = ((30.0 / self.rate) * n as f64) as usize;
        let mut flux = 0.0;
        for k in 0..n / 2 {
            let mag = (self.re[k] * self.re[k] + self.im[k] * self.im[k]).sqrt() * scale;
            let c = compress(mag);
            if k >= k0 {
                flux += (c - self.prev[k]).max(0.0);
            }
            self.prev[k] = c;
        }
        self.flux.push(flux);
        self.level.push(10.0 * (e / n as f32 + 1e-12).log10());
    }

    fn pick_onsets(&self, x: &[f32], opts: &Options) -> Vec<f64> {
        let sens = opts.sensitivity.clamp(0.0, 1.0);
        let gmax = self.level.iter().copied().fold(-120.0, f32::max);
        let floor = gmax - (20.0 + 40.0 * sens);
        let frame_sec = self.hop as f64 / self.rate;
        let half = ((0.016 / frame_sec) as usize).max(1);
        let gap = ((0.05 / frame_sec) as usize).max(1);
        let delta = 0.04 + 0.12 * (1.0 - sens);
        let level = &self.level;
        let picked = pick_peaks(&self.flux, half, delta, gap, |i| {
            // The level a little after the frame (the attack is ahead of the centre).
            let j = (i + 5).min(level.len() - 1);
            level[i].max(level[j]) >= floor
        });
        let env = self.envelope(x);
        let mut out: Vec<f64> = Vec::with_capacity(picked.len());
        for i in picked {
            let t = env.refine_onset(i as f64 * frame_sec, 0.03, 0.05);
            if out.last().is_none_or(|&l| t - l >= 0.03) {
                out.push(t);
            }
        }
        out
    }

    fn envelope(&self, x: &[f32]) -> Envelope {
        Envelope::new(x, self.rate)
    }

    /// Keys (and their fundamental's amplitude) sounding in `[t0, t1)`.
    fn segment(&mut self, x: &[f32], t0: f64, t1: f64, opts: &Options) -> Vec<(u8, f32)> {
        let n = self.long.len();
        let (mut a, mut b) = (t0 + 0.02, (t1 - 0.01).min(t0 + 0.37));
        if b - a < 0.04 {
            a = t0 + 0.005;
            b = t1;
        }
        if b - a < 0.03 {
            return Vec::new();
        }
        let ia = ((a * self.rate) as usize).min(x.len());
        let ib = ((b * self.rate) as usize).min(x.len()).min(ia + n);
        let len = ib - ia;
        if len < 32 {
            return Vec::new();
        }
        let win = blackman(len);
        let wsum: f32 = win.iter().sum();
        self.lre.fill(0.0);
        self.lim.fill(0.0);
        for j in 0..len {
            self.lre[j] = x[ia + j] * win[j];
        }
        self.long.forward(&mut self.lre, &mut self.lim);
        let scale = 2.0 / wsum;
        let mag: Vec<f32> = (0..n / 2)
            .map(|k| (self.lre[k] * self.lre[k] + self.lim[k] * self.lim[k]).sqrt() * scale)
            .collect();
        let peaks = spectral_peaks(&mag, self.rate as f32, n);
        select(&peaks, self.rate as f32, opts)
    }

    pub(crate) fn finish(&self, x: &[f32], opts: &Options) -> Vec<DetectedNote> {
        let Some(onsets) = &self.onsets else {
            return Vec::new();
        };
        let env = self.envelope(x);
        let end = x.len() as f64 / self.rate;
        let amax = self
            .found
            .iter()
            .flatten()
            .map(|&(_, a)| a)
            .fold(1e-9, f32::max);
        let mut notes: Vec<DetectedNote> = Vec::new();
        // Key → index of its sounding note.
        let mut active: BTreeMap<u8, usize> = BTreeMap::new();
        for (k, keys) in self.found.iter().enumerate() {
            let t0 = onsets[k];
            let t1 = onsets.get(k + 1).copied().unwrap_or(end);
            // Where the segment decays away.
            let pk = env.peak(t0, (t0 + 0.1).min(t1));
            let j0 = ((t0 + 0.05) / env.block_sec) as usize;
            let j1 = ((t1 / env.block_sec) as usize).min(env.db.len());
            let seg_end = (j0..j1)
                .find(|&j| env.db[j] < pk - 35.0)
                .map_or(t1, |j| (j as f64 * env.block_sec).min(t1));
            active.retain(|key, _| keys.iter().any(|(p, _)| p == key));
            for &(key, amp) in keys {
                if let Some(&i) = active.get(&key)
                    && !retriggered(x, self.rate, key, t0)
                {
                    notes[i].duration = seg_end - notes[i].start;
                    continue;
                }
                if let Some(&i) = active.get(&key) {
                    let n = &mut notes[i];
                    n.duration = n.duration.min(t0 - n.start);
                }
                let db = 20.0 * (amp / amax).log10();
                active.insert(key, notes.len());
                notes.push(DetectedNote {
                    start: t0,
                    duration: seg_end - t0,
                    pitch: key,
                    velocity: super::velocity_of(db, 0.0),
                });
            }
        }
        notes.retain(|n| n.duration >= opts.min_duration.max(0.0) && n.duration > 0.0);
        notes
    }
}

/// Local maxima of `mag` (bins of an `n`-point FFT at `rate`), with parabolic interpolation
/// on the log magnitude, sorted by frequency.
fn spectral_peaks(mag: &[f32], rate: f32, n: usize) -> Vec<Peak> {
    let max = mag.iter().copied().fold(0.0f32, f32::max);
    let floor = (max * 1e-3).max(1e-6);
    let lo = ((25.0 / rate) * n as f32) as usize;
    let hi = (((0.45 * rate) / rate) * n as f32) as usize;
    let mut out = Vec::new();
    for k in lo.max(2)..hi.min(mag.len().saturating_sub(2)) {
        let m = mag[k];
        if m < floor || m <= mag[k - 1] || m < mag[k + 1] {
            continue;
        }
        let (a, b, c) = (
            mag[k - 1].max(1e-12).ln(),
            m.ln(),
            mag[k + 1].max(1e-12).ln(),
        );
        let den = a - 2.0 * b + c;
        let p = if den.abs() > 1e-12 {
            (0.5 * (a - c) / den).clamp(-0.5, 0.5)
        } else {
            0.0
        };
        out.push(Peak {
            freq: (k as f32 + p) * rate / n as f32,
            amp: (b - 0.25 * (a - c) * p).exp(),
        });
    }
    out
}

/// Iterative harmonic-sum selection (see the module docs). A selected note takes its
/// fundamental and, from each overtone peak, only what a smooth spectral envelope predicts
/// (the smaller of its neighbouring partials), so a note an octave or a twelfth above, whose
/// fundamental shares that peak, keeps the rest.
fn select(peaks: &[Peak], rate: f32, opts: &Options) -> Vec<(u8, f32)> {
    if peaks.is_empty() {
        return Vec::new();
    }
    let sens = opts.sensitivity.clamp(0.0, 1.0);
    let top = peaks.iter().map(|p| p.amp).fold(0.0f32, f32::max);
    let floor = top * 10f32.powf(-(25.0 + 15.0 * sens) / 20.0);
    let stop = 0.25 - 0.15 * sens;
    let ratio = 2f32.powf(TOL / 12.0);
    // Residual amplitude of each peak.
    let mut res: Vec<f32> = peaks.iter().map(|p| p.amp).collect();
    let gone = |res: &[f32], i: usize| res[i] < 0.05 * peaks[i].amp;
    // The strongest remaining peak within the tolerance of `f`.
    let find = |res: &[f32], f: f32| -> Option<usize> {
        let (lo, hi) = (f / ratio, f * ratio);
        let start = peaks.partition_point(|p| p.freq < lo);
        (start..peaks.len())
            .take_while(|&i| peaks[i].freq <= hi)
            .filter(|&i| !gone(res, i))
            .max_by(|&a, &b| res[a].total_cmp(&res[b]))
    };
    let (pmin, pmax) = (
        opts.min_pitch.min(opts.max_pitch),
        opts.max_pitch.max(opts.min_pitch),
    );
    let mut out: Vec<(u8, f32)> = Vec::new();
    let mut first: Option<f32> = None;
    // Per harmonic: the matched peak, if any.
    let mut matched: Vec<Option<usize>> = Vec::with_capacity(HARMONICS);
    for _ in 0..MAX_POLY {
        let mut best: Option<(f32, u8, f32, Vec<Option<usize>>)> = None;
        for key in pmin..=pmax {
            let f0 = hz_of(key as f32);
            if f0 > 0.45 * rate {
                break;
            }
            if out.iter().any(|&(k, _)| k == key) {
                continue;
            }
            let Some(fund) = find(&res, f0) else {
                continue;
            };
            let a1 = res[fund];
            if a1 < floor {
                continue;
            }
            matched.clear();
            let mut score = 0.0;
            let mut amax = 0.0f32;
            for h in 1..=HARMONICS {
                let fh = f0 * h as f32;
                if fh > 0.45 * rate {
                    break;
                }
                let m = find(&res, fh);
                if let Some(i) = m {
                    score += res[i] / (h as f32).sqrt();
                    amax = amax.max(res[i]);
                }
                matched.push(m);
            }
            if a1 < 0.1 * amax {
                continue;
            }
            if best.as_ref().is_none_or(|b| score > b.0) {
                best = Some((score, key, a1, matched.clone()));
            }
        }
        let Some((score, key, a1, m)) = best else {
            break;
        };
        match first {
            None => first = Some(score),
            Some(f) if score < stop * f => break,
            _ => {}
        }
        let amp = |h: usize| m.get(h).copied().flatten().map_or(0.0, |i| res[i]);
        let take: Vec<(usize, f32)> = (0..m.len())
            .filter_map(|h| {
                let i = m[h]?;
                if h == 0 {
                    return Some((i, res[i]));
                }
                let next = if h + 1 < m.len() {
                    amp(h + 1)
                } else {
                    amp(h - 1)
                };
                Some((i, amp(h - 1).min(next).min(res[i])))
            })
            .collect();
        for (i, t) in take {
            res[i] -= t;
        }
        out.push((key, a1));
    }
    out.sort_by_key(|&(k, _)| k);
    out
}

/// Did `key`'s partials get louder at `t` (a re-attack) rather than ring through?
fn retriggered(x: &[f32], rate: f64, key: u8, t: f64) -> bool {
    let len = (0.04 * rate) as usize;
    let at = (t * rate) as isize;
    let gap = (0.005 * rate) as isize;
    let before = harmonic_energy(x, rate, key, at - gap - len as isize, len);
    let after = harmonic_energy(x, rate, key, at + gap, len);
    after > 2.0 * before
}

/// Energy of the first three harmonics of `key` over `x[start..start + len]` (Goertzel).
fn harmonic_energy(x: &[f32], rate: f64, key: u8, start: isize, len: usize) -> f32 {
    let f0 = hz_of(key as f32) as f64;
    let mut total = 0.0f32;
    for h in 1..=3 {
        let f = f0 * h as f64;
        if f > 0.45 * rate {
            break;
        }
        let w = 2.0 * std::f64::consts::PI * f / rate;
        let coeff = 2.0 * w.cos() as f32;
        let (mut s1, mut s2) = (0.0f32, 0.0f32);
        for j in 0..len {
            let i = start + j as isize;
            let v = if i < 0 || i as usize >= x.len() {
                0.0
            } else {
                x[i as usize]
            };
            let s = v + coeff * s1 - s2;
            s2 = s1;
            s1 = s;
        }
        total += s1 * s1 + s2 * s2 - coeff * s1 * s2;
    }
    total
}

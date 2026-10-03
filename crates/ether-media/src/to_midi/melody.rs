//! Melody: monophonic pitch tracking with YIN (difference function through an FFT
//! cross-correlation, cumulative-mean normalisation, absolute threshold, parabolic
//! interpolation), one frame per hop centred on its time. Segmentation: semitone-quantised
//! voiced runs, glitches smoothed, split on energy re-attacks; onsets sharpened on the
//! short-time envelope ([`Envelope::refine_onset`]).

use super::fft::Fft;
use super::prep::Envelope;
use super::{DetectedNote, Options, midi_of};

/// YIN's absolute threshold on the normalised difference.
const YIN_THRESHOLD: f32 = 0.15;

pub(crate) struct Melody {
    rate: f64,
    hop: usize,
    /// Integration window and lag range.
    w: usize,
    tau_min: usize,
    tau_max: usize,
    fft: Fft,
    re: Vec<f32>,
    im: Vec<f32>,
    rr: Vec<f32>,
    ri: Vec<f32>,
    cmnd: Vec<f32>,
    prefix: Vec<f32>,
    next: usize,
    total: usize,
    /// Per frame: fractional MIDI pitch (NaN = none), aperiodicity, level (dB).
    pitch: Vec<f32>,
    ap: Vec<f32>,
    db: Vec<f32>,
}

impl Melody {
    pub(crate) fn new(rate: f64, len: usize, opts: &Options) -> Self {
        let f_lo = super::hz_of(opts.min_pitch.min(opts.max_pitch) as f32 - 0.5).max(20.0);
        let f_hi = super::hz_of(opts.max_pitch.max(opts.min_pitch) as f32 + 0.5);
        let tau_max = ((rate / f_lo as f64).ceil() as usize + 2).min((rate / 20.0) as usize);
        let tau_min = ((rate / f_hi as f64).floor() as usize).max(2);
        let w = tau_max.max((rate * 0.02) as usize);
        let n = (w + tau_max + 1).next_power_of_two();
        let hop = ((rate * 0.004).round() as usize).max(1);
        let total = len.div_ceil(hop) + 1;
        Self {
            rate,
            hop,
            w,
            tau_min,
            tau_max,
            fft: Fft::new(n),
            re: vec![0.0; n],
            im: vec![0.0; n],
            rr: vec![0.0; n],
            ri: vec![0.0; n],
            cmnd: vec![0.0; tau_max + 1],
            prefix: vec![0.0; w + tau_max + 1],
            next: 0,
            total,
            pitch: Vec::with_capacity(total),
            ap: Vec::with_capacity(total),
            db: Vec::with_capacity(total),
        }
    }

    pub(crate) fn done(&self) -> bool {
        self.next >= self.total
    }

    pub(crate) fn progress(&self) -> f32 {
        self.next as f32 / self.total.max(1) as f32
    }

    /// Analyse frames worth up to `budget` analysis samples; returns the samples used.
    pub(crate) fn step(&mut self, x: &[f32], budget: usize) -> usize {
        let mut used = 0;
        while !self.done() && used < budget {
            self.frame(x);
            self.next += 1;
            used += self.hop;
        }
        used
    }

    fn frame(&mut self, x: &[f32]) {
        let (w, tmax) = (self.w, self.tau_max);
        let span = w + tmax;
        // The integration window `a` is centred on the frame time (`b` runs `tau_max` past it).
        let start = (self.next * self.hop) as isize - (w / 2) as isize;
        let at = |j: usize| -> f32 {
            let i = start + j as isize;
            if i < 0 || i as usize >= x.len() {
                0.0
            } else {
                x[i as usize]
            }
        };
        // z = a + i·b: a = the first `w` samples, b = all `span`.
        self.re.fill(0.0);
        self.im.fill(0.0);
        self.prefix[0] = 0.0;
        for j in 0..span {
            let v = at(j);
            if j < w {
                self.re[j] = v;
            }
            self.im[j] = v;
            self.prefix[j + 1] = self.prefix[j] + v * v;
        }
        let e0 = self.prefix[w];
        let level = 10.0 * (e0 / w as f32 + 1e-12).log10();
        if e0 <= 1e-10 {
            self.pitch.push(f32::NAN);
            self.ap.push(1.0);
            self.db.push(level);
            return;
        }
        let n = self.fft.len();
        self.fft.forward(&mut self.re, &mut self.im);
        // R_k = conj(A_k)·B_k, with A, B split out of Z.
        let (r_re, r_im) = (&mut self.rr, &mut self.ri);
        for k in 0..n {
            let m = (n - k) % n;
            let (zr, zi) = (self.re[k], self.im[k]);
            let (cr, ci) = (self.re[m], -self.im[m]);
            let (ar, ai) = ((zr + cr) * 0.5, (zi + ci) * 0.5);
            // B = (Z - conj(Z_{N-k})) / 2i
            let (dr, di) = ((zr - cr) * 0.5, (zi - ci) * 0.5);
            let (br, bi) = (di, -dr);
            r_re[k] = ar * br + ai * bi;
            r_im[k] = ar * bi - ai * br;
        }
        self.fft.inverse(r_re, r_im);
        // Cumulative-mean normalised difference.
        self.cmnd[0] = 1.0;
        let mut sum = 0.0f32;
        for tau in 1..=tmax {
            let et = self.prefix[tau + w] - self.prefix[tau];
            let d = (e0 + et - 2.0 * r_re[tau]).max(0.0);
            sum += d;
            self.cmnd[tau] = if sum > 0.0 {
                d * tau as f32 / sum
            } else {
                1.0
            };
        }
        let lo = self.tau_min.min(tmax);
        let mut best = None;
        let mut tau = lo;
        while tau <= tmax {
            if self.cmnd[tau] < YIN_THRESHOLD {
                while tau < tmax && self.cmnd[tau + 1] < self.cmnd[tau] {
                    tau += 1;
                }
                best = Some(tau);
                break;
            }
            tau += 1;
        }
        let tau = best.unwrap_or_else(|| {
            (lo..=tmax)
                .min_by(|&a, &b| self.cmnd[a].total_cmp(&self.cmnd[b]))
                .unwrap_or(lo)
        });
        let ap = self.cmnd[tau];
        let mut tf = tau as f32;
        if tau > 1 && tau < tmax {
            let (a, b, c) = (self.cmnd[tau - 1], self.cmnd[tau], self.cmnd[tau + 1]);
            let den = a - 2.0 * b + c;
            if den.abs() > 1e-9 {
                tf += (0.5 * (a - c) / den).clamp(-0.5, 0.5);
            }
        }
        let f0 = self.rate as f32 / tf;
        self.pitch.push(midi_of(f0));
        self.ap.push(ap);
        self.db.push(level);
    }

    /// Segment the pitch track into notes (source seconds).
    pub(crate) fn finish(&self, x: &[f32], opts: &Options) -> Vec<DetectedNote> {
        let sens = opts.sensitivity.clamp(0.0, 1.0);
        let n = self.pitch.len();
        let gmax = self.db.iter().copied().fold(-120.0, f32::max);
        let floor = (gmax - (20.0 + 40.0 * sens)).max(-90.0);
        let ap_thr = 0.2 + 0.2 * sens;
        let (pmin, pmax) = (
            opts.min_pitch.min(opts.max_pitch),
            opts.max_pitch.max(opts.min_pitch),
        );
        let mut q: Vec<Option<u8>> = (0..n)
            .map(|i| {
                let m = self.pitch[i];
                if m.is_nan() || self.ap[i] > ap_thr || self.db[i] < floor {
                    return None;
                }
                let k = m.round();
                (k >= pmin as f32 && k <= pmax as f32).then_some(k as u8)
            })
            .collect();
        // Runs under 30 ms are glitches or the glide of a pitch change.
        smooth(&mut q, ((0.03 * self.rate) as usize / self.hop).max(2));

        let frame_t = |i: usize| (i * self.hop) as f64 / self.rate;
        let half_hop = self.hop as f64 / self.rate / 2.0;
        // Runs of one pitch (gaps of up to 2 frames are bridged).
        let mut runs: Vec<(u8, usize, usize)> = Vec::new();
        for (i, v) in q.iter().enumerate() {
            let Some(p) = *v else { continue };
            match runs.last_mut() {
                Some((rp, _, end)) if *rp == p && i - *end <= 3 => *end = i,
                _ => runs.push((p, i, i)),
            }
        }

        if std::env::var("TO_MIDI_DEBUG").is_ok() {
            for i in 0..n.min(400) {
                eprintln!("{:.3} {:?} m={:.2} ap={:.3} db={:.1}", frame_t(i), q[i], self.pitch[i], self.ap[i], self.db[i]);
            }
            eprintln!("runs {runs:?}");
        }
        let env = Envelope::new(x, self.rate);
        let gmax_env = env.max_db();
        let mut notes: Vec<DetectedNote> = Vec::new();
        let legato_gap = ((0.08 * self.rate) as usize / self.hop).max(1);
        for (r, &(p, i0, i1)) in runs.iter().enumerate() {
            // A pitch change: the frames whose window straddles both notes are unvoiced;
            // with centred windows the change is in the middle of that gap.
            let t0 = match r.checked_sub(1).map(|r| runs[r]) {
                Some((_, _, e)) if i0 - e <= legato_gap => 0.5 * (frame_t(e) + frame_t(i0)),
                _ => (frame_t(i0) - half_hop).max(0.0),
            };
            let t1 = frame_t(i1) + half_hop;
            // Re-attacks inside the run split it: the held-peak envelope rises by 6 dB
            // within 40 ms (it holds over a low note's cycles, so those don't count).
            let mut cuts = vec![t0];
            let b = &env.db;
            let peak = env.peak(t0, t1);
            let back = ((0.04 / env.block_sec) as usize).max(2);
            let skip = ((0.06 / env.block_sec) as usize).max(1);
            let j1 = (((t1 - 0.03) / env.block_sec).max(0.0) as usize).min(b.len());
            let mut j = (((t0 + 0.05) / env.block_sec) as usize).max(back);
            while j < j1 {
                let min = b[j - back..j].iter().copied().fold(f32::MAX, f32::min);
                if b[j] - min >= 6.0 && b[j] >= peak - 25.0 {
                    let t = env.refine_onset(j as f64 * env.block_sec, 0.04, 0.02);
                    if t - cuts.last().copied().unwrap_or(t0) >= 0.06 && t < t1 - 0.03 {
                        cuts.push(t);
                    }
                    j += skip;
                } else {
                    j += 1;
                }
            }
            cuts.push(t1);
            for k in 0..cuts.len() - 1 {
                let mut s = cuts[k];
                let e = cuts[k + 1];
                if k == 0 {
                    let prev_start = notes.last().map_or(0.0, |n| n.start);
                    s = env
                        .refine_onset(s, 0.045, 0.07)
                        .clamp(prev_start + 0.001, (e - 0.005).max(prev_start + 0.001));
                }
                // The note ends where its level falls 30 dB below its peak.
                let pk = env.peak(s, (s + 0.1).min(e));
                let thr = pk - 30.0;
                let ja = (s / env.block_sec) as usize;
                let jb = ((e / env.block_sec) as usize).min(b.len());
                let end = (ja..jb)
                    .rev()
                    .find(|&j| b[j] >= thr)
                    .map_or(e, |j| ((j + 1) as f64 * env.block_sec).min(e));
                if let Some(last) = notes.last_mut()
                    && last.start + last.duration > s
                {
                    last.duration = (s - last.start).max(0.0);
                }
                notes.push(DetectedNote {
                    start: s,
                    duration: (end - s).max(0.0),
                    pitch: p,
                    velocity: super::velocity_of(pk, gmax_env),
                });
            }
        }
        notes.retain(|n| n.duration >= opts.min_duration.max(0.0) && n.duration > 0.0);
        notes
    }
}

/// Remove pitch-track glitches: voiced runs shorter than `min` frames take the value of
/// equal neighbours (or are dropped); short gaps between equal neighbours are filled.
pub(crate) fn smooth(q: &mut [Option<u8>], min: usize) {
    let n = q.len();
    let mut runs: Vec<(Option<u8>, usize, usize)> = Vec::new();
    for (i, v) in q.iter().enumerate() {
        match runs.last_mut() {
            Some((rv, _, end)) if *rv == *v => *end = i + 1,
            _ => runs.push((*v, i, i + 1)),
        }
    }
    for k in 0..runs.len() {
        let (v, a, b) = runs[k];
        if b - a >= min {
            continue;
        }
        let prev = k.checked_sub(1).map(|k| runs[k].0);
        let next = runs.get(k + 1).map(|r| r.0);
        let fill = match (prev, next) {
            (Some(Some(p)), Some(Some(nx))) if p == nx => Some(p),
            _ if v.is_some() => None,
            _ => continue,
        };
        for x in q.iter_mut().take(b.min(n)).skip(a) {
            *x = fill;
        }
    }
}

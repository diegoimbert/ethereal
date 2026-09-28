//! The Poly Synth's built-in wavetables: one entry per `Table` label, [`FRAMES`] frames each,
//! band-limited per octave ([`LEVELS`] mip levels, harmonics `1024 >> level`).
//!
//! Every frame is generated once (first instance, non-RT) at [`GEN_SIZE`] samples, analysed
//! with an FFT and re-synthesized per level with only the harmonics that level may play, so
//! the playback side never aliases below 20 kHz (see [`Wavetables::level_for`]). Frames are
//! normalized to a peak of 1 and DC-free. Tables live in a process-wide [`OnceLock`]
//! (~4 MB), shared by every instance.

use std::f64::consts::TAU;
use std::sync::OnceLock;

/// Number of built-in tables (the `Table` labels).
pub(crate) const TABLES: usize = 8;
/// Frames per table (`Position` morphs through them).
pub(crate) const FRAMES: usize = 16;
/// Mip levels: level `l` holds harmonics `1..=MAX_HARMONIC >> l`.
pub(crate) const LEVELS: usize = 11;
/// Harmonics of level 0.
const MAX_HARMONIC: usize = 1024;
/// Time-domain size frames are generated at (well above the kept harmonics: generating
/// discontinuous shapes naively only folds negligible energy into the kept band).
const GEN_SIZE: usize = 4096;

/// Samples per cycle of level `l`: 8 samples per period of its top harmonic (linear
/// interpolation stays clean), between 64 and 2048.
const fn level_size(l: usize) -> usize {
    let s = 8 * (MAX_HARMONIC >> l);
    if s > 2048 {
        2048
    } else if s < 64 {
        64
    } else {
        s
    }
}

/// Offset of each level inside a frame (each level stores `size + 1` samples: a guard
/// sample equal to the first one, so interpolation never wraps).
const LEVEL_OFFSETS: [usize; LEVELS + 1] = {
    let mut o = [0; LEVELS + 1];
    let mut l = 0;
    while l < LEVELS {
        o[l + 1] = o[l] + level_size(l) + 1;
        l += 1;
    }
    o
};
const FRAME_STRIDE: usize = LEVEL_OFFSETS[LEVELS];

/// All tables (read-only after generation).
pub(crate) struct Wavetables {
    data: Vec<f32>,
}

/// One band-limited cycle: `size` samples plus a guard sample.
#[derive(Clone, Copy)]
pub(crate) struct Cycle<'a> {
    pub samples: &'a [f32],
    pub size: f32,
}

impl Cycle<'_> {
    /// Linear interpolation at `phase` in `0..1`.
    #[inline]
    pub(crate) fn at(&self, phase: f32) -> f32 {
        let x = phase * self.size;
        let i = (x as usize).min(self.samples.len() - 2);
        let f = x - i as f32;
        let a = self.samples[i];
        a + (self.samples[i + 1] - a) * f
    }
}

impl Wavetables {
    /// The shared tables (generated on first use: call from non-RT code).
    pub(crate) fn get() -> &'static Wavetables {
        static TABLES_CELL: OnceLock<Wavetables> = OnceLock::new();
        TABLES_CELL.get_or_init(Wavetables::generate)
    }

    /// Mip level for a phase increment `dt` (cycles/sample): the richest level whose top
    /// harmonic folds back (if at all) above 20 kHz.
    #[inline]
    pub(crate) fn level_for(dt: f32, sample_rate: f32) -> usize {
        let limit = (1.0 - 20_000.0 / sample_rate).max(0.5);
        let mut l = 0;
        while l + 1 < LEVELS && (MAX_HARMONIC >> l) as f32 * dt > limit {
            l += 1;
        }
        l
    }

    /// One cycle of `table`, `frame`, `level`.
    #[inline]
    pub(crate) fn cycle(&self, table: usize, frame: usize, level: usize) -> Cycle<'_> {
        let base = (table * FRAMES + frame) * FRAME_STRIDE + LEVEL_OFFSETS[level];
        let size = level_size(level);
        Cycle {
            samples: &self.data[base..base + size + 1],
            size: size as f32,
        }
    }

    fn generate() -> Self {
        let mut data = vec![0.0f32; TABLES * FRAMES * FRAME_STRIDE];
        let mut fft = Fft::new(GEN_SIZE);
        let mut spec = vec![(0.0, 0.0); GEN_SIZE];
        let mut level_buf = vec![(0.0, 0.0); 2048];
        for table in 0..TABLES {
            for frame in 0..FRAMES {
                let t = frame as f64 / (FRAMES - 1) as f64;
                frame_spectrum(table, t, &mut fft, &mut spec);
                // Normalize on the richest level's peak.
                let mut peak = 0.0f64;
                let base = (table * FRAMES + frame) * FRAME_STRIDE;
                for level in 0..LEVELS {
                    let size = level_size(level);
                    let keep = (MAX_HARMONIC >> level).min(size / 2 - 1);
                    let buf = &mut level_buf[..size];
                    buf.fill((0.0, 0.0));
                    let scale = size as f64 / GEN_SIZE as f64;
                    for k in 1..=keep {
                        let (re, im) = spec[k];
                        buf[k] = (re * scale, im * scale);
                        buf[size - k] = (re * scale, -im * scale);
                    }
                    Fft::new(size).inverse(buf);
                    let out = &mut data[base + LEVEL_OFFSETS[level]..][..size + 1];
                    for (o, v) in out.iter_mut().zip(buf.iter()) {
                        *o = v.0 as f32;
                    }
                    out[size] = out[0];
                    if level == 0 {
                        peak = buf.iter().fold(0.0, |m, v| m.max(v.0.abs()));
                    }
                }
                let norm = if peak > 1e-9 { (1.0 / peak) as f32 } else { 1.0 };
                for v in &mut data[base..base + FRAME_STRIDE] {
                    *v *= norm;
                }
            }
        }
        Self { data }
    }
}

// --- frame definitions ---------------------------------------------------------------

/// Spectrum (bins `0..GEN_SIZE`, unnormalized forward FFT) of frame `t` (0..1) of `table`.
fn frame_spectrum(table: usize, t: f64, fft: &mut Fft, spec: &mut [(f64, f64)]) {
    let n = GEN_SIZE;
    match table {
        // Harmonic Sweep, Formant, Organ, Vocal: additive (sine partials).
        1 | 3 | 5 | 6 => {
            spec.fill((0.0, 0.0));
            for k in 1..=MAX_HARMONIC {
                let a = harmonic_amp(table, t, k);
                // Partial a·sin(2πkx) → bin k = -i·a·N/2.
                spec[k] = (0.0, -a * n as f64 / 2.0);
            }
        }
        _ => {
            for (i, s) in spec.iter_mut().enumerate() {
                let p = i as f64 / n as f64;
                *s = (time_sample(table, t, p), 0.0);
            }
            fft.forward(spec);
        }
    }
}

/// Time-domain frames: Basic Shapes, PWM, Digital, Metallic.
fn time_sample(table: usize, t: f64, p: f64) -> f64 {
    let sine = (TAU * p).sin();
    let tri = if p < 0.25 {
        4.0 * p
    } else if p < 0.75 {
        2.0 - 4.0 * p
    } else {
        4.0 * p - 4.0
    };
    let saw = if p < 0.5 { 2.0 * p } else { 2.0 * p - 2.0 };
    let square = if p < 0.5 { 1.0 } else { -1.0 };
    match table {
        // Basic Shapes: sine → triangle → saw → square.
        0 => {
            let x = t * 3.0;
            let (a, b, f) = if x < 1.0 {
                (sine, tri, x)
            } else if x < 2.0 {
                (tri, saw, x - 1.0)
            } else {
                (saw, square, x - 2.0)
            };
            a + (b - a) * f
        }
        // PWM: pulse width 50 % → 3 %.
        2 => {
            let w = 0.5 - 0.47 * t;
            if p < w { 1.0 } else { -1.0 }
        }
        // Digital: hard-synced saw sweeping the slave ratio 1 → 8, windowed so the reset
        // stays soft at the master cycle.
        4 => {
            let ratio = 1.0 + 7.0 * t;
            let slave = (p * ratio).fract();
            let win = (std::f64::consts::PI * p).sin().powf(0.25);
            (2.0 * slave - 1.0) * win
        }
        // Metallic: two-operator FM (ratios 7 and 3), index sweeping up.
        _ => {
            let i1 = 0.3 + 4.5 * t;
            let i2 = 1.5 * t * t;
            (TAU * p + i1 * (TAU * 7.0 * p).sin() + i2 * (TAU * 3.0 * p).sin()).sin()
        }
    }
}

/// Gaussian bump.
fn bump(x: f64, center: f64, width: f64) -> f64 {
    let d = (x - center) / width;
    (-0.5 * d * d).exp()
}

/// Vowel formants (Hz, relative gain) a, e, i, o, u.
const VOWELS: [[(f64, f64); 3]; 5] = [
    [(800.0, 1.0), (1150.0, 0.5), (2900.0, 0.1)],
    [(400.0, 1.0), (1600.0, 0.35), (2700.0, 0.2)],
    [(270.0, 1.0), (2300.0, 0.25), (3000.0, 0.15)],
    [(450.0, 1.0), (800.0, 0.6), (2830.0, 0.08)],
    [(325.0, 1.0), (700.0, 0.3), (2530.0, 0.05)],
];

/// Organ registrations (drawbar levels of harmonics 1, 2, 3, 4, 5, 6, 8).
const REGISTRATIONS: [[f64; 7]; 4] = [
    [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    [1.0, 0.7, 0.5, 0.0, 0.0, 0.0, 0.0],
    [1.0, 0.8, 0.8, 0.6, 0.3, 0.4, 0.3],
    [0.8, 1.0, 1.0, 0.9, 0.7, 0.7, 0.8],
];
const DRAWBARS: [usize; 7] = [1, 2, 3, 4, 5, 6, 8];

/// Additive frames: Harmonic Sweep, Formant, Organ, Vocal.
fn harmonic_amp(table: usize, t: f64, k: usize) -> f64 {
    let kf = k as f64;
    match table {
        // Harmonic Sweep: from a sine to a bright saw-like spectrum.
        1 => {
            let bright = 0.25 + 60.0 * t * t;
            (-(kf - 1.0) / bright).exp() / kf
        }
        // Formant: a saw spectrum with a resonant peak sweeping harmonics 2 → 40.
        3 => {
            let center = 2.0 + 38.0 * t * t;
            0.15 / kf + bump(kf, center, 1.2 + center * 0.08)
        }
        // Organ: drawbar registrations, crossfaded.
        5 => {
            let x = t * (REGISTRATIONS.len() - 1) as f64;
            let i = (x as usize).min(REGISTRATIONS.len() - 2);
            let f = x - i as f64;
            DRAWBARS
                .iter()
                .position(|&h| h == k)
                .map(|d| REGISTRATIONS[i][d] * (1.0 - f) + REGISTRATIONS[i + 1][d] * f)
                .unwrap_or(0.0)
        }
        // Vocal: formants of a, e, i, o, u over a 110 Hz glottal source.
        _ => {
            let x = t * (VOWELS.len() - 1) as f64;
            let i = (x as usize).min(VOWELS.len() - 2);
            let f = x - i as f64;
            let hz = kf * 110.0;
            let env = |v: &[(f64, f64); 3]| {
                v.iter()
                    .map(|&(fc, g)| g * bump(hz, fc, 60.0 + fc * 0.06))
                    .sum::<f64>()
            };
            let source = 1.0 / kf.powf(0.7);
            source * (env(&VOWELS[i]) * (1.0 - f) + env(&VOWELS[i + 1]) * f + 0.01)
        }
    }
}

// --- FFT -------------------------------------------------------------------------------

/// Minimal iterative radix-2 complex FFT (non-RT, table generation only).
struct Fft {
    n: usize,
}

impl Fft {
    fn new(n: usize) -> Self {
        debug_assert!(n.is_power_of_two());
        Self { n }
    }

    fn transform(&self, x: &mut [(f64, f64)], sign: f64) {
        let n = self.n;
        // Bit reversal.
        let mut j = 0;
        for i in 1..n {
            let mut bit = n >> 1;
            while j & bit != 0 {
                j ^= bit;
                bit >>= 1;
            }
            j |= bit;
            if i < j {
                x.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let ang = sign * TAU / len as f64;
            let (wr, wi) = (ang.cos(), ang.sin());
            for start in (0..n).step_by(len) {
                let (mut cr, mut ci) = (1.0, 0.0);
                for k in 0..len / 2 {
                    let a = x[start + k];
                    let b = x[start + k + len / 2];
                    let t = (b.0 * cr - b.1 * ci, b.0 * ci + b.1 * cr);
                    x[start + k] = (a.0 + t.0, a.1 + t.1);
                    x[start + k + len / 2] = (a.0 - t.0, a.1 - t.1);
                    let nr = cr * wr - ci * wi;
                    ci = cr * wi + ci * wr;
                    cr = nr;
                }
            }
            len <<= 1;
        }
    }

    /// `X[k] = Σ x[n]·e^{-2πikn/N}`.
    fn forward(&self, x: &mut [(f64, f64)]) {
        self.transform(x, -1.0);
    }

    /// `x[n] = (1/N) Σ X[k]·e^{2πikn/N}`.
    fn inverse(&self, x: &mut [(f64, f64)]) {
        self.transform(x, 1.0);
        let s = 1.0 / self.n as f64;
        for v in x.iter_mut() {
            v.0 *= s;
            v.1 *= s;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fft_roundtrip_and_sine_bin() {
        let n = 64;
        let fft = Fft::new(n);
        let mut x: Vec<(f64, f64)> = (0..n)
            .map(|i| ((TAU * 3.0 * i as f64 / n as f64).sin(), 0.0))
            .collect();
        let orig = x.clone();
        fft.forward(&mut x);
        assert!((x[3].1 + n as f64 / 2.0).abs() < 1e-9);
        assert!(x[5].0.abs() < 1e-9 && x[5].1.abs() < 1e-9);
        fft.inverse(&mut x);
        for (a, b) in x.iter().zip(&orig) {
            assert!((a.0 - b.0).abs() < 1e-9);
        }
    }

    #[test]
    fn tables_are_normalized_band_limited_and_dc_free() {
        let w = Wavetables::get();
        for table in 0..TABLES {
            for frame in [0, FRAMES / 2, FRAMES - 1] {
                let c = w.cycle(table, frame, 0);
                let n = c.size as usize;
                let peak = c.samples[..n].iter().fold(0.0f32, |m, v| m.max(v.abs()));
                assert!((peak - 1.0).abs() < 1e-3, "{table}/{frame}: peak {peak}");
                let dc: f32 = c.samples[..n].iter().sum::<f32>() / n as f32;
                assert!(dc.abs() < 1e-3, "{table}/{frame}: dc {dc}");
                assert_eq!(c.samples[n], c.samples[0]);
                // The top level is a pure sine (one harmonic) at the frame's level.
                let top = w.cycle(table, frame, LEVELS - 1);
                let m = top.size as usize;
                let mut fft_in: Vec<(f64, f64)> =
                    top.samples[..m].iter().map(|&v| (f64::from(v), 0.0)).collect();
                Fft::new(m).forward(&mut fft_in);
                for (k, v) in fft_in.iter().enumerate().take(m / 2).skip(2) {
                    assert!(v.0.hypot(v.1) < 1e-3, "{table}/{frame}: harmonic {k}");
                }
            }
        }
    }

    #[test]
    fn level_choice_keeps_aliases_above_20k() {
        let sr = 48_000.0;
        for hz in [20.0f32, 55.0, 110.0, 440.0, 1760.0, 7040.0] {
            let dt = hz / sr;
            let l = Wavetables::level_for(dt, sr);
            let top = (MAX_HARMONIC >> l) as f32 * hz;
            assert!(l == LEVELS - 1 || top <= sr - 20_000.0, "{hz}: {top}");
            // Not needlessly dull: the next richer level would alias.
            if l > 0 {
                assert!((MAX_HARMONIC >> (l - 1)) as f32 * hz > sr - 20_000.0);
            }
        }
    }
}

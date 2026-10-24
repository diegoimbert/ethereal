//! Loudness measurement (ITU-R BS.1770-4, EBU R 128 / Tech 3341 and Tech 3342): the
//! K-weighting filter, momentary (400 ms), short-term (3 s) and gated integrated loudness,
//! and the loudness range.
//!
//! - **K-weighting**: the two BS.1770 biquads (a high shelf of about +4 dB above 1.5 kHz,
//!   then a high-pass at about 38 Hz), their analog prototypes re-derived for any sample
//!   rate (the 48 kHz coefficients of the standard come out to 1e-8).
//! - **Blocks**: the K-weighted channels' mean squares are summed (weight 1 for left and
//!   right, mono counted once) per 100 ms sub-block. Momentary loudness is the mean of the
//!   last 4 sub-blocks, short-term the mean of the last 30, both as
//!   `-0.691 + 10 log10(energy)` LUFS; before a reset's first 400 ms / 3 s the window still
//!   holds silence.
//! - **Integrated** (gated, BS.1770-4 §2.8): every 400 ms block (75 % overlap: one per
//!   sub-block) louder than -70 LUFS goes into a histogram of 0.1 LU bins that keeps each
//!   bin's count and summed energy, so the means are exact and only the -10 LU relative
//!   gate falls on a bin edge (bins whose centre passes it count). Unbounded duration, fixed memory.
//! - **Loudness range** (Tech 3342): the short-term values, sampled every 100 ms once the
//!   first 3 s are in, gated at -70 LUFS absolute and -20 LU relative; LRA is the spread
//!   between their 10th and 95th percentiles (0.1 LU resolution).
//!
//! Real-time: buffers are allocated in [`Meter::new`]; [`Meter::push`] is O(1) per sample
//! and the gated values are recomputed once per sub-block (a scan of [`BINS`] bins).

/// Loudness reported for silence (LUFS): finite, below everything the meters show.
pub const FLOOR: f32 = -120.0;
/// Absolute gate (LUFS).
pub const ABSOLUTE_GATE: f64 = -70.0;
/// Histogram range: `ABSOLUTE_GATE ..= ABSOLUTE_GATE + BINS / 10` LUFS (louder blocks count in
/// the top bin, with their exact energy).
pub const BINS: usize = 900;
/// Sub-blocks of the momentary and short-term windows.
const MOMENTARY_SUBS: usize = 4;
const SHORT_SUBS: usize = 30;

/// `-0.691 + 10 log10(energy)`, [`FLOOR`] for silence.
pub fn lufs(energy: f64) -> f32 {
    if energy <= 1e-15 {
        FLOOR
    } else {
        ((-0.691 + 10.0 * energy.log10()) as f32).max(FLOOR)
    }
}

/// Energy of a loudness (inverse of [`lufs`]).
#[cfg_attr(not(test), allow(dead_code))]
pub fn energy(lufs: f64) -> f64 {
    10f64.powf((lufs + 0.691) / 10.0)
}

/// Direct-form-I biquad (`f64`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Biquad {
    pub b: [f64; 3],
    pub a: [f64; 2],
    x: [f64; 2],
    y: [f64; 2],
}

impl Biquad {
    #[inline]
    pub fn tick(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.b[1] * self.x[0] + self.b[2] * self.x[1]
            - self.a[0] * self.y[0]
            - self.a[1] * self.y[1];
        self.x = [x, self.x[0]];
        self.y = [super::flush(y), self.y[0]];
        y
    }

    pub fn reset(&mut self) {
        self.x = [0.0; 2];
        self.y = [0.0; 2];
    }
}

/// The two K-weighting stages at `sample_rate` (pre-filter shelf, then the RLB high-pass).
pub fn k_weighting(sample_rate: f64) -> [Biquad; 2] {
    use std::f64::consts::PI;
    // Stage 1: high shelf.
    let f0 = 1_681.974_450_955_533;
    let g = 3.999_843_853_973_347;
    let q = 0.707_175_236_955_419_6;
    let k = (PI * f0 / sample_rate).tan();
    let vh = 10f64.powf(g / 20.0);
    let vb = vh.powf(0.499_666_774_154_541_6);
    let a0 = 1.0 + k / q + k * k;
    let shelf = Biquad {
        b: [
            (vh + vb * k / q + k * k) / a0,
            2.0 * (k * k - vh) / a0,
            (vh - vb * k / q + k * k) / a0,
        ],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        ..Biquad::default()
    };
    // Stage 2: high-pass.
    let f0 = 38.135_470_876_024_44;
    let q = 0.500_327_037_323_877_3;
    let k = (PI * f0 / sample_rate).tan();
    let a0 = 1.0 + k / q + k * k;
    let hp = Biquad {
        b: [1.0, -2.0, 1.0],
        a: [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
        ..Biquad::default()
    };
    [shelf, hp]
}

/// Loudness histogram: count and summed energy per 0.1 LU bin from [`ABSOLUTE_GATE`].
#[derive(Clone)]
pub struct Histogram {
    count: Vec<u32>,
    energy: Vec<f64>,
    total: u64,
}

impl Histogram {
    /// Non-RT.
    pub fn new() -> Self {
        Self {
            count: vec![0; BINS],
            energy: vec![0.0; BINS],
            total: 0,
        }
    }

    pub fn clear(&mut self) {
        self.count.fill(0);
        self.energy.fill(0.0);
        self.total = 0;
    }

    fn bin_of(l: f64) -> usize {
        (((l - ABSOLUTE_GATE) * 10.0).floor().max(0.0) as usize).min(BINS - 1)
    }

    /// Bin centre (LUFS).
    fn centre(i: usize) -> f64 {
        ABSOLUTE_GATE + (i as f64 + 0.5) / 10.0
    }

    /// Add one block of `energy`, if it passes the absolute gate.
    pub fn add(&mut self, energy: f64) {
        if energy <= 0.0 {
            return;
        }
        let l = -0.691 + 10.0 * energy.log10();
        if l <= ABSOLUTE_GATE {
            return;
        }
        let i = Self::bin_of(l);
        self.count[i] = self.count[i].saturating_add(1);
        self.energy[i] += energy;
        self.total += 1;
    }

    /// Mean energy of the blocks whose bin starts at or above `from` (all, for `None`).
    fn mean_from(&self, from: usize) -> Option<f64> {
        let (mut n, mut e) = (0u64, 0.0);
        for i in from..BINS {
            n += u64::from(self.count[i]);
            e += self.energy[i];
        }
        (n > 0).then(|| e / n as f64)
    }

    /// First bin whose centre is at least `gate` LU below the absolute-gated mean.
    fn relative_from(&self, gate: f64) -> Option<usize> {
        let mean = self.mean_from(0)?;
        let threshold = -0.691 + 10.0 * mean.log10() - gate;
        Some(
            (0..BINS)
                .find(|&i| Self::centre(i) >= threshold)
                .unwrap_or(BINS),
        )
    }

    /// Gated integrated loudness (BS.1770-4: -10 LU relative gate), `None` before the first block.
    pub fn integrated(&self) -> Option<f64> {
        let from = self.relative_from(10.0)?;
        self.mean_from(from).map(|e| -0.691 + 10.0 * e.log10())
    }

    /// Loudness range (Tech 3342: -20 LU relative gate, 10th to 95th percentile).
    pub fn range(&self) -> Option<f64> {
        let from = self.relative_from(20.0)?;
        let n: u64 = self.count[from..].iter().map(|&c| u64::from(c)).sum();
        if n == 0 {
            return None;
        }
        let percentile = |p: f64| {
            // The value at rank round((n - 1) p), as Tech 3342's reference code.
            let rank = ((n - 1) as f64 * p).round() as u64;
            let mut seen = 0u64;
            for i in from..BINS {
                seen += u64::from(self.count[i]);
                if seen > rank {
                    return Self::centre(i);
                }
            }
            Self::centre(BINS - 1)
        };
        Some(percentile(0.95) - percentile(0.10))
    }

    /// Blocks counted.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn len(&self) -> u64 {
        self.total
    }
}

impl Default for Histogram {
    fn default() -> Self {
        Self::new()
    }
}

/// What a finished sub-block reports ([`Meter::push`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    /// LUFS (≥ [`FLOOR`]).
    pub momentary: f32,
    pub short_term: f32,
}

/// The BS.1770 meter of up to two channels.
pub struct Meter {
    filters: [[Biquad; 2]; 2],
    sub_len: usize,
    sub_pos: usize,
    sub_acc: f64,
    /// Ring of the last [`SHORT_SUBS`] sub-block energies.
    subs: [f64; SHORT_SUBS],
    sub_at: usize,
    /// Sub-blocks since the last reset (saturating).
    filled: usize,
    blocks: Histogram,
    shorts: Histogram,
    momentary: f64,
    short_term: f64,
    integrated: Option<f64>,
    range: Option<f64>,
}

impl Meter {
    /// Non-RT.
    pub fn new(sample_rate: f32) -> Self {
        let sr = f64::from(sample_rate.max(1.0));
        let k = k_weighting(sr);
        Self {
            filters: [k, k],
            sub_len: ((sr * 0.1).round() as usize).max(1),
            sub_pos: 0,
            sub_acc: 0.0,
            subs: [0.0; SHORT_SUBS],
            sub_at: 0,
            filled: 0,
            blocks: Histogram::new(),
            shorts: Histogram::new(),
            momentary: 0.0,
            short_term: 0.0,
            integrated: None,
            range: None,
        }
    }

    /// RT. Forget everything (filters, windows, gated values).
    pub fn reset(&mut self) {
        for f in self.filters.iter_mut().flatten() {
            f.reset();
        }
        self.sub_pos = 0;
        self.sub_acc = 0.0;
        self.subs = [0.0; SHORT_SUBS];
        self.sub_at = 0;
        self.filled = 0;
        self.blocks.clear();
        self.shorts.clear();
        self.momentary = 0.0;
        self.short_term = 0.0;
        self.integrated = None;
        self.range = None;
    }

    /// RT. One frame (`right` = `None` for mono). `Some` at the end of a sub-block.
    #[inline]
    pub fn push(&mut self, left: f32, right: Option<f32>) -> Option<Reading> {
        let mut e = 0.0;
        for (c, x) in [Some(left), right].into_iter().enumerate() {
            if let Some(x) = x {
                let x = if x.is_finite() { f64::from(x) } else { 0.0 };
                let [a, b] = &mut self.filters[c];
                let z = b.tick(a.tick(x));
                e += z * z;
            }
        }
        self.sub_acc += e;
        self.sub_pos += 1;
        if self.sub_pos < self.sub_len {
            return None;
        }
        Some(self.end_sub_block())
    }

    fn end_sub_block(&mut self) -> Reading {
        let e = self.sub_acc / self.sub_len as f64;
        self.sub_acc = 0.0;
        self.sub_pos = 0;
        self.subs[self.sub_at] = e;
        self.sub_at = (self.sub_at + 1) % SHORT_SUBS;
        self.filled = self.filled.saturating_add(1);
        let last = |n: usize| {
            (1..=n)
                .map(|k| self.subs[(self.sub_at + SHORT_SUBS - k) % SHORT_SUBS])
                .sum::<f64>()
                / n as f64
        };
        self.momentary = last(MOMENTARY_SUBS);
        self.short_term = last(SHORT_SUBS);
        if self.filled >= MOMENTARY_SUBS {
            self.blocks.add(self.momentary);
            self.integrated = self.blocks.integrated();
        }
        if self.filled >= SHORT_SUBS {
            self.shorts.add(self.short_term);
            self.range = self.shorts.range();
        }
        Reading {
            momentary: lufs(self.momentary),
            short_term: lufs(self.short_term),
        }
    }

    /// Momentary loudness (LUFS).
    pub fn momentary(&self) -> f32 {
        lufs(self.momentary)
    }

    /// Short-term loudness (LUFS).
    pub fn short_term(&self) -> f32 {
        lufs(self.short_term)
    }

    /// Gated integrated loudness (LUFS), [`FLOOR`] until a block passes the gate.
    pub fn integrated(&self) -> f32 {
        self.integrated.map_or(FLOOR, |l| (l as f32).max(FLOOR))
    }

    /// Loudness range (LU), 0 until the first gated short-term value.
    pub fn range(&self) -> f32 {
        self.range.unwrap_or(0.0) as f32
    }

    /// Whether the momentary and short-term windows hold only silence.
    pub fn is_quiet(&self) -> bool {
        self.subs.iter().all(|&e| e < 1e-15) && self.sub_acc < 1e-15
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn k_weighting_matches_the_standard_at_48k() {
        let [s, h] = k_weighting(48_000.0);
        let close = |a: f64, b: f64| (a - b).abs() < 1e-8;
        assert!(close(s.b[0], 1.535_124_859_586_97));
        assert!(close(s.b[1], -2.691_696_189_406_38));
        assert!(close(s.b[2], 1.198_392_810_852_85));
        assert!(close(s.a[0], -1.690_659_293_182_41));
        assert!(close(s.a[1], 0.732_480_774_215_85));
        assert!(close(h.a[0], -1.990_047_454_833_98));
        assert!(close(h.a[1], 0.990_072_250_366_21));
    }

    #[test]
    fn histogram_gates_and_means() {
        let mut h = Histogram::new();
        assert_eq!(h.integrated(), None);
        for _ in 0..10 {
            h.add(energy(-20.0));
        }
        // Below the absolute gate: ignored.
        h.add(energy(-80.0));
        assert_eq!(h.len(), 10);
        assert!((h.integrated().unwrap() + 20.0).abs() < 1e-9);
        // 40 LU quieter: under the relative gate, so it doesn't pull the value down.
        for _ in 0..10 {
            h.add(energy(-60.0));
        }
        assert!((h.integrated().unwrap() + 20.0).abs() < 1e-9);
    }
}

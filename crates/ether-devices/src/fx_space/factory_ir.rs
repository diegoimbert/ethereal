//! The factory impulse responses, synthesized at the engine rate (no recorded or third-party
//! IRs ship with Ethereal: every factory IR is generated here, deterministically, so it is
//! GPL like the rest of the code).
//!
//! Model, per channel: seeded white noise split into three bands (one-pole crossovers, the
//! bands sum back to the noise), each band with its own exponential decay (`RT60` low /
//! mid / high: high frequencies die first, like air and wall absorption), a build-up
//! envelope (diffuse field density growing), a handful of discrete early reflections at
//! seeded times, and a short fade at the end. Left and right use different seeds, so the
//! tail is decorrelated (wide) while sharing the same decay.

use super::FactoryIrSpec;

/// Synthesis recipe of one factory IR.
struct Recipe {
    /// RT60 in seconds for the low (< ~300 Hz), mid and high (> ~4 kHz) bands.
    rt60: [f64; 3],
    /// Gap before the diffuse tail starts (ms).
    predelay_ms: f64,
    /// Build-up time constant of the tail density (ms).
    buildup_ms: f64,
    /// Early reflections: count, spread after the pre-delay (ms), level.
    er_count: usize,
    er_span_ms: f64,
    er_level: f32,
    /// Crossover frequencies (Hz).
    split: [f64; 2],
    seed: u64,
}

fn recipe(id: &str) -> Recipe {
    let r = |rt60, predelay_ms, buildup_ms, er_count, er_span_ms, er_level, split, seed| Recipe {
        rt60,
        predelay_ms,
        buildup_ms,
        er_count,
        er_span_ms,
        er_level,
        split,
        seed,
    };
    match id {
        "room" => r([0.42, 0.38, 0.22], 2.0, 4.0, 8, 22.0, 0.55, [300.0, 4000.0], 11),
        "chamber" => r([0.95, 0.85, 0.5], 6.0, 10.0, 10, 38.0, 0.45, [300.0, 4500.0], 23),
        "plate" => r([1.55, 1.45, 1.15], 0.5, 1.5, 0, 0.0, 0.0, [250.0, 6000.0], 37),
        "hall" => r([2.6, 2.2, 1.3], 18.0, 45.0, 12, 75.0, 0.35, [300.0, 3500.0], 41),
        "cathedral" => r([4.8, 4.0, 2.2], 28.0, 90.0, 14, 120.0, 0.3, [250.0, 3000.0], 53),
        "ambience" => r([0.24, 0.2, 0.12], 1.0, 2.0, 6, 12.0, 0.6, [350.0, 5000.0], 67),
        // Unknown ids (appended without a recipe): a neutral medium room.
        _ => r([1.0, 0.9, 0.6], 8.0, 12.0, 8, 40.0, 0.4, [300.0, 4000.0], 97),
    }
}

/// xorshift64* (deterministic noise; quality is plenty for reverb tails).
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in [0, 1).
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in [-1, 1).
    fn bipolar(&mut self) -> f64 {
        self.unit() * 2.0 - 1.0
    }
}

/// Non-RT. The stereo IR of `spec` at `sample_rate` (`spec.length_seconds` long).
pub(crate) fn synthesize(spec: &FactoryIrSpec, sample_rate: f32) -> Vec<Vec<f32>> {
    let sr = sample_rate.max(1.0) as f64;
    let rec = recipe(spec.id);
    let len = ((spec.length_seconds * sr).round() as usize).max(16);
    let channels = spec.channels.clamp(1, 2) as usize;
    (0..channels)
        .map(|ch| channel(&rec, len, sr, rec.seed * 2 + ch as u64))
        .collect()
}

fn channel(rec: &Recipe, len: usize, sr: f64, seed: u64) -> Vec<f32> {
    let mut rng = Rng::new(seed);
    let mut out = vec![0.0f32; len];
    let pre = (rec.predelay_ms * 0.001 * sr) as usize;
    // One-pole low-pass coefficient for a crossover at `f`.
    let lp = |f: f64| (-std::f64::consts::TAU * f.min(sr * 0.45) / sr).exp();
    let (a0, a1) = (lp(rec.split[0]), lp(rec.split[1]));
    // Per-sample decay factor for RT60 `t`: 10^(-3/(t·sr)).
    let k = |t: f64| 10f64.powf(-3.0 / (t * sr));
    let decay = rec.rt60.map(k);
    let mut env = [1.0f64; 3];
    let (mut low, mut mid_lp) = (0.0f64, 0.0f64);
    let tau = (rec.buildup_ms * 0.001 * sr).max(1.0);
    for (n, o) in out.iter_mut().enumerate().skip(pre) {
        let x = rng.bipolar();
        low = x + (low - x) * a0;
        let rest = x - low;
        mid_lp = rest + (mid_lp - rest) * a1;
        let high = rest - mid_lp;
        let t = (n - pre) as f64;
        let build = 1.0 - (-t / tau).exp();
        let v = low * env[0] + mid_lp * env[1] + high * env[2];
        *o = (v * build) as f32;
        for b in 0..3 {
            env[b] *= decay[b];
        }
    }
    // Early reflections: seeded taps after the pre-delay, decreasing level, random sign.
    let span = rec.er_span_ms * 0.001 * sr;
    for i in 0..rec.er_count {
        let at = pre + (rng.unit() * span) as usize + 1;
        if at >= len {
            continue;
        }
        let level = rec.er_level * (1.0 - 0.6 * i as f32 / rec.er_count.max(1) as f32);
        let sign = if rng.unit() < 0.5 { -1.0 } else { 1.0 };
        out[at] += level * sign;
    }
    // Short fade at the end (no truncation click), and remove DC.
    let fade = (len / 20).max(1);
    for i in 0..fade {
        let g = 0.5 * (1.0 + (std::f64::consts::PI * (i + 1) as f64 / fade as f64).cos());
        out[len - fade + i] *= g as f32;
    }
    let mean = out.iter().map(|&v| v as f64).sum::<f64>() / len as f64;
    out.iter_mut().for_each(|v| *v -= mean as f32);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fx_space::FACTORY_IRS;

    #[test]
    fn deterministic_and_decaying() {
        for spec in FACTORY_IRS {
            let a = synthesize(spec, 48_000.0);
            let b = synthesize(spec, 48_000.0);
            assert_eq!(a, b, "{} is deterministic", spec.id);
            assert_eq!(a.len(), spec.channels as usize);
            let ch = &a[0];
            assert_eq!(ch.len(), (spec.length_seconds * 48_000.0).round() as usize);
            assert!(ch.iter().all(|v| v.is_finite()));
            // Energy of the first tenth is far above the last tenth.
            let tenth = ch.len() / 10;
            let e = |s: &[f32]| s.iter().map(|v| v * v).sum::<f32>();
            assert!(
                e(&ch[..tenth]) > 30.0 * e(&ch[ch.len() - tenth..]),
                "{} decays",
                spec.id
            );
            // Left and right decorrelated.
            if a.len() == 2 {
                let dot: f32 = a[0].iter().zip(&a[1]).map(|(l, r)| l * r).sum();
                let norm = (e(&a[0]) * e(&a[1])).sqrt();
                assert!((dot / norm).abs() < 0.3, "{} L/R correlation", spec.id);
            }
        }
    }
}

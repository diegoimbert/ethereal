//! Building blocks shared by the dynamics devices: a fixed-size param store, a Butterworth
//! TPT state-variable section (Linkwitz-Riley crossovers, detector high-pass) and the
//! sample-accurate gain ramp.

use ether_core::Smoother;
use ether_core::automation_rt::PARAM_GRID;
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::ParamId;

/// Plain param values of a device with `N` params (dense ids `0..N`), clamped to the
/// descriptor's ranges. Allocation-free after `new`.
#[derive(Clone, Debug)]
pub(super) struct Params<const N: usize> {
    values: [f64; N],
    ranges: [(f64, f64, f64); N],
}

impl<const N: usize> Params<N> {
    /// Non-RT. Defaults of `desc` (whose params must be the dense ids `0..N`).
    pub(super) fn new(desc: &DeviceDescriptor) -> Self {
        let mut values = [0.0; N];
        let mut ranges = [(0.0, 0.0, 0.0); N];
        for p in &desc.params {
            let i = p.id.0 as usize;
            values[i] = p.default;
            ranges[i] = (p.min, p.max, p.default);
        }
        Self { values, ranges }
    }

    /// Plain value (as `f32`).
    #[inline]
    pub(super) fn get(&self, id: ParamId) -> f32 {
        self.values[id.0 as usize] as f32
    }

    /// Toggle / enum as a bool (`>= 0.5`).
    #[inline]
    pub(super) fn on(&self, id: ParamId) -> bool {
        self.values[id.0 as usize] >= 0.5
    }

    pub(super) fn plain(&self, id: ParamId) -> Option<f64> {
        self.values.get(id.0 as usize).copied()
    }

    /// Set a plain value (clamped; NaN = default). `false` for an unknown id.
    pub(super) fn set(&mut self, id: ParamId, value: f64) -> bool {
        let Some(&(min, max, default)) = self.ranges.get(id.0 as usize) else {
            return false;
        };
        self.values[id.0 as usize] = if value.is_nan() {
            default
        } else {
            value.clamp(min, max)
        };
        true
    }
}

/// Coefficients of a Butterworth (Q = 1/√2) TPT state-variable section at one frequency.
#[derive(Clone, Copy, Debug)]
pub(super) struct SvfCoefs {
    k: f64,
    a1: f64,
    a2: f64,
    a3: f64,
}

impl SvfCoefs {
    /// Damping of a Butterworth section (`1/Q`).
    const K: f64 = std::f64::consts::SQRT_2;

    pub(super) fn new(freq: f64, sample_rate: f64) -> Self {
        // Keep the prewarp away from Nyquist (tan blows up).
        let f = freq.clamp(1.0, sample_rate * 0.49);
        let g = (std::f64::consts::PI * f / sample_rate).tan();
        let k = Self::K;
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        Self {
            k,
            a1,
            a2,
            a3: g * a2,
        }
    }
}

/// Outputs of one SVF tick.
#[derive(Clone, Copy, Debug)]
pub(super) struct SvfOut {
    pub lp: f64,
    pub bp: f64,
    pub hp: f64,
}

/// State of one SVF section (Simper's trapezoidal SVF), per channel.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Svf {
    ic1: f64,
    ic2: f64,
}

/// Flush values that would decay into the subnormal range (no FTZ on the audio thread).
#[inline]
fn flush(x: f64) -> f64 {
    if x.abs() < 1e-18 { 0.0 } else { x }
}

impl Svf {
    #[inline]
    pub(super) fn tick(&mut self, x: f64, c: &SvfCoefs) -> SvfOut {
        let v3 = x - self.ic2;
        let v1 = c.a1 * self.ic1 + c.a2 * v3;
        let v2 = self.ic2 + c.a2 * self.ic1 + c.a3 * v3;
        self.ic1 = flush(2.0 * v1 - self.ic1);
        self.ic2 = flush(2.0 * v2 - self.ic2);
        SvfOut {
            lp: v2,
            bp: v1,
            hp: x - c.k * v1 - v2,
        }
    }

    /// Second-order allpass with the same poles: `x - 2k·bp`. With `k = √2` this is exactly
    /// the sum of the 4th-order Linkwitz-Riley low and high pass at that frequency.
    #[inline]
    pub(super) fn allpass(&mut self, x: f64, c: &SvfCoefs) -> f64 {
        let o = self.tick(x, c);
        x - 2.0 * c.k * o.bp
    }
}

/// One Linkwitz-Riley (24 dB/oct) band split, per channel: two cascaded Butterworth low
/// passes and two cascaded high passes. `low + high` is an allpass (flat magnitude).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct LrSplit {
    lp: [Svf; 2],
    hp: [Svf; 2],
}

impl LrSplit {
    /// `(low, high)`.
    #[inline]
    pub(super) fn tick(&mut self, x: f64, c: &SvfCoefs) -> (f64, f64) {
        let l = self.lp[0].tick(x, c).lp;
        let l = self.lp[1].tick(l, c).lp;
        let h = self.hp[0].tick(x, c).hp;
        let h = self.hp[1].tick(h, c).hp;
        (l, h)
    }
}

/// Continuous gain param driven by events: each change ramps linearly over one automation
/// grid interval ([`PARAM_GRID`] samples), so grid-spaced automation events are rendered
/// as an exact piecewise-linear curve and knob moves don't click.
#[derive(Clone, Copy, Debug)]
pub(super) struct GainRamp(Smoother);

impl GainRamp {
    pub(super) fn new(value: f32) -> Self {
        Self(Smoother::new(value, 1.0, 48_000.0))
    }

    /// Ramp to `target` (`smooth`) or jump there.
    pub(super) fn set(&mut self, target: f32, smooth: bool) {
        if smooth {
            self.0.ramp_to(target, PARAM_GRID as u32);
        } else {
            self.0.set_immediate(target);
        }
    }

    /// Ramp to `target` over `samples`.
    pub(super) fn ramp(&mut self, target: f32, samples: u32) {
        self.0.ramp_to(target, samples);
    }

    #[inline]
    pub(super) fn tick(&mut self) -> f32 {
        self.0.tick()
    }
}

/// Number of samples of `ms` milliseconds (at least 1).
#[inline]
pub(super) fn samples(ms: f32, sample_rate: f32) -> f32 {
    (ms * 0.001 * sample_rate).max(1.0)
}

/// Sidechain channels (stereo) of the devices that take one.
pub(super) const SIDECHAIN_CHANNELS: usize = 2;

/// The sidechain slice `process_sidechain` got, if usable: at most
/// [`SIDECHAIN_CHANNELS`] channels, each at least `frames` long (else ignored).
pub(super) fn usable_sidechain<'a, 'b>(
    sidechain: &'a [&'b [f32]],
    frames: usize,
) -> &'a [&'b [f32]] {
    let n = sidechain.len().min(SIDECHAIN_CHANNELS);
    if n > 0 && sidechain[..n].iter().all(|c| c.len() >= frames) {
        &sidechain[..n]
    } else {
        &[]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// LR4 low + high = the SVF allpass (flat), and each band is -6 dB at the crossover.
    #[test]
    fn lr_split_sums_to_allpass() {
        let sr = 48_000.0;
        let c = SvfCoefs::new(1000.0, sr);
        let mut split = LrSplit::default();
        let mut ap = Svf::default();
        let mut max_err: f64 = 0.0;
        for i in 0..4800 {
            let x = if i == 0 { 1.0 } else { 0.0 };
            let (l, h) = split.tick(x, &c);
            let a = ap.allpass(x, &c);
            max_err = max_err.max((l + h - a).abs());
        }
        assert!(max_err < 1e-9, "{max_err}");
        // -6 dB per band at the crossover frequency (steady state).
        let mut split = LrSplit::default();
        let (mut sl, mut sh, mut n) = (0.0, 0.0, 0.0);
        for i in 0..48_000 {
            let x = (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / sr).sin();
            let (l, h) = split.tick(x, &c);
            if i > 24_000 {
                sl += l * l;
                sh += h * h;
                n += 1.0;
            }
        }
        let db = |s: f64| 10.0 * (2.0 * s / n).log10();
        assert!((db(sl) + 6.02).abs() < 0.05, "{}", db(sl));
        assert!((db(sh) + 6.02).abs() < 0.05, "{}", db(sh));
    }
}

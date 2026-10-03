//! Helpers shared by the fx-color devices: a plain-value param store, grid ramps, fixed
//! delays, a tiny deterministic RNG.

use ether_core::Smoother;
use ether_core::automation_rt::PARAM_GRID;
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::ParamId;

/// Plain values of a device's params (ids are dense `0..N`), clamped to their ranges.
#[derive(Clone, Debug)]
pub(crate) struct Values<const N: usize> {
    values: [f64; N],
    ranges: [(f64, f64); N],
    defaults: [f64; N],
}

impl<const N: usize> Values<N> {
    /// Non-RT. Defaults of `desc` (params must be ids `0..N` in order).
    pub(crate) fn new(desc: &DeviceDescriptor) -> Self {
        let mut v = Self {
            values: [0.0; N],
            ranges: [(0.0, 0.0); N],
            defaults: [0.0; N],
        };
        assert_eq!(desc.params.len(), N);
        for (i, p) in desc.params.iter().enumerate() {
            assert_eq!(p.id, ParamId(i as u32));
            v.values[i] = p.default;
            v.defaults[i] = p.default;
            v.ranges[i] = (p.min.min(p.max), p.max.max(p.min));
        }
        v
    }

    /// Store `value` (NaN = default); returns false for an unknown id.
    pub(crate) fn set(&mut self, id: ParamId, value: f64) -> bool {
        let i = id.0 as usize;
        if i >= N {
            return false;
        }
        let (lo, hi) = self.ranges[i];
        self.values[i] = if value.is_nan() {
            self.defaults[i]
        } else {
            value.clamp(lo, hi)
        };
        true
    }

    pub(crate) fn get(&self, id: ParamId) -> Option<f64> {
        self.values.get(id.0 as usize).copied()
    }

    /// Value as `f32` (known id).
    #[inline]
    pub(crate) fn f(&self, id: ParamId) -> f32 {
        self.values[id.0 as usize] as f32
    }

    /// Enum index (known id).
    #[inline]
    pub(crate) fn index(&self, id: ParamId) -> usize {
        self.values[id.0 as usize].round().max(0.0) as usize
    }

    #[inline]
    pub(crate) fn on(&self, id: ParamId) -> bool {
        self.values[id.0 as usize] >= 0.5
    }
}

/// Move `s` to `target`: a linear ramp over one automation grid interval (automation arrives
/// every [`PARAM_GRID`] samples, so automated params follow a sample-accurate piecewise
/// linear curve), or a jump when not smoothing (initial state).
#[inline]
pub(crate) fn glide(s: &mut Smoother, target: f32, smooth: bool) {
    if smooth {
        s.ramp_to(target, PARAM_GRID as u32);
    } else {
        s.set_immediate(target);
    }
}

/// A fixed delay of exactly `L` samples (`L >= 1`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct FixedDelay<const L: usize> {
    buf: [f32; L],
    at: usize,
}

impl<const L: usize> FixedDelay<L> {
    pub(crate) const fn new() -> Self {
        Self {
            buf: [0.0; L],
            at: 0,
        }
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::new();
    }

    #[inline]
    pub(crate) fn tick(&mut self, x: f32) -> f32 {
        let y = self.buf[self.at];
        self.buf[self.at] = x;
        self.at = if self.at + 1 == L { 0 } else { self.at + 1 };
        y
    }
}

/// xorshift32 (deterministic per instance, RT-safe).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Rng(u32);

impl Rng {
    pub(crate) const fn new(seed: u32) -> Self {
        Self(if seed == 0 { 0x9E37_79B9 } else { seed })
    }

    /// Uniform in `[0, 1)`.
    #[inline]
    pub(crate) fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// Stateless hash of an integer to `[-1, 1)` (sample & hold steps shared across channels).
#[inline]
pub(crate) fn hash_bipolar(i: i64) -> f32 {
    let mut x = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 29;
    (x >> 40) as f32 / (1u64 << 23) as f32 - 1.0
}

/// Flush denormal-range values to zero (no FTZ on the audio thread).
#[inline]
pub(crate) fn flush(x: f32) -> f32 {
    if x.abs() < 1e-20 { 0.0 } else { x }
}

#[inline]
pub(crate) fn db(x: f32) -> f32 {
    10f32.powf(x * 0.05)
}

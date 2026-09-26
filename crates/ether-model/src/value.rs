//! Small value types shared by the document, the protocol and the engine.
//!

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Musical time in beats (quarter notes), `f64`.
///
/// Positions on the arrangement timeline, clip lengths, note positions (relative to the clip
/// content start) and automation times are all in beats. Beats→seconds conversion goes
/// through the tempo map.
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd, Serialize, Deserialize, TS)]
pub struct Beats(pub f64);

/// Conventions for `f64` beats (decided for v0.1; see docs/CONTRACTS.md):
/// - positions/lengths are serialized as plain JSON numbers, never rounded on save;
/// - never compare beats with `==`: use [`Beats::approx_eq`] / [`Beats::EPSILON`];
/// - grid operations (quantize, snapping, bar math) go through [`Beats::snap`] /
///   [`Beats::floor_to`] / [`Beats::ceil_to`], which absorb float error so a value within
///   `EPSILON` of a grid line is treated as on it.
///
/// The UI mirrors these helpers exactly (`ui/src/state/beats.ts`).
impl Beats {
    /// Tolerance for beat comparisons (~1/1000 of a 1/1024 note).
    pub const EPSILON: f64 = 1e-6;
    pub const ZERO: Self = Self(0.0);

    pub fn approx_eq(self, other: Beats) -> bool {
        (self.0 - other.0).abs() <= Self::EPSILON
    }

    /// Nearest multiple of `grid` (`grid <= 0` returns `self`).
    pub fn snap(self, grid: Beats) -> Beats {
        if grid.0 <= 0.0 {
            return self;
        }
        Beats((self.0 / grid.0).round() * grid.0)
    }

    /// Largest multiple of `grid` that is `<= self + EPSILON`.
    pub fn floor_to(self, grid: Beats) -> Beats {
        if grid.0 <= 0.0 {
            return self;
        }
        Beats(((self.0 + Self::EPSILON) / grid.0).floor() * grid.0)
    }

    /// Smallest multiple of `grid` that is `>= self - EPSILON`.
    pub fn ceil_to(self, grid: Beats) -> Beats {
        if grid.0 <= 0.0 {
            return self;
        }
        Beats(((self.0 - Self::EPSILON) / grid.0).ceil() * grid.0)
    }
}

#[cfg(test)]
mod beats_tests {
    use super::Beats;

    #[test]
    fn beat_helpers_absorb_float_error() {
        let third = Beats(1.0 / 3.0);
        let x = Beats(third.0 * 3.0);
        assert!(x.approx_eq(Beats(1.0)));
        assert_eq!(Beats(0.26).snap(Beats(0.25)), Beats(0.25));
        assert_eq!(Beats(0.999_999_9).floor_to(Beats(1.0)), Beats(1.0));
        assert_eq!(Beats(1.000_000_1).ceil_to(Beats(1.0)), Beats(1.0));
        assert_eq!(Beats(1.3).ceil_to(Beats(1.0)), Beats(2.0));
    }
}

/// Absolute time in seconds, `f64`. Used for media positions (e.g. warp marker source time).
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd, Serialize, Deserialize, TS)]
pub struct Seconds(pub f64);

/// Gain in decibels. `Decibels::SILENCE` (-144 dB) is treated as -inf (JSON has no infinity).
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd, Serialize, Deserialize, TS)]
pub struct Decibels(pub f32);

impl Decibels {
    pub const UNITY: Self = Self(0.0);
    pub const SILENCE: Self = Self(-144.0);

    /// Linear amplitude factor.
    pub fn to_linear(self) -> f32 {
        if self.0 <= Self::SILENCE.0 {
            0.0
        } else {
            10f32.powf(self.0 / 20.0)
        }
    }
}

/// Stereo pan, -1.0 (hard left) ..= 1.0 (hard right).
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd, Serialize, Deserialize, TS)]
pub struct Pan(pub f32);

/// sRGB color packed as `0xRRGGBB`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub struct Color(pub u32);

/// Identifies a parameter *within* a device (CLAP param ids are `u32`; built-in devices
/// use small stable constants). The global address of a parameter is `(DeviceId, ParamId)`.
///
/// Serialized as a number; also accepts a numeric string, because JSON object keys are
/// strings (`Device::params` is keyed by `ParamId`) and serde's buffered (tagged-enum)
/// deserialization does not coerce them back to integers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, TS)]
pub struct ParamId(pub u32);

impl<'de> Deserialize<'de> for ParamId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl serde::de::Visitor<'_> for V {
            type Value = ParamId;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a u32 or a numeric string")
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<ParamId, E> {
                u32::try_from(v).map(ParamId).map_err(E::custom)
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<ParamId, E> {
                u32::try_from(v).map(ParamId).map_err(E::custom)
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<ParamId, E> {
                v.parse().map(ParamId).map_err(E::custom)
            }
        }
        d.deserialize_any(V)
    }
}

/// A musical time signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct TimeSignature {
    pub numerator: u8,
    /// Power of two (1, 2, 4, 8, 16, 32).
    pub denominator: u8,
}

impl Default for TimeSignature {
    fn default() -> Self {
        Self {
            numerator: 4,
            denominator: 4,
        }
    }
}

/// Launch / record quantization (Ableton's global and per-clip quantization).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum Quantization {
    /// Immediate (next audio block).
    None,
    /// Next multiple of `count` bars (respecting the time signature).
    Bars { count: u32 },
    /// Next multiple of `beats` (e.g. 0.25 = 1/16 note).
    Beats { beats: Beats },
}

impl Default for Quantization {
    fn default() -> Self {
        Self::Bars { count: 1 }
    }
}

/// A beat range `[start, end)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct BeatRange {
    pub start: Beats,
    pub end: Beats,
}

/// Opaque binary blob stored as base64 in JSON (e.g. plugin state).
#[derive(Clone, Debug, Default, PartialEq, Eq, TS)]
#[ts(type = "string")]
pub struct Base64Bytes(pub Vec<u8>);

impl Serialize for Base64Bytes {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use base64::Engine as _;
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for Base64Bytes {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use base64::Engine as _;
        let s = String::deserialize(d)?;
        base64::engine::general_purpose::STANDARD
            .decode(s.as_bytes())
            .map(Base64Bytes)
            .map_err(serde::de::Error::custom)
    }
}

/// Fractional-index sort key used to order siblings (tracks, devices, scenes) without
/// indices. Keys are opaque strings compared lexicographically; inserting between two
/// siblings creates a new key between theirs, touching no other entity (CRDT-friendly).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
pub struct OrderKey(pub String);

impl OrderKey {
    /// A key strictly between `before` and `after` (`None` = open end).
    ///
    /// Must be deterministic and produce keys of bounded growth. Implemented by the `model`
    /// node (base-62 midpoint algorithm, as in the `fractional-indexing` npm package, so the
    /// TypeScript mock can use the same scheme).
    pub fn between(before: Option<&OrderKey>, after: Option<&OrderKey>) -> OrderKey {
        let _ = (before, after);
        todo!("model node: fractional index midpoint")
    }
}

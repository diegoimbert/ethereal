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
    ///
    /// # Panics
    /// If a key is malformed or `before >= after`. Keys stored in a validated project are
    /// always well-formed; use [`OrderKey::try_between`] for untrusted input.
    pub fn between(before: Option<&OrderKey>, after: Option<&OrderKey>) -> OrderKey {
        match Self::try_between(before, after) {
            Ok(k) => k,
            Err(e) => panic!("OrderKey::between({before:?}, {after:?}): {e}"),
        }
    }

    /// Fallible [`OrderKey::between`]: errors if a key is malformed or `before >= after`.
    pub fn try_between(
        before: Option<&OrderKey>,
        after: Option<&OrderKey>,
    ) -> Result<OrderKey, String> {
        order_key::key_between(before.map(|k| k.0.as_str()), after.map(|k| k.0.as_str()))
            .map(OrderKey)
    }

    /// `n` sorted keys strictly between `before` and `after` (evenly spread when both ends
    /// are set). Same panics as [`OrderKey::between`].
    pub fn n_between(
        before: Option<&OrderKey>,
        after: Option<&OrderKey>,
        n: usize,
    ) -> Vec<OrderKey> {
        if n == 0 {
            return Vec::new();
        }
        if n == 1 {
            return vec![Self::between(before, after)];
        }
        match (before, after) {
            (_, None) => {
                let mut out = Vec::with_capacity(n);
                let mut c = Self::between(before, None);
                for _ in 1..n {
                    let next = Self::between(Some(&c), None);
                    out.push(std::mem::replace(&mut c, next));
                }
                out.push(c);
                out
            }
            (None, Some(_)) => {
                let mut out = Vec::with_capacity(n);
                let mut c = Self::between(None, after);
                for _ in 1..n {
                    let next = Self::between(None, Some(&c));
                    out.push(std::mem::replace(&mut c, next));
                }
                out.push(c);
                out.reverse();
                out
            }
            (Some(_), Some(_)) => {
                let mid = n / 2;
                let c = Self::between(before, after);
                let mut out = Self::n_between(before, Some(&c), mid);
                let tail = Self::n_between(Some(&c), after, n - mid - 1);
                out.push(c);
                out.extend(tail);
                out
            }
        }
    }

    /// `true` if this is a well-formed fractional-index key.
    pub fn is_valid(&self) -> bool {
        order_key::validate(&self.0).is_ok()
    }
}

/// Port of the `fractional-indexing` npm package (base-62), identical to
/// `ui/src/state/orderKey.ts` so UI- and engine-minted keys interleave.
mod order_key {
    const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    const ZERO: u8 = b'0';
    const LAST: u8 = b'z';
    const SMALLEST_INTEGER: &str = "A00000000000000000000000000";

    fn digit(d: u8) -> Result<usize, String> {
        DIGITS
            .iter()
            .position(|&c| c == d)
            .ok_or_else(|| format!("invalid order key digit: {}", d as char))
    }

    fn s(b: &[u8]) -> String {
        String::from_utf8(b.to_vec()).expect("order keys are ASCII")
    }

    /// Midpoint of fractional parts `a < b` (`None` = +infinity).
    fn midpoint(a: &[u8], b: Option<&[u8]>) -> Result<Vec<u8>, String> {
        if let Some(b) = b {
            if a >= b {
                return Err(format!("{} >= {}", s(a), s(b)));
            }
            if b.last() == Some(&ZERO) {
                return Err("trailing zero".into());
            }
        }
        if a.last() == Some(&ZERO) {
            return Err("trailing zero".into());
        }
        if let Some(b) = b {
            let mut n = 0;
            while n < b.len() && a.get(n).copied().unwrap_or(ZERO) == b[n] {
                n += 1;
            }
            if n > 0 {
                let a_rest = if n <= a.len() { &a[n..] } else { &[][..] };
                let mut out = b[..n].to_vec();
                out.extend(midpoint(a_rest, Some(&b[n..]))?);
                return Ok(out);
            }
        }
        let digit_a = match a.first() {
            Some(&c) => digit(c)?,
            None => 0,
        };
        let digit_b = match b {
            Some(b) => digit(b[0])?,
            None => DIGITS.len(),
        };
        if digit_b - digit_a > 1 {
            // JS Math.round(0.5 * (a + b)) rounds .5 up.
            let mid = (digit_a + digit_b).div_ceil(2);
            return Ok(vec![DIGITS[mid]]);
        }
        if let Some(b) = b {
            if b.len() > 1 {
                return Ok(vec![b[0]]);
            }
        }
        let mut out = vec![DIGITS[digit_a]];
        let a_rest = if a.is_empty() { &[][..] } else { &a[1..] };
        out.extend(midpoint(a_rest, None)?);
        Ok(out)
    }

    fn integer_length(head: u8) -> Result<usize, String> {
        match head {
            b'a'..=b'z' => Ok((head - b'a') as usize + 2),
            b'A'..=b'Z' => Ok((b'Z' - head) as usize + 2),
            _ => Err(format!("invalid order key head: {}", head as char)),
        }
    }

    fn integer_part(key: &[u8]) -> Result<&[u8], String> {
        let head = *key.first().ok_or("empty order key")?;
        let len = integer_length(head)?;
        if len > key.len() {
            return Err(format!("invalid order key: {}", s(key)));
        }
        Ok(&key[..len])
    }

    pub(super) fn validate(key: &str) -> Result<(), String> {
        if key == SMALLEST_INTEGER {
            return Err(format!("invalid order key: {key}"));
        }
        let k = key.as_bytes();
        let int = integer_part(k)?;
        let frac = &k[int.len()..];
        if frac.last() == Some(&ZERO) {
            return Err(format!("invalid order key: {key}"));
        }
        for &c in &k[1..] {
            digit(c)?;
        }
        Ok(())
    }

    fn increment_integer(x: &[u8]) -> Result<Option<Vec<u8>>, String> {
        let head = x[0];
        let mut digs = x[1..].to_vec();
        let mut carry = true;
        for d in digs.iter_mut().rev() {
            if !carry {
                break;
            }
            let v = digit(*d)? + 1;
            if v == DIGITS.len() {
                *d = ZERO;
            } else {
                *d = DIGITS[v];
                carry = false;
            }
        }
        if !carry {
            let mut out = vec![head];
            out.extend(digs);
            return Ok(Some(out));
        }
        if head == b'Z' {
            return Ok(Some(vec![b'a', ZERO]));
        }
        if head == b'z' {
            return Ok(None);
        }
        let h = head + 1;
        if h > b'a' {
            digs.push(ZERO);
        } else {
            digs.pop();
        }
        let mut out = vec![h];
        out.extend(digs);
        Ok(Some(out))
    }

    fn decrement_integer(x: &[u8]) -> Result<Option<Vec<u8>>, String> {
        let head = x[0];
        let mut digs = x[1..].to_vec();
        let mut borrow = true;
        for d in digs.iter_mut().rev() {
            if !borrow {
                break;
            }
            let v = digit(*d)?;
            if v == 0 {
                *d = LAST;
            } else {
                *d = DIGITS[v - 1];
                borrow = false;
            }
        }
        if !borrow {
            let mut out = vec![head];
            out.extend(digs);
            return Ok(Some(out));
        }
        if head == b'a' {
            return Ok(Some(vec![b'Z', LAST]));
        }
        if head == b'A' {
            return Ok(None);
        }
        let h = head - 1;
        if h < b'Z' {
            digs.push(LAST);
        } else {
            digs.pop();
        }
        let mut out = vec![h];
        out.extend(digs);
        Ok(Some(out))
    }

    pub(super) fn key_between(a: Option<&str>, b: Option<&str>) -> Result<String, String> {
        if let Some(a) = a {
            validate(a)?;
        }
        if let Some(b) = b {
            validate(b)?;
        }
        if let (Some(a), Some(b)) = (a, b) {
            if a >= b {
                return Err(format!("{a} >= {b}"));
            }
        }
        let out = match (a.map(str::as_bytes), b.map(str::as_bytes)) {
            (None, None) => vec![b'a', ZERO],
            (None, Some(b)) => {
                let ib = integer_part(b)?;
                let fb = &b[ib.len()..];
                if ib == SMALLEST_INTEGER.as_bytes() {
                    let mut out = ib.to_vec();
                    out.extend(midpoint(&[], Some(fb))?);
                    out
                } else if ib < b {
                    ib.to_vec()
                } else {
                    decrement_integer(ib)?.ok_or("cannot decrement any more")?
                }
            }
            (Some(a), None) => {
                let ia = integer_part(a)?;
                let fa = &a[ia.len()..];
                match increment_integer(ia)? {
                    Some(i) => i,
                    None => {
                        let mut out = ia.to_vec();
                        out.extend(midpoint(fa, None)?);
                        out
                    }
                }
            }
            (Some(a), Some(b)) => {
                let ia = integer_part(a)?;
                let fa = &a[ia.len()..];
                let ib = integer_part(b)?;
                let fb = &b[ib.len()..];
                if ia == ib {
                    let mut out = ia.to_vec();
                    out.extend(midpoint(fa, Some(fb))?);
                    out
                } else {
                    let i = increment_integer(ia)?.ok_or("cannot increment any more")?;
                    if i.as_slice() < b {
                        i
                    } else {
                        let mut out = ia.to_vec();
                        out.extend(midpoint(fa, None)?);
                        out
                    }
                }
            }
        };
        Ok(s(&out))
    }
}

#[cfg(test)]
mod order_key_tests {
    use super::OrderKey;

    fn k(s: &str) -> OrderKey {
        OrderKey(s.into())
    }

    #[test]
    fn matches_fractional_indexing_vectors() {
        // Same vectors as ui/src/state/orderKey.test.ts.
        let cases: &[(Option<&str>, Option<&str>, &str)] = &[
            (None, None, "a0"),
            (None, Some("a0"), "Zz"),
            (None, Some("Zz"), "Zy"),
            (Some("a0"), None, "a1"),
            (Some("a1"), None, "a2"),
            (Some("a0"), Some("a1"), "a0V"),
            (Some("a1"), Some("a2"), "a1V"),
            (Some("a0V"), Some("a1"), "a0l"),
            (Some("Zz"), Some("a0"), "ZzV"),
            (Some("Zz"), Some("a1"), "a0"),
            (None, Some("Y00"), "Xzzz"),
            (Some("bzz"), None, "c000"),
            (Some("a0"), Some("a0V"), "a0G"),
            (Some("a0"), Some("a0G"), "a08"),
            (Some("b125"), Some("b129"), "b127"),
            (Some("a0"), Some("a1V"), "a1"),
            (Some("Zz"), Some("a01"), "a0"),
            (None, Some("a0V"), "a0"),
            (None, Some("b999"), "b99"),
            (
                Some("zzzzzzzzzzzzzzzzzzzzzzzzzzy"),
                None,
                "zzzzzzzzzzzzzzzzzzzzzzzzzzz",
            ),
            (
                Some("zzzzzzzzzzzzzzzzzzzzzzzzzzz"),
                None,
                "zzzzzzzzzzzzzzzzzzzzzzzzzzzV",
            ),
        ];
        for (a, b, want) in cases {
            let a = a.map(k);
            let b = b.map(k);
            assert_eq!(
                OrderKey::between(a.as_ref(), b.as_ref()).0,
                *want,
                "{a:?} {b:?}"
            );
        }
    }

    #[test]
    fn rejects_bad_input() {
        for (a, b) in [
            (Some("a1"), Some("a0")),
            (Some("a0"), Some("a0")),
            (Some("a"), None),
            (Some("a0!"), None),
            (Some("a00"), None),
            (Some("A00000000000000000000000000"), None),
        ] {
            let a = a.map(k);
            let b = b.map(k);
            assert!(OrderKey::try_between(a.as_ref(), b.as_ref()).is_err());
        }
    }

    #[test]
    fn stays_ordered_and_n_between() {
        let mut keys = vec![OrderKey::between(None, None)];
        for i in 0..200 {
            match i % 3 {
                0 => keys.insert(0, OrderKey::between(None, Some(&keys[0]))),
                1 => keys.push(OrderKey::between(keys.last(), None)),
                _ => {
                    let j = keys.len() / 2;
                    let nk = OrderKey::between(Some(&keys[j - 1]), Some(&keys[j]));
                    keys.insert(j, nk);
                }
            }
        }
        let mut sorted = keys.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(keys, sorted);
        assert!(keys.iter().all(OrderKey::is_valid));

        let three: Vec<String> = OrderKey::n_between(None, None, 3)
            .into_iter()
            .map(|k| k.0)
            .collect();
        assert_eq!(three, ["a0", "a1", "a2"]);
        for (a, b) in [
            (None, None),
            (Some(k("a0")), None),
            (None, Some(k("a0"))),
            (Some(k("a0")), Some(k("a1"))),
        ] {
            let ks = OrderKey::n_between(a.as_ref(), b.as_ref(), 7);
            assert_eq!(ks.len(), 7);
            assert!(ks.windows(2).all(|w| w[0] < w[1]));
            assert!(a.as_ref().is_none_or(|a| &ks[0] > a));
            assert!(b.as_ref().is_none_or(|b| ks.last().unwrap() < b));
        }
    }
}

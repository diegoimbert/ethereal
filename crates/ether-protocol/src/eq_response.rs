//! Filter magnitude responses shared by the engine's EQ-style devices and the UI's `EqCurve`
//! widget (v0.2, frozen by the v0.2 contracts; UI mirror, CONTRACTS.md
//! §12.15).
//!
//! Every shape is a Simper TPT state-variable filter (`ether_devices::dsp::svf`, the EQ's
//! filter): `y = m0·x + m1·band + m2·low` with coefficients from [`svf_coefs`]. Its exact
//! digital magnitude at `f` is `|m0 + (m1·s + m2) / (s² + k·s + 1)|` evaluated at
//! `s = j·tan(π·f/fs) / g` (the TPT structure is a bilinear transform). `*24` shapes are two
//! identical 12 dB/oct stages (magnitude squared).
//!
//! **Decramped** (matched to the analog prototype): a plain bilinear design squeezes every
//! response towards Nyquist (a 12 kHz high cut at 44.1 kHz was 19 dB off its analog
//! response at 20 kHz, a 10 kHz bell 3.8 dB). [`svf_coefs`] instead designs each band as a
//! *matched* second-order filter (M. Vicanek, "Matched Second Order Digital Filters", 2016):
//! poles by impulse invariance (`z = e^{sT}`, exact pole frequency and damping), and the
//! numerator by matching the analog magnitude at DC, at Nyquist and at the centre frequency
//! (at most `fs/4`); a notch keeps its zeros on the unit circle at the centre. The digital
//! biquad is then written back as SVF coefficients (`g`, `k` from its poles, `m*` from its
//! numerator: the SVF's three outputs span every second-order numerator), so the device keeps
//! its click-free coefficient glides. Below a few kHz this is the bilinear response to a
//! hundredth of a dB; near Nyquist it stays within about 2 dB of analog even for a narrow
//! bell at 19.5 kHz (bilinear: 10 dB).
//!
use std::f64::consts::PI;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Filter shapes (append-only).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum EqShape {
    /// 12 dB/oct high-pass.
    LowCut,
    LowShelf,
    Bell,
    Notch,
    HighShelf,
    /// 12 dB/oct low-pass.
    HighCut,
    /// 24 dB/oct high-pass (two stages).
    LowCut24,
    /// 24 dB/oct low-pass (two stages).
    HighCut24,
    /// Band-pass, unity gain at the centre.
    BandPass,
}

/// SVF coefficients (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SvfCoefs {
    pub g: f64,
    pub k: f64,
    pub m0: f64,
    pub m1: f64,
    pub m2: f64,
}

/// The analog prototype of one stage: pole frequency (Hz), damping `k` and the output mix,
/// `H(s) = m0 + (m1·s + m2) / (s² + k·s + 1)` with `s = j·f / fp`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Prototype {
    fp: f64,
    k: f64,
    m0: f64,
    m1: f64,
    m2: f64,
}

impl Prototype {
    fn new(shape: EqShape, fc: f64, gain_db: f64, q: f64) -> Self {
        let q = q.max(0.01);
        let k = 1.0 / q;
        let a = 10f64.powf(gain_db / 40.0);
        let p = |fp, k, m0, m1, m2| Prototype { fp, k, m0, m1, m2 };
        match shape {
            EqShape::LowCut | EqShape::LowCut24 => p(fc, k, 1.0, -k, -1.0),
            EqShape::HighCut | EqShape::HighCut24 => p(fc, k, 0.0, 0.0, 1.0),
            EqShape::Notch => p(fc, k, 1.0, -k, 0.0),
            EqShape::BandPass => p(fc, k, 0.0, k, 0.0),
            EqShape::Bell => {
                let k = 1.0 / (q * a);
                p(fc, k, 1.0, k * (a * a - 1.0), 0.0)
            }
            EqShape::LowShelf => p(fc / a.sqrt(), k, 1.0, k * (a - 1.0), a * a - 1.0),
            EqShape::HighShelf => p(fc * a.sqrt(), k, a * a, k * (1.0 - a) * a, 1.0 - a * a),
        }
    }

    /// Complex response at `f` Hz.
    fn at(&self, f: f64) -> (f64, f64) {
        let w = f / self.fp;
        // (m1·jw + m2) / ((1 - w²) + j·k·w)
        let (dr, di) = (1.0 - w * w, self.k * w);
        let (nr, ni) = (self.m2, self.m1 * w);
        let den = dr * dr + di * di;
        (
            self.m0 + (nr * dr + ni * di) / den,
            (ni * dr - nr * di) / den,
        )
    }

    fn mag2(&self, f: f64) -> f64 {
        let (re, im) = self.at(f);
        re * re + im * im
    }
}

/// `|1 + a1·z⁻¹ + a2·z⁻²|²`-style magnitude of a second-order polynomial at `w` (rad/sample)
/// from its `(P0, P1, P2)` = `((c0+c1+c2)², (c0-c1+c2)², -4·c0·c2)`.
fn poly_mag2(p: [f64; 3], w: f64) -> f64 {
    let s = (w * 0.5).sin().powi(2);
    let (phi0, phi1) = (1.0 - s, s);
    p[0] * phi0 + p[1] * phi1 + p[2] * 4.0 * phi0 * phi1
}

/// Coefficients of one stage of `shape` at `freq` Hz, `gain_db` (shelves, bell) and `q`:
/// the matched design of the module docs. Identical to the EQ device's `Coefs::new`
/// (cutoff clamped to `1..=0.49·fs`, `q >= 0.01`; a bell or shelf at 0 dB is exactly
/// pass-through).
pub fn svf_coefs(shape: EqShape, freq: f64, gain_db: f64, q: f64, sample_rate: f64) -> SvfCoefs {
    let fs = sample_rate;
    let fc = freq.clamp(1.0, fs * 0.49);
    let proto = Prototype::new(shape, fc, gain_db, q);
    // Poles by impulse invariance.
    let w0 = 2.0 * PI * proto.fp / fs;
    let zeta = proto.k * 0.5;
    let decay = (-zeta * w0).exp();
    let a1 = if zeta <= 1.0 {
        -2.0 * decay * ((1.0 - zeta * zeta).sqrt() * w0).cos()
    } else {
        -2.0 * decay * ((zeta * zeta - 1.0).sqrt() * w0).cosh()
    };
    let a2 = decay * decay;
    let (g, k) = poles_to_svf(a1, a2);
    let flat = gain_db == 0.0
        && matches!(
            shape,
            EqShape::Bell | EqShape::LowShelf | EqShape::HighShelf
        );
    if flat {
        return SvfCoefs {
            g,
            k,
            m0: 1.0,
            m1: 0.0,
            m2: 0.0,
        };
    }
    let a = [(1.0 + a1 + a2).powi(2), (1.0 - a1 + a2).powi(2), -4.0 * a2];
    let [b0, b1, b2] = if shape == EqShape::Notch {
        // Zeros on the unit circle at the centre, unity at DC.
        let c = -2.0 * (2.0 * PI * fc / fs).cos();
        let scale = (1.0 + a1 + a2) / (2.0 + c);
        [scale, scale * c, scale]
    } else {
        // Match |H|² at DC, Nyquist and the centre (at most fs/4).
        let wm = w0.min(PI * 0.5);
        let big0 = proto.mag2(0.0) * a[0];
        let big1 = proto.mag2(fs * 0.5) * a[1];
        let s = (wm * 0.5).sin().powi(2);
        let (phi0, phi1) = (1.0 - s, s);
        let big2 =
            (proto.mag2(wm * fs / (2.0 * PI)) * poly_mag2(a, wm) - big0 * phi0 - big1 * phi1)
                / (4.0 * phi0 * phi1);
        // b0 + b1 + b2 and b0 - b1 + b2, signed like the analog response there.
        let sign = |x: f64| if x < 0.0 { -1.0 } else { 1.0 };
        let w = big0.max(0.0).sqrt() * sign(proto.at(0.0).0);
        let v = big1.max(0.0).sqrt() * sign(proto.at(fs * 0.5).0);
        let b1 = (w - v) * 0.5;
        let sum = (w + v) * 0.5;
        // b0·b2 = -B2/4, b0 + b2 = sum: the minimum-phase pair (|b0| >= |b2|).
        let r = (sum * sum + big2).max(0.0).sqrt();
        let b0 = if sum >= 0.0 {
            (sum + r) * 0.5
        } else {
            (sum - r) * 0.5
        };
        [b0, b1, sum - b0]
    };
    // Back to SVF form: the SVF's response is (m0·D + m1·g·(1 - z⁻²) + m2·g²·(1 + z⁻¹)²) / D
    // with D = (1 + g·k + g²)·(1 + a1·z⁻¹ + a2·z⁻²).
    let n = 1.0 + g * k + g * g;
    let m0 = (b0 - b1 + b2) * n / 4.0;
    let m2 = ((b0 + b1 + b2) * n - m0 * 4.0 * g * g) / (4.0 * g * g);
    let m1 = (b0 * n - m0 * n - m2 * g * g) / g;
    SvfCoefs { g, k, m0, m1, m2 }
}

/// The plain bilinear-transform design of one stage (the EQ's before decramping): the
/// coefficients every other device's filters use (calibrated against reference renders).
/// Cutoff clamped to `1..=0.49·fs`, `q >= 0.01`.
pub fn svf_coefs_bilinear(
    shape: EqShape,
    freq: f64,
    gain_db: f64,
    q: f64,
    sample_rate: f64,
) -> SvfCoefs {
    let fc = freq.clamp(1.0, sample_rate * 0.49);
    let g = (PI * fc / sample_rate).tan();
    let q = q.max(0.01);
    let k = 1.0 / q;
    let a = 10f64.powf(gain_db / 40.0);
    let c = |g, k, m0, m1, m2| SvfCoefs { g, k, m0, m1, m2 };
    match shape {
        EqShape::LowCut | EqShape::LowCut24 => c(g, k, 1.0, -k, -1.0),
        EqShape::HighCut | EqShape::HighCut24 => c(g, k, 0.0, 0.0, 1.0),
        EqShape::Notch => c(g, k, 1.0, -k, 0.0),
        EqShape::BandPass => c(g, k, 0.0, k, 0.0),
        EqShape::Bell => {
            let k = 1.0 / (q * a);
            c(g, k, 1.0, k * (a * a - 1.0), 0.0)
        }
        EqShape::LowShelf => c(g / a.sqrt(), k, 1.0, k * (a - 1.0), a * a - 1.0),
        EqShape::HighShelf => c(g * a.sqrt(), k, a * a, k * (1.0 - a) * a, 1.0 - a * a),
    }
}

/// The SVF `g`, `k` whose poles are those of `1 + a1·z⁻¹ + a2·z⁻²` (stable).
fn poles_to_svf(a1: f64, a2: f64) -> (f64, f64) {
    let lo = 1.0 + a1 + a2;
    let hi = 1.0 - a1 + a2;
    let g = (lo / hi).max(1e-30).sqrt();
    let k = 2.0 * (1.0 - a2) / (hi * g);
    (g, k)
}

/// Linear magnitude of one SVF stage at `at_hz`.
pub fn svf_magnitude(c: &SvfCoefs, at_hz: f64, sample_rate: f64) -> f64 {
    let f = at_hz.clamp(0.0, sample_rate * 0.4999);
    let w = (PI * f / sample_rate).tan() / c.g;
    // s = j·w: s² = -w², so the denominator is (1 - w²) + j·k·w.
    let (dr, di) = (1.0 - w * w, c.k * w);
    // Numerator m0·(s² + k·s + 1) + m1·s + m2 = (m0·(1 - w²) + m2) + j·w·(m0·k + m1).
    let (nr, ni) = (c.m0 * (1.0 - w * w) + c.m2, w * (c.m0 * c.k + c.m1));
    ((nr * nr + ni * ni) / (dr * dr + di * di)).sqrt()
}

/// Magnitude in dB of `shape` at `at_hz` (floored at -120 dB).
pub fn magnitude_db(
    shape: EqShape,
    freq: f64,
    gain_db: f64,
    q: f64,
    at_hz: f64,
    sample_rate: f64,
) -> f64 {
    let c = svf_coefs(shape, freq, gain_db, q, sample_rate);
    let mut m = svf_magnitude(&c, at_hz, sample_rate);
    if matches!(shape, EqShape::LowCut24 | EqShape::HighCut24) {
        m *= m;
    }
    (20.0 * m.max(1e-6).log10()).max(-120.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f64 = 48_000.0;

    #[test]
    fn landmark_values() {
        // Bell: +6 dB at the centre, flat far away.
        assert!((magnitude_db(EqShape::Bell, 1000.0, 6.0, 1.0, 1000.0, FS) - 6.0).abs() < 1e-9);
        assert!(magnitude_db(EqShape::Bell, 1000.0, 6.0, 1.0, 20.0, FS).abs() < 0.05);
        // Cuts: -3 dB (Q = 1/√2) at the cutoff, -12 dB/oct slope.
        let q = std::f64::consts::FRAC_1_SQRT_2;
        assert!((magnitude_db(EqShape::HighCut, 1000.0, 0.0, q, 1000.0, FS) + 3.0103).abs() < 1e-3);
        assert!(
            (magnitude_db(EqShape::LowCut24, 1000.0, 0.0, q, 1000.0, FS) + 6.0206).abs() < 1e-3
        );
        // Shelves reach their gain, half of it at the corner.
        assert!((magnitude_db(EqShape::LowShelf, 200.0, 12.0, q, 10.0, FS) - 12.0).abs() < 0.05);
        assert!((magnitude_db(EqShape::HighShelf, 5000.0, -6.0, q, 20000.0, FS) + 6.0).abs() < 0.2);
        // Notch and band-pass at the centre.
        assert!(magnitude_db(EqShape::Notch, 1000.0, 0.0, 2.0, 1000.0, FS) <= -119.0);
        assert!(magnitude_db(EqShape::BandPass, 1000.0, 0.0, 2.0, 1000.0, FS).abs() < 0.01);
    }

    /// Analog magnitude (dB) of the prototype.
    fn analog_db(shape: EqShape, freq: f64, gain: f64, q: f64, at: f64) -> f64 {
        let p = Prototype::new(shape, freq, gain, q);
        let mut db = 10.0 * p.mag2(at).max(1e-12).log10();
        if matches!(shape, EqShape::LowCut24 | EqShape::HighCut24) {
            db *= 2.0;
        }
        db
    }

    /// Largest |digital - analog| (dB) over 20 Hz..20 kHz (below Nyquist), where the analog
    /// response is above -60 dB.
    fn worst(shape: EqShape, freq: f64, gain: f64, q: f64, fs: f64) -> f64 {
        let mut f = 20.0;
        let mut e = 0.0f64;
        while f < 20_000f64.min(fs * 0.499) {
            let a = analog_db(shape, freq, gain, q, f);
            if a > -60.0 {
                e = e.max((magnitude_db(shape, freq, gain, q, f, fs) - a).abs());
            }
            f *= 1.02;
        }
        e
    }

    /// Decramped: near Nyquist every shape stays close to its analog response (the plain
    /// bilinear design was off by up to 19 dB in these cases), and low bands are unchanged.
    #[test]
    fn matches_analog_near_nyquist() {
        use EqShape::*;
        for fs in [44_100.0, 48_000.0] {
            for (shape, freq, gain, q, max) in [
                (Bell, 10_000.0, 12.0, 1.0, 0.8),
                (Bell, 16_000.0, 9.0, 2.0, 1.2),
                (Bell, 18_000.0, -12.0, 0.7, 1.7),
                (Bell, 12_000.0, -12.0, 0.3, 1.2),
                (Bell, 19_500.0, 15.0, 5.0, 2.3),
                (HighShelf, 14_000.0, -9.0, 0.707, 0.4),
                (HighShelf, 10_000.0, 6.0, 0.707, 0.2),
                (HighCut, 12_000.0, 0.0, 0.707, 1.0),
                (HighCut, 18_000.0, 0.0, 2.0, 1.2),
                (LowCut, 15_000.0, 0.0, 0.707, 0.8),
                (BandPass, 15_000.0, 0.0, 1.0, 0.9),
                (HighCut24, 12_000.0, 0.0, 0.707, 2.0),
            ] {
                let e = worst(shape, freq, gain, q, fs);
                assert!(e < max, "{shape:?} {freq} Hz at {fs}: {e} dB from analog");
            }
            // Low and mid bands: the analog response, peaks exact.
            for (shape, freq, gain, q) in [
                (Bell, 1000.0, 12.0, 4.0),
                (LowShelf, 200.0, 6.0, 0.707),
                (LowCut, 30.0, 0.0, 0.707),
            ] {
                let e = worst(shape, freq, gain, q, fs);
                assert!(e < 0.02, "{shape:?} {freq} Hz at {fs}: {e} dB");
            }
            assert!((magnitude_db(Bell, 3000.0, 12.0, 1.0, 3000.0, fs) - 12.0).abs() < 1e-6);
            // A notch still nulls its centre, even near Nyquist.
            for f in [1000.0, 15_000.0] {
                assert!(
                    magnitude_db(Notch, f, 0.0, 2.0, f, fs) <= -100.0,
                    "notch {f}"
                );
            }
        }
    }

    /// The SVF form is the matched biquad: a stable filter for every setting (positive `g`,
    /// `k`), and bells/shelves at 0 dB are exact pass-throughs.
    #[test]
    fn coefficients_are_stable_and_flat_is_bypass() {
        let shapes = [
            EqShape::LowCut,
            EqShape::LowShelf,
            EqShape::Bell,
            EqShape::Notch,
            EqShape::HighShelf,
            EqShape::HighCut,
            EqShape::BandPass,
        ];
        for shape in shapes {
            for freq in [1.0, 20.0, 1000.0, 15_000.0, 30_000.0] {
                for q in [0.01, 0.1, 0.707, 18.0] {
                    for gain in [-24.0, 0.0, 24.0] {
                        let c = svf_coefs(shape, freq, gain, q, FS);
                        let ok = [c.g, c.k, c.m0, c.m1, c.m2].iter().all(|v| v.is_finite());
                        assert!(
                            ok && c.g > 0.0 && c.k > 0.0,
                            "{shape:?} {freq} {q} {gain}: {c:?}"
                        );
                    }
                }
            }
        }
        for shape in [EqShape::Bell, EqShape::LowShelf, EqShape::HighShelf] {
            let c = svf_coefs(shape, 500.0, 0.0, 2.0, FS);
            assert_eq!((c.m0, c.m1, c.m2), (1.0, 0.0, 0.0), "{shape:?}");
        }
    }
}

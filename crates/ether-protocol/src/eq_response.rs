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
    let flat = gain_db == 0.0
        && matches!(
            shape,
            EqShape::Bell | EqShape::LowShelf | EqShape::HighShelf
        );
    let zeros = if shape == EqShape::Notch {
        Zeros::Notch(fc)
    } else {
        Zeros::Matched
    };
    matched(&proto, fs, zeros, flat)
}

/// How [`matched`] places a stage's zeros.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Zeros {
    /// Matching `|H|` at DC, Nyquist and the centre.
    Matched,
    /// On the unit circle at this frequency (Hz), unity at DC.
    Notch(f64),
    /// Both at DC (a true second-order high-pass: 12 dB/oct all the way down), matching
    /// `|H|` at Nyquist.
    HighPass,
}

/// The matched design of one analog stage (module docs): poles by impulse invariance, the
/// numerator by [`Zeros`]. `flat` = exact pass-through with the same poles.
fn matched(proto: &Prototype, fs: f64, zeros: Zeros, flat: bool) -> SvfCoefs {
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
    let [b0, b1, b2] = if let Zeros::Notch(fc) = zeros {
        // Zeros on the unit circle at the centre, unity at DC.
        let c = -2.0 * (2.0 * PI * fc / fs).cos();
        let scale = (1.0 + a1 + a2) / (2.0 + c);
        [scale, scale * c, scale]
    } else if zeros == Zeros::HighPass {
        // c·(1 - z⁻¹)², |c·4| = |H| at Nyquist.
        let c = (proto.mag2(fs * 0.5) * a[1]).max(0.0).sqrt() / 4.0;
        [c, -2.0 * c, c]
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

// ---------------------------------------------------------------------------------------
// Graphic EQ bands (`BuiltinDevice::GraphicEq`, `ether_devices::graphic_eq`)
// ---------------------------------------------------------------------------------------

/// Band types of the graphic EQ, in the order of its `Type` params' labels (append-only).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum GraphicEqShape {
    /// Peak / dip: gain at the frequency, width by Q.
    Bell,
    /// Lows raised / lowered by the gain; Q shapes the corner (resonant above 0.71).
    LowShelf,
    /// High-pass at the selected slope; Q is the resonance at the cutoff (0.71 = flat).
    LowCut,
    HighShelf,
    /// Low-pass at the selected slope.
    HighCut,
    /// A null at the frequency, width by Q.
    Notch,
    /// Keeps the band around the frequency (unity there); the rest falls off by the slope.
    BandPass,
    /// Lows at `-gain`, highs at `+gain`, pivoting around the frequency (Q shapes the
    /// transition).
    TiltShelf,
    /// A constant tilt over the audible range: `2·gain` dB between 20 Hz and 20 kHz (about
    /// `gain / 5` dB per octave), 0 dB at the frequency. Q is not used.
    FlatTilt,
}

impl GraphicEqShape {
    /// Every shape, in `Type` label order (plain value = index).
    pub const ALL: [GraphicEqShape; 9] = [
        Self::Bell,
        Self::LowShelf,
        Self::LowCut,
        Self::HighShelf,
        Self::HighCut,
        Self::Notch,
        Self::BandPass,
        Self::TiltShelf,
        Self::FlatTilt,
    ];

    /// The gain changes the response.
    pub fn uses_gain(self) -> bool {
        matches!(
            self,
            Self::Bell | Self::LowShelf | Self::HighShelf | Self::TiltShelf | Self::FlatTilt
        )
    }

    /// The Q changes the response (a 6 dB/oct cut ignores it too).
    pub fn uses_q(self) -> bool {
        !matches!(self, Self::FlatTilt)
    }

    /// The slope changes the response.
    pub fn uses_slope(self) -> bool {
        matches!(self, Self::LowCut | Self::HighCut | Self::BandPass)
    }
}

/// Slopes of the graphic EQ's cuts and band-pass (dB/oct), in `Slope` label order.
pub const GRAPHIC_EQ_SLOPES: [u32; 8] = [6, 12, 18, 24, 36, 48, 72, 96];

/// Most stages one graphic EQ band runs (a 96 dB/oct cut: eight second-order stages).
pub const GRAPHIC_EQ_MAX_STAGES: usize = 8;

/// The range a flat tilt spans with a constant slope (Hz).
pub const FLAT_TILT_RANGE: (f64, f64) = (20.0, 20_000.0);
/// Shelves of a flat tilt (log-spaced over [`FLAT_TILT_RANGE`]) and their Q.
const FLAT_TILT_STAGES: usize = 7;
const FLAT_TILT_Q: f64 = 0.55;

const SQRT_HALF: f64 = std::f64::consts::FRAC_1_SQRT_2;

/// The stages of one band: `stages[..count]` in series (the rest unused).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GraphicEqStages {
    pub stages: [SvfCoefs; GRAPHIC_EQ_MAX_STAGES],
    pub count: usize,
}

impl GraphicEqStages {
    const PASS: SvfCoefs = SvfCoefs {
        g: 0.1,
        k: 1.0,
        m0: 1.0,
        m1: 0.0,
        m2: 0.0,
    };

    fn new() -> Self {
        Self {
            stages: [Self::PASS; GRAPHIC_EQ_MAX_STAGES],
            count: 0,
        }
    }

    fn push(&mut self, c: SvfCoefs) {
        if self.count < GRAPHIC_EQ_MAX_STAGES {
            self.stages[self.count] = c;
            self.count += 1;
        }
    }

    /// The used stages.
    pub fn active(&self) -> &[SvfCoefs] {
        &self.stages[..self.count]
    }

    /// Scale the first stage so the band's magnitude at `at_hz` is 1.
    fn normalize_at(&mut self, at_hz: f64, fs: f64) {
        let m = graphic_eq_magnitude(self, at_hz, fs);
        if self.count > 0 && m > 0.0 && m.is_finite() {
            self.stages[0] = scaled(self.stages[0], 1.0 / m);
        }
    }
}

/// One analog stage (matched), its pole frequency clamped below Nyquist.
fn stage(proto: Prototype, fs: f64, zeros: Zeros) -> SvfCoefs {
    let proto = Prototype {
        fp: proto.fp.clamp(1.0, fs * 0.49),
        ..proto
    };
    matched(&proto, fs, zeros, false)
}

fn scaled(c: SvfCoefs, s: f64) -> SvfCoefs {
    SvfCoefs {
        m0: c.m0 * s,
        m1: c.m1 * s,
        m2: c.m2 * s,
        ..c
    }
}

/// Stages of an `order`-th order Butterworth cut at `fc` (a high-pass when `low`):
/// `order / 2` second-order sections, plus a first-order one for odd orders (written as an
/// SVF with a cancelling zero), the highest-Q section's Q scaled by `q / √½`.
fn cut(out: &mut GraphicEqStages, low: bool, fc: f64, q: f64, order: usize, fs: f64) {
    let order = order.clamp(1, 2 * GRAPHIC_EQ_MAX_STAGES);
    if order % 2 == 1 {
        // Poles at -1 (k = 2): s/(s+1) = (s² + s)/(s + 1)², 1/(s+1) = (s + 1)/(s + 1)².
        let (m0, m1, m2) = if low {
            (1.0, -1.0, -1.0)
        } else {
            (0.0, 1.0, 1.0)
        };
        out.push(stage(
            Prototype {
                fp: fc,
                k: 2.0,
                m0,
                m1,
                m2,
            },
            fs,
            Zeros::Matched,
        ));
    }
    let pairs = order / 2;
    let n = order as f64;
    for i in 0..pairs {
        // Butterworth: Q rises with i, the last section is the resonant one.
        let bq = 1.0 / (2.0 * (PI * (2 * i + 1 + order % 2) as f64 / (2.0 * n)).cos());
        let sq = if i + 1 == pairs {
            bq * q / SQRT_HALF
        } else {
            bq
        };
        let shape = if low {
            EqShape::LowCut
        } else {
            EqShape::HighCut
        };
        let zeros = if low { Zeros::HighPass } else { Zeros::Matched };
        out.push(stage(Prototype::new(shape, fc, 0.0, sq), fs, zeros));
    }
}

/// The stages of one graphic EQ band: `shape` at `freq` Hz (clamped to `1..=0.49·fs`),
/// `gain_db`, `q` (`>= 0.01`) and `slope` (index in [`GRAPHIC_EQ_SLOPES`]) at
/// `sample_rate`. Every stage is the matched design of the module docs, so the device and
/// the UI's curve agree. A bell, shelf or tilt at 0 dB has no stage (exact pass-through).
///
/// Cuts are Butterworth of order `slope / 6` (see [`GraphicEqShape::LowCut`]). A band-pass
/// at 6 or 12 dB/oct is the plain second-order band-pass (the slope counts both skirts, as
/// filter orders do); steeper, it is a Butterworth low cut and high cut of order `slope / 12`
/// each, ±`1/(2q)` octave around the frequency, scaled to unity there.
pub fn graphic_eq_stages(
    shape: GraphicEqShape,
    freq: f64,
    gain_db: f64,
    q: f64,
    slope: usize,
    sample_rate: f64,
) -> GraphicEqStages {
    use GraphicEqShape as S;
    let fs = sample_rate;
    let fc = freq.clamp(1.0, fs * 0.49);
    let q = q.max(0.01);
    let slope_db = GRAPHIC_EQ_SLOPES[slope.min(GRAPHIC_EQ_SLOPES.len() - 1)] as usize;
    let mut out = GraphicEqStages::new();
    match shape {
        S::Bell | S::LowShelf | S::HighShelf if gain_db != 0.0 => {
            let eq = match shape {
                S::Bell => EqShape::Bell,
                S::LowShelf => EqShape::LowShelf,
                _ => EqShape::HighShelf,
            };
            out.push(stage(
                Prototype::new(eq, fc, gain_db, q),
                fs,
                Zeros::Matched,
            ));
        }
        S::Notch => out.push(stage(
            Prototype::new(EqShape::Notch, fc, 0.0, q),
            fs,
            Zeros::Notch(fc),
        )),
        S::LowCut | S::HighCut => cut(&mut out, shape == S::LowCut, fc, q, slope_db / 6, fs),
        S::BandPass if slope_db <= 12 => out.push(stage(
            Prototype::new(EqShape::BandPass, fc, 0.0, q),
            fs,
            Zeros::Matched,
        )),
        S::BandPass => {
            let spread = 2f64.powf(0.5 / q);
            let side = slope_db / 12;
            cut(&mut out, true, fc / spread, SQRT_HALF, side, fs);
            cut(&mut out, false, fc * spread, SQRT_HALF, side, fs);
            out.normalize_at(fc, fs);
        }
        S::TiltShelf if gain_db != 0.0 => {
            // A high shelf of 2·gain, lowered by gain: -gain below, +gain above.
            let c = stage(
                Prototype::new(EqShape::HighShelf, fc, 2.0 * gain_db, q),
                fs,
                Zeros::Matched,
            );
            out.push(scaled(c, 10f64.powf(-gain_db / 20.0)));
        }
        S::FlatTilt if gain_db != 0.0 => {
            let (lo, hi) = FLAT_TILT_RANGE;
            let step = 2.0 * gain_db / FLAT_TILT_STAGES as f64;
            for i in 0..FLAT_TILT_STAGES {
                let f = lo * (hi / lo).powf((i as f64 + 0.5) / FLAT_TILT_STAGES as f64);
                out.push(stage(
                    Prototype::new(EqShape::HighShelf, f, step, FLAT_TILT_Q),
                    fs,
                    Zeros::Matched,
                ));
            }
            out.normalize_at(fc, fs);
        }
        // Bells, shelves and tilts at 0 dB.
        _ => {}
    }
    out
}

/// Linear magnitude of a band's stages at `at_hz`.
pub fn graphic_eq_magnitude(stages: &GraphicEqStages, at_hz: f64, sample_rate: f64) -> f64 {
    stages
        .active()
        .iter()
        .map(|c| svf_magnitude(c, at_hz, sample_rate))
        .product()
}

/// Magnitude in dB of one graphic EQ band at `at_hz` (floored at -120 dB).
pub fn graphic_eq_db(
    shape: GraphicEqShape,
    freq: f64,
    gain_db: f64,
    q: f64,
    slope: usize,
    at_hz: f64,
    sample_rate: f64,
) -> f64 {
    let st = graphic_eq_stages(shape, freq, gain_db, q, slope, sample_rate);
    let m = graphic_eq_magnitude(&st, at_hz, sample_rate);
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

#[cfg(test)]
mod graphic_tests {
    use super::GraphicEqShape as S;
    use super::*;

    const FS: f64 = 48_000.0;
    const BUTTERWORTH_Q: f64 = std::f64::consts::FRAC_1_SQRT_2;

    fn db(shape: S, freq: f64, gain: f64, q: f64, slope: usize, at: f64) -> f64 {
        graphic_eq_db(shape, freq, gain, q, slope, at, FS)
    }

    #[test]
    fn bells_and_shelves_reach_their_gain() {
        assert!((db(S::Bell, 1000.0, 6.0, 1.0, 1, 1000.0) - 6.0).abs() < 1e-9);
        assert!((db(S::Bell, 250.0, -9.0, 4.0, 1, 250.0) + 9.0).abs() < 1e-9);
        assert!(db(S::Bell, 1000.0, 6.0, 1.0, 1, 20.0).abs() < 0.05);
        assert!((db(S::LowShelf, 200.0, 6.0, BUTTERWORTH_Q, 1, 10.0) - 6.0).abs() < 0.05);
        assert!(db(S::LowShelf, 200.0, 6.0, BUTTERWORTH_Q, 1, 15_000.0).abs() < 0.05);
        assert!((db(S::HighShelf, 2000.0, -6.0, BUTTERWORTH_Q, 1, 20_000.0) + 6.0).abs() < 0.1);
    }

    #[test]
    fn zero_gain_shapes_have_no_stage() {
        for shape in [
            S::Bell,
            S::LowShelf,
            S::HighShelf,
            S::TiltShelf,
            S::FlatTilt,
        ] {
            assert_eq!(
                graphic_eq_stages(shape, 1000.0, 0.0, 1.0, 1, FS).count,
                0,
                "{shape:?}"
            );
        }
    }

    /// Every slope: -3 dB at the cutoff (Butterworth at Q 0.71) and the asymptotic slope.
    #[test]
    fn cuts_are_butterworth_at_every_slope() {
        for (i, &slope) in GRAPHIC_EQ_SLOPES.iter().enumerate() {
            let order = slope as usize / 6;
            let st = graphic_eq_stages(S::LowCut, 2000.0, 0.0, BUTTERWORTH_Q, i, FS);
            assert_eq!(st.count, order / 2 + order % 2, "{slope}");
            let at = db(S::LowCut, 2000.0, 0.0, BUTTERWORTH_Q, i, 2000.0);
            assert!(
                (at + 3.0103).abs() < 0.02,
                "low cut {slope}: {at} dB at the cutoff"
            );
            let at = db(S::HighCut, 1000.0, 0.0, BUTTERWORTH_Q, i, 1000.0);
            assert!(
                (at + 3.0103).abs() < 0.02,
                "high cut {slope}: {at} dB at the cutoff"
            );
            // Four octaves below the cutoff to five: one octave of the asymptote.
            let (a, b) = (
                db(S::LowCut, 4000.0, 0.0, BUTTERWORTH_Q, i, 250.0),
                db(S::LowCut, 4000.0, 0.0, BUTTERWORTH_Q, i, 125.0),
            );
            if b > -119.0 {
                assert!(((a - b) - slope as f64).abs() < 0.6, "{slope}: {}", a - b);
            }
            // Far in the passband: flat.
            assert!(db(S::LowCut, 100.0, 0.0, BUTTERWORTH_Q, i, 8000.0).abs() < 0.02);
        }
        // Resonance: Q above 0.71 peaks near the cutoff.
        assert!(db(S::HighCut, 1000.0, 0.0, 4.0, 1, 1000.0) > 10.0);
    }

    #[test]
    fn notch_band_pass_and_tilts() {
        assert!(db(S::Notch, 1000.0, 0.0, 2.0, 1, 1000.0) <= -100.0);
        assert!(db(S::Notch, 1000.0, 0.0, 2.0, 1, 100.0).abs() < 0.1);
        for slope in 0..GRAPHIC_EQ_SLOPES.len() {
            let at = db(S::BandPass, 1000.0, 0.0, 1.0, slope, 1000.0);
            assert!(at.abs() < 1e-6, "band-pass {slope}: {at}");
            assert!(db(S::BandPass, 1000.0, 0.0, 1.0, slope, 100.0) < -12.0);
        }
        // Tilt shelf: -gain below, +gain above, 0 dB at the pivot.
        assert!((db(S::TiltShelf, 1000.0, 6.0, BUTTERWORTH_Q, 1, 20.0) + 6.0).abs() < 0.05);
        assert!((db(S::TiltShelf, 1000.0, 6.0, BUTTERWORTH_Q, 1, 18_000.0) - 6.0).abs() < 0.1);
        assert!(db(S::TiltShelf, 1000.0, 6.0, BUTTERWORTH_Q, 1, 1000.0).abs() < 0.05);
        // Flat tilt: 0 dB at the pivot, a constant slope of 2·gain over 20 Hz..20 kHz.
        let per_oct = 2.0 * 6.0 / 1000f64.log2();
        assert!(db(S::FlatTilt, 500.0, 6.0, 1.0, 1, 500.0).abs() < 1e-9);
        let mut f = 40.0;
        while f < 10_000.0 {
            let d = db(S::FlatTilt, 500.0, 6.0, 1.0, 1, 2.0 * f)
                - db(S::FlatTilt, 500.0, 6.0, 1.0, 1, f);
            assert!(
                (d - per_oct).abs() < 0.25,
                "flat tilt at {f} Hz: {d} dB/oct"
            );
            f *= 1.5;
        }
    }

    #[test]
    fn every_setting_is_stable() {
        for shape in GraphicEqShape::ALL {
            for slope in 0..GRAPHIC_EQ_SLOPES.len() {
                for freq in [1.0, 10.0, 1000.0, 20_000.0, 30_000.0] {
                    for q in [0.025, 0.71, 40.0] {
                        for gain in [-30.0, 0.0, 30.0] {
                            for fs in [44_100.0, 96_000.0] {
                                let st = graphic_eq_stages(shape, freq, gain, q, slope, fs);
                                for c in st.active() {
                                    let ok =
                                        [c.g, c.k, c.m0, c.m1, c.m2].iter().all(|v| v.is_finite());
                                    assert!(
                                        ok && c.g > 0.0 && c.k > 0.0,
                                        "{shape:?} {freq} {q} {gain} {slope}: {c:?}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

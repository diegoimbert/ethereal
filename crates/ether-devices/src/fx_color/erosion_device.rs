//! Erosion: digital degradation made by modulating a very short delay with a sine or with
//! band-limited noise (a fizz, hiss or aliasing-like grit that follows the input).
//!
//! Reading the input `d(t)` samples late, with `d` wobbling fast around a centre `C`, is a
//! phase modulation of everything in the signal: `y(t) ≈ x(t - C) - δ(t)·x'(t - C)` for a
//! small excursion `δ`, so the artefacts scale with the input's level and its brightness
//! (silence stays silent, highs erode first) and sit around the modulator's frequency.
//!
//! - **Mode**: `Sine` modulates with a sine at `Frequency` (sidebands at ±`Frequency`
//!   around every partial: metallic, ring-mod / downsampling-like); `Noise` with white noise
//!   through a band-pass at `Frequency` whose bandwidth is `Width` (octaves), the same noise
//!   on both channels; `Wide Noise` with independent noise per channel (a wide, stereo
//!   fizz). `Width` has no effect on the sine.
//! - **Amount** sets the excursion: up to ±[`MAX_EXCURSION_MS`] at 100 %. The noise is
//!   normalized to the same RMS whatever its band (exactly, see [`noise_gain`]), so `Frequency` and `Width` change the
//!   colour, not the strength.
//! - The read is a 4-point Hermite interpolation of the line, so the only grit is the
//!   modulation's (no interpolation dulling). The centre delay `C` is the reported latency
//!   and the dry path of `Mix` is read at `C`, so `Mix` blends aligned signals.

use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus, Smoother,
};

use super::erosion as ids;
use super::shared::{Rng, Values, db, glide};
use crate::dsp::svf::{Coefs, Svf};
use crate::util::{LateEvents, Route};

/// Largest delay excursion at `Amount` 100 % (ms).
pub const MAX_EXCURSION_MS: f32 = 0.5;
/// Ring length (power of two): the centre delay and its excursion stay inside it up to
/// 384 kHz.
const RING: usize = 512;
/// RMS of the normalized noise modulator (peaks are clipped to ±1).
const NOISE_RMS: f64 = 0.45;
/// Time constant of the band-pass glide on a `Frequency` / `Width` change.
const GLIDE_MS: f64 = 10.0;

/// Modulator (param `Mode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Sine,
    Noise,
    WideNoise,
}

impl Mode {
    fn from_index(i: usize) -> Self {
        match i {
            0 => Self::Sine,
            1 => Self::Noise,
            _ => Self::WideNoise,
        }
    }
}

/// `Q` of a band-pass `octaves` wide (between its -3 dB points).
pub fn width_to_q(octaves: f64) -> f64 {
    let r = octaves.max(0.01).exp2();
    r.sqrt() / (r - 1.0)
}

/// Gain that brings uniform white noise in `[-1, 1)` through the band-pass `c` to an RMS of
/// [`NOISE_RMS`]. Exact for the digital filter (warping near Nyquist included): the
/// filter's output variance for unit white noise is solved from its state-space form (a
/// 2x2 discrete Lyapunov equation, three unknowns). Bounded work, RT-safe.
fn noise_gain(c: &Coefs) -> f64 {
    let (g, k) = (c.g, c.k);
    let a1 = 1.0 / (1.0 + g * (g + k));
    let a2 = g * a1;
    let a3 = g * a2;
    // `Svf::tick` as `s' = M·s + N·v0`, band output `y = C·s + D·v0` (times `m1 = k`).
    let (m11, m12, m21, m22) = (2.0 * a1 - 1.0, -2.0 * a2, 2.0 * a2, 1.0 - 2.0 * a3);
    let (n1, n2) = (2.0 * a2, 2.0 * a3);
    let (c1, c2, d) = (k * a1, -k * a2, k * a2);
    // P = M·P·Mᵀ + N·Nᵀ for P = [[p11, p12], [p12, p22]]: (I - T)·p = n.
    let t = [
        [m11 * m11, 2.0 * m11 * m12, m12 * m12],
        [m11 * m21, m11 * m22 + m12 * m21, m12 * m22],
        [m21 * m21, 2.0 * m21 * m22, m22 * m22],
    ];
    let a = [
        [1.0 - t[0][0], -t[0][1], -t[0][2]],
        [-t[1][0], 1.0 - t[1][1], -t[1][2]],
        [-t[2][0], -t[2][1], 1.0 - t[2][2]],
    ];
    let n = [n1 * n1, n1 * n2, n2 * n2];
    let det3 = |m: [[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let det = det3(a);
    let col = |j: usize| {
        let mut m = a;
        for (row, v) in m.iter_mut().zip(n) {
            row[j] = v;
        }
        det3(m) / det
    };
    let (p11, p12, p22) = (col(0), col(1), col(2));
    let var = c1 * c1 * p11 + 2.0 * c1 * c2 * p12 + c2 * c2 * p22 + d * d;
    // Uniform noise in [-1, 1) has variance 1/3.
    let rms = (var / 3.0).sqrt();
    if rms.is_finite() && rms > 1e-9 {
        NOISE_RMS / rms
    } else {
        1.0
    }
}

/// Unit-peak band-pass (`k · band`).
fn band_pass(freq: f64, q: f64, sample_rate: f64) -> Coefs {
    let f = freq.min(0.45 * sample_rate);
    let g = (std::f64::consts::PI * f / sample_rate).tan();
    let k = 1.0 / q;
    Coefs::from_parts(g, k, 0.0, k, 0.0)
}

/// One channel's delay ring.
#[derive(Clone, Copy)]
struct Line {
    buf: [f32; RING],
    at: usize,
}

impl Line {
    const fn new() -> Self {
        Self {
            buf: [0.0; RING],
            at: 0,
        }
    }

    #[inline]
    fn push(&mut self, x: f32) {
        self.at = (self.at + 1) & (RING - 1);
        self.buf[self.at] = super::shared::flush(x);
    }

    /// Sample written `n` pushes ago (0 = the last one).
    #[inline]
    fn tap(&self, n: usize) -> f32 {
        self.buf[(self.at.wrapping_sub(n)) & (RING - 1)]
    }

    /// 4-point, 3rd-order Hermite read `d` samples back (`1 <= d <= RING - 3`).
    #[inline]
    fn read(&self, d: f32) -> f32 {
        let i = d as usize;
        let t = d - i as f32;
        let (xm1, x0, x1, x2) = (
            self.tap(i - 1),
            self.tap(i),
            self.tap(i + 1),
            self.tap(i + 2),
        );
        let c1 = 0.5 * (x1 - xm1);
        let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
        let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
        ((c3 * t + c2) * t + c1) * t + x0
    }
}

/// The erosion device.
pub struct Erosion {
    values: Values<{ ids::COUNT }>,
    sample_rate: f32,
    mode: Mode,
    /// Centre delay (samples, whole: the latency) and the excursion at 100 %.
    center: usize,
    max_excursion: f32,
    /// Sine phase (cycles) and increment.
    phase: f64,
    inc: f64,
    /// Band-pass: current and target coefficients, per-channel states, noise sources.
    coefs: Coefs,
    target: Coefs,
    gliding: bool,
    glide_coef: f64,
    svf: [Svf; 2],
    rng: [Rng; 2],
    noise_gain: Smoother,
    excursion: Smoother,
    output: Smoother,
    mix: Smoother,
    lines: [Line; 2],
    /// Silence reporting.
    tail: crate::util::Tail,
    /// Every param acts on the read side, `center` samples into the line: its events wait
    /// that long.
    late: LateEvents,
}

impl Default for Erosion {
    fn default() -> Self {
        Self::new()
    }
}

impl Erosion {
    /// Non-RT. An erosion with default params.
    pub fn new() -> Self {
        let desc = descriptor();
        let bp = band_pass(1000.0, 1.0, 48_000.0);
        let mut s = Self {
            values: Values::new(&desc),
            sample_rate: 48_000.0,
            mode: Mode::Noise,
            center: 1,
            max_excursion: 0.0,
            phase: 0.0,
            inc: 0.0,
            coefs: bp,
            target: bp,
            gliding: false,
            glide_coef: 0.0,
            svf: [Svf::ZERO; 2],
            rng: [Rng::new(0xE205_1011), Rng::new(0x51DE_7A1C)],
            noise_gain: Smoother::new(1.0, 0.0, 48_000.0),
            excursion: Smoother::new(0.0, 0.0, 48_000.0),
            output: Smoother::new(1.0, 0.0, 48_000.0),
            mix: Smoother::new(1.0, 0.0, 48_000.0),
            lines: [Line::new(); 2],
            tail: crate::util::Tail::default(),
            late: LateEvents::new(),
        };
        s.sync_all();
        s
    }

    fn sync_all(&mut self) {
        let sr = self.sample_rate;
        self.max_excursion = (MAX_EXCURSION_MS * 0.001 * sr).min(((RING - 4) / 2 - 2) as f32);
        self.center = self.max_excursion.ceil() as usize + 2;
        self.glide_coef = (-1.0 / (GLIDE_MS * 0.001 * f64::from(sr))).exp();
        for i in 0..ids::COUNT {
            self.apply(ParamId(i as u32), false);
        }
    }

    fn band_changed(&mut self, smooth: bool) {
        let sr = f64::from(self.sample_rate);
        let f = f64::from(self.values.f(ids::FREQUENCY));
        let q = width_to_q(f64::from(self.values.f(ids::WIDTH)));
        self.inc = f / sr;
        self.target = band_pass(f, q, sr);
        let g = noise_gain(&self.target) as f32;
        glide(&mut self.noise_gain, g, smooth);
        if smooth {
            self.gliding = true;
        } else {
            self.coefs = self.target;
            self.gliding = false;
        }
    }

    fn apply(&mut self, id: ParamId, smooth: bool) {
        match id {
            ids::MODE => self.mode = Mode::from_index(self.values.index(ids::MODE)),
            ids::FREQUENCY | ids::WIDTH => self.band_changed(smooth),
            ids::AMOUNT => {
                let v = self.values.f(ids::AMOUNT) * 0.01 * self.max_excursion;
                glide(&mut self.excursion, v, smooth);
            }
            ids::OUTPUT => {
                let v = db(self.values.f(ids::OUTPUT));
                glide(&mut self.output, v, smooth);
            }
            ids::MIX => {
                let v = self.values.f(ids::MIX) * 0.01;
                glide(&mut self.mix, v, smooth);
            }
            _ => {}
        }
    }

    fn set(&mut self, id: ParamId, value: f64, smooth: bool) {
        if self.values.set(id, value) {
            self.apply(id, smooth);
        }
    }

    /// One noise sample of channel `c`'s source, band-passed and normalized, `-1..=1`.
    #[inline]
    fn noise(&mut self, c: usize, gain: f64) -> f32 {
        let w = f64::from(self.rng[c].next()) * 2.0 - 1.0;
        (self.svf[c].tick(w, &self.coefs) * gain).clamp(-1.0, 1.0) as f32
    }

    fn render(&mut self, audio: &mut AudioBuffers<'_, '_>, start: usize, end: usize) {
        let n_ch = audio.outputs.len().min(2);
        let center = self.center as f32;
        for i in start..end {
            if self.gliding {
                self.gliding = self.coefs.glide(&self.target, self.glide_coef);
            }
            let gain = f64::from(self.noise_gain.tick());
            let m = match self.mode {
                Mode::Sine => {
                    self.phase += self.inc;
                    self.phase -= self.phase.floor();
                    let s = (std::f64::consts::TAU * self.phase).sin() as f32;
                    [s, s]
                }
                Mode::Noise => {
                    let s = self.noise(0, gain);
                    [s, s]
                }
                Mode::WideNoise => [self.noise(0, gain), self.noise(1, gain)],
            };
            let exc = self.excursion.tick();
            let out_gain = self.output.tick();
            let mix = self.mix.tick();
            for (c, (line, m)) in self.lines.iter_mut().zip(m).enumerate().take(n_ch) {
                let x = audio
                    .inputs
                    .get(c)
                    .or(audio.inputs.first())
                    .map_or(0.0, |b| b[i]);
                line.push(x);
                let dry = line.tap(self.center);
                let wet = line.read(center + exc * m);
                audio.outputs[c][i] = (dry + (wet - dry) * mix) * out_gain;
            }
        }
    }
}

fn descriptor() -> DeviceDescriptor {
    super::descriptor(BuiltinDeviceType::Erosion)
}

impl Node for Erosion {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.sync_all();
        self.late.prepare(self.center, config);
        self.reset();
    }

    fn reset(&mut self) {
        LateEvents::flush(
            self,
            |s| &mut s.late,
            |s, kind| {
                if let EventKind::Param { param, value } = *kind {
                    s.set(param, value, false);
                }
            },
        );
        self.lines = [Line::new(); 2];
        self.svf = [Svf::ZERO; 2];
        self.phase = 0.0;
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let center = self.center;
        LateEvents::split(
            self,
            |s| &mut s.late,
            ctx,
            |_| Route::Late(center),
            |s, a, b| s.render(audio, a, b),
            |s, kind| {
                if let EventKind::Param { param, value } = *kind {
                    s.set(param, value, true);
                }
            },
        );
        // The modulator only moves the read: silence in, silence out once the line has
        // emptied.
        let settled = ![&self.excursion, &self.output, &self.mix]
            .iter()
            .any(|s| s.is_smoothing())
            && self.late.is_idle();
        let hold = 2 * self.center
            + crate::util::samples(crate::util::BASE_HOLD_SECONDS, self.sample_rate);
        self.tail.status_of(ctx, audio, settled, hold)
    }

    /// The centre delay: the dry and wet paths both sit `center` samples late.
    fn latency(&self) -> u32 {
        self.center as u32
    }
}

impl Device for Erosion {
    fn descriptor(&self) -> DeviceDescriptor {
        descriptor()
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.late.readback(id, self.values.get(id))
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.late.cancel(id);
        self.set(id, value, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hermite_read_is_exact_on_whole_delays_and_lines() {
        let mut l = Line::new();
        for k in 0..20 {
            l.push(k as f32);
        }
        // A ramp: Hermite reproduces it exactly, also between samples.
        assert_eq!(l.read(3.0), 16.0);
        assert!((l.read(3.25) - 15.75).abs() < 1e-5);
    }

    #[test]
    fn width_maps_to_band_q() {
        // One octave: Q = √2.
        assert!((width_to_q(1.0) - std::f64::consts::SQRT_2).abs() < 1e-12);
        assert!(width_to_q(0.1) > width_to_q(2.5));
    }

    #[test]
    fn normalized_noise_has_the_target_rms() {
        for (f, w) in [
            (300.0, 0.1),
            (3000.0, 0.3),
            (12_000.0, 2.5),
            (18_000.0, 1.0),
        ] {
            let sr = 48_000.0;
            let q = width_to_q(w);
            let c = band_pass(f, q, sr);
            let g = noise_gain(&c);
            let mut svf = Svf::ZERO;
            let mut rng = Rng::new(7);
            let n = 200_000;
            let mut acc = 0.0;
            for _ in 0..n {
                let y = svf.tick(f64::from(rng.next()) * 2.0 - 1.0, &c) * g;
                acc += y * y;
            }
            let rms = (acc / f64::from(n)).sqrt();
            assert!(
                (rms / NOISE_RMS - 1.0).abs() < 0.03,
                "{f} Hz, {w} oct: {rms}"
            );
        }
    }
}

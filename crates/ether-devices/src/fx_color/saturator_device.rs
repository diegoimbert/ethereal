//! Saturator: a static waveshaper with six curves, oversampled 1x/2x/4x.
//!
//! Signal path per channel: `u = g·(x + b)` (drive `g`, bias `b`) → curve → minus the
//! curve's value at `g·b` (the static offset the bias adds) → DC blocker (10 Hz, removes
//! what is left, e.g. rectification) → tilt `Tone` → auto gain → `Mix` with the aligned dry
//! signal → `Output`.
//!
//! **Oversampling** runs the curve at 2x or 4x through linear-phase half-band filters
//! ([`super::oversample`]). The device always reports the 4x latency
//! ([`LATENCY`] samples) and pads the Off/2x paths to it, so switching modes never moves the
//! track in time (no PDC republish) and the dry path of `Mix` stays aligned. A mode change
//! crossfades the old and new paths over 5 ms.
//!
//! **Auto Gain** keeps a -12 dBFS sine at the same RMS: the compensation is computed from the
//! curve (32-point probe) whenever drive, bias or curve change, then ramped.

use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus, Smoother,
};

use super::oversample::{self, LATENCY_2X, LATENCY_4X, Stage1, Stage2};
use super::saturator as ids;
use super::shared::{FixedDelay, Values, db, flush, glide};
use crate::util::split_at_events;

/// Reported latency (samples): the 4x round trip, used by every mode.
pub const LATENCY: usize = LATENCY_4X;
const PAD_2X: usize = LATENCY_4X - LATENCY_2X;
/// Mode crossfade length.
const FADE_MS: f32 = 5.0;
/// DC blocker corner.
const DC_HZ: f32 = 10.0;
/// Tilt pivot and range (±6 dB at the extremes).
const TILT_HZ: f32 = 800.0;
const TILT_DB: f32 = 6.0;
/// Level of the auto-gain probe sine.
const PROBE_AMP: f32 = 0.25;
/// Negative-side knee of the tube curve (asymmetric: even harmonics).
const TUBE_K: f32 = 1.6;

/// Waveshaper curve (param `Curve`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    Soft,
    Hard,
    Tube,
    Tape,
    Fold,
    Rectify,
}

impl Curve {
    const ALL: [Curve; 6] = [
        Self::Soft,
        Self::Hard,
        Self::Tube,
        Self::Tape,
        Self::Fold,
        Self::Rectify,
    ];

    fn from_index(i: usize) -> Self {
        Self::ALL[i.min(5)]
    }

    /// The transfer function (unit slope at 0 except `Rectify`, which is `|tanh|`).
    #[inline]
    pub fn apply(self, u: f32) -> f32 {
        match self {
            Self::Soft => u.tanh(),
            Self::Hard => u.clamp(-1.0, 1.0),
            Self::Tube => {
                if u >= 0.0 {
                    1.0 - (-u).exp()
                } else {
                    ((TUBE_K * u).exp() - 1.0) / TUBE_K
                }
            }
            Self::Tape => std::f32::consts::FRAC_2_PI * (std::f32::consts::FRAC_PI_2 * u).atan(),
            Self::Fold => u.sin(),
            Self::Rectify => u.tanh().abs(),
        }
    }
}

/// Oversampling mode (param `Oversampling`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Off,
    X2,
    X4,
}

impl Mode {
    fn from_index(i: usize) -> Self {
        match i {
            0 => Self::Off,
            1 => Self::X2,
            _ => Self::X4,
        }
    }
}

/// Current shaping settings, held for one base-rate sample.
#[derive(Clone, Copy)]
struct Shape {
    curve: Curve,
    drive: f32,
    bias: f32,
    /// `curve(drive · bias)`: the static offset removed after shaping.
    offset: f32,
}

impl Shape {
    #[inline]
    fn apply(&self, x: f32) -> f32 {
        self.curve.apply(self.drive * (x + self.bias)) - self.offset
    }
}

#[derive(Clone, Copy)]
struct Channel {
    x2: Stage1,
    x4a: Stage1,
    x4b: Stage2,
    /// One 2x-rate sample of padding on the 4x path (makes its latency whole).
    x4_pad: f32,
    off_delay: FixedDelay<LATENCY>,
    x2_delay: FixedDelay<PAD_2X>,
    dry_delay: FixedDelay<LATENCY>,
    dc_x1: f32,
    dc_y1: f32,
    tilt_lp: f32,
}

impl Channel {
    fn new() -> Self {
        Self {
            x2: oversample::stage1(),
            x4a: oversample::stage1(),
            x4b: oversample::stage2(),
            x4_pad: 0.0,
            off_delay: FixedDelay::new(),
            x2_delay: FixedDelay::new(),
            dry_delay: FixedDelay::new(),
            dc_x1: 0.0,
            dc_y1: 0.0,
            tilt_lp: 0.0,
        }
    }

    fn reset_path(&mut self, mode: Mode) {
        match mode {
            Mode::Off => self.off_delay.reset(),
            Mode::X2 => {
                self.x2.reset();
                self.x2_delay.reset();
            }
            Mode::X4 => {
                self.x4a.reset();
                self.x4b.reset();
                self.x4_pad = 0.0;
            }
        }
    }

    fn reset(&mut self) {
        for m in [Mode::Off, Mode::X2, Mode::X4] {
            self.reset_path(m);
        }
        self.dry_delay.reset();
        self.dc_x1 = 0.0;
        self.dc_y1 = 0.0;
        self.tilt_lp = 0.0;
    }

    /// One base-rate sample through the shaper in `mode` (output delayed by [`LATENCY`]).
    #[inline]
    fn path(&mut self, mode: Mode, x: f32, s: &Shape) -> f32 {
        match mode {
            Mode::Off => self.off_delay.tick(s.apply(x)),
            Mode::X2 => {
                let [a, b] = self.x2.up(x);
                let y = self.x2.down([s.apply(a), s.apply(b)]);
                self.x2_delay.tick(y)
            }
            Mode::X4 => {
                let [a, b] = self.x4a.up(x);
                let [a0, a1] = self.x4b.up(a);
                let [b0, b1] = self.x4b.up(b);
                let a = self.x4b.down([s.apply(a0), s.apply(a1)]);
                let b = self.x4b.down([s.apply(b0), s.apply(b1)]);
                let first = self.x4_pad;
                self.x4_pad = b;
                self.x4a.down([first, a])
            }
        }
    }
}

/// The saturator device.
pub struct Saturator {
    values: Values<{ ids::COUNT }>,
    sample_rate: f32,
    curve: Curve,
    mode: Mode,
    /// Mode being faded out and the samples left in the fade.
    fade_from: Mode,
    fade_left: u32,
    fade_len: u32,
    drive: Smoother,
    bias: Smoother,
    auto_gain: Smoother,
    tilt: Smoother,
    output: Smoother,
    mix: Smoother,
    dc_r: f32,
    tilt_a: f32,
    ch: [Channel; 2],
}

impl Default for Saturator {
    fn default() -> Self {
        Self::new()
    }
}

impl Saturator {
    /// Non-RT. A saturator with default params.
    pub fn new() -> Self {
        let desc = descriptor();
        let values = Values::new(&desc);
        let mut s = Self {
            values,
            sample_rate: 48_000.0,
            curve: Curve::Soft,
            mode: Mode::X2,
            fade_from: Mode::X2,
            fade_left: 0,
            fade_len: 1,
            drive: Smoother::new(1.0, 0.0, 48_000.0),
            bias: Smoother::new(0.0, 0.0, 48_000.0),
            auto_gain: Smoother::new(1.0, 0.0, 48_000.0),
            tilt: Smoother::new(1.0, 0.0, 48_000.0),
            output: Smoother::new(1.0, 0.0, 48_000.0),
            mix: Smoother::new(1.0, 0.0, 48_000.0),
            dc_r: 0.0,
            tilt_a: 0.0,
            ch: [Channel::new(); 2],
        };
        s.sync_all();
        s
    }

    fn sync_all(&mut self) {
        self.dc_r = 1.0 - 2.0 * std::f32::consts::PI * DC_HZ / self.sample_rate;
        self.tilt_a = 1.0 - (-2.0 * std::f32::consts::PI * TILT_HZ / self.sample_rate).exp();
        self.fade_len = ((FADE_MS * 0.001 * self.sample_rate) as u32).max(1);
        for i in 0..ids::COUNT {
            let id = ParamId(i as u32);
            self.apply(id, false);
        }
        self.fade_left = 0;
    }

    fn drive_gain(&self) -> f32 {
        db(self.values.f(ids::DRIVE))
    }

    fn bias_value(&self) -> f32 {
        self.values.f(ids::BIAS) * 0.01
    }

    fn auto_gain_target(&self) -> f32 {
        if self.values.on(ids::AUTO_GAIN) {
            auto_gain(self.curve, self.drive_gain(), self.bias_value())
        } else {
            1.0
        }
    }

    /// React to a changed param (`values` already updated).
    fn apply(&mut self, id: ParamId, smooth: bool) {
        match id {
            ids::CURVE => {
                self.curve = Curve::from_index(self.values.index(ids::CURVE));
                let g = self.auto_gain_target();
                glide(&mut self.auto_gain, g, smooth);
            }
            ids::DRIVE | ids::BIAS | ids::AUTO_GAIN => {
                let (d, b) = (self.drive_gain(), self.bias_value());
                glide(&mut self.drive, d, smooth);
                glide(&mut self.bias, b, smooth);
                let g = self.auto_gain_target();
                glide(&mut self.auto_gain, g, smooth);
            }
            ids::TONE => {
                let t = db(self.values.f(ids::TONE) * 0.01 * TILT_DB);
                glide(&mut self.tilt, t, smooth);
            }
            ids::OVERSAMPLING => {
                let m = Mode::from_index(self.values.index(ids::OVERSAMPLING));
                if m != self.mode {
                    if smooth {
                        for c in &mut self.ch {
                            c.reset_path(m);
                        }
                        self.fade_from = self.mode;
                        self.fade_left = self.fade_len;
                    }
                    self.mode = m;
                }
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

    fn render(&mut self, audio: &mut AudioBuffers<'_, '_>, start: usize, end: usize) {
        let n_ch = audio.outputs.len().min(2);
        for i in start..end {
            let drive = self.drive.tick();
            let bias = self.bias.tick();
            let shape = Shape {
                curve: self.curve,
                drive,
                bias,
                offset: if bias == 0.0 {
                    0.0
                } else {
                    self.curve.apply(drive * bias)
                },
            };
            let ag = self.auto_gain.tick();
            let tilt_hi = self.tilt.tick();
            let tilt_lo = 1.0 / tilt_hi;
            let out_gain = self.output.tick();
            let mix = self.mix.tick();
            let fade = if self.fade_left > 0 {
                self.fade_left -= 1;
                Some(self.fade_left as f32 / self.fade_len as f32)
            } else {
                None
            };
            for c in 0..n_ch {
                let x = match audio.inputs.get(c).or(audio.inputs.first()) {
                    Some(inp) => inp[i],
                    None => 0.0,
                };
                let st = &mut self.ch[c];
                let mut wet = st.path(self.mode, x, &shape);
                if let Some(old) = fade {
                    let prev = st.path(self.fade_from, x, &shape);
                    wet = wet * (1.0 - old) + prev * old;
                }
                // DC blocker.
                let y = wet - st.dc_x1 + self.dc_r * st.dc_y1;
                st.dc_x1 = wet;
                st.dc_y1 = flush(y);
                // Tilt.
                st.tilt_lp = flush(st.tilt_lp + self.tilt_a * (y - st.tilt_lp));
                let toned = st.tilt_lp * tilt_lo + (y - st.tilt_lp) * tilt_hi;
                let dry = st.dry_delay.tick(x);
                let out = (dry + (toned * ag - dry) * mix) * out_gain;
                audio.outputs[c][i] = out;
            }
        }
    }
}

/// Gain that brings a -12 dBFS sine through `curve` (drive `g`, bias `b`, DC removed) back to
/// its input RMS, clamped to ±24 dB. Bounded work (32 points), RT-safe.
pub fn auto_gain(curve: Curve, g: f32, b: f32) -> f32 {
    const N: usize = 32;
    let offset = curve.apply(g * b);
    let mut ys = [0.0f32; N];
    let mut mean = 0.0;
    for (j, y) in ys.iter_mut().enumerate() {
        let th = 2.0 * std::f32::consts::PI * (j as f32 + 0.5) / N as f32;
        *y = curve.apply(g * (PROBE_AMP * th.sin() + b)) - offset;
        mean += *y;
    }
    mean /= N as f32;
    let var = ys.iter().map(|y| (y - mean) * (y - mean)).sum::<f32>() / N as f32;
    let rms_in = PROBE_AMP * std::f32::consts::FRAC_1_SQRT_2;
    let rms_out = var.sqrt();
    if rms_out < 1e-6 {
        return 1.0;
    }
    (rms_in / rms_out).clamp(db(-24.0), db(24.0))
}

fn descriptor() -> DeviceDescriptor {
    super::descriptor(BuiltinDeviceType::Saturator)
}

impl Node for Saturator {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.sync_all();
        self.reset();
    }

    fn reset(&mut self) {
        for c in &mut self.ch {
            c.reset();
        }
        self.fade_left = 0;
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        split_at_events(
            self,
            ctx.events,
            ctx.frames,
            |s, a, b| s.render(audio, a, b),
            |s, kind| {
                if let EventKind::Param { param, value } = *kind {
                    s.set(param, value, true);
                }
            },
        );
        ProcessStatus::Continue
    }

    fn latency(&self) -> u32 {
        LATENCY as u32
    }
}

impl Device for Saturator {
    fn descriptor(&self) -> DeviceDescriptor {
        descriptor()
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.values.get(id)
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.set(id, value, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_pass_through_the_origin_with_unit_slope() {
        for c in Curve::ALL {
            assert_eq!(c.apply(0.0), 0.0, "{c:?}");
            if c != Curve::Rectify {
                let slope = c.apply(1e-3) / 1e-3;
                assert!((slope - 1.0).abs() < 0.01, "{c:?}: {slope}");
            }
            for u in [-1e6f32, -3.0, 3.0, 1e6] {
                assert!(c.apply(u).abs() <= 1.0 + 1e-6, "{c:?} bounded");
            }
        }
    }

    #[test]
    fn auto_gain_is_unity_for_a_linear_region() {
        let g = auto_gain(Curve::Hard, 1.0, 0.0);
        assert!((g - 1.0).abs() < 1e-4, "{g}");
        // Heavy drive is compensated down.
        assert!(auto_gain(Curve::Soft, db(36.0), 0.0) < db(-12.0));
    }
}

//! Built-in `AdaptiveEq`: an EQ that listens and keeps rebalancing the sound. It cuts what
//! sticks out (resonances, a boomy or harsh range: `Tame`) and lifts what is buried
//! (`Recover`), continuously, towards a balanced tonal curve (`Brighten` tilts it).
//!
//! The heuristic, every [`HOP`] samples:
//! 1. **Listen**: a 4096-point FFT (Hann) of the mid signal, its power summed into [`BANDS`]
//!    log-spaced bands between `Low` and `High`, smoothed over time (`Speed`).
//! 2. **Compare** with the reference: each band's level with the reference tilt removed
//!    (-1.5 dB per octave, a typical balanced mix's long-term spectrum, ±1.5 with
//!    `Brighten`). Equal-width (in octaves) bands of pink noise read flat.
//! 3. **Split** the deviation into a broad part (the levels smoothed across about an
//!    octave, minus their mean: tonal balance) and a narrow part (a band above or below its
//!    smoothed neighbourhood: resonances and holes).
//! 4. **Decide**: cut `Tame` × (60 % of the broad excess + 90 % of the narrow peak); boost
//!    `Recover` × (60 % of the broad deficit + 40 % of the narrow dip), only for bands
//!    within 30 dB of the mean (a band that is nearly empty isn't masked, it's absent:
//!    boosting it would only raise noise). Clamped to ±`Range` dB, lightly smoothed
//!    across bands, then glided in time (`Speed`).
//! 5. **Apply**: one bell filter per band (the EQ's state-variable filters, Q from the
//!    band spacing), so the processing adds no latency; coefficients glide every
//!    [`CONTROL`] samples. `Auto Gain` compensates the loudness change the curve makes
//!    (from the band powers, lightly weighted towards the presence range).
//!
//! With `Tame` and `Recover` at 0 (and `Auto Gain`, `Output` neutral) the output is the
//! input, bit for bit. `Freeze` holds the current curve.
//!
//! Analysis: `AnalysisKind::Levels`, see [`Frame`].
//!
//! # Parameter ids (stable, append-only)
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Balance | `Recover` | 0 ..= 100 %, default 50 |
//! | 1 | Balance | `Tame` | 0 ..= 100 %, default 50 |
//! | 2 | Balance | `Brighten` | -100 ..= 100 %, default 0 |
//! | 3 | Balance | `Range` | 1 ..= 12 dB, default 6 |
//! | 4 | Balance | `Speed` | 20 ..= 3000 ms (log), default 300 |
//! | 5 | Bands | `Low` | 20 ..= 500 Hz (log), default 40 |
//! | 6 | Bands | `High` | 2000 ..= 20000 Hz (log), default 16000 |
//! | 7 | Output | `Auto Gain` | toggle, default on |
//! | 8 | Output | `Freeze` | toggle, default off |
//! | 9 | Output | `Output` | -12 ..= 12 dB, default 0 |
//! | 10 | Output | `Mix` | 0 ..= 100 %, default 100 |

use std::sync::Arc;

use ether_core::protocol::devices::{DeviceCategory, DeviceDescriptor, ParamScale, ParamUnit};
use ether_core::protocol::layout::{DeviceLayout, LayoutItem, Widget, WidgetSize};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, ParamId};
use ether_core::{
    AnalysisKind, AnalysisSink, AudioBuffers, Device, EventKind, Node, PrepareConfig,
    ProcessContext, ProcessStatus, Smoother,
};
use realfft::num_complex::Complex32;
use realfft::{RealFftPlanner, RealToComplex};

use crate::contract::{
    FactoryPreset, descriptor as build, item, knob, layout, param, section, toggle,
};
use crate::dsp::simd;
use crate::dsp::svf::{Coefs, Shape, Svf};
use crate::util;

/// Parameter ids (stable: stored in documents and automation).
pub mod params {
    use ether_core::protocol::model::ParamId;
    pub const RECOVER: ParamId = ParamId(0);
    pub const TAME: ParamId = ParamId(1);
    pub const BRIGHTEN: ParamId = ParamId(2);
    pub const RANGE: ParamId = ParamId(3);
    pub const SPEED: ParamId = ParamId(4);
    pub const LOW: ParamId = ParamId(5);
    pub const HIGH: ParamId = ParamId(6);
    pub const AUTO_GAIN: ParamId = ParamId(7);
    pub const FREEZE: ParamId = ParamId(8);
    pub const OUTPUT: ParamId = ParamId(9);
    pub const MIX: ParamId = ParamId(10);
    /// Number of params.
    pub const COUNT: usize = 11;
}
use params as p;

/// Filter bands.
pub const BANDS: usize = 32;
/// Analysis FFT size and hop (samples).
pub const FFT: usize = 4096;
pub const HOP: usize = 1024;
/// Samples between two coefficient glide steps.
pub const CONTROL: usize = 16;
/// Reference tilt (dB per octave) and how far `Brighten` moves it.
const REFERENCE_TILT: f32 = -1.5;
const BRIGHTEN_TILT: f32 = 1.5;
/// Bands quieter than this under the mean aren't boosted.
const PRESENT_DB: f32 = 30.0;
/// Below this level (dBFS, the band powers' sum) the curve holds.
const GATE_DB: f32 = -70.0;
/// Display points of the response.
pub const POINTS: usize = 64;

/// Layout of the `Levels` frame (indices into its values).
pub struct Frame;

impl Frame {
    /// `n`, the points that follow.
    pub const POINTS: usize = 0;
    /// The auto gain now (dB).
    pub const AUTO_GAIN: usize = 1;
    /// `n` frequencies (Hz, log-spaced 20 Hz ..= 20 kHz), `n` response values (dB, the
    /// filters' combined response), then [`BANDS`] band frequencies, [`BANDS`] band levels
    /// relative to the reference (dB, 0 = on the balanced curve) and [`BANDS`] band gains
    /// (dB).
    pub const DATA: usize = 2;
}

/// Descriptor of the adaptive EQ.
pub fn descriptor(ty: BuiltinDeviceType) -> DeviceDescriptor {
    assert_eq!(ty, BuiltinDeviceType::AdaptiveEq);
    let pct = |id, name: &str, group: &str, range: (f64, f64, f64)| {
        param(
            id,
            name,
            group,
            ParamUnit::Percent,
            range,
            ParamScale::Linear,
        )
    };
    let mut d = build(
        BuiltinDeviceType::AdaptiveEq,
        "Adaptive EQ",
        DeviceCategory::AudioEffect,
        vec![
            pct(0, "Recover", "Balance", (0.0, 100.0, 50.0)),
            pct(1, "Tame", "Balance", (0.0, 100.0, 50.0)),
            pct(2, "Brighten", "Balance", (-100.0, 100.0, 0.0)),
            param(
                3,
                "Range",
                "Balance",
                ParamUnit::Decibels,
                (1.0, 12.0, 6.0),
                ParamScale::Linear,
            ),
            param(
                4,
                "Speed",
                "Balance",
                ParamUnit::Milliseconds,
                (20.0, 3000.0, 300.0),
                ParamScale::Log,
            ),
            param(
                5,
                "Low",
                "Bands",
                ParamUnit::Hertz,
                (20.0, 500.0, 40.0),
                ParamScale::Log,
            ),
            param(
                6,
                "High",
                "Bands",
                ParamUnit::Hertz,
                (2000.0, 20_000.0, 16_000.0),
                ParamScale::Log,
            ),
            toggle(7, "Auto Gain", "Output", true),
            toggle(8, "Freeze", "Output", false),
            param(
                9,
                "Output",
                "Output",
                ParamUnit::Decibels,
                (-12.0, 12.0, 0.0),
                ParamScale::Linear,
            ),
            pct(10, "Mix", "Output", (0.0, 100.0, 100.0)),
        ],
        2,
        2,
        false,
        0,
    );
    d.layout = Some(panel());
    d
}

fn panel() -> DeviceLayout {
    use WidgetSize::*;
    layout(vec![
        section(
            "curve",
            None,
            4,
            4,
            vec![LayoutItem {
                colspan: 4,
                ..item(
                    Widget::AdaptiveEq {
                        recover: p::RECOVER,
                        tame: p::TAME,
                    },
                    Large,
                )
            }],
        ),
        section(
            "balance",
            Some("Balance"),
            3,
            5,
            vec![
                knob(p::RECOVER, Large),
                knob(p::TAME, Large),
                knob(p::BRIGHTEN, Medium),
                knob(p::RANGE, Small),
                knob(p::SPEED, Small),
            ],
        ),
        section(
            "output",
            Some("Output"),
            2,
            3,
            vec![
                knob(p::LOW, Small),
                knob(p::HIGH, Small),
                knob(p::OUTPUT, Small),
                item(
                    Widget::Toggle {
                        param: p::AUTO_GAIN,
                    },
                    Small,
                ),
                item(Widget::Toggle { param: p::FREEZE }, Small),
                knob(p::MIX, Small),
            ],
        ),
    ])
}

/// Non-RT. A new instance.
pub fn create(device: &BuiltinDevice) -> Box<dyn Device> {
    assert_eq!(device.device_type(), BuiltinDeviceType::AdaptiveEq);
    Box::new(AdaptiveEq::new())
}

/// `FactoryPreset` entries for `presets/adaptive-eq/<slug>.etherpreset` files.
macro_rules! presets {
    ($($slug:literal),* $(,)?) => {
        &[$(FactoryPreset {
            id: concat!("adaptive-eq/", $slug),
            json: include_str!(concat!("../presets/adaptive-eq/", $slug, ".etherpreset")),
        }),*]
    };
}

/// Factory presets (embedded).
pub fn factory_presets(ty: BuiltinDeviceType) -> &'static [FactoryPreset] {
    match ty {
        BuiltinDeviceType::AdaptiveEq => presets!(
            "gentle-balance",
            "mix-bus-clarity",
            "tame-harshness",
            "open-up-dull",
            "vocal-smoother"
        ),
        _ => &[],
    }
}

/// Fast enough `10^(x/10)`.
#[inline]
fn power_of(db: f32) -> f32 {
    10f32.powf(db * 0.1)
}

/// The built-in adaptive EQ.
pub struct AdaptiveEq {
    values: [f64; p::COUNT],
    ranges: [(f64, f64); p::COUNT],
    sample_rate: f32,
    // Analysis.
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    ring: Vec<f32>,
    at: usize,
    since: usize,
    time: Vec<f32>,
    spec: Vec<Complex32>,
    scratch: Vec<Complex32>,
    /// Bands: centre (Hz), FFT bin range, smoothed level (dB), gain (dB).
    freqs: [f32; BANDS],
    bins: [(usize, usize); BANDS],
    levels: [f32; BANDS],
    primed: bool,
    gains: [f32; BANDS],
    /// Relative levels (for the display).
    relative: [f32; BANDS],
    // Processing.
    filters: [[Svf; 2]; BANDS],
    coefs: [Coefs; BANDS],
    targets: [Coefs; BANDS],
    control: usize,
    glide: f64,
    auto_gain_db: f32,
    gain: Smoother,
    mix: Smoother,
    /// Whether any band filter isn't flat (else they're skipped).
    active: bool,
    tail: util::Tail,
}

impl Default for AdaptiveEq {
    fn default() -> Self {
        Self::new()
    }
}

impl AdaptiveEq {
    /// Non-RT.
    pub fn new() -> Self {
        let d = descriptor(BuiltinDeviceType::AdaptiveEq);
        let mut values = [0.0; p::COUNT];
        let mut ranges = [(0.0, 0.0); p::COUNT];
        for info in &d.params {
            values[info.id.0 as usize] = info.default;
            ranges[info.id.0 as usize] = (info.min, info.max);
        }
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT);
        let scratch = vec![Complex32::new(0.0, 0.0); fft.get_scratch_len()];
        let window = (0..FFT)
            .map(|i| (0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / FFT as f64).cos()) as f32)
            .collect();
        let mut s = Self {
            values,
            ranges,
            sample_rate: 48_000.0,
            fft,
            window,
            ring: vec![0.0; FFT],
            at: 0,
            since: 0,
            time: vec![0.0; FFT],
            spec: vec![Complex32::new(0.0, 0.0); FFT / 2 + 1],
            scratch,
            freqs: [1000.0; BANDS],
            bins: [(0, 0); BANDS],
            levels: [-120.0; BANDS],
            primed: false,
            gains: [0.0; BANDS],
            relative: [0.0; BANDS],
            filters: [[Svf::default(); 2]; BANDS],
            coefs: [Coefs::BYPASS; BANDS],
            targets: [Coefs::BYPASS; BANDS],
            control: 0,
            glide: 0.0,
            auto_gain_db: 0.0,
            gain: Smoother::new(1.0, 50.0, 48_000.0),
            mix: Smoother::new(1.0, 20.0, 48_000.0),
            active: false,
            tail: util::Tail::default(),
        };
        s.configure();
        s
    }

    fn value(&self, id: ParamId) -> f32 {
        self.values[id.0 as usize] as f32
    }

    fn on(&self, id: ParamId) -> bool {
        self.values[id.0 as usize] >= 0.5
    }

    /// Band layout and smoothers for the sample rate and params.
    fn configure(&mut self) {
        let sr = self.sample_rate;
        self.layout_bands();
        for k in 0..BANDS {
            self.targets[k] = self.bell(k, self.gains[k]);
            self.coefs[k] = self.targets[k];
        }
        // Coefficients glide with a 30 ms time constant (per control block).
        self.glide = f64::from(util::tau_coef(30.0, sr)).powi(CONTROL as i32);
        self.gain = Smoother::new(self.output_target(), 50.0, sr);
        self.mix = Smoother::new(self.value(p::MIX) * 0.01, 20.0, sr);
    }

    /// Band centres between `Low` and `High` and their FFT bins.
    fn layout_bands(&mut self) {
        let sr = self.sample_rate;
        let lo = self.value(p::LOW).min(sr * 0.2);
        let hi = self.value(p::HIGH).min(sr * 0.45).max(lo * 4.0);
        let ratio = (hi / lo).powf(1.0 / (BANDS - 1) as f32);
        let half = ratio.sqrt();
        let bin = |f: f32| ((f * FFT as f32 / sr).round() as usize).clamp(1, FFT / 2);
        for k in 0..BANDS {
            let f = lo * ratio.powi(k as i32);
            self.freqs[k] = f;
            let (a, b) = (bin(f / half), bin(f * half));
            self.bins[k] = (a, b.max(a + 1).min(FFT / 2 + 1));
        }
    }

    /// The band spacing (octaves).
    fn spacing(&self) -> f32 {
        (self.freqs[BANDS - 1] / self.freqs[0]).log2() / (BANDS - 1) as f32
    }

    fn bell(&self, k: usize, gain_db: f32) -> Coefs {
        // Q of a bell one band spacing wide (the curve between centres stays smooth).
        let s = f64::from(self.spacing()) * 1.4;
        let q = 2f64.powf(s / 2.0) / (2f64.powf(s) - 1.0);
        Coefs::new(
            Shape::Bell,
            f64::from(self.freqs[k]),
            f64::from(gain_db),
            q,
            f64::from(self.sample_rate),
        )
    }

    fn output_target(&self) -> f32 {
        let auto = if self.on(p::AUTO_GAIN) {
            self.auto_gain_db
        } else {
            0.0
        };
        util::db_to_amp(self.value(p::OUTPUT) + auto)
    }

    fn apply(&mut self, id: ParamId, value: f64, smooth: bool) {
        let Some(&(min, max)) = self.ranges.get(id.0 as usize) else {
            return;
        };
        self.values[id.0 as usize] = util::clamp(value, min, max);
        match id {
            p::MIX => {
                let v = self.value(p::MIX) * 0.01;
                if smooth {
                    self.mix.set_target(v);
                } else {
                    self.mix.set_immediate(v);
                }
            }
            p::OUTPUT | p::AUTO_GAIN => {
                let v = self.output_target();
                if smooth {
                    self.gain.set_target(v);
                } else {
                    self.gain.set_immediate(v);
                }
            }
            p::LOW | p::HIGH => {
                self.layout_bands();
                for k in 0..BANDS {
                    self.targets[k] = self.bell(k, self.gains[k]);
                }
            }
            p::RECOVER | p::TAME if self.value(p::RECOVER) <= 0.0 && self.value(p::TAME) <= 0.0 => {
                // Both off: back to flat (glided).
                self.gains = [0.0; BANDS];
                for k in 0..BANDS {
                    self.targets[k] = self.bell(k, 0.0);
                }
                self.auto_gain_db = 0.0;
                self.gain.set_target(self.output_target());
            }
            _ => {}
        }
    }

    /// RT. One analysis hop: listen, compare, decide (module docs, steps 1-4).
    fn analyse(&mut self) {
        let n = FFT;
        for i in 0..n {
            self.time[i] = self.ring[(self.at + i) % n] * self.window[i];
        }
        let _ = self
            .fft
            .process_with_scratch(&mut self.time, &mut self.spec, &mut self.scratch);
        // Hann power normalization so a full-scale sine reads about 0 dB.
        let norm = 4.0 / (n as f32 * n as f32 * 0.375);
        let mut total = 0.0;
        let mut now = [0.0f32; BANDS];
        for (k, &(a, b)) in self.bins.iter().enumerate() {
            let e: f32 = self.spec[a..b].iter().map(|c| c.norm_sqr()).sum::<f32>() * norm;
            total += e;
            now[k] = 10.0 * e.max(1e-14).log10();
        }
        let frozen = self.on(p::FREEZE);
        let quiet = 10.0 * total.max(1e-14).log10() < GATE_DB;
        if quiet || frozen {
            return;
        }
        // Time smoothing (`Speed`), per hop.
        let k = (-(HOP as f32) / (self.value(p::SPEED) * 0.001 * self.sample_rate)).exp();
        if self.primed {
            for (l, &v) in self.levels.iter_mut().zip(&now) {
                *l = v + (*l - v) * k;
            }
        } else {
            self.levels = now;
            self.primed = true;
        }
        // Reference removed: flat for pink noise, tilted by `Brighten`.
        let tilt = REFERENCE_TILT + BRIGHTEN_TILT * self.value(p::BRIGHTEN) * 0.01;
        let rel: [f32; BANDS] =
            std::array::from_fn(|b| self.levels[b] - tilt * (self.freqs[b] / 1000.0).log2());
        // Broad envelope: a Gaussian about an octave wide across bands.
        let sigma = (1.0 / self.spacing()).max(1.0);
        let mut env = [0.0f32; BANDS];
        for (b, e) in env.iter_mut().enumerate() {
            let (mut acc, mut wsum) = (0.0, 0.0);
            for (c, &r) in rel.iter().enumerate() {
                let d = (b as f32 - c as f32) / sigma;
                let w = (-0.5 * d * d).exp();
                acc += w * r;
                wsum += w;
            }
            *e = acc / wsum;
        }
        let top = rel.iter().copied().fold(f32::MIN, f32::max);
        let present = |b: usize| rel[b] > top - 60.0;
        let (mut sum, mut count) = (0.0, 0.0);
        for (b, &e) in env.iter().enumerate() {
            if present(b) {
                sum += e;
                count += 1.0;
            }
        }
        let mean = if count > 0.0 { sum / count } else { 0.0 };
        let (recover, tame) = (self.value(p::RECOVER) * 0.01, self.value(p::TAME) * 0.01);
        let range = self.value(p::RANGE);
        let mut want = [0.0f32; BANDS];
        for b in 0..BANDS {
            self.relative[b] = rel[b] - mean;
            if !present(b) {
                continue;
            }
            let broad = env[b] - mean;
            let narrow = rel[b] - env[b];
            let cut = tame * (0.6 * broad.max(0.0) + 0.9 * narrow.max(0.0));
            let boost = if rel[b] > mean - PRESENT_DB {
                recover * (0.6 * (-broad).max(0.0) + 0.4 * (-narrow).max(0.0))
            } else {
                0.0
            };
            want[b] = (boost - cut).clamp(-range, range);
        }
        // Light smoothing across bands, then the glide in time.
        for b in 0..BANDS {
            let l = want[b.saturating_sub(1)];
            let r = want[(b + 1).min(BANDS - 1)];
            let v = 0.25 * l + 0.5 * want[b] + 0.25 * r;
            self.gains[b] = v + (self.gains[b] - v) * k;
        }
        for b in 0..BANDS {
            self.targets[b] = self.bell(b, self.gains[b]);
        }
        // Auto gain: the loudness the curve adds, from the band powers (a little more
        // weight from 1 kHz up, as the ear).
        let (mut before, mut after) = (0.0f32, 0.0f32);
        for b in 0..BANDS {
            let w = 1.0 + 0.6 * ((self.freqs[b] / 1000.0).log2() / 2.0).clamp(0.0, 1.0);
            let e = power_of(self.levels[b]) * w;
            before += e;
            after += e * power_of(self.gains[b]);
        }
        self.auto_gain_db = if before > 0.0 {
            -10.0 * (after / before).log10()
        } else {
            0.0
        };
        self.gain.set_target(self.output_target());
    }

    simd::avx2_dispatch! {
        /// RT. Frames `start..end` of a block.
        fn render => render_avx2, render_generic(
            &mut self,
            audio: &mut AudioBuffers<'_, '_>,
            start: usize,
            end: usize,
        )
    }

    #[inline(always)]
    fn render_generic(&mut self, audio: &mut AudioBuffers<'_, '_>, start: usize, end: usize) {
        let inputs = audio.inputs;
        let input = |c: usize, i: usize| {
            inputs
                .get(c)
                .or_else(|| inputs.first())
                .map_or(0.0, |ch| if ch[i].is_finite() { ch[i] } else { 0.0 })
        };
        for i in start..end {
            let x = [input(0, i), input(1, i)];
            self.ring[self.at] = 0.5 * (x[0] + x[1]);
            self.at = (self.at + 1) % FFT;
            self.since += 1;
            if self.since >= HOP {
                self.since = 0;
                self.analyse();
            }
            if self.control == 0 {
                self.control = CONTROL;
                let mut moving = false;
                for b in 0..BANDS {
                    moving |= self.coefs[b].glide(&self.targets[b], self.glide);
                }
                self.active = moving || self.coefs.iter().any(|c| !c.is_bypass());
            }
            self.control -= 1;
            let mut y = [f64::from(x[0]), f64::from(x[1])];
            if self.active {
                for b in 0..BANDS {
                    let c = &self.coefs[b];
                    for (ch, v) in y.iter_mut().enumerate() {
                        *v = self.filters[b][ch].tick(*v, c);
                    }
                }
            }
            let gain = self.gain.tick();
            let mix = self.mix.tick();
            for (c, out) in audio.outputs.iter_mut().enumerate() {
                out[i] = if c < 2 {
                    let wet = y[c] as f32 * gain;
                    x[c] + (wet - x[c]) * mix
                } else {
                    0.0
                };
            }
        }
    }

    /// The combined response of the band filters at `f` (dB).
    fn response_db(&self, f: f32) -> f32 {
        let sr = f64::from(self.sample_rate);
        let m: f64 = self
            .coefs
            .iter()
            .map(|c| c.magnitude(f64::from(f), sr))
            .product();
        (20.0 * m.max(1e-9).log10()) as f32
    }

    /// The band gains now (dB; tests).
    pub fn gains(&self) -> [f32; BANDS] {
        self.gains
    }

    /// The band centres (Hz; tests).
    pub fn frequencies(&self) -> [f32; BANDS] {
        self.freqs
    }
}

impl Node for AdaptiveEq {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(8_000.0);
        self.configure();
        self.reset();
    }

    fn reset(&mut self) {
        for f in self.filters.iter_mut().flatten() {
            f.reset();
        }
        self.ring.fill(0.0);
        self.at = 0;
        self.since = 0;
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let frames = ctx.frames;
        // Asleep for a while (silent input): the analysis ring holds silence and the hops
        // keep their place on the timeline, as if it had run.
        let skipped = self.tail.skipped(ctx) as usize;
        if skipped > 0 {
            for _ in 0..skipped.min(FFT) {
                self.ring[self.at] = 0.0;
                self.at = (self.at + 1) % FFT;
            }
            self.since = (self.since + skipped) % HOP;
            self.control = (self.control + CONTROL - skipped % CONTROL) % CONTROL;
        }
        util::split_at_events(
            self,
            ctx.events,
            frames,
            |s, a, b| s.render(audio, a, b),
            |s, kind| {
                if let EventKind::Param { param, value, step } = *kind {
                    s.apply(param, value, !step);
                }
            },
        );
        let settled = !self.gain.is_smoothing() && !self.mix.is_smoothing();
        // Bells ring a little at low frequencies.
        let hold = util::samples(0.2, self.sample_rate);
        self.tail.status_of(ctx, audio, settled, hold)
    }

    fn has_analysis(&self) -> bool {
        true
    }

    fn analysis(&mut self, out: &mut AnalysisSink<'_>) {
        let Some(f) = out.frame(AnalysisKind::Levels) else {
            return;
        };
        f.push(POINTS as f32);
        f.push(if self.on(p::AUTO_GAIN) {
            self.auto_gain_db
        } else {
            0.0
        });
        let freq = |i: usize| 20.0 * 1000f32.powf(i as f32 / (POINTS - 1) as f32);
        for i in 0..POINTS {
            f.push(freq(i));
        }
        for i in 0..POINTS {
            let v = self.response_db(freq(i));
            f.push(v);
        }
        for ring in [&self.freqs, &self.relative, &self.gains] {
            for &v in ring.iter() {
                f.push(v);
            }
        }
    }
}

impl Device for AdaptiveEq {
    fn descriptor(&self) -> DeviceDescriptor {
        descriptor(BuiltinDeviceType::AdaptiveEq)
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.values.get(id.0 as usize).copied()
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.apply(id, value, false);
    }
}

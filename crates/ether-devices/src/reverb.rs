//! Built-in `Reverb` device (roadmap v2).
//!
//! Algorithmic stereo reverb: stereo pre-delay → two series allpass diffusers per channel
//! → a multichannel diffuser per channel → an 8-line feedback delay network (FDN) with a
//! normalized Hadamard feedback matrix.
//! Each line has a one-pole low-pass (damping) and a gain `10^(-3·len/(fs·RT60))`, so
//! the tail decays by 60 dB in `Decay` seconds at low frequencies regardless of `Size`.
//! The matrix is orthogonal and every loop gain is < 1, so the network is stable at every
//! setting (20 s decay, zero damping included).
//!
//! The multichannel diffuser (at large sizes the tank's first echoes came 60 to 117 ms
//! apart, an early tail of 0.23 echo density 50 ms in) splits each channel into 4, then
//! [`DIFF_STAGES`] times delays the 4 by spread lengths (scaled with `Size`) and mixes them
//! with an orthogonal rotation (a Hadamard-like matrix at full `Diffusion`, the identity at
//! 0 %): 4³ = 64 echoes per line before the tank. Its 4 outputs of the left channel feed the
//! even lines and those of the right the odd ones; the wet outputs are two
//! orthogonal sign patterns over all lines (decorrelated L/R), then mid/side `Width`.
//! `Size` scales the line lengths (read with fractional delays, gliding on change), the
//! other params are smoothed; buffers are allocated in `prepare`.
//!
//! # Parameter ids (stable, append-only)
//!
//! | id | name        | range                  |
//! |----|-------------|------------------------|
//! | 0  | `Pre-Delay` | 0 ..= 250 ms           |
//! | 1  | `Size`      | 0 ..= 100 %            |
//! | 2  | `Decay`     | 0.2 ..= 20 s (log)     |
//! | 3  | `Damping`   | 0 ..= 100 %            |
//! | 4  | `Width`     | 0 ..= 100 %            |
//! | 5  | `Mix`       | 0 ..= 100 %            |
//! | 6  | `Low Cut`   | 20 ..= 20000 Hz (log), 20 = off |
//! | 7  | `High Cut`  | 20 ..= 20000 Hz (log), 20000 = off |
//! | 8  | `Diffusion` | 0 ..= 100 %, default 60 (the former fixed allpass gain 0.6) |
//! | 9  | `Freeze`    | toggle: the tank stops taking input and sustains (unity loop gain, no damping) |
//! | 10 | `Output`    | -36 ..= +12 dB, default 0: wet level trim (the dry path is untouched) |
//!
//! The cut filters shape what enters the tank (12 dB/oct; with both off the input is
//! untouched); every appended param defaults to the sound before it (bit-exact).

use ether_core::protocol::devices::{
    DeviceCategory, DeviceDescriptor, DeviceTypeRef, ParamInfo, ParamScale, ParamUnit,
};
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus, Smoother,
};

use crate::dsp::cut_filter::{CutFilter, HIGH_OFF_HZ, LOW_OFF_HZ};
use crate::dsp::delay_line::DelayLine;
use crate::dsp::flush32;
use crate::util;

/// Parameter ids (stable: stored in documents and automation).
pub mod params {
    use ether_core::protocol::model::ParamId;
    pub const PRE_DELAY: ParamId = ParamId(0);
    pub const SIZE: ParamId = ParamId(1);
    pub const DECAY: ParamId = ParamId(2);
    pub const DAMPING: ParamId = ParamId(3);
    pub const WIDTH: ParamId = ParamId(4);
    pub const MIX: ParamId = ParamId(5);
    /// Input low cut (Hz, 20 = off).
    pub const LOW_CUT: ParamId = ParamId(6);
    /// Input high cut (Hz, 20000 = off).
    pub const HIGH_CUT: ParamId = ParamId(7);
    /// Allpass diffusion (percent).
    pub const DIFFUSION: ParamId = ParamId(8);
    /// Freeze (toggle).
    pub const FREEZE: ParamId = ParamId(9);
    /// Wet level trim (dB, 0 = the reverb's level before it existed). Level-matches imported
    /// reverbs (Ableton's Reflect / Diffuse levels).
    pub const OUTPUT: ParamId = ParamId(10);
}

const NUM_PARAMS: usize = 11;
const LINES: usize = 8;
/// FDN line lengths at 48 kHz for a size factor of 1 (mutually prime).
const BASE_LENGTHS: [f32; LINES] = [
    1433.0, 1601.0, 1867.0, 2053.0, 2251.0, 2399.0, 2617.0, 2797.0,
];
/// Size factor range (`Size` 0 % .. 100 %).
const SIZE_MIN: f32 = 0.2;
const SIZE_MAX: f32 = 2.0;
/// Allpass diffuser lengths at 48 kHz (left, right) and feedback.
const DIFFUSERS: [[f32; 2]; 2] = [[155.0, 413.0], [163.0, 421.0]];
/// Default `Diffusion` (percent): the allpass gain 0.6 the reverb always had.
const DEFAULT_DIFFUSION: f64 = 60.0;
/// Largest allpass gain (`Diffusion` 100 %).
const MAX_DIFFUSION: f64 = 0.8;
const MAX_PRE_DELAY_MS: f32 = 250.0;
/// Glide time constant for size and pre-delay changes.
const GLIDE_MS: f32 = 80.0;
/// Coefficient update interval (loop gains) in samples.
const CONTROL_INTERVAL: usize = 32;
/// Wet output scale (roughly unity loudness for broadband input at mid settings).
const WET_GAIN: f32 = 0.35;
/// Stages of the multichannel diffuser.
const DIFF_STAGES: usize = 3;
/// Channels per side of the multichannel diffuser (each feeds one FDN line).
const DIFF_CH: usize = LINES / 2;
/// Span of each diffuser stage at 48 kHz for a size factor of 1 (samples): channel `k`
/// delays by `span · (k + DIFF_POS[side][stage][k]) / 4`.
const DIFF_SPANS: [f32; DIFF_STAGES] = [211.0, 467.0, 997.0];
/// Where each channel's delay sits in its quarter of the span (left, right: decorrelated).
const DIFF_POS: [[[f32; DIFF_CH]; DIFF_STAGES]; 2] = [
    [
        [0.37, 0.81, 0.13, 0.62],
        [0.71, 0.24, 0.55, 0.09],
        [0.18, 0.66, 0.92, 0.41],
    ],
    [
        [0.58, 0.07, 0.86, 0.33],
        [0.26, 0.69, 0.11, 0.84],
        [0.77, 0.35, 0.49, 0.03],
    ],
];
/// Polarity of each channel after each stage's delays.
const DIFF_FLIPS: [[f32; DIFF_CH]; DIFF_STAGES] = [
    [1.0, -1.0, 1.0, -1.0],
    [1.0, 1.0, -1.0, -1.0],
    [-1.0, 1.0, 1.0, -1.0],
];

/// Parameter list (identical for every instance).
pub fn param_infos() -> Vec<ParamInfo> {
    use util::param;
    vec![
        param(
            0,
            "Pre-Delay",
            "Reverb",
            ParamUnit::Milliseconds,
            (0.0, MAX_PRE_DELAY_MS as f64, 20.0),
            ParamScale::Power { exponent: 2.0 },
        ),
        param(
            1,
            "Size",
            "Reverb",
            ParamUnit::Percent,
            (0.0, 100.0, 50.0),
            ParamScale::Linear,
        ),
        param(
            2,
            "Decay",
            "Reverb",
            ParamUnit::Seconds,
            (0.2, 20.0, 2.0),
            ParamScale::Log,
        ),
        param(
            3,
            "Damping",
            "Reverb",
            ParamUnit::Percent,
            (0.0, 100.0, 50.0),
            ParamScale::Linear,
        ),
        param(
            4,
            "Width",
            "Output",
            ParamUnit::Percent,
            (0.0, 100.0, 100.0),
            ParamScale::Linear,
        ),
        param(
            5,
            "Mix",
            "Output",
            ParamUnit::Percent,
            (0.0, 100.0, 30.0),
            ParamScale::Linear,
        ),
        // --- devices pass effects ---
        param(
            6,
            "Low Cut",
            "Filter",
            ParamUnit::Hertz,
            (LOW_OFF_HZ, 20_000.0, LOW_OFF_HZ),
            ParamScale::Log,
        ),
        param(
            7,
            "High Cut",
            "Filter",
            ParamUnit::Hertz,
            (LOW_OFF_HZ, HIGH_OFF_HZ, HIGH_OFF_HZ),
            ParamScale::Log,
        ),
        param(
            8,
            "Diffusion",
            "Reverb",
            ParamUnit::Percent,
            (0.0, 100.0, DEFAULT_DIFFUSION),
            ParamScale::Linear,
        ),
        util::choice(9, "Freeze", "Reverb", ParamUnit::Toggle, &["Off", "On"], 0),
        param(
            10,
            "Output",
            "Output",
            ParamUnit::Decibels,
            (-36.0, 12.0, 0.0),
            ParamScale::Linear,
        ),
    ]
}

/// Descriptor of the `Reverb` type.
pub fn descriptor() -> DeviceDescriptor {
    DeviceDescriptor {
        layout: Some(layout()),
        device_type: DeviceTypeRef::Builtin {
            device: BuiltinDeviceType::Reverb,
        },
        name: "Reverb".to_owned(),
        category: DeviceCategory::AudioEffect,
        params: param_infos(),
        audio_inputs: 2,
        audio_outputs: 2,
        midi_input: false,
        sidechain_inputs: 0,
        implemented: true,
    }
}

/// Declarative panel (drawn by the shared renderer in the device's window): the decay
/// display (with its decay / pre-delay / size / damping / freeze controls), then the input
/// filter, the diffusion and the output.
pub fn layout() -> ether_core::protocol::layout::DeviceLayout {
    use crate::contract::{item, knob, layout, section};
    use ether_core::protocol::layout::{Widget, WidgetSize::*};
    layout(vec![
        section(
            "decay",
            Some("Reverb"),
            4,
            1,
            vec![item(
                Widget::ReverbDecay {
                    decay: params::DECAY,
                    pre_delay: Some(params::PRE_DELAY),
                    size: Some(params::SIZE),
                    damping: Some(params::DAMPING),
                    freeze: Some(params::FREEZE),
                },
                Large,
            )],
        ),
        section(
            "filter",
            Some("Input Filter"),
            1,
            2,
            vec![
                knob(params::LOW_CUT, Medium),
                knob(params::HIGH_CUT, Medium),
            ],
        ),
        section(
            "character",
            Some("Character"),
            1,
            1,
            vec![knob(params::DIFFUSION, Medium)],
        ),
        section(
            "output",
            Some("Output"),
            1,
            3,
            vec![
                knob(params::WIDTH, Medium),
                knob(params::OUTPUT, Medium),
                knob(params::MIX, Large),
            ],
        ),
    ])
}

/// Non-RT. A new instance with default params.
pub fn create() -> Box<dyn Device> {
    Box::new(Reverb::new())
}

/// Schroeder allpass on a delay line.
#[derive(Clone, Debug, Default)]
struct Allpass {
    line: DelayLine,
    len: usize,
}

/// Allpass gain of a `Diffusion` value (percent): linear 0 → 0.6 up to the default 60 %
/// (exactly the former fixed 0.6 there), then 0.6 → [`MAX_DIFFUSION`] at 100 %.
fn diffusion_gain(percent: f64) -> f32 {
    let g = if percent <= DEFAULT_DIFFUSION {
        percent / 100.0
    } else {
        0.6 + (percent - DEFAULT_DIFFUSION) / (100.0 - DEFAULT_DIFFUSION) * (MAX_DIFFUSION - 0.6)
    };
    g as f32
}

impl Allpass {
    fn new(len: usize) -> Self {
        Self {
            line: DelayLine::new(len),
            len,
        }
    }

    #[inline]
    fn tick(&mut self, x: f32, diffusion: f32) -> f32 {
        let d = self.line.tap(self.len);
        let v = x - diffusion * d;
        self.line.push(v);
        d + diffusion * v
    }
}

/// In-place normalized 8-point Walsh-Hadamard transform (orthogonal).
#[inline]
fn hadamard8(x: &mut [f32; LINES]) {
    let mut h = 1;
    while h < LINES {
        let mut i = 0;
        while i < LINES {
            for j in i..i + h {
                let (a, b) = (x[j], x[j + h]);
                x[j] = a + b;
                x[j + h] = a - b;
            }
            i += 2 * h;
        }
        h *= 2;
    }
    let norm = 1.0 / (LINES as f32).sqrt();
    x.iter_mut().for_each(|v| *v *= norm);
}

/// One stage's mix `cos θ · x + sin θ · J·x`: `J` (skew-symmetric, orthogonal, its entries
/// ±1/√3 off the diagonal) makes it a rotation for every `θ`; at `θ = π/3` every entry is
/// ±1/2 (each output mixes all four inputs equally).
#[inline]
fn diffuse_mix(x: &mut [f32; DIFF_CH], cos: f32, sin: f32) {
    let s = sin * 0.577_350_27; // 1/√3
    let [a, b, c, d] = *x;
    *x = [
        cos * a + s * (b + c + d),
        cos * b + s * (-a - c + d),
        cos * c + s * (-a + b - d),
        cos * d + s * (-a - b + c),
    ];
}

/// The mixing angle of a `Diffusion` value (percent): 0 → none, full (π/3) from the
/// default up.
fn diffuse_angle(percent: f64) -> f32 {
    (std::f64::consts::FRAC_PI_3 * (percent / DEFAULT_DIFFUSION).min(1.0)) as f32
}

/// Output sign patterns (two orthogonal Hadamard rows).
const OUT_L: [f32; LINES] = [1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0];
const OUT_R: [f32; LINES] = [1.0, 1.0, -1.0, -1.0, 1.0, 1.0, -1.0, -1.0];

/// The built-in reverb.
#[derive(Debug)]
pub struct Reverb {
    values: [f64; NUM_PARAMS],
    ranges: [(f64, f64); NUM_PARAMS],
    sample_rate: f32,
    pre_delay: [DelayLine; 2],
    diffusers: [[Allpass; 2]; 2],
    /// Multichannel diffuser delays: `[side][stage][channel]`.
    multi: [[[DelayLine; DIFF_CH]; DIFF_STAGES]; 2],
    /// Its mixing angle (`Diffusion`), smoothed.
    angle: Smoother,
    lines: [DelayLine; LINES],
    /// Damping low-pass state per line.
    lp: [f32; LINES],
    /// Per-line loop gain (updated every `CONTROL_INTERVAL` samples).
    gains: [f32; LINES],
    control_countdown: usize,
    glide_coef: f32,
    /// Current (gliding) size factor and pre-delay in samples.
    size: f32,
    pre: f32,
    snap: bool,
    decay: Smoother,
    damping: Smoother,
    width: Smoother,
    mix: Smoother,
    /// Wet trim as a linear gain (`Output`; exactly 1.0 at 0 dB, so the default is
    /// bit-identical to the reverb before the param).
    output: Smoother,
    /// Allpass gain (`Diffusion`), freeze amount (0..1) and the input filters.
    diffusion: Smoother,
    freeze: Smoother,
    filter: CutFilter,
    /// Silence reporting: the tank or the diffusers carried signal above
    /// the floor in this block.
    loud: bool,
    /// Samples a signal can stay inside the delays before it reaches the output.
    hold: usize,
    tail: util::Tail,
}

impl Default for Reverb {
    fn default() -> Self {
        Self::new()
    }
}

impl Reverb {
    /// Non-RT. A reverb with default parameters (buffers sized for 48 kHz until
    /// `prepare`).
    pub fn new() -> Self {
        let infos = param_infos();
        let mut values = [0.0; NUM_PARAMS];
        let mut ranges = [(0.0, 0.0); NUM_PARAMS];
        for (i, info) in infos.iter().enumerate() {
            values[i] = info.default;
            ranges[i] = (info.min, info.max);
        }
        let s = || Smoother::new(0.0, 20.0, 48_000.0);
        let mut r = Self {
            values,
            ranges,
            sample_rate: 48_000.0,
            pre_delay: Default::default(),
            diffusers: Default::default(),
            multi: Default::default(),
            angle: s(),
            lines: Default::default(),
            lp: [0.0; LINES],
            gains: [0.0; LINES],
            control_countdown: 0,
            glide_coef: 0.0,
            size: 1.0,
            pre: 0.0,
            snap: true,
            decay: s(),
            damping: s(),
            width: s(),
            mix: s(),
            output: s(),
            diffusion: s(),
            freeze: s(),
            filter: CutFilter::default(),
            loud: false,
            hold: 0,
            tail: util::Tail::default(),
        };
        r.allocate();
        r
    }

    fn value(&self, id: ParamId) -> f32 {
        self.values[id.0 as usize] as f32
    }

    fn scale(&self) -> f32 {
        self.sample_rate / 48_000.0
    }

    fn allocate(&mut self) {
        let scale = self.scale();
        let pre_len = (MAX_PRE_DELAY_MS * 0.001 * self.sample_rate).ceil() as usize + 2;
        self.pre_delay = [DelayLine::new(pre_len), DelayLine::new(pre_len)];
        self.diffusers = DIFFUSERS
            .map(|pair| pair.map(|len| Allpass::new(((len * scale).round() as usize).max(1))));
        self.multi = [0, 1].map(|_| {
            DIFF_SPANS.map(|span| {
                let longest = span * SIZE_MAX * scale;
                std::array::from_fn(|_| DelayLine::new(longest.ceil() as usize + 2))
            })
        });
        self.lines =
            BASE_LENGTHS.map(|len| DelayLine::new((len * SIZE_MAX * scale).ceil() as usize + 2));
        self.glide_coef = util::tau_coef(GLIDE_MS, self.sample_rate);
        // Longest path through pre-delay, diffusers and the longest line, plus margin.
        let diffusion: f32 =
            DIFFUSERS.iter().flatten().sum::<f32>() + DIFF_SPANS.iter().sum::<f32>() * SIZE_MAX;
        let longest = BASE_LENGTHS.iter().fold(0.0f32, |m, &l| m.max(l));
        self.hold = pre_len
            + ((diffusion + longest * SIZE_MAX) * scale).ceil() as usize
            + util::samples(util::BASE_HOLD_SECONDS, self.sample_rate);
        let sr = self.sample_rate;
        let mk = |v: f32| Smoother::new(v, 20.0, sr);
        self.decay = mk(self.value(params::DECAY));
        self.damping = mk(self.damping_coef());
        self.width = mk(self.value(params::WIDTH) * 0.01);
        self.mix = mk(self.value(params::MIX) * 0.01);
        self.output = mk(util::db_to_amp(self.value(params::OUTPUT)));
        self.diffusion = mk(diffusion_gain(self.values[params::DIFFUSION.0 as usize]));
        self.angle = mk(diffuse_angle(self.values[params::DIFFUSION.0 as usize]));
        self.freeze = mk(self.freeze_target());
        self.filter.prepare(self.sample_rate);
        self.update_filter(false);
        self.reset();
    }

    fn freeze_target(&self) -> f32 {
        if self.value(params::FREEZE) >= 0.5 {
            1.0
        } else {
            0.0
        }
    }

    fn update_filter(&mut self, smooth: bool) {
        self.filter.set(
            self.values[params::LOW_CUT.0 as usize],
            self.values[params::HIGH_CUT.0 as usize],
            smooth,
        );
    }

    /// One-pole low-pass coefficient for the `Damping` param: 0 % ≈ 20 kHz, 100 % ≈ 1 kHz.
    fn damping_coef(&self) -> f32 {
        let d = self.value(params::DAMPING) * 0.01;
        let fc = (20_000.0 * 0.05f32.powf(d)).min(self.sample_rate * 0.45);
        (-std::f32::consts::TAU * fc / self.sample_rate).exp()
    }

    fn target_size(&self) -> f32 {
        SIZE_MIN + (SIZE_MAX - SIZE_MIN) * self.value(params::SIZE) * 0.01
    }

    fn target_pre(&self) -> f32 {
        self.value(params::PRE_DELAY) * 0.001 * self.sample_rate
    }

    fn apply_param(&mut self, id: ParamId, value: f64, smooth: bool) {
        let Some(&(min, max)) = self.ranges.get(id.0 as usize) else {
            return;
        };
        self.values[id.0 as usize] = util::clamp(value, min, max);
        let v = match id {
            params::DECAY => self.value(params::DECAY),
            params::DAMPING => self.damping_coef(),
            params::WIDTH => self.value(params::WIDTH) * 0.01,
            params::MIX => self.value(params::MIX) * 0.01,
            params::OUTPUT => util::db_to_amp(self.value(params::OUTPUT)),
            params::DIFFUSION => {
                let percent = self.values[params::DIFFUSION.0 as usize];
                if smooth {
                    self.angle.set_target(diffuse_angle(percent));
                } else {
                    self.angle.set_immediate(diffuse_angle(percent));
                }
                diffusion_gain(percent)
            }
            params::FREEZE => self.freeze_target(),
            params::LOW_CUT | params::HIGH_CUT => {
                self.update_filter(smooth);
                return;
            }
            _ => {
                // Size / pre-delay glide in `render`.
                if !smooth {
                    self.snap = true;
                }
                return;
            }
        };
        let smoother = match id {
            params::DECAY => &mut self.decay,
            params::DAMPING => &mut self.damping,
            params::WIDTH => &mut self.width,
            params::DIFFUSION => &mut self.diffusion,
            params::FREEZE => &mut self.freeze,
            params::OUTPUT => &mut self.output,
            _ => &mut self.mix,
        };
        if smooth {
            smoother.set_target(v);
        } else {
            smoother.set_immediate(v);
        }
    }

    fn update_gains(&mut self) {
        let rt60 = self.decay.current().max(0.01);
        let scale = self.scale() * self.size;
        for (g, base) in self.gains.iter_mut().zip(BASE_LENGTHS) {
            let seconds = base * scale / self.sample_rate;
            *g = 10f32.powf(-3.0 * seconds / rt60);
        }
    }

    fn render(&mut self, audio: &mut AudioBuffers<'_, '_>, start: usize, end: usize) {
        let (size_t, pre_t) = (self.target_size(), self.target_pre());
        if self.snap {
            self.size = size_t;
            self.pre = pre_t;
            self.snap = false;
            self.control_countdown = 0;
        }
        let inputs = audio.inputs;
        let input = |c: usize, i: usize| {
            inputs
                .get(c)
                .or_else(|| inputs.first())
                .map_or(0.0, |ch| ch[i])
        };
        let scale = self.scale();
        for i in start..end {
            self.size = size_t + (self.size - size_t) * self.glide_coef;
            self.pre = pre_t + (self.pre - pre_t) * self.glide_coef;
            self.decay.tick();
            if self.control_countdown == 0 {
                self.update_gains();
                self.control_countdown = CONTROL_INTERVAL;
            }
            self.control_countdown -= 1;
            let damp = self.damping.tick();
            let diffusion = self.diffusion.tick();
            let (sin, cos) = self.angle.tick().sin_cos();
            let freeze = self.freeze.tick();
            let filtered = !self.filter.is_bypass();
            if filtered {
                self.filter.glide();
            }

            let dry = [input(0, i), input(1, i)];
            // Pre-delay (0 = straight through), input filters and diffusion.
            let mut feed = [0.0f32; 2];
            for c in 0..2 {
                let line = &mut self.pre_delay[c];
                line.push(dry[c]);
                let mut x = if self.pre < 1.0 {
                    dry[c]
                } else {
                    line.read(self.pre + 1.0)
                };
                if filtered {
                    x = self.filter.tick(c, x);
                }
                let [a, b] = &mut self.diffusers[c];
                feed[c] = b.tick(a.tick(x, diffusion), diffusion);
            }
            // Multichannel diffusion: 4 lines' worth of echoes per channel.
            let mut lanes = [[0.0f32; DIFF_CH]; 2];
            // A quarter of a stage's span (each channel delays within its own quarter).
            let quarter = scale * self.size * 0.25;
            for (c, lane) in lanes.iter_mut().enumerate() {
                *lane = [feed[c]; DIFF_CH];
                for (st, stage) in self.multi[c].iter_mut().enumerate() {
                    let span = DIFF_SPANS[st] * quarter;
                    for (k, line) in stage.iter_mut().enumerate() {
                        let d = (span * (k as f32 + DIFF_POS[c][st][k])).max(1.0);
                        let y = line.read(d);
                        line.push(lane[k]);
                        lane[k] = y * DIFF_FLIPS[st][k];
                    }
                    diffuse_mix(lane, cos, sin);
                }
                if freeze > 0.0 {
                    // Frozen: the tank stops taking input.
                    lane.iter_mut().for_each(|v| *v *= 1.0 - freeze);
                }
            }

            // FDN.
            let mut o = [0.0f32; LINES];
            let mut wet = [0.0f32; 2];
            let mut level = lanes
                .iter()
                .flatten()
                .chain(&feed)
                .fold(0.0f32, |m, v| m.max(v.abs()));
            for (k, line) in self.lines.iter().enumerate() {
                let y = line.read(BASE_LENGTHS[k] * scale * self.size);
                o[k] = y;
                wet[0] += y * OUT_L[k];
                wet[1] += y * OUT_R[k];
                level = level.max(y.abs());
            }
            self.loud |= level >= util::SILENCE_FLOOR;
            if freeze > 0.0 {
                // Frozen: unity loop gain and no damping (the tail sustains).
                let damp = damp * (1.0 - freeze);
                for ((v, lp), g) in o.iter_mut().zip(&mut self.lp).zip(self.gains) {
                    *lp = flush32(*v * (1.0 - damp) + *lp * damp);
                    *v = *lp * (g + (1.0 - g) * freeze);
                }
            } else {
                for ((v, lp), g) in o.iter_mut().zip(&mut self.lp).zip(self.gains) {
                    *lp = flush32(*v * (1.0 - damp) + *lp * damp);
                    *v = *lp * g;
                }
            }
            hadamard8(&mut o);
            for (k, line) in self.lines.iter_mut().enumerate() {
                line.push(o[k] + lanes[k & 1][k >> 1]);
            }

            let width = self.width.tick();
            let mix = self.mix.tick();
            let trim = self.output.tick();
            let (wl, wr) = (wet[0] * WET_GAIN * trim, wet[1] * WET_GAIN * trim);
            let mid = (wl + wr) * 0.5;
            let side = (wl - wr) * 0.5 * width;
            let out = [
                dry[0] * (1.0 - mix) + (mid + side) * mix,
                dry[1] * (1.0 - mix) + (mid - side) * mix,
            ];
            for (c, ch) in audio.outputs.iter_mut().enumerate() {
                ch[i] = if c < 2 { out[c] } else { 0.0 };
            }
        }
    }
}

impl Node for Reverb {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.allocate();
    }

    fn reset(&mut self) {
        self.pre_delay.iter_mut().for_each(DelayLine::clear);
        for pair in &mut self.diffusers {
            pair.iter_mut().for_each(|a| a.line.clear());
        }
        self.multi
            .iter_mut()
            .flatten()
            .flatten()
            .for_each(DelayLine::clear);
        self.lines.iter_mut().for_each(DelayLine::clear);
        self.lp = [0.0; LINES];
        self.filter.reset();
        self.snap = true;
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        self.loud = false;
        util::split_at_events(
            self,
            ctx.events,
            ctx.frames,
            |s, a, b| s.render(audio, a, b),
            |s, kind| {
                if let EventKind::Param { param, value } = *kind {
                    s.apply_param(param, value, true);
                }
            },
        );
        // Watch the tank (not just the output: a dry-only mix would hide it) and wait out
        // the longest delay path; settled once every glide arrived.
        let n = ctx.frames;
        let silent_in = util::all_zero(audio.inputs, n);
        let quiet = silent_in
            && !self.loud
            && util::below_floor(audio.outputs.iter().map(|o| &o[..n.min(o.len())]));
        let settled = (self.size - self.target_size()).abs() < 1e-6
            && (self.pre - self.target_pre()).abs() < 1e-3
            && ![
                &self.decay,
                &self.damping,
                &self.width,
                &self.mix,
                &self.output,
                &self.diffusion,
                &self.angle,
                &self.freeze,
            ]
            .iter()
            .any(|s| s.is_smoothing())
            // A frozen tail never dies out.
            && self.freeze.current() == 0.0;
        self.tail.status(
            !ctx.events.is_empty(),
            silent_in,
            quiet,
            settled,
            n,
            self.hold,
        )
    }
}

impl Device for Reverb {
    fn descriptor(&self) -> DeviceDescriptor {
        descriptor()
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.values.get(id.0 as usize).copied()
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.apply_param(id, value, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hadamard_is_orthogonal() {
        let mut x = [1.0, 2.0, -3.0, 0.5, 0.0, 4.0, -1.0, 2.5];
        let e0: f32 = x.iter().map(|v| v * v).sum();
        hadamard8(&mut x);
        let e1: f32 = x.iter().map(|v| v * v).sum();
        assert!((e0 - e1).abs() < 1e-4);
        hadamard8(&mut x);
        assert!((x[2] + 3.0).abs() < 1e-5, "involution");
    }
}

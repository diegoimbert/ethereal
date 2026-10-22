//! Shifter: a pitch shifter, a frequency shifter and a ring modulator in one device (in the
//! spirit of Live's Shifter).
//!
//! - **Pitch** mode: a time-domain (two-tap delay-line) pitch shifter. Each channel writes a
//!   short delay line read by two taps half a window apart; their delay sweeps across
//!   `Window` at the speed that transposes by `Pitch` + `Detune` (a tap read faster than the
//!   line fills rises in pitch), and jumps back when it reaches the end. Each tap is
//!   weighted by `sin²` of its position in the sweep, so the jump happens at zero weight and
//!   the two weights always sum to one. Long windows suit low notes (fewer, smoother
//!   splices), short ones high notes and drums (less doubling). No latency is reported: the
//!   taps sit `Window / 2` behind the input on average (like a hardware shifter).
//! - **Frequency** mode: single-sideband frequency shifting. A pair of IIR allpass chains
//!   (four 2nd-order sections each, Olli Niemitalo's coefficients) puts the input in
//!   quadrature (an analytic signal); multiplying it by a complex carrier at `Shift` +
//!   `Fine` Hz and keeping the real part moves every partial by that many Hz (not a ratio:
//!   harmonics stop being harmonic, metallic at large shifts, a phaser-like swirl at a few
//!   Hz).
//! - **Ring** mode: the input multiplied by a cosine at `Carrier` + `Fine` Hz (both
//!   sidebands; at a few Hz with some dry signal, a tremolo). `Drive` soft-clips the
//!   modulated signal after `Amount` dB of gain.
//! - `Wide` mirrors the right channel: the opposite `Detune` (pitch), the opposite shift
//!   (frequency), a carrier 90° apart (ring: the tremolo alternates between the sides).
//! - `Mix` blends the dry signal (linear). A mode change crossfades over 10 ms, a `Window`
//!   change over 20 ms (from the old sweep to the new one); every continuous
//!   param glides.
//!
//! The delay lines are allocated in `prepare`; `process` allocates nothing.
//!
//! # Params (`BuiltinDeviceType::Shifter`, stable and append-only)
//!
//! | id | group | name | range |
//! |----|-------|------|-------|
//! | 0 | Mode | `Mode` | Pitch / Frequency / Ring, default Pitch |
//! | 1 | Pitch | `Pitch` | -24 ..= 24 Semitones (whole steps on the knob), default 0 |
//! | 2 | Pitch | `Detune` | -100 ..= 100 Cents, default 0 |
//! | 3 | Pitch | `Window` | 10 ..= 350 Milliseconds (log), default 80 |
//! | 4 | Frequency | `Shift` | -5000 ..= 5000 Hertz, default 0 |
//! | 5 | Frequency | `Fine` | -50 ..= 50 Hertz (adds to `Shift` and `Carrier`), default 0 |
//! | 6 | Ring | `Carrier` | 1 ..= 10000 Hertz (log), default 100 |
//! | 7 | Ring | `Drive` | toggle, default off |
//! | 8 | Ring | `Amount` | 0 ..= 24 Decibels (drive gain), default 6 |
//! | 9 | Output | `Wide` | toggle, default off |
//! | 10 | Output | `Mix` | 0 ..= 100 Percent, default 100 |

use std::f64::consts::{PI, TAU};

use ether_core::protocol::devices::{
    DeviceCategory, DeviceDescriptor, ParamInfo, ParamScale, ParamUnit,
};
use ether_core::protocol::layout::{DeviceLayout, Widget, WidgetSize};
use ether_core::protocol::model::{BuiltinDevice, BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus, Smoother,
};

use crate::contract::{
    FactoryPreset, choice, descriptor as build, item, knob, layout, param, section, toggle,
};
use crate::dsp::flush;
use crate::util::{self, split_at_events};

/// Param ids (stable, append-only).
pub mod params {
    use ether_core::protocol::model::ParamId;
    pub const MODE: ParamId = ParamId(0);
    pub const PITCH: ParamId = ParamId(1);
    pub const DETUNE: ParamId = ParamId(2);
    pub const WINDOW: ParamId = ParamId(3);
    pub const SHIFT: ParamId = ParamId(4);
    pub const FINE: ParamId = ParamId(5);
    pub const CARRIER: ParamId = ParamId(6);
    pub const DRIVE: ParamId = ParamId(7);
    pub const DRIVE_AMOUNT: ParamId = ParamId(8);
    pub const WIDE: ParamId = ParamId(9);
    pub const MIX: ParamId = ParamId(10);
    /// Number of params.
    pub const COUNT: usize = 11;
}
use params as ids;

/// `Mode` values.
pub mod mode {
    pub const PITCH: usize = 0;
    pub const FREQUENCY: usize = 1;
    pub const RING: usize = 2;
}

/// `Mode` labels.
pub const MODES: [&str; 3] = ["Pitch", "Frequency", "Ring"];
/// Longest `Window` (ms).
pub const MAX_WINDOW_MS: f64 = 350.0;
/// Mode-change crossfade (ms).
const XFADE_MS: f64 = 10.0;
/// Glide time constant of the continuous params (ms).
const GLIDE_MS: f64 = 15.0;
/// Crossfade of a `Window` change (ms).
const WINDOW_FADE_MS: f64 = 20.0;
/// Samples kept between a read and the write head (the Hermite read's look-ahead).
const GUARD: f64 = 3.0;

/// Descriptor of the shifter.
pub fn descriptor(ty: BuiltinDeviceType) -> DeviceDescriptor {
    assert_eq!(ty, BuiltinDeviceType::Shifter);
    let mut d = build(
        BuiltinDeviceType::Shifter,
        "Shifter",
        DeviceCategory::AudioEffect,
        vec![
            choice(0, "Mode", "Mode", &MODES, mode::PITCH),
            ParamInfo {
                step: Some(1.0),
                ..param(
                    1,
                    "Pitch",
                    "Pitch",
                    ParamUnit::Semitones,
                    (-24.0, 24.0, 0.0),
                    ParamScale::Linear,
                )
            },
            param(
                2,
                "Detune",
                "Pitch",
                ParamUnit::Cents,
                (-100.0, 100.0, 0.0),
                ParamScale::Linear,
            ),
            param(
                3,
                "Window",
                "Pitch",
                ParamUnit::Milliseconds,
                (10.0, MAX_WINDOW_MS, 80.0),
                ParamScale::Log,
            ),
            param(
                4,
                "Shift",
                "Frequency",
                ParamUnit::Hertz,
                (-5000.0, 5000.0, 0.0),
                ParamScale::Linear,
            ),
            param(
                5,
                "Fine",
                "Frequency",
                ParamUnit::Hertz,
                (-50.0, 50.0, 0.0),
                ParamScale::Linear,
            ),
            param(
                6,
                "Carrier",
                "Ring",
                ParamUnit::Hertz,
                (1.0, 10000.0, 100.0),
                ParamScale::Log,
            ),
            toggle(7, "Drive", "Ring", false),
            param(
                8,
                "Amount",
                "Ring",
                ParamUnit::Decibels,
                (0.0, 24.0, 6.0),
                ParamScale::Linear,
            ),
            toggle(9, "Wide", "Output", false),
            param(
                10,
                "Mix",
                "Output",
                ParamUnit::Percent,
                (0.0, 100.0, 100.0),
                ParamScale::Linear,
            ),
        ],
        2,
        2,
        false,
        0,
    );
    d.layout = Some(device_layout());
    d
}

/// The panel: the mode, one section per mode, the output.
fn device_layout() -> DeviceLayout {
    use WidgetSize::*;
    layout(vec![
        section(
            "mode",
            Some("Mode"),
            1,
            1,
            vec![
                item(Widget::Choice { param: ids::MODE }, Medium),
                item(Widget::Toggle { param: ids::WIDE }, Small),
            ],
        ),
        section(
            "pitch",
            Some("Pitch"),
            1,
            3,
            vec![
                knob(ids::PITCH, Large),
                knob(ids::DETUNE, Medium),
                knob(ids::WINDOW, Medium),
            ],
        ),
        section(
            "frequency",
            Some("Frequency"),
            1,
            2,
            vec![knob(ids::SHIFT, Large), knob(ids::FINE, Medium)],
        ),
        section(
            "ring",
            Some("Ring"),
            1,
            3,
            vec![
                knob(ids::CARRIER, Large),
                item(Widget::Toggle { param: ids::DRIVE }, Small),
                knob(ids::DRIVE_AMOUNT, Medium),
            ],
        ),
        section("output", Some("Output"), 1, 1, vec![knob(ids::MIX, Large)]),
    ])
}

/// Non-RT. A new instance.
pub fn create(device: &BuiltinDevice) -> Box<dyn Device> {
    assert_eq!(device.device_type(), BuiltinDeviceType::Shifter);
    Box::new(Shifter::new())
}

/// `FactoryPreset` entries for `presets/shifter/<slug>.etherpreset` files.
macro_rules! presets {
    ($($slug:literal),* $(,)?) => {
        &[$(FactoryPreset {
            id: concat!("shifter/", $slug),
            json: include_str!(concat!("../presets/shifter/", $slug, ".etherpreset")),
        }),*]
    };
}

/// Factory presets (embedded).
pub fn factory_presets(ty: BuiltinDeviceType) -> &'static [FactoryPreset] {
    match ty {
        BuiltinDeviceType::Shifter => presets!(
            "octave-up",
            "fifth-harmony",
            "wide-detune",
            "barber-pole-swirl",
            "metallic-shift",
            "ring-tremolo",
            "driven-ring"
        ),
        _ => &[],
    }
}

/// Plain values of the params, clamped (NaN = default).
struct Values {
    v: [f64; ids::COUNT],
    range: [(f64, f64, f64); ids::COUNT],
}

impl Values {
    fn new() -> Self {
        let d = descriptor(BuiltinDeviceType::Shifter);
        let mut s = Self {
            v: [0.0; ids::COUNT],
            range: [(0.0, 0.0, 0.0); ids::COUNT],
        };
        for (i, p) in d.params.iter().enumerate() {
            assert_eq!(p.id, ParamId(i as u32));
            s.v[i] = p.default;
            s.range[i] = (p.min, p.max, p.default);
        }
        s
    }

    fn set(&mut self, id: ParamId, value: f64) -> bool {
        let i = id.0 as usize;
        let Some(&(lo, hi, def)) = self.range.get(i) else {
            return false;
        };
        self.v[i] = if value.is_nan() {
            def
        } else {
            value.clamp(lo, hi)
        };
        true
    }

    fn f(&self, id: ParamId) -> f64 {
        self.v[id.0 as usize]
    }

    fn on(&self, id: ParamId) -> bool {
        self.f(id) >= 0.5
    }

    fn mode(&self) -> usize {
        (self.f(ids::MODE).round().max(0.0) as usize).min(MODES.len() - 1)
    }
}

/// A one-pole glide toward a target (f64; exact once within `eps`).
#[derive(Clone, Copy)]
struct Glide {
    current: f64,
    target: f64,
    coef: f64,
    eps: f64,
}

impl Glide {
    fn new(v: f64, eps: f64) -> Self {
        Self {
            current: v,
            target: v,
            coef: 0.0,
            eps,
        }
    }

    fn set_time(&mut self, ms: f64, sr: f64) {
        self.coef = (-1.0 / (ms * 0.001 * sr).max(1.0)).exp();
    }

    #[inline]
    fn tick(&mut self) -> f64 {
        if self.current != self.target {
            self.current = self.target + (self.current - self.target) * self.coef;
            if (self.current - self.target).abs() < self.eps {
                self.current = self.target;
            }
        }
        self.current
    }

    fn settled(&self) -> bool {
        self.current == self.target
    }
}

/// Niemitalo's 90° phase-difference allpass pair: the two chains' outputs differ by 90°
/// (± 0.7°) from ~20 Hz to ~23.9 kHz at 48 kHz. Each coefficient is `a` of a section
/// `y[n] = a² (x[n] + y[n-2]) - x[n-2]`.
const HILBERT_A: [f64; 4] = [
    0.692_387_8,
    0.936_065_432_295_9,
    0.988_229_522_686,
    0.998_748_845_273_7,
];
const HILBERT_B: [f64; 4] = [
    0.402_192_116_242_6,
    0.856_171_088_242,
    0.972_290_954_565_1,
    0.995_288_479_127_8,
];

/// One allpass chain (4 sections, each with two samples of input and output history).
#[derive(Clone, Copy, Default)]
struct AllpassChain {
    x: [[f64; 2]; 4],
    y: [[f64; 2]; 4],
}

impl AllpassChain {
    #[inline]
    fn tick(&mut self, coefs: &[f64; 4], input: f64) -> f64 {
        let mut v = input;
        for (k, a) in coefs.iter().enumerate() {
            let a2 = a * a;
            let y = flush(a2 * (v + self.y[k][1]) - self.x[k][1]);
            self.x[k][1] = self.x[k][0];
            self.x[k][0] = v;
            self.y[k][1] = self.y[k][0];
            self.y[k][0] = y;
            v = y;
        }
        v
    }
}

/// Analytic signal of one channel: `(re, im)` with `im` 90° ahead of `re` (so `re·cos + im·sin` moves it up).
#[derive(Clone, Copy, Default)]
struct Hilbert {
    a: AllpassChain,
    b: AllpassChain,
    /// Chain A's previous output (it is delayed by one sample).
    a_prev: f64,
}

impl Hilbert {
    #[inline]
    fn tick(&mut self, x: f64) -> (f64, f64) {
        let re = self.a_prev;
        self.a_prev = self.a.tick(&HILBERT_A, x);
        let im = self.b.tick(&HILBERT_B, x);
        (re, im)
    }
}

/// A sweep of the two taps: tap 1's position in `[0, 1)` (tap 2 is half a sweep ahead)
/// across `window` samples.
#[derive(Clone, Copy)]
struct Sweep {
    phase: f64,
    window: f64,
}

/// The two-tap delay-line pitch shifter of one channel. A `Window` change crossfades from
/// the old sweep to a new one (moving the taps' delay instead would bend the pitch).
struct PitchTaps {
    line: Vec<f32>,
    write: usize,
    sweep: Sweep,
    /// The sweep being faded out, `fade` of `fade_len` samples left.
    old: Sweep,
    fade: u32,
    fade_len: u32,
}

impl PitchTaps {
    fn new() -> Self {
        let sweep = Sweep {
            phase: 0.0,
            window: 0.0,
        };
        Self {
            line: Vec::new(),
            write: 0,
            sweep,
            old: sweep,
            fade: 0,
            fade_len: 1,
        }
    }

    /// Non-RT. Size the line for `sr` and start at `window` samples.
    fn allocate(&mut self, sr: f64, window: f64) {
        let n = (MAX_WINDOW_MS * 0.001 * sr).ceil() as usize + 16;
        if self.line.len() != n {
            self.line = vec![0.0; n];
        }
        self.fade_len = ((WINDOW_FADE_MS * 0.001 * sr).round() as u32).max(1);
        self.sweep.window = window;
        self.reset();
    }

    fn reset(&mut self) {
        self.line.fill(0.0);
        self.write = 0;
        self.sweep.phase = 0.0;
        self.old = self.sweep;
        self.fade = 0;
    }

    /// Advance `s` for `ratio` and read its two taps.
    #[inline]
    fn taps(&self, s: &mut Sweep, ratio: f64) -> f32 {
        // The delay changes by `1 - ratio` per sample: a sweep of `window` samples.
        s.phase -= (ratio - 1.0) / s.window;
        s.phase -= s.phase.floor();
        let max = (self.line.len() as f64) - GUARD - 3.0;
        let p2 = (s.phase + 0.5).fract();
        // Tap 1 weighs sin²(πp), tap 2 sin²(π(p + ½)) = 1 - sin²(πp).
        let w = (PI * s.phase).sin();
        let w1 = (w * w) as f32;
        let a = self.read((GUARD + s.phase * s.window).min(max));
        let b = self.read((GUARD + p2 * s.window).min(max));
        b + (a - b) * w1
    }

    /// 4-point Hermite read `d` (≥ 1) samples behind the newest sample.
    #[inline]
    fn read(&self, d: f64) -> f32 {
        let line = &self.line;
        let n = line.len();
        let i = d as usize;
        let t = (d - i as f64) as f32;
        // Index of the sample `i - 1` behind the newest, then walk back (`d` < n - 3).
        let mut k = self.write + n - (i - 1);
        if k >= n {
            k -= n;
        }
        let back = |k: usize| if k == 0 { n - 1 } else { k - 1 };
        let xm1 = line[k];
        let k = back(k);
        let x0 = line[k];
        let k = back(k);
        let x1 = line[k];
        let x2 = line[back(k)];
        let c1 = 0.5 * (x1 - xm1);
        let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
        let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
        ((c3 * t + c2) * t + c1) * t + x0
    }

    /// Write the next input sample (every sample, whatever the mode).
    #[inline]
    fn push(&mut self, x: f32) {
        self.write += 1;
        if self.write == self.line.len() {
            self.write = 0;
        }
        self.line[self.write] = if x.abs() < 1e-20 { 0.0 } else { x };
    }

    /// Read at `ratio` (after [`Self::push`]); a new `window` (samples) starts a
    /// crossfade once the previous one is over.
    #[inline]
    fn pitched(&mut self, ratio: f64, window: f64) -> f32 {
        if self.fade == 0 && window != self.sweep.window {
            self.old = self.sweep;
            self.sweep.window = window;
            self.fade = self.fade_len;
        }
        let mut sweep = self.sweep;
        let mut out = self.taps(&mut sweep, ratio);
        self.sweep = sweep;
        if self.fade > 0 {
            self.fade -= 1;
            let mut old = self.old;
            let o = self.taps(&mut old, ratio);
            self.old = old;
            // Equal-gain: the two sweeps read the same signal at nearby delays.
            let t = self.fade as f32 / self.fade_len as f32;
            out += (o - out) * t;
        }
        out
    }

    fn settled(&self) -> bool {
        self.fade == 0
    }
}

/// The shifter device.
pub struct Shifter {
    values: Values,
    sample_rate: f64,
    taps: [PitchTaps; 2],
    hilbert: [Hilbert; 2],
    /// Carrier phase in cycles, `[0, 1)` (frequency and ring modes).
    phase: f64,
    /// Current mode and the one being faded out (`fade` samples left of `fade_len`).
    mode: usize,
    prev_mode: usize,
    fade: u32,
    fade_len: u32,
    semitones: Glide,
    detune: Glide,
    /// `Window` in samples.
    window: f64,
    shift: Glide,
    carrier: Glide,
    drive: Smoother,
    mix: Smoother,
    tail: util::Tail,
}

impl Default for Shifter {
    fn default() -> Self {
        Self::new()
    }
}

impl Shifter {
    /// Non-RT. A shifter with default params (lines allocated for 48 kHz).
    pub fn new() -> Self {
        let mut s = Self {
            values: Values::new(),
            sample_rate: 48_000.0,
            taps: [PitchTaps::new(), PitchTaps::new()],
            hilbert: [Hilbert::default(); 2],
            phase: 0.0,
            mode: mode::PITCH,
            prev_mode: mode::PITCH,
            fade: 0,
            fade_len: 1,
            semitones: Glide::new(0.0, 1e-6),
            detune: Glide::new(0.0, 1e-4),
            window: 80.0 * 48.0,
            shift: Glide::new(0.0, 1e-4),
            carrier: Glide::new(100.0, 1e-4),
            drive: Smoother::new(0.0, 0.0, 48_000.0),
            mix: Smoother::new(1.0, 0.0, 48_000.0),
            tail: util::Tail::default(),
        };
        s.configure();
        s
    }

    fn configure(&mut self) {
        let sr = self.sample_rate;
        for g in [
            &mut self.semitones,
            &mut self.detune,
            &mut self.shift,
            &mut self.carrier,
        ] {
            g.set_time(GLIDE_MS, sr);
        }
        self.fade_len = ((XFADE_MS * 0.001 * sr).round() as u32).max(1);
        for i in 0..ids::COUNT {
            self.apply(ParamId(i as u32), false);
        }
        let window = self.window.max(8.0);
        for t in &mut self.taps {
            t.allocate(sr, window);
        }
    }

    fn apply(&mut self, id: ParamId, smooth: bool) {
        let v = self.values.f(id);
        let glide = |g: &mut Glide, target: f64| {
            g.target = target;
            if !smooth {
                g.current = target;
            }
        };
        let ramp = |s: &mut Smoother, v: f32| {
            if smooth {
                s.ramp_param(v, ether_core::automation_rt::PARAM_GRID as u32);
            } else {
                s.set_immediate(v);
            }
        };
        match id {
            ids::MODE => {
                let m = self.values.mode();
                if m != self.mode {
                    if smooth {
                        self.prev_mode = self.mode;
                        self.fade = self.fade_len;
                    }
                    if m == mode::FREQUENCY {
                        // Idle since it was last used: start from silence, not stale state.
                        self.hilbert = [Hilbert::default(); 2];
                    }
                    self.mode = m;
                }
            }
            ids::PITCH => glide(&mut self.semitones, v),
            ids::DETUNE => glide(&mut self.detune, v * 0.01),
            ids::WINDOW => self.window = v * 0.001 * self.sample_rate,
            ids::SHIFT | ids::FINE | ids::CARRIER => {
                let fine = self.values.f(ids::FINE);
                glide(&mut self.shift, self.values.f(ids::SHIFT) + fine);
                // A negative carrier is the same carrier (cos is even).
                glide(
                    &mut self.carrier,
                    (self.values.f(ids::CARRIER) + fine).abs(),
                );
            }
            ids::DRIVE | ids::DRIVE_AMOUNT => {
                let g = if self.values.on(ids::DRIVE) {
                    util::db_to_amp(self.values.f(ids::DRIVE_AMOUNT) as f32)
                } else {
                    0.0
                };
                ramp(&mut self.drive, g);
            }
            ids::MIX => ramp(&mut self.mix, (v * 0.01) as f32),
            _ => {}
        }
    }

    fn set(&mut self, id: ParamId, value: f64, smooth: bool) {
        if self.values.set(id, value) {
            self.apply(id, smooth);
        }
    }

    /// The wet output of mode `m` for this sample. `x`: input, `ratio`: pitch ratios per
    /// channel, `taps`: the pitch taps' outputs, `ana`: analytic signals, `cs`: the
    /// carrier's `(cos, sin)`, `drive`: the drive gain (0 = off).
    #[allow(clippy::too_many_arguments)]
    #[inline]
    fn wet(
        m: usize,
        c: usize,
        wide: bool,
        x: f64,
        taps: f32,
        ana: (f64, f64),
        cs: (f64, f64),
        drive: f32,
    ) -> f32 {
        let right = c == 1 && wide;
        match m {
            mode::FREQUENCY => {
                // re·cos(ωt) + im·sin(ωt): every partial up by the shift (down on a wide right).
                let (cos, sin) = cs;
                let s = if right { -sin } else { sin };
                (ana.0 * cos + ana.1 * s) as f32
            }
            mode::RING => {
                let (cos, sin) = cs;
                let y = (x * if right { sin } else { cos }) as f32;
                if drive > 0.0 { (drive * y).tanh() } else { y }
            }
            _ => taps,
        }
    }

    fn render(&mut self, audio: &mut AudioBuffers<'_, '_>, start: usize, end: usize) {
        let n_ch = audio.outputs.len().min(2);
        let sr = self.sample_rate;
        let wide = self.values.on(ids::WIDE);
        for i in start..end {
            let semis = self.semitones.tick();
            let detune = self.detune.tick();
            let window = self.window.max(8.0);
            let shift = self.shift.tick();
            let carrier = self.carrier.tick();
            let drive = self.drive.tick();
            let mix = self.mix.tick();
            let fade = if self.fade > 0 {
                self.fade -= 1;
                self.fade as f32 / self.fade_len as f32
            } else {
                0.0
            };
            // Only the modes heard run (the pitch lines are always written).
            let uses = |m: usize| self.mode == m || (fade > 0.0 && self.prev_mode == m);
            let (pitch, freq, ring) = (uses(mode::PITCH), uses(mode::FREQUENCY), uses(mode::RING));
            // The frequency mode's carrier is `shift`, the ring mode's `carrier`: one phase
            // (both run only during the 10 ms of a fade between them).
            let (sin, cos) = if freq || ring {
                let hz = if self.mode == mode::RING {
                    carrier
                } else {
                    shift
                };
                let sc = (TAU * self.phase).sin_cos();
                self.phase += hz / sr;
                self.phase -= self.phase.floor();
                sc
            } else {
                (0.0, 1.0)
            };
            let ratio = ((semis + detune) / 12.0).exp2();
            let ratio_r = if wide && pitch {
                ((semis - detune) / 12.0).exp2()
            } else {
                ratio
            };
            for c in 0..n_ch {
                let x = audio
                    .inputs
                    .get(c)
                    .or(audio.inputs.first())
                    .map_or(0.0, |b| b[i]);
                let taps = &mut self.taps[c];
                taps.push(x);
                let taps = if pitch {
                    taps.pitched(if c == 1 { ratio_r } else { ratio }, window)
                } else {
                    0.0
                };
                let ana = if freq {
                    self.hilbert[c].tick(f64::from(x))
                } else {
                    (0.0, 0.0)
                };
                let xd = f64::from(x);
                let mut y = Self::wet(self.mode, c, wide, xd, taps, ana, (cos, sin), drive);
                if fade > 0.0 {
                    let old = Self::wet(self.prev_mode, c, wide, xd, taps, ana, (cos, sin), drive);
                    y += (old - y) * fade;
                }
                audio.outputs[c][i] = x + (y - x) * mix;
            }
        }
    }
}

impl Node for Shifter {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = f64::from(config.sample_rate.max(1.0));
        self.configure();
        self.reset();
    }

    fn reset(&mut self) {
        for t in &mut self.taps {
            t.reset();
        }
        self.hilbert = [Hilbert::default(); 2];
        self.phase = 0.0;
        self.fade = 0;
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
        let settled = !self.mix.is_smoothing()
            && !self.drive.is_smoothing()
            && self.fade == 0
            && self.semitones.settled()
            && self.taps.iter().all(PitchTaps::settled);
        // The pitch taps can hold a window of signal; the allpass chains ring briefly.
        let hold = self.taps[0].line.len()
            + util::samples(util::BASE_HOLD_SECONDS, self.sample_rate as f32);
        self.tail.status_of(ctx, audio, settled, hold)
    }
}

impl Device for Shifter {
    fn descriptor(&self) -> DeviceDescriptor {
        descriptor(BuiltinDeviceType::Shifter)
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.values.v.get(id.0 as usize).copied()
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.set(id, value, false);
    }
}

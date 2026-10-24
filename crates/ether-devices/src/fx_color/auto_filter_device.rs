//! Auto Filter: a zero-delay-feedback multimode filter swept by an envelope follower and an
//! LFO.
//!
//! - **Filters.** `Type` × `Slope`. Low/high-pass at 12 dB/oct, `Band-pass`, `Notch` and
//!   `Peak` are a trapezoidal (TPT) state-variable filter (A. Simper); low/high-pass at
//!   24 dB/oct a TPT 4-pole ladder (V. Zavalishin, "The Art of VA Filter Design") with the
//!   feedback solved without a unit delay; the high-pass taps the ladder stages
//!   (`u - 4y1 + 6y2 - 4y3 + y4`); at 48 dB/oct four cascaded SVF sections with the poles
//!   of an 8th-order Butterworth (flat passband, -3 dB at the cutoff), resonance raising the
//!   highest-Q section's Q by `max(1, 24^res / √2)` (about the 12 dB filter's peak above
//!   Butterworth). SVF Q follows the UI's curve: `Q = 0.5 · 24^res`; ladder
//!   feedback `k = 3.9 · res` (self-oscillation at the top). `Peak` is a bell of
//!   `6 + 12·res` dB. `Slope` only shapes the low- and high-pass. Cutoff moves in octaves, so every modulation is musical and the TPT
//!   structures stay stable while it moves every sample.
//! - **Drive** (0..24 dB) boosts the filter input into a `tanh` stage (on the ladder, in the
//!   feedback path, which also tames resonance); the output is compensated by half the
//!   drive in dB. At 0 dB the filter is linear; the saturation fades in over the first 3 dB.
//! - **Envelope follower**: a stereo-linked peak follower (`Attack`/`Release`) on the
//!   sidechain when the device has one ([`Node::process_sidechain`], latency-aligned),
//!   else on the input. Its level, mapped -48..0 dBFS → 0..1, moves the cutoff by
//!   `Amount × 6` octaves (negative = down).
//! - **LFO**: six shapes, free (`Rate` Hz) or tempo-synced (`Sync Rate`, phase locked to
//!   the song position while playing; free-running at the tempo while stopped), `Amount`
//!   = ±4 octaves at 100 %. `Stereo Phase` offsets the right channel (0..180°). A 1 ms
//!   one-pole softens the steps of square / sample & hold.
//! - A type or slope change between the SVF, the ladder and the cascade crossfades over 5 ms.
//! - **4x oversampled**: the filters and the drive run at four times the rate
//!   between the saturator's linear-phase half-band stages ([`super::oversample`]), so
//!   driven input folds far less back into the audio band (at 24 dB of drive on a 5 kHz
//!   sine: -17 dB at 1x, -35 dB at 2x, -60 dB at 4x). The device reports the round trip as
//!   its latency ([`super::oversample::LATENCY_4X`] samples) and delays the dry signal of
//!   `Mix` by it. Cutoffs are mapped to the 4x rate exactly (`tan(θ/2)` twice from
//!   `tan θ`).

use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{BuiltinDeviceType, ParamId};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus, Smoother,
    TransportInfo,
};

use super::auto_filter as ids;
use super::oversample::{self, LATENCY_4X, Stage1, Stage2};
use super::shared::{FixedDelay, Values, db, flush, glide, hash_bipolar};
use crate::contract::SYNC_RATE_BEATS;
use crate::util::{LateEvents, Route, tau_coef};

const SIDECHAIN_CHANNELS: usize = 2;
/// Octaves of cutoff movement at `Envelope Amount` ±100 %.
const ENV_OCTAVES: f32 = 6.0;
/// Envelope level range mapped to 0..1 (dBFS).
const ENV_FLOOR_DB: f32 = -48.0;
/// Octaves of cutoff movement (peak) at `LFO Amount` 100 %.
const LFO_OCTAVES: f32 = 4.0;
/// Ladder feedback at resonance 100 %.
const LADDER_K_MAX: f32 = 3.9;
const FADE_MS: f32 = 5.0;
const LFO_SMOOTH_MS: f32 = 1.0;

/// Effective filter: params `Type` × `Slope` (or `Type (legacy)` in an old document).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterType {
    LowPass12,
    LowPass24,
    LowPass48,
    HighPass12,
    HighPass24,
    HighPass48,
    BandPass,
    Notch,
    Peak,
}

/// Filter structure of a [`FilterType`] (each keeps its own state).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Topology {
    Svf,
    Ladder,
    Cascade,
}

/// Q of the four 2-pole sections of an 8th-order Butterworth, the highest last
/// (`1 / (2 sin((2k - 1)π / 16))`).
const BUTTERWORTH_8_Q: [f32; 4] = [0.509_795_6, 0.601_344_9, 0.899_976_2, 2.562_915_5];

impl FilterType {
    /// `Type (legacy)` order.
    const LEGACY: [Self; 7] = [
        Self::LowPass12,
        Self::LowPass24,
        Self::HighPass12,
        Self::HighPass24,
        Self::BandPass,
        Self::Notch,
        Self::Peak,
    ];

    /// From `Type`, `Slope` and `Type (legacy)` (indices): the legacy type decides while the
    /// other two are at their defaults.
    pub fn resolve(mode: usize, slope: usize, legacy: usize) -> Self {
        if mode == 0 && slope == 0 && legacy != 0 {
            return Self::LEGACY[legacy.min(6)];
        }
        match (mode.min(4), slope.min(2)) {
            (0, 0) => Self::LowPass12,
            (0, 1) => Self::LowPass24,
            (0, _) => Self::LowPass48,
            (1, 0) => Self::HighPass12,
            (1, 1) => Self::HighPass24,
            (1, _) => Self::HighPass48,
            (2, _) => Self::BandPass,
            (3, _) => Self::Notch,
            _ => Self::Peak,
        }
    }

    /// `Type` and `Slope` indices of this filter.
    pub fn mode_slope(self) -> (usize, usize) {
        match self {
            Self::LowPass12 => (0, 0),
            Self::LowPass24 => (0, 1),
            Self::LowPass48 => (0, 2),
            Self::HighPass12 => (1, 0),
            Self::HighPass24 => (1, 1),
            Self::HighPass48 => (1, 2),
            Self::BandPass => (2, 0),
            Self::Notch => (3, 0),
            Self::Peak => (4, 0),
        }
    }

    /// The filter of a param map (plain values; a missing param is at its default).
    pub fn of_params(params: &std::collections::BTreeMap<ParamId, f64>) -> Self {
        let idx = |id| params.get(&id).map_or(0, |v: &f64| v.round().max(0.0) as usize);
        Self::resolve(idx(ids::MODE), idx(ids::SLOPE), idx(ids::TYPE))
    }

    /// Rewrites an old document's `Type (legacy)` as `Type` + `Slope` (same sound), so the
    /// panel shows what plays. `None` when there is nothing to rewrite; else the `Type` index
    /// it set (for retargeting automation of the legacy param).
    pub fn upgrade_params(params: &mut std::collections::BTreeMap<ParamId, f64>) -> Option<usize> {
        let legacy = params.get(&ids::TYPE).map_or(0, |v| v.round().max(0.0) as usize);
        if legacy == 0 {
            return None;
        }
        let (mode, slope) = Self::of_params(params).mode_slope();
        params.insert(ids::TYPE, 0.0);
        params.insert(ids::MODE, mode as f64);
        params.insert(ids::SLOPE, slope as f64);
        Some(mode)
    }

    /// The `Type` index a `Type (legacy)` index stands for.
    pub fn legacy_mode(legacy: usize) -> usize {
        Self::LEGACY[legacy.min(6)].mode_slope().0
    }

    fn topology(self) -> Topology {
        match self {
            Self::LowPass24 | Self::HighPass24 => Topology::Ladder,
            Self::LowPass48 | Self::HighPass48 => Topology::Cascade,
            _ => Topology::Svf,
        }
    }
}

/// LFO shape (param `LFO Shape`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LfoShape {
    Sine,
    Triangle,
    SawUp,
    SawDown,
    Square,
    SampleHold,
}

impl LfoShape {
    fn from_index(i: usize) -> Self {
        [
            Self::Sine,
            Self::Triangle,
            Self::SawUp,
            Self::SawDown,
            Self::Square,
            Self::SampleHold,
        ][i.min(5)]
    }

    /// Value at `cycle + frac` (`frac` in `[0, 1)`), -1..1.
    #[inline]
    fn at(self, cycle: i64, frac: f64) -> f32 {
        let p = frac as f32;
        match self {
            Self::Sine => (2.0 * std::f32::consts::PI * p).sin(),
            Self::Triangle => {
                if p < 0.25 {
                    4.0 * p
                } else if p < 0.75 {
                    2.0 - 4.0 * p
                } else {
                    4.0 * p - 4.0
                }
            }
            Self::SawUp => 2.0 * p - 1.0,
            Self::SawDown => 1.0 - 2.0 * p,
            Self::Square => {
                if p < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Self::SampleHold => hash_bipolar(cycle),
        }
    }
}

/// Per-sample filter coefficients (shared by both topologies; each uses its part).
#[derive(Clone, Copy)]
struct Coefs {
    /// `tan(π fc / fs)`.
    g: f32,
    /// SVF damping `1/Q` (for the bell: `1/(Q·A)`).
    k: f32,
    /// Bell: `A² - 1`.
    bell: f32,
    /// Ladder feedback.
    fb: f32,
    /// Damping `1/Q` of the cascade's resonant (last) section.
    k_last: f32,
    /// Input drive gain, saturation blend, output compensation.
    drive: f32,
    sat: f32,
    comp: f32,
}

#[derive(Clone, Copy, Default)]
struct FilterState {
    ic1: f32,
    ic2: f32,
    s: [f32; 4],
    /// The cascade's four SVF sections (`ic1`, `ic2`).
    cas: [[f32; 2]; 4],
}

/// One TPT SVF step on `ic` (`[ic1, ic2]`): `(v1, v2)` (band, low).
#[inline]
fn svf_step(ic: &mut [f32; 2], v0: f32, g: f32, k: f32) -> (f32, f32) {
    let a1 = 1.0 / (1.0 + g * (g + k));
    let a2 = g * a1;
    let a3 = g * a2;
    let v3 = v0 - ic[1];
    let v1 = a1 * ic[0] + a2 * v3;
    let v2 = ic[1] + a2 * ic[0] + a3 * v3;
    ic[0] = flush(2.0 * v1 - ic[0]);
    ic[1] = flush(2.0 * v2 - ic[1]);
    (v1, v2)
}

#[inline]
fn saturate(x: f32, sat: f32) -> f32 {
    if sat <= 0.0 {
        x
    } else {
        x + (x.tanh() - x) * sat
    }
}

impl FilterState {
    fn reset_svf(&mut self) {
        self.ic1 = 0.0;
        self.ic2 = 0.0;
    }

    fn reset_ladder(&mut self) {
        self.s = [0.0; 4];
    }

    fn reset(&mut self, t: Topology) {
        match t {
            Topology::Svf => self.reset_svf(),
            Topology::Ladder => self.reset_ladder(),
            Topology::Cascade => self.cas = [[0.0; 2]; 4],
        }
    }

    #[inline]
    fn tick(&mut self, ty: FilterType, x: f32, c: &Coefs) -> f32 {
        let x = x * c.drive;
        let y = match ty.topology() {
            Topology::Ladder => self.ladder(ty, x, c),
            Topology::Svf => self.svf(ty, saturate(x, c.sat), c),
            Topology::Cascade => self.cascade(ty, saturate(x, c.sat), c),
        };
        y * c.comp
    }

    #[inline]
    fn svf(&mut self, ty: FilterType, v0: f32, c: &Coefs) -> f32 {
        let k = c.k;
        let mut ic = [self.ic1, self.ic2];
        let (v1, v2) = svf_step(&mut ic, v0, c.g, k);
        [self.ic1, self.ic2] = ic;
        match ty {
            FilterType::HighPass12 => v0 - k * v1 - v2,
            FilterType::BandPass => k * v1,
            FilterType::Notch => v0 - k * v1,
            FilterType::Peak => v0 + k * c.bell * v1,
            _ => v2,
        }
    }

    /// Four Butterworth SVF sections in series (8 poles), the resonant one last.
    #[inline]
    fn cascade(&mut self, ty: FilterType, v0: f32, c: &Coefs) -> f32 {
        let high = ty == FilterType::HighPass48;
        let mut v = v0;
        for (i, ic) in self.cas.iter_mut().enumerate() {
            let k = if i == 3 {
                c.k_last
            } else {
                1.0 / BUTTERWORTH_8_Q[i]
            };
            let (v1, v2) = svf_step(ic, v, c.g, k);
            v = if high { v - k * v1 - v2 } else { v2 };
        }
        v
    }

    #[inline]
    fn ladder(&mut self, ty: FilterType, x: f32, c: &Coefs) -> f32 {
        let g = c.g;
        let big_g = g / (1.0 + g);
        let inv = 1.0 / (1.0 + g);
        let b = [
            self.s[0] * inv,
            self.s[1] * inv,
            self.s[2] * inv,
            self.s[3] * inv,
        ];
        let g2 = big_g * big_g;
        let sum = g2 * big_g * b[0] + g2 * b[1] + big_g * b[2] + b[3];
        let k = c.fb;
        let mut u = (x - k * sum) / (1.0 + k * g2 * g2);
        u = saturate(u, c.sat);
        // Four TPT one-poles.
        let mut y = [0.0f32; 4];
        let mut inp = u;
        for (i, yi) in y.iter_mut().enumerate() {
            let v = (inp - self.s[i]) * big_g;
            *yi = v + self.s[i];
            self.s[i] = flush(*yi + v);
            inp = *yi;
        }
        match ty {
            // Partial passband compensation (the feedback costs 1/(1+k) at DC).
            FilterType::LowPass24 => y[3] * (1.0 + k).sqrt(),
            _ => u - 4.0 * y[0] + 6.0 * y[1] - 4.0 * y[2] + y[3],
        }
    }
}

/// The auto filter device.
pub struct AutoFilter {
    values: Values<{ ids::COUNT }>,
    sample_rate: f32,
    ty: FilterType,
    fade_from: FilterType,
    /// Indices of `Type`, `Slope`, `Type (legacy)`.
    sel: [usize; 3],
    fade_left: u32,
    fade_len: u32,
    /// log2 of the cutoff (Hz).
    cutoff: Smoother,
    res: Smoother,
    /// Drive in dB.
    drive: Smoother,
    env_amount: Smoother,
    att: f32,
    rel: f32,
    env: f32,
    lfo_amount: Smoother,
    lfo_shape: LfoShape,
    lfo_rate: f64,
    lfo_sync: bool,
    lfo_beats: f64,
    /// Right-channel phase offset (cycles).
    lfo_offset: Smoother,
    cycle: i64,
    frac: f64,
    lfo_lp: [f32; 2],
    lfo_lp_a: f32,
    mix: Smoother,
    output: Smoother,
    ch: [FilterState; 2],
    /// Per channel: the two 2x stages, the 2x-rate sample that pads the round trip to
    /// whole base samples, and the dry path delayed by it.
    os1: [Stage1; 2],
    os2: [Stage2; 2],
    pad: [f32; 2],
    dry: [FixedDelay<LATENCY_4X>; 2],
    /// Control rate: envelope/LFO cutoff modulation (log10, LFO shape,
    /// exp2, tan) once per [`CONTROL`] samples, `g` ramped linearly per channel in between.
    g: [f32; 2],
    g_step: [f32; 2],
    g_primed: bool,
    ctl_left: usize,
    /// `lfo_lp_a` over one control chunk.
    lfo_lp_a_ctl: f32,
    /// Envelope level the current chunk was computed with.
    env_ctl: f32,
    /// Resonance/drive-derived coefficients for the last (res, drive dB, peak) seen.
    common: Option<((u32, u32, bool), Coefs)>,
    /// Silence reporting.
    tail: crate::util::Tail,
    /// Mix and output act on the filtered signal, [`LATENCY_4X`] samples into the path;
    /// the filter's params where it sees the input (after the up-sampling filters).
    late: LateEvents,
}

/// Control-rate chunk (samples) of the cutoff modulation.
const CONTROL: usize = 16;

impl Default for AutoFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl AutoFilter {
    /// Non-RT. An auto filter with default params.
    pub fn new() -> Self {
        let desc = descriptor();
        let z = || Smoother::new(0.0, 0.0, 48_000.0);
        let mut s = Self {
            tail: crate::util::Tail::default(),
            late: LateEvents::new(),
            g: [0.0; 2],
            g_step: [0.0; 2],
            g_primed: false,
            ctl_left: 0,
            lfo_lp_a_ctl: 1.0,
            env_ctl: 0.0,
            common: None,
            values: Values::new(&desc),
            sample_rate: 48_000.0,
            ty: FilterType::LowPass12,
            fade_from: FilterType::LowPass12,
            sel: [0; 3],
            fade_left: 0,
            fade_len: 1,
            cutoff: z(),
            res: z(),
            drive: z(),
            env_amount: z(),
            att: 0.0,
            rel: 0.0,
            env: 0.0,
            lfo_amount: z(),
            lfo_shape: LfoShape::Sine,
            lfo_rate: 1.0,
            lfo_sync: false,
            lfo_beats: 1.0,
            lfo_offset: z(),
            cycle: 0,
            frac: 0.0,
            lfo_lp: [0.0; 2],
            lfo_lp_a: 1.0,
            mix: z(),
            output: z(),
            ch: [FilterState::default(); 2],
            os1: [oversample::stage1(); 2],
            os2: [oversample::stage2(); 2],
            pad: [0.0; 2],
            dry: [FixedDelay::new(); 2],
        };
        s.sync_all();
        s
    }

    fn sync_all(&mut self) {
        self.fade_len = ((FADE_MS * 0.001 * self.sample_rate) as u32).max(1);
        self.lfo_lp_a = 1.0 - (-1.0 / (LFO_SMOOTH_MS * 0.001 * self.sample_rate)).exp();
        self.lfo_lp_a_ctl = 1.0 - (1.0 - self.lfo_lp_a).powi(CONTROL as i32);
        for i in 0..ids::COUNT {
            self.apply(ParamId(i as u32), false);
        }
        self.fade_left = 0;
    }

    fn apply(&mut self, id: ParamId, smooth: bool) {
        let v = |s: &Self, id| s.values.f(id);
        match id {
            ids::TYPE | ids::MODE | ids::SLOPE => {
                let slot = match id {
                    ids::MODE => 0,
                    ids::SLOPE => 1,
                    _ => 2,
                };
                self.sel[slot] = self.values.index(id);
                let [mode, slope, legacy] = self.sel;
                let t = FilterType::resolve(mode, slope, legacy);
                if t != self.ty {
                    if smooth && t.topology() != self.ty.topology() {
                        for c in &mut self.ch {
                            c.reset(t.topology());
                        }
                        self.fade_from = self.ty;
                        self.fade_left = self.fade_len;
                    }
                    self.ty = t;
                }
            }
            ids::CUTOFF => {
                let t = v(self, ids::CUTOFF).log2();
                glide(&mut self.cutoff, t, smooth);
            }
            ids::RESONANCE => {
                let t = v(self, ids::RESONANCE) * 0.01;
                glide(&mut self.res, t, smooth);
            }
            ids::DRIVE => {
                let t = v(self, ids::DRIVE);
                glide(&mut self.drive, t, smooth);
            }
            ids::ENV_AMOUNT => {
                let t = v(self, ids::ENV_AMOUNT) * 0.01;
                glide(&mut self.env_amount, t, smooth);
            }
            ids::ENV_ATTACK => self.att = tau_coef(v(self, ids::ENV_ATTACK), self.sample_rate),
            ids::ENV_RELEASE => self.rel = tau_coef(v(self, ids::ENV_RELEASE), self.sample_rate),
            ids::LFO_AMOUNT => {
                let t = v(self, ids::LFO_AMOUNT) * 0.01;
                glide(&mut self.lfo_amount, t, smooth);
            }
            ids::LFO_SHAPE => self.lfo_shape = LfoShape::from_index(self.values.index(id)),
            ids::LFO_RATE => self.lfo_rate = self.values.get(id).unwrap_or(1.0),
            ids::LFO_SYNC => self.lfo_sync = self.values.on(id),
            ids::LFO_SYNC_RATE => {
                self.lfo_beats =
                    SYNC_RATE_BEATS[self.values.index(id).min(SYNC_RATE_BEATS.len() - 1)]
            }
            ids::LFO_PHASE => {
                let t = v(self, ids::LFO_PHASE) / 360.0;
                glide(&mut self.lfo_offset, t, smooth);
            }
            ids::MIX => {
                let t = v(self, ids::MIX) * 0.01;
                glide(&mut self.mix, t, smooth);
            }
            ids::OUTPUT => {
                let t = db(v(self, ids::OUTPUT));
                glide(&mut self.output, t, smooth);
            }
            _ => {}
        }
    }

    fn set(&mut self, id: ParamId, value: f64, smooth: bool) {
        if self.values.set(id, value) {
            self.apply(id, smooth);
        }
    }

    /// Advance the LFO to frame `i` of the block (free, or locked to the song position).
    #[inline]
    fn lfo_step(&mut self, transport: &TransportInfo, i: usize) {
        if self.lfo_sync && transport.playing {
            let beats = transport.position + i as f64 * transport.beats_per_sample;
            let cycles = beats / self.lfo_beats;
            let c = cycles.floor();
            self.cycle = c as i64;
            self.frac = cycles - c;
            return;
        }
        let hz = if self.lfo_sync {
            transport.bpm / 60.0 / self.lfo_beats
        } else {
            self.lfo_rate
        };
        self.frac += hz / f64::from(self.sample_rate);
        if self.frac >= 1.0 {
            let whole = self.frac.floor();
            self.frac -= whole;
            self.cycle = self.cycle.wrapping_add(whole as i64);
        }
    }

    /// LFO `(cycle, frac)` `n` samples after frame `i` (as `lfo_step` would get there).
    #[inline]
    fn lfo_ahead(&self, transport: &TransportInfo, i: usize, n: usize) -> (i64, f64) {
        if self.lfo_sync && transport.playing {
            let beats = transport.position + (i + n) as f64 * transport.beats_per_sample;
            let cycles = beats / self.lfo_beats;
            let c = cycles.floor();
            return (c as i64, cycles - c);
        }
        let hz = if self.lfo_sync {
            transport.bpm / 60.0 / self.lfo_beats
        } else {
            self.lfo_rate
        };
        let f = self.frac + hz / f64::from(self.sample_rate) * n as f64;
        let whole = f.floor();
        (self.cycle.wrapping_add(whole as i64), f - whole)
    }

    #[inline]
    fn lfo_value_at(&self, (cycle, frac): (i64, f64), offset: f32) -> f32 {
        let mut f = frac + f64::from(offset);
        let mut cycle = cycle;
        if f >= 1.0 {
            f -= 1.0;
            cycle = cycle.wrapping_add(1);
        }
        self.lfo_shape.at(cycle, f)
    }

    fn render(
        &mut self,
        transport: &TransportInfo,
        audio: &mut AudioBuffers<'_, '_>,
        sidechain: &[&[f32]],
        start: usize,
        end: usize,
    ) {
        let n_ch = audio.outputs.len().min(2);
        let nyq = (0.45 * self.sample_rate).min(20_000.0);
        let sr = self.sample_rate;
        for i in start..end {
            // Envelope follower (stereo-linked peak).
            let level = if sidechain.is_empty() {
                audio.inputs.iter().fold(0.0f32, |m, ch| m.max(ch[i].abs()))
            } else {
                sidechain.iter().fold(0.0f32, |m, ch| m.max(ch[i].abs()))
            };
            let coef = if level > self.env { self.att } else { self.rel };
            self.env = flush(level + (self.env - level) * coef);
            let env_amount = self.env_amount.tick();
            // LFO.
            self.lfo_step(transport, i);
            let lfo_amount = self.lfo_amount.tick();
            let offset = self.lfo_offset.tick();
            let cutoff = self.cutoff.tick();
            // Cutoff modulation once per control chunk; an envelope that
            // moved by more than 0.5 dB since (an onset) starts a new chunk right away.
            let env_moved = env_amount != 0.0
                && (self.env > self.env_ctl * 1.0593 || self.env * 1.0593 < self.env_ctl);
            if self.ctl_left == 0 || env_moved {
                self.ctl_left = CONTROL;
                self.env_ctl = self.env;
                let env_oct = if env_amount != 0.0 && self.env > 1e-6 {
                    let l =
                        ((20.0 * self.env.log10() - ENV_FLOOR_DB) / -ENV_FLOOR_DB).clamp(0.0, 1.0);
                    env_amount * l * ENV_OCTAVES
                } else {
                    0.0
                };
                // The LFO where this chunk ends: `g` ramps there from where it is.
                let end = self.lfo_ahead(transport, i, CONTROL);
                let raw = [self.lfo_value_at(end, 0.0), self.lfo_value_at(end, offset)];
                let base = cutoff + env_oct;
                let a = if self.g_primed {
                    self.lfo_lp_a_ctl
                } else {
                    1.0
                };
                for (c, raw) in raw.into_iter().enumerate().take(n_ch) {
                    self.lfo_lp[c] = flush(self.lfo_lp[c] + a * (raw - self.lfo_lp[c]));
                    let oct = base + lfo_amount * self.lfo_lp[c] * LFO_OCTAVES;
                    let fc = oct.exp2().clamp(20.0, nyq);
                    let g = (std::f32::consts::PI * fc / sr).tan();
                    if !self.g_primed || env_moved {
                        // Start from the cutoff here (the LFO now; an onset moves the
                        // cutoff at once, as per sample).
                        let now = base + lfo_amount * self.lfo_lp[c] * LFO_OCTAVES;
                        self.g[c] = (std::f32::consts::PI * now.exp2().clamp(20.0, nyq) / sr).tan();
                    }
                    self.g_step[c] = (g - self.g[c]) / CONTROL as f32;
                }
                self.g_primed = true;
            }
            self.ctl_left -= 1;
            let res = self.res.tick();
            let drive_db = self.drive.tick();
            let peak = self.ty == FilterType::Peak || self.fade_from == FilterType::Peak;
            // Resonance/drive coefficients only when they move (same values: exact).
            let key = (res.to_bits(), drive_db.to_bits(), peak);
            let common = match self.common {
                Some((k, c)) if k == key => c,
                _ => {
                    let (q_k, bell) = if peak {
                        let a = db((6.0 + 12.0 * res) * 0.5);
                        (1.0 / (0.5 * 24f32.powf(res) * a), a * a - 1.0)
                    } else {
                        (1.0 / (0.5 * 24f32.powf(res)), 0.0)
                    };
                    let drive = db(drive_db);
                    let boost = (24f32.powf(res) * std::f32::consts::FRAC_1_SQRT_2).max(1.0);
                    let c = Coefs {
                        g: 0.0,
                        k: q_k,
                        bell,
                        fb: LADDER_K_MAX * res,
                        k_last: 1.0 / (BUTTERWORTH_8_Q[3] * boost),
                        drive,
                        sat: (drive_db / 3.0).min(1.0),
                        comp: db(-0.5 * drive_db),
                    };
                    self.common = Some((key, c));
                    c
                }
            };
            let fade = if self.fade_left > 0 {
                self.fade_left -= 1;
                Some(self.fade_left as f32 / self.fade_len as f32)
            } else {
                None
            };
            let mix = self.mix.tick();
            let out_gain = self.output.tick();
            for c in 0..n_ch {
                // The same cutoff at four times the rate: tan(θ/2) = tan θ / (1 + √(1 + tan² θ)),
                // twice.
                let half = |g: f32| g / (1.0 + (1.0 + g * g).sqrt());
                let coefs = Coefs {
                    g: half(half(self.g[c])),
                    ..common
                };
                self.g[c] += self.g_step[c];
                let x = audio
                    .inputs
                    .get(c)
                    .or(audio.inputs.first())
                    .map_or(0.0, |b| b[i]);
                let st = &mut self.ch[c];
                let [a, b] = self.os1[c].up(x);
                let mut quad = [self.os2[c].up(a), self.os2[c].up(b)];
                for u in quad.iter_mut().flatten() {
                    let mut wet = st.tick(self.ty, *u, &coefs);
                    if let Some(old) = fade {
                        let prev = st.tick(self.fade_from, *u, &coefs);
                        wet = wet * (1.0 - old) + prev * old;
                    }
                    *u = wet;
                }
                let a = self.os2[c].down(quad[0]);
                let b = self.os2[c].down(quad[1]);
                let first = std::mem::replace(&mut self.pad[c], b);
                let wet = self.os1[c].down([first, a]);
                let dry = self.dry[c].tick(x);
                audio.outputs[c][i] = (dry + (wet - dry) * mix) * out_gain;
            }
        }
    }

    fn run(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        sidechain: &[&[f32]],
    ) -> ProcessStatus {
        let frames = ctx.frames;
        let n = sidechain.len().min(SIDECHAIN_CHANNELS);
        let sidechain = if sidechain[..n].iter().all(|c| c.len() >= frames) {
            &sidechain[..n]
        } else {
            &[]
        };
        let transport = ctx.transport;
        // Asleep for a while: a free LFO goes on as if it had run.
        let skipped = self.tail.skipped(ctx);
        if skipped > 0 {
            if !(self.lfo_sync && transport.playing) {
                let hz = if self.lfo_sync {
                    transport.bpm / 60.0 / self.lfo_beats
                } else {
                    self.lfo_rate
                };
                let f = self.frac + hz / f64::from(self.sample_rate) * skipped as f64;
                let whole = f.floor();
                self.frac = f - whole;
                self.cycle = self.cycle.wrapping_add(whole as i64);
            }
            self.g_primed = false;
            self.ctl_left = 0;
        }
        LateEvents::split(
            self,
            |s| &mut s.late,
            ctx,
            |kind| match kind {
                EventKind::Param {
                    param: ids::MIX | ids::OUTPUT,
                    ..
                } => Route::Late(LATENCY_4X),
                EventKind::Param { .. } => Route::Late(oversample::UP_DELAY_4X),
                _ => Route::Now,
            },
            |s, a, b| s.render(transport, audio, sidechain, a, b),
            |s, kind| {
                if let EventKind::Param { param, value, step } = *kind {
                    s.set(param, value, !step);
                    // A fresh control chunk from the event on. Only events (at fixed
                    // song frames) restart the chunk grid, never block boundaries, so
                    // renders stay block-size independent (CONTRACTS §12.7).
                    s.ctl_left = 0;
                }
            },
        );
        // The filter rings out under the floor; settled once the follower released and no
        // glide or type fade is in flight.
        let settled = self.env <= 1e-6
            && self.fade_left == 0
            && self.late.is_idle()
            && ![
                &self.cutoff,
                &self.res,
                &self.drive,
                &self.env_amount,
                &self.lfo_amount,
                &self.lfo_offset,
                &self.mix,
                &self.output,
            ]
            .iter()
            .any(|s| s.is_smoothing())
            && crate::util::all_zero(sidechain, frames);
        let hold =
            LATENCY_4X + crate::util::samples(crate::util::BASE_HOLD_SECONDS, self.sample_rate);
        self.tail.status_of(ctx, audio, settled, hold)
    }

    /// Current envelope follower level (linear peak), for tests.
    pub fn envelope(&self) -> f32 {
        self.env
    }
}

fn descriptor() -> DeviceDescriptor {
    super::descriptor(BuiltinDeviceType::AutoFilter)
}

impl Node for AutoFilter {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.sync_all();
        self.late.prepare(LATENCY_4X, config);
        self.reset();
    }

    fn reset(&mut self) {
        LateEvents::flush(
            self,
            |s| &mut s.late,
            |s, kind| {
                if let EventKind::Param { param, value, .. } = *kind {
                    s.set(param, value, false);
                }
            },
        );
        self.ch = [FilterState::default(); 2];
        for c in 0..2 {
            self.os1[c].reset();
            self.os2[c].reset();
            self.dry[c].reset();
        }
        self.pad = [0.0; 2];
        self.env = 0.0;
        self.lfo_lp = [0.0; 2];
        self.fade_left = 0;
        self.g_primed = false;
        self.ctl_left = 0;
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        self.run(ctx, audio, &[])
    }

    fn sidechain_inputs(&self) -> u16 {
        SIDECHAIN_CHANNELS as u16
    }

    fn process_sidechain(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        sidechain: &[&[f32]],
    ) -> ProcessStatus {
        self.run(ctx, audio, sidechain)
    }

    fn latency(&self) -> u32 {
        LATENCY_4X as u32
    }
}

impl Device for AutoFilter {
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

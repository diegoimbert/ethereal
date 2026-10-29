//! Volume LFO: the gain follows a drawn LFO cycle (`ether_model::LfoCurve`), for sidechain
//! pumping without a sidechain, gates, chops and tremolos, over the whole signal or per
//! frequency band.
//!
//! - **Gain**: `1 - Depth × (1 - curve(phase))`, per sample, so 0 % depth is bit-exact
//!   unity and 100 % follows the curve. The right channel reads the curve `Stereo Phase`
//!   degrees later.
//! - **Phase**: synced (the default) and playing, it is computed from the song position at
//!   every sample (`position / cycle beats + Phase`): phase 0 falls on every multiple of the
//!   cycle from beat 0, wherever playback starts, and a bounce is sample-exact. Stopped, a
//!   synced LFO runs on at the tempo; free (Hz) it runs on its own and restarts at phase 0
//!   when the transport starts (and on `reset`), so renders are deterministic.
//! - **Bands** (2 or 3): Linkwitz-Riley 24 dB/oct crossovers split the signal (low through
//!   the upper crossover's allpass, so the bands sum to a flat allpass: unity everywhere is
//!   transparent in magnitude). Each band has its own curve, depth and speed (a factor of
//!   the cycle rate: band phase = `cycles × speed`, so a ×2 band restarts on every half
//!   cycle and a ÷2 band spans two). The split is always three-way: with 2 bands, the
//!   upper crossover's two sides share band 2's gain. Switching the band count crossfades
//!   (10 ms) between the plain path and the split (and between 2 and 3 bands); with one
//!   band the filters don't run and the output is the plain shaper, sample for sample.
//!   Crossover moves glide (8 ms, on a log scale) so automation doesn't zipper.
//! - **Smoothing**: a one-pole on each band's gain (`Smoothing` ms, default 1) tames the
//!   hard edges of a drawn curve, a curve swap or a phase jump: the gain moves at most
//!   `1 - e^(-1/(ms·sr))` of the way per sample. 0 ms = the curve as drawn.
//! - **Curve edits** arrive as a [`BandCurves`] through `Node::set_data` (swapped, never
//!   allocated or freed here); the smoothing glides from the old curves to the new ones.
//! - Zero latency (IIR crossovers, no look-ahead). One band: memoryless on the audio
//!   (silent in, silent out); split, the crossovers ring out under the floor first.

use ether_core::node::NodeData;
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::{
    BuiltinDevice, BuiltinDeviceType, LfoCurve, LfoPoint, MAX_LFO_POINTS, ParamId,
    VOLUME_SHAPER_BANDS, lfo_value_at, volume_shaper_band,
};
use ether_core::{
    AudioBuffers, Device, EventKind, Node, PrepareConfig, ProcessContext, ProcessStatus,
};

use super::shared::{Params, Ramp, input};
use super::volume_lfo as p;
use super::{VOLUME_LFO_SPEED_FACTORS, VOLUME_LFO_SPEEDS};
use crate::contract::SYNC_RATE_BEATS;
use crate::fx_dynamics::shared::{LrSplit, Svf, SvfCoefs};
use crate::util::{self, db_to_amp};

const N: usize = p::COUNT;
const BANDS: usize = VOLUME_SHAPER_BANDS;
/// Crossfade of a band count change (ms).
const SWITCH_MS: f32 = 10.0;
/// Crossover glide time constant (ms).
const GLIDE_MS: f32 = 8.0;
/// Silence hold while the crossovers may still ring (s).
const SPLIT_HOLD_S: f32 = 0.05;
/// Free-running cycles counted (mod this) so a ÷2 / ÷4 band's phase stays continuous.
const CYCLE_WRAP: u32 = 4;

/// A curve as the node plays it: a fixed array (no allocation on the audio thread).
#[derive(Clone, Copy, Debug)]
pub struct CurveData {
    points: [LfoPoint; MAX_LFO_POINTS],
    len: usize,
}

impl CurveData {
    /// Non-RT. The playable copy of `curve` (sanitized: sorted, clamped, at most 64 points).
    pub fn new(curve: &LfoCurve) -> Self {
        let curve = curve.sanitized();
        let mut points = [LfoPoint::linear(0.0, 1.0); MAX_LFO_POINTS];
        let len = curve.points.len().min(MAX_LFO_POINTS);
        points[..len].copy_from_slice(&curve.points[..len]);
        Self { points, len }
    }

    /// Value at `phase` (cycles), `0..=1`.
    #[inline]
    pub fn at(&self, phase: f64) -> f32 {
        lfo_value_at(&self.points[..self.len], phase) as f32
    }
}

/// The curves of every band, as the node plays them (`Node::set_data` payload).
#[derive(Clone, Copy, Debug)]
pub struct BandCurves {
    curves: [CurveData; BANDS],
}

impl BandCurves {
    /// Non-RT. Band 1 = `curve`, bands 2 and 3 = `bands` (flat when missing).
    pub fn new(curve: &LfoCurve, bands: &[LfoCurve]) -> Self {
        Self {
            curves: std::array::from_fn(|b| CurveData::new(&volume_shaper_band(curve, bands, b))),
        }
    }
}

/// Non-RT. `Node::set_data` payload of a curve edit (bridges' `update_builtin`); `None` for
/// another device.
pub fn curve_data(device: &BuiltinDevice) -> Option<NodeData> {
    match device {
        BuiltinDevice::VolumeLfo { curve, bands } => Some(Box::new(BandCurves::new(curve, bands))),
        _ => None,
    }
}

/// Crossover state of one channel.
#[derive(Clone, Copy, Debug, Default)]
struct Split {
    low: LrSplit,
    /// The low band through the upper crossover's allpass (phase match).
    low_ap: Svf,
    high: LrSplit,
}

/// One linear 0..1 crossfade, stepped per sample.
#[derive(Clone, Copy, Debug, Default)]
struct Fade {
    value: f32,
    target: f32,
}

impl Fade {
    #[inline]
    fn step(&mut self, by: f32) {
        if self.value < self.target {
            self.value = (self.value + by).min(self.target);
        } else if self.value > self.target {
            self.value = (self.value - by).max(self.target);
        }
    }

    fn jump(&mut self) {
        self.value = self.target;
    }

    fn moving(&self) -> bool {
        self.value != self.target
    }
}

pub(super) struct VolumeLfo {
    params: Params<N>,
    sample_rate: f32,
    curves: BandCurves,
    /// Phase (cycles, `0..1`) before the offsets.
    phase: f64,
    /// Free-running whole cycles, mod [`CYCLE_WRAP`] (the slow bands' phase).
    cycles: u32,
    was_playing: bool,
    depth: [Ramp; BANDS],
    gain: Ramp,
    /// Smoothed per-band, per-channel curve gains.
    smoothed: [[f32; 2]; BANDS],
    /// One-pole coefficient of the smoothing (0 = none).
    coef: f32,
    /// 0 = the plain full-band path, 1 = the band split.
    split_fade: Fade,
    /// 0 = 2 bands (band 2's gain above the upper crossover too), 1 = 3 bands.
    three_fade: Fade,
    fade_step: f32,
    /// The crossover filters are running (state valid).
    split_on: bool,
    split: [Split; 2],
    /// Current (gliding) crossover frequencies, their logs and coefficients.
    freq: [f32; 2],
    log_freq: [f32; 2],
    coefs: [SvfCoefs; 2],
    glide_coef: f32,
    tail: util::Tail,
}

impl VolumeLfo {
    pub(super) fn new(desc: &DeviceDescriptor, curve: &LfoCurve, bands: &[LfoCurve]) -> Self {
        let mut s = Self {
            params: Params::<N>::new(desc),
            sample_rate: 48_000.0,
            curves: BandCurves::new(curve, bands),
            phase: 0.0,
            cycles: 0,
            was_playing: false,
            depth: [Ramp::new(1.0); BANDS],
            gain: Ramp::new(1.0),
            smoothed: [[1.0; 2]; BANDS],
            coef: 0.0,
            split_fade: Fade::default(),
            three_fade: Fade::default(),
            fade_step: 1.0,
            split_on: false,
            split: [Split::default(); 2],
            freq: [200.0, 15_000.0],
            log_freq: [200f32.ln(), 15_000f32.ln()],
            coefs: [SvfCoefs::new(200.0, 48_000.0); 2],
            glide_coef: 0.0,
            tail: util::Tail::default(),
        };
        for id in 0..N as u32 {
            s.sync_param(ParamId(id), false);
        }
        s.sync_rate_dependent();
        s.reset();
        s
    }

    fn band_count(&self) -> usize {
        self.params.index(p::BANDS, BANDS) + 1
    }

    fn sync_param(&mut self, id: ParamId, smooth: bool) {
        let v = self.params.get(id);
        if let Some(b) = p::DEPTHS.iter().position(|&d| d == id) {
            self.depth[b].set(v * 0.01, smooth);
            return;
        }
        match id {
            p::OUTPUT => self.gain.set(db_to_amp(v), smooth),
            p::SMOOTHING => {
                self.coef = if v <= 0.0 {
                    0.0
                } else {
                    util::tau_coef(v, self.sample_rate)
                };
            }
            p::BANDS => {
                let n = self.band_count();
                self.split_fade.target = if n > 1 { 1.0 } else { 0.0 };
                self.three_fade.target = if n > 2 { 1.0 } else { 0.0 };
                if !smooth {
                    self.split_fade.jump();
                    self.three_fade.jump();
                }
            }
            p::CROSSOVER_1 | p::CROSSOVER_2 if !smooth => {
                self.freq = self.target_freqs();
                self.log_freq = self.freq.map(f32::ln);
                self.update_coefs();
            }
            _ => {}
        }
    }

    /// Coefficients that depend on the sample rate (prepare).
    fn sync_rate_dependent(&mut self) {
        self.sync_param(p::SMOOTHING, false);
        self.glide_coef = util::tau_coef(GLIDE_MS, self.sample_rate);
        self.fade_step = 1.0 / (SWITCH_MS * 0.001 * self.sample_rate).max(1.0);
        self.freq = self.target_freqs();
        self.log_freq = self.freq.map(f32::ln);
        self.update_coefs();
    }

    fn apply(&mut self, id: ParamId, value: f64, smooth: bool) {
        if self.params.set(id, value) {
            self.sync_param(id, smooth);
        }
    }

    /// Target crossover frequencies (the upper one kept at or above the lower one).
    fn target_freqs(&self) -> [f32; 2] {
        let lo = self.params.get(p::CROSSOVER_1);
        [lo, self.params.get(p::CROSSOVER_2).max(lo)]
    }

    fn update_coefs(&mut self) {
        let sr = f64::from(self.sample_rate);
        self.coefs = self.freq.map(|f| SvfCoefs::new(f64::from(f), sr));
    }

    /// Glide the crossovers one sample towards their targets.
    #[inline]
    fn glide(&mut self) {
        let target = self.target_freqs();
        if target == self.freq {
            return;
        }
        for ((f, lf), t) in self.freq.iter_mut().zip(&mut self.log_freq).zip(target) {
            let lt = t.ln();
            *lf = lt + (*lf - lt) * self.glide_coef;
            let next = lf.exp();
            if (next - t).abs() < 1e-3 * t {
                *f = t;
                *lf = lt;
            } else {
                *f = next;
            }
        }
        self.update_coefs();
    }

    /// Cycle length in beats when synced.
    fn sync_beats(&self) -> Option<f64> {
        self.params.on(p::SYNC).then(|| {
            let i = self.params.index(p::SYNC_RATE, SYNC_RATE_BEATS.len());
            SYNC_RATE_BEATS[i]
        })
    }

    /// Speed factor of each band's cycle.
    fn speeds(&self) -> [f64; BANDS] {
        p::SPEEDS.map(|id| {
            id.map_or(1.0, |id| {
                VOLUME_LFO_SPEED_FACTORS[self.params.index(id, VOLUME_LFO_SPEEDS.len())]
            })
        })
    }

    /// Free-running phase increment per sample (synced: at the tempo `bpm`).
    fn free_inc(&self, bpm: f64) -> f64 {
        let sr = f64::from(self.sample_rate);
        match self.sync_beats() {
            Some(beats) => {
                let bpm = if bpm > 0.0 { bpm } else { 120.0 };
                bpm / 60.0 / beats / sr
            }
            None => f64::from(self.params.get(p::RATE)).max(0.0) / sr,
        }
    }

    /// Advance the free-running phase by `by` cycles.
    #[inline]
    fn advance(&mut self, by: f64) {
        let ph = self.phase + by;
        let whole = ph.floor();
        self.phase = ph - whole;
        if whole != 0.0 {
            let w = whole.rem_euclid(f64::from(CYCLE_WRAP)) as u32;
            self.cycles = (self.cycles + w) % CYCLE_WRAP;
        }
    }

    /// Phase of each band, from the phase of band 1 and the unwrapped cycle count.
    #[inline]
    fn band_phases(base: f64, unwrapped: f64, speeds: &[f64; BANDS]) -> [f64; BANDS] {
        let mut out = [base; BANDS];
        for b in 1..BANDS {
            let q = unwrapped * speeds[b];
            out[b] = q - q.floor();
        }
        out
    }

    /// Phase offsets `[left, right]` (cycles).
    fn offsets(&self) -> [f64; 2] {
        let phase = f64::from(self.params.get(p::PHASE)) / 360.0;
        let stereo = f64::from(self.params.get(p::STEREO_PHASE)) / 360.0;
        [phase, phase + stereo]
    }

    /// Unsmoothed gains `[band][left, right]` at the current phase and depth targets.
    fn targets(&self) -> [[f32; 2]; BANDS] {
        let o = self.offsets();
        let unwrapped = f64::from(self.cycles) + self.phase;
        let ph = Self::band_phases(self.phase, unwrapped, &self.speeds());
        std::array::from_fn(|b| {
            let depth = self.params.get(p::DEPTHS[b]) * 0.01;
            [0, 1].map(|ch| 1.0 - depth * (1.0 - self.curves.curves[b].at(ph[b] + o[ch])))
        })
    }

    crate::dsp::simd::avx2_dispatch! {
        fn render => render_avx2, render_generic(
            &mut self,
            ctx: &ProcessContext<'_>,
            audio: &mut AudioBuffers<'_, '_>,
            start: usize,
            end: usize,
        )
    }

    #[inline(always)]
    fn render_generic(
        &mut self,
        ctx: &ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
        start: usize,
        end: usize,
    ) {
        let t = ctx.transport;
        let locked = (t.playing && t.beats_per_sample > 0.0)
            .then_some(())
            .and(self.sync_beats());
        let inc = self.free_inc(t.bpm);
        let offsets = self.offsets();
        let speeds = self.speeds();
        let channels = audio.outputs.len().min(2);
        let inputs = audio.inputs;
        let k = self.coef;
        // The split path runs while it is (or is becoming) audible.
        let split = self.split_fade.value > 0.0 || self.split_fade.target > 0.0;
        if split && !self.split_on {
            self.split = [Split::default(); 2];
            self.split_on = true;
            // The upper bands' gains weren't followed while unsplit.
            let t = self.targets();
            self.smoothed[1..].copy_from_slice(&t[1..]);
        }
        let bands = if split { BANDS } else { 1 };
        let step = self.fade_step;
        for i in start..end {
            let unwrapped = match locked {
                Some(beats) => {
                    let ph = (t.position + t.beats_per_sample * i as f64) / beats;
                    self.phase = ph - ph.floor();
                    ph
                }
                None => {
                    self.advance(inc);
                    f64::from(self.cycles) + self.phase
                }
            };
            let depth = [
                self.depth[0].tick(),
                self.depth[1].tick(),
                self.depth[2].tick(),
            ];
            let gain = self.gain.tick();
            let phases = Self::band_phases(self.phase, unwrapped, &speeds);
            for b in 0..bands {
                let curve = &self.curves.curves[b];
                for (ch, &offset) in offsets.iter().enumerate().take(channels) {
                    let target = 1.0 - depth[b] * (1.0 - curve.at(phases[b] + offset));
                    let sm = &mut self.smoothed[b][ch];
                    *sm = crate::dsp::flush32(target + (*sm - target) * k);
                }
            }
            if !split {
                for ch in 0..channels {
                    // A tiny input times a decaying gain can't go subnormal.
                    audio.outputs[ch][i] =
                        crate::dsp::flush32(input(inputs, ch, i) * self.smoothed[0][ch] * gain);
                }
            } else {
                self.glide();
                let (c_lo, c_hi) = (self.coefs[0], self.coefs[1]);
                let (m, three) = (self.split_fade.value, self.three_fade.value);
                for ch in 0..channels {
                    let x = input(inputs, ch, i);
                    let st = &mut self.split[ch];
                    let (low, rest) = st.low.tick(f64::from(x), &c_lo);
                    let low = st.low_ap.allpass(low, &c_hi);
                    let (mid, high) = st.high.tick(rest, &c_hi);
                    let [g0, g1, g2] = [0, 1, 2].map(|b| self.smoothed[b][ch]);
                    let g_high = g1 + three * (g2 - g1);
                    let y = low * f64::from(g0) + mid * f64::from(g1) + high * f64::from(g_high);
                    let v = (1.0 - m) * (x * g0) + m * y as f32;
                    audio.outputs[ch][i] = crate::dsp::flush32(v * gain);
                }
                self.split_fade.step(step);
                self.three_fade.step(step);
            }
            for out in audio.outputs.iter_mut().skip(channels) {
                out[i] = 0.0;
            }
        }
        if !split || (self.split_fade.value == 0.0 && self.split_fade.target == 0.0) {
            self.split_on = false;
            // Not running: a later change takes effect at once.
            self.three_fade.jump();
        }
    }
}

impl Node for VolumeLfo {
    fn prepare(&mut self, config: &PrepareConfig) {
        self.sample_rate = config.sample_rate.max(1.0);
        self.sync_rate_dependent();
        self.reset();
    }

    fn reset(&mut self) {
        self.phase = 0.0;
        self.cycles = 0;
        self.smoothed = self.targets();
        self.split = [Split::default(); 2];
        self.split_fade.jump();
        self.three_fade.jump();
        self.split_on = false;
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        audio: &mut AudioBuffers<'_, '_>,
    ) -> ProcessStatus {
        let c: &ProcessContext<'_> = ctx;
        let t = c.transport;
        // A free LFO restarts with the transport (deterministic renders); a synced one
        // follows the song position anyway.
        if t.playing && !self.was_playing && self.sync_beats().is_none() {
            self.phase = 0.0;
            self.cycles = 0;
        }
        self.was_playing = t.playing;
        // Asleep for a while: the free LFO goes on as if it had run; the crossovers rang out.
        let skipped = self.tail.skipped(c);
        if skipped > 0 {
            self.advance(self.free_inc(t.bpm) * skipped as f64);
            self.smoothed = self.targets();
            self.split = [Split::default(); 2];
            self.split_fade.jump();
            self.three_fade.jump();
        }
        util::split_at_events(
            self,
            c.events,
            c.frames,
            |s, a, b| s.render(c, audio, a, b),
            |s, kind| {
                if let EventKind::Param { param, value, step } = *kind {
                    s.apply(param, value, !step);
                }
            },
        );
        let settled = !self.depth.iter().any(Ramp::is_smoothing)
            && !self.gain.is_smoothing()
            && !self.split_fade.moving()
            && !self.three_fade.moving();
        // One band: memoryless (input × gain), silent in, silent out; split: let the
        // crossovers ring out.
        let hold = if self.split_on {
            util::samples(SPLIT_HOLD_S, self.sample_rate)
        } else {
            1
        };
        self.tail.status_of(c, audio, settled, hold)
    }

    fn set_data(&mut self, data: NodeData) -> Option<NodeData> {
        match data.downcast::<BandCurves>() {
            Ok(mut new) => {
                // The old curves go back in the box (dropped off the audio thread).
                std::mem::swap(&mut self.curves, &mut *new);
                Some(new)
            }
            Err(other) => Some(other),
        }
    }
}

impl Device for VolumeLfo {
    fn descriptor(&self) -> DeviceDescriptor {
        super::descriptor(BuiltinDeviceType::VolumeLfo)
    }

    fn param(&self, id: ParamId) -> Option<f64> {
        self.params.plain(id)
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        self.apply(id, value, false);
    }
}

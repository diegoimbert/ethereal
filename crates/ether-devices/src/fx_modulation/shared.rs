//! Building blocks shared by the modulation effects: a fixed-size param store, parameter
//! glides/ramps, a tempo-lockable LFO and a modulated delay line with 4-point Hermite
//! interpolation (no zipper noise when the read position moves every sample).

use ether_core::Smoother;
use ether_core::TransportInfo;
use ether_core::automation_rt::PARAM_GRID;
use ether_core::protocol::devices::DeviceDescriptor;
use ether_core::protocol::model::ParamId;

use crate::contract::SYNC_RATE_BEATS;

/// Plain param values of a device with `N` params (dense ids `0..N`), clamped to the
/// descriptor's ranges. Allocation-free after `new`.
#[derive(Clone, Debug)]
pub(super) struct Params<const N: usize> {
    values: [f64; N],
    ranges: [(f64, f64, f64); N],
}

impl<const N: usize> Params<N> {
    /// Non-RT. Defaults of `desc` (whose params must be the dense ids `0..N`).
    pub(super) fn new(desc: &DeviceDescriptor) -> Self {
        assert_eq!(desc.params.len(), N, "{}", desc.name);
        let mut values = [0.0; N];
        let mut ranges = [(0.0, 0.0, 0.0); N];
        for p in &desc.params {
            let i = p.id.0 as usize;
            values[i] = p.default;
            ranges[i] = (p.min, p.max, p.default);
        }
        Self { values, ranges }
    }

    /// Plain value (as `f32`).
    #[inline]
    pub(super) fn get(&self, id: ParamId) -> f32 {
        self.values[id.0 as usize] as f32
    }

    /// Toggle as a bool (`>= 0.5`).
    #[inline]
    pub(super) fn on(&self, id: ParamId) -> bool {
        self.values[id.0 as usize] >= 0.5
    }

    /// Enum index (`0..count`).
    #[inline]
    pub(super) fn index(&self, id: ParamId, count: usize) -> usize {
        crate::util::index(self.values[id.0 as usize], count)
    }

    pub(super) fn plain(&self, id: ParamId) -> Option<f64> {
        self.values.get(id.0 as usize).copied()
    }

    /// Set a plain value (clamped; NaN = default). `false` for an unknown id.
    pub(super) fn set(&mut self, id: ParamId, value: f64) -> bool {
        let Some(&(min, max, default)) = self.ranges.get(id.0 as usize) else {
            return false;
        };
        self.values[id.0 as usize] = if value.is_nan() {
            default
        } else {
            value.clamp(min, max)
        };
        true
    }
}

/// Gain-like param driven by events: each change ramps linearly over one automation grid
/// interval ([`PARAM_GRID`] samples), so grid-spaced automation renders as the exact
/// piecewise-linear curve and knob moves don't click.
#[derive(Clone, Copy, Debug)]
pub(super) struct Ramp(Smoother);

impl Ramp {
    pub(super) fn new(value: f32) -> Self {
        Self(Smoother::new(value, 1.0, 48_000.0))
    }

    /// Ramp to `target` (`smooth`) or jump there.
    pub(super) fn set(&mut self, target: f32, smooth: bool) {
        if smooth {
            self.0.ramp_to(target, PARAM_GRID as u32);
        } else {
            self.0.set_immediate(target);
        }
    }

    #[inline]
    pub(super) fn tick(&mut self) -> f32 {
        self.0.tick()
    }
}

/// One-pole glide toward a target (delay times, depths, filter frequencies): moving a
/// modulated delay's read position in a 32-sample ramp would be an audible pitch blip, so
/// these follow with a time constant instead. The target is set at the event's sample.
#[derive(Clone, Copy, Debug)]
pub(super) struct Glide {
    current: f32,
    target: f32,
    coef: f32,
}

impl Glide {
    pub(super) fn new(value: f32) -> Self {
        Self {
            current: value,
            target: value,
            coef: 0.0,
        }
    }

    /// Non-RT (or RT, cheap). Time constant `ms` at `sample_rate`.
    pub(super) fn set_time(&mut self, ms: f32, sample_rate: f32) {
        self.coef = crate::util::tau_coef(ms, sample_rate);
    }

    pub(super) fn set(&mut self, target: f32, smooth: bool) {
        self.target = target;
        if !smooth {
            self.current = target;
        }
    }

    #[inline]
    pub(super) fn tick(&mut self) -> f32 {
        let d = self.current - self.target;
        self.current = if d.abs() < 1e-9 * self.target.abs().max(1e-6) {
            self.target
        } else {
            self.target + d * self.coef
        };
        self.current
    }
}

/// LFO waveforms (the Tremolo's `Shape` order; the renderer's `waveAt` draws the same).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Shape {
    Sine,
    Triangle,
    Square,
    SawUp,
    SawDown,
}

impl Shape {
    pub(super) const ALL: [Shape; 5] = [
        Shape::Sine,
        Shape::Triangle,
        Shape::Square,
        Shape::SawUp,
        Shape::SawDown,
    ];

    /// Value at `phase` (cycles, `0..1`), `-1..=1`.
    #[inline]
    pub(super) fn at(self, phase: f64) -> f32 {
        let p = phase - phase.floor();
        (match self {
            Shape::Sine => (std::f64::consts::TAU * p).sin(),
            Shape::Triangle => {
                if p < 0.25 {
                    4.0 * p
                } else if p < 0.75 {
                    2.0 - 4.0 * p
                } else {
                    4.0 * p - 4.0
                }
            }
            Shape::Square => {
                if p < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Shape::SawUp => 2.0 * p - 1.0,
            Shape::SawDown => 1.0 - 2.0 * p,
        }) as f32
    }
}

/// How an LFO's rate is set for one sub-block.
#[derive(Clone, Copy, Debug)]
pub(super) enum Rate {
    /// Free-running at this many Hertz.
    Hz(f64),
    /// Tempo-synced: one cycle per `beats` (a [`SYNC_RATE_BEATS`] entry).
    Beats(f64),
}

impl Rate {
    /// `Hz(free)` or, when `sync`, the synced rate `SYNC_RATE_BEATS[index]`.
    pub(super) fn of(sync: bool, free_hz: f32, index: usize) -> Self {
        if sync {
            Rate::Beats(SYNC_RATE_BEATS[index.min(SYNC_RATE_BEATS.len() - 1)])
        } else {
            Rate::Hz(f64::from(free_hz))
        }
    }
}

/// Time constant (seconds) with which a synced LFO pulls its phase onto the song position.
/// Phase errors (play from a new position, loop wraps that aren't a whole number of cycles,
/// tempo jumps) are corrected smoothly instead of jumping, so a modulated delay never clicks.
const LOCK_SECONDS: f64 = 0.03;

/// Phase accumulator of an LFO. Free-running, or (synced while the transport plays) locked
/// to the song position: phase 0 on every multiple of the synced length from beat 0, like
/// Ableton's synced LFOs.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Lfo {
    phase: f64,
}

/// Per-sub-block constants of an [`Lfo`] advance.
#[derive(Clone, Copy, Debug)]
pub(super) struct LfoStep {
    /// Phase increment per sample.
    inc: f64,
    /// Song-locked phase at sample 0 of the sub-block and its increment per sample (synced
    /// and playing), else `None`.
    lock: Option<(f64, f64)>,
    /// Correction gain per sample.
    k: f64,
}

impl LfoStep {
    pub(super) fn new(rate: Rate, transport: &TransportInfo, sample_rate: f32) -> Self {
        let sr = f64::from(sample_rate.max(1.0));
        let k = 1.0 / (LOCK_SECONDS * sr);
        match rate {
            Rate::Hz(hz) => Self {
                inc: hz.max(0.0) / sr,
                lock: None,
                k,
            },
            Rate::Beats(beats) => {
                let bpm = if transport.bpm > 0.0 {
                    transport.bpm
                } else {
                    120.0
                };
                let inc = bpm / 60.0 / beats / sr;
                let lock = (transport.playing && transport.beats_per_sample > 0.0).then(|| {
                    (
                        transport.position / beats,
                        transport.beats_per_sample / beats,
                    )
                });
                Self {
                    // While playing, follow the actual song speed (tempo ramps).
                    inc: lock.map_or(inc, |(_, d)| d),
                    lock,
                    k,
                }
            }
        }
    }
}

impl Lfo {
    /// Current phase (cycles, `0..1`).
    #[inline]
    pub(super) fn phase(&self) -> f64 {
        self.phase
    }

    /// Advance one sample; `i` is the sample index in the sub-block the step was made for.
    #[inline]
    pub(super) fn advance(&mut self, step: &LfoStep, i: usize) {
        let mut p = self.phase + step.inc;
        if let Some((start, per)) = step.lock {
            let target = start + per * i as f64;
            let mut err = (target - p).rem_euclid(1.0);
            if err >= 0.5 {
                err -= 1.0;
            }
            p += err * step.k;
        }
        self.phase = p - p.floor();
    }
}

/// Mono delay line for modulated reads: power-of-two ring, allocated once (`new`), read at
/// fractional positions with 4-point, 3rd-order Hermite interpolation.
#[derive(Clone, Debug, Default)]
pub(super) struct ModLine {
    buf: Vec<f32>,
    mask: usize,
    /// Index of the newest sample.
    write: usize,
}

impl ModLine {
    /// Non-RT. A line readable up to `max_delay` samples back.
    pub(super) fn new(max_delay: usize) -> Self {
        let len = (max_delay + 4).next_power_of_two();
        Self {
            buf: vec![0.0; len],
            mask: len - 1,
            write: 0,
        }
    }

    pub(super) fn clear(&mut self) {
        self.buf.fill(0.0);
        self.write = 0;
    }

    /// Largest delay [`Self::read`] accepts.
    pub(super) fn max_delay(&self) -> f32 {
        (self.buf.len() - 3) as f32
    }

    /// Push one sample; it becomes `tap(0)`.
    #[inline]
    pub(super) fn push(&mut self, x: f32) {
        self.write = (self.write + 1) & self.mask;
        self.buf[self.write] = crate::dsp::flush32(x);
    }

    /// The sample pushed `k` pushes ago (0 = the newest).
    #[inline]
    pub(super) fn tap(&self, k: usize) -> f32 {
        self.buf[self.write.wrapping_sub(k) & self.mask]
    }

    /// Fractional read `delay` samples back (clamped to `1 ..= max_delay`).
    #[inline]
    pub(super) fn read(&self, delay: f32) -> f32 {
        let d = delay.clamp(1.0, self.max_delay());
        let i = d as usize;
        let f = d - i as f32;
        let x0 = self.tap(i - 1);
        let x1 = self.tap(i);
        let x2 = self.tap(i + 1);
        let x3 = self.tap(i + 2);
        let c1 = 0.5 * (x2 - x0);
        let c2 = x0 - 2.5 * x1 + 2.0 * x2 - 0.5 * x3;
        let c3 = 0.5 * (x3 - x0) + 1.5 * (x1 - x2);
        ((c3 * f + c2) * f + c1) * f + x1
    }
}

/// Topology-preserving one-pole low-pass (exact -3 dB at the cutoff), per channel.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct OnePoleLp {
    s: f32,
}

impl OnePoleLp {
    /// Coefficient for `hz` at `sample_rate`.
    #[inline]
    pub(super) fn coef(hz: f32, sample_rate: f32) -> f32 {
        let g = (std::f32::consts::PI * hz.clamp(10.0, sample_rate * 0.45) / sample_rate).tan();
        g / (1.0 + g)
    }

    #[inline]
    pub(super) fn tick(&mut self, x: f32, a: f32) -> f32 {
        let v = (x - self.s) * a;
        let y = v + self.s;
        self.s = crate::dsp::flush32(y + v);
        y
    }

    pub(super) fn reset(&mut self) {
        self.s = 0.0;
    }
}

/// Input sample of channel `ch` (mono input feeds both channels).
#[inline]
pub(super) fn input(inputs: &[&[f32]], ch: usize, i: usize) -> f32 {
    inputs
        .get(ch)
        .or_else(|| inputs.first())
        .map_or(0.0, |c| c[i])
}

/// Samples of `ms` milliseconds.
#[inline]
pub(super) fn ms(ms: f32, sample_rate: f32) -> f32 {
    ms * 0.001 * sample_rate
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hermite_reads_integer_taps_exactly_and_interpolates_smoothly() {
        let mut l = ModLine::new(16);
        for i in 0..40 {
            l.push(i as f32);
        }
        assert_eq!(l.tap(0), 39.0);
        assert_eq!(l.read(3.0), 36.0);
        // A ramp is reproduced exactly by the cubic.
        assert!((l.read(2.25) - 36.75).abs() < 1e-4);
        // A sine is read with tiny error at fractional positions.
        let mut l = ModLine::new(64);
        let w = 0.05f32;
        for i in 0..100 {
            l.push((w * i as f32).sin());
        }
        let d = 7.4;
        let expected = (w * (99.0 - d)).sin();
        assert!((l.read(d) - expected).abs() < 1e-4);
    }

    #[test]
    fn shapes_match_the_renderer() {
        assert!(Shape::Sine.at(0.25) > 0.999);
        assert_eq!(Shape::Triangle.at(0.5), 0.0);
        assert_eq!(Shape::Square.at(0.75), -1.0);
        assert_eq!(Shape::SawUp.at(0.0), -1.0);
        assert_eq!(Shape::SawDown.at(0.0), 1.0);
    }

    #[test]
    fn synced_lfo_locks_onto_the_song_position() {
        let t = TransportInfo {
            playing: true,
            bpm: 120.0,
            beats_per_sample: 2.0 / 48_000.0,
            position: 0.3,
            ..TransportInfo::STOPPED
        };
        // One cycle per beat; start far off the song phase.
        let step = LfoStep::new(Rate::Beats(1.0), &t, 48_000.0);
        let mut lfo = Lfo { phase: 0.8 };
        let n = 48_000 / 2; // half a second: > 10 time constants
        for i in 0..n {
            lfo.advance(&step, i);
        }
        let target = (0.3 + 2.0 * n as f64 / 48_000.0).rem_euclid(1.0);
        let mut err = (lfo.phase() - target).abs();
        err = err.min(1.0 - err);
        assert!(err < 1e-3, "{err}");
    }
}

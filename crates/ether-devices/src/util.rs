//! Small shared DSP and descriptor helpers.

use ether_core::protocol::devices::{ParamInfo, ParamScale, ParamUnit};
use ether_core::protocol::model::ParamId;

/// Builder for a continuous [`ParamInfo`].
pub(crate) fn param(
    id: u32,
    name: &str,
    group: &str,
    unit: ParamUnit,
    (min, max, default): (f64, f64, f64),
    scale: ParamScale,
) -> ParamInfo {
    ParamInfo {
        step: None,
        remote: None,
        id: ParamId(id),
        name: name.to_owned(),
        group: Some(group.to_owned()),
        unit,
        min,
        max,
        default,
        scale,
        labels: None,
        automatable: true,
        hidden: false,
    }
}

/// Builder for a discrete (enum) [`ParamInfo`] with plain values `0..labels.len()-1`.
pub(crate) fn choice(
    id: u32,
    name: &str,
    group: &str,
    unit: ParamUnit,
    labels: &[&str],
    default: usize,
) -> ParamInfo {
    ParamInfo {
        id: ParamId(id),
        name: name.to_owned(),
        group: Some(group.to_owned()),
        unit,
        min: 0.0,
        max: (labels.len() - 1) as f64,
        default: default as f64,
        scale: ParamScale::Linear,
        labels: Some(labels.iter().map(|s| (*s).to_owned()).collect()),
        automatable: true,
        hidden: false,
        step: Some(1.0),
        remote: None,
    }
}

/// Clamp `value` into the range of the param described by `(min, max)`; NaN maps to `min`.
#[inline]
pub(crate) fn clamp(value: f64, min: f64, max: f64) -> f64 {
    if value.is_nan() {
        min
    } else {
        value.clamp(min, max)
    }
}

/// Round an enum param's plain value to an index.
#[inline]
pub(crate) fn index(value: f64, count: usize) -> usize {
    (value.round().max(0.0) as usize).min(count - 1)
}

#[inline]
pub(crate) fn db_to_amp(db: f32) -> f32 {
    10f32.powf(db * 0.05)
}

#[inline]
pub(crate) fn amp_to_db(amp: f32) -> f32 {
    20.0 * amp.max(1e-9).log10()
}

/// MIDI key (fractional) to frequency, A4 = 440 Hz.
#[inline]
pub(crate) fn key_to_hz(key: f32) -> f32 {
    440.0 * 2f32.powf((key - 69.0) / 12.0)
}

/// One-pole coefficient such that an exponential segment covers ~-80 dB (1e-4) in `ms`.
/// Used for decay/release segments.
#[inline]
pub(crate) fn exp_coef(ms: f32, sample_rate: f32) -> f32 {
    let samples = (ms * 0.001 * sample_rate).max(1.0);
    (-9.21 / samples).exp()
}

/// One-pole coefficient with time constant `ms` (reaches ~63 % in `ms`).
#[inline]
pub(crate) fn tau_coef(ms: f32, sample_rate: f32) -> f32 {
    let samples = (ms * 0.001 * sample_rate).max(1.0);
    (-1.0 / samples).exp()
}

/// Amplitude envelope (ADSR). Linear attack, exponential decay and release.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Adsr {
    stage: Stage,
    level: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// Precomputed per-sample envelope rates (shared by all voices of a device).
#[derive(Clone, Copy, Debug)]
pub(crate) struct AdsrRates {
    /// Level increment per sample during attack.
    pub attack_step: f32,
    pub decay_coef: f32,
    pub sustain: f32,
    pub release_coef: f32,
}

impl AdsrRates {
    pub(crate) fn new(
        attack_ms: f32,
        decay_ms: f32,
        sustain: f32,
        release_ms: f32,
        sample_rate: f32,
    ) -> Self {
        Self {
            attack_step: 1.0 / (attack_ms * 0.001 * sample_rate).max(1.0),
            decay_coef: exp_coef(decay_ms, sample_rate),
            sustain: sustain.clamp(0.0, 1.0),
            release_coef: exp_coef(release_ms, sample_rate),
        }
    }
}

/// Level below which a releasing envelope is considered finished (-100 dB).
const ENV_FLOOR: f32 = 1e-5;

impl Adsr {
    pub(crate) const IDLE: Self = Self {
        stage: Stage::Idle,
        level: 0.0,
    };

    /// Start (or retrigger from the current level, avoiding clicks).
    pub(crate) fn trigger(&mut self) {
        self.stage = Stage::Attack;
    }

    pub(crate) fn release(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }

    pub(crate) fn kill(&mut self) {
        *self = Self::IDLE;
    }

    pub(crate) fn is_active(&self) -> bool {
        self.stage != Stage::Idle
    }

    pub(crate) fn is_releasing(&self) -> bool {
        self.stage == Stage::Release
    }

    #[inline]
    pub(crate) fn tick(&mut self, r: &AdsrRates) -> f32 {
        match self.stage {
            Stage::Idle => {}
            Stage::Attack => {
                self.level += r.attack_step;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                self.level = r.sustain + (self.level - r.sustain) * r.decay_coef;
                if (self.level - r.sustain).abs() < ENV_FLOOR {
                    self.level = r.sustain;
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Sustain => {
                // Follow sustain changes smoothly.
                self.level = r.sustain + (self.level - r.sustain) * r.decay_coef;
                if self.level < ENV_FLOOR {
                    self.kill();
                }
            }
            Stage::Release => {
                self.level *= r.release_coef;
                if self.level < ENV_FLOOR {
                    self.kill();
                }
            }
        }
        self.level
    }
}

/// Split a block at event offsets: calls `render(start, end)` for each event-free span and
/// `event(kind)` for each event, in order. Offsets past `frames` are clamped.
#[inline]
pub(crate) fn split_at_events<S>(
    state: &mut S,
    events: &[ether_core::ProcessEvent],
    frames: usize,
    mut render: impl FnMut(&mut S, usize, usize),
    mut event: impl FnMut(&mut S, &ether_core::EventKind),
) {
    let mut pos = 0;
    for ev in events {
        let off = (ev.offset as usize).min(frames);
        if off > pos {
            render(state, pos, off);
            pos = off;
        }
        event(state, &ev.kind);
    }
    if pos < frames {
        render(state, pos, frames);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adsr_shape() {
        let r = AdsrRates::new(1.0, 10.0, 0.5, 10.0, 1000.0);
        let mut e = Adsr::IDLE;
        e.trigger();
        assert_eq!(e.tick(&r), 1.0);
        for _ in 0..100 {
            e.tick(&r);
        }
        assert!((e.tick(&r) - 0.5).abs() < 1e-3);
        e.release();
        for _ in 0..20 {
            e.tick(&r);
        }
        assert!(!e.is_active());
    }

    #[test]
    fn db_roundtrip() {
        assert!((db_to_amp(-6.0206) - 0.5).abs() < 1e-4);
        assert!((amp_to_db(0.5) + 6.0206).abs() < 1e-3);
        assert!((key_to_hz(69.0) - 440.0).abs() < 1e-3);
    }
}

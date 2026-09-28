//! Parameter changes from the UI (lock-free queue) and smoothing.

use ether_protocol::model::{ModulatorId, ParamId, SendId, TrackId};
use serde::{Deserialize, Serialize};

use crate::node::NodeKey;

/// What a live parameter change addresses. Mixer controls are addressed by track/send (the
/// engine owns the mixer nodes); device params by node.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ParamTarget {
    TrackVolume {
        track: TrackId,
    },
    TrackPan {
        track: TrackId,
    },
    TrackMute {
        track: TrackId,
    },
    SendLevel {
        send: SendId,
    },
    Node {
        node: NodeKey,
        param: ParamId,
    },
    /// v0.2 (`racks-modulation`): a modulator's param (plain), `crate::modulation`.
    Modulator {
        modulator: ModulatorId,
        param: ParamId,
    },
}

/// A parameter change pushed from the controller thread (`EngineHandle::set_param`). Values
/// are plain (linear gain for volume/send level, -1..=1 for pan, 0/1 for mute). Applied at
/// the start of the next block (offset 0 of its first sub-block); continuous params are
/// smoothed by the receiver. `TrackVolume`/`TrackMute` of a VCA id reach `crate::vca` (v0.2).
///
/// **Node parameter-event API (v0.2, frozen; CONTRACTS.md §12.7).** Nodes receive every
/// param change (live, automation, modulation) as `EventKind::Param { param, value }` at a
/// sample `offset` in `ProcessContext::events`, sorted. Built-in devices apply them at their
/// offset (`ether_devices::util::split_at_events` + per-param smoothing); plugin hosts
/// forward the offset: CLAP `clap_event_param_value.header.time`, VST3
/// `IParameterChanges`/`IParamValueQueue::addPoint(sampleOffset)`, AU
/// `AudioUnitScheduleParameters` (`AUParameterEvent` with `eventSampleTime`). A node that
/// ignores offsets applies them at block start (the backwards-compatible default).
///
/// While an enabled automation lane drives a target, automation wins and manual changes are
/// overwritten on the next automation value (no override/re-enable mechanism in v0.1).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParamChange {
    pub target: ParamTarget,
    pub value: f64,
}

/// One-pole / linear parameter smoother. RT-safe, no allocation.
#[derive(Clone, Copy, Debug)]
pub struct Smoother {
    current: f32,
    target: f32,
    step: f32,
    remaining: u32,
    ramp_samples: u32,
}

impl Smoother {
    /// `ramp_ms` at `sample_rate` (typ. 10-20 ms).
    pub fn new(value: f32, ramp_ms: f32, sample_rate: f32) -> Self {
        let ramp_samples = ((ramp_ms * 0.001 * sample_rate).max(1.0)) as u32;
        Self {
            current: value,
            target: value,
            step: 0.0,
            remaining: 0,
            ramp_samples,
        }
    }

    pub fn set_target(&mut self, target: f32) {
        self.target = target;
        self.remaining = self.ramp_samples;
        self.step = (target - self.current) / self.ramp_samples as f32;
    }

    /// RT. Linear ramp from the current value to `target` over exactly `samples` ticks
    /// (the `samples`-th tick returns `target`). Sample-accurate automation drives mixer
    /// targets with it, one ramp per [`crate::automation_rt::PARAM_GRID`] interval
    /// (CONTRACTS.md §12.7). `samples == 0` jumps.
    pub fn ramp_to(&mut self, target: f32, samples: u32) {
        if samples == 0 {
            self.set_immediate(target);
            return;
        }
        self.target = target;
        self.remaining = samples;
        self.step = (target - self.current) / samples as f32;
    }

    /// Jump without ramp.
    pub fn set_immediate(&mut self, value: f32) {
        self.current = value;
        self.target = value;
        self.remaining = 0;
    }

    #[inline]
    pub fn tick(&mut self) -> f32 {
        if self.remaining > 0 {
            self.remaining -= 1;
            self.current = if self.remaining == 0 {
                self.target
            } else {
                self.current + self.step
            };
        }
        self.current
    }

    pub fn is_smoothing(&self) -> bool {
        self.remaining > 0
    }

    pub fn current(&self) -> f32 {
        self.current
    }

    pub fn target(&self) -> f32 {
        self.target
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoother_reaches_target() {
        let mut s = Smoother::new(0.0, 1.0, 1000.0);
        s.set_target(1.0);
        assert!(s.is_smoothing());
        let v = s.tick();
        assert!(v > 0.0 && v <= 1.0);
        for _ in 0..10 {
            s.tick();
        }
        assert_eq!(s.current(), 1.0);
    }

    #[test]
    fn ramp_to_is_linear_and_exact_at_the_end() {
        let mut s = Smoother::new(0.0, 10.0, 1000.0);
        s.ramp_to(1.0, 4);
        let v: Vec<f32> = (0..5).map(|_| s.tick()).collect();
        assert_eq!(v, vec![0.25, 0.5, 0.75, 1.0, 1.0]);
        // Re-targeting to the current value holds it exactly.
        s.ramp_to(1.0, 32);
        assert!((0..40).all(|_| s.tick() == 1.0));
        s.ramp_to(0.5, 0);
        assert_eq!(s.current(), 0.5);
    }
}

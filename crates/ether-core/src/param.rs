//! Parameter changes from the UI (lock-free queue) and smoothing.

use ether_protocol::model::{ParamId, SendId, TrackId};
use serde::{Deserialize, Serialize};

use crate::node::NodeKey;

/// What a live parameter change addresses. Mixer controls are addressed by track/send (the
/// engine owns the mixer nodes); device params by node.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ParamTarget {
    TrackVolume { track: TrackId },
    TrackPan { track: TrackId },
    TrackMute { track: TrackId },
    SendLevel { send: SendId },
    Node { node: NodeKey, param: ParamId },
}

/// A parameter change pushed from the controller thread (`EngineHandle::set_param`). Values
/// are plain (linear gain for volume/send level, -1..=1 for pan, 0/1 for mute). Applied at
/// the start of the next block; continuous params are smoothed by the receiver.
///
/// Automation playback overrides UI values while a lane is enabled (Ableton semantics).
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
}

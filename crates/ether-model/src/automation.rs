//! Automation lanes and breakpoints.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{AutomationLaneId, AutomationPointId, ClipId, DeviceId, SendId, TrackId};
use crate::value::{Beats, ParamId};

/// An automation lane. Points are separate entities (`AutomationPoint::lane == this`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct AutomationLane {
    pub id: AutomationLaneId,
    pub owner: AutomationOwner,
    pub target: AutomationTarget,
    /// Disabled lanes are kept but not applied (Ableton's "re-enable automation").
    pub enabled: bool,
}

/// Who owns the lane, which also defines its time base.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum AutomationOwner {
    /// Arrangement automation on a track; times are arrangement beats.
    Track { track: TrackId },
    /// Clip envelope; times are content-relative beats of the clip.
    Clip { clip: ClipId },
}

/// The automated parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum AutomationTarget {
    TrackVolume { track: TrackId },
    TrackPan { track: TrackId },
    SendLevel { send: SendId },
    DeviceParam { device: DeviceId, param: ParamId },
}

/// A breakpoint. `value` is **normalized** 0..=1; the target's `ParamInfo` maps it to a
/// plain value. `curve` shapes the segment from this point to the next one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct AutomationPoint {
    pub id: AutomationPointId,
    pub lane: AutomationLaneId,
    pub time: Beats,
    pub value: f64,
    pub curve: CurveShape,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum CurveShape {
    #[default]
    Linear,
    /// Hold this value until the next point.
    Step,
    /// Power curve; `tension` in -1..=1 (0 = linear, >0 = slow start, <0 = fast start).
    Curve { tension: f32 },
}

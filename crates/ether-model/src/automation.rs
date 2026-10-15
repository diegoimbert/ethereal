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
    /// Explicit user toggle: disabled lanes are kept but not applied. v0.1 has no
    /// "manual move overrides automation / re-enable automation" behaviour.
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
    TrackVolume {
        track: TrackId,
    },
    TrackPan {
        track: TrackId,
    },
    SendLevel {
        send: SendId,
    },
    DeviceParam {
        device: DeviceId,
        param: ParamId,
    },
    /// A device's on/off switch: normalized 0/1 (on at >= 0.5), held between points (never
    /// interpolated). While an enabled lane drives it, it overrides `Device::enabled`.
    DeviceEnabled {
        device: DeviceId,
    },
    /// Clip envelopes of audio clips only (`AutomationOwner::Clip` of an audio clip): the
    /// clip's own gain. **Modulates**: the envelope's dB are added to `AudioContent::gain`
    /// (there is no other value to override). Normalized 0..=1 with the track volume's
    /// fader law, [`CLIP_GAIN_MIN_DB`]..=[`CLIP_GAIN_MAX_DB`], 0 dB = no change.
    ClipGain,
    /// Clip envelopes of audio clips only: transposition in semitones **added** to
    /// `AudioContent::transpose` (like Live's transposition clip envelope, a modulation),
    /// linear [`-CLIP_TRANSPOSE_RANGE`, `CLIP_TRANSPOSE_RANGE`], normalized 0.5 = no change.
    /// The total is clamped to the clip transpose range.
    ClipTranspose,
}

/// Range of [`AutomationTarget::ClipGain`] (dB, fader law; the bottom is silence).
pub const CLIP_GAIN_MIN_DB: f64 = -144.0;
pub const CLIP_GAIN_MAX_DB: f64 = 6.0;
/// Range of [`AutomationTarget::ClipTranspose`] (± semitones, linear).
pub const CLIP_TRANSPOSE_RANGE: f64 = 48.0;

impl AutomationTarget {
    /// The device this target belongs to, if any.
    pub fn device(&self) -> Option<DeviceId> {
        match *self {
            Self::DeviceParam { device, .. } | Self::DeviceEnabled { device } => Some(device),
            _ => None,
        }
    }

    /// A property of the owning clip itself (only valid on an audio clip's envelope),
    /// rather than of its track.
    pub fn is_clip_property(&self) -> bool {
        matches!(self, Self::ClipGain | Self::ClipTranspose)
    }
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

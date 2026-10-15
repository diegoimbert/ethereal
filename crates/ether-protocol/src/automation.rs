//! Automation lanes and breakpoints.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{
    AutomationLaneId, AutomationOwner, AutomationPointId, AutomationTarget, Beats, CurveShape,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum AutomationCommand {
    /// At most one lane per `(owner, target)`.
    CreateLane {
        id: AutomationLaneId,
        owner: AutomationOwner,
        target: AutomationTarget,
    },
    /// Deletes the lane and its points.
    DeleteLane {
        id: AutomationLaneId,
    },
    SetLaneEnabled {
        id: AutomationLaneId,
        enabled: bool,
    },
    AddPoints {
        lane: AutomationLaneId,
        points: Vec<PointSpec>,
    },
    RemovePoints {
        ids: Vec<AutomationPointId>,
    },
    /// Partial edits (drag = many edits in one gesture).
    EditPoints {
        edits: Vec<PointEdit>,
    },
    /// Remove all points of `lane` in `[start, end)`.
    ClearRange {
        lane: AutomationLaneId,
        start: Beats,
        end: Beats,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PointSpec {
    pub id: AutomationPointId,
    pub time: Beats,
    /// Normalized 0..=1.
    pub value: f64,
    pub curve: CurveShape,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PointEdit {
    pub id: AutomationPointId,
    pub time: Option<Beats>,
    pub value: Option<f64>,
    pub curve: Option<CurveShape>,
}

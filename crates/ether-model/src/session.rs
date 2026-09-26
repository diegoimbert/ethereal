//! Session view: scenes and (derived) clip slots.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::{ClipId, SceneId, TrackId};
use crate::value::{Color, OrderKey, TimeSignature};

/// A scene (row of the session grid). Order among scenes = `order`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Scene {
    pub id: SceneId,
    pub name: String,
    pub color: Option<Color>,
    pub order: OrderKey,
    /// Launching the scene sets the tempo (Ableton scene tempo), if set.
    pub tempo: Option<f64>,
    pub time_signature: Option<TimeSignature>,
}

/// A cell of the session grid. **Derived, not stored**: slots are identified by
/// `(track, scene)` and hold the session clip whose location is `Session { scene }` on that
/// track, if any. Provided for convenience (UI, snapshot compiler).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct ClipSlot {
    pub track: TrackId,
    pub scene: SceneId,
    pub clip: Option<ClipId>,
}

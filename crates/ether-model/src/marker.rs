//! Arrangement markers (locators). Roadmap v2, owned by the `clip-editing` node (see
//! `docs/ROADMAP.md`).
//!
//! A marker is a named position on the arrangement timeline. Markers are plain entities
//! (no parent), edited with `MarkerCommand` and jumped to with `Transport::Locate`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ids::MarkerId;
use crate::value::{Beats, Color};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Marker {
    pub id: MarkerId,
    /// Arrangement position (finite, `>= 0`). Several markers may share a position.
    pub position: Beats,
    pub name: String,
    /// `None` = the theme's default marker color.
    pub color: Option<Color>,
}

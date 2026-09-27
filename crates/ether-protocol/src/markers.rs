//! Arrangement markers (roadmap v2, `clip-editing` node). Undoable document edits on
//! `Project::markers`. Jumping to a marker is `Transport::Locate { position }` (the UI reads
//! the position from its mirror).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{Beats, Color, MarkerId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum MarkerCommand {
    /// Client-chosen id; idempotent on an existing id. `name: None` = "Marker N".
    Add {
        id: MarkerId,
        position: Beats,
        name: Option<String>,
        color: Option<Color>,
    },
    /// Send with a gesture while dragging.
    Move {
        id: MarkerId,
        position: Beats,
    },
    Rename {
        id: MarkerId,
        name: String,
    },
    SetColor {
        id: MarkerId,
        color: Option<Color>,
    },
    Remove {
        ids: Vec<MarkerId>,
    },
}

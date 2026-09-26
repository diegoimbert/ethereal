//! Undo/redo history.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::ModelError;
use crate::op::{Op, Transaction};
use crate::project::Project;

/// Opaque id of a continuous gesture (fader drag, note drag). All transactions committed
/// with the same gesture id are merged into one undo step until the gesture ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub struct GestureId(pub u32);

/// Undo/redo stacks of applied transactions (stored as their inverse ops).
#[derive(Debug, Default)]
pub struct History {
    _private: (),
}

/// What the UI needs to render undo/redo buttons.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct HistoryState {
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
}

impl History {
    pub fn new(max_depth: usize) -> Self {
        let _ = max_depth;
        todo!("model node")
    }

    /// Apply `tx` to `project` and record it. Clears the redo stack. If `gesture` equals the
    /// gesture of the previous transaction (and it hasn't been ended), the two are merged.
    /// Returns the ops actually applied (for patch generation).
    pub fn commit(
        &mut self,
        project: &mut Project,
        tx: Transaction,
        gesture: Option<GestureId>,
    ) -> Result<Vec<Op>, ModelError> {
        let _ = (project, tx, gesture);
        todo!("model node")
    }

    /// Close an open gesture so the next commit starts a new undo step.
    pub fn end_gesture(&mut self, gesture: GestureId) {
        let _ = gesture;
        todo!("model node")
    }

    /// Undo the last step; returns the applied ops (for patches), or `None` if nothing to undo.
    pub fn undo(&mut self, project: &mut Project) -> Result<Option<Vec<Op>>, ModelError> {
        let _ = project;
        todo!("model node")
    }

    pub fn redo(&mut self, project: &mut Project) -> Result<Option<Vec<Op>>, ModelError> {
        let _ = project;
        todo!("model node")
    }

    pub fn clear(&mut self) {
        todo!("model node")
    }

    pub fn state(&self) -> HistoryState {
        todo!("model node")
    }
}

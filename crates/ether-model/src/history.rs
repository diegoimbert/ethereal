//! Undo/redo history.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::ModelError;
use crate::op::{Op, Transaction};
use crate::project::Project;

/// Opaque id of a continuous gesture (fader drag, note drag). All transactions committed
/// with the same gesture id are merged into one undo step until the gesture ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub struct GestureId(pub u32);

/// One undo step.
#[derive(Debug, Clone)]
struct Step {
    label: String,
    /// Ops as applied (in order), for redo.
    forward: Vec<Op>,
    /// Inverse ops, ready to apply in order, for undo.
    inverse: Vec<Op>,
}

/// Undo/redo stacks of applied transactions (stored as their inverse ops).
///
/// - `commit` applies a transaction atomically and pushes one step (clearing redo).
/// - Commits with the same open [`GestureId`] as the previous commit merge into its step.
///   A gesture closes on [`History::end_gesture`], on a commit with another (or no) gesture,
///   and on undo/redo/clear.
/// - At most `max_depth` steps are kept (the oldest are dropped); `0` = unlimited.
#[derive(Debug, Default)]
pub struct History {
    max_depth: usize,
    undo: VecDeque<Step>,
    redo: Vec<Step>,
    open_gesture: Option<GestureId>,
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
        Self {
            max_depth,
            ..Self::default()
        }
    }

    /// Apply `tx` to `project` and record it. Clears the redo stack. If `gesture` equals the
    /// gesture of the previous transaction (and it hasn't been ended), the two are merged.
    /// Returns the ops actually applied (for patch generation).
    ///
    /// On error nothing changes (the project, the stacks and the open gesture). An empty
    /// transaction applies nothing and records nothing.
    pub fn commit(
        &mut self,
        project: &mut Project,
        tx: Transaction,
        gesture: Option<GestureId>,
    ) -> Result<Vec<Op>, ModelError> {
        let mut inverse = project.apply_all(&tx.ops)?;
        if tx.ops.is_empty() {
            return Ok(Vec::new());
        }
        self.redo.clear();
        let merge = gesture.is_some() && gesture == self.open_gesture && !self.undo.is_empty();
        self.open_gesture = gesture;
        if merge {
            let step = self.undo.back_mut().expect("checked non-empty");
            step.forward.extend(tx.ops.iter().cloned());
            inverse.append(&mut step.inverse);
            step.inverse = inverse;
        } else {
            self.undo.push_back(Step {
                label: tx.label,
                forward: tx.ops.clone(),
                inverse,
            });
            if self.max_depth > 0 {
                while self.undo.len() > self.max_depth {
                    self.undo.pop_front();
                }
            }
        }
        Ok(tx.ops)
    }

    /// Close an open gesture so the next commit starts a new undo step.
    pub fn end_gesture(&mut self, gesture: GestureId) {
        if self.open_gesture == Some(gesture) {
            self.open_gesture = None;
        }
    }

    /// Undo the last step; returns the applied ops (for patches), or `None` if nothing to undo.
    /// On error (the document diverged from the history) the step stays on the undo stack.
    pub fn undo(&mut self, project: &mut Project) -> Result<Option<Vec<Op>>, ModelError> {
        self.open_gesture = None;
        let Some(mut step) = self.undo.pop_back() else {
            return Ok(None);
        };
        match project.apply_all(&step.inverse) {
            Ok(forward) => {
                let applied = std::mem::take(&mut step.inverse);
                // Re-derive the forward ops from the actual state (identical in practice).
                step.forward = forward;
                self.redo.push(Step {
                    inverse: applied.clone(),
                    ..step
                });
                Ok(Some(applied))
            }
            Err(e) => {
                self.undo.push_back(step);
                Err(e)
            }
        }
    }

    /// Redo the last undone step; returns the applied ops, or `None` if nothing to redo.
    pub fn redo(&mut self, project: &mut Project) -> Result<Option<Vec<Op>>, ModelError> {
        self.open_gesture = None;
        let Some(step) = self.redo.pop() else {
            return Ok(None);
        };
        match project.apply_all(&step.forward) {
            Ok(inverse) => {
                let applied = step.forward.clone();
                self.undo.push_back(Step {
                    label: step.label,
                    forward: step.forward,
                    inverse,
                });
                Ok(Some(applied))
            }
            Err(e) => {
                self.redo.push(step);
                Err(e)
            }
        }
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.open_gesture = None;
    }

    pub fn state(&self) -> HistoryState {
        HistoryState {
            can_undo: !self.undo.is_empty(),
            can_redo: !self.redo.is_empty(),
            undo_label: self.undo.back().map(|s| s.label.clone()),
            redo_label: self.redo.last().map(|s| s.label.clone()),
        }
    }
}

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
        self.commit_with_inverse(project, tx, gesture)
            .map(|(ops, _)| ops)
    }

    /// [`History::commit`] that also returns this transaction's own inverse ops (ready to
    /// apply in order), whether or not it merged into the previous step (collab: pending
    /// transactions are rebased with them).
    pub fn commit_with_inverse(
        &mut self,
        project: &mut Project,
        tx: Transaction,
        gesture: Option<GestureId>,
    ) -> Result<(Vec<Op>, Vec<Op>), ModelError> {
        let mut inverse = project.apply_all(&tx.ops)?;
        if tx.ops.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        let own_inverse = inverse.clone();
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
        Ok((tx.ops, own_inverse))
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

    /// Undo the last step through `apply` instead of applying its inverse strictly (collab:
    /// per-site undo skips what peers changed since; see `docs/COLLAB.md` §3).
    ///
    /// `apply(project, ops, guards)` gets the step's inverse ops (in application order) and
    /// its forward ops, aligned so that `ops[i]` reverts `guards[len - 1 - i]`. It applies
    /// what it decides and returns `(applied, redo)`: the ops it applied, and the ops that
    /// revert them, in application order (aligned the same way). The redo step is recorded
    /// from them. Returns the applied ops (for patches), or `None` if nothing to undo. On
    /// error nothing is recorded and the step stays on the undo stack (`apply` must leave the
    /// project unchanged when it fails).
    pub fn undo_with<F>(
        &mut self,
        project: &mut Project,
        apply: F,
    ) -> Result<Option<Vec<Op>>, ModelError>
    where
        F: FnOnce(&mut Project, &[Op], &[Op]) -> Result<(Vec<Op>, Vec<Op>), ModelError>,
    {
        self.open_gesture = None;
        let Some(step) = self.undo.pop_back() else {
            return Ok(None);
        };
        match apply(project, &step.inverse, &step.forward) {
            Ok((applied, redo)) => {
                self.redo.push(Step {
                    label: step.label,
                    forward: redo,
                    inverse: applied.clone(),
                });
                Ok(Some(applied))
            }
            Err(e) => {
                self.undo.push_back(step);
                Err(e)
            }
        }
    }

    /// Redo counterpart of [`History::undo_with`]: `apply` gets the step's forward ops and
    /// its undo ops as guards, and returns `(applied, inverse)`.
    pub fn redo_with<F>(
        &mut self,
        project: &mut Project,
        apply: F,
    ) -> Result<Option<Vec<Op>>, ModelError>
    where
        F: FnOnce(&mut Project, &[Op], &[Op]) -> Result<(Vec<Op>, Vec<Op>), ModelError>,
    {
        self.open_gesture = None;
        let Some(step) = self.redo.pop() else {
            return Ok(None);
        };
        match apply(project, &step.forward, &step.inverse) {
            Ok((applied, inverse)) => {
                self.undo.push_back(Step {
                    label: step.label,
                    forward: applied.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::op::SettingsChange;

    fn set_name(name: &str) -> Transaction {
        Transaction {
            label: "Rename".into(),
            ops: vec![Op::Settings {
                change: SettingsChange::Name(name.into()),
            }],
        }
    }

    #[test]
    fn commit_with_inverse_returns_the_own_inverse_when_merging() {
        let mut ids = crate::IdGen::new(1);
        let mut p = Project::new(&mut ids, 1);
        let mut h = History::new(0);
        let g = Some(GestureId(1));
        let (_, inv1) = h.commit_with_inverse(&mut p, set_name("a"), g).unwrap();
        let (_, inv2) = h.commit_with_inverse(&mut p, set_name("b"), g).unwrap();
        assert_eq!(
            inv2,
            vec![Op::Settings {
                change: SettingsChange::Name("a".into())
            }]
        );
        assert_ne!(inv1, inv2);
    }

    #[test]
    fn undo_with_records_what_was_applied() {
        let mut ids = crate::IdGen::new(1);
        let mut p = Project::new(&mut ids, 1);
        let mut h = History::new(0);
        h.commit(&mut p, set_name("a"), None).unwrap();
        // Skip everything: the step moves to redo with nothing in it.
        let applied = h
            .undo_with(&mut p, |_, ops, guards| {
                assert_eq!(ops.len(), guards.len());
                Ok((Vec::new(), Vec::new()))
            })
            .unwrap();
        assert_eq!(applied, Some(vec![]));
        assert_eq!(p.settings.name, "a");
        assert!(h.state().can_redo);
        // Strict application through the closure behaves like `undo`/`redo`.
        h.commit(&mut p, set_name("b"), None).unwrap();
        h.undo_with(&mut p, |p, ops, _| {
            let inv = p.apply_all(ops)?;
            Ok((ops.to_vec(), inv))
        })
        .unwrap();
        assert_eq!(p.settings.name, "a");
        h.redo_with(&mut p, |p, ops, _| {
            let inv = p.apply_all(ops)?;
            Ok((ops.to_vec(), inv))
        })
        .unwrap();
        assert_eq!(p.settings.name, "b");
        // A failing closure leaves the step where it was.
        assert!(
            h.undo_with(&mut p, |_, _, _| Err(ModelError::Invariant("x".into())))
                .is_err()
        );
        assert_eq!(h.state().undo_label.as_deref(), Some("Rename"));
    }
}

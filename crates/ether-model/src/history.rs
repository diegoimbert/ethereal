//! Undo/redo history.

use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::ModelError;
use crate::ids::DeviceId;
use crate::op::{DeviceChange, EntityUpdate, Op, Transaction};
use crate::project::Project;
use crate::text::TextKey;
use crate::value::ParamId;

/// Opaque id of a continuous gesture (fader drag, note drag). All transactions committed
/// with the same gesture id are merged into one undo step until the gesture ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub struct GestureId(pub u32);

/// One undo step.
#[derive(Debug, Clone)]
struct Step {
    /// Monotonic id for the life of this `History` (never reused; merged gesture commits
    /// keep it).
    id: u32,
    /// Clock ([`History::set_now_ms`]) at the step's first commit.
    time_ms: u64,
    /// Checkpoint name ([`History::set_checkpoint`]).
    checkpoint: Option<String>,
    label: String,
    /// The label as a catalog message ([`Transaction::key`]).
    key: Option<TextKey>,
    /// Ops as applied (in order), for redo.
    forward: Vec<Op>,
    /// Inverse ops, ready to apply in order, for undo.
    inverse: Vec<Op>,
    /// Merged gesture commits: the device params set since the step's last structural op,
    /// with the index of their op in `forward` (see [`History::commit_with_inverse`]).
    params: HashMap<(DeviceId, ParamId), usize>,
}

/// `(device, param)` of a device param-value op.
fn param_key(op: &Op) -> Option<(DeviceId, ParamId)> {
    match op {
        Op::Update {
            update:
                EntityUpdate::Device {
                    id,
                    change: DeviceChange::Param { param, .. },
                },
        } => Some((*id, *param)),
        _ => None,
    }
}

/// Undo/redo stacks of applied transactions (stored as their inverse ops).
///
/// - `commit` applies a transaction atomically and pushes one step (clearing redo).
/// - Commits with the same open [`GestureId`] as the previous commit merge into its step.
///   A gesture closes on [`History::end_gesture`], on a commit with another (or no) gesture,
///   and on undo/redo/clear.
/// - At most `max_depth` steps are kept (the oldest are dropped); `0` = unlimited.
/// - Chat ops ([`crate::social::is_untracked`]) are applied by `commit` but never recorded:
///   a transaction of only chat ops pushes nothing, keeps the redo stack and the open
///   gesture.
#[derive(Debug, Default)]
pub struct History {
    max_depth: usize,
    undo: VecDeque<Step>,
    redo: Vec<Step>,
    open_gesture: Option<GestureId>,
    /// Id of the next new step (ids start at 1).
    next_id: u32,
    /// Stamped on new steps (see [`History::set_now_ms`]).
    now_ms: u64,
    /// Steps were dropped from the front since the last [`History::clear`].
    truncated: bool,
    /// Bumped on every change of the steps (see [`History::version`]).
    version: u64,
}

/// One step as listed by [`History::steps`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StepInfo<'a> {
    pub id: u32,
    pub label: &'a str,
    /// The label as a catalog message ([`Transaction::key`]).
    pub key: Option<&'a TextKey>,
    /// [`History::set_now_ms`] clock at the step's first commit.
    pub time_ms: u64,
    /// On the redo stack.
    pub undone: bool,
    /// Checkpoint name ([`History::set_checkpoint`]).
    pub checkpoint: Option<&'a str>,
}

/// What the UI needs to render undo/redo buttons.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct HistoryState {
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
    /// `undo_label` as a catalog message, shown in the user's language (docs/LOCALIZATION.md).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub undo_key: Option<TextKey>,
    /// `redo_label` as a catalog message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub redo_key: Option<TextKey>,
    /// Id of the step `Undo` would undo (unique per session): lets the UI tie its own view
    /// state (e.g. the selection when the step was made) to undo / redo.
    #[serde(default)]
    pub undo_id: Option<u32>,
    /// Id of the step `Redo` would redo.
    #[serde(default)]
    pub redo_id: Option<u32>,
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
        // Chat ops (`social::is_untracked`) are applied but never recorded: a transaction of
        // only those leaves the stacks and the open gesture as they were.
        let tracked = |op: &Op| !op.key().is_some_and(crate::social::is_untracked);
        let forward: Vec<Op> = tx.ops.iter().filter(|op| tracked(op)).cloned().collect();
        inverse.retain(tracked);
        if forward.is_empty() {
            return Ok((tx.ops, own_inverse));
        }
        self.redo.clear();
        self.version = self.version.wrapping_add(1);
        let merge = gesture.is_some() && gesture == self.open_gesture && !self.undo.is_empty();
        self.open_gesture = gesture;
        if merge {
            // A gesture can merge commits for minutes (a plugin reporting edits on every
            // tick): repeated param sets coalesce, the newest value kept for redo and the
            // oldest for undo, so the step stays bounded and a merge costs only its own ops.
            // Forward op `j` and inverse op `len - 1 - j` are a pair (`undo_with`): a
            // coalesced op drops both. A structural op ends coalescing (later ops may
            // depend on it).
            let step = self.undo.back_mut().expect("checked non-empty");
            let n = forward.len();
            let mut keep = vec![true; n];
            for (j, op) in forward.into_iter().enumerate() {
                match param_key(&op) {
                    Some(key) => match step.params.get(&key) {
                        Some(&i) => {
                            step.forward[i] = op;
                            keep[j] = false;
                        }
                        None => {
                            step.params.insert(key, step.forward.len());
                            step.forward.push(op);
                        }
                    },
                    None => {
                        step.params.clear();
                        step.forward.push(op);
                    }
                }
            }
            if inverse.len() == n {
                let mut j = n;
                inverse.retain(|_| {
                    j -= 1;
                    keep[j]
                });
            }
            if !inverse.is_empty() {
                inverse.append(&mut step.inverse);
                step.inverse = inverse;
            }
        } else {
            self.next_id = self.next_id.max(1);
            let id = self.next_id;
            self.next_id = self.next_id.wrapping_add(1);
            let mut params = HashMap::new();
            for (i, op) in forward.iter().enumerate() {
                match param_key(op) {
                    Some(key) => {
                        params.entry(key).or_insert(i);
                    }
                    None => params.clear(),
                }
            }
            self.undo.push_back(Step {
                id,
                time_ms: self.now_ms,
                checkpoint: None,
                label: tx.label,
                key: tx.key,
                forward,
                inverse,
                params,
            });
            if self.max_depth > 0 {
                while self.undo.len() > self.max_depth {
                    self.undo.pop_front();
                    self.truncated = true;
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

    /// `gesture` is open: the last undo step is its (merged) commits.
    pub fn is_open_gesture(&self, gesture: GestureId) -> bool {
        self.open_gesture == Some(gesture)
    }

    /// Undo the last step; returns the applied ops (for patches), or `None` if nothing to undo.
    /// On error (the document diverged from the history) the step stays on the undo stack.
    pub fn undo(&mut self, project: &mut Project) -> Result<Option<Vec<Op>>, ModelError> {
        self.open_gesture = None;
        self.version = self.version.wrapping_add(1);
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
                    params: HashMap::new(),
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
        self.version = self.version.wrapping_add(1);
        let Some(step) = self.redo.pop() else {
            return Ok(None);
        };
        match project.apply_all(&step.forward) {
            Ok(inverse) => {
                let applied = step.forward.clone();
                self.undo.push_back(Step {
                    inverse,
                    params: HashMap::new(),
                    ..step
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
    /// per-site undo skips what peers changed since).
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
        self.version = self.version.wrapping_add(1);
        let Some(step) = self.undo.pop_back() else {
            return Ok(None);
        };
        match apply(project, &step.inverse, &step.forward) {
            Ok((applied, redo)) => {
                self.redo.push(Step {
                    forward: redo,
                    inverse: applied.clone(),
                    params: HashMap::new(),
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
        self.version = self.version.wrapping_add(1);
        let Some(step) = self.redo.pop() else {
            return Ok(None);
        };
        match apply(project, &step.forward, &step.inverse) {
            Ok((applied, inverse)) => {
                self.undo.push_back(Step {
                    forward: applied.clone(),
                    inverse,
                    params: HashMap::new(),
                    ..step
                });
                Ok(Some(applied))
            }
            Err(e) => {
                self.redo.push(step);
                Err(e)
            }
        }
    }

    /// Clear both stacks. Step ids keep counting (never reused).
    /// Forget the last undone step, so redo can't bring it back (an edit its caller rolled
    /// back with [`Self::undo`] because the whole operation failed). Returns whether there
    /// was one.
    pub fn discard_redo(&mut self) -> bool {
        let dropped = self.redo.pop().is_some();
        if dropped {
            self.version = self.version.wrapping_add(1);
        }
        dropped
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.open_gesture = None;
        self.version = self.version.wrapping_add(1);
        self.truncated = false;
    }

    /// Set the clock stamped on the next new steps (the controller's wall clock, Unix ms).
    pub fn set_now_ms(&mut self, now_ms: u64) {
        self.now_ms = now_ms;
    }

    /// The steps as a timeline: the applied steps oldest first, then the undone steps in
    /// the order they would be redone.
    pub fn steps(&self) -> impl Iterator<Item = StepInfo<'_>> {
        fn info(s: &Step, undone: bool) -> StepInfo<'_> {
            StepInfo {
                id: s.id,
                label: &s.label,
                key: s.key.as_ref(),
                time_ms: s.time_ms,
                undone,
                checkpoint: s.checkpoint.as_deref(),
            }
        }
        self.undo
            .iter()
            .map(|s| info(s, false))
            .chain(self.redo.iter().rev().map(|s| info(s, true)))
    }

    /// Name step `id` (a checkpoint), or clear its name. Returns `false` (and changes
    /// nothing) if there is no such step. The name lives as long as the step.
    pub fn set_checkpoint(&mut self, id: u32, name: Option<String>) -> bool {
        match self
            .undo
            .iter_mut()
            .chain(self.redo.iter_mut())
            .find(|s| s.id == id)
        {
            Some(step) => {
                step.checkpoint = name;
                self.version = self.version.wrapping_add(1);
                true
            }
            None => false,
        }
    }

    /// Changes whenever the steps (or their checkpoints) may have changed: a cheap check
    /// before listing them again. Starts at 0 for a new `History`.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// The last applied step (`None` = nothing to undo).
    pub fn current_step(&self) -> Option<u32> {
        self.undo.back().map(|s| s.id)
    }

    /// Steps were dropped from the front (history depth) since the last clear.
    pub fn is_truncated(&self) -> bool {
        self.truncated
    }

    /// How to make `step` the last applied step: `Some(-n)` = undo `n` times, `Some(n)` =
    /// redo `n` times, `Some(0)` = already there; `None` = no such step (dropped or never
    /// existed). `step: None` means before the oldest kept step (undo everything).
    pub fn distance_to(&self, step: Option<u32>) -> Option<i64> {
        let Some(id) = step else {
            return Some(-(self.undo.len() as i64));
        };
        if let Some(i) = self.undo.iter().position(|s| s.id == id) {
            return Some(-((self.undo.len() - 1 - i) as i64));
        }
        // `redo` is a stack: its last step is redone first.
        let j = self.redo.iter().position(|s| s.id == id)?;
        Some((self.redo.len() - j) as i64)
    }

    pub fn state(&self) -> HistoryState {
        HistoryState {
            can_undo: !self.undo.is_empty(),
            can_redo: !self.redo.is_empty(),
            undo_label: self.undo.back().map(|s| s.label.clone()),
            redo_label: self.redo.last().map(|s| s.label.clone()),
            undo_key: self.undo.back().and_then(|s| s.key.clone()),
            redo_key: self.redo.last().and_then(|s| s.key.clone()),
            undo_id: self.undo.back().map(|s| s.id),
            redo_id: self.redo.last().map(|s| s.id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::op::SettingsChange;

    fn set_name(name: &str) -> Transaction {
        Transaction {
            key: None,
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

    #[test]
    fn discard_redo_forgets_the_undone_step() {
        let mut ids = crate::IdGen::new(1);
        let mut p = Project::new(&mut ids, 1);
        let mut h = History::new(0);
        h.commit(&mut p, set_name("a"), None).unwrap();
        h.undo(&mut p).unwrap();
        assert!(h.state().can_redo);
        let v = h.version();
        assert!(h.discard_redo());
        assert!(!h.state().can_redo && !h.state().can_undo);
        assert_ne!(h.version(), v);
        assert!(!h.discard_redo());
        assert_eq!(h.redo(&mut p).unwrap(), None);
    }

    fn ids(h: &History) -> Vec<(u32, bool)> {
        h.steps().map(|s| (s.id, s.undone)).collect()
    }

    #[test]
    fn steps_read_as_a_timeline_with_stable_ids_and_first_commit_times() {
        let mut ids_gen = crate::IdGen::new(1);
        let mut p = Project::new(&mut ids_gen, 1);
        let mut h = History::new(0);
        assert_eq!(h.current_step(), None);
        h.set_now_ms(100);
        h.commit(&mut p, set_name("a"), Some(GestureId(7))).unwrap();
        h.set_now_ms(150);
        // Merged into the same step: same id, first-commit time.
        h.commit(&mut p, set_name("a2"), Some(GestureId(7)))
            .unwrap();
        h.set_now_ms(200);
        h.commit(&mut p, set_name("b"), None).unwrap();
        h.set_now_ms(300);
        h.commit(&mut p, set_name("c"), None).unwrap();
        let steps: Vec<_> = h.steps().map(|s| (s.id, s.time_ms)).collect();
        assert_eq!(steps, vec![(1, 100), (2, 200), (3, 300)]);
        assert_eq!(h.current_step(), Some(3));
        h.undo(&mut p).unwrap();
        h.undo(&mut p).unwrap();
        // Undone steps follow in redo order.
        assert_eq!(ids(&h), vec![(1, false), (2, true), (3, true)]);
        assert_eq!(h.current_step(), Some(1));
        assert_eq!(h.distance_to(Some(3)), Some(2));
        assert_eq!(h.distance_to(Some(2)), Some(1));
        assert_eq!(h.distance_to(Some(1)), Some(0));
        assert_eq!(h.distance_to(None), Some(-1));
        assert_eq!(h.distance_to(Some(9)), None);
        h.redo(&mut p).unwrap();
        assert_eq!(ids(&h), vec![(1, false), (2, false), (3, true)]);
        assert_eq!(h.steps().nth(1).unwrap().time_ms, 200);
        assert_eq!(h.distance_to(Some(1)), Some(-1));
        // A new commit drops the redo stack; ids are never reused.
        h.commit(&mut p, set_name("d"), None).unwrap();
        assert_eq!(ids(&h), vec![(1, false), (2, false), (4, false)]);
        h.clear();
        h.commit(&mut p, set_name("e"), None).unwrap();
        assert_eq!(ids(&h), vec![(5, false)]);
    }

    #[test]
    fn dropped_steps_mark_the_history_truncated() {
        let mut ids_gen = crate::IdGen::new(1);
        let mut p = Project::new(&mut ids_gen, 1);
        let mut h = History::new(2);
        h.commit(&mut p, set_name("a"), None).unwrap();
        h.commit(&mut p, set_name("b"), None).unwrap();
        assert!(!h.is_truncated());
        h.commit(&mut p, set_name("c"), None).unwrap();
        assert!(h.is_truncated());
        assert_eq!(ids(&h), vec![(2, false), (3, false)]);
        assert_eq!(h.distance_to(Some(1)), None);
        h.clear();
        assert!(!h.is_truncated());
    }

    #[test]
    fn guarded_undo_and_redo_keep_the_step_id() {
        let mut ids_gen = crate::IdGen::new(1);
        let mut p = Project::new(&mut ids_gen, 1);
        let mut h = History::new(0);
        h.set_now_ms(42);
        h.commit(&mut p, set_name("a"), None).unwrap();
        h.undo_with(&mut p, |_, _, _| Ok((Vec::new(), Vec::new())))
            .unwrap();
        assert_eq!(ids(&h), vec![(1, true)]);
        assert!(h.set_checkpoint(1, Some("Before".into())));
        assert!(!h.set_checkpoint(2, Some("Nope".into())));
        h.redo_with(&mut p, |_, _, _| Ok((Vec::new(), Vec::new())))
            .unwrap();
        assert_eq!(ids(&h), vec![(1, false)]);
        let s = h.steps().next().unwrap();
        assert_eq!((s.time_ms, s.checkpoint), (42, Some("Before")));
        h.undo(&mut p).unwrap();
        h.redo(&mut p).unwrap();
        assert_eq!(h.steps().next().unwrap().checkpoint, Some("Before"));
        let v = h.version();
        assert!(h.set_checkpoint(1, None));
        assert_eq!(h.steps().next().unwrap().checkpoint, None);
        assert_ne!(h.version(), v, "a checkpoint change is a change");
        let v = h.version();
        assert!(!h.set_checkpoint(9, None));
        h.end_gesture(GestureId(1));
        assert_eq!(h.version(), v);
        h.commit(&mut p, set_name("z"), None).unwrap();
        assert_ne!(h.version(), v);
    }
}

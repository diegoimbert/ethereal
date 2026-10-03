//! Undo history panel (v0.3, owned by the `undo-history` node; protocol
//! `ether_protocol::undo_history`, CONTRACTS.md §13.8).
//!
//! - [`EtherController::history_command`]: `History::{List, JumpTo, SetCheckpoint}`
//!   (dispatched from `handlers.rs`). `JumpTo` undoes/redoes step by step through the same
//!   path as `Edit::{Undo, Redo}` ([`EtherController::undo_redo_step`]; per-site in a collab
//!   session: the history only holds this site's own steps, peers' edits are applied outside
//!   it and survive) and emits one patch for the whole jump.
//! - Step ids, first-commit times and checkpoint names live on the steps of
//!   `ether_model::History` (additive accessors): they are dropped with their step and a
//!   new history starts with each opened project.
//! - [`EtherController::history_tick`]: `HistoryEvent::Changed` (at most every 100 ms, only
//!   when the list changed) once a client listed the history.

use ether_core::protocol::undo_history::{
    HistoryCommand, HistoryEvent, HistoryList, HistoryStep, HistoryStepId,
};
use ether_core::protocol::{Event, ReplyValue, ServerMessage};
use ether_model::History;

use crate::handlers::no_project;
use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, invalid, not_found};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// Minimum time between two `HistoryEvent::Changed`.
pub(crate) const CHANGED_INTERVAL_MS: u64 = 100;
/// Longest checkpoint name kept (characters).
pub(crate) const MAX_CHECKPOINT_CHARS: usize = 120;

/// Runtime state of the history panel.
#[derive(Debug, Default)]
pub(crate) struct UndoHistoryState {
    /// A client listed the history: changes are pushed from then on.
    listening: bool,
    /// The list as the clients last saw it (reply or event).
    last_sent: Option<HistoryList>,
    last_sent_ms: Option<u64>,
}

/// The protocol view of `history`.
pub(crate) fn history_list(history: &History) -> HistoryList {
    HistoryList {
        steps: history
            .steps()
            .map(|s| HistoryStep {
                id: s.id,
                label: s.label.to_string(),
                time_ms: s.time_ms,
                undone: s.undone,
                checkpoint: s.checkpoint.map(str::to_string),
            })
            .collect(),
        current: history.current_step(),
        truncated: history.is_truncated(),
    }
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    fn current_history_list(&self) -> HistoryList {
        self.doc
            .as_ref()
            .map(|d| history_list(&d.history))
            .unwrap_or_default()
    }

    /// Reply with the current list, which the clients now know.
    fn history_reply(&mut self, now: u64) -> ReplyValue {
        let history = self.current_history_list();
        let st = &mut self.undo_history;
        st.listening = true;
        st.last_sent = Some(history.clone());
        st.last_sent_ms = Some(now);
        ReplyValue::History { history }
    }

    pub(crate) fn history_command(
        &mut self,
        command: &HistoryCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        match command {
            HistoryCommand::List => Ok(self.history_reply(now)),
            HistoryCommand::JumpTo { step } => {
                self.history_jump(*step, now, out)?;
                Ok(self.history_reply(now))
            }
            HistoryCommand::SetCheckpoint { step, name } => {
                let name = match name.as_deref().map(str::trim) {
                    None | Some("") => None,
                    Some(n) if n.chars().count() > MAX_CHECKPOINT_CHARS => {
                        return Err(invalid(format!(
                            "a checkpoint name is at most {MAX_CHECKPOINT_CHARS} characters"
                        )));
                    }
                    Some(n) => Some(n.to_string()),
                };
                let doc = self.doc.as_mut().ok_or_else(no_project)?;
                if !doc.history.set_checkpoint(*step, name) {
                    return Err(not_found(format!("history step {step}")));
                }
                // Pushed by the next tick to the listening clients.
                Ok(ReplyValue::Unit)
            }
        }
    }

    /// Undo/redo until `step` is the last applied step; one patch for the whole jump.
    fn history_jump(
        &mut self,
        step: Option<HistoryStepId>,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<()> {
        let doc = self.doc.as_ref().ok_or_else(no_project)?;
        let distance = doc.history.distance_to(step).ok_or_else(|| {
            not_found(format!(
                "history step {} (dropped or unknown)",
                step.map_or_else(|| "none".to_string(), |s| s.to_string())
            ))
        })?;
        if distance == 0 {
            return Ok(());
        }
        let undo = distance < 0;
        let mut applied = Vec::new();
        let mut result = Ok(());
        let mut done = 0;
        for _ in 0..distance.unsigned_abs() {
            match self.undo_redo_step(undo) {
                Ok(Some(mut ops)) => {
                    applied.append(&mut ops);
                    done += 1;
                }
                Ok(None) => break,
                Err(e) => {
                    // What was already undone/redone stays (and is patched below).
                    result = Err(e);
                    break;
                }
            }
        }
        if done > 0 {
            self.transport.tap_gesture = None;
            // Also when no op was applied (a collab step whose changes peers all
            // overwrote): the patch carries the new undo/redo state, like `Edit::Undo`.
            self.after_ops(&applied, now, out);
        }
        result
    }

    /// Called every tick.
    pub(crate) fn history_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        let st = &self.undo_history;
        if !st.listening
            || st
                .last_sent_ms
                .is_some_and(|t| now.saturating_sub(t) < CHANGED_INTERVAL_MS)
        {
            return;
        }
        let history = self.current_history_list();
        let st = &mut self.undo_history;
        if st.last_sent.as_ref() == Some(&history) {
            return;
        }
        st.last_sent = Some(history.clone());
        st.last_sent_ms = Some(now);
        out.send(ServerMessage::Event(Event::History {
            event: HistoryEvent::Changed { history },
        }));
    }
}

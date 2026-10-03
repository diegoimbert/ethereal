//! Undo history panel (v0.3, owned by the `undo-history` node; protocol
//! `ether_protocol::undo_history`, CONTRACTS.md §13.8).
//!
//! - [`EtherController::history_command`]: `History::{List, JumpTo, SetCheckpoint}`
//!   (dispatched from `handlers.rs`). `JumpTo` undoes/redoes step by step through the same
//!   path as `Edit::{Undo, Redo}` (per-site in a collab session: own steps only).
//! - Step ids and times: `ether_model::History` gains a step id and first-commit time per
//!   step (shared touch: `crates/ether-model/src/history.rs`, additive accessors; merged
//!   gesture commits keep their step's id).
//! - [`EtherController::history_tick`]: `HistoryEvent::Changed` (throttled to 100 ms) while
//!   a client listed the history; checkpoints are cleared on project change.
//!
//! Until the node lands: commands reply `Unsupported` (tests/roadmap_v4.rs).

use ether_core::protocol::ReplyValue;
use ether_core::protocol::undo_history::HistoryCommand;

use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, unsupported};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    pub(crate) fn history_command(
        &mut self,
        command: &HistoryCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let _ = (command, now, out);
        Err(unsupported(
            "the history panel is not implemented yet (undo-history)",
        ))
    }

    /// Called every tick.
    pub(crate) fn history_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        let _ = (now, out);
    }
}

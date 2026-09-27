//! Time-selection edits across tracks (v0.2, owned by the `time-edits` node; protocol
//! `ether_protocol::time_edit`, CONTRACTS.md §12.3).
//!
//! [`EtherController::time_edit_command`] (dispatched from `handlers.rs`): `Copy` fills the
//! controller's time clipboard (runtime state, add it to this module); every other command is
//! one document transaction through `edit_with`, with new ids from `derive_id(seed, i)`.
//! Placeholder: `Unsupported`.

use ether_core::protocol::ReplyValue;
use ether_core::protocol::model::GestureId;
use ether_core::protocol::time_edit::TimeEditCommand;

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
    pub(crate) fn time_edit_command(
        &mut self,
        command: &TimeEditCommand,
        gesture: Option<GestureId>,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let _ = (command, gesture, now, out);
        Err(unsupported(
            "time edits are not implemented yet (time-edits)",
        ))
    }
}

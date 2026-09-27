//! External media references: import in place, missing media, relink, collect (v0.2, owned
//! by the `media-references` node; model `ether_model::MediaLocation`, protocol
//! `ether_protocol::media_refs`, CONTRACTS.md §12.9).
//!
//! [`EtherController::media_ref_command`] (dispatched from `handlers.rs`) and
//! [`EtherController::media_refs_tick`]. Shared touches: `media/` (resolution order: external
//! path with matching hash → project copy at `MediaRef::file` → missing; import of
//! `MediaSource::Location` becomes a reference via `Library::external_path`), the stores
//! (`Library::read_external`), collab media transfer by hash. Placeholder: `Unsupported`,
//! imports still copy (v0.1 behaviour).

use ether_core::protocol::ReplyValue;
use ether_core::protocol::media_refs::MediaRefCommand;

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
    pub(crate) fn media_ref_command(
        &mut self,
        command: &MediaRefCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let _ = (command, now, out);
        Err(unsupported(
            "external media references are not implemented yet (media-references)",
        ))
    }

    /// Called every tick (background missing-media checks and searches).
    pub(crate) fn media_refs_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        let _ = (now, out);
    }
}

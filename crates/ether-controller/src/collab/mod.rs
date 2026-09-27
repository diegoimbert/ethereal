//! Collaboration (RESERVED; roadmap v2, owned by the `collab` node; see `docs/ROADMAP.md`,
//! `ether_protocol::collab` and the `ether-collab` crate).
//!
//! Every `CollabCommand` replies `Unsupported` until the collab node lands. Its intended
//! hooks: stamp committed transactions (`ether_model::OpOrigin`) in `History::commit`'s
//! caller (`handlers.rs::edit_with`), apply remote transactions as non-undoable ops with
//! patches, and publish presence as `Event::Collab`.

use ether_core::protocol::ReplyValue;
use ether_core::protocol::collab::CollabCommand;

use crate::store::{Library, ProjectStore};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};
use crate::tx::{CmdResult, unsupported};

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    pub(crate) fn collab_command(
        &mut self,
        c: &CollabCommand,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let _ = (c, out);
        Err(unsupported("collaboration is not available yet (collab node)"))
    }
}

//! Presence v2 (`presence-v2` node; docs/COLLAB.md §8): the high-rate arranger pointer.
//! The other presence v2 fields (viewport, activity, following) travel in `PresenceState`
//! through the existing `SetPresence` path and need no controller code.
//!
//! Pre-wired by base-53 (dispatch in `collab/mod.rs`); until the node lands `SetPointer`
//! replies `Unsupported` and incoming pointers are ignored.
//!
//! To do: throttle `SetPointer` to `POINTER_MAX_HZ` (keep the latest, always send a clear,
//! send it on the next tick if throttled), send `CollabMessage::Pointer`; emit
//! `CollabEvent::Pointer` for peers' pointers (dropping our own), and a clear
//! (`pointer: None`) when a peer leaves.

use ether_core::protocol::ReplyValue;
use ether_core::protocol::collab::ArrangerPointer;
use ether_core::protocol::model::SiteId;

use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, unsupported};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// Pointer state of a session (one field of `collab::Session`).
#[derive(Default)]
pub(crate) struct PointerState {}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// `CollabCommand::SetPointer`.
    pub(crate) fn collab_set_pointer(
        &mut self,
        pointer: &Option<ArrangerPointer>,
        now: u64,
    ) -> CmdResult<ReplyValue> {
        let _ = (pointer, now);
        Err(unsupported("live pointers are not implemented yet"))
    }

    /// A peer's `CollabMessage::Pointer` (relay-stamped `site`).
    pub(crate) fn collab_pointer_message(
        &mut self,
        site: SiteId,
        pointer: Option<ArrangerPointer>,
        out: &mut dyn MessageSink,
    ) {
        let _ = (site, pointer, out);
    }

    /// Every collab tick (flush a throttled pointer).
    pub(crate) fn collab_pointer_tick(&mut self, now: u64) {
        let _ = now;
    }

    /// Peer `site` left the session.
    pub(crate) fn collab_pointer_peer_left(&mut self, site: SiteId, out: &mut dyn MessageSink) {
        let _ = (site, out);
    }
}

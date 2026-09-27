//! Chat, pinned notes and peers' playheads (base-62 contract, node `collab-social`;
//! docs/COLLAB.md §12). Stubs until the node lands: `Chat::*` and `PinnedNote::*` reply
//! `Unsupported` (pinned in `tests/social_prewire.rs`) and our presence carries no
//! `transport`.
//!
//! What the node implements here:
//! - [`EtherController::chat_command`]: `Send` → validate, author from the session (name,
//!   site, own relay colour), `sent_at` from the host clock, then one `edit_with` transaction:
//!   `Insert ChatMessage { seq: 0 }` + `Remove` of `Project::chat_overflow(CHAT_MAX_MESSAGES)`.
//!   `History` never records chat ops (`ether_model::social::is_untracked`), so the message
//!   is stamped and sent like any edit but is never an undo step.
//! - [`pinned_note_command`]: `Add`/`Edit`/`Delete` as ordinary undoable document edits (the
//!   `MarkerCommand` pattern in `clip_editing`), in a session or not.
//! - [`EtherController::social_presence`]: the controller-owned `PresenceState::transport`
//!   (position, playing, loop region, host-clock `sent_at_ms`), refreshed every
//!   `PEER_TRANSPORT_REFRESH_MS` while playing and at once on play/stop/locate/loop.
//! - `CollabEvent::ChatReceived` for peers' live messages (after the join catch-up).

use ether_core::protocol::ReplyValue;
use ether_core::protocol::collab::PresenceState;
use ether_core::protocol::model::{Entity, Op, SiteId};
use ether_core::protocol::social::{ChatCommand, PinnedNoteCommand};

use crate::doc::DocCtx;
use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, unsupported};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// Receive-side sanitising of a peer's transaction (docs/COLLAB.md §12.1): an
/// `Insert ChatMessage` is dropped unless its `author.site` is the relay-verified
/// `origin` and its `seq` is 0 (senders always send 0; `Project::apply` numbers it). No
/// legitimate path sends anything else: chat is never undone or redone. Deterministic
/// (a function of the op and the stamped origin), so every replica drops the same ops.
/// Notes cannot use this rule (an undo re-inserts another user's note): note authorship is
/// best-effort.
pub(crate) fn is_forged_chat(op: &Op, origin: SiteId) -> bool {
    matches!(
        op,
        Op::Insert { entity: Entity::ChatMessage(m) }
            if m.author.site != Some(origin) || m.seq != 0
    )
}

/// `PinnedNoteCommand` (a document command, from `doc::apply`).
pub(crate) fn pinned_note_command(ctx: &mut DocCtx, c: &PinnedNoteCommand) -> CmdResult<()> {
    let _ = (ctx, c);
    Err(unsupported("pinned notes are not implemented yet"))
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// `Command::Chat` (not a document command).
    pub(crate) fn chat_command(
        &mut self,
        c: &ChatCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let _ = (c, now, out);
        Err(unsupported("chat is not implemented yet"))
    }

    /// Set the controller-owned `transport` of our outgoing presence.
    pub(crate) fn social_presence(&self, state: &mut PresenceState) {
        state.transport = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ether_core::protocol::model::{Author, ChatMessage, ChatMessageId};

    fn insert(site: Option<SiteId>, seq: u64) -> Op {
        Op::Insert {
            entity: Entity::ChatMessage(ChatMessage {
                id: ChatMessageId::NIL,
                seq,
                author: Author {
                    name: "B".into(),
                    site,
                    actor: None,
                    color: None,
                },
                text: "hi".into(),
                sent_at: 0,
            }),
        }
    }

    #[test]
    fn only_well_formed_chat_inserts_pass() {
        let origin = SiteId(2);
        assert!(!is_forged_chat(&insert(Some(origin), 0), origin));
        assert!(is_forged_chat(&insert(Some(SiteId(1)), 0), origin));
        assert!(is_forged_chat(&insert(None, 0), origin));
        assert!(is_forged_chat(&insert(Some(origin), 3), origin));
        // Prunes (removes) are never filtered.
        let prune = Op::Remove {
            key: ether_core::protocol::model::EntityKey::ChatMessage(ChatMessageId::NIL),
        };
        assert!(!is_forged_chat(&prune, origin));
    }
}

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
use std::collections::BTreeSet;

use ether_core::protocol::model::{
    CHAT_MAX_MESSAGES, ChatMessageId, Entity, EntityKey, Op, Project, SiteId,
};
use ether_core::protocol::social::{ChatCommand, PinnedNoteCommand};

use crate::doc::DocCtx;
use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, unsupported};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// A chat insert that is not a well-formed message by `origin`: its `author.site` must be
/// the relay-verified sender and its `seq` 0 (senders always send 0; `Project::apply`
/// numbers it). No legitimate path sends anything else: chat is never undone or redone.
pub(crate) fn is_forged_chat(op: &Op, origin: SiteId) -> bool {
    matches!(
        op,
        Op::Insert { entity: Entity::ChatMessage(m) }
            if m.author.site != Some(origin) || m.seq != 0
    )
}

fn is_chat(op: &Op) -> bool {
    matches!(op.key(), Some(EntityKey::ChatMessage(_)))
}

/// Chat sanitising of a sequenced transaction by `origin` (docs/COLLAB.md §12.1), applied
/// to `project` **as it is right before the transaction** (the confirmed state for a peer's
/// transaction or an own echo; the rebase state for a re-applied pending one). A function of
/// that state, the ops and the relay-verified origin, so every replica keeps the same ops.
/// Chat is a journal: nobody edits or deletes other people's messages.
/// - forged inserts ([`is_forged_chat`]) are dropped;
/// - any `Update` of a chat message is dropped (sent messages are final; the model has no
///   chat update today, this keeps it so if one is ever added);
/// - the chat `Remove`s are kept only as a **prune**: the transaction also inserts a
///   well-formed message by `origin`, the removed messages that still exist are the oldest
///   ones (a prefix of `Project::chat_ordered`), and removing them leaves at least
///   `CHAT_MAX_MESSAGES` messages once the new one is in. Otherwise all its chat `Remove`s
///   are dropped (a concurrent prune may have done the job; the next send prunes again).
///
/// Notes cannot be checked like this (an undo legitimately re-inserts or deletes another
/// user's note): note authorship is best-effort.
pub(crate) fn sanitize_chat(project: &Project, ops: &[Op], origin: SiteId) -> Vec<Op> {
    let mut out: Vec<Op> = ops
        .iter()
        .filter(|op| !is_forged_chat(op, origin))
        .filter(|op| !(is_chat(op) && matches!(op, Op::Update { .. })))
        .cloned()
        .collect();
    let removed: Vec<ChatMessageId> = out
        .iter()
        .filter_map(|op| match op {
            Op::Remove {
                key: EntityKey::ChatMessage(id),
            } => Some(*id),
            _ => None,
        })
        .collect();
    if removed.is_empty() {
        return out;
    }
    let inserts = out.iter().any(|op| {
        matches!(
            op,
            Op::Insert {
                entity: Entity::ChatMessage(_)
            }
        )
    });
    let existing: BTreeSet<ChatMessageId> = removed
        .iter()
        .copied()
        .filter(|id| project.chat.contains_key(id))
        .collect();
    let oldest: BTreeSet<ChatMessageId> = project
        .chat_ordered()
        .iter()
        .take(existing.len())
        .map(|m| m.id)
        .collect();
    let left = project.chat.len() - existing.len() + 1;
    let prune = inserts && existing == oldest && left >= CHAT_MAX_MESSAGES;
    if !prune {
        out.retain(|op| !(is_chat(op) && matches!(op, Op::Remove { .. })));
    }
    out
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

    fn with_id(op: Op, id: ChatMessageId) -> Op {
        let Op::Insert {
            entity: Entity::ChatMessage(mut m),
        } = op
        else {
            unreachable!()
        };
        m.id = id;
        Op::Insert {
            entity: Entity::ChatMessage(m),
        }
    }

    fn remove(id: ChatMessageId) -> Op {
        Op::Remove {
            key: EntityKey::ChatMessage(id),
        }
    }

    /// A project whose chat holds `n` messages; returns their ids, oldest first.
    fn chat_of(n: usize) -> (Project, Vec<ChatMessageId>) {
        let mut ids = ether_core::protocol::model::IdGen::new(3);
        let mut p = Project::new(&mut ids, 1);
        let mut out = Vec::new();
        for _ in 0..n {
            let id: ChatMessageId = ids.next(2);
            p.apply(&with_id(insert(Some(SiteId(9)), 0), id)).unwrap();
            out.push(id);
        }
        (p, out)
    }

    #[test]
    fn chat_removes_are_kept_only_as_a_prune() {
        let origin = SiteId(2);
        let (p, ids) = chat_of(CHAT_MAX_MESSAGES);
        let new = with_id(
            insert(Some(origin), 0),
            ChatMessageId(ether_core::protocol::model::Ulid(u128::MAX)),
        );
        let kept = |ops: &[Op]| {
            sanitize_chat(&p, ops, origin)
                .iter()
                .filter(|op| matches!(op, Op::Remove { .. }))
                .count()
        };
        // At the cap, a message plus the oldest removed: a prune.
        assert_eq!(kept(&[new.clone(), remove(ids[0])]), 1);
        // Removing an already-gone message along the oldest one still counts as a prefix.
        assert_eq!(
            kept(&[new.clone(), remove(ChatMessageId::NIL), remove(ids[0])]),
            2
        );
        // Not the oldest: dropped.
        assert_eq!(kept(&[new.clone(), remove(ids[5])]), 0);
        // Without a message of the sender: dropped.
        assert_eq!(kept(&[remove(ids[0])]), 0);
        // A forged message doesn't make it a prune.
        let forged = with_id(
            insert(Some(SiteId(7)), 0),
            ChatMessageId(ether_core::protocol::model::Ulid(1)),
        );
        assert_eq!(kept(&[forged, remove(ids[0])]), 0);
        // Going below the cap: dropped.
        assert_eq!(kept(&[new.clone(), remove(ids[0]), remove(ids[1])]), 0);
        // Other ops of the transaction are untouched.
        let settings = Op::Settings {
            change: ether_core::protocol::model::SettingsChange::Swing(0.5),
        };
        assert_eq!(
            sanitize_chat(&p, &[settings.clone(), remove(ids[3])], origin),
            [settings]
        );
    }
}

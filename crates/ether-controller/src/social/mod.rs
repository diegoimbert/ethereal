//! Chat, pinned notes and peers' playheads (base-62 contract, node `collab-social`;
//! docs/COLLAB.md §12).
//!
//! - [`EtherController::chat_command`]: `Send` → validate, author from the session (name,
//!   site, own relay colour), `sent_at` from the host clock, then one `edit_with` transaction:
//!   `Insert ChatMessage { seq: 0 }` + `Remove` of `Project::chat_overflow(CHAT_MAX_MESSAGES)`.
//!   `History` never records chat ops (`ether_model::social::is_untracked`), so the message
//!   is stamped and sent like any edit but is never an undo step.
//! - [`pinned_note_command`]: `Add`/`Edit`/`Delete` as ordinary undoable document edits (the
//!   `MarkerCommand` pattern in `clip_editing`), in a session or not. The author of an `Add`
//!   is the session identity while one is active ([`NoteAuthorScope`]).
//! - [`EtherController::social_presence`]: the controller-owned `PresenceState::transport`
//!   (position, playing, loop region, host-clock `sent_at_ms`), refreshed every
//!   `PEER_TRANSPORT_REFRESH_MS` while playing and at once on play/stop/locate/loop/tempo
//!   changes ([`EtherController::social_transport_due`]); `None` while listening to a host.
//! - `CollabEvent::ChatReceived` for peers' live messages (after the join catch-up, which
//!   ends with our own presence from the relay: [`EtherController::social_own_presence`]).

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use ether_core::protocol::collab::{
    CollabEvent, PEER_TRANSPORT_REFRESH_MS, PeerTransport, Presence, PresenceState,
};
use ether_core::protocol::model::{
    Author, BeatRange, CHAT_MAX_MESSAGES, ChatMessage, ChatMessageId, Color, Entity, EntityKey,
    EntityUpdate, Op, PinnedNote, PinnedNoteChange, PinnedNoteId, Project, SiteId, TempoPoint,
    TempoPointId,
};
use ether_core::protocol::social::{ChatCommand, PinnedNoteCommand};
use ether_core::protocol::{Event, ReplyValue};

use crate::doc::DocCtx;
use crate::handlers::event;
use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, invalid, invalid_state, not_found};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// Social state of a session (reset on `Join`; one field of `CollabState`).
#[derive(Default)]
pub(crate) struct SocialState {
    /// Our relay-assigned colour, learned from our own presence (`Author::color`).
    own_color: Option<Color>,
    /// The relay sent our own presence on this link: the join catch-up is over, peers'
    /// messages from now on are live (`ChatReceived`).
    caught_up: bool,
    /// The transport as last published (see [`TransportKey`]).
    published: Option<TransportKey>,
    /// Host time of the last publication (refresh while playing).
    published_ms: u64,
    /// A transport command ran (locate while playing is not visible in the key).
    changed: bool,
}

/// What makes a new transport sample necessary at once when it changes.
#[derive(Clone, PartialEq)]
struct TransportKey {
    listening: bool,
    playing: bool,
    /// Only while stopped (while playing, receivers extrapolate).
    stopped_at: Option<f64>,
    loop_region: Option<BeatRange>,
    tempo: BTreeMap<TempoPointId, TempoPoint>,
}

thread_local! {
    /// The session identity for notes added by the command being dispatched (see
    /// [`NoteAuthorScope`]).
    static NOTE_AUTHOR: RefCell<Option<Author>> = const { RefCell::new(None) };
}

/// Makes the session identity the author of `PinnedNote::Add`s for the duration of one
/// command's dispatch (document commands only get a [`DocCtx`], which carries no session;
/// a `Batch` may contain notes too). Restores the previous value on drop.
pub(crate) struct NoteAuthorScope(Option<Author>);

impl NoteAuthorScope {
    pub(crate) fn enter(author: Option<Author>) -> Self {
        Self(NOTE_AUTHOR.with(|a| a.replace(author)))
    }
}

impl Drop for NoteAuthorScope {
    fn drop(&mut self) {
        let prev = self.0.take();
        NOTE_AUTHOR.with(|a| *a.borrow_mut() = prev);
    }
}

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

fn note(ctx: &DocCtx, id: PinnedNoteId) -> CmdResult<PinnedNote> {
    ctx.p()
        .pinned_notes
        .get(&id)
        .cloned()
        .ok_or_else(|| not_found(format!("note {id}")))
}

fn set_note(ctx: &mut DocCtx, id: PinnedNoteId, change: PinnedNoteChange) -> CmdResult<()> {
    ctx.tx.update(EntityUpdate::PinnedNote { id, change })
}

/// `PinnedNoteCommand` (a document command, from `doc::apply`). Text, position and count
/// are validated by `Project::apply`.
pub(crate) fn pinned_note_command(ctx: &mut DocCtx, c: &PinnedNoteCommand) -> CmdResult<()> {
    match c {
        PinnedNoteCommand::Add {
            id,
            position,
            text,
            author_name,
        } => {
            if ctx.p().pinned_notes.contains_key(id) {
                return Ok(());
            }
            let author = NOTE_AUTHOR
                .with(|a| a.borrow().clone())
                .unwrap_or_else(|| Author {
                    name: author_name
                        .as_deref()
                        .unwrap_or("")
                        .trim()
                        .chars()
                        .take(ether_core::protocol::model::AUTHOR_NAME_MAX_CHARS)
                        .collect(),
                    site: None,
                    actor: None,
                    color: None,
                });
            ctx.tx.insert(Entity::PinnedNote(PinnedNote {
                id: *id,
                position: position.clone(),
                text: text.clone(),
                author,
                created_at: ctx.now,
                resolved: false,
            }))
        }
        PinnedNoteCommand::Edit {
            id,
            text,
            position,
            resolved,
        } => {
            let n = note(ctx, *id)?;
            if let Some(t) = text
                && *t != n.text
            {
                set_note(ctx, *id, PinnedNoteChange::Text(t.clone()))?;
            }
            if let Some(p) = position
                && *p != n.position
            {
                set_note(ctx, *id, PinnedNoteChange::Position(p.clone()))?;
            }
            if let Some(r) = resolved
                && *r != n.resolved
            {
                set_note(ctx, *id, PinnedNoteChange::Resolved(*r))?;
            }
            Ok(())
        }
        PinnedNoteCommand::Delete { ids } => {
            for id in ids {
                if ctx.p().pinned_notes.contains_key(id) {
                    ctx.tx.remove(EntityKey::PinnedNote(*id))?;
                }
            }
            Ok(())
        }
    }
}

/// Chat inserts among applied ops (a peer's transaction), by id.
pub(crate) fn chat_inserts(ops: &[Op]) -> Vec<ChatMessageId> {
    ops.iter()
        .filter_map(|op| match op {
            Op::Insert {
                entity: Entity::ChatMessage(m),
            } => Some(m.id),
            _ => None,
        })
        .collect()
}

fn check_chat_text(text: &str) -> CmdResult<()> {
    use ether_core::protocol::model::{CHAT_TEXT_MAX_CHARS, TEXT_MAX_BYTES};
    if text.trim().is_empty() {
        return Err(invalid("the message is empty"));
    }
    let n = text.chars().count();
    if n > CHAT_TEXT_MAX_CHARS {
        return Err(invalid(format!(
            "the message is {n} characters (max {CHAT_TEXT_MAX_CHARS})"
        )));
    }
    if text.len() > TEXT_MAX_BYTES {
        return Err(invalid(format!(
            "the message is {} bytes (max {TEXT_MAX_BYTES})",
            text.len()
        )));
    }
    Ok(())
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// This site's identity in the active session, as a snapshot (`None` outside one).
    pub(crate) fn social_author(&mut self) -> Option<Author> {
        if !self.collab_active() {
            return None;
        }
        let name = self.collab_session_name()?;
        Some(Author {
            name,
            site: Some(self.collab_site()),
            actor: None,
            color: self.collab.social.own_color,
        })
    }

    /// The author scope for one dispatched command (see [`NoteAuthorScope`]): only commands
    /// that can add a note pay for the snapshot.
    pub(crate) fn social_note_scope(
        &mut self,
        command: &ether_core::protocol::Command,
    ) -> Option<NoteAuthorScope> {
        use ether_core::protocol::Command;
        matches!(command, Command::PinnedNote(_) | Command::Edit(_))
            .then(|| NoteAuthorScope::enter(self.social_author()))
    }

    /// `Command::Chat` (not a document command).
    pub(crate) fn chat_command(
        &mut self,
        c: &ChatCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        match c {
            ChatCommand::Send { id, text } => {
                let Some(author) = self.social_author() else {
                    return Err(invalid_state("chat needs a collaboration session"));
                };
                check_chat_text(text)?;
                if self
                    .doc
                    .as_ref()
                    .is_some_and(|d| d.project.chat.contains_key(id))
                {
                    return Ok(ReplyValue::Unit);
                }
                let sent_at = self.host.now_ms();
                self.edit_with("Chat", None, now, out, |ctx| {
                    let overflow = ctx.p().chat_overflow(CHAT_MAX_MESSAGES);
                    ctx.tx.insert(Entity::ChatMessage(ChatMessage {
                        id: *id,
                        seq: 0,
                        author,
                        text: text.clone(),
                        sent_at,
                    }))?;
                    for old in overflow {
                        ctx.tx.remove(EntityKey::ChatMessage(old))?;
                    }
                    Ok(())
                })?;
                Ok(ReplyValue::Unit)
            }
        }
    }

    /// The relay sent us our own stamped presence: our colour, and the end of the catch-up
    /// on this link (docs/COLLAB.md §12.1).
    pub(crate) fn social_own_presence(&mut self, presence: &Presence) {
        let s = &mut self.collab.social;
        s.own_color = Some(presence.color);
        s.caught_up = true;
    }

    /// A new link: what the relay sends until our own presence is catch-up.
    pub(crate) fn social_link_opened(&mut self) {
        self.collab.social.caught_up = false;
    }

    /// A session starts: forget the previous one's colour and transport.
    pub(crate) fn social_reset(&mut self) {
        self.collab.social = SocialState::default();
    }

    /// `ChatReceived` for a sequenced transaction of `origin` that inserted `ids` (after
    /// sanitising and resolving), when live: after our catch-up and not our own.
    pub(crate) fn social_chat_received(
        &mut self,
        origin: SiteId,
        ids: Vec<ChatMessageId>,
        out: &mut dyn MessageSink,
    ) {
        if ids.is_empty() || !self.collab.social.caught_up || origin == self.collab_site() {
            return;
        }
        let Some(doc) = self.doc.as_ref() else { return };
        let mut msgs: Vec<&ChatMessage> = ids
            .iter()
            .filter_map(|id| doc.project.chat.get(id))
            .collect();
        msgs.sort_by(|a, b| a.seq.cmp(&b.seq).then(a.id.cmp(&b.id)));
        let ids: Vec<ChatMessageId> = msgs.iter().map(|m| m.id).collect();
        if ids.is_empty() {
            return;
        }
        event(
            out,
            Event::Collab {
                event: CollabEvent::ChatReceived { ids },
            },
        );
    }

    /// A transport command ran: publish the transport at once (handlers hook).
    pub(crate) fn social_transport_changed(&mut self) {
        self.collab.social.changed = true;
    }

    fn social_transport_key(&self, listening: bool) -> TransportKey {
        let t = &self.transport;
        let p = self.doc.as_ref().map(|d| &d.project);
        TransportKey {
            listening,
            playing: t.playing,
            stopped_at: (!t.playing).then_some(t.position.0),
            loop_region: p
                .filter(|p| p.settings.loop_enabled)
                .map(|p| p.settings.loop_region),
            tempo: p.map(|p| p.tempo_points.clone()).unwrap_or_default(),
        }
    }

    /// Whether our presence must be re-sent for the transport: it changed (play, stop,
    /// locate, loop, tempo map, listening) or it is playing and the last sample is
    /// `PEER_TRANSPORT_REFRESH_MS` old. Called by the collab tick before the presence flush.
    pub(crate) fn social_transport_due(&mut self, now: u64) -> bool {
        let mut probe = PresenceState::default();
        self.collab_listen_presence(&mut probe);
        let listening = probe.listening_to.is_some();
        let key = self.social_transport_key(listening);
        let s = &self.collab.social;
        s.changed
            || s.published.as_ref() != Some(&key)
            || (key.playing
                && !listening
                && now.saturating_sub(s.published_ms) >= PEER_TRANSPORT_REFRESH_MS)
    }

    /// Set the controller-owned `transport` of our outgoing presence (after
    /// `listening_to`: `None` while listening to a host, §9).
    pub(crate) fn social_presence(&mut self, state: &mut PresenceState, now: u64) {
        let listening = state.listening_to.is_some();
        let key = self.social_transport_key(listening);
        state.transport = (!listening).then(|| PeerTransport {
            position: self.transport.position,
            playing: key.playing,
            sent_at_ms: self.host.now_ms(),
            loop_region: key.loop_region,
        });
        let s = &mut self.collab.social;
        s.published = Some(key);
        s.published_ms = now;
        s.changed = false;
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
    fn note_author_scope_never_outlives_its_command() {
        let author = Author {
            name: "A".into(),
            site: Some(SiteId(1)),
            actor: None,
            color: None,
        };
        let current = || NOTE_AUTHOR.with(|a| a.borrow().clone());
        {
            let _g = NoteAuthorScope::enter(Some(author.clone()));
            assert_eq!(current(), Some(author.clone()));
            {
                // Nested (a command dispatched while another runs): restored after.
                let _inner = NoteAuthorScope::enter(None);
                assert_eq!(current(), None);
            }
            assert_eq!(current(), Some(author.clone()));
        }
        assert_eq!(current(), None);
        // Cleared on unwinding too.
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _g = NoteAuthorScope::enter(Some(author.clone()));
            panic!("a command panicked");
        }));
        assert!(r.is_err());
        assert_eq!(current(), None);
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

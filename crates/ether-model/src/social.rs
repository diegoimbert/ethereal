//! Social layer of the document (base-62, node `collab-social`; docs/COLLAB.md §12).
//!
//! - [`ChatMessage`]: the session chat, **saved with the project** as a journal. Replicated
//!   like any op but **never undoable**: [`crate::History`] applies chat ops without
//!   recording them ([`is_untracked`]). Ordered by [`ChatMessage::seq`], which
//!   [`crate::Project::apply`] assigns in log order (an `Insert` with `seq: 0` gets the next
//!   one), not by the sender's clock.
//! - [`PinnedNote`]: a short note pinned on the arranger in song coordinates (like presence
//!   pointers), saved in the project like markers. Undoable, replicated, works outside a
//!   session too. Named `PinnedNote` because `Note` is the MIDI note.
//!
//! Both tables are `#[serde(default)]` on `Project`, so `.ether` v3 files without them load
//! unchanged (no version bump).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::collab::{ActorId, SiteId};
use crate::entity::EntityKey;
use crate::ids::{ChatMessageId, PinnedNoteId, TrackId};
use crate::project::Project;
use crate::value::{Beats, Color};

/// Longest chat message, in Unicode scalar values (`char`s).
pub const CHAT_TEXT_MAX_CHARS: usize = 2000;
/// Chat messages kept in the document: sending a message past this prunes the oldest (by
/// `seq`) in the same transaction, so every replica keeps the same ones.
pub const CHAT_MAX_MESSAGES: usize = 2000;
/// Longest pinned note text, in `char`s.
pub const NOTE_TEXT_MAX_CHARS: usize = 2000;
/// Longest author display name, in `char`s (the collab dialog's name field).
pub const AUTHOR_NAME_MAX_CHARS: usize = 64;

/// Who wrote a chat message or a note: a snapshot taken when it was written (names and
/// relay colours change between sessions; the journal keeps what was shown then).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Author {
    /// Display name (≤ [`AUTHOR_NAME_MAX_CHARS`]; may be empty outside a session: the UI
    /// shows "You" / "Someone").
    pub name: String,
    /// The writer's site in the session it was written in (`None` outside a session).
    pub site: Option<SiteId>,
    pub actor: Option<ActorId>,
    /// The writer's relay-assigned presence colour at the time (`None` outside a session:
    /// the theme's neutral colour).
    pub color: Option<Color>,
}

/// One chat message of the project journal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ChatMessage {
    /// Client-chosen ULID (collab-safe: minted by the sender, unique across sites).
    pub id: ChatMessageId,
    /// Position in the chat, in relay log order. **Assigned by `Project::apply`**: an
    /// `Insert` with `seq: 0` gets `1 +` the highest `seq` in the table, so replicas agree
    /// (the confirmed document is the fold of the log). A non-zero `seq` is kept (inverse
    /// ops, snapshots). Senders always send `0`. Ties (only possible with forged values)
    /// sort by id.
    #[ts(type = "number")]
    pub seq: u64,
    pub author: Author,
    /// 1..=[`CHAT_TEXT_MAX_CHARS`] chars, not only whitespace.
    pub text: String,
    /// Unix ms on the sender's clock (display only; clocks are not synchronized).
    #[ts(type = "number")]
    pub sent_at: u64,
}

/// Where a note is pinned: the same song coordinates as `ArrangerPointer`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct NotePosition {
    /// Horizontal position (finite, `>= 0`).
    pub beats: Beats,
    /// The track row it is pinned on; `None` = the ruler / off-track area. A **weak**
    /// reference: not validated and never cascaded. A note whose track no longer exists
    /// shows in the ruler row at `beats` (undoing the track delete puts it back).
    pub track: Option<TrackId>,
    /// Vertical position in that row, 0 (top) ..= 1 (bottom); 0 with `track: None`.
    pub y: f32,
}

/// A note pinned on the arrangement ("Leave a note").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PinnedNote {
    pub id: PinnedNoteId,
    pub position: NotePosition,
    /// 1..=[`NOTE_TEXT_MAX_CHARS`] chars, not only whitespace.
    pub text: String,
    pub author: Author,
    /// Unix ms on the author's clock (display only).
    #[ts(type = "number")]
    pub created_at: u64,
    /// Marked as done (shown dimmed). Older documents load as `false`.
    #[serde(default)]
    pub resolved: bool,
}

/// Ops on these entities are applied but never recorded by [`crate::History`] (chat is a
/// journal, not an edit: undo never removes a sent message, and a message never becomes an
/// undo step).
pub fn is_untracked(key: EntityKey) -> bool {
    matches!(key, EntityKey::ChatMessage(_))
}

impl Project {
    /// Chat messages in chat order (`seq`, ties by id).
    pub fn chat_ordered(&self) -> Vec<&ChatMessage> {
        let mut v: Vec<&ChatMessage> = self.chat.values().collect();
        v.sort_by(|a, b| a.seq.cmp(&b.seq).then(a.id.cmp(&b.id)));
        v
    }

    /// The oldest messages to remove so that one more message keeps the chat at `cap`
    /// (the pruning rule of [`CHAT_MAX_MESSAGES`]; computed on the sender's live document and
    /// sent as `Remove` ops in the message's transaction).
    pub fn chat_overflow(&self, cap: usize) -> Vec<ChatMessageId> {
        let keep = cap.saturating_sub(1);
        let ordered = self.chat_ordered();
        let n = ordered.len().saturating_sub(keep);
        ordered.into_iter().take(n).map(|m| m.id).collect()
    }

    /// Pinned notes sorted by position (beats, ties by id).
    pub fn pinned_notes_sorted(&self) -> Vec<&PinnedNote> {
        let mut v: Vec<&PinnedNote> = self.pinned_notes.values().collect();
        v.sort_by(|a, b| {
            a.position
                .beats
                .0
                .total_cmp(&b.position.beats.0)
                .then(a.id.cmp(&b.id))
        });
        v
    }
}

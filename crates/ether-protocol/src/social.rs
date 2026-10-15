//! Chat and pinned notes (base-62, node `collab-social`; docs/COLLAB.md §12). Both edit the
//! document (`Project::chat`, `Project::pinned_notes`), so they replicate as ordinary
//! transactions and arrive at the UI as `Event::Patch`es.
//!
//! - [`ChatCommand`] is **not** a document command: it never enters the undo history and
//!   is not allowed in `Edit::Batch`. The controller fills the author (this site's session
//!   name, site and relay colour) and `sent_at`, and prunes the oldest messages past
//!   `CHAT_MAX_MESSAGES` in the same transaction.
//! - [`PinnedNoteCommand`] is a document command (undoable, allowed in a `Batch`), in a
//!   session or not.
//!
//! Peers' playheads travel in `PresenceState::transport` (`crate::collab::PeerTransport`).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{ChatMessageId, NotePosition, PinnedNoteId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ChatCommand {
    /// Post a message to the session chat. `id` is client-chosen (a fresh ULID); an
    /// existing id is a no-op (idempotent resend). `text`: 1..=`CHAT_TEXT_MAX_CHARS` chars,
    /// not only whitespace (`InvalidArgument` otherwise). Outside a session:
    /// `InvalidState` (chat is hidden there). Replies `Unit`; the message arrives as a
    /// patch (never an undo step).
    Send { id: ChatMessageId, text: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum PinnedNoteCommand {
    /// "Leave a note" at `position` (song coordinates, like `ArrangerPointer`). `id` is
    /// client-chosen; an existing id is a no-op. The controller fills `author` (in a
    /// session: this site's name, site and colour; outside: `author_name` or "", no site
    /// or colour) and `created_at`.
    Add {
        id: PinnedNoteId,
        position: NotePosition,
        text: String,
        /// The local user's display name outside a session (the collab dialog's name
        /// field). Ignored in a session.
        author_name: Option<String>,
    },
    /// Change a note (any user may; `None` = unchanged). Send with a gesture while
    /// dragging the note.
    Edit {
        id: PinnedNoteId,
        text: Option<String>,
        position: Option<NotePosition>,
        resolved: Option<bool>,
    },
    /// Discard notes for everyone (any user may; undoable). Missing ids are skipped.
    Delete { ids: Vec<PinnedNoteId> },
}

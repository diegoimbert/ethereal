//! Collaboration (RESERVED; roadmap v2, `collab` node refines via BCR).
//!
//! Two layers, both reserved:
//! - **UI ↔ engine** ([`CollabCommand`], [`CollabEvent`]): join/leave a session, publish
//!   this user's presence, receive the other peers' presence. Every host replies
//!   `Unsupported` to `CollabCommand` until the collab node lands.
//! - **engine ↔ engine** ([`CollabMessage`]): what sites exchange (directly or through a
//!   relay): stamped transactions or CRDT update blobs, sync handshakes, presence. Lives
//!   here so it is versioned with the rest of the wire protocol; transported by
//!   `ether-collab`.
//!
//! Document edits still flow as ordinary commands → ops → patches; remote edits arrive at
//! the UI as ordinary `Event::Patch`es (undo stays per-site: undo only reverts own ops).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{
    ActorId, Base64Bytes, Beats, ClipId, Color, DeviceId, NoteId, SiteId, StampedTransaction,
    TrackId,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum CollabCommand {
    /// RESERVED. Join session `session` at `server` (a URL the engine connects to).
    Join {
        server: String,
        session: String,
        token: Option<String>,
        name: String,
    },
    /// RESERVED.
    Leave,
    /// RESERVED. Publish this user's presence (throttled by the host to ~10 Hz).
    SetPresence { presence: PresenceState },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum CollabEvent {
    /// Session state changed (joined, left, reconnecting).
    Session { status: CollabStatus },
    /// Full list of the *other* peers' presence (sent on any change).
    Presence { peers: Vec<Presence> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum CollabStatus {
    Offline,
    Connecting { session: String },
    Online { session: String, site: SiteId },
}

/// What a user shares about where they are and what they have selected.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct PresenceState {
    /// Edit cursor / playhead-independent position on the timeline.
    pub cursor: Option<Beats>,
    pub selected_tracks: Vec<TrackId>,
    pub selected_clips: Vec<ClipId>,
    pub selected_notes: Vec<NoteId>,
    pub selected_devices: Vec<DeviceId>,
    /// Free-form view id (e.g. `"arrangement"`, `"piano-roll"`).
    pub view: Option<String>,
}

/// A peer's presence as shown to the UI.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Presence {
    pub site: SiteId,
    pub actor: Option<ActorId>,
    pub name: String,
    pub color: Color,
    pub state: PresenceState,
}

/// Engine ↔ engine collaboration wire (RESERVED; the collab node may add variants).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum CollabMessage {
    /// First message of a site in a session.
    Hello {
        site: SiteId,
        actor: Option<ActorId>,
        name: String,
        protocol_version: u32,
    },
    /// Op-based sync: one committed transaction.
    Transaction { transaction: StampedTransaction },
    /// CRDT-based sync (Loro/Yrs): an opaque update blob.
    Update { site: SiteId, data: Base64Bytes },
    /// Ask for everything after `version` (opaque; empty = full state).
    SyncRequest { site: SiteId, version: Base64Bytes },
    /// Full state (reply to `SyncRequest` or on join).
    Snapshot { data: Base64Bytes },
    Presence { presence: Presence },
    Leave { site: SiteId },
}

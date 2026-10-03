//! Sharing (base-115; design: docs/SHARING.md): peer-to-peer collaboration where the
//! sharer's app is the hub, invite links carry the capability, and identities are an
//! anonymous display name + colour.
//!
//! Three layers, all append-only:
//! - **UI ↔ engine** ([`ShareCommand`], [`ShareEvent`]): start/stop sharing, links, the
//!   participants, opening an invite, the join confirmation, the session state the top-bar
//!   pill shows. The underlying document sync is unchanged (`CollabMessage`s, docs/COLLAB.md);
//!   `CollabEvent::{Presence, Pointer, ...}` keep flowing as today.
//! - **engine ↔ signaling service** ([`SignalClientMessage`], [`SignalServerMessage`]): JSON
//!   text frames on the room WebSocket of `services/signal` (a Cloudflare Worker). It only
//!   introduces peers and forwards SDP/ICE; it never sees the project or the link secret.
//! - **joiner ↔ host, first frames on the data channel** ([`PeerHandshake`]): mutual
//!   authentication with the link (or member) key, bound to both DTLS fingerprints; then
//!   the data channel carries ordinary `CollabMessage` frames (`ether_collab::wire`).
//!
//! Until their nodes land every [`ShareCommand`] but `Get` replies `Unsupported`
//! (`ether-controller/tests/share_prewire.rs`).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::collab::{IceServer, StreamSignal};
use crate::model::{Color, ProjectId, SiteId};

/// Version of the signaling wire ([`SignalClientMessage`]); the service refuses others.
pub const SIGNAL_PROTOCOL_VERSION: u32 = 1;
/// Version of the data-channel handshake ([`PeerHandshake`]).
pub const SHARE_PROTOCOL_VERSION: u32 = 1;
/// Default signaling service: a Pages Function on the web app's origin that forwards to the
/// `services/signal` Durable Object (docs/SHARING.md §3.1). Overridable in Settings >
/// Advanced, and per link with `?s=<origin>`.
pub const DEFAULT_SIGNAL_URL: &str = "https://etherealws.pages.dev/signal";
/// Default web origin of invite links.
pub const DEFAULT_INVITE_ORIGIN: &str = "https://etherealws.pages.dev";
/// Desktop deep-link scheme (`ethereal://join/<room>#<key>`).
pub const DEEP_LINK_SCHEME: &str = "ethereal";
/// At most this many participants (host included) in a shared project.
pub const MAX_PARTICIPANTS: usize = 16;
/// At most this many members (people who joined once and may come back) per shared project.
pub const MAX_MEMBERS: usize = 64;

/// A random room id: 16 bytes, base64url without padding (22 chars). Stable for a shared
/// project until "Stop sharing".
pub type RoomId = String;
/// A collaborator's stable identity in one shared project (16 random bytes, base64url),
/// issued by the host on first join. The seam for accounts later.
pub type MemberId = String;
/// A pairing on the signaling service (assigned by the service per joiner socket).
pub type PeerId = u32;

/// What an invite link (or a member key) lets its holder do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub enum ShareRole {
    /// Edit the project (a full collaborator).
    Edit,
    /// Hear the host's mix and see the project; no document edits (chat allowed).
    Listen,
}

/// A participant's role in the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub enum ParticipantRole {
    /// The sharer: its app is the hub; its copy is the master.
    Host,
    Edit,
    Listen,
}

// ─── UI ↔ engine ────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ShareCommand {
    /// Re-emit `ShareEvent::State` (UI mount/reload). Replies `Unit`.
    Get,
    /// This user's anonymous identity (sent by the UI at startup and when edited in
    /// Settings). `name`: 1..=64 chars after trimming. `color`: a preference; the host
    /// gives another one when it is taken. Replies `Unit`.
    SetIdentity { name: String, color: Option<Color> },
    /// Settings > Advanced: the signaling service (`None` = [`DEFAULT_SIGNAL_URL`]) and
    /// the web origin invite links use (`None` = [`DEFAULT_INVITE_ORIGIN`]). Replies `Unit`.
    SetServers {
        signal_url: Option<String>,
        invite_origin: Option<String>,
    },
    // ─── host ───
    /// Share the open project: (re)open its room on the signaling service, start the hub,
    /// and join it as the host site. A project shared before resumes with the same links.
    /// Replies `Unit`; `State` becomes `Hosting`.
    Start,
    /// Stop sharing: revoke every link and member key, delete the room, end the session.
    /// Collaborators keep an offline copy. Replies `Unit`.
    Stop,
    /// New secret for the `role` link (the old link stops working; members stay). Replies
    /// `Unit`; `State` carries the new link.
    ResetLink { role: ShareRole },
    /// Disconnect a member and revoke its member key (it can come back only with a current
    /// link). Replies `Unit`.
    RemoveParticipant { member: MemberId },
    /// Change a member's role (applies at once if online). Replies `Unit`.
    SetParticipantRole { member: MemberId, role: ShareRole },
    // ─── joiner ───
    /// Open an invite (`https://<origin>/join/<room>#<key>` or `ethereal://join/...`):
    /// contact the signaling service, connect to the host and authenticate. Replies `Unit`
    /// at once; progress is `State::Joining`, and `Joining { stage: Ready }` carries the
    /// preview for the "<host> invites you to <project>: Join?" screen.
    OpenInvite { link: String },
    /// Confirm the invite shown in `Joining { stage: Ready }`: sync and open the project
    /// (an existing offline copy is reconciled, docs/COLLAB.md §4). Replies `Unit`.
    AcceptInvite,
    /// Joiner: decline a pending invite, or leave the session (the project stays as an
    /// offline copy). Replies `Unit`. `InvalidState` for the host (use `Stop`).
    Leave,
    /// Reconnect an offline copy (Recents, or automatically when it is opened). Replies
    /// `Unit`; `State` goes `Joining` → `Joined`, or `Joined { link: HostOffline }`.
    Reconnect { project: ProjectId },
    /// Turn an offline copy into an ordinary private project (forget its invite and member
    /// key). Replies `Unit`.
    Detach { project: ProjectId },
    // ─── web UI endpoint (the browser's `RTCPeerConnection` lives in the UI thread) ───
    /// A signal from the UI's peer connection for pairing `peer` (forwarded to the
    /// signaling service). Replies `Unit`. Native builds reply `Unsupported`.
    PeerSignal { peer: PeerId, signal: StreamSignal },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ShareEvent {
    /// The sharing state (on every change and on `Get`).
    State { state: ShareState },
    /// Something to toast.
    Notice { notice: ShareNotice },
    /// Web only: open or close a UI peer connection for pairing `peer` (the data channel's
    /// bytes then travel on the collab `MessagePort`, never in this protocol).
    PeerEndpoint {
        peer: PeerId,
        action: PeerEndpointAction,
    },
    /// Web only: a signal for the UI peer connection of pairing `peer`.
    PeerSignal { peer: PeerId, signal: StreamSignal },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum PeerEndpointAction {
    /// Create an `RTCPeerConnection` with `ice_servers`. `offer`: this side creates the
    /// data channel and the offer (the joiner); else it waits for an offer (the host).
    Open {
        offer: bool,
        ice_servers: Vec<IceServer>,
    },
    Close,
}

/// What the session pill, the Share popover and the join screen show.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ShareState {
    /// Not sharing, not joined.
    #[default]
    Off,
    /// This app shares the open project.
    Hosting {
        project: ProjectId,
        /// Full invite URLs (`None` while the room is being (re)opened).
        edit_link: Option<String>,
        listen_link: Option<String>,
        /// Host first, then members (online or not).
        participants: Vec<Participant>,
        signal: SignalStatus,
    },
    /// Opening an invite or reconnecting an offline copy.
    Joining { stage: JoinStage },
    /// In a host's session (or waiting for it to come back).
    Joined {
        project: ProjectId,
        role: ShareRole,
        /// The host first.
        participants: Vec<Participant>,
        link: HostLink,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum SignalStatus {
    Connecting,
    /// Joinable.
    Online,
    /// The signaling service is unreachable: nobody new can join (connected peers stay).
    Offline {
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum HostLink {
    /// (Re)connecting, `attempt` from 1 (exponential backoff).
    Connecting {
        attempt: u32,
    },
    Online,
    /// The host's app is closed: the project is an offline copy until it comes back
    /// (reconnects automatically).
    HostOffline {
        since_ms: f64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum JoinStage {
    /// Asking the signaling service.
    Contacting,
    /// Peer connection + authentication.
    Connecting,
    /// Authenticated: show "<host> invites you to <project>: Join?".
    Ready { invite: InvitePreview },
    /// Accepted: receiving media and the snapshot.
    Syncing {
        received_bytes: f64,
        total_bytes: Option<f64>,
    },
    /// The invite is valid but the host's app is closed (waits; joins when it is back).
    HostOffline { local_copy: Option<ProjectId> },
    /// Invalid, reset or revoked invite, full session, version mismatch, or no route
    /// (symmetric NAT without TURN). Terminal until the next `OpenInvite`.
    Failed {
        reason: JoinFailure,
        message: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum JoinFailure {
    /// Malformed link.
    BadLink,
    /// Unknown room, reset link or revoked member key (indistinguishable on purpose).
    InvalidInvite,
    /// The host refused (session full, removed).
    Refused,
    /// Different app versions (collab or share protocol).
    Version,
    /// No network path to the host (ICE failed; TURN would help).
    Unreachable,
    /// The host failed authentication (possible tampering): never joined.
    HostNotVerified,
    Network,
}

/// "<host> invites you to <project>: Join?"
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct InvitePreview {
    pub host: ParticipantSummary,
    pub project: ProjectId,
    pub project_name: String,
    pub role: ShareRole,
    /// Others in the session now (for avatars).
    pub online: Vec<ParticipantSummary>,
    /// This store already has a copy of the project (it will be reconciled).
    pub local_copy: bool,
}

/// One person in a shared project.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Participant {
    /// `None` for the host.
    pub member: Option<MemberId>,
    /// Their site while online (presence, pointers and listen streams are keyed by site).
    pub site: Option<SiteId>,
    pub name: String,
    pub color: Color,
    pub role: ParticipantRole,
    pub online: bool,
    /// This user.
    pub you: bool,
}

/// Name + colour, for avatars.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ParticipantSummary {
    pub name: String,
    pub color: Color,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ShareNotice {
    ParticipantJoined {
        name: String,
        color: Color,
    },
    ParticipantLeft {
        name: String,
    },
    /// Joiner: the host's app closed; this is now an offline copy.
    HostOffline {
        host_name: String,
    },
    /// Joiner: reconnected after `HostOffline`.
    HostBack {
        host_name: String,
    },
    /// Joiner: the host stopped sharing or removed this member.
    SharingEnded {
        host_name: String,
    },
    /// On rejoin, this copy had offline changes the session did not have: kept as a
    /// separate project (docs/COLLAB.md §4, "(local copy)").
    LocalCopyKept {
        project: ProjectId,
        name: String,
    },
}

/// Recents metadata of a stored shared project (`ProjectSummary::share`). Lives in the
/// project's host-local `share.json`, never in the replicated document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ProjectShareInfo {
    /// `Host`: this store holds the master copy; else an offline copy of someone else's.
    pub role: ParticipantRole,
    pub host_name: String,
    /// Last known participants (host first, at most 8), for avatars.
    pub participants: Vec<ParticipantSummary>,
    /// Host: links are valid (not stopped). Copy: the invite is still stored.
    pub active: bool,
    /// Copy: when it last finished syncing (Unix ms).
    pub last_synced_ms: Option<f64>,
}

// ─── engine ↔ signaling service (services/signal) ─────────────────────────────────────

/// Client → service, JSON text frames on `wss://<signal>/v1/rooms/<room>/{host,join}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum SignalClientMessage {
    /// First frame on `/host`. The first host to connect claims the room (stores
    /// SHA-256 of `host_token`); later connections must present the same token.
    HostHello {
        protocol: u32,
        /// 32 random bytes, base64url; never leaves the host's `share.json` otherwise.
        host_token: String,
        /// SHA-256 (hex) of each valid door (one per link and per member).
        doors: Vec<String>,
        app: String,
    },
    /// Host: replace the doors (link reset, member added or removed).
    SetDoors {
        doors: Vec<String>,
    },
    /// Host: stop sharing: forget the room and close every socket.
    CloseRoom,
    /// First frame on `/join`. `door` = HMAC-SHA256(key, "ethereal/share/v1/door" ‖ room),
    /// first 16 bytes, base64url (docs/SHARING.md §4.2).
    JoinHello {
        protocol: u32,
        door: String,
        app: String,
    },
    /// A signal for the other side of pairing `peer` (the joiner's `peer` is ignored: the
    /// service stamps its own).
    Signal {
        peer: PeerId,
        signal: StreamSignal,
    },
    /// Host: end pairing `peer` (refused, done or failed).
    EndPeer {
        peer: PeerId,
        reason: Option<String>,
    },
    Ping,
}

/// Service → client.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum SignalServerMessage {
    /// `HostHello` accepted.
    HostWelcome {
        ice_servers: Vec<IceServer>,
        /// The room is forgotten this long after the host was last connected.
        room_ttl_s: u64,
    },
    /// Host: a joiner passed a door. The joiner offers; the host answers.
    PeerArrived {
        peer: PeerId,
    },
    /// Host: a joiner socket closed before or after pairing.
    PeerLeft {
        peer: PeerId,
    },
    /// `JoinHello` accepted and the host is online (and was told).
    JoinWelcome {
        peer: PeerId,
        ice_servers: Vec<IceServer>,
    },
    /// The door is valid but the host is not connected. The socket stays open (bounded);
    /// `JoinWelcome` follows when the host connects.
    HostOffline {
        last_seen_ms: Option<f64>,
    },
    Signal {
        peer: PeerId,
        signal: StreamSignal,
    },
    /// Refused; the socket closes after this.
    Refused {
        reason: SignalRefusal,
        message: String,
    },
    Pong,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum SignalRefusal {
    /// Unknown room or wrong door (one answer for both: no room enumeration).
    InvalidInvite,
    /// The room is claimed with another host token.
    NotHost,
    Version,
    RateLimited,
    /// Too many joiners waiting or paired.
    RoomFull,
    Malformed,
}

// ─── joiner ↔ host, first frames on the data channel ──────────────────────────────────

/// What the joiner proves it holds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum Credential {
    /// An invite link key (first join).
    Link,
    /// A member key issued on a previous join (rejoin; survives link resets).
    Member { member: MemberId },
}

/// Text frames before any `CollabMessage` (docs/SHARING.md §4.3). Proofs are
/// HMAC-SHA256 over both DTLS fingerprints, so a tampering signaling service cannot relay
/// them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum PeerHandshake {
    /// Joiner → host, first frame.
    Hello {
        share_protocol: u32,
        collab_protocol: u32,
        credential: Credential,
        /// HMAC(key, "ethereal/share/v1/join" ‖ joiner fp ‖ host fp), base64url.
        proof: String,
        name: String,
        color: Option<Color>,
        app: String,
    },
    /// Host → joiner: authenticated.
    Welcome {
        /// HMAC(key, "ethereal/share/v1/host" ‖ host fp ‖ joiner fp), base64url.
        host_proof: String,
        role: ShareRole,
        member: MemberId,
        /// A fresh member key (base64url) on a link join; `None` on a member rejoin.
        member_key: Option<String>,
        host: ParticipantSummary,
        project: ProjectId,
        project_name: String,
        online: Vec<ParticipantSummary>,
    },
    /// Joiner → host: the user confirmed "Join". `CollabMessage` frames follow both ways.
    Accept,
    /// Host → joiner; the host closes the channel after it.
    Refused {
        reason: JoinFailure,
        message: String,
    },
}

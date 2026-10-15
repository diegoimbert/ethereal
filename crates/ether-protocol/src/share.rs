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
//! [`ShareCommand::PeerSignal`] replies `Unsupported` on hosts without a web UI endpoint
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
pub const DEFAULT_SIGNAL_URL: &str = "https://ethereal.ws/signal";
/// The app's public web origin: THE single place to change the domain. Invite links are
/// built on it; `gen-types` also emits it to `ui/src/generated/constants.ts` (`APP_ORIGIN`).
/// The worker `ALLOWED_ORIGINS` lists (`services/*/wrangler.toml`) are config and are edited
/// separately. Parsers accept any host, so links on older origins (`app.ethereal.ws`,
/// `etherealws.pages.dev`) keep working.
pub const APP_ORIGIN: &str = "https://ethereal.ws";
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

/// "Play with" on the join screen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum JoinEngine {
    /// This computer's engine.
    Mine,
    /// No engine: view (and edit, per role) without compiling the project.
    NoEngine,
    /// Listen on the main engine of the participant named `name` once the project is in
    /// (no local engine; falls back to no engine if they cannot host).
    Participant { name: String },
    /// Run a dedicated engine for this site on the computer of the participant named `name`
    /// (docs/COLLAB.md §16) once the project is in; they are asked first. Falls back to no
    /// engine if they cannot.
    Dedicated { name: String },
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
    /// the web origin invite links use (`None` = [`APP_ORIGIN`]). Replies `Unit`.
    SetServers {
        signal_url: Option<String>,
        invite_origin: Option<String>,
    },
    /// This user's sharing preferences (sent by the UI at startup with `SetIdentity`, and
    /// when changed in Settings). `resume_on_open`: reopening a shared project resumes
    /// sharing (decision 13). `auto_listen`: joining with a listen link starts `Listen` on
    /// the host (§2.3). `relay_only`: hide this device's IP (web: `iceTransportPolicy:
    /// "relay"`; native: host/srflx candidates dropped; needs a TURN server). Replies `Unit`.
    SetPreferences {
        resume_on_open: bool,
        auto_listen: bool,
        relay_only: bool,
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
    /// Listeners get the project without its devices (docs/SHARING.md §14.7): the hub filters
    /// what listen connections receive (editors see everything) and resyncs them. Kept in
    /// `share.json`. Replies `Unit`; `State` carries it.
    SetHideDevices { hide: bool },
    /// #717: the Share popover's "allow editing" switch (off: only the listen link is
    /// offered). Kept in `share.json`, so it survives a reload. Replies `Unit`; `State`
    /// carries it.
    SetAllowEdit { allow: bool },
    // ─── joiner ───
    /// Open an invite (`https://<origin>/join/<room>#<key>` or `ethereal://join/...`):
    /// contact the signaling service, connect to the host and authenticate. Replies `Unit`
    /// at once; progress is `State::Joining`, and `Joining { stage: Ready }` carries the
    /// preview for the "<host> invites you to <project>: Join?" screen.
    OpenInvite { link: String },
    /// Confirm the invite shown in `Joining { stage: Ready }`: sync and open the project
    /// (an existing offline copy is reconciled, docs/COLLAB.md §4). Replies `Unit`.
    ///
    /// `engine`: which engine plays the project (the join screen's "Play with"; `None` = the
    /// user's own). `NoEngine` and `Participant` skip compiling the graph locally, so a
    /// project over this computer's capacity still opens.
    AcceptInvite {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        engine: Option<JoinEngine>,
    },
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
    // ─── diagnostics (docs/SHARING.md §15) ───
    /// Joiner: try the failed join again (the same invite, or the same offline copy).
    /// Replies `Unit`; `InvalidState` when nothing failed.
    Retry,
    /// The join traces (this side's recent joins and host pairings). Replies `Unit` and
    /// emits `ShareEvent::Diagnostics`.
    Diagnostics,
    /// Check this device's connectivity for sharing (docs/SHARING.md §15.5): reach the
    /// signaling service, gather candidates, connect to itself through the TURN relay.
    /// Replies `Unit` at once; `ShareEvent::ConnectionTest` follows (within ~25 s).
    /// `InvalidState` while hosting or connecting to a host.
    TestConnection,
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
    /// The answer to `ShareCommand::Diagnostics`.
    Diagnostics { diagnostics: ShareDiagnostics },
    /// A join (joiner) or a pairing (host) ended: the privacy-safe summary the UI sends as
    /// telemetry when usage statistics are on (docs/PRIVACY.md).
    Outcome { outcome: ShareOutcome },
    /// The answer to `ShareCommand::TestConnection`.
    ConnectionTest { result: ConnectionTestResult },
}

/// What `ShareCommand::TestConnection` found (no addresses).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ConnectionTestResult {
    /// The signaling service answered in this many ms; `None`: it didn't (`service_error`).
    pub service_ms: Option<f64>,
    pub service_error: Option<String>,
    /// The service (or Settings) offered a TURN server with credentials.
    pub turn_offered: bool,
    /// Candidate types this device gathered (`srflx`: STUN works, `relay`: TURN allocations work).
    pub candidates: CandidateKinds,
    /// A data channel to itself through the relay, relay candidates only: `None` when not
    /// tried (no TURN), else whether it connected.
    pub relay_ok: Option<bool>,
    /// Why the relay check failed, or how it connected (`turn over tls`).
    pub relay_detail: String,
    /// The fallback when WebRTC can't connect at all (docs/SHARING.md §3.5): a frame
    /// relayed to itself through the signaling service's WebSocket relay. `None` when not
    /// tried (the service didn't answer), else whether it came back.
    #[serde(default)]
    pub ws_relay_ok: Option<bool>,
    /// The relay's host name (`ethereal.ws`), and why it failed or its round trip.
    #[serde(default)]
    pub ws_relay_via: String,
    #[serde(default)]
    pub ws_relay_detail: String,
    pub duration_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum PeerEndpointAction {
    /// Create an `RTCPeerConnection` with `ice_servers`. `offer`: this side creates the
    /// data channel and the offer (the joiner); else it waits for an offer (the host).
    Open {
        offer: bool,
        ice_servers: Vec<IceServer>,
        /// `ShareCommand::SetPreferences { relay_only }`: `iceTransportPolicy: "relay"`.
        #[serde(default)]
        relay_only: bool,
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
        /// Listeners get the project without its devices (`SetHideDevices`, §14.7).
        #[serde(default)]
        hide_devices: bool,
        /// #717: the edit link is offered (`SetAllowEdit`).
        #[serde(default = "crate::share::yes")]
        allow_edit: bool,
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
    /// WebRTC couldn't connect (or isn't available): the join goes through the signaling
    /// service's WebSocket relay instead (docs/SHARING.md §3.5), `via` its host name
    /// (`ethereal.ws`). Authentication follows, as for `Connecting`.
    Relaying { via: String },
    /// Authenticated: show "<host> invites you to <project>: Join?".
    Ready { invite: InvitePreview },
    /// Accepted: receiving media and the snapshot.
    Syncing {
        received_bytes: f64,
        total_bytes: Option<f64>,
        /// The link goes through the signaling service's WebSocket relay (its host name,
        /// `ethereal.ws`); `None`: a WebRTC data channel.
        #[serde(default)]
        relayed_via: Option<String>,
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
    /// The catch-up broke twice at the same byte (a project the link can't carry).
    TooLarge,
    /// The download stopped making progress, or the link to the host could not be
    /// restored during it.
    Stalled,
    /// The project was downloaded but could not be opened.
    OpenFailed,
}

// ─── diagnostics (docs/SHARING.md §15) ──────────────────────────────────────────────────

/// Which side of a join a trace or an outcome is from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum ShareSide {
    /// The joiner (a first join, a rejoin or a reconnect).
    Guest,
    /// The host: one pairing with a joiner, or its signaling socket.
    Host,
}

/// The phases of a join, in order. A failure names the phase it happened in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
pub enum JoinPhase {
    /// Reaching the signaling service and its answer (`JoinWelcome`, `HostOffline`).
    Service,
    /// The host is offline: waiting for it.
    Waiting,
    /// ICE + DTLS: the data channel to the host (directly, or through TURN).
    Ice,
    /// The handshake on the data channel (key proofs, `Welcome`).
    Handshake,
    /// The join screen: waiting for the user to accept.
    Confirm,
    /// Downloading the project (media, snapshot, log).
    Download,
    /// Downloaded: opening the project.
    Open,
    /// Done.
    Done,
}

/// How the data channel reached the other side.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum ConnectionPath {
    /// Never connected, or not known.
    Unknown,
    /// Peer to peer (host or server-reflexive candidates on both ends).
    Direct,
    /// Through a TURN relay (a relay candidate on either end).
    Turn,
    /// No WebRTC connection at all: through the signaling service's WebSocket relay
    /// (docs/SHARING.md §3.5), end-to-end encrypted.
    Relay,
}

/// Candidate types seen in a pairing (counts; never addresses).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct CandidateKinds {
    pub host: u32,
    pub srflx: u32,
    pub prflx: u32,
    pub relay: u32,
}

/// The privacy-safe summary of one join or pairing (no names, ids, addresses or keys).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ShareOutcome {
    pub side: ShareSide,
    /// `None` when it worked; `cancelled` when the user gave up first.
    pub failure: Option<JoinFailure>,
    pub cancelled: bool,
    /// The phase reached (where it failed or was cancelled; `Done` when it worked).
    pub phase: JoinPhase,
    /// From the start to the end, in ms.
    pub duration_ms: f64,
    pub path: ConnectionPath,
    pub local_candidates: CandidateKinds,
    pub remote_candidates: CandidateKinds,
    /// The ICE servers offered a TURN server with credentials.
    pub turn_offered: bool,
    /// A member coming back (a rejoin or a reconnect), not a first join.
    pub rejoin: bool,
}

/// One line of a trace. `detail` is redacted: no addresses, keys or names.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ShareTraceEntry {
    /// Since the trace started, in ms.
    pub t_ms: f64,
    pub phase: JoinPhase,
    pub event: String,
    pub detail: String,
}

/// One join (guest) or pairing (host), from the start to its outcome.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ShareTrace {
    pub side: ShareSide,
    /// `join`, `rejoin`, `reconnect`, `pairing 7`, `signaling`.
    pub label: String,
    /// When it started (ms since the Unix epoch).
    pub started_ms: f64,
    pub entries: Vec<ShareTraceEntry>,
    /// Entries dropped from the middle of a long trace.
    pub dropped: u32,
    /// `None` while in progress.
    pub outcome: Option<ShareOutcome>,
}

/// `ShareEvent::Diagnostics`: what "copy diagnostics" and issue reports attach.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ShareDiagnostics {
    /// `Ethereal <version>`.
    pub app: String,
    /// The signaling service in use (the default or the link's).
    pub signal_url: String,
    /// `off`, `hosting`, `joining: <stage>`, `joined: <link>`.
    pub state: String,
    pub relay_only: bool,
    /// Settings > Advanced overrides the advertised ICE servers.
    pub ice_override: bool,
    /// Oldest first, the one in progress last.
    pub traces: Vec<ShareTrace>,
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
    /// When an offline member was last connected (ms since the Unix epoch, from
    /// `share.json`); `None` while online or never seen.
    #[serde(default)]
    pub last_seen_ms: Option<f64>,
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
    /// Joiner, paired: WebRTC can't connect (ICE failed, or no `RTCPeerConnection`), relay
    /// this pairing through the service instead (docs/SHARING.md §3.5). Binary frames
    /// follow on both sockets: joiner ↔ service raw, service ↔ host `[peer u32 BE][bytes]`.
    /// The service only forwards them (end-to-end encrypted, [`RELAY_MAX_FRAME_BYTES`]
    /// each) within its budgets; past them it refuses (`RateLimited`) and closes.
    RelayOpen,
}

/// Largest binary relay frame (header and AEAD tag included): one data channel fragment
/// (16 KiB) plus framing, with room to spare.
pub const RELAY_MAX_FRAME_BYTES: usize = 16 * 1024 + 256;

/// Service → client.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum SignalServerMessage {
    /// `HostHello` accepted.
    HostWelcome {
        ice_servers: Vec<IceServer>,
        /// The room is forgotten this long after the host was last connected.
        #[ts(type = "number")]
        room_ttl_s: u64,
    },
    /// Host: a joiner passed a door. The joiner offers; the host answers. `ice_servers`:
    /// fresh ones for this pairing (short-lived TURN credentials; older services: absent,
    /// the host uses `HostWelcome`'s).
    PeerArrived {
        peer: PeerId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        ice_servers: Option<Vec<IceServer>>,
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
    /// Host: the joiner of pairing `peer` sent `RelayOpen`: its binary frames follow, as
    /// `[peer u32 BE][bytes]` (docs/SHARING.md §3.5).
    RelayOpen {
        peer: PeerId,
    },
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
        /// Bytes of a full catch-up (media, snapshot, log) when the host sent this: the
        /// join screen's download total. Absent from older hosts; older joiners ignore it.
        #[serde(default)]
        catch_up_bytes: Option<f64>,
    },
    /// Joiner → host: the user confirmed "Join". `CollabMessage` frames follow both ways.
    /// `name`/`color`: the identity the user chose on the join screen, which replaces the
    /// one in `Hello` (sent before the user typed a name). Absent from older joiners (the
    /// `Hello` identity stands); older hosts ignore them.
    Accept {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        color: Option<Color>,
    },
    /// Host → joiner; the host closes the channel after it.
    Refused {
        reason: JoinFailure,
        message: String,
    },
}

/// Serde default `true` (`ShareState::Hosting::allow_edit`).
pub(crate) fn yes() -> bool {
    true
}

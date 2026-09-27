//! Collaboration (roadmap v2 `collab` node; design: docs/COLLAB.md).
//!
//! Two layers:
//! - **UI ↔ engine** ([`CollabCommand`], [`CollabEvent`]): join/leave a session, publish
//!   this user's presence, receive the other peers' presence.
//! - **engine ↔ engine** ([`CollabMessage`]): what sites exchange (directly or through a
//!   relay): stamped transactions or CRDT update blobs, sync handshakes, presence. Lives
//!   here so it is versioned with the rest of the wire protocol; transported by
//!   `ether-collab`.
//!
//! Presence v2 and "listen on <peer>" streaming (base-53, docs/COLLAB.md §8-§10) are
//! additive: new optional [`PresenceState`] fields, a separate high-rate pointer channel
//! ([`ArrangerPointer`]), and site-to-site messages the relay delivers to one target site
//! (WebRTC signaling, listen requests, forwarded transport requests, the stream clock).
//! Every enum here is append-only.
//!
//! Document edits still flow as ordinary commands → ops → patches; remote edits arrive at
//! the UI as ordinary `Event::Patch`es (undo stays per-site: undo only reverts own ops).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::model::{
    ActorId, AutomationLaneId, Base64Bytes, BeatRange, Beats, ClipId, Color, DeviceId, NoteId,
    ParamId, SiteId, StampedTransaction, TrackId,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum CollabCommand {
    /// Join session `session` at `server` (a URL the engine connects to).
    Join {
        server: String,
        session: String,
        token: Option<String>,
        name: String,
    },
    Leave,
    /// Publish this user's presence (throttled by the host to ~10 Hz).
    SetPresence {
        presence: PresenceState,
    },
    /// Re-emit the current `Session` and `Presence` events (UI mount/reload). Replies `Unit`.
    /// (Once their nodes land, also `ListenStatus` and `IceServers`.)
    Get,
    // ─── base-53 (docs/COLLAB.md §8-§10). Reply `Unsupported` until their node lands. ───
    /// Presence v2 (`presence-v2`): this user's pointer over the arranger (`None` = the
    /// pointer left the arranger). A high-rate channel separate from `SetPresence`: the
    /// controller sends at most [`POINTER_MAX_HZ`] and always sends a clear. Replies `Unit`.
    SetPointer {
        pointer: Option<ArrangerPointer>,
    },
    /// Listen on `host`'s computer (`stream-listen`): the local timeline goes silent (the
    /// local transport is held stopped), the host's output is streamed over WebRTC, and the
    /// transport follows the host. Replies `Unit`; progress comes as
    /// `CollabEvent::ListenStatus`. `Unsupported` where the UI has no WebRTC receiver.
    Listen {
        host: SiteId,
    },
    /// Stop listening (back to the local engine). No-op when not listening. Replies `Unit`.
    StopListening,
    /// A WebRTC signal produced by this site's UI endpoint (the listener's
    /// `RTCPeerConnection`, or the web host's sender) for peer `to`, stream `stream`. The
    /// controller forwards it through the relay. Replies `Unit`.
    SendSignal {
        to: SiteId,
        stream: u32,
        signal: StreamSignal,
    },
    /// Web host only (its sender runs in the UI): a stream clock anchor for listener `to`.
    /// Native hosts produce anchors in the engine bridge instead. Replies `Unit`.
    SendStreamClock {
        to: SiteId,
        stream: u32,
        clock: StreamClock,
    },
    /// Hosting policy of this site (`stream-host`). `allow`: others may listen here (the
    /// default in a session is `true`). `ui_sender`: the UI runs a WebRTC sender (web
    /// build); hosts with a native sender ignore it. `remote_transport`: listeners'
    /// transport requests are applied (default `true`). Replies `Unit`.
    SetHosting {
        allow: bool,
        ui_sender: bool,
        remote_transport: bool,
    },
    /// Override the ICE servers the relay advertises (settings). `None` = use the relay's.
    /// Re-emits `CollabEvent::IceServers`. Replies `Unit`.
    SetIceServers {
        servers: Option<Vec<IceServer>>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum CollabEvent {
    /// Session state changed (joined, left, reconnecting).
    Session { status: CollabStatus },
    /// Full list of the *other* peers' presence (sent on any change).
    Presence { peers: Vec<Presence> },
    // ─── base-53 (docs/COLLAB.md §8-§10) ───
    /// A peer's arranger pointer moved, or left the arranger (`None`). High rate: never
    /// folded into `Presence`. Receivers interpolate between updates. A peer leaving the
    /// session clears its pointer implicitly.
    Pointer {
        site: SiteId,
        pointer: Option<ArrangerPointer>,
    },
    /// A WebRTC signal for this site's UI endpoint (the listener's `RTCPeerConnection`, or
    /// the web host's sender), from peer `from`.
    Signal {
        from: SiteId,
        stream: u32,
        signal: StreamSignal,
    },
    /// Listening/hosting state of this site (sent on any change and on `Get`).
    ListenStatus { status: ListenStatus },
    /// A stream clock anchor from the host this site listens to (docs/COLLAB.md §9.4).
    StreamClock {
        from: SiteId,
        stream: u32,
        clock: StreamClock,
    },
    /// The ICE servers to use (relay-advertised, or the settings override); sent when they
    /// change and on `Get`.
    IceServers {
        servers: Vec<IceServer>,
        source: IceServerSource,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum CollabStatus {
    Offline,
    Connecting { session: String },
    Online { session: String, site: SiteId },
}

/// What a user shares about where they are and what they have selected (≤ 10 Hz; the live
/// pointer has its own channel, [`ArrangerPointer`]).
///
/// The base-53 fields are optional and omitted when unset (older peers and stored presence
/// still parse). `listening_to` and `can_host` are **controller-owned**: the controller
/// overwrites whatever the UI puts there.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct PresenceState {
    /// Edit (insert) cursor on the timeline: where a paste or recording would start. Not the
    /// mouse pointer (that is [`ArrangerPointer`], sent separately).
    pub cursor: Option<Beats>,
    pub selected_tracks: Vec<TrackId>,
    pub selected_clips: Vec<ClipId>,
    pub selected_notes: Vec<NoteId>,
    pub selected_devices: Vec<DeviceId>,
    /// Free-form view id (e.g. `"arrangement"`, `"piano-roll"`).
    pub view: Option<String>,
    /// The visible part of this user's arranger (for follow mode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub viewport: Option<ArrangerViewport>,
    /// What this user is doing right now ("dragging clip X"): set when a gesture starts,
    /// cleared when it ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub activity: Option<Activity>,
    /// The peer whose viewport this user follows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub following: Option<SiteId>,
    /// Controller-owned: the host this site listens to (docs/COLLAB.md §9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub listening_to: Option<SiteId>,
    /// Controller-owned: others can "Listen on" this site (it allows it and has a sender).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub can_host: bool,
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

// ─── Presence v2 (docs/COLLAB.md §8) ─────────────────────────────────────────────────────

/// Highest pointer rate a site sends (`SetPointer` is throttled to it by the controller).
pub const POINTER_MAX_HZ: u32 = 30;

/// A pointer over the arranger, in **song** coordinates (row heights, folds, zoom and
/// scroll are local to each user).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ArrangerPointer {
    /// Horizontal position.
    pub beats: Beats,
    /// The track row under the pointer (including its expanded lanes); `None` = over the
    /// ruler/header area or below the last track.
    pub track: Option<TrackId>,
    /// Vertical position within that row, 0 (top) ..= 1 (bottom). With `track: None`: 0
    /// over the ruler/header area; > 0 below the last track, as the fraction of the free
    /// space there (from the last track down to the bottom of the view), which each user
    /// maps onto their own free space.
    pub y: f32,
}

/// The visible part of a user's arranger.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ArrangerViewport {
    /// Visible beat range (left and right edges).
    pub start: Beats,
    pub end: Beats,
    /// The track row at the top edge (`None`: no tracks) and how much of it is scrolled
    /// past, as a fraction 0..1 of that row's height.
    pub top_track: Option<TrackId>,
    pub top_offset: f32,
}

/// An ephemeral gesture a user is performing, for "Diego is dragging clip X" hints.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct Activity {
    pub kind: ActivityKind,
    pub target: ActivityTarget,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum ActivityKind {
    /// Moving (or copy-dragging) the target.
    Dragging,
    /// Resizing, trimming or fading the target.
    Resizing,
    /// Drawing notes or automation.
    Drawing,
    /// Turning a knob, fader or parameter.
    Adjusting,
    /// Typing a name.
    Renaming,
    /// Recording into the target.
    Recording,
    /// Anything else (shown generically).
    Other,
}

/// What an [`Activity`] acts on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ActivityTarget {
    Clip {
        clip: ClipId,
    },
    Track {
        track: TrackId,
    },
    Device {
        device: DeviceId,
    },
    Param {
        device: DeviceId,
        param: ParamId,
    },
    /// Notes of a MIDI clip (the piano roll).
    Notes {
        clip: ClipId,
    },
    Lane {
        lane: AutomationLaneId,
    },
    /// Several things, or nothing in particular.
    Selection,
}

// ─── Listen on <peer> (docs/COLLAB.md §9) ────────────────────────────────────────────────

/// RTP clock rate of the stream (Opus always uses 48 kHz).
pub const STREAM_RTP_RATE: u32 = 48_000;

/// A host sends each listener an anchor at least this often while streaming (plus one at
/// every discontinuity).
pub const STREAM_CLOCK_INTERVAL_MS: u64 = 100;

/// One WebRTC signaling message (mirrors the browser's `RTCSessionDescriptionInit` /
/// `RTCIceCandidateInit`). The host is always the offerer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum StreamSignal {
    /// Host → listener: SDP offer (one send-only Opus audio track).
    Offer { sdp: String },
    /// Listener → host: SDP answer (receive-only).
    Answer { sdp: String },
    /// Trickle ICE, either way.
    Ice { candidate: IceCandidate },
    /// Either way: this stream is over (refused, stopped, failed). `reason` is shown.
    Bye { reason: Option<String> },
}

/// `RTCIceCandidateInit`. An empty `candidate` = end of candidates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct IceCandidate {
    pub candidate: String,
    pub sdp_mid: Option<String>,
    pub sdp_m_line_index: Option<u16>,
    pub username_fragment: Option<String>,
}

/// `RTCIceServer`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct IceServer {
    /// `stun:host:port`, `turn:host:port?transport=udp`, ...
    pub urls: Vec<String>,
    pub username: Option<String>,
    pub credential: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum IceServerSource {
    /// Advertised by the relay (`CollabMessage::IceServers`).
    Relay,
    /// Set by `CollabCommand::SetIceServers`.
    Settings,
}

/// "The sample with RTP timestamp `rtp` was rendered at song position `position`"
/// (docs/COLLAB.md §9.4), plus the host transport state the listener's UI mirrors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct StreamClock {
    /// RTP timestamp ([`STREAM_RTP_RATE`], wrapping) in the listener's RTP stream.
    pub rtp: u32,
    pub position: Beats,
    pub playing: bool,
    pub recording: bool,
    /// Tempo at `position` (fallback: listeners integrate with the replicated tempo map).
    pub bpm: f64,
    pub loop_enabled: bool,
    pub loop_region: BeatRange,
    pub metronome: bool,
    /// `position` jumped here (play, stop, locate, loop wrap, latency change): never
    /// interpolate across it. `rtp` is when the first post-jump sample is *heard* (the jump
    /// sample plus the graph latency; docs/COLLAB.md §9.4).
    pub discontinuity: bool,
    /// The record start while a recording count-in is armed: the host plays the pre-roll
    /// (`playing` and `recording` true, `position` before this, possibly negative).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub count_in_end: Option<Beats>,
}

/// A transport command a listener forwards to its host (the host's engine decides).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum TransportRequest {
    Play,
    Stop,
    TogglePlay,
    Locate { position: Beats },
    SetLoopEnabled { enabled: bool },
    SetLoopRegion { region: BeatRange },
}

/// This site's listening and hosting state (`CollabEvent::ListenStatus`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct ListenStatus {
    pub listening: ListenState,
    /// Listeners of this site (host side). A UI sender (web host) opens one
    /// `RTCPeerConnection` per `Ui` entry and closes the ones that disappear.
    pub listeners: Vec<ListenerLink>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum ListenState {
    /// Hearing the local engine.
    #[default]
    Off,
    /// Asked `host`; negotiating.
    Connecting { host: SiteId, stream: u32 },
    /// Media flows: the local timeline is silent and the transport follows `host`.
    Listening { host: SiteId, stream: u32 },
    /// The stream ended without a `StopListening` (refused, host left or stopped, network
    /// failure). The site is back on its local engine. Cleared by the next `Listen`.
    Ended { host: SiteId, reason: String },
}

/// One listener of this site.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ListenerLink {
    pub site: SiteId,
    pub stream: u32,
    /// Where this site's sender runs for it.
    pub endpoint: StreamEndpoint,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum StreamEndpoint {
    /// The engine bridge (native sender: desktop, `ether-server`).
    Engine,
    /// The UI (web build: browser WebRTC fed by the worklet's stream output).
    Ui,
}

/// Engine ↔ engine collaboration wire (append-only). Messages travel boxed in `Arc`s on
/// the relay, so variant sizes do not matter.
#[allow(clippy::large_enum_variant)]
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
    Transaction {
        transaction: StampedTransaction,
    },
    /// CRDT-based sync (Loro/Yrs): an opaque update blob.
    Update {
        site: SiteId,
        data: Base64Bytes,
    },
    /// Ask for everything after `version` (opaque; empty = full state).
    SyncRequest {
        site: SiteId,
        version: Base64Bytes,
    },
    /// Full state (reply to `SyncRequest` or on join).
    Snapshot {
        data: Base64Bytes,
    },
    Presence {
        presence: Presence,
    },
    /// One chunk of a media file (sent by the importing site before the transaction that
    /// inserts the media; the relay caches by `hash`). `file` is the project-relative path.
    Media {
        file: String,
        hash: String,
        offset: u64,
        total: u64,
        data: Base64Bytes,
    },
    Leave {
        site: SiteId,
    },
    // ─── base-53 (docs/COLLAB.md §8-§10) ─────────────────────────────────────────────────
    // Site-to-site variants carry `from` and `to` ([`CollabMessage::route`]): the relay
    // checks `from` is the sender's site and delivers the message to `to` only (a ready
    // peer of the same session); they are never logged or cached.
    /// A site's arranger pointer (high rate; relay-stamped like `Presence`, broadcast to the
    /// other ready peers, not cached for late joiners).
    Pointer {
        site: SiteId,
        pointer: Option<ArrangerPointer>,
    },
    /// WebRTC signaling between two sites.
    Signal {
        from: SiteId,
        to: SiteId,
        stream: u32,
        signal: StreamSignal,
    },
    /// Listener → host: "stream to me". `stream` is chosen by the listener (random, new
    /// for every request). The host answers with `Signal::Offer` or `Signal::Bye`.
    Listen {
        from: SiteId,
        to: SiteId,
        stream: u32,
    },
    /// Listener → host: stop streaming `stream`.
    Unlisten {
        from: SiteId,
        to: SiteId,
        stream: u32,
    },
    /// Listener → host: a forwarded transport command.
    TransportRequest {
        from: SiteId,
        to: SiteId,
        stream: u32,
        request: TransportRequest,
    },
    /// Host → listener: a stream clock anchor.
    StreamClock {
        from: SiteId,
        to: SiteId,
        stream: u32,
        clock: StreamClock,
    },
    /// Relay → site (after its sync, and again before its TURN credentials expire): the
    /// ICE servers to use. Dropped when a site sends it.
    IceServers {
        servers: Vec<IceServer>,
    },
}

impl CollabMessage {
    /// `(from, to)` of a site-to-site message (the relay delivers it to `to` only).
    pub fn route(&self) -> Option<(SiteId, SiteId)> {
        match self {
            Self::Signal { from, to, .. }
            | Self::Listen { from, to, .. }
            | Self::Unlisten { from, to, .. }
            | Self::TransportRequest { from, to, .. }
            | Self::StreamClock { from, to, .. } => Some((*from, *to)),
            _ => None,
        }
    }
}

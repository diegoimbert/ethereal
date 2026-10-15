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
    ActorId, AutomationLaneId, Base64Bytes, BeatRange, Beats, ChatMessageId, ClipId, Color,
    DeviceId, DrumPadId, NoteId, ParamId, SiteId, StampedTransaction, TrackId,
};

// `SetPresence` carries a whole `PresenceState` (presence v2 grew it). Commands are rare and
// short-lived, so the variant size gap doesn't matter; boxing would only churn call sites.
#[allow(clippy::large_enum_variant)]
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
    /// Re-emit the current `Session`, `Presence`, `ListenStatus` and `IceServers` events (UI
    /// mount/reload). Replies `Unit`.
    Get,
    // ─── base-53 (docs/COLLAB.md §8-§10) ───
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
    /// Host side: answer a listen request (`ListenStatus.requests`). `allow` also remembers
    /// the consent for `site` for the rest of the session (it can switch listening off and on
    /// without being asked again). Replies `Unit`; a request that is gone is ignored.
    AnswerListen {
        site: SiteId,
        allow: bool,
    },
    /// Host side: end `site`'s listening now and forget its consent (it is asked again next
    /// time). Replies `Unit`.
    RevokeListen {
        site: SiteId,
    },
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
    // ─── engine choice (docs/COLLAB.md §15) ───
    /// Choose which engine plays this site's project: its own, no engine (the document
    /// loads but no graph is compiled and no plugin instantiated: "view only"), or a
    /// participant's (`Participant` = `Listen` on its main engine, with the local engine off).
    /// Switching to `Mine` compiles the graph then; if the engine refuses it (a project over
    /// its capacity) the choice falls back to `None` and `LocalEngineStatus.error` says why.
    /// Replies `Unit`; the result is `CollabEvent::Engine`. `Unsupported` for `Participant`
    /// where the UI has no WebRTC receiver.
    SetEngine {
        engine: EngineChoice,
    },
    // ─── dedicated remote engines (docs/COLLAB.md §16) ───
    /// Host side: answer a request to run a dedicated engine here
    /// (`ListenStatus.engine_requests`). `allow` also remembers the consent for `site` for
    /// the rest of the session. Replies `Unit`; a request that is gone is ignored.
    AnswerEngine {
        site: SiteId,
        allow: bool,
    },
    /// Host side: stop `site`'s dedicated engine now and forget its consent (it is asked
    /// again next time). Replies `Unit`.
    RevokeEngine {
        site: SiteId,
    },
    /// Host side: the policy for dedicated engines of this site: whether others may ask for
    /// one (`allow`) and how many run at once (`max`, 1..=[`MAX_DEDICATED_ENGINES`]).
    /// Lowering `max` stops the newest engines. Replies `Unit`.
    SetEngineHosting {
        allow: bool,
        max: u32,
    },
}

/// Most dedicated engines a site can be asked to run at once (a hard cap; the user's `max`
/// is at most this).
pub const MAX_DEDICATED_ENGINES: u32 = 4;
/// Default of `SetEngineHosting.max`.
pub const DEFAULT_DEDICATED_ENGINES: u32 = 2;

/// What the user picks as "Play with" (join screen and session menu).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type")]
pub enum EngineChoice {
    /// This computer's engine (the default).
    Mine,
    /// No engine: the project is shown and edited, nothing plays.
    None,
    /// The main engine of participant `host` (`Listen`), the local one off.
    Participant { host: SiteId },
    /// A second engine started on `host`'s computer just for this site (its plugins, its CPU,
    /// not heard there), the local one off. `host` is asked first.
    Dedicated { host: SiteId },
}

/// Whether this site's own engine runs the project.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum LocalEngine {
    #[default]
    Mine,
    /// "No engine - view only".
    None,
}

/// The local engine's state (`CollabEvent::Engine`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct LocalEngineStatus {
    pub local: LocalEngine,
    /// Why the last switch to `Mine` failed (the engine fell back to `None`), until the next
    /// switch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
    /// The host whose dedicated engine plays this site's project (starting or running).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub dedicated: Option<SiteId>,
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
    // ─── base-62 (`collab-social`, docs/COLLAB.md §12) ───
    /// Peers' chat messages that just arrived live (sequenced after this site's join
    /// catch-up), in chat order. Never for the join snapshot or catch-up log, a project
    /// load, or this site's own messages. The messages themselves come in the same tick's
    /// `Event::Patch`; the UI toasts these ids while the chat panel is closed.
    ChatReceived { ids: Vec<ChatMessageId> },
    // ─── engine choice (docs/COLLAB.md §15) ───
    /// The local engine was switched on or off (sent on any change and on `Get`).
    Engine { status: LocalEngineStatus },
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
    /// The clip this user has open in the piano roll (others show who edits what).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub editing_clip: Option<ClipId>,
    /// The visible part of this user's piano roll (follow mode there).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub editor_viewport: Option<EditorViewport>,
    /// Controller-owned: the host this site listens to (docs/COLLAB.md §9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub listening_to: Option<SiteId>,
    /// Controller-owned: others can "Listen on" this site (it allows it and has a sender).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub can_host: bool,
    /// Controller-owned (collab protocol 4, docs/COLLAB.md §16): others can ask this site to
    /// run a dedicated engine for them (it allows it, has a native engine host and a free
    /// slot is not required: the limit is checked when asked).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub can_run_engine: bool,
    /// Controller-owned (base-62, `collab-social`): this site's own transport, so peers
    /// draw its playhead. `None` while listening to a host (§9: the host's playhead is the
    /// one heard) and from older peers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub transport: Option<PeerTransport>,
}

// ─── Social (base-62, docs/COLLAB.md §12) ────────────────────────────────────────────────

/// While playing steadily, a site refreshes [`PresenceState::transport`] at least this
/// often (receivers extrapolate in between); play, stop, locate, loop and tempo changes are
/// published at once (subject to the 10 Hz presence throttle).
pub const PEER_TRANSPORT_REFRESH_MS: u64 = 1000;

/// A site's transport as its peers see it (each site has its own transport, §1).
///
/// Receivers extrapolate while `playing`: `position` plus the beats elapsed since the sample
/// arrived, integrated over the **replicated** tempo map from `position`, then wrapped into
/// `loop_region` when set and `position` was inside it. Clocks are not synchronized:
/// extrapolate from the local **arrival** time of the sample (a new `sent_at_ms` = a new
/// sample), never from `sent_at_ms` itself.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PeerTransport {
    /// Timeline position at `sent_at_ms` (beats; negative during a count-in pre-roll).
    pub position: Beats,
    pub playing: bool,
    /// Unix ms on the sender's clock when `position` was read. Identifies the sample.
    #[ts(type = "number")]
    pub sent_at_ms: u64,
    /// The loop region while loop playback is on (`None` = not looping).
    pub loop_region: Option<BeatRange>,
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
    /// Set when the pointer is over a piano roll instead of the arranger (the arranger
    /// fields are then `track: None, y: 0` and ignored).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub editor: Option<EditorPointer>,
}

/// A pointer inside the piano roll of `clip`, in its **content** coordinates (each user's
/// zoom, scroll, key height and folded rows are local).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct EditorPointer {
    pub clip: ClipId,
    /// Position on the clip's content axis (beats from its content start).
    pub beats: Beats,
    /// The key under the pointer plus how far up it is: 60.0 = bottom edge of C3's row,
    /// 60.5 = its middle.
    pub pitch: f32,
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

/// The visible part of a user's piano roll, in the content coordinates of the clip it shows
/// (like [`EditorPointer`]: each user's key height and folded rows are local).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct EditorViewport {
    pub clip: ClipId,
    /// Visible range on the clip's content axis (left and right edges).
    pub start: Beats,
    pub end: Beats,
    /// The key whose row is at the top edge and how much of it is scrolled past, as a
    /// fraction 0..1 of the row's height.
    pub top_key: u8,
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
    /// Host side: sites waiting for this site to allow listening (prompt the user).
    #[serde(default)]
    pub requests: Vec<ListenRequest>,
    /// Host side: sites allowed to listen here for the rest of the session (also the ones
    /// that are not listening right now; `RevokeListen` forgets one).
    #[serde(default)]
    pub allowed: Vec<SiteId>,
    // ─── dedicated engines, host side (docs/COLLAB.md §16) ───
    /// Sites waiting for this site to allow a dedicated engine (prompt the user).
    #[serde(default)]
    pub engine_requests: Vec<ListenRequest>,
    /// The dedicated engines running here (or parked after their site left), with their load.
    #[serde(default)]
    pub engines: Vec<HostedEngine>,
    /// Sites allowed to run a dedicated engine here for the rest of the session.
    #[serde(default)]
    pub engine_allowed: Vec<SiteId>,
    /// This site's policy for dedicated engines (`SetEngineHosting`).
    #[serde(default)]
    pub engine_policy: EnginePolicy,
}

/// A dedicated engine this site runs for another one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct HostedEngine {
    pub site: SiteId,
    pub stream: u32,
    /// DSP load of that engine, percent of one audio thread (0 = idle or unknown).
    pub cpu: u8,
    /// The site left and the engine waits for it for a while before it is stopped.
    #[serde(default)]
    pub parked: bool,
}

/// What this site accepts for dedicated engines.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct EnginePolicy {
    pub allow: bool,
    pub max: u32,
}

impl Default for EnginePolicy {
    fn default() -> Self {
        Self {
            allow: true,
            max: DEFAULT_DEDICATED_ENGINES,
        }
    }
}

/// A site asking to hear this site's engine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct ListenRequest {
    pub site: SiteId,
    pub name: String,
}

/// Mute and solo of a site (docs/COLLAB.md §2.1, §9.8): only what is on, everything else is
/// off. Track `(id, mute, solo)`, drum pad `(id, mute)`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
pub struct MixSeed {
    pub tracks: Vec<(TrackId, bool, bool)>,
    pub pads: Vec<(DrumPadId, bool)>,
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
    /// Participant → host (collab protocol 4, docs/COLLAB.md §16): "run a dedicated engine for
    /// me and stream it to me". Like `Listen` (`stream` is chosen by the sender; the host
    /// answers with `Signal::Offer` or `Signal::Bye`; `Unlisten`, `Signal`, `StreamClock`,
    /// `TransportRequest` and `Monitor` then use the same `stream`), but the stream is a
    /// second engine of the host's, started for the sender: its transport requests and its
    /// `Monitor` mute/solo drive that engine only, and the host's own mix is not sent back.
    /// `mix` is the sender's mute/solo at the time of the request.
    EngineRequest {
        from: SiteId,
        to: SiteId,
        stream: u32,
        mix: MixSeed,
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
    /// Listener ↔ host while a stream runs (collab protocol 3, docs/COLLAB.md §9.8): the
    /// sender's whole mute/solo. The host sends its own when the stream starts and when it
    /// changes; the listener sends its changes (the host applies them unless remote control is
    /// off, and passes them to its other listeners).
    Monitor {
        from: SiteId,
        to: SiteId,
        stream: u32,
        mix: MixSeed,
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
            | Self::EngineRequest { from, to, .. }
            | Self::Unlisten { from, to, .. }
            | Self::TransportRequest { from, to, .. }
            | Self::Monitor { from, to, .. }
            | Self::StreamClock { from, to, .. } => Some((*from, *to)),
            _ => None,
        }
    }
}

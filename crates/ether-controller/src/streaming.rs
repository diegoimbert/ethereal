//! Engine-bridge types for "listen on <peer>" (base-53; docs/COLLAB.md §9) and plugin GUI
//! mirrors. The [`crate::EngineBridge`] methods using them are defaulted to
//! `Unsupported`/no-op; `stream-host` implements them natively (`ether-native`: the engine's
//! stream tap → Opus → str0m, one peer connection per listener), `plugin-mirror` the
//! mirrors.

use ether_core::protocol::collab::{StreamClock, StreamSignal};
use ether_core::protocol::model::SiteId;

/// What the bridge can do for streaming (queried when hosting starts and when the hosting
/// policy changes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamCapabilities {
    /// A native WebRTC sender streams the engine's stream tap (the host's
    /// `ListenerLink::endpoint` is then `Engine`).
    pub native_sender: bool,
}

/// Something the native sender produced for the controller to forward (drained by
/// `EngineBridge::poll_stream` from the controller tick).
#[derive(Clone, Debug, PartialEq)]
pub enum StreamOutput {
    /// A signal for `listener`'s UI (the offer, trickle ICE, a `Bye`).
    Signal {
        listener: SiteId,
        stream: u32,
        signal: StreamSignal,
    },
    /// A stream clock anchor for `listener` (in that listener's RTP timeline).
    Clock {
        listener: SiteId,
        stream: u32,
        clock: StreamClock,
    },
    /// The peer connection to `listener` changed state.
    State {
        listener: SiteId,
        stream: u32,
        state: StreamLinkState,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamLinkState {
    /// ICE + DTLS done: media flows.
    Connected,
    /// ICE failed, DTLS failed, or the listener stopped answering (consent freshness).
    Failed { reason: String },
    /// Closed (by `stream_close` or a `Bye`).
    Closed,
}

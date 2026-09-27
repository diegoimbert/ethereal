//! "Listen on <peer>", host side (`stream-host` node; docs/COLLAB.md §9).
//!
//! Pre-wired by base-53 (dispatch in `collab/mod.rs`); until the node lands `SetHosting`
//! and `SendStreamClock` reply `Unsupported`, `Listen` requests are refused with a `Bye`
//! ("this peer cannot host"), and `can_host` stays false.
//!
//! To do (§9.2, §9.3, §9.5): hosting policy (`SetHosting`; default allow + remote
//! transport); on `Listen { from, stream }` accept (≤ `MAX_LISTENERS`) or `Bye`; endpoint =
//! `Engine` when `EngineBridge::stream_capabilities().native_sender`, else `Ui` when the UI
//! declared `ui_sender`, else refuse. Engine endpoint: `start_stream_capture` (first
//! listener), `stream_open` with `collab_ice_servers()`, route the listener's signals to
//! `stream_signal`, drain `poll_stream` every tick (signals/clocks → relay, states →
//! listener set), `stream_close` + `stop_stream_capture` (last). Ui endpoint: list the
//! listener in `ListenStatus.listeners` (the web sender opens a peer connection per
//! entry), route its signals to the UI (`CollabEvent::Signal`) and forward the UI's
//! `SendStreamClock`s. `TransportRequest`s from current listeners: apply in arrival order
//! as local transport commands unless remote transport is off or we are recording/counting
//! in. End streams on `Unlisten`, a listener `Bye`, its `Leave`, our `Leave`, or when
//! hosting is disallowed (`Bye` to each). `can_host` in our presence.

use ether_core::protocol::ReplyValue;
use ether_core::protocol::collab::{
    CollabCommand, CollabMessage, ListenerLink, PresenceState, StreamSignal,
};
use ether_core::protocol::model::SiteId;

use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, unsupported};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// Host state of a session (one field of `collab::Session`).
#[derive(Default)]
pub(crate) struct HostState {}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// `CollabCommand::{SetHosting, SendStreamClock}`.
    pub(crate) fn collab_host_command(
        &mut self,
        c: &CollabCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let _ = (c, now, out);
        Err(unsupported("hosting a stream is not implemented yet"))
    }

    /// `CollabMessage::{Listen, Unlisten, TransportRequest}` addressed to this site.
    /// Default: refuse every `Listen` with a `Bye`.
    pub(crate) fn collab_host_message(&mut self, m: CollabMessage, out: &mut dyn MessageSink) {
        let _ = out;
        if let CollabMessage::Listen { from, stream, .. } = m {
            self.collab_send_routed_signal(
                from,
                stream,
                StreamSignal::Bye {
                    reason: Some("this peer cannot host a stream".into()),
                },
            );
        }
    }

    /// Whether `stream` from `peer` is one this site hosts (its signals go to
    /// [`Self::collab_host_signal`], the others to the listener side).
    pub(crate) fn collab_hosts_stream(&self, peer: SiteId, stream: u32) -> bool {
        let _ = (peer, stream);
        false
    }

    /// A `Signal` from listener `from` for a hosted stream (answer, ICE, bye).
    pub(crate) fn collab_host_signal(
        &mut self,
        from: SiteId,
        stream: u32,
        signal: StreamSignal,
        out: &mut dyn MessageSink,
    ) {
        let _ = (from, stream, signal, out);
    }

    /// Our UI sent `signal` to `to` (`SendSignal`, already forwarded): the web sender's
    /// offer/ICE, or its `Bye` when its peer connection failed.
    pub(crate) fn collab_host_outgoing_signal(
        &mut self,
        to: SiteId,
        stream: u32,
        signal: &StreamSignal,
        out: &mut dyn MessageSink,
    ) {
        let _ = (to, stream, signal, out);
    }

    /// Every collab tick (drain `EngineBridge::poll_stream`).
    pub(crate) fn collab_host_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        let _ = (now, out);
    }

    /// Peer `site` left the session (it may be a listener).
    pub(crate) fn collab_host_peer_left(&mut self, site: SiteId, out: &mut dyn MessageSink) {
        let _ = (site, out);
    }

    /// We leave the session: close every stream.
    pub(crate) fn collab_host_session_end(&mut self, out: &mut dyn MessageSink) {
        let _ = out;
    }

    /// Current listeners (the host part of `CollabEvent::ListenStatus`, read by
    /// `collab_listen_emit_status`).
    #[allow(dead_code)]
    pub(crate) fn collab_host_listeners(&self) -> Vec<ListenerLink> {
        Vec::new()
    }

    /// Set the controller-owned `can_host` of our outgoing presence.
    pub(crate) fn collab_host_presence(&self, state: &mut PresenceState) {
        state.can_host = false;
    }
}

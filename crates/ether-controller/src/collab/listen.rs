//! "Listen on <peer>", listener side (`stream-listen` node; docs/COLLAB.md §9).
//!
//! Pre-wired by base-53 (dispatch in `collab/mod.rs`, the transport intercept in
//! `handlers.rs::transport_command`); until the node lands `Listen`/`StopListening` reply
//! `Unsupported` and nothing is intercepted.
//!
//! To do (§9.2, §9.5): `Listen { host }` → pick a random `stream`, send
//! `CollabMessage::Listen`, hold the local transport stopped (remember whether it was
//! playing), status `Connecting`; relay the host's `Signal`s to the UI
//! (`CollabEvent::Signal`) and the UI's back (`SendSignal`, forwarded by `mod.rs`, which
//! shows them to [`collab_listen_outgoing_signal`]: a UI `Bye` = the receiver failed or
//! closed → `Ended`); the first `StreamClock` (hosts send clocks only once media flows) →
//! `Listening`; forward `StreamClock`s (`CollabEvent::StreamClock`) and mirror the host
//! transport state into `Event::Transport`;
//! `transport_intercept`: forward Play/Stop/TogglePlay/Locate/loop as
//! `CollabMessage::TransportRequest` (reply `Unit`), refuse recording, keep metronome,
//! tempo and signature local/document as usual; end on `Bye`, the host's `Leave`, our
//! `Leave`, `StopListening` (send `Unlisten`), restoring the local transport at the last
//! heard position; `listening_to` in our presence.
//!
//! [`collab_listen_outgoing_signal`]: EtherController::collab_listen_outgoing_signal

use ether_core::protocol::ReplyValue;
use ether_core::protocol::collab::{CollabCommand, PresenceState, StreamClock, StreamSignal};
use ether_core::protocol::model::SiteId;
use ether_core::protocol::transport::TransportCommand;

use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, unsupported};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// Listener state of a session (one field of `collab::Session`).
#[derive(Default)]
pub(crate) struct ListenerState {}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// `CollabCommand::{Listen, StopListening}`.
    pub(crate) fn collab_listen_command(
        &mut self,
        c: &CollabCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let _ = (c, now, out);
        Err(unsupported("listening on a peer is not implemented yet"))
    }

    /// `Some(reply)`: `c` was handled for the host we listen to (forwarded or refused);
    /// `None`: not listening, the local transport handles it.
    pub(crate) fn collab_transport_intercept(
        &mut self,
        c: &TransportCommand,
        out: &mut dyn MessageSink,
    ) -> Option<CmdResult<ReplyValue>> {
        let _ = (c, out);
        None
    }

    /// A `Signal` from `from` for a stream this site listens to (offer, ICE, bye).
    pub(crate) fn collab_listen_signal(
        &mut self,
        from: SiteId,
        stream: u32,
        signal: StreamSignal,
        out: &mut dyn MessageSink,
    ) {
        let _ = (from, stream, signal, out);
    }

    /// Our UI sent `signal` to `to` for `stream` (`SendSignal`, already forwarded).
    pub(crate) fn collab_listen_outgoing_signal(
        &mut self,
        to: SiteId,
        stream: u32,
        signal: &StreamSignal,
        out: &mut dyn MessageSink,
    ) {
        let _ = (to, stream, signal, out);
    }

    /// A `StreamClock` anchor from `from`.
    pub(crate) fn collab_listen_clock(
        &mut self,
        from: SiteId,
        stream: u32,
        clock: StreamClock,
        out: &mut dyn MessageSink,
    ) {
        let _ = (from, stream, clock, out);
    }

    /// Every collab tick.
    pub(crate) fn collab_listen_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        let _ = (now, out);
    }

    /// Peer `site` left the session (it may be our host).
    pub(crate) fn collab_listen_peer_left(&mut self, site: SiteId, out: &mut dyn MessageSink) {
        let _ = (site, out);
    }

    /// We leave the session (the session state is still there): stop listening.
    pub(crate) fn collab_listen_session_end(&mut self, out: &mut dyn MessageSink) {
        let _ = out;
    }

    /// Emit `CollabEvent::ListenStatus` (on `Get`). Listener and host parts together: the
    /// host part comes from [`Self::collab_host_listeners`].
    pub(crate) fn collab_listen_emit_status(&mut self, out: &mut dyn MessageSink) {
        let _ = out;
    }

    /// Set the controller-owned `listening_to` of our outgoing presence.
    pub(crate) fn collab_listen_presence(&self, state: &mut PresenceState) {
        state.listening_to = None;
    }
}

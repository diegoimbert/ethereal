//! "Listen on <peer>", listener side (`stream-listen` node; docs/COLLAB.md §9).
//!
//! - `Listen { host }` picks a random `stream` id, sends `CollabMessage::Listen`, holds the
//!   local transport stopped and goes `Connecting`. The host's `Signal`s for that stream
//!   (offer, ICE) go to the UI receiver (`CollabEvent::Signal`); the UI's answers and ICE
//!   are forwarded by `mod.rs` (`SendSignal`). The first `StreamClock` (hosts send clocks
//!   only once media flows) → `Listening` and `listening_to` in our presence.
//! - Clock anchors are relayed to the UI (`CollabEvent::StreamClock`, mapped to the playhead
//!   there, §9.4) and the latest one drives `Event::Transport` ([`collab_listen_transport_state`]).
//! - The transport intercept (§9.5): Play/Stop/TogglePlay/Locate and loop changes become
//!   `TransportRequest`s to the host (reply `Unit` at once); recording is refused; tempo,
//!   signature, tap tempo and metronome stay local/document commands.
//! - The stream ends on `StopListening` (`Unlisten` → `Off`), the host's `Bye`, a UI `Bye`
//!   (receiver failed), the host's `Leave`, our relay link dropping, or our own `Leave`
//!   (→ `Ended { reason }`, or `Off` for a leave). The local transport then stays stopped,
//!   located at the last heard position (never auto-plays).
//!
//! [`collab_listen_transport_state`]: EtherController::collab_listen_transport_state

use ether_core::TransportControl;
use ether_core::protocol::collab::{
    CollabCommand, CollabEvent, CollabMessage, ListenState, ListenStatus, PresenceState,
    STREAM_CLOCK_INTERVAL_MS, StreamClock, StreamSignal, TransportRequest,
};
use ether_core::protocol::model::{Beats, SiteId};
use ether_core::protocol::recording::RecordingCommand;
use ether_core::protocol::transport::{TransportCommand, TransportState};
use ether_core::protocol::{Command, Event, ReplyValue};

use crate::engine::bridge_err;
use crate::handlers::event;
use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, invalid, invalid_state, not_found};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// When the stream ends, the last heard position is the latest anchor extrapolated by the
/// time since it arrived, at most this long (anchors come every 100 ms while playing).
const MAX_EXTRAPOLATION_MS: u64 = 2 * STREAM_CLOCK_INTERVAL_MS;

/// The stream this site asked for.
struct Link {
    host: SiteId,
    /// The host's display name (kept for "Diego left": its `Leave` removes it).
    name: String,
    stream: u32,
    /// Latest clock anchor and when it arrived (ms). `Some` = `Listening`.
    anchor: Option<(StreamClock, u64)>,
}

/// Listener state of a session (one field of `collab::Session`).
#[derive(Default)]
pub(crate) struct ListenerState {
    link: Option<Link>,
    /// Set when a stream ended without `StopListening` (cleared by the next `Listen`).
    ended: Option<(SiteId, String)>,
    /// Last emitted `ListenStatus` (to emit only changes).
    emitted: Option<ListenStatus>,
}

impl ListenerState {
    fn state(&self) -> ListenState {
        match (&self.link, &self.ended) {
            (Some(l), _) if l.anchor.is_some() => ListenState::Listening {
                host: l.host,
                stream: l.stream,
            },
            (Some(l), _) => ListenState::Connecting {
                host: l.host,
                stream: l.stream,
            },
            (None, Some((host, reason))) => ListenState::Ended {
                host: *host,
                reason: reason.clone(),
            },
            (None, None) => ListenState::Off,
        }
    }
}

/// Why a stream ends.
enum End {
    /// `StopListening` or our own `Leave`: status `Off`.
    Stopped,
    /// Status `Ended { reason }`.
    Ended(String),
}

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
        match c {
            CollabCommand::Listen { host } => self.collab_listen_start(*host, now, out)?,
            // No-op when not listening.
            CollabCommand::StopListening => self.collab_listen_end(End::Stopped, true, now, out),
            _ => {}
        }
        Ok(ReplyValue::Unit)
    }

    fn collab_listen_start(
        &mut self,
        host: SiteId,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<()> {
        let me = self.collab_site();
        if host == me {
            return Err(invalid("cannot listen to this site"));
        }
        let Some(s) = self.collab.session.as_ref() else {
            return Err(invalid_state("not in a collaboration session"));
        };
        if !s.joined {
            return Err(invalid_state("not in a collaboration session"));
        }
        // A peer is known from its `Hello` (names) or its presence.
        let name = match (s.peers.get(&host), s.names.get(&host)) {
            (Some(p), _) if !p.name.is_empty() => p.name.clone(),
            (_, Some(n)) => n.clone(),
            (Some(_), None) => String::new(),
            (None, None) => return Err(not_found(format!("peer {}", host.0))),
        };
        if self.transport.recording {
            return Err(invalid_state("stop recording before listening to a peer"));
        }
        if let Some(l) = self.collab_listen_link() {
            if l.host == host {
                return Ok(());
            }
            // Listening elsewhere: that stream ends first (one host at a time).
            self.collab_listen_end(End::Stopped, true, now, out);
        }
        // Hold the local timeline stopped (previews and live input still sound).
        if self.transport.playing {
            self.bridge
                .transport(TransportControl::Stop)
                .map_err(bridge_err)?;
            self.transport.playing = false;
        }
        let stream = loop {
            let r = self.host.random_seed() as u32;
            if r != 0 {
                break r;
            }
        };
        let s = self.collab.session.as_mut().expect("checked above");
        s.listener.ended = None;
        s.listener.link = Some(Link {
            host,
            name,
            stream,
            anchor: None,
        });
        s.send(&CollabMessage::Listen {
            from: me,
            to: host,
            stream,
        });
        self.collab_listen_emit_if_changed(out);
        Ok(())
    }

    /// Transport commands while listening (called first in `dispatch`). `Some(reply)`: `c`
    /// was forwarded to the host or refused; `None`: the local controller handles it.
    pub(crate) fn collab_transport_intercept(
        &mut self,
        c: &Command,
        out: &mut dyn MessageSink,
    ) -> Option<CmdResult<ReplyValue>> {
        let _ = out;
        let (host, stream) = {
            let l = self.collab_listen_link()?;
            (l.host, l.stream)
        };
        let request = match c {
            Command::Transport(t) => match t {
                TransportCommand::Play => TransportRequest::Play,
                TransportCommand::Stop => TransportRequest::Stop,
                TransportCommand::TogglePlay => TransportRequest::TogglePlay,
                TransportCommand::Locate { position } => {
                    if !(position.0.is_finite() && position.0 >= 0.0) {
                        return Some(Err(invalid("position must be >= 0")));
                    }
                    TransportRequest::Locate {
                        position: *position,
                    }
                }
                TransportCommand::SetLoopEnabled { enabled } => {
                    TransportRequest::SetLoopEnabled { enabled: *enabled }
                }
                TransportCommand::SetLoopRegion { region } => {
                    if !(region.start.0.is_finite()
                        && region.end.0.is_finite()
                        && region.start.0 >= 0.0
                        && region.end.0 > region.start.0)
                    {
                        return Some(Err(invalid("the loop region must be non-empty")));
                    }
                    TransportRequest::SetLoopRegion { region: *region }
                }
                // Tempo/signature/tap are document edits, the metronome a local setting.
                TransportCommand::SetTempo { .. }
                | TransportCommand::SetTimeSignature { .. }
                | TransportCommand::SetMetronome { .. }
                | TransportCommand::TapTempo => return None,
            },
            Command::Recording(RecordingCommand::SetRecording { enabled: true }) => {
                return Some(Err(invalid_state("stop listening to record")));
            }
            _ => return None,
        };
        let from = self.collab_site();
        if let Some(s) = self.collab.session.as_mut() {
            s.send(&CollabMessage::TransportRequest {
                from,
                to: host,
                stream,
                request,
            });
        }
        Some(Ok(ReplyValue::Unit))
    }

    /// While listening (an anchor arrived): the transport state follows the host's latest
    /// anchor (`Event::Transport`, §9.4); `None` otherwise.
    pub(crate) fn collab_listen_transport_state(&self) -> Option<TransportState> {
        let (clock, _) = self.collab_listen_link()?.anchor.as_ref()?;
        let doc = self.doc.as_ref()?;
        let map = doc.project.tempo_map();
        Some(TransportState {
            playing: clock.playing,
            recording: clock.recording,
            loop_enabled: clock.loop_enabled,
            loop_region: clock.loop_region,
            bpm: clock.bpm,
            time_signature: map.signature_at(Beats(clock.position.0.max(0.0))),
            metronome: clock.metronome,
            start_position: self.transport.start_position,
        })
    }

    /// The host this site listens to (connecting or listening), for the host side: a site
    /// that listens elsewhere cannot host (§9.3).
    #[allow(dead_code)]
    pub(crate) fn collab_listening_host(&self) -> Option<SiteId> {
        self.collab_listen_link().map(|l| l.host)
    }

    /// A `Signal` from `from` for a stream this site listens to (offer, ICE, bye).
    pub(crate) fn collab_listen_signal(
        &mut self,
        from: SiteId,
        stream: u32,
        signal: StreamSignal,
        out: &mut dyn MessageSink,
    ) {
        if !self.collab_listen_is_ours(from, stream) {
            return; // stale or unsolicited (§9.2, §9.7)
        }
        match signal {
            StreamSignal::Offer { .. } | StreamSignal::Ice { .. } => event(
                out,
                Event::Collab {
                    event: CollabEvent::Signal {
                        from,
                        stream,
                        signal,
                    },
                },
            ),
            StreamSignal::Bye { reason } => {
                // The UI closes its receiver on the status change.
                let now = self.host.now_ms();
                let reason = reason.unwrap_or_else(|| "the host ended the stream".into());
                self.collab_listen_end(End::Ended(reason), false, now, out);
            }
            // The host is always the offerer.
            StreamSignal::Answer { .. } => {}
        }
    }

    /// Our UI sent `signal` to `to` for `stream` (`SendSignal`, already forwarded): a `Bye`
    /// means the receiver failed or closed.
    pub(crate) fn collab_listen_outgoing_signal(
        &mut self,
        to: SiteId,
        stream: u32,
        signal: &StreamSignal,
        out: &mut dyn MessageSink,
    ) {
        if let StreamSignal::Bye { reason } = signal
            && self.collab_listen_is_ours(to, stream)
        {
            let now = self.host.now_ms();
            let reason = reason
                .clone()
                .unwrap_or_else(|| "the connection failed".into());
            self.collab_listen_end(End::Ended(reason), false, now, out);
        }
    }

    /// A `StreamClock` anchor from `from`.
    pub(crate) fn collab_listen_clock(
        &mut self,
        from: SiteId,
        stream: u32,
        clock: StreamClock,
        out: &mut dyn MessageSink,
    ) {
        if !self.collab_listen_is_ours(from, stream) {
            return;
        }
        let now = self.host.now_ms();
        let s = self.collab.session.as_mut().expect("in session");
        let link = s.listener.link.as_mut().expect("checked");
        let first = link.anchor.is_none();
        link.anchor = Some((clock.clone(), now));
        if first {
            s.presence_dirty = true;
        }
        event(
            out,
            Event::Collab {
                event: CollabEvent::StreamClock {
                    from,
                    stream,
                    clock,
                },
            },
        );
        if first {
            self.collab_listen_emit_if_changed(out);
        }
        self.emit_transport_if_changed(out);
    }

    /// Every collab tick.
    pub(crate) fn collab_listen_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        let Some(s) = self.collab.session.as_ref() else {
            return;
        };
        if s.listener.link.is_none() {
            return;
        }
        // Our relay link dropped: the host saw us leave, the stream is gone (§9.2).
        if !s.open() {
            self.collab_listen_end(
                End::Ended("the connection to the relay was lost".into()),
                false,
                now,
                out,
            );
            return;
        }
        // The engine's transport may have been started from elsewhere (its play flag is
        // adopted by the controller): keep the local timeline stopped.
        if self.transport.playing && self.bridge.transport(TransportControl::Stop).is_ok() {
            self.transport.playing = false;
        }
    }

    /// Peer `site` left the session (it may be our host).
    pub(crate) fn collab_listen_peer_left(&mut self, site: SiteId, out: &mut dyn MessageSink) {
        let Some(l) = self.collab_listen_link() else {
            return;
        };
        if l.host != site {
            return;
        }
        let who = if l.name.is_empty() {
            "the host".to_string()
        } else {
            l.name.clone()
        };
        let now = self.host.now_ms();
        self.collab_listen_end(End::Ended(format!("{who} left")), false, now, out);
    }

    /// We leave the session (the session state is still there): stop listening.
    pub(crate) fn collab_listen_session_end(&mut self, out: &mut dyn MessageSink) {
        if self.collab_listen_link().is_some() {
            let now = self.host.now_ms();
            self.collab_listen_end(End::Stopped, true, now, out);
        }
    }

    /// Emit `CollabEvent::ListenStatus` (on `Get`). Listener and host parts together: the
    /// host part comes from [`Self::collab_host_listeners`].
    pub(crate) fn collab_listen_emit_status(&mut self, out: &mut dyn MessageSink) {
        let status = self.collab_listen_status();
        if let Some(s) = self.collab.session.as_mut() {
            s.listener.emitted = Some(status.clone());
        }
        event(
            out,
            Event::Collab {
                event: CollabEvent::ListenStatus { status },
            },
        );
    }

    /// Set the controller-owned `listening_to` of our outgoing presence.
    pub(crate) fn collab_listen_presence(&self, state: &mut PresenceState) {
        state.listening_to = self
            .collab_listen_link()
            .filter(|l| l.anchor.is_some())
            .map(|l| l.host);
    }

    // ─── Internals ──────────────────────────────────────────────────────────────────────

    fn collab_listen_link(&self) -> Option<&Link> {
        self.collab.session.as_ref()?.listener.link.as_ref()
    }

    fn collab_listen_is_ours(&self, host: SiteId, stream: u32) -> bool {
        self.collab_listen_link()
            .is_some_and(|l| l.host == host && l.stream == stream)
    }

    fn collab_listen_status(&self) -> ListenStatus {
        ListenStatus {
            listening: self
                .collab
                .session
                .as_ref()
                .map(|s| s.listener.state())
                .unwrap_or_default(),
            listeners: self.collab_host_listeners(),
        }
    }

    fn collab_listen_emit_if_changed(&mut self, out: &mut dyn MessageSink) {
        let status = self.collab_listen_status();
        let changed = self
            .collab
            .session
            .as_ref()
            .is_none_or(|s| s.listener.emitted.as_ref() != Some(&status));
        if changed {
            self.collab_listen_emit_status(out);
        }
    }

    /// End the stream: `Unlisten` to the host when `unlisten`, status `Off`/`Ended`, the
    /// local transport stopped at the last heard position, `listening_to` cleared.
    fn collab_listen_end(&mut self, why: End, unlisten: bool, now: u64, out: &mut dyn MessageSink) {
        let me = self.collab_site();
        let Some(s) = self.collab.session.as_mut() else {
            return;
        };
        let Some(link) = s.listener.link.take() else {
            return;
        };
        if unlisten {
            s.send(&CollabMessage::Unlisten {
                from: me,
                to: link.host,
                stream: link.stream,
            });
        }
        s.listener.ended = match why {
            End::Stopped => None,
            End::Ended(reason) => Some((link.host, reason)),
        };
        s.presence_dirty = true;
        if let Some((clock, at)) = &link.anchor {
            let position = last_heard(clock, now.saturating_sub(*at));
            // Stays stopped (never auto-plays), located where the listener was.
            if self.transport.playing && self.bridge.transport(TransportControl::Stop).is_ok() {
                self.transport.playing = false;
            }
            if self
                .bridge
                .transport(TransportControl::Locate { position })
                .is_ok()
            {
                self.transport.position = position;
                self.transport.start_position = position;
            }
        }
        self.collab_listen_emit_if_changed(out);
        self.emit_transport_if_changed(out);
    }
}

/// The position heard `elapsed_ms` after `clock` arrived (extrapolated at most
/// [`MAX_EXTRAPOLATION_MS`], loop-wrapped, never negative).
fn last_heard(clock: &StreamClock, elapsed_ms: u64) -> Beats {
    let mut p = clock.position.0;
    if clock.playing {
        let dt = elapsed_ms.min(MAX_EXTRAPOLATION_MS) as f64 / 1000.0;
        let next = p + dt * clock.bpm / 60.0;
        let (start, end) = (clock.loop_region.start.0, clock.loop_region.end.0);
        p = if clock.loop_enabled && end > start && p < end && next >= end {
            start + (next - start).rem_euclid(end - start)
        } else {
            next
        };
    }
    Beats(if p.is_finite() { p.max(0.0) } else { 0.0 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ether_core::protocol::model::BeatRange;

    fn clock(position: f64, playing: bool, loop_enabled: bool) -> StreamClock {
        StreamClock {
            rtp: 0,
            position: Beats(position),
            playing,
            recording: false,
            bpm: 120.0,
            loop_enabled,
            loop_region: BeatRange {
                start: Beats(0.0),
                end: Beats(8.0),
            },
            metronome: false,
            discontinuity: false,
            count_in_end: None,
        }
    }

    #[test]
    fn last_heard_extrapolates_bounded_and_wraps() {
        assert_eq!(last_heard(&clock(4.0, false, false), 1_000), Beats(4.0));
        assert_eq!(last_heard(&clock(4.0, true, false), 100), Beats(4.2));
        // Capped: anchors stopped arriving long ago.
        assert_eq!(last_heard(&clock(4.0, true, false), 60_000), Beats(4.4));
        let wrapped = last_heard(&clock(7.9, true, true), 200).0;
        assert!((wrapped - 0.3).abs() < 1e-9, "{wrapped}");
        // Count-in pre-roll: never negative.
        assert_eq!(last_heard(&clock(-4.0, true, false), 100), Beats(0.0));
    }
}

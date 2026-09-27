//! "Listen on <peer>", host side (`stream-host` node; docs/COLLAB.md §9.2, §9.3, §9.5).
//!
//! - Hosting policy (`SetHosting`; default in a session: allow + remote transport, no UI
//!   sender). `can_host` in our presence = allowed ∧ a sender exists (the bridge's native
//!   sender, or a UI sender the UI declared) ∧ not listening to someone else.
//! - `Listen { from, stream }`: accepted up to [`MAX_LISTENERS`], else refused with a `Bye`.
//!   Endpoint `Engine` when the bridge has a native sender (`start_stream_capture` for the
//!   first engine listener, `stream_open` with our ICE servers; its offer, ICE, anchors and
//!   link states come back through `poll_stream`, drained every tick), else `Ui` when the UI
//!   declared a sender (the listener shows up in `ListenStatus.listeners`; its signals go to
//!   the UI, the UI's `SendStreamClock`s go to it).
//! - A stream ends on `Unlisten`, a `Bye` either way, a failed link, the listener's `Leave`,
//!   our `Leave`, or hosting turned off; the listener gets a `Bye` whenever it did not end
//!   the stream itself. `stream_close` + `stop_stream_capture` after the last engine
//!   listener.
//! - `TransportRequest`s from current listeners are applied like our own transport commands,
//!   in arrival order (loop changes as site-local settings outside the undo history), unless
//!   remote transport is off or we record / count in. Each listener gets at most
//!   [`REQUESTS_PER_SECOND`]; beyond that only its latest `Locate` and latest loop change
//!   are kept (applied when its budget refills), the rest is dropped.
//!
//! `CollabEvent::ListenStatus` (listener and host parts) is emitted by
//! `collab_listen_emit_status` (`listen.rs`), which reads [`Self::collab_host_listeners`];
//! this module calls it whenever the listener set changes.

use ether_core::protocol::collab::{
    CollabCommand, CollabEvent, CollabMessage, ListenerLink, PresenceState, StreamEndpoint,
    StreamSignal, TransportRequest,
};
use ether_core::protocol::model::{BeatRange, Beats, SiteId};
use ether_core::protocol::transport::TransportCommand;
use ether_core::protocol::{ClientMessage, Command, Event, ReplyValue};
use ether_model::op::{Op, SettingsChange};

use crate::handlers::event;
use crate::store::{Library, ProjectStore};
use crate::streaming::{StreamLinkState, StreamOutput};
use crate::tx::{CmdResult, invalid, invalid_state};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// Most listeners a site streams to at once (a mesh: one peer connection each).
pub const MAX_LISTENERS: usize = 8;

/// Transport requests applied per listener and second (burst: one second's worth).
pub const REQUESTS_PER_SECOND: f64 = 20.0;

/// Host state of a session (one field of `collab::Session`).
pub(crate) struct HostState {
    allow: bool,
    ui_sender: bool,
    remote_transport: bool,
    links: Vec<Link>,
    /// `start_stream_capture` succeeded and no `stop_stream_capture` since.
    capturing: bool,
    /// Scratch for `poll_stream`.
    polled: Vec<StreamOutput>,
}

impl Default for HostState {
    fn default() -> Self {
        Self {
            allow: true,
            ui_sender: false,
            remote_transport: true,
            links: Vec::new(),
            capturing: false,
            polled: Vec::new(),
        }
    }
}

struct Link {
    site: SiteId,
    stream: u32,
    endpoint: StreamEndpoint,
    /// Transport request budget (token bucket, [`REQUESTS_PER_SECOND`]).
    budget: f64,
    refilled_ms: u64,
    /// Over budget: the latest `Locate` / loop change, applied when the budget refills.
    held_locate: Option<Beats>,
    held_loop: Option<HeldLoop>,
}

#[derive(Clone, Copy)]
enum HeldLoop {
    Enabled(bool),
    Region(BeatRange),
}

impl Link {
    fn refill(&mut self, now: u64) {
        let dt = now.saturating_sub(self.refilled_ms) as f64 / 1000.0;
        self.budget = (self.budget + dt * REQUESTS_PER_SECOND).min(REQUESTS_PER_SECOND);
        self.refilled_ms = now;
    }

    fn take_token(&mut self) -> bool {
        if self.budget >= 1.0 {
            self.budget -= 1.0;
            true
        } else {
            false
        }
    }
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    fn host_state(&mut self) -> Option<&mut HostState> {
        self.collab.session.as_mut().map(|s| &mut s.host)
    }

    /// `CollabCommand::{SetHosting, SendStreamClock}`.
    pub(crate) fn collab_host_command(
        &mut self,
        c: &CollabCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let _ = now;
        match c {
            CollabCommand::SetHosting {
                allow,
                ui_sender,
                remote_transport,
            } => {
                let Some(h) = self.host_state() else {
                    // Outside a session: nothing to host (a session starts with the
                    // defaults; the UI declares its sender again after joining).
                    return Ok(ReplyValue::Unit);
                };
                let changed = h.allow != *allow || h.ui_sender != *ui_sender;
                h.allow = *allow;
                h.ui_sender = *ui_sender;
                h.remote_transport = *remote_transport;
                if !*allow {
                    self.collab_host_end_all("hosting was turned off", out);
                } else if !*ui_sender {
                    // The UI sender went away: its streams cannot continue.
                    let ui: Vec<(SiteId, u32)> = self
                        .host_state()
                        .map(|h| {
                            h.links
                                .iter()
                                .filter(|l| l.endpoint == StreamEndpoint::Ui)
                                .map(|l| (l.site, l.stream))
                                .collect()
                        })
                        .unwrap_or_default();
                    for (site, stream) in ui {
                        self.collab_host_end(site, stream, Some("the host stopped streaming"), out);
                    }
                }
                if changed && let Some(s) = self.collab.session.as_mut() {
                    // `can_host` may have changed.
                    s.presence_dirty = true;
                }
                Ok(ReplyValue::Unit)
            }
            CollabCommand::SendStreamClock { to, stream, clock } => {
                if !self.collab.session.as_ref().is_some_and(|s| s.joined) {
                    return Err(invalid_state("not in a collaboration session"));
                }
                if !clock.position.0.is_finite() || !clock.bpm.is_finite() {
                    return Err(invalid("stream clock values must be finite"));
                }
                // Stale clocks (the stream just ended) are dropped silently.
                if self.host_link(*to, *stream).map(|l| l.endpoint) == Some(StreamEndpoint::Ui) {
                    let from = self.collab_site();
                    if let Some(s) = self.collab.session.as_mut() {
                        s.send(&CollabMessage::StreamClock {
                            from,
                            to: *to,
                            stream: *stream,
                            clock: clock.clone(),
                        });
                    }
                }
                Ok(ReplyValue::Unit)
            }
            _ => Err(invalid("not a hosting command")),
        }
    }

    fn host_link(&self, site: SiteId, stream: u32) -> Option<&Link> {
        self.collab
            .session
            .as_ref()?
            .host
            .links
            .iter()
            .find(|l| l.site == site && l.stream == stream)
    }

    fn host_link_mut(&mut self, site: SiteId, stream: u32) -> Option<&mut Link> {
        self.host_state()?
            .links
            .iter_mut()
            .find(|l| l.site == site && l.stream == stream)
    }

    /// This site is listening to someone (the listener side's `listening_to`).
    fn collab_host_is_listening(&self) -> bool {
        let mut state = PresenceState::default();
        self.collab_listen_presence(&mut state);
        state.listening_to.is_some()
    }

    /// The endpoint that would stream to a new listener, if any.
    fn collab_host_endpoint(&self) -> Option<StreamEndpoint> {
        let h = &self.collab.session.as_ref()?.host;
        if self.bridge.stream_capabilities().native_sender {
            Some(StreamEndpoint::Engine)
        } else if h.ui_sender {
            Some(StreamEndpoint::Ui)
        } else {
            None
        }
    }

    /// `CollabMessage::{Listen, Unlisten, TransportRequest}` addressed to this site.
    pub(crate) fn collab_host_message(&mut self, m: CollabMessage, out: &mut dyn MessageSink) {
        match m {
            CollabMessage::Listen { from, stream, .. } => {
                self.collab_host_listen(from, stream, out)
            }
            CollabMessage::Unlisten { from, stream, .. } => {
                if self.host_link(from, stream).is_some() {
                    self.collab_host_end(from, stream, None, out);
                }
            }
            CollabMessage::TransportRequest {
                from,
                stream,
                request,
                ..
            } => self.collab_host_request(from, stream, request, out),
            _ => {}
        }
    }

    fn collab_host_listen(&mut self, from: SiteId, stream: u32, out: &mut dyn MessageSink) {
        let Some(h) = self.collab.session.as_ref().map(|s| &s.host) else {
            return;
        };
        if h.links.iter().any(|l| l.site == from && l.stream == stream) {
            return; // a duplicate
        }
        let allow = h.allow;
        // A new request from a current listener replaces its old stream.
        let old: Vec<u32> = h
            .links
            .iter()
            .filter(|l| l.site == from)
            .map(|l| l.stream)
            .collect();
        for s in old {
            self.collab_host_end(from, s, None, out);
        }
        let refuse = |me: &mut Self, reason: &str| {
            me.collab_send_routed_signal(
                from,
                stream,
                StreamSignal::Bye {
                    reason: Some(reason.into()),
                },
            );
        };
        if !allow {
            return refuse(self, "the host does not allow listening");
        }
        if self.collab_host_is_listening() {
            return refuse(self, "the host is listening to someone else");
        }
        let count = self.host_state().map_or(0, |h| h.links.len());
        if count >= MAX_LISTENERS {
            return refuse(
                self,
                &format!("the host already has {MAX_LISTENERS} listeners"),
            );
        }
        let Some(endpoint) = self.collab_host_endpoint() else {
            return refuse(self, "this peer cannot host a stream");
        };
        if endpoint == StreamEndpoint::Engine
            && let Err(e) = self.collab_host_open_engine(from, stream)
        {
            return refuse(self, &format!("the host could not stream: {e}"));
        }
        let now = self.host.now_ms();
        if let Some(h) = self.host_state() {
            h.links.push(Link {
                site: from,
                stream,
                endpoint,
                budget: REQUESTS_PER_SECOND,
                refilled_ms: now,
                held_locate: None,
                held_loop: None,
            });
        }
        self.collab_listen_emit_status(out);
    }

    fn collab_host_open_engine(&mut self, listener: SiteId, stream: u32) -> Result<(), String> {
        let capturing = self.host_state().is_some_and(|h| h.capturing);
        if !capturing {
            self.bridge
                .start_stream_capture()
                .map_err(|e| e.to_string())?;
            if let Some(h) = self.host_state() {
                h.capturing = true;
            }
        }
        let (ice, _) = self.collab_ice_servers();
        if let Err(e) = self.bridge.stream_open(listener, stream, &ice) {
            self.collab_host_stop_capture_if_idle();
            return Err(e.to_string());
        }
        Ok(())
    }

    fn collab_host_stop_capture_if_idle(&mut self) {
        let Some(h) = self.host_state() else { return };
        if h.capturing && !h.links.iter().any(|l| l.endpoint == StreamEndpoint::Engine) {
            h.capturing = false;
            // Best effort: the sender idles without listeners anyway.
            let _ = self.bridge.stop_stream_capture();
        }
    }

    /// End the stream to `site` (`bye`: tell the listener why; `None` when it ended the
    /// stream itself or is gone).
    fn collab_host_end(
        &mut self,
        site: SiteId,
        stream: u32,
        bye: Option<&str>,
        out: &mut dyn MessageSink,
    ) {
        let Some(h) = self.host_state() else { return };
        let Some(i) = h
            .links
            .iter()
            .position(|l| l.site == site && l.stream == stream)
        else {
            return;
        };
        let link = h.links.remove(i);
        if link.endpoint == StreamEndpoint::Engine {
            // Unknown streams are ignored by the bridge; nothing else can fail usefully.
            let _ = self.bridge.stream_close(site, stream);
            self.collab_host_stop_capture_if_idle();
        }
        if let Some(reason) = bye {
            self.collab_send_routed_signal(
                site,
                stream,
                StreamSignal::Bye {
                    reason: Some(reason.into()),
                },
            );
        }
        self.collab_listen_emit_status(out);
    }

    /// End every hosted stream with a `Bye` (`reason`). Also for the listener side: a host
    /// that starts listening elsewhere ends its own streams first (docs/COLLAB.md §9.3).
    pub(crate) fn collab_host_end_all(&mut self, reason: &str, out: &mut dyn MessageSink) {
        let links: Vec<(SiteId, u32)> = self
            .host_state()
            .map(|h| h.links.iter().map(|l| (l.site, l.stream)).collect())
            .unwrap_or_default();
        for (site, stream) in links {
            self.collab_host_end(site, stream, Some(reason), out);
        }
    }

    /// Whether `stream` from `peer` is one this site hosts (its signals go to
    /// [`Self::collab_host_signal`], the others to the listener side).
    pub(crate) fn collab_hosts_stream(&self, peer: SiteId, stream: u32) -> bool {
        self.host_link(peer, stream).is_some()
    }

    /// A `Signal` from listener `from` for a hosted stream (answer, ICE, bye).
    pub(crate) fn collab_host_signal(
        &mut self,
        from: SiteId,
        stream: u32,
        signal: StreamSignal,
        out: &mut dyn MessageSink,
    ) {
        let Some(endpoint) = self.host_link(from, stream).map(|l| l.endpoint) else {
            return;
        };
        let bye = matches!(signal, StreamSignal::Bye { .. });
        match endpoint {
            StreamEndpoint::Engine => {
                // A bad answer/candidate fails the link, reported by `poll_stream`.
                if !bye {
                    let _ = self.bridge.stream_signal(from, stream, &signal);
                }
            }
            StreamEndpoint::Ui => event(
                out,
                Event::Collab {
                    event: CollabEvent::Signal {
                        from,
                        stream,
                        signal,
                    },
                },
            ),
        }
        if bye {
            // The listener ended it: no `Bye` back.
            self.collab_host_end(from, stream, None, out);
        }
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
        if matches!(signal, StreamSignal::Bye { .. }) {
            // The `Bye` is already on its way.
            self.collab_host_end(to, stream, None, out);
        }
    }

    /// A listener's transport request (§9.5).
    fn collab_host_request(
        &mut self,
        from: SiteId,
        stream: u32,
        request: TransportRequest,
        out: &mut dyn MessageSink,
    ) {
        let remote = self.host_state().is_some_and(|h| h.remote_transport);
        if !remote || self.collab_host_busy() {
            return;
        }
        let now = self.host.now_ms();
        let Some(link) = self.host_link_mut(from, stream) else {
            return; // not a current listener (or a stale stream)
        };
        link.refill(now);
        if !link.take_token() {
            match request {
                TransportRequest::Locate { position } => link.held_locate = Some(position),
                TransportRequest::SetLoopEnabled { enabled } => {
                    link.held_loop = Some(HeldLoop::Enabled(enabled))
                }
                TransportRequest::SetLoopRegion { region } => {
                    link.held_loop = Some(HeldLoop::Region(region))
                }
                _ => {}
            }
            return;
        }
        // Applied now: an older held one of the same kind is superseded.
        match request {
            TransportRequest::Locate { .. } => link.held_locate = None,
            TransportRequest::SetLoopEnabled { .. } | TransportRequest::SetLoopRegion { .. } => {
                link.held_loop = None
            }
            _ => {}
        }
        self.collab_host_apply(request, now, out);
    }

    /// We record or count in: only the host stops its own recording.
    fn collab_host_busy(&self) -> bool {
        self.transport.recording || self.engine.count_in_end.is_some()
    }

    /// Apply a listener's request like our own transport command.
    fn collab_host_apply(
        &mut self,
        request: TransportRequest,
        now: u64,
        out: &mut dyn MessageSink,
    ) {
        let command = match request {
            TransportRequest::Play => TransportCommand::Play,
            TransportRequest::Stop => TransportCommand::Stop,
            TransportRequest::TogglePlay => TransportCommand::TogglePlay,
            TransportRequest::Locate { position } => {
                if !(position.0.is_finite() && position.0 >= 0.0) {
                    return;
                }
                TransportCommand::Locate { position }
            }
            TransportRequest::SetLoopEnabled { enabled } => {
                return self.collab_host_loop(SettingsChange::LoopEnabled(enabled), now, out);
            }
            TransportRequest::SetLoopRegion { region } => {
                if !(region.start.0.is_finite()
                    && region.end.0.is_finite()
                    && region.start.0 >= 0.0
                    && region.end.0 > region.start.0)
                {
                    return;
                }
                return self.collab_host_loop(SettingsChange::LoopRegion(region), now, out);
            }
        };
        let msg = ClientMessage {
            id: 0,
            gesture: None,
            command: Command::Transport(command),
        };
        // Like a UI command whose reply nobody reads: failures change nothing.
        let _ = self.dispatch(&msg, now, out);
        self.emit_transport_if_changed(out);
    }

    /// A forwarded loop change: a site-local setting, applied outside the undo history
    /// (like any remote change).
    fn collab_host_loop(&mut self, change: SettingsChange, now: u64, out: &mut dyn MessageSink) {
        let Some(doc) = self.doc.as_mut() else { return };
        let (applied, _) =
            super::resolve::resolve_all(&mut doc.project, &[Op::Settings { change }]);
        if applied.is_empty() {
            return;
        }
        self.after_ops(&applied, now, out);
        self.emit_transport_if_changed(out);
    }

    /// Every collab tick: drain `EngineBridge::poll_stream`, apply held requests.
    pub(crate) fn collab_host_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        let Some(h) = self.host_state() else { return };
        let mut polled = std::mem::take(&mut h.polled);
        let any_engine =
            h.capturing || h.links.iter().any(|l| l.endpoint == StreamEndpoint::Engine);
        if any_engine {
            self.bridge.poll_stream(&mut polled);
        }
        let me = self.collab_site();
        for o in polled.drain(..) {
            match o {
                StreamOutput::Signal {
                    listener,
                    stream,
                    signal,
                } => {
                    if self.host_link(listener, stream).is_none() {
                        continue;
                    }
                    let bye = matches!(signal, StreamSignal::Bye { .. });
                    self.collab_send_routed_signal(listener, stream, signal);
                    if bye {
                        self.collab_host_end(listener, stream, None, out);
                    }
                }
                StreamOutput::Clock {
                    listener,
                    stream,
                    clock,
                } => {
                    if self.host_link(listener, stream).is_none() {
                        continue;
                    }
                    if let Some(s) = self.collab.session.as_mut() {
                        s.send(&CollabMessage::StreamClock {
                            from: me,
                            to: listener,
                            stream,
                            clock,
                        });
                    }
                }
                StreamOutput::State {
                    listener,
                    stream,
                    state,
                } => match state {
                    StreamLinkState::Connected => {}
                    StreamLinkState::Failed { reason } => {
                        self.collab_host_end(listener, stream, Some(&reason), out)
                    }
                    StreamLinkState::Closed => {
                        self.collab_host_end(listener, stream, Some("the stream was closed"), out)
                    }
                },
            }
        }
        if let Some(h) = self.host_state() {
            h.polled = polled;
        }
        self.collab_host_apply_held(now, out);
    }

    /// Held (over-budget) `Locate`s and loop changes, once their listener's budget refills.
    fn collab_host_apply_held(&mut self, now: u64, out: &mut dyn MessageSink) {
        let Some(h) = self.host_state() else { return };
        if !h
            .links
            .iter()
            .any(|l| l.held_locate.is_some() || l.held_loop.is_some())
        {
            return;
        }
        let remote = h.remote_transport;
        let mut due = Vec::new();
        for l in &mut h.links {
            if l.held_locate.is_none() && l.held_loop.is_none() {
                continue;
            }
            if !remote {
                l.held_locate = None;
                l.held_loop = None;
                continue;
            }
            l.refill(now);
            if l.held_loop.is_some() && l.take_token() {
                due.push(match l.held_loop.take() {
                    Some(HeldLoop::Enabled(enabled)) => {
                        TransportRequest::SetLoopEnabled { enabled }
                    }
                    Some(HeldLoop::Region(region)) => TransportRequest::SetLoopRegion { region },
                    None => continue,
                });
            }
            if let Some(position) = l.held_locate
                && l.take_token()
            {
                l.held_locate = None;
                due.push(TransportRequest::Locate { position });
            }
        }
        if self.collab_host_busy() {
            return; // dropped: requests while recording are ignored
        }
        for r in due {
            self.collab_host_apply(r, now, out);
        }
    }

    /// Peer `site` left the session (it may be a listener).
    pub(crate) fn collab_host_peer_left(&mut self, site: SiteId, out: &mut dyn MessageSink) {
        let streams: Vec<u32> = self
            .host_state()
            .map(|h| {
                h.links
                    .iter()
                    .filter(|l| l.site == site)
                    .map(|l| l.stream)
                    .collect()
            })
            .unwrap_or_default();
        for stream in streams {
            self.collab_host_end(site, stream, None, out);
        }
    }

    /// We leave the session: close every stream.
    pub(crate) fn collab_host_session_end(&mut self, out: &mut dyn MessageSink) {
        self.collab_host_end_all("the host left the session", out);
        self.collab_host_stop_capture_if_idle();
    }

    /// Current listeners (the host part of `CollabEvent::ListenStatus`, read by
    /// `collab_listen_emit_status`).
    #[allow(dead_code)]
    pub(crate) fn collab_host_listeners(&self) -> Vec<ListenerLink> {
        self.collab
            .session
            .as_ref()
            .map(|s| {
                s.host
                    .links
                    .iter()
                    .map(|l| ListenerLink {
                        site: l.site,
                        stream: l.stream,
                        endpoint: l.endpoint,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Set the controller-owned `can_host` of our outgoing presence.
    pub(crate) fn collab_host_presence(&self, state: &mut PresenceState) {
        state.can_host = self.collab.session.as_ref().is_some_and(|s| s.host.allow)
            && self.collab_host_endpoint().is_some()
            && state.listening_to.is_none();
    }
}

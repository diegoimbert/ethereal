//! The joiner side of sharing (docs/SHARING.md §4.3, §7.2): opening an invite, the join
//! screen, the collab session over the data channel, reconnects, offline copies.
//!
//! A **dial** reaches the host: the join socket on the signaling service (`JoinHello` with
//! the door), then a WebRTC pairing (this side offers), then the handshake. A first join
//! (link key) stops at `Ready` until the user accepts; a member rejoin (member key: a
//! dropped link, a reopened offline copy) accepts at once.

use ether_collab::ConnectRequest;
use ether_collab::LinkState;
use ether_collab::share::file::{CopyShare, SHARE_FILE_VERSION, ShareFile};
use ether_collab::share::handshake;
use ether_collab::share::hub::SHARE_SESSION;
use ether_collab::share::invite::{Invite, InviteError, KEY_V1, LinkKey};
use ether_collab::share::keys;
use ether_collab::share::{BoxPeerLink, BoxSignalLink, PeerOutput};
use ether_core::protocol::collab::CollabCommand;
use ether_core::protocol::model::{Color, ProjectId};
use ether_core::protocol::share::{
    Credential, HostLink, InvitePreview, JoinFailure, JoinStage, MemberId, Participant,
    ParticipantRole, ParticipantSummary, PeerHandshake, PeerId, SIGNAL_PROTOCOL_VERSION,
    ShareNotice, ShareRole, ShareState, SignalClientMessage, SignalRefusal, SignalServerMessage,
};

use super::file::{read_share, write_share};
use super::port::Port;
use super::{Mode, notice, peer_ice, signal_socket_url};
use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, invalid_state};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// From the introduction to the host's `Welcome`.
const CONNECT_TIMEOUT_MS: u64 = 30_000;
/// The join socket is reopened this long after the service closed it while waiting for an
/// offline host.
const REOPEN_MS: u64 = 2_000;
const RETRY_MIN_MS: u64 = 500;
const RETRY_MAX_MS: u64 = 10_000;
/// Participants kept in `share.json` for Recents avatars.
const MAX_SAVED_PARTICIPANTS: usize = 8;

struct Conn {
    link: BoxPeerLink,
    local: String,
    remote: String,
}

struct Welcomed {
    conn: Conn,
    role: ShareRole,
    member: MemberId,
    member_key: Option<String>,
    host: ParticipantSummary,
    project: ProjectId,
    project_name: String,
    online: Vec<ParticipantSummary>,
}

enum DialEvent {
    Arrived,
    Offline(Option<f64>),
    Welcomed(Box<Welcomed>),
    Failed(JoinFailure, String),
}

struct Dial {
    signal: Option<BoxSignalLink>,
    hello_sent: bool,
    peer: Option<PeerId>,
    conn: Option<Conn>,
    since: u64,
    offline: bool,
    reopen_at: Option<u64>,
}

impl Dial {
    fn new(now: u64) -> Self {
        Self {
            signal: None,
            hello_sent: false,
            peer: None,
            conn: None,
            since: now,
            offline: false,
            reopen_at: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Phase {
    // `ShareState::Joining` stages.
    Contacting,
    Connecting,
    Ready,
    Syncing,
    Offline,
    Failed(JoinFailure, String),
    /// `ShareState::Joined`.
    Joined,
}

#[derive(Clone, Debug, PartialEq)]
enum Link {
    Connecting,
    Online,
    Offline(f64),
}

pub(crate) struct JoinRt {
    room: String,
    signal_url: Option<String>,
    key: LinkKey,
    credential: Credential,
    pub phase: Phase,
    dial: Option<Dial>,
    pub project: Option<ProjectId>,
    pub role: ShareRole,
    host: ParticipantSummary,
    project_name: String,
    member: Option<MemberId>,
    /// The member key issued on a first join (written to `share.json` once synced).
    member_key: Option<String>,
    online: Vec<ParticipantSummary>,
    local_copy: bool,
    /// `Ready`: the authenticated link, waiting for `AcceptInvite`.
    ready: Option<Conn>,
    port: Port,
    /// The collab session runs over [`Port`].
    started: bool,
    link: Link,
    attempt: u32,
    retry_at: u64,
    backoff: u64,
    copy_written: bool,
    listened: bool,
    own_color: Option<Color>,
    /// Last known participants (offline copy).
    saved: Vec<ParticipantSummary>,
}

impl JoinRt {
    fn new(room: String, signal_url: Option<String>, key: LinkKey, credential: Credential) -> Self {
        Self {
            room,
            signal_url,
            key,
            credential,
            phase: Phase::Contacting,
            dial: None,
            project: None,
            role: ShareRole::Edit,
            host: ParticipantSummary {
                name: String::new(),
                color: Color(0),
            },
            project_name: String::new(),
            member: None,
            member_key: None,
            online: Vec::new(),
            local_copy: false,
            ready: None,
            port: Port::default(),
            started: false,
            link: Link::Connecting,
            attempt: 0,
            retry_at: 0,
            backoff: RETRY_MIN_MS,
            copy_written: false,
            listened: false,
            own_color: None,
            saved: Vec::new(),
        }
    }

    fn failed(reason: JoinFailure, message: String) -> Self {
        let mut j = Self::new(
            String::new(),
            None,
            LinkKey {
                version: KEY_V1,
                secret: String::new(),
            },
            Credential::Link,
        );
        j.phase = Phase::Failed(reason, message);
        j
    }

    pub(crate) fn view_only(&self) -> bool {
        self.role == ShareRole::Listen
            && self.started
            && matches!(self.phase, Phase::Joined | Phase::Syncing)
    }

    fn first_join(&self) -> bool {
        matches!(self.credential, Credential::Link)
    }

    fn server(&self) -> String {
        format!("share://{}", self.room)
    }
}

fn refusal_failure(r: SignalRefusal) -> JoinFailure {
    match r {
        SignalRefusal::InvalidInvite => JoinFailure::InvalidInvite,
        SignalRefusal::Version => JoinFailure::Version,
        SignalRefusal::RoomFull | SignalRefusal::RateLimited => JoinFailure::Refused,
        SignalRefusal::NotHost | SignalRefusal::Malformed => JoinFailure::Network,
    }
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    fn take_join(&mut self) -> Option<Box<JoinRt>> {
        match std::mem::replace(&mut self.share.mode, Mode::Off) {
            Mode::Join(j) => Some(j),
            other => {
                self.share.mode = other;
                None
            }
        }
    }

    /// `OpenInvite`.
    pub(crate) fn share_open_invite(
        &mut self,
        link: &str,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<()> {
        match &self.share.mode {
            Mode::Host(_) => {
                return Err(invalid_state(
                    "stop sharing your project before joining another one",
                ));
            }
            Mode::Join(j) if j.started => {
                return Err(invalid_state("leave the current session first"));
            }
            Mode::Join(_) => self.share_join_end(out, true),
            Mode::Off => {}
        }
        let invite = match Invite::parse(link) {
            Ok(i) => i,
            Err(e) => {
                let reason = match e {
                    InviteError::UnknownKeyVersion => JoinFailure::Version,
                    _ => JoinFailure::BadLink,
                };
                self.share.mode = Mode::Join(Box::new(JoinRt::failed(reason, e.to_string())));
                return Ok(());
            }
        };
        let Some(key) = invite.key else {
            self.share.mode = Mode::Join(Box::new(JoinRt::failed(
                JoinFailure::BadLink,
                "this link has no key (joining with an account is not supported yet)".into(),
            )));
            return Ok(());
        };
        let mut j = JoinRt::new(invite.room, invite.signal_url, key, Credential::Link);
        j.dial = Some(Dial::new(now));
        self.share.mode = Mode::Join(Box::new(j));
        Ok(())
    }

    /// Reconnect an offline copy (`Reconnect`, or opening it).
    pub(crate) fn share_join_copy(
        &mut self,
        pid: ProjectId,
        c: CopyShare,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<()> {
        let Some(key) = keys::parse_key(&c.key) else {
            return Err(invalid_state("sharing ended for this project"));
        };
        if let Mode::Join(_) = &self.share.mode {
            self.share_join_end(out, false);
        }
        let mut j = JoinRt::new(
            c.room,
            c.signal_url,
            key,
            Credential::Member {
                member: c.member.clone(),
            },
        );
        j.project = Some(pid);
        j.role = c.role;
        j.member = Some(c.member);
        j.host = c
            .participants
            .first()
            .cloned()
            .unwrap_or(ParticipantSummary {
                name: c.host_name.clone(),
                color: Color(0),
            });
        j.host.name = c.host_name;
        j.saved = c.participants;
        j.copy_written = true;
        j.local_copy = true;
        j.dial = Some(Dial::new(now));
        self.share.mode = Mode::Join(Box::new(j));
        Ok(())
    }

    pub(crate) fn share_reconnect(
        &mut self,
        project: ProjectId,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<()> {
        match &self.share.mode {
            Mode::Join(j) if j.project == Some(project) => return Ok(()),
            Mode::Host(_) => {
                return Err(invalid_state("stop sharing your project first"));
            }
            _ => {}
        }
        let c = match read_share(&mut self.store, project) {
            Some(ShareFile::Copy(c)) if !c.key.is_empty() => c,
            Some(ShareFile::Copy(_)) => {
                return Err(invalid_state("sharing ended for this project"));
            }
            _ => return Err(invalid_state("this project is not a shared copy")),
        };
        if self.doc.as_ref().map(|d| d.project.id) != Some(project) {
            self.open(project, now, out)?;
        }
        self.share_join_copy(project, c, now, out)
    }

    /// `AcceptInvite`: start the collab session over the authenticated link.
    pub(crate) fn share_accept(&mut self, now: u64, out: &mut dyn MessageSink) -> CmdResult<()> {
        let Some(mut j) = self.take_join() else {
            return Err(invalid_state("no invite to accept"));
        };
        let Some(mut conn) = j.ready.take().filter(|_| j.phase == Phase::Ready) else {
            let phase = j.phase.clone();
            self.share.mode = Mode::Join(j);
            return Err(invalid_state(format!(
                "the invite is not ready to accept ({phase:?})"
            )));
        };
        conn.link.send(&handshake::encode(&PeerHandshake::Accept));
        j.port.install(conn.link);
        // From now on this app is a member: it reconnects with its member key.
        if let (Some(member), Some(key)) = (
            j.member.clone(),
            j.member_key.as_deref().and_then(keys::parse_key),
        ) {
            j.credential = Credential::Member { member };
            j.key = key;
        }
        self.share_join_start_session(&mut j, now, out);
        j.phase = Phase::Syncing;
        self.share.mode = Mode::Join(j);
        Ok(())
    }

    fn share_join_start_session(&mut self, j: &mut JoinRt, now: u64, out: &mut dyn MessageSink) {
        let name = self.share_name();
        let request = ConnectRequest {
            server: j.server(),
            session: SHARE_SESSION.into(),
            token: None,
            client: self.share_app(),
        };
        self.collab_start(request, &name, Some(j.port.connector()), now, out);
        j.started = true;
        j.link = Link::Connecting;
    }

    /// `Leave`.
    pub(crate) fn share_leave(&mut self, out: &mut dyn MessageSink) -> CmdResult<()> {
        match &self.share.mode {
            Mode::Host(_) => Err(invalid_state(
                "you are sharing this project: use Stop sharing",
            )),
            Mode::Off => Ok(()),
            Mode::Join(_) => {
                self.share_join_end(out, true);
                Ok(())
            }
        }
    }

    /// End the joiner side (leave, cancel, another project opened). The project stays as
    /// an offline copy (saved, `share.json` kept).
    pub(crate) fn share_join_end(&mut self, out: &mut dyn MessageSink, save: bool) {
        let Some(mut j) = self.take_join() else {
            return;
        };
        self.share_dial_close(&mut j);
        if let Some(mut c) = j.ready.take() {
            c.link.close();
        }
        if j.started {
            if self.collab_view().is_some_and(|v| v.server == j.server()) {
                self.collab_leave(out);
            }
            j.port.end("left the session");
            if save
                && self.doc.as_ref().is_some_and(|d| d.dirty)
                && self.doc.as_ref().map(|d| d.project.id) == j.project
            {
                let _ = self.save_current(out);
            }
            self.emit_list_changed(out);
        }
    }

    fn share_dial_close(&mut self, j: &mut JoinRt) {
        if let Some(mut d) = j.dial.take() {
            if let Some(mut s) = d.signal.take() {
                s.close();
            }
            if let Some(mut c) = d.conn.take() {
                c.link.close();
            }
            if let Some(p) = d.peer {
                self.share_services().peers.close(p);
            }
        }
    }

    fn share_dial_tick(&mut self, j: &mut JoinRt, now: u64) -> Vec<DialEvent> {
        let mut events = Vec::new();
        let relay_only = self.share.prefs.relay_only;
        let ice_override = self.collab_ice_override().map(<[_]>::to_vec);
        let name = self.share_name();
        let color = self.share.color;
        let app = self.share_app();
        let url = signal_socket_url(j.signal_url.as_deref(), &j.room, "join");
        let services = self
            .share
            .services
            .get_or_insert_with(ether_collab::share::default_services);
        let Some(d) = j.dial.as_mut() else {
            return events;
        };
        if d.signal.is_none() && d.conn.is_none() && d.peer.is_none() {
            if d.reopen_at.is_some_and(|t| now < t) {
                return events;
            }
            d.signal = Some((services.signal)(&url));
            d.hello_sent = false;
            d.reopen_at = None;
        }
        // The join socket.
        let mut messages = Vec::new();
        let mut closed = None;
        if let Some(sig) = d.signal.as_mut() {
            sig.poll(&mut messages);
            match sig.state() {
                LinkState::Open => {
                    if !d.hello_sent {
                        sig.send(&SignalClientMessage::JoinHello {
                            protocol: SIGNAL_PROTOCOL_VERSION,
                            door: keys::door(&j.key, &j.room),
                            app: app.clone(),
                        });
                        d.hello_sent = true;
                    }
                }
                LinkState::Connecting => {}
                LinkState::Closed { reason, fatal } => {
                    d.signal = None;
                    closed = Some((reason, fatal));
                }
            }
        }
        for m in messages {
            match m {
                SignalServerMessage::JoinWelcome { peer, ice_servers } => {
                    d.peer = Some(peer);
                    d.offline = false;
                    d.since = now;
                    let ice = peer_ice(ice_override.as_deref(), &ice_servers, relay_only);
                    services.peers.open(peer, true, &ice);
                    events.push(DialEvent::Arrived);
                }
                SignalServerMessage::HostOffline { last_seen_ms } => {
                    d.offline = true;
                    events.push(DialEvent::Offline(last_seen_ms));
                }
                SignalServerMessage::Signal { signal, .. } => {
                    if let Some(p) = d.peer {
                        services.peers.signal(p, signal);
                    }
                }
                SignalServerMessage::Refused { reason, message } => {
                    events.push(DialEvent::Failed(refusal_failure(reason), message));
                    return events;
                }
                _ => {}
            }
        }
        // The peer connection.
        let mut outputs = Vec::new();
        services.peers.poll(&mut outputs);
        for o in outputs {
            match o {
                PeerOutput::Signal { peer, signal } if Some(peer) == d.peer => {
                    if let Some(s) = d.signal.as_mut() {
                        s.send(&SignalClientMessage::Signal { peer, signal });
                    }
                }
                PeerOutput::Connected {
                    peer,
                    mut link,
                    local_fingerprint,
                    remote_fingerprint,
                } => {
                    if Some(peer) != d.peer || d.conn.is_some() {
                        link.close();
                        continue;
                    }
                    link.send(&handshake::encode(&handshake::hello(
                        j.credential.clone(),
                        &j.key,
                        &local_fingerprint,
                        &remote_fingerprint,
                        &name,
                        color,
                        &app,
                    )));
                    d.conn = Some(Conn {
                        link,
                        local: local_fingerprint,
                        remote: remote_fingerprint,
                    });
                    // The introduction is over.
                    if let Some(mut s) = d.signal.take() {
                        s.close();
                    }
                }
                PeerOutput::Failed { peer, reason } if Some(peer) == d.peer => {
                    events.push(DialEvent::Failed(JoinFailure::Unreachable, reason));
                    return events;
                }
                _ => {}
            }
        }
        // The socket closed before the data channel was up (checked after the peer
        // connection, which may have failed first).
        if let Some((reason, fatal)) = closed
            && d.conn.is_none()
        {
            if d.offline && !fatal {
                // The service closes waiting sockets after a while: wait again.
                d.peer = None;
                d.reopen_at = Some(now + REOPEN_MS);
            } else {
                events.push(DialEvent::Failed(JoinFailure::Network, reason));
                return events;
            }
        }
        // The handshake.
        if let Some(c) = d.conn.as_mut() {
            let mut frames = Vec::new();
            c.link.poll(&mut frames);
            if let Some(f) = frames.first() {
                match handshake::decode(f) {
                    Ok(w @ PeerHandshake::Welcome { .. }) => {
                        if !handshake::verify_welcome(&w, &j.key, &c.remote, &c.local) {
                            c.link.close();
                            events.push(DialEvent::Failed(
                                JoinFailure::HostNotVerified,
                                "the host's identity could not be verified".into(),
                            ));
                            return events;
                        }
                        let PeerHandshake::Welcome {
                            role,
                            member,
                            member_key,
                            host,
                            project,
                            project_name,
                            online,
                            ..
                        } = w
                        else {
                            unreachable!()
                        };
                        let conn = d.conn.take().expect("checked");
                        j.dial = None;
                        events.push(DialEvent::Welcomed(Box::new(Welcomed {
                            conn,
                            role,
                            member,
                            member_key,
                            host,
                            project,
                            project_name,
                            online,
                        })));
                        return events;
                    }
                    Ok(PeerHandshake::Refused { reason, message }) => {
                        events.push(DialEvent::Failed(reason, message));
                        return events;
                    }
                    _ => {
                        events.push(DialEvent::Failed(
                            JoinFailure::Network,
                            "unexpected answer from the host".into(),
                        ));
                        return events;
                    }
                }
            }
            if matches!(c.link.state(), LinkState::Closed { .. }) {
                events.push(DialEvent::Failed(
                    JoinFailure::Network,
                    "the connection to the host was lost".into(),
                ));
                return events;
            }
        }
        if d.peer.is_some() && !d.offline && now >= d.since + CONNECT_TIMEOUT_MS {
            events.push(DialEvent::Failed(
                JoinFailure::Unreachable,
                "couldn't reach the host's computer".into(),
            ));
        }
        events
    }

    pub(crate) fn share_join_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        let Some(mut j) = self.take_join() else {
            return;
        };
        if j.started {
            j.own_color = self.social_author().and_then(|a| a.color).or(j.own_color);
        }
        // Need a dial? (a dropped link the session wants back, or a retry)
        if j.dial.is_none() && now >= j.retry_at {
            let wanted = if j.started {
                j.port.wanted()
            } else {
                !j.first_join() && matches!(j.phase, Phase::Joined | Phase::Contacting)
            };
            if wanted {
                j.dial = Some(Dial::new(now));
                j.attempt += 1;
                if j.link == Link::Online {
                    j.link = Link::Connecting;
                }
            }
        }
        let events = self.share_dial_tick(&mut j, now);
        for ev in events {
            match ev {
                DialEvent::Arrived => {
                    if matches!(j.phase, Phase::Contacting | Phase::Offline) {
                        j.phase = Phase::Connecting;
                    }
                }
                DialEvent::Offline(last_seen) => {
                    let since = last_seen.unwrap_or(now as f64);
                    if j.first_join() && !j.started {
                        j.phase = Phase::Offline;
                    } else {
                        if !matches!(j.link, Link::Offline(_)) {
                            notice(
                                out,
                                ShareNotice::HostOffline {
                                    host_name: j.host.name.clone(),
                                },
                            );
                        }
                        j.link = Link::Offline(since);
                        if !j.started {
                            j.phase = Phase::Joined;
                        }
                    }
                }
                DialEvent::Welcomed(w) => self.share_join_welcomed(&mut j, *w, now, out),
                DialEvent::Failed(reason, message) => {
                    self.share_dial_close(&mut j);
                    if reason == JoinFailure::InvalidInvite && !j.first_join() {
                        // Removed, or sharing stopped (one answer for both).
                        self.share_sharing_ended(j, out);
                        return;
                    }
                    // Terminal: a first join's failure; a version or identity problem.
                    let terminal = j.first_join()
                        || matches!(reason, JoinFailure::Version | JoinFailure::HostNotVerified);
                    if terminal && !j.started {
                        j.phase = Phase::Failed(reason, message);
                    } else {
                        j.retry_at = now + j.backoff;
                        j.backoff = (j.backoff * 2).min(RETRY_MAX_MS);
                        if !j.started && j.phase != Phase::Joined {
                            j.phase = Phase::Contacting;
                        }
                    }
                }
            }
        }
        // `Ready`: the link must stay up while the user decides.
        if j.phase == Phase::Ready
            && j.ready
                .as_ref()
                .is_none_or(|c| matches!(c.link.state(), LinkState::Closed { .. }))
        {
            j.ready = None;
            j.phase = Phase::Failed(
                JoinFailure::Network,
                format!("the connection to {} was lost", j.host.name),
            );
        }
        if j.started && !self.share_join_session(&mut j, now, out) {
            return;
        }
        self.share.mode = Mode::Join(j);
    }

    /// The host authenticated itself.
    fn share_join_welcomed(
        &mut self,
        j: &mut JoinRt,
        w: Welcomed,
        now: u64,
        out: &mut dyn MessageSink,
    ) {
        if j.project.is_some_and(|p| p != w.project) {
            let mut c = w.conn;
            c.link.close();
            j.phase = Phase::Failed(
                JoinFailure::HostNotVerified,
                "the host shares another project now".into(),
            );
            return;
        }
        j.role = w.role;
        j.host = w.host;
        j.project = Some(w.project);
        j.project_name = w.project_name;
        j.member = Some(w.member);
        if w.member_key.is_some() {
            j.member_key = w.member_key;
        }
        j.online = w.online;
        j.backoff = RETRY_MIN_MS;
        if j.first_join() && !j.started {
            j.local_copy = self.store.load(w.project).is_ok();
            j.ready = Some(w.conn);
            j.phase = Phase::Ready;
            return;
        }
        // A member: no confirmation.
        let mut conn = w.conn;
        conn.link.send(&handshake::encode(&PeerHandshake::Accept));
        j.port.install(conn.link);
        if !j.started {
            self.share_join_start_session(j, now, out);
            if j.phase != Phase::Joined {
                j.phase = Phase::Syncing;
            }
        }
    }

    /// The collab session over the port. Returns false when the join ended.
    fn share_join_session(&mut self, j: &mut JoinRt, now: u64, out: &mut dyn MessageSink) -> bool {
        let Some(view) = self.collab_view().filter(|v| v.server == j.server()) else {
            // Replaced by another session (Settings > Relay session) or ended.
            j.port.end("left the session");
            self.emit_list_changed(out);
            return false;
        };
        let up = j.port.link_open() && view.open;
        if up && j.dial.is_none() {
            if matches!(j.link, Link::Offline(_)) {
                notice(
                    out,
                    ShareNotice::HostBack {
                        host_name: j.host.name.clone(),
                    },
                );
            }
            if j.link != Link::Online {
                j.link = Link::Online;
                j.attempt = 0;
            }
        } else if !up && j.link == Link::Online {
            j.link = Link::Connecting;
            j.listened = false;
        }
        let open = self.doc.as_ref().map(|d| d.project.id);
        let synced = view.synced && view.project == j.project && open == j.project;
        if synced && j.phase == Phase::Syncing {
            j.phase = Phase::Joined;
        }
        if synced && j.phase == Phase::Joined && j.link == Link::Online {
            if !j.copy_written {
                self.share_write_copy(j, now);
                self.emit_list_changed(out);
            }
            // A listen link hears the host's mix (docs/SHARING.md §2.3).
            if j.role == ShareRole::Listen
                && self.share.prefs.auto_listen
                && !j.listened
                && let Some(host) = host_site(j, &view.peers)
            {
                j.listened = true;
                let _ = self.collab_command(&CollabCommand::Listen { host }, out);
            }
        }
        true
    }

    fn share_write_copy(&mut self, j: &mut JoinRt, now: u64) {
        let (Some(pid), Some(member)) = (j.project, j.member.clone()) else {
            return;
        };
        let key = match (&j.member_key, read_share(&mut self.store, pid)) {
            (Some(k), _) => k.clone(),
            (None, Some(ShareFile::Copy(c))) => c.key,
            _ => return,
        };
        let mut participants = vec![j.host.clone()];
        participants.extend(j.online.iter().cloned());
        participants.truncate(MAX_SAVED_PARTICIPANTS);
        let copy = CopyShare {
            version: SHARE_FILE_VERSION,
            room: j.room.clone(),
            signal_url: j.signal_url.clone(),
            member,
            key,
            role: j.role,
            host_name: j.host.name.clone(),
            participants,
            last_synced_ms: Some(now as f64),
        };
        if write_share(&mut self.store, pid, &ShareFile::Copy(copy)).is_ok() {
            j.copy_written = true;
        }
    }

    /// The host stopped sharing or removed us: the copy stays, "Sharing ended".
    fn share_sharing_ended(&mut self, j: Box<JoinRt>, out: &mut dyn MessageSink) {
        notice(
            out,
            ShareNotice::SharingEnded {
                host_name: j.host.name.clone(),
            },
        );
        if let Some(pid) = j.project
            && let Some(ShareFile::Copy(mut c)) = read_share(&mut self.store, pid)
        {
            c.key = String::new();
            let _ = write_share(&mut self.store, pid, &ShareFile::Copy(c));
        }
        j.port.end("sharing ended");
        if j.started && self.collab_view().is_some_and(|v| v.server == j.server()) {
            self.collab_leave(out);
        }
        self.share.mode = Mode::Off;
        self.emit_list_changed(out);
    }

    pub(crate) fn share_join_state(&self, j: &JoinRt) -> ShareState {
        let stage = match &j.phase {
            Phase::Contacting => JoinStage::Contacting,
            Phase::Connecting => JoinStage::Connecting,
            Phase::Ready => JoinStage::Ready {
                invite: InvitePreview {
                    host: j.host.clone(),
                    project: j.project.unwrap_or(ProjectId::NIL),
                    project_name: j.project_name.clone(),
                    role: j.role,
                    online: j.online.clone(),
                    local_copy: j.local_copy,
                },
            },
            Phase::Syncing => JoinStage::Syncing {
                received_bytes: j.port.received() as f64,
                total_bytes: None,
            },
            Phase::Offline => JoinStage::HostOffline {
                local_copy: j.project.filter(|_| j.local_copy),
            },
            Phase::Failed(reason, message) => JoinStage::Failed {
                reason: *reason,
                message: message.clone(),
            },
            Phase::Joined => {
                return ShareState::Joined {
                    project: j.project.unwrap_or(ProjectId::NIL),
                    role: j.role,
                    participants: self.share_join_participants(j),
                    link: match j.link {
                        Link::Online => HostLink::Online,
                        Link::Connecting => HostLink::Connecting {
                            attempt: j.attempt.max(1),
                        },
                        Link::Offline(since_ms) => HostLink::HostOffline { since_ms },
                    },
                };
            }
        };
        ShareState::Joining { stage }
    }

    fn share_join_participants(&self, j: &JoinRt) -> Vec<Participant> {
        let online = j.link == Link::Online;
        let view = self.collab_view().filter(|v| v.server == j.server());
        let peers = view.as_ref().map_or(&[][..], |v| v.peers.as_slice());
        let host = host_site(j, peers);
        let mine = view.as_ref().and_then(|v| v.site);
        let mut v = vec![Participant {
            member: None,
            site: host,
            name: j.host.name.clone(),
            color: j.host.color,
            role: ParticipantRole::Host,
            online: online && host.is_some(),
            you: false,
            last_seen_ms: None,
        }];
        v.push(Participant {
            member: j.member.clone(),
            site: mine,
            name: self.share_name(),
            color: j.own_color.or(self.share.color).unwrap_or(Color(0)),
            role: match j.role {
                ShareRole::Edit => ParticipantRole::Edit,
                ShareRole::Listen => ParticipantRole::Listen,
            },
            online,
            you: true,
            last_seen_ms: None,
        });
        if online {
            for p in peers {
                if Some(p.site) == host || Some(p.site) == mine {
                    continue;
                }
                v.push(Participant {
                    member: None,
                    site: Some(p.site),
                    name: p.name.clone(),
                    color: p.color,
                    // Joiners don't learn each other's roles.
                    role: ParticipantRole::Edit,
                    online: true,
                    you: false,
                    last_seen_ms: None,
                });
            }
        } else {
            for p in j.saved.iter().skip(1) {
                v.push(Participant {
                    member: None,
                    site: None,
                    name: p.name.clone(),
                    color: p.color,
                    role: ParticipantRole::Edit,
                    online: false,
                    you: false,
                    last_seen_ms: None,
                });
            }
        }
        v
    }
}

/// The host's site among the session's peers: its authenticated name and colour (the hub
/// pins names; the host's colour is the first and is never given to anyone else).
fn host_site(
    j: &JoinRt,
    peers: &[ether_core::protocol::collab::Presence],
) -> Option<ether_core::protocol::model::SiteId> {
    peers
        .iter()
        .find(|p| p.name == j.host.name && p.color == j.host.color)
        .or_else(|| peers.iter().find(|p| p.name == j.host.name))
        .map(|p| p.site)
}

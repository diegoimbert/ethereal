//! The host side of sharing (docs/SHARING.md §2, §4, §7.1): `share.json`, the hub, the
//! signaling socket, pairings and their handshakes, links, members.

use std::collections::BTreeMap;

use ether_collab::ConnectRequest;
use ether_collab::LinkState;
use ether_collab::share::file::{HostShare, MemberRecord, SHARE_FILE_VERSION, ShareFile};
use ether_collab::share::handshake::{
    self, ACCEPT_TIMEOUT_MS, Admitted, HELLO_TIMEOUT_MS, HostKeys, Refusal,
};
use ether_collab::share::hub::{Hub, HubConfig, SHARE_SESSION};
use ether_collab::share::invite::{Invite, LinkKey};
use ether_collab::share::keys;
use ether_collab::share::{BoxPeerLink, BoxSignalLink, PeerOutput};
use ether_core::protocol::collab::IceServer;
use ether_core::protocol::model::{Color, ProjectId};
use ether_core::protocol::share::{
    DEFAULT_INVITE_ORIGIN, JoinFailure, MAX_MEMBERS, MAX_PARTICIPANTS, MemberId, Participant,
    ParticipantRole, ParticipantSummary, PeerHandshake, PeerId, SIGNAL_PROTOCOL_VERSION,
    ShareNotice, ShareRole, ShareState, SignalClientMessage, SignalServerMessage, SignalStatus,
};

use super::file::{read_share, write_share};
use super::{Mode, clean_name, notice, peer_ice, signal_socket_url};
use crate::handlers::no_project;
use crate::store::{Library, ProjectStore};
use crate::tx::{CmdResult, internal, invalid, invalid_state};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

const SIGNAL_RETRY_MIN_MS: u64 = 1_000;
const SIGNAL_RETRY_MAX_MS: u64 = 30_000;
const PING_EVERY_MS: u64 = 30_000;
/// `share.json` is rewritten at most this often for `sites`/`last_seen` updates.
const SAVE_EVERY_MS: u64 = 2_000;
/// Pairings waiting for ICE at once.
const MAX_PAIRINGS: usize = 32;
/// ICE + DTLS must complete within this.
const ICE_TIMEOUT_MS: u64 = 30_000;

struct Conn {
    link: BoxPeerLink,
    local: String,
    remote: String,
}

struct Pending {
    admitted: Admitted,
    member: MemberId,
    member_key: Option<LinkKey>,
    name: String,
    color: Option<Color>,
}

enum Stage {
    Ice,
    Hello(Conn),
    Accept(Conn, Pending),
}

struct Pairing {
    since: u64,
    stage: Stage,
}

pub(crate) struct HostRt {
    pub project: ProjectId,
    pub file: HostShare,
    hub: Hub,
    signal: Option<BoxSignalLink>,
    hello_sent: bool,
    pub status: SignalStatus,
    retry_at: u64,
    backoff: u64,
    last_ping: u64,
    ice: Vec<IceServer>,
    pairings: BTreeMap<PeerId, Pairing>,
    /// Members online, with their pairing (closed on the endpoint when they leave).
    live: BTreeMap<MemberId, PeerId>,
    doors_dirty: bool,
    file_dirty: bool,
    saved_at: u64,
}

fn share_server(room: &str) -> String {
    format!("share://{room}")
}

fn key_of(text: &Option<String>) -> Option<LinkKey> {
    text.as_deref().and_then(keys::parse_key)
}

/// SHA-256 of every valid door (both links, every member).
fn doors(f: &HostShare) -> Vec<String> {
    let mut v: Vec<String> = [key_of(&f.edit_key), key_of(&f.listen_key)]
        .into_iter()
        .flatten()
        .chain(f.members.iter().filter_map(|m| keys::parse_key(&m.key)))
        .map(|k| keys::door_hash(&keys::door(&k, &f.room)))
        .collect();
    v.sort();
    v.dedup();
    v
}

fn new_host_share(signal_url: Option<String>) -> Result<HostShare, String> {
    Ok(HostShare {
        version: SHARE_FILE_VERSION,
        room: keys::new_id()?,
        signal_url,
        host_token: keys::new_host_token()?,
        edit_key: Some(keys::new_key()?.to_string()),
        listen_key: Some(keys::new_key()?.to_string()),
        members: Vec::new(),
        sites: BTreeMap::new(),
        resume: true,
    })
}

impl HostRt {
    fn end_peer(&mut self, peer: PeerId, reason: &str) {
        if let Some(s) = self.signal.as_mut()
            && s.state() == LinkState::Open
        {
            s.send(&SignalClientMessage::EndPeer {
                peer,
                reason: Some(reason.to_string()),
            });
        }
    }

    fn send_doors(&mut self) {
        if !self.doors_dirty || !self.hello_sent {
            return;
        }
        if let Some(s) = self.signal.as_mut()
            && s.state() == LinkState::Open
        {
            s.send(&SignalClientMessage::SetDoors {
                doors: doors(&self.file),
            });
            self.doors_dirty = false;
        }
    }

    fn member_mut(&mut self, member: &str) -> Option<&mut MemberRecord> {
        self.file.members.iter_mut().find(|m| m.member == member)
    }
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    fn take_host(&mut self) -> Option<Box<HostRt>> {
        match std::mem::replace(&mut self.share.mode, Mode::Off) {
            Mode::Host(h) => Some(h),
            other => {
                self.share.mode = other;
                None
            }
        }
    }

    fn host_mut(&mut self) -> CmdResult<&mut HostRt> {
        match &mut self.share.mode {
            Mode::Host(h) => Ok(h),
            _ => Err(invalid_state("this project is not shared")),
        }
    }

    /// `Start`: share the open project (resuming its links if it was shared before).
    pub(crate) fn share_start(&mut self, now: u64, out: &mut dyn MessageSink) -> CmdResult<()> {
        let pid = self.doc.as_ref().ok_or_else(no_project)?.project.id;
        match &self.share.mode {
            Mode::Host(h) if h.project == pid => return Ok(()),
            Mode::Host(_) => self.share_host_pause(out),
            Mode::Join(_) => {
                return Err(invalid_state(
                    "leave the shared session before sharing a project",
                ));
            }
            Mode::Off => {}
        }
        let mut file = match read_share(&mut self.store, pid) {
            Some(ShareFile::Host(h)) => h,
            Some(ShareFile::Copy(_)) => {
                return Err(invalid_state(
                    "this is a copy of someone else's project: make a private copy to share it",
                ));
            }
            None => new_host_share(self.share.signal_url.clone()).map_err(internal)?,
        };
        if file.edit_key.is_none() && file.listen_key.is_none() && file.members.is_empty() {
            // Stopped before (links revoked): a new share is a new room.
            file = new_host_share(self.share.signal_url.clone()).map_err(internal)?;
        }
        file.resume = true;
        write_share(&mut self.store, pid, &ShareFile::Host(file.clone())).map_err(internal)?;
        let name = self.share_name();
        let color = self.share.color;
        let hub = Hub::new(HubConfig::default());
        // Sites the document includes (across host restarts): returning members' resends
        // are recognized (docs/SHARING.md §7.1).
        self.collab_seed_sites(pid, &file.sites);
        let request = ConnectRequest {
            server: share_server(&file.room),
            session: SHARE_SESSION.into(),
            token: None,
            client: self.share_app(),
        };
        self.collab_start(request, &name, Some(hub.connector(&name, color)), now, out);
        self.share.mode = Mode::Host(Box::new(HostRt {
            project: pid,
            file,
            hub,
            signal: None,
            hello_sent: false,
            status: SignalStatus::Connecting,
            retry_at: now,
            backoff: SIGNAL_RETRY_MIN_MS,
            last_ping: now,
            ice: Vec::new(),
            pairings: BTreeMap::new(),
            live: BTreeMap::new(),
            doors_dirty: false,
            file_dirty: false,
            saved_at: now,
        }));
        self.emit_list_changed(out);
        Ok(())
    }

    /// Persist the host's `share.json` (with the sites the collab session knows).
    fn share_host_save(&mut self, h: &mut HostRt) {
        if let Some(v) = self.collab_view()
            && v.project == Some(h.project)
        {
            for (site, seq) in v.sites {
                let e = h.file.sites.entry(site).or_insert(0);
                *e = (*e).max(seq);
            }
        }
        let _ = write_share(&mut self.store, h.project, &ShareFile::Host(h.file.clone()));
        h.file_dirty = false;
    }

    /// Quit or another project opened: sharing pauses (links stay valid; joiners see the
    /// host offline). Reopening the project resumes it.
    pub(crate) fn share_host_pause(&mut self, out: &mut dyn MessageSink) {
        let Some(mut h) = self.take_host() else {
            return;
        };
        self.share_host_close(&mut h, "the host went offline", out);
        if let Some(sites) = self.collab_left_sites(h.project) {
            for (site, seq) in sites {
                let e = h.file.sites.entry(site).or_insert(0);
                *e = (*e).max(seq);
            }
        }
        h.file.resume = true;
        let _ = write_share(&mut self.store, h.project, &ShareFile::Host(h.file.clone()));
    }

    /// Close the hub, the pairings, the signaling socket, and our collab session.
    fn share_host_close(&mut self, h: &mut HostRt, reason: &str, out: &mut dyn MessageSink) {
        h.hub.close_peers(reason);
        let peers: Vec<PeerId> = h
            .pairings
            .keys()
            .copied()
            .chain(h.live.values().copied())
            .collect();
        for p in peers {
            self.share_services().peers.close(p);
        }
        if let Some(mut s) = h.signal.take() {
            s.close();
        }
        if self
            .collab_view()
            .is_some_and(|v| v.server == share_server(&h.file.room))
        {
            self.collab_leave(out);
        }
    }

    /// `Stop`: revoke every link and member key, forget the room, end the session.
    pub(crate) fn share_stop(&mut self, out: &mut dyn MessageSink) -> CmdResult<()> {
        if !matches!(self.share.mode, Mode::Host(_)) {
            // A project shared before but paused: stop it too.
            if let Some(pid) = self.doc.as_ref().map(|d| d.project.id)
                && let Some(ShareFile::Host(_)) = read_share(&mut self.store, pid)
                && matches!(self.share.mode, Mode::Off)
            {
                super::clear_share_file(&mut self.store, pid);
                self.emit_list_changed(out);
                return Ok(());
            }
            return Err(invalid_state("this app is not sharing a project"));
        }
        let mut h = self.take_host().expect("checked");
        if let Some(s) = h.signal.as_mut()
            && s.state() == LinkState::Open
            && h.hello_sent
        {
            s.send(&SignalClientMessage::CloseRoom);
        }
        self.share_host_close(&mut h, "sharing stopped", out);
        super::clear_share_file(&mut self.store, h.project);
        self.emit_list_changed(out);
        Ok(())
    }

    pub(crate) fn share_reset_link(&mut self, role: ShareRole) -> CmdResult<()> {
        let key = keys::new_key().map_err(internal)?;
        let h = self.host_mut()?;
        match role {
            ShareRole::Edit => h.file.edit_key = Some(key.to_string()),
            ShareRole::Listen => h.file.listen_key = Some(key.to_string()),
        }
        h.doors_dirty = true;
        h.send_doors();
        let (pid, file) = (h.project, h.file.clone());
        write_share(&mut self.store, pid, &ShareFile::Host(file)).map_err(internal)
    }

    pub(crate) fn share_remove(&mut self, member: &str) -> CmdResult<()> {
        let h = self.host_mut()?;
        if !h.file.members.iter().any(|m| m.member == member) {
            return Err(invalid("no such member"));
        }
        h.file.members.retain(|m| m.member != member);
        h.hub.close_member(member, "removed by the host");
        if let Some(p) = h.live.remove(member) {
            h.end_peer(p, "removed");
        }
        h.doors_dirty = true;
        h.send_doors();
        let (pid, file) = (h.project, h.file.clone());
        write_share(&mut self.store, pid, &ShareFile::Host(file)).map_err(internal)
    }

    pub(crate) fn share_set_role(&mut self, member: &str, role: ShareRole) -> CmdResult<()> {
        let h = self.host_mut()?;
        let Some(m) = h.member_mut(member) else {
            return Err(invalid("no such member"));
        };
        if m.role == role {
            return Ok(());
        }
        m.role = role;
        // It rejoins at once with its member key and gets the new role in `Welcome`.
        h.hub.close_member(member, "role changed");
        let (pid, file) = (h.project, h.file.clone());
        write_share(&mut self.store, pid, &ShareFile::Host(file)).map_err(internal)
    }

    pub(crate) fn share_host_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        let Some(mut h) = self.take_host() else {
            return;
        };
        // Our project and our session (a relay session started from Settings replaces it).
        let open = self.doc.as_ref().map(|d| d.project.id);
        let ours = self
            .collab_view()
            .is_some_and(|v| v.server == share_server(&h.file.room));
        if open != Some(h.project) || !ours {
            self.share.mode = Mode::Host(h);
            self.share_host_pause(out);
            return;
        }
        self.share_host_signal(&mut h, now);
        h.hub.poll(now);
        for left in h.hub.take_left() {
            if let Some(m) = left.member.as_deref() {
                if let Some(p) = h.live.remove(m) {
                    self.share_services().peers.close(p);
                }
                if let Some(rec) = h.member_mut(m) {
                    rec.last_seen_ms = now as f64;
                    h.file_dirty = true;
                }
            }
            notice(out, ShareNotice::ParticipantLeft { name: left.name });
        }
        self.share_host_peers(&mut h, now, out);
        h.send_doors();
        // `sites` (and last-seen times) follow the session, throttled.
        if let Some(v) = self.collab_view()
            && v.sites.iter().any(|(s, q)| h.file.sites.get(s) != Some(q))
        {
            h.file_dirty = true;
        }
        if h.file_dirty && now >= h.saved_at + SAVE_EVERY_MS {
            h.saved_at = now;
            self.share_host_save(&mut h);
        }
        self.share.mode = Mode::Host(h);
    }

    fn share_host_signal(&mut self, h: &mut HostRt, now: u64) {
        if h.signal.is_none() && now >= h.retry_at {
            let url = signal_socket_url(h.file.signal_url.as_deref(), &h.file.room, "host");
            h.signal = Some((self.share_services().signal)(&url));
            h.hello_sent = false;
            h.status = SignalStatus::Connecting;
        }
        let Some(sig) = h.signal.as_mut() else {
            return;
        };
        let mut messages = Vec::new();
        sig.poll(&mut messages);
        match sig.state() {
            LinkState::Open => {
                if !h.hello_sent {
                    let hello = SignalClientMessage::HostHello {
                        protocol: SIGNAL_PROTOCOL_VERSION,
                        host_token: h.file.host_token.clone(),
                        doors: doors(&h.file),
                        app: self.share_app(),
                    };
                    let sig = h.signal.as_mut().expect("checked");
                    sig.send(&hello);
                    h.hello_sent = true;
                    h.doors_dirty = false;
                    h.last_ping = now;
                } else if now >= h.last_ping + PING_EVERY_MS {
                    h.last_ping = now;
                    sig.send(&SignalClientMessage::Ping);
                }
            }
            LinkState::Connecting => {}
            LinkState::Closed { reason, fatal } => {
                h.signal = None;
                h.hello_sent = false;
                if !matches!(h.status, SignalStatus::Offline { .. }) {
                    h.status = SignalStatus::Offline { reason };
                }
                h.retry_at = now
                    + if fatal {
                        SIGNAL_RETRY_MAX_MS
                    } else {
                        h.backoff
                    };
                h.backoff = (h.backoff * 2).min(SIGNAL_RETRY_MAX_MS);
                // Introductions in progress cannot finish without the service.
                let waiting: Vec<PeerId> = h
                    .pairings
                    .iter()
                    .filter(|(_, p)| matches!(p.stage, Stage::Ice))
                    .map(|(id, _)| *id)
                    .collect();
                for p in waiting {
                    h.pairings.remove(&p);
                    self.share_services().peers.close(p);
                }
            }
        }
        for m in messages {
            match m {
                SignalServerMessage::HostWelcome { ice_servers, .. } => {
                    h.status = SignalStatus::Online;
                    h.backoff = SIGNAL_RETRY_MIN_MS;
                    h.ice = ice_servers;
                    let hub_ice = match self.collab_ice_override() {
                        Some(v) => v.to_vec(),
                        None => h.ice.clone(),
                    };
                    h.hub.set_ice_servers(hub_ice);
                }
                SignalServerMessage::PeerArrived { peer } => {
                    if h.pairings.len() >= MAX_PAIRINGS {
                        h.end_peer(peer, "busy");
                        continue;
                    }
                    let ice = peer_ice(
                        self.collab_ice_override(),
                        &h.ice,
                        self.share.prefs.relay_only,
                    );
                    let relay_only = self.share.prefs.relay_only;
                    self.share_services()
                        .peers
                        .open(peer, false, &ice, relay_only);
                    h.pairings.insert(
                        peer,
                        Pairing {
                            since: now,
                            stage: Stage::Ice,
                        },
                    );
                }
                SignalServerMessage::Signal { peer, signal } => {
                    if h.pairings.contains_key(&peer) {
                        self.share_services().peers.signal(peer, signal);
                    }
                }
                SignalServerMessage::PeerLeft { peer } => {
                    // Joiners close their socket once the data channel is up.
                    if h.pairings
                        .get(&peer)
                        .is_some_and(|p| matches!(p.stage, Stage::Ice))
                    {
                        h.pairings.remove(&peer);
                        self.share_services().peers.close(peer);
                    }
                }
                SignalServerMessage::Refused { message, .. } => {
                    h.status = SignalStatus::Offline { reason: message };
                }
                SignalServerMessage::JoinWelcome { .. }
                | SignalServerMessage::HostOffline { .. }
                | SignalServerMessage::Pong => {}
            }
        }
    }

    fn share_host_peers(&mut self, h: &mut HostRt, now: u64, out: &mut dyn MessageSink) {
        let mut outputs = Vec::new();
        self.share_services().peers.poll(&mut outputs);
        for o in outputs {
            match o {
                PeerOutput::Signal { peer, signal } => {
                    if h.pairings.contains_key(&peer)
                        && let Some(s) = h.signal.as_mut()
                    {
                        s.send(&SignalClientMessage::Signal { peer, signal });
                    }
                }
                PeerOutput::Connected {
                    peer,
                    mut link,
                    local_fingerprint,
                    remote_fingerprint,
                } => match h.pairings.get_mut(&peer) {
                    Some(p) if matches!(p.stage, Stage::Ice) => {
                        p.since = now;
                        p.stage = Stage::Hello(Conn {
                            link,
                            local: local_fingerprint,
                            remote: remote_fingerprint,
                        });
                    }
                    _ => link.close(),
                },
                PeerOutput::Failed { peer, reason } => {
                    if h.pairings.remove(&peer).is_some() {
                        h.end_peer(peer, &reason);
                    }
                }
            }
        }
        let ids: Vec<PeerId> = h.pairings.keys().copied().collect();
        for peer in ids {
            self.share_host_pairing(h, peer, now, out);
        }
    }

    fn share_host_pairing(
        &mut self,
        h: &mut HostRt,
        peer: PeerId,
        now: u64,
        out: &mut dyn MessageSink,
    ) {
        let Some(mut p) = h.pairings.remove(&peer) else {
            return;
        };
        let end = |this: &mut Self, h: &mut HostRt, conn: Option<Conn>, reason: &str| {
            if let Some(mut c) = conn {
                c.link.close();
            }
            this.share_services().peers.close(peer);
            h.end_peer(peer, reason);
        };
        match p.stage {
            Stage::Ice => {
                if now >= p.since + ICE_TIMEOUT_MS {
                    end(self, h, None, "timed out");
                    return;
                }
                h.pairings.insert(peer, p);
            }
            Stage::Hello(mut c) => {
                let mut frames = Vec::new();
                c.link.poll(&mut frames);
                let Some(frame) = frames.first() else {
                    if now >= p.since + HELLO_TIMEOUT_MS
                        || matches!(c.link.state(), LinkState::Closed { .. })
                    {
                        end(self, h, Some(c), "no hello");
                    } else {
                        p.stage = Stage::Hello(c);
                        h.pairings.insert(peer, p);
                    }
                    return;
                };
                let hello = match handshake::decode(frame) {
                    Ok(hello @ PeerHandshake::Hello { .. }) => hello,
                    _ => return end(self, h, Some(c), "bad hello"),
                };
                match self.share_host_admit(h, &hello, &c) {
                    Ok((welcome, pending)) => {
                        c.link.send(&handshake::encode(&welcome));
                        p.since = now;
                        p.stage = Stage::Accept(c, pending);
                        h.pairings.insert(peer, p);
                    }
                    Err(refusal) => {
                        c.link.send(&handshake::encode(&refusal.frame()));
                        end(self, h, Some(c), &refusal.message);
                    }
                }
            }
            Stage::Accept(mut c, pending) => {
                let mut frames = Vec::new();
                c.link.poll(&mut frames);
                let Some(frame) = frames.first() else {
                    if now >= p.since + ACCEPT_TIMEOUT_MS
                        || matches!(c.link.state(), LinkState::Closed { .. })
                    {
                        // Declined or gone: no trace (members are recorded on Accept).
                        end(self, h, Some(c), "not accepted");
                    } else {
                        p.stage = Stage::Accept(c, pending);
                        h.pairings.insert(peer, p);
                    }
                    return;
                };
                if handshake::decode(frame) != Ok(PeerHandshake::Accept) {
                    return end(self, h, Some(c), "bad accept");
                }
                // What follows `Accept` is collab traffic (the joiner's Hello, SyncRequest).
                frames.remove(0);
                self.share_host_admitted(h, peer, c, pending, frames, now, out);
            }
        }
    }

    fn share_host_admit(
        &mut self,
        h: &HostRt,
        hello: &PeerHandshake,
        c: &Conn,
    ) -> Result<(PeerHandshake, Pending), Refusal> {
        let edit = key_of(&h.file.edit_key);
        let listen = key_of(&h.file.listen_key);
        let member_keys: Vec<(String, LinkKey, ShareRole)> = h
            .file
            .members
            .iter()
            .filter_map(|m| Some((m.member.clone(), keys::parse_key(&m.key)?, m.role)))
            .collect();
        let hk = HostKeys {
            edit: edit.as_ref(),
            listen: listen.as_ref(),
            members: member_keys
                .iter()
                .map(|(m, k, r)| (m.as_str(), k, *r))
                .collect(),
        };
        // `c.remote` is the joiner's fingerprint, `c.local` ours.
        let admitted = handshake::admit(hello, &hk, &c.remote, &c.local)?;
        let refused = |message: &str| Refusal {
            reason: JoinFailure::Refused,
            message: message.into(),
        };
        let conns = h.hub.conns();
        if conns.len() >= MAX_PARTICIPANTS {
            return Err(refused("the session is full"));
        }
        let PeerHandshake::Hello { name, color, .. } = hello else {
            unreachable!("admitted a hello")
        };
        let (member, member_key) = match &admitted {
            Admitted::Link { .. } => {
                if h.file.members.len() >= MAX_MEMBERS {
                    return Err(refused("this project has too many members"));
                }
                let id = keys::new_id().map_err(|e| refused(&e))?;
                let key = keys::new_key().map_err(|e| refused(&e))?;
                (id, Some(key))
            }
            Admitted::Member { member, .. } => (member.clone(), None),
        };
        let host_color = conns
            .iter()
            .find(|c| c.role == ParticipantRole::Host)
            .map_or(Color(0), |c| c.color);
        let welcome = PeerHandshake::Welcome {
            host_proof: keys::host_proof(admitted.key(), &c.local, &c.remote),
            role: admitted.role(),
            member: member.clone(),
            member_key: member_key.as_ref().map(ToString::to_string),
            host: ParticipantSummary {
                name: self.share_name(),
                color: host_color,
            },
            project: h.project,
            project_name: self
                .doc
                .as_ref()
                .map(|d| d.project.settings.name.clone())
                .unwrap_or_default(),
            online: conns
                .iter()
                .filter(|c| c.role != ParticipantRole::Host)
                .map(|c| ParticipantSummary {
                    name: c.name.clone(),
                    color: c.color,
                })
                .collect(),
        };
        let pending = Pending {
            admitted,
            member,
            member_key,
            name: clean_name(name),
            color: *color,
        };
        Ok((welcome, pending))
    }

    #[allow(clippy::too_many_arguments)]
    fn share_host_admitted(
        &mut self,
        h: &mut HostRt,
        peer: PeerId,
        c: Conn,
        pending: Pending,
        rest: Vec<ether_collab::wire::WireFrame>,
        now: u64,
        out: &mut dyn MessageSink,
    ) {
        let role = pending.admitted.role();
        // A returning member keeps its colour (when free).
        let preferred = h
            .file
            .members
            .iter()
            .find(|m| m.member == pending.member)
            .map(|m| m.color)
            .or(pending.color);
        let conn = match h.hub.add_peer(
            c.link,
            role,
            pending.member.clone(),
            &pending.name,
            preferred,
        ) {
            Ok(conn) => conn,
            Err((mut link, reason)) => {
                link.send(&handshake::encode(&PeerHandshake::Refused {
                    reason: JoinFailure::Refused,
                    message: reason.clone(),
                }));
                link.close();
                self.share_services().peers.close(peer);
                h.end_peer(peer, &reason);
                return;
            }
        };
        h.hub.deliver(conn, rest);
        let color = h
            .hub
            .conns()
            .into_iter()
            .find(|x| x.conn == conn)
            .map_or(Color(0), |x| x.color);
        match pending.member_key {
            Some(key) => {
                h.file.members.push(MemberRecord {
                    member: pending.member.clone(),
                    key: key.to_string(),
                    role,
                    name: pending.name.clone(),
                    color,
                    last_seen_ms: now as f64,
                });
                h.doors_dirty = true;
            }
            None => {
                if let Some(m) = h.member_mut(&pending.member) {
                    m.name = pending.name.clone();
                    m.color = color;
                    m.last_seen_ms = now as f64;
                }
            }
        }
        if let Some(old) = h.live.insert(pending.member, peer) {
            self.share_services().peers.close(old);
        }
        self.share_host_save(h);
        h.saved_at = now;
        notice(
            out,
            ShareNotice::ParticipantJoined {
                name: pending.name,
                color,
            },
        );
    }

    pub(crate) fn share_host_state(&self, h: &HostRt) -> ShareState {
        let online = matches!(h.status, SignalStatus::Online);
        let origin = self
            .share
            .invite_origin
            .as_deref()
            .unwrap_or(DEFAULT_INVITE_ORIGIN);
        let link = |k: &Option<String>| {
            let key = key_of(k)?;
            online.then(|| {
                Invite {
                    room: h.file.room.clone(),
                    key: Some(key),
                    signal_url: h.file.signal_url.clone(),
                }
                .web_url(origin)
            })
        };
        let conns = h.hub.conns();
        let me = conns.iter().find(|c| c.role == ParticipantRole::Host);
        let mut participants = vec![Participant {
            member: None,
            site: me.and_then(|c| c.site),
            name: self.share_name(),
            color: me.map_or(self.share.color.unwrap_or(Color(0)), |c| c.color),
            role: ParticipantRole::Host,
            online: true,
            you: true,
            last_seen_ms: None,
        }];
        for m in &h.file.members {
            let conn = conns
                .iter()
                .find(|c| c.member.as_deref() == Some(m.member.as_str()));
            participants.push(Participant {
                member: Some(m.member.clone()),
                site: conn.and_then(|c| c.site),
                name: conn.map_or(m.name.clone(), |c| c.name.clone()),
                color: conn.map_or(m.color, |c| c.color),
                role: match m.role {
                    ShareRole::Edit => ParticipantRole::Edit,
                    ShareRole::Listen => ParticipantRole::Listen,
                },
                online: conn.is_some(),
                you: false,
                last_seen_ms: conn.is_none().then_some(m.last_seen_ms),
            });
        }
        ShareState::Hosting {
            project: h.project,
            edit_link: link(&h.file.edit_key),
            listen_link: link(&h.file.listen_key),
            participants,
            signal: h.status.clone(),
        }
    }
}

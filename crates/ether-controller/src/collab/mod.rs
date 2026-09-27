//! Real-time collaboration: this controller as one **site** of a session (design:
//! `docs/COLLAB.md`; relay and transports: `ether-collab`).
//!
//! - **Join/Leave/SetPresence/Get** (`CollabCommand`) and `CollabEvent::{Session, Presence}`.
//! - **Local edits** (`edit_with`, undo/redo) commit as usual, then [`local_commit`] stamps
//!   their shared ops (`OpOrigin { site, seq }`), keeps them **pending** with their inverse,
//!   and sends them (media files they insert first).
//! - **Sequenced transactions** from the relay: an own echo pops the pending head; a peer's
//!   transaction is applied by undoing the pending ones, applying it with
//!   [`resolve::resolve_all`], and re-applying the pending ones (one patch with `origin`).
//! - **Per-site undo**: `History::undo_with/redo_with` with [`resolve::apply_guarded`].
//! - **Join**: the first site sends its media and a snapshot; later ones adopt the session
//!   project (a differing local copy is kept as a backup project first).
//! - **Reconnect**: backoff, `SyncRequest` with the confirmed index, pending resent.
//!
//! [`local_commit`]: EtherController::collab_local_commit

// base-53 (docs/COLLAB.md §8-§9): one module per node, dispatched from here.
mod listen;
mod presence;
pub(crate) mod resolve;
mod stream_host;

use std::collections::{BTreeMap, VecDeque};

use ether_collab::wire::{
    COLLAB_PROTOCOL_VERSION, SnapshotData, decode_version, encode_version, media,
    valid_session_name,
};
use ether_collab::{BoxTransport, ConnectRequest, Connector, LinkState};
use ether_core::protocol::collab::{
    CollabCommand, CollabEvent, CollabMessage, CollabStatus, IceServer, IceServerSource, Presence,
    PresenceState, StreamSignal,
};
use ether_core::protocol::model::file;
use ether_core::protocol::model::*;
use ether_core::protocol::{Event, NotificationLevel, ReplyValue};

use crate::handlers::{event, no_project, notify, store_err};
use crate::store::{Library, ProjectStore, check_relative_path};
use crate::tx::{CmdResult, internal, invalid, invalid_state};
use crate::{EngineBridge, EtherController, HostServices, MessageSink, content_hash};

/// Presence updates are sent at most this often.
const PRESENCE_INTERVAL_MS: u64 = 100;
const RECONNECT_MIN_MS: u64 = 500;
const RECONNECT_MAX_MS: u64 = 10_000;
/// Largest media file accepted from a peer.
const MAX_MEDIA_FILE: u64 = 2 << 30;

/// A local transaction sent (or to send) and not yet sequenced.
struct Pending {
    tx: StampedTransaction,
    /// Reverts its ops as currently applied to the live document (in order).
    inverse: Vec<Op>,
}

/// See [`Session::backup`].
struct Backup {
    /// The stored `.ether` file.
    json: String,
    /// Its shared part ([`resolve::shared_part`]; `None`: unreadable, always kept).
    shared: Option<Project>,
}

/// A media file being received.
struct Incoming {
    hash: String,
    total: u64,
    received: u64,
    /// Upload staging id in the store (`None`: buffered in `buf`).
    staged: Option<String>,
    buf: Vec<u8>,
}

struct Session {
    request: ConnectRequest,
    name: String,
    link: Option<BoxTransport>,
    /// `Hello` + `SyncRequest` sent on the current link.
    greeted: bool,
    /// Our `SyncRequest` is not answered yet (a `SyncRequest` back means "create it").
    awaiting_sync: bool,
    /// The session project was adopted or created.
    joined: bool,
    project: Option<ProjectId>,
    /// Transactions applied from the log (the confirmed index).
    index: u64,
    /// The session incarnation we are synced with (`SnapshotData::epoch`).
    epoch: u64,
    /// Last sequenced `seq` per site.
    sites: BTreeMap<SiteId, u64>,
    /// Last local `seq` assigned.
    seq: u64,
    pending: VecDeque<Pending>,
    /// The relay asked for a compaction snapshot.
    owe_snapshot: bool,
    peers: BTreeMap<SiteId, Presence>,
    names: BTreeMap<SiteId, String>,
    presence: PresenceState,
    presence_dirty: bool,
    last_presence_ms: u64,
    reconnect_at: u64,
    backoff_ms: u64,
    incoming: BTreeMap<String, Incoming>,
    /// Media received before the session project exists (file, bytes).
    staged: Vec<(String, Vec<u8>)>,
    upload_counter: u64,
    status: Option<CollabStatus>,
    /// Rejoin: the stored copy of the project differed from the snapshot. It is kept as a
    /// backup project unless it turns out to be a state of the session log (no offline
    /// work): compared after every sequenced transaction, decided at the first quiet tick.
    backup: Option<Backup>,
    /// Last plugin state this site sent, received, or started from, per device: a save
    /// replicates a plugin's state only when its live state differs (COLLAB.md §2.2).
    captured: BTreeMap<DeviceId, Base64Bytes>,
    /// This site's mix (mute/solo) as of its copy of the project before it joined: the
    /// tracks the log (re-)inserts get it back (COLLAB.md §2.1).
    mix_seed: resolve::LocalMix,
    /// ICE servers the relay advertised (`CollabMessage::IceServers`).
    ice_relay: Vec<IceServer>,
    /// base-53 node state: live pointer (`presence-v2`), listener (`stream-listen`), host
    /// (`stream-host`). Unused until those nodes land.
    #[allow(dead_code)]
    pointer: presence::PointerState,
    #[allow(dead_code)]
    listener: listen::ListenerState,
    #[allow(dead_code)]
    host: stream_host::HostState,
}

impl Session {
    fn open(&self) -> bool {
        self.greeted
            && self
                .link
                .as_ref()
                .is_some_and(|l| l.state() == LinkState::Open)
    }

    fn status(&self, site: SiteId) -> CollabStatus {
        let session = self.request.session.clone();
        if self.joined && self.open() {
            CollabStatus::Online { session, site }
        } else {
            CollabStatus::Connecting { session }
        }
    }

    fn send(&mut self, m: &CollabMessage) {
        if let Some(l) = self.link.as_mut() {
            l.send(m);
        }
    }
}

/// Collaboration state of the controller (one field on `EtherController`).
#[derive(Default)]
pub(crate) struct CollabState {
    connector: Option<Connector>,
    site: Option<SiteId>,
    session: Option<Box<Session>>,
    /// Sessions this controller created (for distinct epochs).
    created: u64,
    /// Last `seq` this site used, across sessions: a re-join of the same session (same
    /// site id) must never reuse a `seq` the relay already sequenced.
    last_seq: u64,
    /// The sequenced `seq` per site that the project's document includes, as of the last
    /// session left with it: a site that later re-creates the session from this document
    /// seeds its snapshot with it (returning sites must not resend what it already has).
    left_sites: Option<(ProjectId, BTreeMap<SiteId, u64>)>,
    /// ICE servers from the settings (`CollabCommand::SetIceServers`), over the relay's.
    ice_override: Option<Vec<IceServer>>,
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// Open collaboration links with `connector` instead of real WebSockets (tests,
    /// simulations: `ether_collab::memory::Hub::connector`).
    pub fn set_collab_connector(&mut self, connector: Connector) {
        self.collab.connector = Some(connector);
    }

    /// This controller's site id (stable for its lifetime).
    pub fn collab_site(&mut self) -> SiteId {
        if let Some(s) = self.collab.site {
            return s;
        }
        let mut id = self.host.random_seed();
        if id == 0 {
            id = 1;
        }
        let site = SiteId(id);
        self.collab.site = Some(site);
        site
    }

    /// Transactions sent and not sequenced yet (0 outside a session).
    pub fn collab_pending(&self) -> usize {
        self.collab.session.as_ref().map_or(0, |s| s.pending.len())
    }

    /// In a session whose project is open: edits are stamped and sent.
    pub(crate) fn collab_active(&self) -> bool {
        let open = self.doc.as_ref().map(|d| d.project.id);
        self.collab
            .session
            .as_ref()
            .is_some_and(|s| s.joined && s.project.is_some() && s.project == open)
    }

    pub(crate) fn collab_command(
        &mut self,
        c: &CollabCommand,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let now = self.host.now_ms();
        match c {
            CollabCommand::Join {
                server,
                session,
                token,
                name,
            } => {
                if !valid_session_name(session) {
                    return Err(invalid(
                        "session names are 1-64 letters, digits, '.', '_' or '-'",
                    ));
                }
                if !(server.starts_with("ws://") || server.starts_with("wss://")) {
                    return Err(invalid("the relay URL must start with ws:// or wss://"));
                }
                if self.collab.session.is_some() {
                    self.collab_leave(out);
                }
                let site = self.collab_site();
                let request = ConnectRequest {
                    server: server.trim().to_string(),
                    session: session.clone(),
                    token: token.clone().filter(|t| !t.is_empty()),
                    client: format!("Ethereal {} ({name})", self.config.app_version),
                };
                let name: String = name.trim().chars().take(64).collect();
                self.collab.session = Some(Box::new(Session {
                    request,
                    name,
                    link: None,
                    greeted: false,
                    awaiting_sync: false,
                    joined: false,
                    project: None,
                    index: 0,
                    epoch: 0,
                    sites: BTreeMap::new(),
                    seq: self.collab.last_seq,
                    pending: VecDeque::new(),
                    owe_snapshot: false,
                    peers: BTreeMap::new(),
                    names: BTreeMap::new(),
                    presence: PresenceState::default(),
                    presence_dirty: true,
                    last_presence_ms: 0,
                    reconnect_at: now,
                    backoff_ms: RECONNECT_MIN_MS,
                    incoming: BTreeMap::new(),
                    staged: Vec::new(),
                    upload_counter: 0,
                    status: None,
                    captured: BTreeMap::new(),
                    mix_seed: Default::default(),
                    backup: None,
                    ice_relay: Vec::new(),
                    pointer: Default::default(),
                    listener: Default::default(),
                    host: Default::default(),
                }));
                let _ = site;
                self.collab_connect(now);
                self.collab_emit_status(out);
                self.collab_emit_peers(out);
                Ok(ReplyValue::Unit)
            }
            CollabCommand::Leave => {
                self.collab_leave(out);
                Ok(ReplyValue::Unit)
            }
            CollabCommand::SetPresence { presence } => {
                if let Some(s) = self.collab.session.as_mut() {
                    if s.presence != *presence {
                        s.presence = presence.clone();
                        s.presence_dirty = true;
                    }
                    self.collab_flush_presence(now);
                }
                Ok(ReplyValue::Unit)
            }
            CollabCommand::Get => {
                self.collab_emit_status_forced(out);
                self.collab_emit_peers(out);
                self.collab_listen_emit_status(out);
                self.collab_emit_ice_servers(out);
                Ok(ReplyValue::Unit)
            }
            // ─── base-53 ───
            CollabCommand::SetPointer { pointer } => self.collab_set_pointer(pointer, now),
            CollabCommand::Listen { .. } | CollabCommand::StopListening => {
                self.collab_listen_command(c, now, out)
            }
            CollabCommand::SetHosting { .. } | CollabCommand::SendStreamClock { .. } => {
                self.collab_host_command(c, now, out)
            }
            CollabCommand::SendSignal { to, stream, signal } => {
                let me = self.collab_site();
                if *to == me {
                    return Err(invalid("cannot signal this site"));
                }
                if !self.collab.session.as_ref().is_some_and(|s| s.joined) {
                    return Err(invalid_state("not in a collaboration session"));
                }
                self.collab_send_routed_signal(*to, *stream, signal.clone());
                if self.collab_hosts_stream(*to, *stream) {
                    self.collab_host_outgoing_signal(*to, *stream, signal, out);
                } else {
                    self.collab_listen_outgoing_signal(*to, *stream, signal, out);
                }
                Ok(ReplyValue::Unit)
            }
            CollabCommand::SetIceServers { servers } => {
                self.collab.ice_override = servers.clone();
                self.collab_emit_ice_servers(out);
                Ok(ReplyValue::Unit)
            }
        }
    }

    /// Send a `Signal` from this site to `to` (the relay delivers it to `to` only).
    pub(crate) fn collab_send_routed_signal(
        &mut self,
        to: SiteId,
        stream: u32,
        signal: StreamSignal,
    ) {
        let from = self.collab_site();
        if let Some(s) = self.collab.session.as_mut() {
            s.send(&CollabMessage::Signal {
                from,
                to,
                stream,
                signal,
            });
        }
    }

    /// The ICE servers to use: the settings override, else what the relay advertised.
    pub(crate) fn collab_ice_servers(&self) -> (Vec<IceServer>, IceServerSource) {
        match &self.collab.ice_override {
            Some(v) => (v.clone(), IceServerSource::Settings),
            None => (
                self.collab
                    .session
                    .as_ref()
                    .map(|s| s.ice_relay.clone())
                    .unwrap_or_default(),
                IceServerSource::Relay,
            ),
        }
    }

    fn collab_emit_ice_servers(&mut self, out: &mut dyn MessageSink) {
        let (servers, source) = self.collab_ice_servers();
        event(
            out,
            Event::Collab {
                event: CollabEvent::IceServers { servers, source },
            },
        );
    }

    // ─── Link ───────────────────────────────────────────────────────────────────────────

    fn collab_connect(&mut self, now: u64) {
        let Some(s) = self.collab.session.as_mut() else {
            return;
        };
        let connector = self
            .collab
            .connector
            .get_or_insert_with(ether_collab::default_connector);
        s.link = Some(connector(&s.request));
        s.greeted = false;
        s.reconnect_at = now;
    }

    /// Leave the session (the document stays open as a normal local project).
    pub(crate) fn collab_leave(&mut self, out: &mut dyn MessageSink) {
        if self.collab.session.is_some() {
            self.collab_listen_session_end(out);
            self.collab_host_session_end(out);
        }
        let Some(mut s) = self.collab.session.take() else {
            return;
        };
        self.collab.last_seq = self.collab.last_seq.max(s.seq);
        if s.joined
            && let Some(pid) = s.project
        {
            let mut sites = s.sites.clone();
            if let Some(site) = self.collab.site {
                // Pending edits stay in the document (they may be re-sent by a re-join,
                // with a fresh seq, never with an old one).
                sites.insert(site, s.seq.max(sites.get(&site).copied().unwrap_or(0)));
            }
            self.collab.left_sites = Some((pid, sites));
        }
        if let Some(site) = self.collab.site {
            s.send(&CollabMessage::Leave { site });
        }
        if let Some(mut l) = s.link.take() {
            l.close();
        }
        for inc in s.incoming.values() {
            if let Some(id) = &inc.staged {
                let _ = self.store.discard_upload(id);
            }
        }
        event(
            out,
            Event::Collab {
                event: CollabEvent::Session {
                    status: CollabStatus::Offline,
                },
            },
        );
        event(
            out,
            Event::Collab {
                event: CollabEvent::Presence { peers: Vec::new() },
            },
        );
    }

    fn collab_emit_status(&mut self, out: &mut dyn MessageSink) {
        let site = self.collab_site();
        let Some(s) = self.collab.session.as_mut() else {
            return;
        };
        let status = s.status(site);
        if s.status.as_ref() != Some(&status) {
            s.status = Some(status.clone());
            event(
                out,
                Event::Collab {
                    event: CollabEvent::Session { status },
                },
            );
        }
    }

    fn collab_emit_status_forced(&mut self, out: &mut dyn MessageSink) {
        let site = self.collab_site();
        let status = match self.collab.session.as_mut() {
            Some(s) => {
                let st = s.status(site);
                s.status = Some(st.clone());
                st
            }
            None => CollabStatus::Offline,
        };
        event(
            out,
            Event::Collab {
                event: CollabEvent::Session { status },
            },
        );
    }

    fn collab_emit_peers(&mut self, out: &mut dyn MessageSink) {
        let peers = self
            .collab
            .session
            .as_ref()
            .map(|s| s.peers.values().cloned().collect())
            .unwrap_or_default();
        event(
            out,
            Event::Collab {
                event: CollabEvent::Presence { peers },
            },
        );
    }

    fn collab_flush_presence(&mut self, now: u64) {
        let Some(site) = self.collab.site else { return };
        // Controller-owned fields (base-53): whatever the UI put there is overwritten.
        let mut state = match self.collab.session.as_ref() {
            Some(s) => s.presence.clone(),
            None => return,
        };
        self.collab_listen_presence(&mut state);
        self.collab_host_presence(&mut state);
        let Some(s) = self.collab.session.as_mut() else {
            return;
        };
        if !s.presence_dirty
            || !s.open()
            || !s.joined
            || now.saturating_sub(s.last_presence_ms) < PRESENCE_INTERVAL_MS
        {
            return;
        }
        s.presence_dirty = false;
        s.last_presence_ms = now;
        let presence = Presence {
            site,
            actor: None,
            name: s.name.clone(),
            color: Color(0),
            state,
        };
        s.send(&CollabMessage::Presence { presence });
    }

    /// Controller tick: link upkeep, incoming messages, presence, owed snapshots.
    pub(crate) fn collab_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        if self.collab.session.is_none() {
            return;
        }
        let site = self.collab_site();
        // Opening another project leaves the session.
        let open = self.doc.as_ref().map(|d| d.project.id);
        if let Some(s) = self.collab.session.as_ref()
            && s.joined
            && s.project != open
        {
            self.collab_leave(out);
            notify(
                out,
                NotificationLevel::Info,
                "left the collaboration session (another project was opened)",
            );
            return;
        }
        let state = self
            .collab
            .session
            .as_ref()
            .and_then(|s| s.link.as_ref().map(|l| l.state()));
        match state {
            None => {
                if self
                    .collab
                    .session
                    .as_ref()
                    .is_some_and(|s| now >= s.reconnect_at)
                {
                    self.collab_connect(now);
                }
            }
            Some(LinkState::Connecting) => {}
            Some(LinkState::Open) => self.collab_greet(site),
            Some(LinkState::Closed { reason, fatal }) => {
                if fatal {
                    self.collab_leave(out);
                    notify(
                        out,
                        NotificationLevel::Error,
                        format!("collaboration: {reason}"),
                    );
                    return;
                }
                // A host's relay reconnect is a leave: its streams end (COLLAB.md §9.2).
                self.collab_host_end_all("the host lost its relay connection", out);
                let s = self.collab.session.as_mut().expect("checked");
                s.link = None;
                s.greeted = false;
                s.reconnect_at = now + s.backoff_ms;
                s.backoff_ms = (s.backoff_ms * 2).min(RECONNECT_MAX_MS);
            }
        }
        // Incoming.
        let mut messages = Vec::new();
        if let Some(l) = self.collab.session.as_mut().and_then(|s| s.link.as_mut()) {
            l.poll(&mut messages);
        }
        let quiet = messages.is_empty();
        for m in messages {
            if self.collab.session.is_none() {
                break;
            }
            self.collab_message(m, now, out);
        }
        if self.collab.session.is_none() {
            return;
        }
        if quiet {
            self.collab_flush_backup(now, out);
        }
        self.collab_flush_presence(now);
        self.collab_pointer_tick(now);
        self.collab_listen_tick(now, out);
        self.collab_host_tick(now, out);
        if self.collab.session.is_none() {
            return;
        }
        self.collab_send_owed_snapshot();
        self.collab_emit_status(out);
    }

    /// First thing on a new link: `Hello`, `SyncRequest`, then the pending transactions.
    fn collab_greet(&mut self, site: SiteId) {
        let Some(s) = self.collab.session.as_mut() else {
            return;
        };
        if s.greeted {
            return;
        }
        s.greeted = true;
        s.awaiting_sync = true;
        s.backoff_ms = RECONNECT_MIN_MS;
        s.presence_dirty = true;
        let hello = CollabMessage::Hello {
            site,
            actor: None,
            name: s.name.clone(),
            protocol_version: COLLAB_PROTOCOL_VERSION,
        };
        s.send(&hello);
        let version = if s.joined {
            encode_version(s.epoch, s.index)
        } else {
            Base64Bytes(Vec::new())
        };
        s.send(&CollabMessage::SyncRequest { site, version });
        self.collab_resend_pending(0);
    }

    /// (Re)send pending transactions with `seq > after` (the relay drops duplicates).
    fn collab_resend_pending(&mut self, after: u64) {
        let Some(s) = self.collab.session.as_ref() else {
            return;
        };
        if !s.joined {
            return;
        }
        let txs: Vec<StampedTransaction> = s
            .pending
            .iter()
            .filter(|p| p.tx.origin.seq > after)
            .map(|p| p.tx.clone())
            .collect();
        for t in txs {
            self.collab_send_tx(t);
        }
    }

    /// Send a transaction, preceded by the files of the media it inserts.
    fn collab_send_tx(&mut self, t: StampedTransaction) {
        let Some(pid) = self.collab.session.as_ref().and_then(|s| s.project) else {
            return;
        };
        for op in &t.transaction.ops {
            if let Op::Insert {
                entity: Entity::Media(m),
            } = op
            {
                self.collab_push_media(pid, m);
            }
        }
        if let Some(s) = self.collab.session.as_mut() {
            s.send(&CollabMessage::Transaction { transaction: t });
        }
    }

    fn collab_push_media(&mut self, pid: ProjectId, m: &MediaRef) {
        // `file-import`: an external reference pushes the referenced file's bytes.
        let Ok(bytes) = crate::file_import::media_bytes(&mut self.store, &mut self.library, pid, m)
        else {
            return;
        };
        let hash = m.hash.clone().unwrap_or_else(|| content_hash(&bytes));
        if let Some(s) = self.collab.session.as_mut() {
            for chunk in media::chunks(&m.file, &hash, &bytes) {
                s.send(&chunk);
            }
        }
    }

    // ─── Local edits ────────────────────────────────────────────────────────────────────

    /// A local transaction was committed (`ops` as applied, `inverse` reverting them): stamp
    /// and send its shared ops. No-op outside a session.
    pub(crate) fn collab_local_commit(&mut self, label: &str, ops: &[Op], inverse: &[Op]) {
        if !self.collab_active() {
            return;
        }
        let (ops, inverse) = resolve::split_shared(ops, inverse);
        if ops.is_empty() {
            return;
        }
        if let Some(s) = self.collab.session.as_mut()
            && let Some(b) = s.backup.take()
        {
            // Decided now: the document is about to diverge from any log state.
            let now = self.host.now_ms();
            let pid = s.project;
            if let Some(pid) = pid {
                let mut sink = Vec::new();
                let _ = self.collab_backup(pid, &b.json, now, &mut sink);
            }
        }
        let site = self.collab_site();
        let s = self.collab.session.as_mut().expect("active");
        s.seq += 1;
        let tx = StampedTransaction {
            origin: OpOrigin {
                site,
                actor: None,
                seq: s.seq,
            },
            transaction: Transaction {
                label: label.to_string(),
                ops,
            },
        };
        s.pending.push_back(Pending {
            tx: tx.clone(),
            inverse,
        });
        if s.open() {
            self.collab_send_tx(tx);
        }
    }

    /// Undo/redo in a session: only what this site did, skipping fields changed since
    /// (`resolve::apply_guarded`). Returns the applied ops (after stamping them).
    pub(crate) fn collab_undo_redo(&mut self, undo: bool) -> CmdResult<Option<Vec<Op>>> {
        let doc = self.doc.as_mut().ok_or_else(no_project)?;
        let mut inverse = Vec::new();
        let f = |p: &mut Project, ops: &[Op], guards: &[Op]| {
            let (applied, inv) = resolve::apply_guarded(p, ops, guards);
            inverse = inv.clone();
            Ok((applied, inv))
        };
        let applied = if undo {
            doc.history.undo_with(&mut doc.project, f)
        } else {
            doc.history.redo_with(&mut doc.project, f)
        }
        .map_err(crate::tx::model_err)?;
        if let Some(applied) = &applied {
            let label = if undo { "Undo" } else { "Redo" };
            self.collab_local_commit(label, applied, &inverse);
        }
        Ok(applied)
    }

    // ─── Incoming ───────────────────────────────────────────────────────────────────────

    fn collab_message(&mut self, m: CollabMessage, now: u64, out: &mut dyn MessageSink) {
        let site = self.collab_site();
        match m {
            CollabMessage::SyncRequest { site: to, version } => {
                if to != site || !matches!(decode_version(&version), Ok(None)) {
                    return;
                }
                let s = self.collab.session.as_mut().expect("in session");
                if s.awaiting_sync || !s.joined {
                    // The session is empty on the relay: we create it.
                    s.awaiting_sync = false;
                    if let Err(e) = self.collab_create(site) {
                        self.collab_leave(out);
                        notify(
                            out,
                            NotificationLevel::Error,
                            format!("collaboration: {}", e.message),
                        );
                    }
                } else {
                    s.owe_snapshot = true;
                }
            }
            CollabMessage::Snapshot { data } => {
                if let Some(s) = self.collab.session.as_mut() {
                    s.awaiting_sync = false;
                }
                if let Err(e) = self.collab_adopt(&data, now, out) {
                    self.collab_leave(out);
                    notify(
                        out,
                        NotificationLevel::Error,
                        format!("collaboration: could not join: {}", e.message),
                    );
                }
            }
            CollabMessage::Transaction { transaction } => {
                let s = self.collab.session.as_mut().expect("in session");
                s.awaiting_sync = false;
                if !s.joined {
                    return;
                }
                s.index += 1;
                let origin = transaction.origin.clone();
                let last = s.sites.entry(origin.site).or_insert(0);
                *last = (*last).max(origin.seq);
                if origin.site == site {
                    // Never reuse a sequenced `seq` (a re-join replays our older ones).
                    s.seq = s.seq.max(origin.seq);
                    if s.pending
                        .front()
                        .is_some_and(|p| p.tx.origin.seq == origin.seq)
                    {
                        // Our own echo: the head of `pending` (see docs/COLLAB.md §2).
                        #[cfg(debug_assertions)]
                        if let Some(doc) = self.doc.as_ref() {
                            check_echo(&doc.project, &s.pending, &transaction);
                        }
                        s.pending.pop_front();
                        self.collab_check_backup();
                        return;
                    }
                    // Ours from before a re-join (not in our document): like a peer's.
                }
                self.collab_apply_remote(transaction, now, out);
                self.collab_check_backup();
            }
            CollabMessage::Hello {
                site: peer, name, ..
            } => {
                if peer == site {
                    return;
                }
                let s = self.collab.session.as_mut().expect("in session");
                s.names.insert(peer, name.clone());
                if let Some(p) = s.peers.get_mut(&peer) {
                    p.name = name;
                }
                self.collab_emit_peers(out);
            }
            CollabMessage::Presence { mut presence } => {
                if presence.site == site {
                    return;
                }
                let s = self.collab.session.as_mut().expect("in session");
                if presence.name.is_empty()
                    && let Some(n) = s.names.get(&presence.site)
                {
                    presence.name = n.clone();
                }
                if s.peers.get(&presence.site) != Some(&presence) {
                    s.peers.insert(presence.site, presence);
                    self.collab_emit_peers(out);
                }
            }
            CollabMessage::Leave { site: peer } => {
                let s = self.collab.session.as_mut().expect("in session");
                s.names.remove(&peer);
                if s.peers.remove(&peer).is_some() {
                    self.collab_emit_peers(out);
                }
                self.collab_pointer_peer_left(peer, out);
                self.collab_listen_peer_left(peer, out);
                self.collab_host_peer_left(peer, out);
            }
            CollabMessage::Media {
                file,
                hash,
                offset,
                total,
                data,
            } => self.collab_media_chunk(file, hash, offset, total, data.0),
            CollabMessage::Update { .. } => {}
            // ─── base-53 ───
            CollabMessage::Pointer {
                site: peer,
                pointer,
            } => {
                if peer != site {
                    self.collab_pointer_message(peer, pointer, out);
                }
            }
            m @ (CollabMessage::Listen { .. }
            | CollabMessage::Unlisten { .. }
            | CollabMessage::TransportRequest { .. }) => {
                if m.route().is_some_and(|(_, to)| to == site) {
                    self.collab_host_message(m, out);
                }
            }
            CollabMessage::Signal {
                from,
                to,
                stream,
                signal,
            } => {
                if to != site {
                    return;
                }
                if self.collab_hosts_stream(from, stream) {
                    self.collab_host_signal(from, stream, signal, out);
                } else {
                    self.collab_listen_signal(from, stream, signal, out);
                }
            }
            CollabMessage::StreamClock {
                from,
                to,
                stream,
                clock,
            } => {
                if to == site {
                    self.collab_listen_clock(from, stream, clock, out);
                }
            }
            CollabMessage::IceServers { servers } => {
                let s = self.collab.session.as_mut().expect("in session");
                if s.ice_relay != servers {
                    s.ice_relay = servers;
                    if self.collab.ice_override.is_none() {
                        self.collab_emit_ice_servers(out);
                    }
                }
            }
        }
    }

    /// Undo pending, apply `t` with the collab rules, re-apply pending; one patch.
    fn collab_apply_remote(&mut self, t: StampedTransaction, now: u64, out: &mut dyn MessageSink) {
        let (Some(doc), Some(s)) = (self.doc.as_mut(), self.collab.session.as_mut()) else {
            return;
        };
        let project = &mut doc.project;
        // Derived, never-replicated data (media length from decode, mirrored plugin
        // params) of entities our pending transactions created: the re-apply below
        // re-inserts them as sent, so it is carried over (COLLAB.md §2).
        let derived = derived_of_pending(project, &s.pending);
        // This site's mix (mute/solo, COLLAB.md §2.1) is kept across the rebase: re-applied
        // pending inserts carry none; tracks a peer created start unmuted, unless ours before.
        let mix = resolve::LocalMix::of(project).or(&s.mix_seed);
        let mut touched = Vec::new();
        for p in s.pending.iter().rev() {
            for inv in &p.inverse {
                let r = project.apply(inv);
                debug_assert!(r.is_ok(), "pending inverses always apply: {r:?}");
                if r.is_ok() {
                    touched.push(inv.clone());
                }
            }
        }
        let ops: Vec<Op> = t
            .transaction
            .ops
            .into_iter()
            .filter(|op| !resolve::is_local_only(op))
            .collect();
        let (applied, _) = resolve::resolve_all(project, &ops);
        let replaced: Vec<DeviceId> = applied
            .iter()
            .filter_map(|op| match op {
                Op::Update {
                    update:
                        EntityUpdate::Device {
                            id,
                            change: DeviceChange::Plugin(_),
                        },
                } => Some(*id),
                _ => None,
            })
            .collect();
        touched.extend(applied);
        for p in s.pending.iter_mut() {
            let (applied, inverse) = resolve::resolve_all(project, &p.tx.transaction.ops);
            p.inverse = inverse;
            touched.extend(applied);
        }
        restore_derived(project, derived);
        touched.extend(mix.overlay(project));
        // A peer replicated a plugin state (COLLAB.md §2.2): our running instance is
        // re-created from it (a missing plugin has no instance: the document keeps it), and
        // it is our new baseline, so our next save doesn't send it back.
        for id in replaced {
            let doc_state =
                self.doc
                    .as_ref()
                    .and_then(|d| match &d.project.devices.get(&id)?.kind {
                        DeviceKind::Plugin { plugin } => plugin.state.clone(),
                        DeviceKind::Builtin { .. } => None,
                    });
            let Some(state) = doc_state else { continue };
            let Ok(Some(live)) = self.bridge.plugin_state(id) else {
                continue;
            };
            if live != state {
                self.engine.request_reload_from_doc(id);
            }
            if let Some(s) = self.collab.session.as_mut() {
                s.captured.insert(id, state);
            }
        }
        self.after_ops_from(&touched, Some(t.origin), now, out);
    }

    // ─── Media ──────────────────────────────────────────────────────────────────────────

    fn collab_media_chunk(
        &mut self,
        file: String,
        hash: String,
        offset: u64,
        total: u64,
        data: Vec<u8>,
    ) {
        let valid = check_relative_path(&file).is_ok()
            && file.starts_with(&format!("{}/", file::MEDIA_DIR))
            && total <= MAX_MEDIA_FILE
            && offset + data.len() as u64 <= total;
        let Some(s) = self.collab.session.as_mut() else {
            return;
        };
        if !valid {
            return;
        }
        if offset == 0 {
            if let Some(old) = s.incoming.remove(&file)
                && let Some(id) = old.staged
            {
                let _ = self.store.discard_upload(&id);
            }
            s.upload_counter += 1;
            let id = format!("collab-{}-{}", s.upload_counter, s.seq);
            let staged = self.store.begin_upload(&id, total).ok().map(|()| id);
            s.incoming.insert(
                file.clone(),
                Incoming {
                    hash: hash.clone(),
                    total,
                    received: 0,
                    staged,
                    buf: Vec::new(),
                },
            );
        }
        let Some(inc) = s.incoming.get_mut(&file) else {
            return;
        };
        if inc.received != offset || inc.hash != hash || inc.total != total {
            if let Some(id) = s.incoming.remove(&file).and_then(|i| i.staged) {
                let _ = self.store.discard_upload(&id);
            }
            return;
        }
        match &inc.staged {
            Some(id) => {
                if self.store.append_upload(id, offset, &data).is_err() {
                    let _ = self.store.discard_upload(id);
                    s.incoming.remove(&file);
                    return;
                }
            }
            None => inc.buf.extend_from_slice(&data),
        }
        inc.received += data.len() as u64;
        if inc.received < inc.total {
            return;
        }
        let inc = s.incoming.remove(&file).expect("present");
        let bytes = match &inc.staged {
            Some(id) => {
                let b = self.store.read_upload(id);
                let _ = self.store.discard_upload(id);
                match b {
                    Ok(b) => b,
                    Err(_) => return,
                }
            }
            None => inc.buf,
        };
        if content_hash(&bytes) != inc.hash {
            return;
        }
        match s.project.filter(|_| s.joined) {
            Some(pid) => {
                let project = self.doc.as_ref().map(|d| &d.project);
                if media_acceptable(&mut self.store, pid, project, &file, &bytes)
                    && self.store.write(pid, &file, &bytes).is_ok()
                    && let Some(doc) = self.doc.as_ref()
                {
                    // Media inserted before its bytes arrived (should not happen with the
                    // relay's FIFO order) is retried now.
                    self.media.sync(&mut self.bridge, Some(&doc.project));
                }
            }
            None => s.staged.push((file, bytes)),
        }
    }

    // ─── Snapshots ──────────────────────────────────────────────────────────────────────

    /// The open project as a snapshot `.ether` file: every plugin's live state (like saving),
    /// no site-local mix.
    fn collab_ether(&mut self) -> CmdResult<String> {
        let doc = self.doc.as_ref().ok_or_else(no_project)?;
        let mut copy = doc.project.clone();
        // Snapshots carry no mix: mute/solo are per-site (COLLAB.md §2.1).
        resolve::LocalMix::default().overlay(&mut copy);
        let live = self.collab_live_plugin_states();
        for d in copy.devices.values_mut() {
            if let DeviceKind::Plugin { plugin } = &mut d.kind
                && let Some(state) = live.get(&d.id)
            {
                plugin.state = Some(state.clone());
            }
        }
        file::save(&copy, &self.config.app_version).map_err(|e| internal(e.to_string()))
    }

    /// The live state of every instantiated plugin of the open project (a missing,
    /// bypassed plugin has none).
    fn collab_live_plugin_states(&mut self) -> BTreeMap<DeviceId, Base64Bytes> {
        let ids: Vec<DeviceId> = self.doc.as_ref().map_or_else(Vec::new, |d| {
            d.project
                .devices
                .values()
                .filter(|d| matches!(d.kind, DeviceKind::Plugin { .. }))
                .map(|d| d.id)
                .collect()
        });
        ids.into_iter()
            .filter_map(|id| Some((id, self.bridge.plugin_state(id).ok().flatten()?)))
            .collect()
    }

    /// Before a save (explicit or autosave) in a session: replicate the plugin states that
    /// changed since this site last sent, received or started from them, once, as a
    /// stamped transaction outside the undo history (COLLAB.md §2.2). Missing plugins have
    /// no live state and never replicate anything.
    pub(crate) fn collab_before_save(&mut self, now: u64, out: &mut dyn MessageSink) {
        if !self.collab_active() {
            return;
        }
        let live = self.collab_live_plugin_states();
        let (Some(doc), Some(s)) = (self.doc.as_mut(), self.collab.session.as_mut()) else {
            return;
        };
        let mut ops = Vec::new();
        for (id, state) in live {
            if s.captured.get(&id) == Some(&state) {
                continue;
            }
            s.captured.insert(id, state.clone());
            if let Some(Device {
                kind: DeviceKind::Plugin { plugin },
                ..
            }) = doc.project.devices.get(&id)
                && plugin.state.as_ref() != Some(&state)
            {
                let mut plugin = plugin.clone();
                plugin.state = Some(state);
                ops.push(Op::Update {
                    update: EntityUpdate::Device {
                        id,
                        change: DeviceChange::Plugin(plugin),
                    },
                });
            }
        }
        if ops.is_empty() {
            return;
        }
        let (applied, inverse) = resolve::resolve_all(&mut doc.project, &ops);
        self.collab_local_commit("Plugin State", &applied, &inverse);
        self.after_ops(&applied, now, out);
    }

    /// We create the session: media first, then a snapshot of the open project (index 0).
    fn collab_create(&mut self, site: SiteId) -> CmdResult<()> {
        let doc = self
            .doc
            .as_ref()
            .ok_or_else(|| invalid_state("open a project to start a session"))?;
        let pid = doc.project.id;
        let medias: Vec<MediaRef> = doc.project.media.values().cloned().collect();
        let ether = self.collab_ether()?;
        let live = self.collab_live_plugin_states();
        let epoch = {
            let seed = self.host.random_seed();
            let now = self.host.now_ms();
            // Distinct per creating site, and per creation on one site.
            let mut x = seed ^ site.0.rotate_left(17) ^ now.rotate_left(41);
            x ^= self
                .collab
                .last_seq
                .wrapping_add(1)
                .wrapping_mul(0x9e37_79b9_7f4a_7c15);
            self.collab.created += 1;
            x ^ self.collab.created.wrapping_mul(0xbf58_476d_1ce4_e5b9)
        };
        // Everything the document includes stays included: a recreated session keeps the
        // per-site `seq` it has (ours, or the last session left with this project), so a
        // returning site resends only what we don't have (never an old edit that would be
        // re-sequenced after, and silently revert, a newer one).
        let left = match self.collab.left_sites.take() {
            Some((p, sites)) if p == pid => Some(sites),
            _ => None,
        };
        let s = self.collab.session.as_mut().expect("in session");
        let mut sites = if s.joined && s.project == Some(pid) {
            std::mem::take(&mut s.sites)
        } else {
            left.unwrap_or_default()
        };
        let own = sites.get(&site).copied().unwrap_or(0).max(s.seq);
        s.seq = own;
        sites.insert(site, own);
        // Peers start from these plugin states.
        s.captured = live;
        // Pending edits are part of the snapshot.
        s.pending.clear();
        s.joined = true;
        s.project = Some(pid);
        s.index = 0;
        // A new incarnation of the session (it may have existed before on the relay).
        s.epoch = epoch;
        s.sites = sites;
        for m in &medias {
            self.collab_push_media(pid, m);
        }
        let s = self.collab.session.as_mut().expect("in session");
        let data = SnapshotData {
            epoch,
            index: 0,
            sites: s.sites.clone(),
            ether,
        }
        .encode();
        s.send(&CollabMessage::Snapshot { data });
        Ok(())
    }

    fn collab_send_owed_snapshot(&mut self) {
        let Some(s) = self.collab.session.as_ref() else {
            return;
        };
        if !s.owe_snapshot || !s.pending.is_empty() || !s.open() {
            return;
        }
        let Ok(ether) = self.collab_ether() else {
            return;
        };
        let s = self.collab.session.as_mut().expect("checked");
        s.owe_snapshot = false;
        let data = SnapshotData {
            epoch: s.epoch,
            index: s.index,
            sites: s.sites.clone(),
            ether,
        }
        .encode();
        s.send(&CollabMessage::Snapshot { data });
    }

    /// Adopt the session state from a snapshot (join, or resync after compaction).
    fn collab_adopt(
        &mut self,
        data: &Base64Bytes,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<()> {
        let site = self.collab_site();
        let snap = SnapshotData::decode(data).map_err(invalid)?;
        let mut project = file::load(&snap.ether).map_err(|e| invalid(e.to_string()))?;
        let pid = project.id;
        let current = self.doc.as_ref().map(|d| d.project.clone());
        // Keep our local settings when we already have this project open.
        let local_from = current
            .as_ref()
            .filter(|p| p.id == pid)
            .map(|p| p.settings.clone());
        let local_ops: Vec<Op> = local_from
            .as_ref()
            .map(resolve::local_settings)
            .unwrap_or_default();
        for op in &local_ops {
            let _ = project.apply(op);
        }
        // Our mix (mute/solo, COLLAB.md §2.1): from the open copy of this project, else from
        // the stored one (saved with our mix); tracks we don't know start unmuted/unsoloed.
        let mix = match current.as_ref().filter(|p| p.id == pid) {
            Some(p) => resolve::LocalMix::of(p),
            None => self
                .store
                .load(pid)
                .ok()
                .and_then(|json| file::load(&json).ok())
                .map(|p| resolve::LocalMix::of(&p))
                .unwrap_or_default(),
        };
        mix.overlay(&mut project);
        // Save our current work first (it is not overwritten: see below).
        if self.doc.as_ref().is_some_and(|d| d.dirty) {
            self.save_current(out)?;
        }
        let ether =
            file::save(&project, &self.config.app_version).map_err(|e| internal(e.to_string()))?;
        // A resync of the session we are in keeps our work in `pending` (re-applied below);
        // otherwise a stored copy that differs is local work we must not overwrite.
        let resync = self
            .collab
            .session
            .as_ref()
            .is_some_and(|s| s.joined && s.project == Some(pid));
        let mut backup = None;
        match self.store.load(pid) {
            Ok(existing) => {
                if !resync
                    && let Ok(p) = file::load(&existing)
                    && resolve::shared_part(&p) != resolve::shared_part(&project)
                {
                    // Maybe offline work, maybe just older than the log: decided once the
                    // log is applied (see `Session::backup`).
                    backup = Some(Backup {
                        shared: Some(resolve::shared_part(&p)),
                        json: existing,
                    });
                } else if !resync && file::load(&existing).is_err() {
                    backup = Some(Backup {
                        shared: None,
                        json: existing,
                    });
                }
            }
            Err(_) => {
                let _ = self.store.create(pid);
            }
        }
        self.store.save(pid, &ether).map_err(store_err)?;
        let staged = self
            .collab
            .session
            .as_mut()
            .map(|s| std::mem::take(&mut s.staged))
            .unwrap_or_default();
        for (f, bytes) in staged {
            if media_acceptable(&mut self.store, pid, Some(&project), &f, &bytes) {
                let _ = self.store.write(pid, &f, &bytes);
            }
        }
        if let Some(d) = self.doc.as_mut() {
            // Saved above: don't save the old state over the snapshot when switching.
            d.dirty = false;
        }
        self.open(pid, now, out)?;
        let s = self.collab.session.as_mut().expect("in session");
        let own = snap.sites.get(&site).copied().unwrap_or(0);
        s.pending.retain(|p| p.tx.origin.seq > own);
        s.seq = s.seq.max(own);
        s.joined = true;
        s.project = Some(pid);
        s.index = snap.index;
        s.epoch = snap.epoch;
        s.sites = snap.sites;
        s.backup = backup;
        s.mix_seed = mix.or(&s.mix_seed);
        // Our plugins are created from the snapshot's states.
        s.captured = project
            .devices
            .values()
            .filter_map(|d| match &d.kind {
                DeviceKind::Plugin { plugin } => Some((d.id, plugin.state.clone()?)),
                DeviceKind::Builtin { .. } => None,
            })
            .collect();
        // Pending edits (resync) go on top of the new state and are sent again.
        if !s.pending.is_empty() {
            let doc = self.doc.as_mut().expect("opened");
            let mix = resolve::LocalMix::of(&doc.project).or(&s.mix_seed);
            let mut touched = Vec::new();
            for p in s.pending.iter_mut() {
                let (applied, inverse) =
                    resolve::resolve_all(&mut doc.project, &p.tx.transaction.ops);
                p.inverse = inverse;
                touched.extend(applied);
            }
            touched.extend(mix.overlay(&mut doc.project));
            self.after_ops_from(&touched, None, now, out);
            self.collab_resend_pending(own);
        }
        self.collab_check_backup();
        Ok(())
    }

    /// Drop the backup candidate if the document is now the stored copy (it was a state of
    /// the session, not offline work).
    fn collab_check_backup(&mut self) {
        let (Some(doc), Some(s)) = (self.doc.as_ref(), self.collab.session.as_mut()) else {
            return;
        };
        if s.pending.is_empty()
            && s.backup
                .as_ref()
                .is_some_and(|b| b.shared.as_ref() == Some(&resolve::shared_part(&doc.project)))
        {
            s.backup = None;
        }
    }

    /// The log is applied (a quiet tick) and the stored copy never matched: keep it.
    fn collab_flush_backup(&mut self, now: u64, out: &mut dyn MessageSink) {
        let Some(s) = self.collab.session.as_mut() else {
            return;
        };
        let (Some(b), Some(pid)) = (s.backup.take(), s.project) else {
            return;
        };
        if let Err(e) = self.collab_backup(pid, &b.json, now, out) {
            notify(
                out,
                NotificationLevel::Error,
                format!(
                    "collaboration: could not keep your local version: {}",
                    e.message
                ),
            );
        }
    }

    /// Keep `json` (the stored copy of `pid` before the session replaced it) as a new
    /// project "<name> (local copy)", with `pid`'s media (media files are never removed and
    /// are named per media id, so the old ones are all there).
    fn collab_backup(
        &mut self,
        pid: ProjectId,
        json: &str,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<()> {
        let backup = self.ids.next_project_id(now);
        self.store.duplicate(pid, backup).map_err(store_err)?;
        let mut copy = file::load(json).map_err(|e| internal(e.to_string()))?;
        copy.id = backup;
        copy.settings.name = format!("{} (local copy)", copy.settings.name);
        let name = copy.settings.name.clone();
        let json =
            file::save(&copy, &self.config.app_version).map_err(|e| internal(e.to_string()))?;
        self.store.save(backup, &json).map_err(store_err)?;
        notify(
            out,
            NotificationLevel::Info,
            format!("your local version of this project was kept as \"{name}\""),
        );
        self.emit_list_changed(out);
        Ok(())
    }
}

/// Debug check of the convergence invariant for an own echo (COLLAB.md §2): processing it
/// like a peer's transaction (undo pending, apply it with resolve on the confirmed state,
/// re-apply the rest) yields the live document we already have, so popping it is exact.
#[cfg(debug_assertions)]
fn check_echo(live: &Project, pending: &VecDeque<Pending>, echo: &StampedTransaction) {
    let Some(head) = pending.front() else { return };
    if head.tx.origin.seq != echo.origin.seq {
        return;
    }
    let mut p = live.clone();
    for q in pending.iter().rev() {
        for inv in &q.inverse {
            let _ = p.apply(inv);
        }
    }
    let ops: Vec<Op> = echo
        .transaction
        .ops
        .iter()
        .filter(|op| !resolve::is_local_only(op))
        .cloned()
        .collect();
    resolve::resolve_all(&mut p, &ops);
    for q in pending.iter().skip(1) {
        resolve::resolve_all(&mut p, &q.tx.transaction.ops);
    }
    resolve::LocalMix::of(live).overlay(&mut p);
    debug_assert!(
        &p == live,
        "own echo processed like a remote transaction must give the live document"
    );
}

/// May `bytes` be written to `pid`'s `file`? Not if the document has a media with that file
/// and a different hash, nor if a different file is already stored there (media files are
/// named per media id: an existing one is never replaced). Identical content: rewriting is
/// harmless.
fn media_acceptable<S: ProjectStore>(
    store: &mut S,
    pid: ProjectId,
    project: Option<&Project>,
    file: &str,
    bytes: &[u8],
) -> bool {
    let hash = content_hash(bytes);
    if let Some(p) = project
        && p.media
            .values()
            .any(|m| m.file == file && m.hash.as_ref().is_some_and(|h| *h != hash))
    {
        return false;
    }
    match store.read(pid, file) {
        Ok(existing) => existing == bytes,
        Err(_) => true,
    }
}

/// Derived data of the entities pending transactions insert (see `collab_apply_remote`).
struct Derived {
    media: Vec<(MediaId, u64)>,
    params: Vec<(DeviceId, BTreeMap<ParamId, f64>)>,
}

fn derived_of_pending(project: &Project, pending: &VecDeque<Pending>) -> Derived {
    let mut d = Derived {
        media: Vec::new(),
        params: Vec::new(),
    };
    for p in pending {
        for op in &p.tx.transaction.ops {
            match op {
                Op::Insert {
                    entity: Entity::Media(m),
                } => {
                    if let Some(cur) = project.media.get(&m.id) {
                        d.media.push((m.id, cur.frames));
                    }
                }
                Op::Insert {
                    entity: Entity::Device(dev),
                } if matches!(dev.kind, DeviceKind::Plugin { .. }) => {
                    if let Some(cur) = project.devices.get(&dev.id) {
                        d.params.push((dev.id, cur.params.clone()));
                    }
                }
                _ => {}
            }
        }
    }
    d
}

fn restore_derived(project: &mut Project, d: Derived) {
    for (id, frames) in d.media {
        if let Some(m) = project.media.get_mut(&id)
            && m.frames == 0
        {
            m.frames = frames;
        }
    }
    for (id, params) in d.params {
        if let Some(dev) = project.devices.get_mut(&id) {
            dev.params = params;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryStore;

    #[test]
    fn received_media_never_replaces_other_content() {
        let mut ids = IdGen::new(7);
        let mut project = Project::new(&mut ids, 0);
        let pid = project.id;
        let mut store = MemoryStore::new();
        store.create(pid).unwrap();
        let file = "media/01-kick.wav";
        let good = b"kick".to_vec();
        // The document's hash is authoritative (not only the sender's declared one).
        let id = MediaId(ids.next_ulid(0));
        project.media.insert(
            id,
            MediaRef {
                location: Default::default(),
                id,
                name: "kick.wav".into(),
                file: file.into(),
                sample_rate: 48_000,
                channels: 1,
                frames: 0,
                hash: Some(content_hash(&good)),
            },
        );
        assert!(!media_acceptable(
            &mut store,
            pid,
            Some(&project),
            file,
            b"evil"
        ));
        assert!(media_acceptable(
            &mut store,
            pid,
            Some(&project),
            file,
            &good
        ));
        // An existing file is never overwritten with different bytes (same bytes: fine).
        store.write(pid, "media/02-snare.wav", b"snare").unwrap();
        assert!(!media_acceptable(
            &mut store,
            pid,
            None,
            "media/02-snare.wav",
            b"other"
        ));
        assert!(media_acceptable(
            &mut store,
            pid,
            None,
            "media/02-snare.wav",
            b"snare"
        ));
        assert!(media_acceptable(
            &mut store,
            pid,
            None,
            "media/03-new.wav",
            b"new"
        ));
    }
}

//! The relay: sequencer and fan-out of collaboration sessions (docs/COLLAB.md §1-§6).
//!
//! [`Relay`] is a pure state machine (no I/O, wasm-safe): connections come and go, messages
//! come in, and every call returns the messages to send and to whom. The WebSocket server
//! ([`server`], native) and the in-memory hub used by tests ([`crate::memory`]) drive it.
//!
//! Per session it keeps: the latest snapshot (and the log index it stands for), the log of
//! transactions after it, the media chunks seen (for late joiners), and the peers.
//!
//! - A connection starts with `Hello { site, .. }`, then `SyncRequest { version }` (empty =
//!   full state, else "I have applied `index` transactions"). Nothing is sent to a peer
//!   before its `SyncRequest` is answered ("ready").
//! - The first site of an empty session gets `SyncRequest { site: itself, version: [] }`
//!   back: it must upload its media and then a `Snapshot` (index 0). Other sites wait.
//! - `Transaction`s are appended to the log and sent to every ready peer, the author
//!   included (the echo is its ack). A transaction whose `origin.site` is not the sender's
//!   site, or whose `seq` is not above that site's last sequenced one (a resend), is dropped.
//!   Those sent while waiting for the first snapshot are held and sequenced once the sender
//!   is ready: per site, what is sequenced is always a gap-free prefix of what was sent.
//! - `Presence` is stamped with the sender's site and color, `Pointer` with its site;
//!   `Hello`, `Presence`, `Pointer` and `Leave` are forwarded to the other ready peers
//!   (pointers are never cached).
//! - Site-to-site messages (`Signal`, `Listen`, `Unlisten`, `TransportRequest`,
//!   `StreamClock`; [`CollabMessage::route`]) must come from the sender's own site, between
//!   synced members of the session, and go to their `to` site only. `IceServers` only goes
//!   relay → site ([`Relay::set_ice_provider`]).
//! - A site id is held by one connection. A `Hello` for a site another connection holds
//!   starts a contest: the holder is pinged ([`Relay::take_pings`]) and the newcomer's
//!   messages are held. Any sign of life from the holder ([`Relay::heard`], or a message)
//!   within [`RelayConfig::site_probe_ms`] refuses the newcomer; otherwise ([`Relay::tick`])
//!   the holder is dropped as half-open and the newcomer proceeds. A newcomer never evicts
//!   a live holder (the token is relay-wide). Connections to close are in
//!   [`Relay::take_closing`].
//! - Past [`RelayConfig::compact_after`] log entries, one ready peer is asked for a snapshot
//!   (`SyncRequest { site: it, version: [] }`); the log before its index is dropped.

#[cfg(not(target_arch = "wasm32"))]
pub mod ice;
pub mod limits;
#[cfg(not(target_arch = "wasm32"))]
pub mod server;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use ether_protocol::collab::{CollabMessage, IceServer, Presence, PresenceState, StreamSignal};
use ether_protocol::model::{Base64Bytes, Color, SiteId, StampedTransaction};

use crate::wire::{PEER_COLORS, SnapshotData, decode_version, valid_session_name};

/// Relay-local connection id.
pub type ConnId = u64;

/// One outgoing message (shared between the recipients of a broadcast).
pub type Outgoing = (ConnId, Arc<CollabMessage>);

#[derive(Clone, Debug, PartialEq)]
pub struct RelayConfig {
    pub max_sessions: usize,
    pub max_sites_per_session: usize,
    /// Media bytes cached per session for late joiners; chunks beyond it (or beyond
    /// `max_total_media_bytes`) are still forwarded to the sites online, but late joiners
    /// won't get them.
    pub max_media_bytes: usize,
    /// Media bytes cached across all sessions.
    pub max_total_media_bytes: usize,
    /// Ask for a snapshot when the log grows past this many transactions.
    pub compact_after: usize,
    /// Hard cap: past it, new transactions are refused (the sender is disconnected and
    /// resyncs on reconnect) until a snapshot shrinks the log.
    pub max_log: usize,
    /// How long a site id's holder has to show a sign of life when another connection
    /// says hello with that id (after that it is taken for half-open and dropped).
    pub site_probe_ms: u64,
    /// ICE servers are advertised again this long after the last advertisement (before
    /// time-limited TURN credentials expire; docs/COLLAB.md §10).
    pub ice_refresh_ms: u64,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            max_sessions: 64,
            max_sites_per_session: 16,
            max_media_bytes: 1 << 30,
            max_total_media_bytes: 4 << 30,
            compact_after: 5_000,
            max_log: 20_000,
            site_probe_ms: 4_000,
            ice_refresh_ms: 6 * 3_600_000,
        }
    }
}

/// Why a connection was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    #[error("invalid session name")]
    BadSession,
    #[error("too many sessions on this relay")]
    TooManySessions,
    #[error("the session is full")]
    SessionFull,
}

struct Peer {
    /// Set by `Hello`.
    site: Option<SiteId>,
    hello: Option<Arc<CollabMessage>>,
    color: Color,
    presence: Option<Arc<CollabMessage>>,
    ready: bool,
    /// Asked to provide the session's first snapshot.
    creator: bool,
    /// A `SyncRequest` that arrived before the session had a snapshot.
    waiting: Option<Option<(u64, u64)>>,
    /// This connection said hello with a site id another connection holds.
    contest: Option<Contest>,
    /// Transactions sent while waiting for the session's first snapshot, sequenced (in
    /// order) once this peer is ready: dropping them would let a later `seq` be sequenced
    /// first, and the dropped ones would then be refused as resends forever.
    early: Vec<StampedTransaction>,
    /// When the ICE servers were last advertised to this peer (relay clock).
    ice_sent_ms: Option<u64>,
}

/// A newcomer waiting for the holder of its site id to prove alive (or not).
struct Contest {
    holder: ConnId,
    since_ms: u64,
    /// The newcomer's messages (its `Hello` first), handled once it wins.
    held: Vec<CollabMessage>,
}

/// Messages a contending connection may send before the contest is decided.
const MAX_HELD: usize = 64;

struct MediaChunk {
    /// Log length when it arrived (resumers get the chunks at or after their index).
    at: u64,
    message: Arc<CollabMessage>,
}

#[derive(Default)]
struct Session {
    peers: BTreeMap<ConnId, Peer>,
    snapshot: Option<Arc<CollabMessage>>,
    /// Log index of the snapshot (= index of `log[0]`).
    base: u64,
    /// The session incarnation (`SnapshotData::epoch` of its first snapshot).
    epoch: u64,
    log: Vec<Arc<CollabMessage>>,
    last_seq: HashMap<SiteId, u64>,
    media: Vec<MediaChunk>,
    media_keys: std::collections::HashSet<(String, u64)>,
    media_bytes: usize,
    /// A compaction snapshot was requested from this connection, at this log length.
    compacting: Option<(ConnId, u64)>,
}

impl Session {
    fn end(&self) -> u64 {
        self.base + self.log.len() as u64
    }

    fn free_color(&self) -> Color {
        PEER_COLORS
            .iter()
            .copied()
            .find(|c| !self.peers.values().any(|p| p.color == *c))
            .unwrap_or(PEER_COLORS[self.peers.len() % PEER_COLORS.len()])
    }

    fn broadcast(&self, except: Option<ConnId>, m: &Arc<CollabMessage>, out: &mut Vec<Outgoing>) {
        for (id, p) in &self.peers {
            if p.ready && Some(*id) != except {
                out.push((*id, m.clone()));
            }
        }
    }
}

/// See the module docs.
#[derive(Default)]
pub struct Relay {
    config: RelayConfig,
    sessions: HashMap<String, Session>,
    conns: HashMap<ConnId, String>,
    /// Media bytes cached across sessions.
    media_bytes: usize,
    /// The driver's clock ([`Relay::tick`]).
    now_ms: u64,
    /// Connections to ping now (holders of a contested site id).
    pings: Vec<ConnId>,
    /// Connections the relay dropped, with why: the driver closes their sockets.
    closing: Vec<(ConnId, String)>,
    /// The ICE servers advertised to each site ([`Relay::set_ice_provider`]).
    ice: Option<IceProvider>,
}

/// The ICE servers the relay advertises to `site` at relay time `now_ms` (per site, so TURN
/// credentials can name it and expire; docs/COLLAB.md §10). An empty list sends nothing.
#[cfg(not(target_arch = "wasm32"))]
pub type IceProvider = Box<dyn Fn(SiteId, u64) -> Vec<IceServer> + Send>;
#[cfg(target_arch = "wasm32")]
pub type IceProvider = Box<dyn Fn(SiteId, u64) -> Vec<IceServer>>;

/// Largest SDP a site may send in a `Signal` (audio-only SDPs are a few KiB).
pub const MAX_SDP_BYTES: usize = 32 << 10;
/// Largest ICE candidate line / `Bye` reason in a `Signal`.
pub const MAX_CANDIDATE_BYTES: usize = 1 << 10;

/// Checks the size of a site-to-site message's free-form strings.
fn check_routed(m: &CollabMessage) -> Result<(), Dropped> {
    let ok = match m {
        CollabMessage::Signal { signal, .. } => match signal {
            StreamSignal::Offer { sdp } | StreamSignal::Answer { sdp } => {
                sdp.len() <= MAX_SDP_BYTES
            }
            StreamSignal::Ice { candidate } => {
                candidate.candidate.len()
                    + candidate.sdp_mid.as_ref().map_or(0, String::len)
                    + candidate.username_fragment.as_ref().map_or(0, String::len)
                    <= MAX_CANDIDATE_BYTES
            }
            StreamSignal::Bye { reason } => {
                reason.as_ref().map_or(0, String::len) <= MAX_CANDIDATE_BYTES
            }
        },
        _ => true,
    };
    if ok {
        Ok(())
    } else {
        Err("signal too large".into())
    }
}

/// Why a message was dropped (for logs). `disconnect`: close the sender's link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dropped {
    pub reason: String,
    pub disconnect: bool,
}

impl From<String> for Dropped {
    fn from(reason: String) -> Self {
        Self {
            reason,
            disconnect: false,
        }
    }
}

impl From<&str> for Dropped {
    fn from(reason: &str) -> Self {
        reason.to_string().into()
    }
}

impl Relay {
    pub fn new(config: RelayConfig) -> Self {
        Self {
            config,
            ..Self::default()
        }
    }

    /// Advertise ICE servers (STUN/TURN, docs/COLLAB.md §10) to every site once it is
    /// synced, and again every [`RelayConfig::ice_refresh_ms`].
    pub fn set_ice_provider(&mut self, provider: IceProvider) {
        self.ice = Some(provider);
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Sites connected to `session`.
    pub fn peer_count(&self, session: &str) -> usize {
        self.sessions.get(session).map_or(0, |s| s.peers.len())
    }

    /// Transactions in `session`'s log since its snapshot, and the snapshot's index.
    pub fn log_len(&self, session: &str) -> (u64, usize) {
        self.sessions
            .get(session)
            .map_or((0, 0), |s| (s.base, s.log.len()))
    }

    /// A new (authenticated) connection to `session`.
    pub fn connect(&mut self, conn: ConnId, session: &str) -> Result<(), Refusal> {
        if !valid_session_name(session) {
            return Err(Refusal::BadSession);
        }
        if !self.sessions.contains_key(session) && self.sessions.len() >= self.config.max_sessions {
            return Err(Refusal::TooManySessions);
        }
        let s = self.sessions.entry(session.to_string()).or_default();
        if s.peers.len() >= self.config.max_sites_per_session {
            if s.peers.is_empty() {
                self.sessions.remove(session);
            }
            return Err(Refusal::SessionFull);
        }
        let color = s.free_color();
        s.peers.insert(
            conn,
            Peer {
                site: None,
                hello: None,
                color,
                presence: None,
                ready: false,
                creator: false,
                waiting: None,
                contest: None,
                early: Vec::new(),
                ice_sent_ms: None,
            },
        );
        self.conns.insert(conn, session.to_string());
        Ok(())
    }

    /// The connection is gone (closed, dropped, timed out).
    pub fn disconnect(&mut self, conn: ConnId, out: &mut Vec<Outgoing>) {
        let Some(name) = self.conns.remove(&conn) else {
            return;
        };
        let Some(s) = self.sessions.get_mut(&name) else {
            return;
        };
        let Some(peer) = s.peers.remove(&conn) else {
            return;
        };
        if let Some(site) = peer.site
            && peer.ready
            && !s.peers.values().any(|p| p.site == Some(site))
        {
            s.broadcast(None, &Arc::new(CollabMessage::Leave { site }), out);
        }
        if s.compacting.is_some_and(|(c, _)| c == conn) {
            s.compacting = None;
        }
        if s.peers.is_empty() {
            // Nobody left: the session (log, snapshot, media) is dropped. Sites keep their
            // replica and can create it again.
            if let Some(s) = self.sessions.remove(&name) {
                self.media_bytes -= s.media_bytes;
            }
            return;
        }
        if peer.creator && s.snapshot.is_none() {
            Self::elect_creator(s, out);
        }
        // Newcomers contesting this connection's site id win.
        let winners: Vec<(ConnId, Vec<CollabMessage>)> = s
            .peers
            .iter_mut()
            .filter(|(_, p)| p.contest.as_ref().is_some_and(|c| c.holder == conn))
            .filter_map(|(id, p)| Some((*id, p.contest.take()?.held)))
            .collect();
        for (w, held) in winners {
            for m in held {
                if let Err(e) = self.message(w, m, out)
                    && e.disconnect
                {
                    self.drop_conn(w, e.reason, out);
                    break;
                }
            }
        }
    }

    /// Drop `conn` (the driver closes its socket, see [`Relay::take_closing`]).
    fn drop_conn(&mut self, conn: ConnId, reason: String, out: &mut Vec<Outgoing>) {
        if self.conns.contains_key(&conn) {
            self.closing.push((conn, reason));
            self.disconnect(conn, out);
        }
    }

    /// Advance the clock: holders that didn't answer their probe in time are dropped (their
    /// contenders proceed).
    pub fn tick(&mut self, now_ms: u64, out: &mut Vec<Outgoing>) {
        self.now_ms = self.now_ms.max(now_ms);
        let probe = self.config.site_probe_ms;
        let expired: Vec<ConnId> = self
            .sessions
            .values()
            .flat_map(|s| s.peers.values())
            .filter_map(|p| p.contest.as_ref())
            .filter(|c| self.now_ms >= c.since_ms + probe)
            .map(|c| c.holder)
            .collect();
        for holder in expired {
            self.drop_conn(
                holder,
                "no answer (a new connection of this site took over)".into(),
                out,
            );
        }
        if self.ice.is_some() {
            let names: Vec<String> = self.sessions.keys().cloned().collect();
            for name in names {
                self.advertise_ice(&name, out);
            }
        }
    }

    /// A sign of life from `conn` (a pong, any frame): contenders for its site id lose.
    pub fn heard(&mut self, conn: ConnId) {
        let Some(s) = self.conns.get(&conn).and_then(|n| self.sessions.get_mut(n)) else {
            return;
        };
        let mut losers = Vec::new();
        for (id, p) in s.peers.iter_mut() {
            if p.contest.as_ref().is_some_and(|c| c.holder == conn) {
                p.contest = None;
                losers.push(*id);
            }
        }
        let mut out = Vec::new();
        for l in losers {
            self.drop_conn(
                l,
                "this site is already connected to the session".into(),
                &mut out,
            );
        }
        debug_assert!(out.is_empty(), "contenders are not introduced to anyone");
    }

    /// Connections to ping now (see the module docs).
    pub fn take_pings(&mut self) -> Vec<ConnId> {
        std::mem::take(&mut self.pings)
    }

    /// Connections the relay dropped (already disconnected from the relay state): the
    /// driver closes their sockets with the reason.
    pub fn take_closing(&mut self) -> Vec<(ConnId, String)> {
        std::mem::take(&mut self.closing)
    }

    fn elect_creator(s: &mut Session, out: &mut Vec<Outgoing>) {
        if s.peers.values().any(|p| p.creator) {
            return;
        }
        if let Some((id, p)) = s
            .peers
            .iter_mut()
            .find(|(_, p)| p.waiting.is_some() && p.site.is_some())
        {
            p.creator = true;
            let site = p.site.expect("checked");
            out.push((
                *id,
                Arc::new(CollabMessage::SyncRequest {
                    site,
                    version: Base64Bytes(Vec::new()),
                }),
            ));
        }
    }

    /// A message from `conn`. Malformed or unauthorized messages are dropped (`Err` says
    /// why, for logs); the connection stays.
    pub fn message(
        &mut self,
        conn: ConnId,
        message: CollabMessage,
        out: &mut Vec<Outgoing>,
    ) -> Result<(), Dropped> {
        let session = self.conns.get(&conn).cloned();
        let r = self.message_inner(conn, message, out);
        if let Some(name) = session {
            self.sequence_early(&name, out);
            self.advertise_ice(&name, out);
        }
        r
    }

    /// Sequence the transactions peers of `session` sent before they were ready, now that
    /// they are (see [`Peer::early`]).
    fn sequence_early(&mut self, session: &str, out: &mut Vec<Outgoing>) {
        loop {
            let Some(s) = self.sessions.get_mut(session) else {
                return;
            };
            let Some((conn, early)) = s
                .peers
                .iter_mut()
                .find(|(_, p)| p.ready && !p.early.is_empty())
                .map(|(id, p)| (*id, std::mem::take(&mut p.early)))
            else {
                return;
            };
            for transaction in early {
                if let Err(e) =
                    self.message_inner(conn, CollabMessage::Transaction { transaction }, out)
                    && e.disconnect
                {
                    self.drop_conn(conn, e.reason, out);
                    break;
                }
            }
        }
    }

    /// Send the ICE servers to the ready peers of `session` that have none yet, or whose
    /// advertisement is older than [`RelayConfig::ice_refresh_ms`].
    fn advertise_ice(&mut self, session: &str, out: &mut Vec<Outgoing>) {
        let Some(ice) = self.ice.as_ref() else { return };
        let Some(s) = self.sessions.get_mut(session) else {
            return;
        };
        let now = self.now_ms;
        let refresh = self.config.ice_refresh_ms;
        for (id, p) in s.peers.iter_mut() {
            let due = p
                .ice_sent_ms
                .is_none_or(|t| now >= t.saturating_add(refresh));
            let (true, Some(site), true) = (p.ready, p.site, due) else {
                continue;
            };
            p.ice_sent_ms = Some(now);
            let servers = ice(site, now);
            if !servers.is_empty() {
                out.push((*id, Arc::new(CollabMessage::IceServers { servers })));
            }
        }
    }

    fn message_inner(
        &mut self,
        conn: ConnId,
        message: CollabMessage,
        out: &mut Vec<Outgoing>,
    ) -> Result<(), Dropped> {
        let name = self.conns.get(&conn).ok_or("unknown connection")?.clone();
        // A message is a sign of life (for a contest on this connection's site id).
        self.heard(conn);
        if !self.conns.contains_key(&conn) {
            return Err("unknown connection".into());
        }
        let now_ms = self.now_ms;
        let RelayConfig {
            compact_after,
            max_log,
            max_media_bytes,
            max_total_media_bytes,
            ..
        } = self.config;
        let total_media = self.media_bytes;
        let s = self.sessions.get_mut(&name).ok_or("unknown session")?;
        // A site id is held by one live connection at a time (no impersonation, no
        // eviction of a live holder; a half-open one is probed and dropped).
        let holder = match &message {
            CollabMessage::Hello { site, .. } => s
                .peers
                .iter()
                .find(|(id, p)| **id != conn && p.site == Some(*site))
                .map(|(id, _)| *id),
            _ => None,
        };
        let peer = s.peers.get_mut(&conn).ok_or("unknown peer")?;
        if let Some(c) = peer.contest.as_mut() {
            if c.held.len() >= MAX_HELD {
                return Err(Dropped {
                    reason: "too many messages before the site id was granted".into(),
                    disconnect: true,
                });
            }
            c.held.push(message);
            return Ok(());
        }
        match message {
            CollabMessage::Hello {
                site,
                actor,
                name,
                protocol_version,
            } => {
                if peer.site.is_some() {
                    return Err(Dropped {
                        reason: "second hello".into(),
                        disconnect: true,
                    });
                }
                if protocol_version != crate::wire::COLLAB_PROTOCOL_VERSION {
                    return Err(Dropped {
                        reason: format!("unsupported collab protocol {protocol_version}"),
                        disconnect: true,
                    });
                }
                if let Some(holder) = holder {
                    peer.contest = Some(Contest {
                        holder,
                        since_ms: now_ms,
                        held: vec![CollabMessage::Hello {
                            site,
                            actor,
                            name,
                            protocol_version,
                        }],
                    });
                    self.pings.push(holder);
                    return Ok(());
                }
                peer.site = Some(site);
                let name: String = name.chars().take(64).collect();
                peer.hello = Some(Arc::new(CollabMessage::Hello {
                    site,
                    actor,
                    name,
                    protocol_version,
                }));
                Ok(())
            }
            CollabMessage::SyncRequest { site, version } => {
                let own = peer.site.ok_or("sync before hello")?;
                if site != own {
                    return Err("sync request for another site".into());
                }
                let from = decode_version(&version)?;
                if s.snapshot.is_none() {
                    peer.waiting = Some(from);
                    Self::elect_creator(s, out);
                    return Ok(());
                }
                Self::answer_sync(s, conn, from, out);
                Ok(())
            }
            CollabMessage::Snapshot { data } => {
                let site = peer.site.ok_or("snapshot before hello")?;
                let snap = SnapshotData::decode(&data)?;
                if s.snapshot.is_none() {
                    if !peer.creator {
                        return Err("unrequested snapshot".into());
                    }
                    if snap.index != 0 {
                        return Err("the first snapshot must have index 0".into());
                    }
                    s.base = 0;
                    s.epoch = snap.epoch;
                    s.log.clear();
                    s.last_seq = snap.sites.clone().into_iter().collect();
                    s.snapshot = Some(Arc::new(CollabMessage::Snapshot { data }));
                    peer.creator = false;
                    peer.waiting = None;
                    peer.ready = true;
                    // The creator already has this state; everyone else waiting gets it.
                    let waiting: Vec<(ConnId, Option<(u64, u64)>)> = s
                        .peers
                        .iter()
                        .filter_map(|(id, p)| p.waiting.map(|w| (*id, w)))
                        .collect();
                    Self::introduce(s, conn, out);
                    for (id, from) in waiting {
                        Self::answer_sync(s, id, from, out);
                    }
                    let _ = site;
                    return Ok(());
                }
                if !s.compacting.is_some_and(|(c, _)| c == conn) {
                    return Err("unrequested snapshot".into());
                }
                if snap.epoch != s.epoch {
                    return Err("snapshot of another session epoch".into());
                }
                if snap.index < s.base || snap.index > s.end() {
                    return Err("snapshot index outside the log".into());
                }
                s.compacting = None;
                let drop = (snap.index - s.base) as usize;
                s.log.drain(..drop);
                s.base = snap.index;
                s.snapshot = Some(Arc::new(CollabMessage::Snapshot { data }));
                Ok(())
            }
            CollabMessage::Transaction { transaction } => {
                let site = peer.site.ok_or("transaction before hello")?;
                if !peer.ready {
                    // Waiting for the first snapshot (a site re-creating an emptied session
                    // resends its pending edits right after its `SyncRequest`): kept, in
                    // order, until it is ready. Anything else is a protocol error: never
                    // drop a transaction silently, a later one would be sequenced first.
                    if peer.waiting.is_none() && !peer.creator {
                        return Err(Dropped {
                            reason: "transaction before sync".into(),
                            disconnect: true,
                        });
                    }
                    if peer.early.len() >= max_log {
                        return Err(Dropped {
                            reason: "too many transactions before the session was created"
                                .into(),
                            disconnect: true,
                        });
                    }
                    peer.early.push(transaction);
                    return Ok(());
                }
                if transaction.origin.site != site {
                    return Err("transaction from another site".into());
                }
                let last = s.last_seq.get(&site).copied().unwrap_or(0);
                if transaction.origin.seq <= last {
                    // A resend of something already sequenced (lost ack on reconnect).
                    return Ok(());
                }
                if s.log.len() >= max_log {
                    return Err(Dropped {
                        reason: "session log full (waiting for a snapshot)".into(),
                        disconnect: true,
                    });
                }
                s.last_seq.insert(site, transaction.origin.seq);
                let m = Arc::new(CollabMessage::Transaction { transaction });
                s.log.push(m.clone());
                s.broadcast(None, &m, out);
                // Ask for a snapshot past `compact_after`; ask another peer if the last one
                // didn't answer within another `compact_after` transactions.
                let len = s.log.len() as u64;
                let stale = s
                    .compacting
                    .is_some_and(|(_, at)| len >= at + compact_after as u64);
                if s.log.len() > compact_after && (s.compacting.is_none() || stale) {
                    let skip = s.compacting.map(|(c, _)| c);
                    // Any ready peer can answer; prefer the oldest connection.
                    if let Some((id, p)) = s
                        .peers
                        .iter()
                        .find(|(id, p)| p.ready && Some(**id) != skip)
                        .or_else(|| s.peers.iter().find(|(_, p)| p.ready))
                    {
                        s.compacting = Some((*id, len));
                        out.push((
                            *id,
                            Arc::new(CollabMessage::SyncRequest {
                                site: p.site.expect("ready peers have a site"),
                                version: Base64Bytes(Vec::new()),
                            }),
                        ));
                    }
                }
                Ok(())
            }
            CollabMessage::Presence { presence } => {
                let site = peer.site.ok_or("presence before hello")?;
                let m = Arc::new(CollabMessage::Presence {
                    presence: Presence {
                        site,
                        color: peer.color,
                        name: presence.name.chars().take(64).collect(),
                        actor: presence.actor,
                        state: presence.state,
                    },
                });
                peer.presence = Some(m.clone());
                if peer.ready {
                    s.broadcast(Some(conn), &m, out);
                }
                Ok(())
            }
            CollabMessage::Leave { .. } => {
                // Same as a disconnect; the transport closes the socket afterwards.
                self.disconnect(conn, out);
                Ok(())
            }
            m @ CollabMessage::Media { .. } => {
                if !(peer.ready || peer.creator) {
                    return Err("media before sync".into());
                }
                let CollabMessage::Media {
                    file,
                    offset,
                    data,
                    total,
                    ..
                } = &m
                else {
                    unreachable!()
                };
                if offset.saturating_add(data.0.len() as u64) > *total {
                    return Err("media chunk past its total".into());
                }
                let key = (file.clone(), *offset);
                let len = data.0.len();
                let m = Arc::new(m);
                let cache = !s.media_keys.contains(&key)
                    && s.media_bytes + len <= max_media_bytes
                    && total_media + len <= max_total_media_bytes;
                if cache {
                    s.media_keys.insert(key);
                    s.media_bytes += len;
                    s.media.push(MediaChunk {
                        at: s.end(),
                        message: m.clone(),
                    });
                }
                s.broadcast(Some(conn), &m, out);
                if cache {
                    self.media_bytes += len;
                }
                Ok(())
            }
            CollabMessage::Update { .. } => Err("CRDT updates are not used".into()),
            CollabMessage::Pointer { pointer, .. } => {
                let site = peer.site.ok_or("pointer before hello")?;
                if !peer.ready {
                    return Ok(());
                }
                let m = Arc::new(CollabMessage::Pointer { site, pointer });
                s.broadcast(Some(conn), &m, out);
                Ok(())
            }
            CollabMessage::IceServers { .. } => Err("ICE servers come from the relay".into()),
            m @ (CollabMessage::Signal { .. }
            | CollabMessage::Listen { .. }
            | CollabMessage::Unlisten { .. }
            | CollabMessage::TransportRequest { .. }
            | CollabMessage::StreamClock { .. }) => {
                // Site to site: only between synced members of this session, from the
                // sender's own site, delivered to the target only (docs/COLLAB.md §9.7).
                let site = peer.site.ok_or("signal before hello")?;
                if !peer.ready {
                    return Err("signal before sync".into());
                }
                let (from, target) = m.route().expect("routed variants");
                if from != site {
                    return Err("signal from another site".into());
                }
                if target == site {
                    return Err("signal to itself".into());
                }
                check_routed(&m)?;
                let (id, _) = s
                    .peers
                    .iter()
                    .find(|(_, p)| p.ready && p.site == Some(target))
                    .ok_or("signal target is not in the session")?;
                out.push((*id, Arc::new(m)));
                Ok(())
            }
        }
    }

    /// Send `conn` what it is missing since `from` (media, snapshot, log), then the other
    /// peers, and mark it ready (it receives broadcasts from now on).
    fn answer_sync(
        s: &mut Session,
        conn: ConnId,
        from: Option<(u64, u64)>,
        out: &mut Vec<Outgoing>,
    ) {
        // Resume only within this incarnation of the session and the kept log.
        let resume = from
            .filter(|&(e, i)| e == s.epoch && i >= s.base && i <= s.end())
            .map(|(_, i)| i);
        let media_from = resume.unwrap_or(0);
        for c in &s.media {
            if c.at >= media_from {
                out.push((conn, c.message.clone()));
            }
        }
        let start = match resume {
            Some(i) => (i - s.base) as usize,
            None => {
                if let Some(snap) = &s.snapshot {
                    out.push((conn, snap.clone()));
                }
                0
            }
        };
        for m in &s.log[start..] {
            out.push((conn, m.clone()));
        }
        if let Some(p) = s.peers.get_mut(&conn) {
            p.waiting = None;
            p.ready = true;
        }
        Self::introduce(s, conn, out);
    }

    /// Exchange `Hello`/`Presence` between `conn` and the other ready peers.
    fn introduce(s: &Session, conn: ConnId, out: &mut Vec<Outgoing>) {
        let Some(me) = s.peers.get(&conn) else { return };
        for (id, p) in &s.peers {
            if *id == conn || !p.ready || p.site == me.site {
                continue;
            }
            if let Some(h) = &p.hello {
                out.push((conn, h.clone()));
            }
            out.push((
                conn,
                p.presence.clone().unwrap_or_else(|| default_presence(p)),
            ));
            if let Some(h) = &me.hello {
                out.push((*id, h.clone()));
            }
            out.push((
                *id,
                me.presence.clone().unwrap_or_else(|| default_presence(me)),
            ));
        }
    }
}

/// Presence of a peer that has not published any yet (so others learn its color).
fn default_presence(p: &Peer) -> Arc<CollabMessage> {
    let (site, name, actor) = match p.hello.as_deref() {
        Some(CollabMessage::Hello {
            site, name, actor, ..
        }) => (*site, name.clone(), actor.clone()),
        _ => (p.site.unwrap_or(SiteId(0)), String::new(), None),
    };
    Arc::new(CollabMessage::Presence {
        presence: Presence {
            site,
            actor,
            name,
            color: p.color,
            state: PresenceState::default(),
        },
    })
}

#[cfg(test)]
mod tests;

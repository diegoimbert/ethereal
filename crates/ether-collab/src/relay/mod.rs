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
//! - `Presence` is stamped with the sender's site and color; `Hello`, `Presence` and `Leave`
//!   are forwarded to the other ready peers.
//! - Past [`RelayConfig::compact_after`] log entries, one ready peer is asked for a snapshot
//!   (`SyncRequest { site: it, version: [] }`); the log before its index is dropped.

#[cfg(not(target_arch = "wasm32"))]
pub mod server;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use ether_protocol::collab::{CollabMessage, Presence, PresenceState};
use ether_protocol::model::{Base64Bytes, Color, SiteId};

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
}

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
        let name = self.conns.get(&conn).ok_or("unknown connection")?.clone();
        let RelayConfig {
            compact_after,
            max_log,
            max_media_bytes,
            max_total_media_bytes,
            ..
        } = self.config;
        let total_media = self.media_bytes;
        let s = self.sessions.get_mut(&name).ok_or("unknown session")?;
        // A site id is held by one live connection at a time (no impersonation; a
        // reconnecting site's old connection is closed before it says hello again).
        let taken = match &message {
            CollabMessage::Hello { site, .. } => s
                .peers
                .iter()
                .any(|(id, p)| *id != conn && p.site == Some(*site)),
            _ => false,
        };
        let peer = s.peers.get_mut(&conn).ok_or("unknown peer")?;
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
                if taken {
                    return Err(Dropped {
                        reason: "this site is already connected to the session".into(),
                        disconnect: true,
                    });
                }
                if protocol_version != crate::wire::COLLAB_PROTOCOL_VERSION {
                    return Err(Dropped {
                        reason: format!("unsupported collab protocol {protocol_version}"),
                        disconnect: true,
                    });
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
                    return Err("transaction before sync".into());
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

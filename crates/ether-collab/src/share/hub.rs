//! The share hub (docs/SHARING.md §2): the relay's [`Relay`] state machine run inside the
//! host's app, for one session ([`SHARE_SESSION`]). The host's own site joins it over an
//! in-process loopback link ([`Hub::loopback`], delivered at once), and every joiner over the
//! data channel of its pairing ([`Hub::add_peer`], after the handshake) carrying ordinary
//! `CollabMessage` frames ([`crate::wire::encode_frame`], fragmented by the link).
//!
//! On top of the relay it enforces (§2.3):
//! - **roles**: a `Listen` connection may only say hello, sync, publish presence and
//!   pointers, leave, ask to listen, signal, request transport, and chat (transactions made
//!   only of chat ops). Anything else closes it;
//! - **identity**: each joiner's `Hello`/`Presence` name is the authenticated one, and a
//!   site id belongs to the member that first used it in this hub (another member cannot
//!   claim it, so a contested site id is always the same member reconnecting: its old link
//!   is not pinged and is dropped after the relay's probe);
//! - **the host is the snapshot source** (creation and compaction, [`Relay::set_snapshot_source`]);
//! - **colours**: a joiner's preferred colour when free ([`Relay::set_color`]);
//! - **backpressure**: a link with more than [`HubConfig::max_buffered`] bytes queued is
//!   closed like a slow reader (it resyncs when it reconnects);
//! - **ICE servers**: the signaling service's (or the settings'), advertised to every site as
//!   by a relay ([`Hub::set_ice_servers`]).
//!
//! Wasm-safe, no threads: the controller calls [`Hub::poll`] from its tick.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use ether_protocol::collab::{CollabMessage, IceServer};
use ether_protocol::model::{Color, Entity, EntityKey, Op, SiteId};
use ether_protocol::share::{MAX_PARTICIPANTS, MemberId, ParticipantRole, ShareRole};

use super::BoxPeerLink;
use crate::relay::{ConnId, Outgoing, Relay, RelayConfig};
use crate::wire::{WireFrame, decode_binary, decode_text, encode_frame};
use crate::{BoxTransport, CollabTransport, ConnectRequest, Connector, LinkState};

/// The hub's single session name.
pub const SHARE_SESSION: &str = "share";
/// Default byte budget of a joiner link (a 16 MiB snapshot plus media in flight).
pub const MAX_BUFFERED_BYTES: usize = 64 << 20;

#[derive(Clone, Debug, PartialEq)]
pub struct HubConfig {
    pub relay: RelayConfig,
    pub max_buffered: usize,
}

impl Default for HubConfig {
    fn default() -> Self {
        Self {
            relay: RelayConfig {
                max_sessions: 1,
                max_sites_per_session: MAX_PARTICIPANTS,
                ..RelayConfig::default()
            },
            max_buffered: MAX_BUFFERED_BYTES,
        }
    }
}

/// A connection of the hub, for the participant list.
#[derive(Clone, Debug, PartialEq)]
pub struct HubConn {
    pub conn: ConnId,
    pub role: ParticipantRole,
    /// `None` for the host.
    pub member: Option<MemberId>,
    pub name: String,
    pub color: Color,
    /// Its site, once it said hello.
    pub site: Option<SiteId>,
}

/// A joiner's connection closed.
#[derive(Clone, Debug, PartialEq)]
pub struct HubLeft {
    pub member: Option<MemberId>,
    pub name: String,
    pub site: Option<SiteId>,
    pub reason: String,
}

struct Conn {
    /// `None`: the host's loopback.
    link: Option<BoxPeerLink>,
    role: ParticipantRole,
    member: Option<MemberId>,
    name: String,
    site: Option<SiteId>,
}

struct Inner {
    relay: Relay,
    config: HubConfig,
    next: ConnId,
    conns: BTreeMap<ConnId, Conn>,
    /// Messages for loopback connections (polled by the host's collab session).
    loop_out: HashMap<ConnId, VecDeque<CollabMessage>>,
    /// Closed loopback connections, with why.
    loop_closed: HashMap<ConnId, String>,
    /// Who owns each site id seen in this hub (`None`: the host).
    pins: HashMap<SiteId, Option<MemberId>>,
    left: Vec<HubLeft>,
    ice: Arc<Mutex<Vec<IceServer>>>,
}

/// May a `Listen` connection send this? (docs/SHARING.md §2.3)
fn listen_allows(m: &CollabMessage) -> bool {
    match m {
        CollabMessage::Hello { .. }
        | CollabMessage::SyncRequest { .. }
        | CollabMessage::Presence { .. }
        | CollabMessage::Pointer { .. }
        | CollabMessage::Leave { .. }
        | CollabMessage::Listen { .. }
        | CollabMessage::Unlisten { .. }
        | CollabMessage::Signal { .. }
        | CollabMessage::TransportRequest { .. } => true,
        // Chat only (receivers still apply the chat sanitize rules, COLLAB.md §12.1).
        CollabMessage::Transaction { transaction } => {
            !transaction.transaction.ops.is_empty()
                && transaction.transaction.ops.iter().all(|op| match op {
                    Op::Insert {
                        entity: Entity::ChatMessage(_),
                    } => true,
                    Op::Remove {
                        key: EntityKey::ChatMessage(_),
                    } => true,
                    _ => false,
                })
        }
        CollabMessage::Media { .. }
        | CollabMessage::Snapshot { .. }
        | CollabMessage::Update { .. }
        | CollabMessage::StreamClock { .. }
        | CollabMessage::IceServers { .. } => false,
    }
}

impl Inner {
    fn process(&mut self, conn: ConnId, m: CollabMessage) {
        let Some(c) = self.conns.get(&conn) else {
            return;
        };
        if c.role == ParticipantRole::Listen && !listen_allows(&m) {
            self.close(conn, "view only: this connection cannot edit".into());
            return;
        }
        let peer = c.link.is_some();
        let (member, pinned_name) = (c.member.clone(), c.name.clone());
        let m = match m {
            CollabMessage::Hello {
                site,
                actor,
                name,
                protocol_version,
            } => {
                if self.pins.get(&site).is_some_and(|owner| *owner != member) {
                    self.close(conn, "this site id belongs to someone else".into());
                    return;
                }
                let c = self.conns.get_mut(&conn).expect("checked");
                if c.site.is_none() {
                    c.site = Some(site);
                    self.pins.insert(site, member);
                }
                CollabMessage::Hello {
                    site,
                    actor,
                    name: if peer { pinned_name } else { name },
                    protocol_version,
                }
            }
            CollabMessage::Presence { mut presence } if peer => {
                presence.name = pinned_name;
                CollabMessage::Presence { presence }
            }
            m => m,
        };
        let leave = matches!(m, CollabMessage::Leave { .. });
        let mut out = Vec::new();
        let r = self.relay.message(conn, m, &mut out);
        self.route(out);
        self.after_relay();
        if leave {
            self.close(conn, "left".into());
        } else if let Err(e) = r
            && e.disconnect
        {
            self.close(conn, e.reason);
        }
    }

    fn route(&mut self, out: Vec<Outgoing>) {
        // A broadcast is encoded once.
        let mut frames: HashMap<*const CollabMessage, WireFrame> = HashMap::new();
        for (c, m) in out {
            let Some(conn) = self.conns.get_mut(&c) else {
                continue;
            };
            match conn.link.as_mut() {
                None => self.loop_out.entry(c).or_default().push_back((*m).clone()),
                Some(link) => {
                    let frame = frames
                        .entry(Arc::as_ptr(&m))
                        .or_insert_with(|| encode_frame(&m));
                    link.send(frame);
                }
            }
        }
    }

    /// Pings: the loopback answers at once; joiner links never do (a contested site id is
    /// the same member reconnecting, see the module docs). Closing: what the relay dropped.
    fn after_relay(&mut self) {
        loop {
            let pings = self.relay.take_pings();
            let closing = self.relay.take_closing();
            if pings.is_empty() && closing.is_empty() {
                return;
            }
            for c in pings {
                if self.conns.get(&c).is_some_and(|c| c.link.is_none()) {
                    self.relay.heard(c);
                }
            }
            for (c, reason) in closing {
                self.close(c, reason);
            }
        }
    }

    fn close(&mut self, conn: ConnId, reason: String) {
        let Some(c) = self.conns.remove(&conn) else {
            return;
        };
        let mut out = Vec::new();
        self.relay.disconnect(conn, &mut out);
        self.route(out);
        self.after_relay();
        match c.link {
            Some(mut link) => {
                link.close();
                self.left.push(HubLeft {
                    member: c.member,
                    name: c.name,
                    site: c.site,
                    reason,
                });
            }
            None => {
                self.loop_out.remove(&conn);
                self.loop_closed.insert(conn, reason);
            }
        }
    }

    fn connect(
        &mut self,
        link: Option<BoxPeerLink>,
        role: ParticipantRole,
        member: Option<MemberId>,
        name: String,
        color: Option<Color>,
    ) -> Result<ConnId, (Option<BoxPeerLink>, String)> {
        let conn = self.next;
        self.next += 1;
        if let Err(e) = self.relay.connect(conn, SHARE_SESSION) {
            return Err((link, e.to_string()));
        }
        if let Some(color) = color {
            self.relay.set_color(conn, color);
        }
        self.conns.insert(
            conn,
            Conn {
                link,
                role,
                member,
                name,
                site: None,
            },
        );
        Ok(conn)
    }

    fn poll_link(&mut self, id: ConnId) {
        let max = self.config.max_buffered;
        let Some(link) = self.conns.get_mut(&id).and_then(|c| c.link.as_mut()) else {
            return;
        };
        if link.buffered() > max {
            self.close(id, "the connection is too slow (it will resync)".into());
            return;
        }
        let mut frames = Vec::new();
        link.poll(&mut frames);
        let state = link.state();
        for f in frames {
            let m = match f {
                WireFrame::Text(t) => decode_text(&t),
                WireFrame::Binary(b) => decode_binary(&b),
            };
            match m {
                Ok(m) => self.process(id, m),
                Err(e) => {
                    self.close(id, e);
                    return;
                }
            }
            if !self.conns.contains_key(&id) {
                return;
            }
        }
        if let LinkState::Closed { reason, .. } = state {
            self.close(id, reason);
        }
    }
}

/// See the module docs. Cheap to clone (shared state: the host's loopback transport holds
/// one).
#[derive(Clone)]
pub struct Hub {
    inner: Arc<Mutex<Inner>>,
}

impl Default for Hub {
    fn default() -> Self {
        Self::new(HubConfig::default())
    }
}

impl Hub {
    pub fn new(config: HubConfig) -> Self {
        let ice = Arc::new(Mutex::new(Vec::new()));
        let mut relay = Relay::new(config.relay.clone());
        let servers = ice.clone();
        relay.set_ice_provider(Box::new(move |_, _| {
            servers.lock().map(|s| s.clone()).unwrap_or_default()
        }));
        Self {
            inner: Arc::new(Mutex::new(Inner {
                relay,
                config,
                next: 1,
                conns: BTreeMap::new(),
                loop_out: HashMap::new(),
                loop_closed: HashMap::new(),
                pins: HashMap::new(),
                left: Vec::new(),
                ice,
            })),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().expect("share hub lock")
    }

    /// The ICE servers advertised to sites (from now on; refreshed per the relay's
    /// `ice_refresh_ms`).
    pub fn set_ice_servers(&self, servers: Vec<IceServer>) {
        let h = self.lock();
        if let Ok(mut s) = h.ice.lock() {
            *s = servers;
        }
    }

    /// The host's own site: a loopback connection (role `Host`, the snapshot source).
    pub fn loopback(&self, name: &str, color: Option<Color>) -> LoopbackLink {
        let mut h = self.lock();
        let conn = match h.connect(None, ParticipantRole::Host, None, name.to_string(), color) {
            Ok(conn) => {
                h.relay.set_snapshot_source(conn);
                conn
            }
            Err((_, reason)) => {
                let conn = h.next;
                h.next += 1;
                h.loop_closed.insert(conn, reason);
                conn
            }
        };
        LoopbackLink {
            hub: self.clone(),
            conn,
        }
    }

    /// A collab [`Connector`] handing out loopback links (the host's session).
    pub fn connector(&self, name: &str, color: Option<Color>) -> Connector {
        let hub = self.clone();
        let name = name.to_string();
        Box::new(move |_: &ConnectRequest| -> BoxTransport { Box::new(hub.loopback(&name, color)) })
    }

    /// An authenticated joiner (after `Accept`). `Err` (session full): the link is handed
    /// back so the caller can refuse it.
    pub fn add_peer(
        &self,
        link: BoxPeerLink,
        role: ShareRole,
        member: MemberId,
        name: &str,
        color: Option<Color>,
    ) -> Result<ConnId, (BoxPeerLink, String)> {
        let role = match role {
            ShareRole::Edit => ParticipantRole::Edit,
            ShareRole::Listen => ParticipantRole::Listen,
        };
        self.lock()
            .connect(Some(link), role, Some(member), name.to_string(), color)
            .map_err(|(link, e)| (link.expect("a peer link"), e))
    }

    /// Frames `conn` already sent (read from its link with the handshake's last frame).
    pub fn deliver(&self, conn: ConnId, frames: Vec<WireFrame>) {
        let mut h = self.lock();
        for f in frames {
            if !h.conns.contains_key(&conn) {
                return;
            }
            let m = match f {
                WireFrame::Text(t) => decode_text(&t),
                WireFrame::Binary(b) => decode_binary(&b),
            };
            match m {
                Ok(m) => h.process(conn, m),
                Err(e) => h.close(conn, e),
            }
        }
    }

    /// Drive the hub: joiners' frames, closed links, the relay's clock.
    pub fn poll(&self, now_ms: u64) {
        let mut h = self.lock();
        let mut out = Vec::new();
        h.relay.tick(now_ms, &mut out);
        h.route(out);
        h.after_relay();
        let ids: Vec<ConnId> = h
            .conns
            .iter()
            .filter(|(_, c)| c.link.is_some())
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            h.poll_link(id);
        }
    }

    /// Close every connection of `member` (removed, role changed).
    pub fn close_member(&self, member: &str, reason: &str) {
        let mut h = self.lock();
        let ids: Vec<ConnId> = h
            .conns
            .iter()
            .filter(|(_, c)| c.member.as_deref() == Some(member))
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            h.close(id, reason.to_string());
        }
    }

    /// Close every joiner (stop sharing, project closed). The loopback stays.
    pub fn close_peers(&self, reason: &str) {
        let mut h = self.lock();
        let ids: Vec<ConnId> = h
            .conns
            .iter()
            .filter(|(_, c)| c.link.is_some())
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            h.close(id, reason.to_string());
        }
        h.left.clear();
    }

    /// The connections, oldest first.
    pub fn conns(&self) -> Vec<HubConn> {
        let h = self.lock();
        h.conns
            .iter()
            .map(|(id, c)| HubConn {
                conn: *id,
                role: c.role,
                member: c.member.clone(),
                name: c.name.clone(),
                color: h.relay.color(*id).unwrap_or(Color(0)),
                site: c.site,
            })
            .collect()
    }

    /// Joiner connections that closed since the last call.
    pub fn take_left(&self) -> Vec<HubLeft> {
        std::mem::take(&mut self.lock().left)
    }

    /// Run `f` on the relay state (tests, diagnostics).
    pub fn with_relay<T>(&self, f: impl FnOnce(&Relay) -> T) -> T {
        f(&self.lock().relay)
    }

    /// Sites currently connected, with their owners (diagnostics).
    pub fn sites(&self) -> HashSet<SiteId> {
        self.lock().conns.values().filter_map(|c| c.site).collect()
    }
}

/// The host's own site link to its [`Hub`]: messages are delivered at once.
pub struct LoopbackLink {
    hub: Hub,
    conn: ConnId,
}

impl CollabTransport for LoopbackLink {
    fn send(&mut self, message: &CollabMessage) {
        self.hub.lock().process(self.conn, message.clone());
    }

    fn poll(&mut self, out: &mut Vec<CollabMessage>) {
        if let Some(q) = self.hub.lock().loop_out.get_mut(&self.conn) {
            out.extend(q.drain(..));
        }
    }

    fn state(&self) -> LinkState {
        let h = self.hub.lock();
        if h.conns.contains_key(&self.conn) {
            LinkState::Open
        } else {
            LinkState::Closed {
                reason: h
                    .loop_closed
                    .get(&self.conn)
                    .cloned()
                    .unwrap_or_else(|| "closed".into()),
                fatal: false,
            }
        }
    }

    fn close(&mut self) {
        self.hub.lock().close(self.conn, "closed".into());
    }
}

impl Drop for LoopbackLink {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::share::fake::pipe;
    use crate::wire::{COLLAB_PROTOCOL_VERSION, SnapshotData};
    use ether_protocol::model::{
        Base64Bytes, ChatMessageId, OpOrigin, StampedTransaction, Transaction,
    };
    use std::collections::BTreeMap as Map;

    fn hello(site: u64, name: &str) -> CollabMessage {
        CollabMessage::Hello {
            site: SiteId(site),
            actor: None,
            name: name.into(),
            protocol_version: COLLAB_PROTOCOL_VERSION,
        }
    }

    fn sync(site: u64) -> CollabMessage {
        CollabMessage::SyncRequest {
            site: SiteId(site),
            version: Base64Bytes(vec![]),
        }
    }

    fn tx(site: u64, seq: u64, ops: Vec<Op>) -> CollabMessage {
        CollabMessage::Transaction {
            transaction: StampedTransaction {
                origin: OpOrigin {
                    site: SiteId(site),
                    actor: None,
                    seq,
                },
                transaction: Transaction {
                    label: "t".into(),
                    ops,
                },
            },
        }
    }

    /// A hub whose host (site 1) created the session.
    fn hosted() -> (Hub, LoopbackLink) {
        let hub = Hub::default();
        let mut host = hub.loopback("Diego", Some(Color(0x111111)));
        host.send(&hello(1, "Diego"));
        host.send(&sync(1));
        let mut got = Vec::new();
        host.poll(&mut got);
        assert!(matches!(
            got.as_slice(),
            [CollabMessage::SyncRequest { .. }]
        ));
        host.send(&CollabMessage::Snapshot {
            data: SnapshotData {
                epoch: 1,
                index: 0,
                sites: Map::new(),
                ether: "{}".into(),
            }
            .encode(),
        });
        host.poll(&mut got);
        (hub, host)
    }

    /// The joiner's end of a pipe: send messages, receive what the hub sent.
    struct Joiner(BoxPeerLink);

    impl Joiner {
        fn send(&mut self, m: &CollabMessage) {
            self.0.send(&encode_frame(m));
        }
        fn recv(&mut self) -> Vec<CollabMessage> {
            let mut frames = Vec::new();
            self.0.poll(&mut frames);
            frames
                .into_iter()
                .map(|f| match f {
                    WireFrame::Text(t) => decode_text(&t).unwrap(),
                    WireFrame::Binary(b) => decode_binary(&b).unwrap(),
                })
                .collect()
        }
    }

    fn join(hub: &Hub, role: ShareRole, member: &str, name: &str, site: u64) -> Joiner {
        let (a, b) = pipe();
        hub.add_peer(b, role, member.into(), name, None)
            .ok()
            .unwrap();
        let mut j = Joiner(a);
        j.send(&hello(site, "Mallory"));
        j.send(&sync(site));
        hub.poll(0);
        j
    }

    /// A chat prune (chat ops only; receivers sanitize them).
    fn chat() -> Op {
        Op::Remove {
            key: EntityKey::ChatMessage(ChatMessageId::NIL),
        }
    }

    #[test]
    fn joiners_sync_through_the_hub_with_pinned_names() {
        let (hub, mut host) = hosted();
        let mut ada = join(&hub, ShareRole::Edit, "m-ada", "Ada", 2);
        let got = ada.recv();
        assert!(matches!(got.first(), Some(CollabMessage::Snapshot { .. })));
        // The host sees Ada by her authenticated name, not the one she claimed.
        let mut h = Vec::new();
        host.poll(&mut h);
        assert!(h.iter().any(
            |m| matches!(m, CollabMessage::Hello { name, site, .. } if name == "Ada" && *site == SiteId(2))
        ));
        // Edits flow both ways.
        ada.send(&tx(2, 1, vec![]));
        hub.poll(0);
        h.clear();
        host.poll(&mut h);
        assert!(
            h.iter()
                .any(|m| matches!(m, CollabMessage::Transaction { .. }))
        );
        assert!(
            ada.recv()
                .iter()
                .any(|m| matches!(m, CollabMessage::Transaction { .. })),
            "echo"
        );
        let conns = hub.conns();
        assert_eq!(conns.len(), 2);
        assert_eq!(conns[1].member.as_deref(), Some("m-ada"));
        assert_eq!(conns[1].site, Some(SiteId(2)));
    }

    #[test]
    fn listen_connections_chat_but_cannot_edit() {
        let (hub, mut host) = hosted();
        let mut tom = join(&hub, ShareRole::Listen, "m-tom", "Tom", 3);
        tom.recv();
        tom.send(&tx(3, 1, vec![chat()]));
        hub.poll(0);
        let mut h = Vec::new();
        host.poll(&mut h);
        assert!(
            h.iter()
                .any(|m| matches!(m, CollabMessage::Transaction { .. })),
            "chat passes"
        );
        tom.send(&tx(3, 2, vec![]));
        hub.poll(0);
        assert_eq!(hub.conns().len(), 1, "an edit closes the listen link");
        let left = hub.take_left();
        assert_eq!(left[0].member.as_deref(), Some("m-tom"));
        assert!(left[0].reason.contains("view only"));
    }

    #[test]
    fn listen_connections_are_never_asked_for_snapshots() {
        let hub = Hub::default();
        // The host's loopback is connected first (it has not said hello yet).
        let mut host = hub.loopback("Diego", None);
        let (a, b) = pipe();
        hub.add_peer(b, ShareRole::Listen, "m".into(), "Tom", None)
            .ok()
            .unwrap();
        let mut tom = Joiner(a);
        tom.send(&hello(3, "Tom"));
        tom.send(&sync(3));
        hub.poll(0);
        assert!(tom.recv().is_empty(), "waits for the host");
        host.send(&hello(1, "Diego"));
        host.send(&sync(1));
        let mut h = Vec::new();
        host.poll(&mut h);
        assert!(
            matches!(h.as_slice(), [CollabMessage::SyncRequest { .. }]),
            "the host creates"
        );
    }

    #[test]
    fn a_member_cannot_take_another_ones_site() {
        let (hub, _host) = hosted();
        let mut ada = join(&hub, ShareRole::Edit, "m-ada", "Ada", 2);
        ada.recv();
        // Tom claims the host's site, then Ada's.
        let tom = join(&hub, ShareRole::Edit, "m-tom", "Tom", 1);
        assert_eq!(
            tom.0.state(),
            LinkState::Closed {
                reason: "closed".into(),
                fatal: false
            }
        );
        let left = hub.take_left();
        assert!(left[0].reason.contains("someone else"), "{left:?}");
        join(&hub, ShareRole::Edit, "m-tom", "Tom", 2);
        assert_eq!(hub.conns().len(), 2);
        // Ada reconnecting with her own site takes over (no ping to the old link).
        let _ada2 = join(&hub, ShareRole::Edit, "m-ada", "Ada", 2);
        hub.poll(10_000);
        let members: Vec<_> = hub.conns().into_iter().filter_map(|c| c.member).collect();
        assert_eq!(
            members,
            ["m-ada"],
            "the old link was dropped after the probe"
        );
    }

    #[test]
    fn preferred_colours_and_ice_servers() {
        let (hub, _host) = hosted();
        hub.set_ice_servers(vec![IceServer {
            urls: vec!["stun:stun.example.org:3478".into()],
            username: None,
            credential: None,
        }]);
        let (a, b) = pipe();
        hub.add_peer(b, ShareRole::Edit, "m".into(), "Ada", Some(Color(0x111111)))
            .ok()
            .unwrap();
        let (c, d) = pipe();
        hub.add_peer(d, ShareRole::Edit, "n".into(), "Tom", Some(Color(0xabcdef)))
            .ok()
            .unwrap();
        let conns = hub.conns();
        assert_ne!(conns[1].color, Color(0x111111), "taken by the host");
        assert_eq!(conns[2].color, Color(0xabcdef));
        let mut ada = Joiner(a);
        ada.send(&hello(2, "Ada"));
        ada.send(&sync(2));
        hub.poll(0);
        assert!(
            ada.recv()
                .iter()
                .any(|m| matches!(m, CollabMessage::IceServers { .. }))
        );
        drop(c);
    }

    #[test]
    fn slow_and_closed_links_leave() {
        let hub = Hub::new(HubConfig {
            max_buffered: 0,
            ..HubConfig::default()
        });
        let mut host = hub.loopback("Diego", None);
        host.send(&hello(1, "Diego"));
        host.send(&sync(1));
        let (a, b) = pipe();
        hub.add_peer(b, ShareRole::Edit, "m".into(), "Ada", None)
            .ok()
            .unwrap();
        let mut ada = Joiner(a);
        ada.send(&hello(2, "Ada"));
        hub.poll(0);
        assert_eq!(hub.conns().len(), 2, "nothing buffered yet");
        // The fake pipe reports what the other side has not read as buffered.
        ada.send(&sync(2));
        host.send(&CollabMessage::Snapshot {
            data: SnapshotData {
                epoch: 1,
                index: 0,
                sites: Map::new(),
                ether: "{}".into(),
            }
            .encode(),
        });
        hub.poll(0);
        hub.poll(0);
        assert_eq!(hub.conns().len(), 1, "too slow");
        assert!(hub.take_left()[0].reason.contains("too slow"));
        // Stopping closes every joiner, not the host.
        let (_c, d) = pipe();
        hub.add_peer(d, ShareRole::Edit, "n".into(), "Tom", None)
            .ok()
            .unwrap();
        hub.close_peers("stopped");
        assert_eq!(hub.conns().len(), 1);
        assert_eq!(host.state(), LinkState::Open);
    }
}

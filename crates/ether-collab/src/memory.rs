//! An in-process relay for tests and simulations: the real [`Relay`] state machine with
//! manual (or automatic) delivery, so tests control how sites' messages interleave.
//!
//! Messages a site sends wait in the hub's inbox until [`Hub::deliver`] (or immediately
//! with [`Hub::set_auto`]); what the relay sends waits in each link's queue until the site
//! polls. Several sites committing before a `deliver` are concurrent edits.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use ether_protocol::collab::CollabMessage;

use crate::relay::{ConnId, Relay, RelayConfig};
use crate::{BoxTransport, CollabTransport, ConnectRequest, Connector, LinkState};

#[derive(Default)]
struct Inner {
    relay: Relay,
    next: ConnId,
    inbox: VecDeque<(ConnId, CollabMessage)>,
    outbox: HashMap<ConnId, VecDeque<CollabMessage>>,
    closed: HashMap<ConnId, (String, bool)>,
    /// Links that are open (not closed/killed).
    open: HashSet<ConnId>,
    auto: bool,
    /// Every message delivered to the relay, in order (for assertions).
    delivered: usize,
}

impl Inner {
    fn process(&mut self, conn: ConnId, m: CollabMessage) {
        if !self.open.contains(&conn) {
            return;
        }
        self.delivered += 1;
        let leave = matches!(m, CollabMessage::Leave { .. });
        let mut out = Vec::new();
        let r = self.relay.message(conn, m, &mut out);
        self.route(out);
        if leave {
            self.close(conn, "left".into(), false);
        } else if let Err(e) = r
            && e.disconnect
        {
            self.close(conn, e.reason, false);
        }
    }

    fn route(&mut self, out: Vec<(ConnId, Arc<CollabMessage>)>) {
        for (c, m) in out {
            if self.open.contains(&c) {
                self.outbox.entry(c).or_default().push_back((*m).clone());
            }
        }
    }

    fn close(&mut self, conn: ConnId, reason: String, fatal: bool) {
        if self.open.remove(&conn) {
            let mut out = Vec::new();
            self.relay.disconnect(conn, &mut out);
            self.route(out);
            self.inbox.retain(|(c, _)| *c != conn);
            self.closed.insert(conn, (reason, fatal));
        }
    }
}

/// See the module docs. Cheap to clone (shared state).
#[derive(Clone)]
pub struct Hub {
    inner: Arc<Mutex<Inner>>,
}

impl Default for Hub {
    fn default() -> Self {
        Self::new(RelayConfig::default())
    }
}

impl Hub {
    pub fn new(config: RelayConfig) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                relay: Relay::new(config),
                next: 1,
                ..Inner::default()
            })),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().expect("hub lock")
    }

    /// Deliver messages as soon as they are sent (no concurrency).
    pub fn set_auto(&self, auto: bool) {
        let mut h = self.lock();
        h.auto = auto;
        if auto {
            while let Some((c, m)) = h.inbox.pop_front() {
                h.process(c, m);
            }
        }
    }

    /// Deliver every queued message to the relay (in send order). Returns how many.
    pub fn deliver(&self) -> usize {
        let mut h = self.lock();
        let mut n = 0;
        while let Some((c, m)) = h.inbox.pop_front() {
            h.process(c, m);
            n += 1;
        }
        n
    }

    /// Deliver the oldest queued message of `conn` only. Returns false if none.
    pub fn deliver_from(&self, conn: ConnId) -> bool {
        let mut h = self.lock();
        let Some(i) = h.inbox.iter().position(|(c, _)| *c == conn) else {
            return false;
        };
        let (c, m) = h.inbox.remove(i).expect("index checked");
        h.process(c, m);
        true
    }

    /// Messages waiting to reach the relay.
    pub fn queued(&self) -> usize {
        self.lock().inbox.len()
    }

    /// Messages delivered to the relay so far.
    pub fn delivered(&self) -> usize {
        self.lock().delivered
    }

    /// Messages waiting for `conn` to poll them.
    pub fn outgoing(&self, conn: ConnId) -> usize {
        self.lock().outbox.get(&conn).map_or(0, VecDeque::len)
    }

    /// Simulate a dropped connection (the site sees `Closed { fatal: false }`).
    pub fn kill(&self, conn: ConnId) {
        self.lock().close(conn, "connection lost".into(), false);
    }

    /// Open links, oldest first.
    pub fn links(&self) -> Vec<ConnId> {
        let mut v: Vec<ConnId> = self.lock().open.iter().copied().collect();
        v.sort_unstable();
        v
    }

    /// Run `f` on the relay state.
    pub fn with_relay<T>(&self, f: impl FnOnce(&Relay) -> T) -> T {
        f(&self.lock().relay)
    }

    /// A new link to `request.session` (what [`Hub::connector`] hands out).
    pub fn link(&self, request: &ConnectRequest) -> MemoryLink {
        let mut h = self.lock();
        let conn = h.next;
        h.next += 1;
        match h.relay.connect(conn, &request.session) {
            Ok(()) => {
                h.open.insert(conn);
            }
            Err(e) => {
                h.closed.insert(conn, (e.to_string(), true));
            }
        }
        MemoryLink {
            hub: self.clone(),
            conn,
        }
    }

    /// A [`Connector`] opening links on this hub.
    pub fn connector(&self) -> Connector {
        let hub = self.clone();
        Box::new(move |r: &ConnectRequest| -> BoxTransport { Box::new(hub.link(r)) })
    }
}

/// One site's link to a [`Hub`].
pub struct MemoryLink {
    hub: Hub,
    conn: ConnId,
}

impl MemoryLink {
    pub fn conn(&self) -> ConnId {
        self.conn
    }
}

impl CollabTransport for MemoryLink {
    fn send(&mut self, message: &CollabMessage) {
        let mut h = self.hub.lock();
        if !h.open.contains(&self.conn) {
            return;
        }
        if h.auto {
            h.process(self.conn, message.clone());
        } else {
            h.inbox.push_back((self.conn, message.clone()));
        }
    }

    fn poll(&mut self, out: &mut Vec<CollabMessage>) {
        if let Some(q) = self.hub.lock().outbox.get_mut(&self.conn) {
            out.extend(q.drain(..));
        }
    }

    fn state(&self) -> LinkState {
        let h = self.hub.lock();
        if h.open.contains(&self.conn) {
            LinkState::Open
        } else {
            let (reason, fatal) = h
                .closed
                .get(&self.conn)
                .cloned()
                .unwrap_or_else(|| ("closed".into(), false));
            LinkState::Closed { reason, fatal }
        }
    }

    fn close(&mut self) {
        let mut h = self.hub.lock();
        // Deliver what this site already sent (a Leave, typically), then close.
        let mine: Vec<CollabMessage> = h
            .inbox
            .iter()
            .filter(|(c, _)| *c == self.conn)
            .map(|(_, m)| m.clone())
            .collect();
        h.inbox.retain(|(c, _)| *c != self.conn);
        for m in mine {
            h.process(self.conn, m);
        }
        h.close(self.conn, "closed".into(), false);
    }
}

impl Drop for MemoryLink {
    fn drop(&mut self) {
        self.close();
    }
}

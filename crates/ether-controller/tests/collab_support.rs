//! Collab test support (shared by `collab.rs` and `collab_relay.rs` through `#[path]`):
//! sites = real controllers with distinct seeds, connected to an in-memory hub or a real
//! relay.

#![allow(dead_code, unused_imports)]

#[path = "common/mod.rs"]
mod common;

pub use common::{Call, FakeBridge, T0, ok, patches, plugin_descriptor, wav};

use std::sync::{Arc, Mutex};

use ether_collab::memory::Hub;
use ether_collab::{
    BoxTransport, CollabMessage, CollabTransport, ConnectRequest, Connector, LinkState,
};
use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::{Controller, ControllerConfig, EtherController, HostServices};
use ether_core::protocol::collab::{CollabCommand, CollabEvent, CollabStatus, Presence};
use ether_core::protocol::model::*;
use ether_core::protocol::project::ProjectCommand;
use ether_core::protocol::*;

pub struct SeededHost {
    pub now: u64,
    pub seed: u64,
}

impl HostServices for SeededHost {
    fn now_ms(&self) -> u64 {
        self.now
    }
    fn random_seed(&mut self) -> u64 {
        self.seed
    }
}

pub type Ctl = EtherController<FakeBridge, SeededHost, MemoryStore, MemoryLibrary>;

pub struct Site {
    pub ctl: Ctl,
    pub ids: IdGen,
    next_request: u32,
    /// Everything the controller sent (events and replies), for assertions.
    pub log: Vec<ServerMessage>,
}

impl Site {
    pub fn new(seed: u64, connector: Connector, library: MemoryLibrary) -> Self {
        let mut store = MemoryStore::new();
        store.now_ms = T0;
        let config = ControllerConfig {
            autosave_after_ms: None,
            ..ControllerConfig::default()
        };
        let mut ctl = EtherController::with_config(
            FakeBridge::default(),
            SeededHost { now: T0, seed },
            store,
            library,
            config,
        );
        ctl.set_collab_connector(connector);
        Self {
            ctl,
            ids: IdGen::new(seed.wrapping_mul(7919)),
            next_request: 1,
            log: Vec::new(),
        }
    }

    pub fn on_hub(seed: u64, hub: &Hub) -> Self {
        Self::new(seed, hub.connector(), MemoryLibrary::new())
    }

    pub fn id<I: Id>(&mut self) -> I {
        self.ids.next(T0)
    }

    pub fn send(&mut self, command: Command) -> Vec<ServerMessage> {
        let id = self.next_request;
        self.next_request += 1;
        let mut out = Vec::new();
        self.ctl.handle(
            ClientMessage {
                id,
                gesture: None,
                command,
            },
            &mut out,
        );
        self.log.extend(out.iter().cloned());
        out
    }

    pub fn ok(&mut self, command: Command) -> ReplyValue {
        let out = self.send(command);
        ok(&out)
    }

    pub fn tick(&mut self) -> Vec<ServerMessage> {
        self.ctl.host.now += 20;
        self.ctl.store.now_ms = self.ctl.host.now;
        let mut out = Vec::new();
        let now = self.ctl.host.now;
        self.ctl.tick(now, &mut out);
        self.log.extend(out.iter().cloned());
        out
    }

    pub fn advance(&mut self, ms: u64) {
        self.ctl.host.now += ms;
    }

    pub fn project(&self) -> &Project {
        self.ctl.project().expect("open project")
    }

    pub fn create_project(&mut self, name: &str) -> ProjectId {
        let id = self.ids.next_project_id(T0);
        self.ok(Command::Project(ProjectCommand::Create {
            id,
            name: name.into(),
        }));
        id
    }

    pub fn join(&mut self, server: &str, session: &str, name: &str, token: Option<&str>) {
        self.ok(Command::Collab(CollabCommand::Join {
            server: server.into(),
            session: session.into(),
            token: token.map(Into::into),
            name: name.into(),
        }));
    }

    /// The last session status and peer list the UI would show.
    pub fn status(&self) -> Option<CollabStatus> {
        self.log.iter().rev().find_map(|m| match m {
            ServerMessage::Event(Event::Collab {
                event: CollabEvent::Session { status },
            }) => Some(status.clone()),
            _ => None,
        })
    }

    pub fn peers(&self) -> Vec<Presence> {
        self.log
            .iter()
            .rev()
            .find_map(|m| match m {
                ServerMessage::Event(Event::Collab {
                    event: CollabEvent::Presence { peers },
                }) => Some(peers.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    pub fn online(&self) -> bool {
        matches!(self.status(), Some(CollabStatus::Online { .. }))
    }

    /// Track ids in display order (top level, then children), non-master.
    pub fn tracks(&self) -> Vec<TrackId> {
        let mut v: Vec<TrackId> = self
            .project()
            .tracks
            .values()
            .filter(|t| t.kind != TrackKind::Master)
            .map(|t| t.id)
            .collect();
        v.sort();
        v
    }
}

/// The project without site-local state (docs/COLLAB.md §2.1), for comparing replicas.
pub fn shared(p: &Project) -> Project {
    let mut p = p.clone();
    let d = ProjectSettings::default();
    p.settings.loop_enabled = d.loop_enabled;
    p.settings.loop_region = d.loop_region;
    p.settings.metronome = d.metronome;
    p.settings.count_in_bars = d.count_in_bars;
    p.settings.metronome_volume = d.metronome_volume;
    p.settings.metronome_accent = d.metronome_accent;
    p.settings.metronome_sound = d.metronome_sound;
    // Mute and solo are per-site (COLLAB.md §2.1).
    for t in p.tracks.values_mut() {
        t.mixer.mute = false;
        t.mixer.solo = false;
    }
    for d in p.drum_pads.values_mut() {
        d.mute = false;
    }
    p
}

/// A wire tap on a site's links: records the transactions it sends, and can add ops to its
/// next one (to act as an old peer that still sends mute/solo).
#[derive(Clone, Default)]
pub struct Tap {
    sent: Arc<Mutex<Vec<StampedTransaction>>>,
    inject: Arc<Mutex<Vec<Op>>>,
}

impl Tap {
    pub fn connector(&self, hub: &Hub) -> Connector {
        let tap = self.clone();
        let mut inner = hub.connector();
        Box::new(move |r: &ConnectRequest| -> BoxTransport {
            Box::new(TapLink {
                inner: inner(r),
                tap: tap.clone(),
            })
        })
    }

    /// Every transaction sent so far (resends included).
    pub fn sent(&self) -> Vec<StampedTransaction> {
        self.sent.lock().unwrap().clone()
    }

    /// Every op sent so far.
    pub fn sent_ops(&self) -> Vec<Op> {
        self.sent()
            .into_iter()
            .flat_map(|t| t.transaction.ops)
            .collect()
    }

    /// Append `ops` to the next transaction sent.
    pub fn inject(&self, ops: Vec<Op>) {
        *self.inject.lock().unwrap() = ops;
    }
}

struct TapLink {
    inner: BoxTransport,
    tap: Tap,
}

impl CollabTransport for TapLink {
    fn send(&mut self, message: &CollabMessage) {
        let mut message = message.clone();
        if let CollabMessage::Transaction { transaction } = &mut message {
            transaction
                .transaction
                .ops
                .append(&mut self.tap.inject.lock().unwrap());
            self.tap.sent.lock().unwrap().push(transaction.clone());
        }
        self.inner.send(&message);
    }
    fn poll(&mut self, out: &mut Vec<CollabMessage>) {
        self.inner.poll(out);
    }
    fn state(&self) -> LinkState {
        self.inner.state()
    }
    fn close(&mut self) {
        self.inner.close();
    }
}

/// Tick every site and deliver every message until nothing moves (bounded).
pub fn settle(sites: &mut [&mut Site], hub: &Hub) {
    for _ in 0..500 {
        for s in sites.iter_mut() {
            s.tick();
        }
        let delivered = hub.deliver();
        let waiting: usize = hub.links().iter().map(|c| hub.outgoing(*c)).sum();
        let pending: usize = sites.iter().map(|s| s.ctl.collab_pending()).sum();
        if delivered == 0 && waiting == 0 && pending == 0 {
            for s in sites.iter_mut() {
                s.tick();
            }
            return;
        }
    }
    let state: Vec<_> = sites
        .iter()
        .map(|s| (s.status(), s.ctl.collab_pending()))
        .collect();
    let waiting: Vec<_> = hub.links().iter().map(|c| (*c, hub.outgoing(*c))).collect();
    panic!(
        "collab sites did not settle: {state:?}, relay out {waiting:?}, queued {}",
        hub.queued()
    );
}

/// A session "jam" created by a new site `a` (with a project) and joined by `others`.
pub fn session(hub: &Hub, n: usize) -> Vec<Site> {
    let sites: Vec<Site> = (0..n)
        .map(|i| Site::on_hub(0x1000 + i as u64 * 0x9e37, hub))
        .collect();
    session_of(hub, sites)
}

/// Like [`session`], with every site's links tapped.
pub fn tapped_session(hub: &Hub, n: usize) -> (Vec<Site>, Vec<Tap>) {
    let taps: Vec<Tap> = (0..n).map(|_| Tap::default()).collect();
    let sites = taps
        .iter()
        .enumerate()
        .map(|(i, t)| {
            Site::new(
                0x1000 + i as u64 * 0x9e37,
                t.connector(hub),
                MemoryLibrary::new(),
            )
        })
        .collect();
    (session_of(hub, sites), taps)
}

/// `sites[0]` creates "Jam" and the session, then the others join.
pub fn session_of(hub: &Hub, mut sites: Vec<Site>) -> Vec<Site> {
    sites[0].create_project("Jam");
    sites[0].join("ws://hub", "jam", "Site 0", None);
    {
        let mut refs: Vec<&mut Site> = sites.iter_mut().collect();
        settle(&mut refs, hub);
    }
    for (i, s) in sites.iter_mut().enumerate().skip(1) {
        s.join("ws://hub", "jam", &format!("Site {i}"), None);
    }
    let mut refs: Vec<&mut Site> = sites.iter_mut().collect();
    settle(&mut refs, hub);
    for s in &sites {
        assert!(s.online(), "site online: {:?}", s.status());
    }
    sites
}

pub fn assert_converged(sites: &[&Site]) {
    let first = shared(sites[0].project());
    for s in sites {
        s.project().validate().expect("replica validates");
        assert_eq!(shared(s.project()), first, "replicas diverged");
    }
}

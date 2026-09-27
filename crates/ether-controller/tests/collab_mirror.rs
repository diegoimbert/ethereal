//! Plugin GUI mirrors while listening (`plugin-mirror`; docs/COLLAB.md §9.6): a real
//! listener controller over a bridge with fake mirrors, a real peer, and a scripted host.
//!
//! - Listening swaps the listener's plugin instances for mirrors (the engine slot becomes
//!   the bridge's stand-in), and stopping swaps them back, from the mirror's last state.
//! - Mirror GUI edits are ordinary undoable, replicated `SetParam`s; document changes
//!   (a peer's edit, undo, reset) are pushed into the mirror.
//! - A save replicates the mirror's opaque state; armed tracks, the setting, a bridge
//!   without mirrors and a plugin without a mirror keep live instances.

#[path = "collab_support.rs"]
mod support;

use std::collections::{BTreeMap, BTreeSet};

use ether_collab::memory::{Hub, MemoryLink};
use ether_collab::{CollabTransport, ConnectRequest};
use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::{BridgeError, Controller, ControllerConfig, EngineBridge, EtherController};
use ether_core::plugin::PluginNotification;
use ether_core::protocol::collab::*;
use ether_core::protocol::devices::{DeviceCategory, DeviceCommand, DeviceDescriptor, DeviceSpec};
use ether_core::protocol::model::*;
use ether_core::protocol::project::{EditCommand, ProjectCommand};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::recording::RecordingCommand;
use ether_core::protocol::*;
use ether_core::{EngineOutputs, NodeKey, ParamChange, RenderGraphDesc, TransportControl};
use ether_media::DecodedAudio;
use support::*;

const VERB: &str = "com.test.Verb";
const MIX: ParamId = ParamId(7);
const HOST: u64 = 0xdead_beef;

#[derive(Clone, Debug, PartialEq)]
enum MirrorCall {
    Create(DeviceId, Option<Base64Bytes>),
    Destroy(DeviceId),
    Param(DeviceId, ParamId, f64),
}

#[derive(Default)]
struct FakeMirror {
    state: Option<Base64Bytes>,
}

/// [`FakeBridge`] plus mirrors that behave like the native bridge's: while a device has a
/// mirror its engine slot is a stand-in, and the state of a dropped mirror seeds the live
/// instance that replaces it.
#[derive(Default)]
struct MirrorBridge {
    inner: FakeBridge,
    mirrors: BTreeMap<DeviceId, FakeMirror>,
    parked: BTreeMap<DeviceId, Base64Bytes>,
    stand_ins: BTreeMap<NodeKey, DeviceId>,
    log: Vec<MirrorCall>,
    /// Notifications the mirrors' GUIs report at the next poll.
    gui: Vec<(DeviceId, PluginNotification)>,
    /// No mirror support at all (web).
    unsupported: bool,
    /// Plugins this site cannot mirror.
    missing: BTreeSet<String>,
}

impl MirrorBridge {
    fn creates(&self) -> usize {
        self.log
            .iter()
            .filter(|c| matches!(c, MirrorCall::Create(..)))
            .count()
    }

    fn pushes(&self, device: DeviceId) -> Vec<(ParamId, f64)> {
        self.log
            .iter()
            .filter_map(|c| match c {
                MirrorCall::Param(d, p, v) if *d == device => Some((*p, *v)),
                _ => None,
            })
            .collect()
    }

    /// Whether `device` currently has a live plugin node (not a stand-in).
    fn live_plugin(&self, device: DeviceId) -> bool {
        self.inner
            .live
            .iter()
            .any(|(k, d)| *d == device && !self.stand_ins.contains_key(k))
    }

    fn stand_in(&self, device: DeviceId) -> bool {
        self.stand_ins.values().any(|d| *d == device)
    }
}

impl EngineBridge for MirrorBridge {
    fn create_builtin(
        &mut self,
        device: DeviceId,
        kind: &BuiltinDevice,
        params: &[(ParamId, f64)],
    ) -> Result<NodeKey, BridgeError> {
        self.inner.create_builtin(device, kind, params)
    }

    fn create_plugin(
        &mut self,
        device: DeviceId,
        plugin: &PluginInstance,
        state: Option<&Base64Bytes>,
    ) -> Result<NodeKey, BridgeError> {
        if self.mirrors.contains_key(&device) {
            let key = self
                .inner
                .create_builtin(device, &BuiltinDevice::Synth, &[])?;
            self.stand_ins.insert(key, device);
            return Ok(key);
        }
        self.parked.remove(&device);
        self.inner.create_plugin(device, plugin, state)
    }

    fn destroy_node(&mut self, key: NodeKey) -> Result<(), BridgeError> {
        self.stand_ins.remove(&key);
        self.inner.destroy_node(key)
    }

    fn load_media(
        &mut self,
        media: &MediaRef,
        audio: std::sync::Arc<DecodedAudio>,
    ) -> Result<(), BridgeError> {
        self.inner.load_media(media, audio)
    }

    fn unload_media(&mut self, media: MediaId) -> Result<(), BridgeError> {
        self.inner.unload_media(media)
    }

    fn publish(&mut self, graph: RenderGraphDesc) -> Result<(), BridgeError> {
        self.inner.publish(graph)
    }

    fn set_param(&mut self, change: ParamChange) -> Result<(), BridgeError> {
        self.inner.set_param(change)
    }

    fn transport(&mut self, control: TransportControl) -> Result<(), BridgeError> {
        self.inner.transport(control)
    }

    fn poll(&mut self, out: &mut EngineOutputs) {
        self.inner.poll(out);
    }

    fn descriptor(&mut self, device: DeviceId) -> Option<DeviceDescriptor> {
        self.inner.descriptor(device)
    }

    fn poll_plugins(&mut self, out: &mut Vec<(DeviceId, PluginNotification)>) {
        self.inner.poll_plugins(out);
        out.append(&mut self.gui);
    }

    fn plugin_state(&mut self, device: DeviceId) -> Result<Option<Base64Bytes>, BridgeError> {
        if let Some(m) = self.mirrors.get(&device) {
            return Ok(m.state.clone());
        }
        if let Some(s) = self.parked.get(&device) {
            return Ok(Some(s.clone()));
        }
        self.inner.plugin_state(device)
    }

    fn create_plugin_mirror(
        &mut self,
        device: DeviceId,
        plugin: &PluginInstance,
        state: Option<&Base64Bytes>,
    ) -> Result<(), BridgeError> {
        if self.unsupported {
            return Err(BridgeError::Unsupported("no mirrors".into()));
        }
        self.log.push(MirrorCall::Create(device, state.cloned()));
        if self.missing.contains(&plugin.plugin_id) {
            return Err(BridgeError::Other("not installed".into()));
        }
        self.mirrors.insert(
            device,
            FakeMirror {
                state: state.cloned(),
            },
        );
        Ok(())
    }

    fn destroy_plugin_mirror(&mut self, device: DeviceId) -> Result<(), BridgeError> {
        if let Some(m) = self.mirrors.remove(&device) {
            self.log.push(MirrorCall::Destroy(device));
            if let Some(s) = m.state {
                self.parked.insert(device, s);
            }
        }
        Ok(())
    }

    fn set_plugin_mirror_param(
        &mut self,
        device: DeviceId,
        param: ParamId,
        value: f64,
    ) -> Result<(), BridgeError> {
        assert!(self.mirrors.contains_key(&device), "push into a dropped mirror");
        self.log.push(MirrorCall::Param(device, param, value));
        Ok(())
    }
}

type MCtl = EtherController<MirrorBridge, SeededHost, MemoryStore, MemoryLibrary>;

/// The listener: a real controller over [`MirrorBridge`].
struct Listener {
    ctl: MCtl,
    next_request: u32,
}

impl Listener {
    fn new(hub: &Hub) -> Self {
        let mut store = MemoryStore::new();
        store.now_ms = T0;
        let mut bridge = MirrorBridge::default();
        bridge.inner.plugins = Some(plugins());
        let mut ctl = EtherController::with_config(
            bridge,
            SeededHost {
                now: T0,
                seed: 0x4242,
            },
            store,
            MemoryLibrary::new(),
            ControllerConfig {
                autosave_after_ms: None,
                ..ControllerConfig::default()
            },
        );
        ctl.set_collab_connector(hub.connector());
        Self {
            ctl,
            next_request: 1,
        }
    }

    fn send(&mut self, command: Command) -> Vec<ServerMessage> {
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
        out
    }

    fn ok(&mut self, command: Command) -> ReplyValue {
        ok(&self.send(command))
    }

    fn tick(&mut self) {
        self.ctl.host.now += 20;
        self.ctl.store.now_ms = self.ctl.host.now;
        let now = self.ctl.host.now;
        self.ctl.tick(now, &mut Vec::new());
    }

    fn param(&self, device: DeviceId) -> Option<f64> {
        self.ctl.project()?.devices.get(&device)?.params.get(&MIX).copied()
    }

    fn bridge(&mut self) -> &mut MirrorBridge {
        &mut self.ctl.bridge
    }
}

fn plugins() -> BTreeMap<String, DeviceDescriptor> {
    BTreeMap::from([(
        VERB.to_string(),
        plugin_descriptor("Verb", DeviceCategory::AudioEffect),
    )])
}

/// A scripted host: joins over a raw link and answers `Listen` like a host bridge would.
struct ScriptedHost {
    link: MemoryLink,
}

impl ScriptedHost {
    fn join(hub: &Hub) -> Self {
        let mut link = hub.link(&ConnectRequest {
            server: "ws://hub".into(),
            session: "jam".into(),
            token: None,
            client: "scripted host".into(),
        });
        link.send(&CollabMessage::Hello {
            site: SiteId(HOST),
            actor: None,
            name: "Host".into(),
            protocol_version: ether_collab::wire::COLLAB_PROTOCOL_VERSION,
        });
        link.send(&CollabMessage::SyncRequest {
            site: SiteId(HOST),
            version: Base64Bytes(Vec::new()),
        });
        Self { link }
    }

    /// Answer a pending `Listen` with an offer and a first clock anchor (media flowing).
    fn answer(&mut self) {
        let mut inbox = Vec::new();
        self.link.poll(&mut inbox);
        for m in inbox {
            if let CollabMessage::Listen { from, stream, .. } = m {
                self.link.send(&CollabMessage::Signal {
                    from: SiteId(HOST),
                    to: from,
                    stream,
                    signal: StreamSignal::Offer {
                        sdp: "v=0 offer".into(),
                    },
                });
                self.link.send(&CollabMessage::StreamClock {
                    from: SiteId(HOST),
                    to: from,
                    stream,
                    clock: StreamClock {
                        rtp: 1_000,
                        position: Beats(0.0),
                        playing: false,
                        recording: false,
                        bpm: 120.0,
                        loop_enabled: false,
                        loop_region: BeatRange {
                            start: Beats(0.0),
                            end: Beats(4.0),
                        },
                        metronome: false,
                        discontinuity: true,
                        count_in_end: None,
                    },
                });
            }
        }
    }
}

/// A peer (session creator, with the plugin), the listener, and the scripted host.
struct World {
    hub: Hub,
    peer: Site,
    listener: Listener,
    host: ScriptedHost,
}

impl World {
    fn new() -> Self {
        let hub = Hub::default();
        let mut peer = Site::on_hub(0x1000, &hub);
        peer.ctl.bridge.plugins = Some(plugins());
        peer.create_project("Jam");
        peer.join("ws://hub", "jam", "Peer", None);
        settle(&mut [&mut peer], &hub);
        let mut listener = Listener::new(&hub);
        listener.ok(Command::Collab(CollabCommand::Join {
            server: "ws://hub".into(),
            session: "jam".into(),
            token: None,
            name: "Listener".into(),
        }));
        let host = ScriptedHost::join(&hub);
        let mut w = Self {
            hub,
            peer,
            listener,
            host,
        };
        w.settle();
        w
    }

    fn settle(&mut self) {
        for _ in 0..500 {
            self.peer.tick();
            self.listener.tick();
            let delivered = self.hub.deliver();
            self.host.answer();
            let waiting: usize = self.hub.links().iter().map(|c| self.hub.outgoing(*c)).sum();
            let pending = self.peer.ctl.collab_pending() + self.listener.ctl.collab_pending();
            if delivered == 0 && waiting == 0 && pending == 0 {
                self.peer.tick();
                self.listener.tick();
                return;
            }
        }
        panic!("did not settle");
    }

    /// A plugin device on a new track, inserted by the peer.
    fn plugin_device(&mut self, kind: TrackKind) -> (TrackId, DeviceId) {
        let track: TrackId = self.peer.id();
        self.peer.ok(Command::Track(TrackCommand::Create {
            id: track,
            kind,
            name: None,
            color: None,
            parent: None,
            before: None,
        }));
        let device: DeviceId = self.peer.id();
        self.peer.ok(Command::Device(DeviceCommand::Insert {
            id: device,
            track,
            device: DeviceSpec::Plugin {
                plugin_id: VERB.into(),
                sandboxed: None,
                format: None,
            },
            before: None,
        }));
        self.settle();
        assert!(self.listener.bridge().live_plugin(device), "live on the listener");
        (track, device)
    }

    fn listen(&mut self) {
        self.listener.ok(Command::Collab(CollabCommand::Listen {
            host: SiteId(HOST),
        }));
        self.settle();
    }

    fn stop_listening(&mut self) {
        self.listener
            .ok(Command::Collab(CollabCommand::StopListening));
        self.settle();
    }

    fn set_param(&mut self, device: DeviceId, value: f64) {
        self.peer.ok(Command::Device(DeviceCommand::SetParam {
            device,
            param: MIX,
            value,
        }));
        self.settle();
    }

    fn peer_param(&self, device: DeviceId) -> Option<f64> {
        self.peer.project().devices.get(&device)?.params.get(&MIX).copied()
    }

    fn peer_state(&self, device: DeviceId) -> Option<Base64Bytes> {
        match &self.peer.project().devices.get(&device)?.kind {
            DeviceKind::Plugin { plugin } => plugin.state.clone(),
            DeviceKind::Builtin { .. } => None,
        }
    }
}

#[test]
fn listening_swaps_plugins_for_mirrors_and_back() {
    let mut w = World::new();
    let (_, d) = w.plugin_device(TrackKind::Audio);
    w.set_param(d, 30.0);
    // What the listener's live instance holds right now.
    w.listener
        .bridge()
        .inner
        .plugin_states
        .insert(d, Base64Bytes(vec![5]));
    assert!(w.listener.ctl.plugin_mirrors().is_empty());

    w.listen();
    let b = w.listener.bridge();
    assert_eq!(
        b.log.first(),
        Some(&MirrorCall::Create(d, Some(Base64Bytes(vec![5])))),
        "the mirror starts from the live instance's state"
    );
    assert!(b.stand_in(d), "the engine slot is a stand-in");
    assert!(!b.live_plugin(d), "the live instance is gone");
    assert_eq!(b.pushes(d), vec![(MIX, 30.0)], "document values pushed in");
    assert_eq!(w.listener.ctl.plugin_mirrors(), vec![d]);

    // Mirror GUI edits: an undoable, replicated SetParam (the host's instance gets it).
    w.listener.bridge().gui.extend([
        (d, PluginNotification::GestureBegin { param: MIX }),
        (
            d,
            PluginNotification::ParamEdited {
                param: MIX,
                value: 70.0,
            },
        ),
        (
            d,
            PluginNotification::ParamEdited {
                param: MIX,
                value: 80.0,
            },
        ),
        (d, PluginNotification::GestureEnd { param: MIX }),
    ]);
    w.settle();
    assert_eq!(w.listener.param(d), Some(80.0));
    assert_eq!(w.peer_param(d), Some(80.0), "replicated");
    // One gesture = one undo step: back to 30 everywhere, and the mirror shows it.
    w.listener.ok(Command::Edit(EditCommand::Undo));
    w.settle();
    assert_eq!(w.peer_param(d), Some(30.0));
    assert_eq!(w.listener.bridge().pushes(d).last(), Some(&(MIX, 30.0)));

    // A peer's edit and a reset reach the mirror (reset = the param's default).
    w.set_param(d, 12.0);
    assert_eq!(w.listener.bridge().pushes(d).last(), Some(&(MIX, 12.0)));
    w.peer.ok(Command::Device(DeviceCommand::ResetParam {
        device: d,
        param: MIX,
    }));
    w.settle();
    let default = plugin_descriptor("Verb", DeviceCategory::AudioEffect).params[0].default;
    assert_eq!(w.listener.bridge().pushes(d).last(), Some(&(MIX, default)));
    // Pushes are only for changes.
    let n = w.listener.bridge().pushes(d).len();
    for _ in 0..5 {
        w.listener.tick();
    }
    assert_eq!(w.listener.bridge().pushes(d).len(), n);

    // A preset loaded in the mirror GUI (opaque state) replicates at the next save.
    w.listener
        .bridge()
        .mirrors
        .get_mut(&d)
        .unwrap()
        .state = Some(Base64Bytes(vec![7]));
    w.listener.ok(Command::Project(ProjectCommand::Save));
    w.settle();
    assert_eq!(w.peer_state(d), Some(Base64Bytes(vec![7])));

    // Stop: the live instance comes back from the mirror's last state.
    w.stop_listening();
    let b = w.listener.bridge();
    assert!(b.log.contains(&MirrorCall::Destroy(d)));
    assert!(b.live_plugin(d));
    assert!(!b.stand_in(d));
    assert!(
        matches!(
            b.inner.calls.iter().rev().find(|c| matches!(c, Call::CreatePlugin(dev, ..) if *dev == d)),
            Some(Call::CreatePlugin(_, _, Some(s))) if s.0 == vec![7]
        ),
        "live instance re-created from the mirror's state"
    );
    assert!(w.listener.ctl.plugin_mirrors().is_empty());
}

#[test]
fn what_gets_mirrored() {
    let mut w = World::new();
    let (_, a) = w.plugin_device(TrackKind::Audio);
    let (tb, b) = w.plugin_device(TrackKind::Midi);
    // A record-armed track keeps its live instance (live MIDI / monitoring still sound).
    w.listener.ok(Command::Recording(RecordingCommand::Arm {
        track: tb,
        armed: true,
        exclusive: false,
    }));
    w.listen();
    assert_eq!(w.listener.ctl.plugin_mirrors(), vec![a]);
    assert!(w.listener.bridge().live_plugin(b));
    // Disarming mirrors it too.
    w.listener.ok(Command::Recording(RecordingCommand::Arm {
        track: tb,
        armed: false,
        exclusive: false,
    }));
    w.settle();
    let mut both = vec![a, b];
    both.sort();
    assert_eq!(w.listener.ctl.plugin_mirrors(), both);

    // The setting off: live instances while listening.
    w.listener.ctl.set_plugin_mirrors(false);
    w.settle();
    assert!(w.listener.ctl.plugin_mirrors().is_empty());
    assert!(w.listener.bridge().live_plugin(a) && w.listener.bridge().live_plugin(b));
    assert!(w.listener.bridge().mirrors.is_empty());
    w.listener.ctl.set_plugin_mirrors(true);
    w.settle();
    assert_eq!(w.listener.ctl.plugin_mirrors(), both);

    // A device deleted by a peer drops its mirror.
    w.peer.ok(Command::Device(DeviceCommand::Remove { id: a }));
    w.settle();
    assert_eq!(w.listener.ctl.plugin_mirrors(), vec![b]);
    assert!(!w.listener.bridge().mirrors.contains_key(&a));

    // Leaving the session ends listening: live instances again.
    w.listener.ok(Command::Collab(CollabCommand::Leave));
    w.listener.tick();
    w.listener.tick();
    assert!(w.listener.ctl.plugin_mirrors().is_empty());
    assert!(w.listener.bridge().mirrors.is_empty());
    assert!(w.listener.bridge().live_plugin(b));
}

#[test]
fn bridges_without_mirrors_keep_live_instances() {
    // No mirror support (web): asked once, never again.
    let mut w = World::new();
    let (_, d) = w.plugin_device(TrackKind::Audio);
    w.listener.bridge().unsupported = true;
    w.listen();
    assert!(w.listener.ctl.plugin_mirrors().is_empty());
    assert!(w.listener.bridge().live_plugin(d));

    // A plugin this site cannot mirror: tried once per plugin, the live instance stays.
    let mut w = World::new();
    let (_, d) = w.plugin_device(TrackKind::Audio);
    w.listener.bridge().missing.insert(VERB.into());
    w.listen();
    for _ in 0..10 {
        w.listener.tick();
    }
    assert_eq!(w.listener.bridge().creates(), 1);
    assert!(w.listener.ctl.plugin_mirrors().is_empty());
    assert!(w.listener.bridge().live_plugin(d));
    // Params still sync through the live instance (not the mirror path).
    w.set_param(d, 44.0);
    assert!(w.listener.bridge().pushes(d).is_empty());
}

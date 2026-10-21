//! Test doubles: a recording fake `EngineBridge`, a manual clock, a harness that drives
//! `ClientMessage`s, and a WAV generator.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::Arc;

use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::{
    BridgeError, Controller, ControllerConfig, EngineBridge, EtherController, HostServices,
};
use ether_core::plugin::PluginNotification;
use ether_core::protocol::devices::{
    DeviceCategory, DeviceDescriptor, DeviceTypeRef, ParamInfo, ParamScale, ParamUnit,
};
use ether_core::protocol::meters::TrackMeter;
use ether_core::protocol::model::*;
use ether_core::protocol::project::ProjectCommand;
use ether_core::protocol::*;
use ether_core::{
    EngineOutputs, NodeKey, ParamChange, PlayheadState, RenderGraphDesc, TransportControl,
};
use ether_media::DecodedAudio;

/// Everything the controller asked the engine to do, in order.
#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    CreateBuiltin(DeviceId, NodeKey),
    CreatePlugin(DeviceId, NodeKey, Option<Base64Bytes>),
    Destroy(NodeKey),
    LoadMedia(MediaId),
    UnloadMedia(MediaId),
    Publish(u64),
    SetParam(ParamChange),
    Transport(TransportControl),
    /// A state loaded into a running plugin instance in place (`set_plugin_state`).
    SetPluginState(DeviceId, Base64Bytes),
}

#[derive(Default)]
pub struct FakeBridge {
    pub calls: Vec<Call>,
    pub live: BTreeMap<NodeKey, DeviceId>,
    pub graphs: Vec<RenderGraphDesc>,
    pub media: BTreeMap<MediaId, Arc<DecodedAudio>>,
    pub next: u32,
    /// Plugins: `None` = unsupported (web).
    pub plugins: Option<BTreeMap<String, DeviceDescriptor>>,
    pub plugin_states: BTreeMap<DeviceId, Base64Bytes>,
    /// Every `plugin_state` call, in order (a slow main-thread call on real hosts).
    pub plugin_state_reads: Vec<DeviceId>,
    pub plugin_notes: Vec<(DeviceId, PluginNotification)>,
    pub playhead: Option<PlayheadState>,
    pub meters: Vec<TrackMeter>,
    /// v0.2 analysis watches the controller sent (`watch_analysis`), in order.
    pub analysis_watches: Vec<(NodeKey, bool)>,
    /// Make `watch_analysis` fail (queue full) while set.
    pub fail_watches: bool,
    /// `create_plugin` panics (a plugin crashing the process while it loads).
    pub panic_on_plugin: bool,
    /// What `plugin_catalog` reports (`None` = no plugin host).
    pub catalog: Option<ether_controller::PluginCatalogView>,
    /// The plugin id each live plugin device was created from (per-device descriptors).
    pub plugin_of: BTreeMap<DeviceId, String>,
    /// The plugin name each plugin device was created with (`PluginInstance.name`: what
    /// picks one of several plugins sharing a format + id).
    pub plugin_names: BTreeMap<DeviceId, String>,
    /// `publish` fails with this message while set (a graph over the engine's capacity).
    pub reject_publish: Option<String>,
    /// Media the bridge asks the controller to decode again (`take_ir_media_requests`).
    pub ir_requests: Vec<MediaId>,
    /// The audio handed over for them (`ir_media`), in order.
    pub ir_audio: Vec<(MediaId, Arc<DecodedAudio>)>,
    /// `create_offline_plugin` works (a Utility node stands in for the plugin); unset: the
    /// default `Unsupported` (no plugin host).
    pub offline_plugins: bool,
    /// `set_plugin_state` fails (a plugin that cannot load a state while running).
    pub live_state_fails: bool,
    /// Capture: `None` = unsupported (web), else what the host records.
    pub recording: Option<FakeRecording>,
}

/// A fake capture host: what `poll_recording` hands out next and what `stop_recording`
/// returns (the test writes the take files into the store itself).
#[derive(Debug, Default)]
pub struct FakeRecording {
    /// A session is open (`start_recording` .. `stop_recording`).
    pub active: bool,
    pub sessions: Vec<ether_controller::RecordSession>,
    pub live_audio: Vec<ether_core::protocol::recording::LiveAudioChunk>,
    pub live_midi: Vec<ether_core::protocol::recording::LiveMidiNote>,
    pub takes: ether_controller::RecordedTakes,
}

impl FakeBridge {
    pub fn last_graph(&self) -> &RenderGraphDesc {
        self.graphs.last().expect("a graph was published")
    }

    pub fn param_changes(&self) -> Vec<ParamChange> {
        self.calls
            .iter()
            .filter_map(|c| match c {
                Call::SetParam(p) => Some(*p),
                _ => None,
            })
            .collect()
    }

    pub fn publishes(&self) -> usize {
        self.calls
            .iter()
            .filter(|c| matches!(c, Call::Publish(_)))
            .count()
    }

    fn key(&mut self) -> NodeKey {
        self.next += 1;
        NodeKey {
            index: self.next,
            generation: 1,
        }
    }
}

pub fn plugin_descriptor(name: &str, category: DeviceCategory) -> DeviceDescriptor {
    DeviceDescriptor {
        layout: None,
        device_type: DeviceTypeRef::Plugin {
            plugin_id: format!("com.test.{name}"),
        },
        name: name.into(),
        category,
        params: vec![ParamInfo {
            step: None,
            remote: None,
            id: ParamId(7),
            name: "Mix".into(),
            group: None,
            unit: ParamUnit::Percent,
            min: 0.0,
            max: 100.0,
            default: 50.0,
            scale: ParamScale::Linear,
            labels: None,
            automatable: true,
            hidden: false,
        }],
        audio_inputs: 2,
        audio_outputs: 2,
        midi_input: false,
        sidechain_inputs: 0,
        implemented: true,
    }
}

impl EngineBridge for FakeBridge {
    fn create_builtin(
        &mut self,
        device: DeviceId,
        _kind: &BuiltinDevice,
        _params: &[(ParamId, f64)],
    ) -> Result<NodeKey, BridgeError> {
        let key = self.key();
        self.live.insert(key, device);
        self.calls.push(Call::CreateBuiltin(device, key));
        Ok(key)
    }

    fn create_plugin(
        &mut self,
        device: DeviceId,
        plugin: &PluginInstance,
        state: Option<&Base64Bytes>,
    ) -> Result<NodeKey, BridgeError> {
        assert!(!self.panic_on_plugin, "plugin crashed while loading");
        let Some(plugins) = &self.plugins else {
            return Err(BridgeError::Unsupported("no plugins".into()));
        };
        if !plugins.contains_key(&plugin.plugin_id) {
            return Err(BridgeError::Other(format!(
                "unknown plugin {}",
                plugin.plugin_id
            )));
        }
        let key = self.key();
        self.live.insert(key, device);
        self.plugin_of.insert(device, plugin.plugin_id.clone());
        self.plugin_names.insert(device, plugin.name.clone());
        self.calls
            .push(Call::CreatePlugin(device, key, state.cloned()));
        Ok(key)
    }

    fn destroy_node(&mut self, key: NodeKey) -> Result<(), BridgeError> {
        assert!(
            self.live.remove(&key).is_some(),
            "destroying unknown node {key:?}"
        );
        // Never destroy a node the current graph still references.
        if let Some(g) = self.graphs.last() {
            assert!(
                !g.tracks.iter().any(|t| t.chain.iter().any(|c| c.node == key)
                    || t.automation.iter().any(|a| matches!(a.resolved, ether_core::graph::ResolvedTarget::Node { node, .. } if node == key))),
                "destroyed node {key:?} is still referenced by the published graph"
            );
        }
        self.calls.push(Call::Destroy(key));
        Ok(())
    }

    fn load_media(
        &mut self,
        media: &MediaRef,
        audio: Arc<DecodedAudio>,
    ) -> Result<(), BridgeError> {
        self.media.insert(media.id, audio);
        self.calls.push(Call::LoadMedia(media.id));
        Ok(())
    }

    fn take_ir_media_requests(&mut self) -> Vec<MediaId> {
        std::mem::take(&mut self.ir_requests)
    }

    fn ir_media(&mut self, media: MediaId, audio: Arc<DecodedAudio>) -> Result<(), BridgeError> {
        self.ir_audio.push((media, audio));
        Ok(())
    }

    fn unload_media(&mut self, media: MediaId) -> Result<(), BridgeError> {
        self.media.remove(&media);
        self.calls.push(Call::UnloadMedia(media));
        Ok(())
    }

    fn publish(&mut self, graph: RenderGraphDesc) -> Result<(), BridgeError> {
        if let Some(m) = self
            .reject_publish
            .as_ref()
            .filter(|_| !graph.tracks.is_empty())
        {
            return Err(BridgeError::Other(m.clone()));
        }
        // Every chain node must be live.
        for t in &graph.tracks {
            for c in &t.chain {
                assert!(
                    self.live.contains_key(&c.node),
                    "graph references dead node {:?}",
                    c.node
                );
            }
        }
        self.calls.push(Call::Publish(graph.version));
        self.graphs.push(graph);
        Ok(())
    }

    fn set_param(&mut self, change: ParamChange) -> Result<(), BridgeError> {
        self.calls.push(Call::SetParam(change));
        Ok(())
    }

    fn transport(&mut self, control: TransportControl) -> Result<(), BridgeError> {
        self.calls.push(Call::Transport(control));
        Ok(())
    }

    fn poll(&mut self, out: &mut EngineOutputs) {
        out.clear();
        out.playhead = self.playhead;
        out.meters = std::mem::take(&mut self.meters);
    }

    fn watch_analysis(&mut self, node: NodeKey, on: bool) -> Result<(), BridgeError> {
        if self.fail_watches {
            return Err(BridgeError::Unsupported("queue full".into()));
        }
        self.analysis_watches.push((node, on));
        Ok(())
    }

    fn descriptor(&mut self, device: DeviceId) -> Option<DeviceDescriptor> {
        let plugins = self.plugins.as_ref()?;
        // The device's own plugin when known, else the only plugin type of the test.
        match self.plugin_of.get(&device) {
            Some(id) => plugins.get(id).cloned(),
            None => plugins.values().next().cloned(),
        }
    }

    fn plugin_catalog(&self) -> Option<ether_controller::PluginCatalogView> {
        self.catalog.clone()
    }

    fn poll_plugins(&mut self, out: &mut Vec<(DeviceId, PluginNotification)>) {
        out.append(&mut self.plugin_notes);
    }

    fn create_offline_plugin(
        &mut self,
        device: DeviceId,
        plugin: &PluginInstance,
        state: Option<&Base64Bytes>,
        sample_rate: u32,
    ) -> Result<Box<dyn ether_core::Node>, BridgeError> {
        let _ = (device, plugin, state, sample_rate);
        if !self.offline_plugins {
            return Err(BridgeError::Unsupported(
                "offline plugin rendering is not available on this host".into(),
            ));
        }
        struct NoSamples;
        impl ether_devices::SampleResolver for NoSamples {
            fn resolve(&self, _: MediaId) -> Option<Arc<dyn ether_core::AudioSource>> {
                None
            }
        }
        Ok(ether_devices::create(&BuiltinDevice::Utility, &NoSamples))
    }

    fn plugin_state(&mut self, device: DeviceId) -> Result<Option<Base64Bytes>, BridgeError> {
        self.plugin_state_reads.push(device);
        Ok(self.plugin_states.get(&device).cloned())
    }

    fn set_plugin_state(
        &mut self,
        device: DeviceId,
        state: &Base64Bytes,
    ) -> Result<bool, BridgeError> {
        if !self.live.values().any(|d| *d == device) {
            return Ok(false);
        }
        if self.live_state_fails {
            return Err(BridgeError::Other(
                "cannot load a state while running".into(),
            ));
        }
        self.calls.push(Call::SetPluginState(device, state.clone()));
        self.plugin_states.insert(device, state.clone());
        Ok(true)
    }

    fn probe_plugin(&mut self, plugin: &PluginInstance) -> Option<DeviceDescriptor> {
        self.plugins.as_ref()?.get(&plugin.plugin_id).cloned()
    }

    fn start_recording(
        &mut self,
        session: &ether_controller::RecordSession,
    ) -> Result<(), BridgeError> {
        let r = self
            .recording
            .as_mut()
            .ok_or_else(|| BridgeError::Unsupported("no capture".into()))?;
        r.active = true;
        r.sessions.push(session.clone());
        Ok(())
    }

    fn poll_recording(
        &mut self,
        audio: &mut Vec<ether_core::protocol::recording::LiveAudioChunk>,
        midi: &mut Vec<ether_core::protocol::recording::LiveMidiNote>,
    ) {
        // Also right after `stop_recording` (what the host flushed while closing the takes).
        if let Some(r) = self.recording.as_mut() {
            audio.append(&mut r.live_audio);
            midi.append(&mut r.live_midi);
        }
    }

    fn stop_recording(&mut self) -> Result<ether_controller::RecordedTakes, BridgeError> {
        let r = self
            .recording
            .as_mut()
            .ok_or_else(|| BridgeError::Unsupported("no capture".into()))?;
        r.active = false;
        Ok(std::mem::take(&mut r.takes))
    }
}

pub struct FakeHost {
    pub now: u64,
    /// Each clock read advances the clock by this much (0: a frozen clock until `advance`),
    /// to simulate slow work such as plugin loading.
    pub step_per_read: u64,
    reads: std::cell::Cell<u64>,
    /// A fake web for `http_get`: URL → (status, body); other URLs answer 404.
    pub web: BTreeMap<String, (u16, Vec<u8>)>,
    /// Every request fails like a machine without network.
    pub offline: bool,
    /// The URLs requested so far.
    pub requested: Vec<String>,
    http_pending: Vec<(u64, String, u64)>,
}

impl FakeHost {
    pub fn at(now: u64) -> Self {
        Self {
            now,
            step_per_read: 0,
            reads: std::cell::Cell::new(0),
            web: BTreeMap::new(),
            offline: false,
            requested: Vec::new(),
            http_pending: Vec::new(),
        }
    }
}

impl HostServices for FakeHost {
    fn now_ms(&self) -> u64 {
        if self.step_per_read == 0 {
            return self.now;
        }
        let n = self.reads.get() + 1;
        self.reads.set(n);
        self.now + n * self.step_per_read
    }
    fn random_seed(&mut self) -> u64 {
        0x5eed
    }
    fn http_get(&mut self, id: u64, url: &str, max_bytes: u64) -> Result<(), String> {
        self.requested.push(url.to_string());
        self.http_pending.push((id, url.to_string(), max_bytes));
        Ok(())
    }
    fn poll_http(&mut self, out: &mut Vec<ether_controller::HttpEvent>) {
        use ether_controller::HttpEvent;
        for (id, url, max) in std::mem::take(&mut self.http_pending) {
            let result = if self.offline {
                Err("offline".to_string())
            } else {
                match self.web.get(&url) {
                    Some((_, body)) if body.len() as u64 > max => Err("too big".to_string()),
                    Some((status, body)) => {
                        out.push(HttpEvent::Progress {
                            id,
                            received: body.len() as u64 / 2,
                            total: Some(body.len() as u64),
                        });
                        Ok((*status, body.clone()))
                    }
                    None => Ok((404, Vec::new())),
                }
            };
            out.push(HttpEvent::Done { id, result });
        }
    }
}

pub type Ctl = EtherController<FakeBridge, FakeHost, MemoryStore, MemoryLibrary>;

pub const T0: u64 = 1_750_000_000_000;

/// `X`: an extension's controller state (`()` for the app's own controller).
pub struct Harness<X = ()> {
    pub ctl: EtherController<FakeBridge, FakeHost, MemoryStore, MemoryLibrary, X>,
    next_request: u32,
    ids: IdGen,
}

impl Harness {
    pub fn new() -> Self {
        Self::with(
            FakeBridge::default(),
            MemoryLibrary::new(),
            ControllerConfig::default(),
        )
    }

    pub fn with(bridge: FakeBridge, library: MemoryLibrary, config: ControllerConfig) -> Self {
        Harness::build(bridge, library, config, EtherController::with_config)
    }

    /// A harness with a created, open project.
    pub fn with_project() -> Self {
        let mut h = Self::new();
        h.create_project("Test");
        h
    }
}

/// The store a harness starts with (at [`T0`]).
pub fn test_store() -> MemoryStore {
    let mut store = MemoryStore::new();
    store.now_ms = T0;
    store
}

impl<X> Harness<X> {
    /// A harness over the controller `make` builds (an extension's, with its state).
    pub fn build(
        bridge: FakeBridge,
        library: MemoryLibrary,
        config: ControllerConfig,
        make: impl FnOnce(
            FakeBridge,
            FakeHost,
            MemoryStore,
            MemoryLibrary,
            ControllerConfig,
        )
            -> EtherController<FakeBridge, FakeHost, MemoryStore, MemoryLibrary, X>,
    ) -> Self {
        Self {
            ctl: make(bridge, FakeHost::at(T0), test_store(), library, config),
            next_request: 1,
            ids: IdGen::new(99),
        }
    }

    pub fn id<I: Id>(&mut self) -> I {
        self.ids.next(T0)
    }

    pub fn project_id(&mut self) -> ProjectId {
        self.ids.next_project_id(T0)
    }

    pub fn create_project(&mut self, name: &str) -> ProjectId {
        let id = self.project_id();
        let out = self.send(Command::Project(ProjectCommand::Create {
            id,
            name: name.into(),
        }));
        ok(&out);
        id
    }

    pub fn advance(&mut self, ms: u64) {
        self.ctl.host.now += ms;
        self.ctl.store.now_ms = self.ctl.host.now;
    }

    pub fn send_with(
        &mut self,
        command: Command,
        gesture: Option<GestureId>,
    ) -> Vec<ServerMessage> {
        let id = self.next_request;
        self.next_request += 1;
        let mut out = Vec::new();
        self.ctl.handle(
            ClientMessage {
                id,
                gesture,
                command,
            },
            &mut out,
        );
        // Exactly one reply, last, with our id.
        let replies: Vec<&Reply> = out
            .iter()
            .filter_map(|m| match m {
                ServerMessage::Reply(r) => Some(r),
                _ => None,
            })
            .collect();
        assert_eq!(replies.len(), 1, "exactly one reply: {out:#?}");
        assert!(
            matches!(out.last(), Some(ServerMessage::Reply(r)) if r.id == id),
            "reply comes last"
        );
        out
    }

    pub fn send(&mut self, command: Command) -> Vec<ServerMessage> {
        self.send_with(command, None)
    }

    /// Send and expect success; returns the reply value.
    pub fn ok(&mut self, command: Command) -> ReplyValue {
        let out = self.send(command);
        ok(&out)
    }

    pub fn tick(&mut self) -> Vec<ServerMessage> {
        let mut out = Vec::new();
        let now = self.ctl.host.now;
        self.ctl.tick(now, &mut out);
        out
    }

    /// Tick until no media job is pending (bounded).
    pub fn drain_media(&mut self) -> Vec<ServerMessage> {
        let mut all = Vec::new();
        for _ in 0..10_000 {
            all.extend(self.tick());
            if !self.ctl.media_pending() {
                return all;
            }
        }
        panic!("media jobs did not finish");
    }

    pub fn project(&self) -> &Project {
        self.ctl.project().expect("open project")
    }
}

pub fn ok(out: &[ServerMessage]) -> ReplyValue {
    match out.last() {
        Some(ServerMessage::Reply(Reply {
            result: ReplyResult::Ok { value },
            ..
        })) => value.clone(),
        other => panic!("expected Ok reply, got {other:#?}"),
    }
}

pub fn err(out: &[ServerMessage]) -> CommandError {
    match out.last() {
        Some(ServerMessage::Reply(Reply {
            result: ReplyResult::Err { error },
            ..
        })) => error.clone(),
        other => panic!("expected Err reply, got {other:#?}"),
    }
}

pub fn patches(out: &[ServerMessage]) -> Vec<Patch> {
    out.iter()
        .filter_map(|m| match m {
            ServerMessage::Event(Event::Patch { patch }) => Some(patch.clone()),
            _ => None,
        })
        .collect()
}

/// Sends `c` and asserts it replies `Unsupported` without changing the document.
pub fn assert_unsupported(h: &mut Harness, c: Command) {
    let before = h.project().clone();
    let out = h.send(c.clone());
    assert_eq!(err(&out).code, ErrorCode::Unsupported, "{c:?}");
    assert!(patches(&out).is_empty(), "{c:?}");
    assert_eq!(h.project(), &before, "{c:?}");
}

pub fn events(out: &[ServerMessage]) -> Vec<Event> {
    out.iter()
        .filter_map(|m| match m {
            ServerMessage::Event(e) => Some(e.clone()),
            _ => None,
        })
        .collect()
}

/// 16-bit PCM WAV bytes.
pub fn wav(sample_rate: u32, channels: &[Vec<f32>]) -> Vec<u8> {
    let n_ch = channels.len() as u16;
    let frames = channels[0].len();
    let data_len = frames * n_ch as usize * 2;
    let mut b = Vec::with_capacity(44 + data_len);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&n_ch.to_le_bytes());
    b.extend_from_slice(&sample_rate.to_le_bytes());
    b.extend_from_slice(&(sample_rate * n_ch as u32 * 2).to_le_bytes());
    b.extend_from_slice(&(n_ch * 2).to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(data_len as u32).to_le_bytes());
    for i in 0..frames {
        for ch in channels {
            let s = (ch[i].clamp(-1.0, 1.0) * 32767.0).round() as i16;
            b.extend_from_slice(&s.to_le_bytes());
        }
    }
    b
}

/// A sine burst at `hz`, `frames` long.
pub fn sine(sample_rate: u32, hz: f32, frames: usize, amp: f32) -> Vec<f32> {
    (0..frames)
        .map(|i| amp * (2.0 * std::f32::consts::PI * hz * i as f32 / sample_rate as f32).sin())
        .collect()
}

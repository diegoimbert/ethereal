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
    pub plugin_notes: Vec<(DeviceId, PluginNotification)>,
    pub playhead: Option<PlayheadState>,
    pub meters: Vec<TrackMeter>,
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
        device_type: DeviceTypeRef::Plugin {
            plugin_id: format!("com.test.{name}"),
        },
        name: name.into(),
        category,
        params: vec![ParamInfo {
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

    fn unload_media(&mut self, media: MediaId) -> Result<(), BridgeError> {
        self.media.remove(&media);
        self.calls.push(Call::UnloadMedia(media));
        Ok(())
    }

    fn publish(&mut self, graph: RenderGraphDesc) -> Result<(), BridgeError> {
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

    fn descriptor(&mut self, device: DeviceId) -> Option<DeviceDescriptor> {
        let _ = device;
        // Only one plugin type is instantiated per test.
        self.plugins.as_ref()?.values().next().cloned()
    }

    fn poll_plugins(&mut self, out: &mut Vec<(DeviceId, PluginNotification)>) {
        out.append(&mut self.plugin_notes);
    }

    fn plugin_state(&mut self, device: DeviceId) -> Result<Option<Base64Bytes>, BridgeError> {
        Ok(self.plugin_states.get(&device).cloned())
    }
}

pub struct FakeHost {
    pub now: u64,
}

impl HostServices for FakeHost {
    fn now_ms(&self) -> u64 {
        self.now
    }
    fn random_seed(&mut self) -> u64 {
        0x5eed
    }
}

pub type Ctl = EtherController<FakeBridge, FakeHost, MemoryStore, MemoryLibrary>;

pub const T0: u64 = 1_750_000_000_000;

pub struct Harness {
    pub ctl: Ctl,
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
        let mut store = MemoryStore::new();
        store.now_ms = T0;
        Self {
            ctl: EtherController::with_config(bridge, FakeHost { now: T0 }, store, library, config),
            next_request: 1,
            ids: IdGen::new(99),
        }
    }

    /// A harness with a created, open project.
    pub fn with_project() -> Self {
        let mut h = Self::new();
        h.create_project("Test");
        h
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

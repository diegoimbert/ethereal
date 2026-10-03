//! External devices in the controller (`external-instrument`, CONTRACTS.md §13.7):
//! `SetRouting` (validation, undo, compile into `hw_io`), `ListPorts`, `MeasureLatency`
//! (the result sets `Latency` as one undoable edit; failures and the wall-clock deadline)
//! and `PortsChanged`, with a bridge that has hardware I/O.

mod common;

use std::collections::VecDeque;
use std::sync::Arc;

use common::*;
use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::{BridgeError, Controller, ControllerConfig, EngineBridge, EtherController};
use ether_core::hw_io::HwLatencyResult;
use ether_core::plugin::PluginNotification;
use ether_core::protocol::devices::{DeviceCommand, DeviceDescriptor, DeviceSpec};
use ether_core::protocol::external::{ExternalCommand, ExternalEvent, HardwarePorts};
use ether_core::protocol::model::*;
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::recording::{AudioInputChannel, MidiPort};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::*;
use ether_core::{EngineOutputs, NodeKey, ParamChange, RenderGraphDesc, TransportControl};
use ether_devices::external::{external_audio_effect as fx, external_instrument as inst};
use ether_media::DecodedAudio;

/// [`FakeBridge`] plus hardware: ports, and measurements answered with `answer`.
#[derive(Default)]
struct HwBridge {
    fake: FakeBridge,
    ports: HardwarePorts,
    requests: Vec<NodeKey>,
    /// What the engine reports for the next request (`None` = keep it running).
    answer: Option<Option<u32>>,
    results: VecDeque<HwLatencyResult>,
}

impl EngineBridge for HwBridge {
    fn create_builtin(
        &mut self,
        device: DeviceId,
        kind: &BuiltinDevice,
        params: &[(ParamId, f64)],
    ) -> Result<NodeKey, BridgeError> {
        self.fake.create_builtin(device, kind, params)
    }
    fn create_plugin(
        &mut self,
        device: DeviceId,
        plugin: &PluginInstance,
        state: Option<&Base64Bytes>,
    ) -> Result<NodeKey, BridgeError> {
        self.fake.create_plugin(device, plugin, state)
    }
    fn destroy_node(&mut self, key: NodeKey) -> Result<(), BridgeError> {
        self.fake.destroy_node(key)
    }
    fn load_media(
        &mut self,
        media: &MediaRef,
        audio: Arc<DecodedAudio>,
    ) -> Result<(), BridgeError> {
        self.fake.load_media(media, audio)
    }
    fn unload_media(&mut self, media: MediaId) -> Result<(), BridgeError> {
        self.fake.unload_media(media)
    }
    fn publish(&mut self, graph: RenderGraphDesc) -> Result<(), BridgeError> {
        self.fake.publish(graph)
    }
    fn set_param(&mut self, change: ParamChange) -> Result<(), BridgeError> {
        self.fake.set_param(change)
    }
    fn transport(&mut self, control: TransportControl) -> Result<(), BridgeError> {
        self.fake.transport(control)
    }
    fn poll(&mut self, out: &mut EngineOutputs) {
        self.fake.poll(out)
    }
    fn descriptor(&mut self, device: DeviceId) -> Option<DeviceDescriptor> {
        self.fake.descriptor(device)
    }
    fn poll_plugins(&mut self, out: &mut Vec<(DeviceId, PluginNotification)>) {
        self.fake.poll_plugins(out)
    }
    fn plugin_state(&mut self, device: DeviceId) -> Result<Option<Base64Bytes>, BridgeError> {
        self.fake.plugin_state(device)
    }
    fn list_hardware_ports(&mut self) -> Result<HardwarePorts, BridgeError> {
        Ok(self.ports.clone())
    }
    fn measure_hw_latency(&mut self, node: NodeKey) -> Result<(), BridgeError> {
        self.requests.push(node);
        if let Some(samples) = self.answer {
            self.results.push_back(HwLatencyResult { node, samples });
        }
        Ok(())
    }
    fn poll_hw_latency(&mut self) -> Option<HwLatencyResult> {
        self.results.pop_front()
    }
}

struct H {
    ctl: EtherController<HwBridge, FakeHost, MemoryStore, MemoryLibrary>,
    next: u32,
    ids: IdGen,
}

impl H {
    fn new() -> Self {
        let mut store = MemoryStore::new();
        store.now_ms = T0;
        let bridge = HwBridge {
            ports: ports(true),
            ..Default::default()
        };
        let mut h = Self {
            ctl: EtherController::with_config(
                bridge,
                FakeHost { now: T0 },
                store,
                MemoryLibrary::new(),
                ControllerConfig::default(),
            ),
            next: 1,
            ids: IdGen::new(7),
        };
        let id = h.ids.next_project_id(T0);
        h.ok(Command::Project(
            ether_core::protocol::project::ProjectCommand::Create {
                id,
                name: "Ext".into(),
            },
        ));
        h
    }

    fn id<I: Id>(&mut self) -> I {
        self.ids.next(T0)
    }

    fn send(&mut self, command: Command) -> Vec<ServerMessage> {
        let id = self.next;
        self.next += 1;
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

    fn tick(&mut self) -> Vec<ServerMessage> {
        let mut out = Vec::new();
        let now = self.ctl.host.now;
        self.ctl.tick(now, &mut out);
        out
    }

    fn advance(&mut self, ms: u64) {
        self.ctl.host.now += ms;
        self.ctl.store.now_ms = self.ctl.host.now;
    }

    fn project(&self) -> &Project {
        self.ctl.project().expect("project")
    }

    fn track(&mut self, kind: TrackKind) -> TrackId {
        let id: TrackId = self.id();
        self.ok(Command::Track(TrackCommand::Create {
            id,
            kind,
            name: None,
            color: None,
            parent: None,
            before: None,
        }));
        id
    }

    fn insert(&mut self, track: TrackId, ty: BuiltinDeviceType) -> DeviceId {
        let id: DeviceId = self.id();
        self.ok(Command::Device(DeviceCommand::Insert {
            id,
            track,
            device: DeviceSpec::Builtin {
                device: BuiltinDevice::new(ty),
            },
            before: None,
        }));
        id
    }

    fn routing(&self, device: DeviceId) -> ExternalRouting {
        match &self.project().devices[&device].kind {
            DeviceKind::Builtin {
                device:
                    BuiltinDevice::ExternalInstrument { routing }
                    | BuiltinDevice::ExternalAudioEffect { routing },
            } => routing.clone(),
            other => panic!("not external: {other:?}"),
        }
    }
}

fn ports(with_synth: bool) -> HardwarePorts {
    let ch = |index, name: &str| AudioInputChannel {
        index,
        name: name.into(),
    };
    HardwarePorts {
        midi_outputs: if with_synth {
            vec![MidiPort {
                id: "Synth".into(),
                name: "Synth".into(),
            }]
        } else {
            vec![]
        },
        audio_inputs: vec![ch(0, "In 1"), ch(1, "In 2")],
        audio_outputs: vec![
            ch(0, "Out 1"),
            ch(1, "Out 2"),
            ch(2, "Out 3"),
            ch(3, "Out 4"),
        ],
    }
}

fn effect_routing() -> ExternalRouting {
    ExternalRouting {
        midi_out: None,
        midi_channel: 1,
        audio_send: Some(HwChannels { first: 2, count: 2 }),
        audio_return: Some(HwChannels { first: 0, count: 2 }),
    }
}

fn set_routing(device: DeviceId, routing: ExternalRouting) -> Command {
    Command::External(ExternalCommand::SetRouting { device, routing })
}

fn external_events(out: &[ServerMessage]) -> Vec<ExternalEvent> {
    events(out)
        .into_iter()
        .filter_map(|e| match e {
            Event::External { event } => Some(event),
            _ => None,
        })
        .collect()
}

#[test]
fn set_routing_validates_compiles_and_undoes() {
    let mut h = H::new();
    let t = h.track(TrackKind::Midi);
    let a = h.track(TrackKind::Audio);
    let i = h.insert(t, BuiltinDeviceType::ExternalInstrument);
    let e = h.insert(a, BuiltinDeviceType::ExternalAudioEffect);
    let other = h.insert(a, BuiltinDeviceType::Utility);
    let r = ExternalRouting {
        midi_out: Some("Synth".into()),
        midi_channel: 5,
        audio_send: None,
        audio_return: Some(HwChannels { first: 1, count: 1 }),
    };
    h.ok(set_routing(i, r.clone()));
    assert_eq!(h.routing(i), r);
    h.ok(set_routing(e, effect_routing()));
    h.tick();
    let g = h.ctl.bridge.fake.last_graph();
    let desc = g.tracks.iter().find(|x| x.id == a).unwrap();
    // Only the external device is compiled (chain order).
    assert_eq!(desc.hw_io.len(), 1);
    assert_eq!(desc.hw_io[0].routing, effect_routing());
    assert_eq!(desc.hw_io[0].node, desc.chain[0].node);

    // Validation.
    let bad = [
        (
            i,
            ExternalRouting {
                audio_send: Some(HwChannels { first: 0, count: 2 }),
                ..r.clone()
            },
        ),
        (
            e,
            ExternalRouting {
                midi_out: Some("Synth".into()),
                ..effect_routing()
            },
        ),
        (
            i,
            ExternalRouting {
                midi_channel: 17,
                ..r.clone()
            },
        ),
        (
            e,
            ExternalRouting {
                audio_return: Some(HwChannels { first: 0, count: 3 }),
                ..effect_routing()
            },
        ),
        (other, r.clone()),
    ];
    for (d, routing) in bad {
        let out = h.send(set_routing(d, routing.clone()));
        assert_eq!(err(&out).code, ErrorCode::InvalidArgument, "{routing:?}");
    }

    // One undo step each.
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(h.routing(e), ExternalRouting::default());
    assert_eq!(h.routing(i), r);
}

#[test]
fn list_ports_replies_the_hardware() {
    let mut h = H::new();
    match h.ok(Command::External(ExternalCommand::ListPorts)) {
        ReplyValue::HardwarePorts { ports: p } => assert_eq!(p, ports(true)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn measured_latency_sets_the_param_as_one_undoable_edit() {
    let mut h = H::new();
    let a = h.track(TrackKind::Audio);
    let e = h.insert(a, BuiltinDeviceType::ExternalAudioEffect);
    // Nothing routed yet.
    let out = h.send(Command::External(ExternalCommand::MeasureLatency {
        device: e,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidState);
    h.ok(set_routing(e, effect_routing()));
    h.tick();
    h.ctl.bridge.answer = Some(Some(480));
    assert_eq!(
        h.ok(Command::External(ExternalCommand::MeasureLatency {
            device: e
        })),
        ReplyValue::Unit
    );
    assert_eq!(h.ctl.bridge.requests.len(), 1);
    let out = h.tick();
    assert_eq!(
        external_events(&out),
        vec![ExternalEvent::LatencyMeasured {
            device: e,
            latency_ms: 10.0
        }]
    );
    assert_eq!(
        h.project().devices[&e].params.get(&fx::LATENCY),
        Some(&10.0)
    );
    // The engine got the new value (PDC follows through `Node::latency`).
    assert!(
        h.ctl
            .bridge
            .fake
            .param_changes()
            .iter()
            .any(|c| c.value == 10.0)
    );
    h.ok(Command::Edit(EditCommand::Undo));
    assert_ne!(
        h.project().devices[&e].params.get(&fx::LATENCY),
        Some(&10.0)
    );
    // The routing edit is the step before.
    assert_eq!(h.routing(e), effect_routing());
}

#[test]
fn instrument_measurement_needs_midi_out_and_return() {
    let mut h = H::new();
    let t = h.track(TrackKind::Midi);
    let i = h.insert(t, BuiltinDeviceType::ExternalInstrument);
    h.ok(set_routing(
        i,
        ExternalRouting {
            midi_out: Some("Synth".into()),
            ..ExternalRouting::default()
        },
    ));
    let out = h.send(Command::External(ExternalCommand::MeasureLatency {
        device: i,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidState);
    h.ok(set_routing(
        i,
        ExternalRouting {
            midi_out: Some("Synth".into()),
            audio_return: Some(HwChannels { first: 0, count: 1 }),
            ..ExternalRouting::default()
        },
    ));
    h.ctl.bridge.answer = Some(Some(2400));
    h.ok(Command::External(ExternalCommand::MeasureLatency {
        device: i,
    }));
    h.tick();
    assert_eq!(
        h.project().devices[&i].params.get(&inst::LATENCY),
        Some(&50.0)
    );
}

#[test]
fn failed_or_silent_measurements_report_measure_failed() {
    let mut h = H::new();
    let a = h.track(TrackKind::Audio);
    let e = h.insert(a, BuiltinDeviceType::ExternalAudioEffect);
    h.ok(set_routing(e, effect_routing()));
    // The engine gives up (nothing came back).
    h.ctl.bridge.answer = Some(None);
    h.ok(Command::External(ExternalCommand::MeasureLatency {
        device: e,
    }));
    let ev = external_events(&h.tick());
    assert!(matches!(ev.as_slice(), [ExternalEvent::MeasureFailed { device, .. }] if *device == e));
    assert_eq!(
        h.project().devices[&e]
            .params
            .get(&fx::LATENCY)
            .copied()
            .unwrap_or(0.0),
        0.0
    );
    // The engine never answers (audio stopped): the controller's deadline.
    h.ctl.bridge.answer = None;
    h.ok(Command::External(ExternalCommand::MeasureLatency {
        device: e,
    }));
    let out = h.send(Command::External(ExternalCommand::MeasureLatency {
        device: e,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidState, "one at a time");
    h.advance(1000);
    assert!(external_events(&h.tick()).is_empty());
    h.advance(2500);
    let ev = external_events(&h.tick());
    assert!(matches!(
        ev.as_slice(),
        [ExternalEvent::MeasureFailed { .. }]
    ));
    // Above the param range.
    h.ctl.bridge.answer = Some(Some(48_000));
    h.ok(Command::External(ExternalCommand::MeasureLatency {
        device: e,
    }));
    let ev = external_events(&h.tick());
    assert!(matches!(
        ev.as_slice(),
        [ExternalEvent::MeasureFailed { .. }]
    ));
}

#[test]
fn port_changes_are_announced_while_external_devices_exist() {
    let mut h = H::new();
    // No external device: no polling.
    h.ctl.bridge.ports = ports(false);
    h.advance(5000);
    assert!(external_events(&h.tick()).is_empty());
    let t = h.track(TrackKind::Midi);
    h.insert(t, BuiltinDeviceType::ExternalInstrument);
    h.tick(); // first listing: no event
    h.ctl.bridge.ports = ports(true);
    h.advance(500);
    assert!(external_events(&h.tick()).is_empty(), "polled every 2 s");
    h.advance(2000);
    assert_eq!(
        external_events(&h.tick()),
        vec![ExternalEvent::PortsChanged { ports: ports(true) }]
    );
    h.advance(2500);
    assert!(external_events(&h.tick()).is_empty(), "unchanged");
}

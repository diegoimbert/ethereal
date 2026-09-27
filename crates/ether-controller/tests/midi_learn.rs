//! MIDI learn / controller mappings (`midi-learn` node): learn mode, `Map`/`Edit`/`Unmap`/
//! `List`, every mapping mode driven by injected `MidiInputEvent`s, undo coalescing and
//! persistence.

mod common;

use common::*;
use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::{
    BridgeError, Controller, ControllerConfig, EngineBridge, EtherController, HostServices,
};
use ether_core::protocol::devices::{DeviceCommand, DeviceDescriptor, DeviceSpec, ParamScale};
use ether_core::protocol::midi_map::{MidiInputEvent, MidiMapCommand, MidiMapEvent};
use ether_core::protocol::mixer::MixerCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::project::{EditCommand, ProjectCommand};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::*;
use ether_core::{EngineOutputs, NodeKey, ParamChange, RenderGraphDesc, TransportControl};

// ─── A bridge with a MIDI input queue ──────────────────────────────────────────────────

#[derive(Default)]
struct MidiBridge {
    inner: FakeBridge,
    midi: Vec<MidiInputEvent>,
}

impl EngineBridge for MidiBridge {
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
        self.inner.create_plugin(device, plugin, state)
    }
    fn destroy_node(&mut self, key: NodeKey) -> Result<(), BridgeError> {
        self.inner.destroy_node(key)
    }
    fn load_media(
        &mut self,
        media: &MediaRef,
        audio: std::sync::Arc<ether_media::DecodedAudio>,
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
        self.inner.poll(out)
    }
    fn descriptor(&mut self, device: DeviceId) -> Option<DeviceDescriptor> {
        self.inner.descriptor(device)
    }
    fn poll_midi_input(&mut self, out: &mut Vec<MidiInputEvent>) {
        out.append(&mut self.midi);
    }
}

type MidiCtl = EtherController<MidiBridge, FakeHost, MemoryStore, MemoryLibrary>;

struct H {
    ctl: MidiCtl,
    ids: IdGen,
    next_request: u32,
}

const CC: u8 = 0xb0;
const NOTE_ON: u8 = 0x90;
const NOTE_OFF: u8 = 0x80;

impl H {
    fn new() -> Self {
        let mut store = MemoryStore::new();
        store.now_ms = T0;
        let mut h = Self {
            ctl: EtherController::with_config(
                MidiBridge::default(),
                FakeHost { now: T0 },
                store,
                MemoryLibrary::new(),
                ControllerConfig::default(),
            ),
            ids: IdGen::new(7),
            next_request: 1,
        };
        let id = h.ids.next_project_id(T0);
        h.ok(Command::Project(ProjectCommand::Create {
            id,
            name: "MIDI".into(),
        }));
        h
    }

    fn id<I: Id>(&mut self) -> I {
        self.ids.next(T0)
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

    fn advance(&mut self, ms: u64) {
        self.ctl.host.now += ms;
        self.ctl.store.now_ms = self.ctl.host.now;
    }

    fn tick(&mut self) -> Vec<ServerMessage> {
        let mut out = Vec::new();
        let now = self.ctl.host.now;
        self.ctl.tick(now, &mut out);
        out
    }

    /// Queue messages from `port` and tick once.
    fn midi(&mut self, port: &str, messages: &[[u8; 3]]) -> Vec<ServerMessage> {
        for data in messages {
            self.ctl.bridge.midi.push(MidiInputEvent {
                port: port.into(),
                data: *data,
                time_ms: self.ctl.host.now as f64,
            });
        }
        self.tick()
    }

    fn project(&self) -> &Project {
        self.ctl.project().unwrap()
    }

    fn track(&mut self, kind: TrackKind) -> TrackId {
        let id = self.id();
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

    fn map(
        &mut self,
        source: MidiSource,
        target: MidiMapTarget,
        mode: MidiMapMode,
    ) -> MidiMappingId {
        self.map_range(source, target, mode, 0.0, 1.0)
    }

    fn map_range(
        &mut self,
        source: MidiSource,
        target: MidiMapTarget,
        mode: MidiMapMode,
        min: f64,
        max: f64,
    ) -> MidiMappingId {
        let id = self.id();
        self.ok(Command::MidiMap(MidiMapCommand::Map {
            mapping: MidiMapping {
                id,
                source,
                target,
                min,
                max,
                mode,
            },
        }));
        id
    }

    fn undo(&mut self) {
        self.ok(Command::Edit(EditCommand::Undo));
    }

    fn volume(&self, track: TrackId) -> f32 {
        self.project().tracks[&track].mixer.volume.0
    }
}

fn cc(number: u8) -> MidiSource {
    MidiSource {
        port: None,
        channel: None,
        control: MidiControl::Cc { number },
    }
}

fn note(key: u8) -> MidiSource {
    MidiSource {
        port: None,
        channel: None,
        control: MidiControl::Note { key },
    }
}

fn volume(track: TrackId) -> MidiMapTarget {
    MidiMapTarget::Param {
        target: AutomationTarget::TrackVolume { track },
    }
}

fn midi_events(out: &[ServerMessage]) -> Vec<MidiMapEvent> {
    events(out)
        .into_iter()
        .filter_map(|e| match e {
            Event::MidiMap { event } => Some(event),
            _ => None,
        })
        .collect()
}

/// Track volume (dB) for a normalized fader position, as the controller maps it.
fn fader_db(n: f64) -> f32 {
    let m = ether_controller::compile::TRACK_VOLUME_MAPPING;
    ether_core::protocol::devices::scale_to_plain(m.scale, m.min, m.max, n) as f32
}

// ─── Learn ───────────────────────────────────────────────────────────────────────────

#[test]
fn learn_binds_the_next_control_in_one_undo_step() {
    let mut h = H::new();
    let t = h.track(TrackKind::Audio);
    let before = h.volume(t);
    let out = h.send(Command::MidiMap(MidiMapCommand::Learn {
        target: Some(volume(t)),
    }));
    ok(&out);
    assert_eq!(
        midi_events(&out),
        vec![MidiMapEvent::LearnChanged {
            target: Some(volume(t))
        }]
    );

    // A note-off never completes a learn; the next CC does.
    let out = h.midi("Knobs", &[[NOTE_OFF | 2, 60, 0], [CC | 2, 21, 100]]);
    let mappings: Vec<&MidiMapping> = h.project().midi_mappings.values().collect();
    assert_eq!(mappings.len(), 1);
    let m = mappings[0].clone();
    assert_eq!(
        m.source,
        MidiSource {
            port: Some("Knobs".into()),
            channel: Some(2),
            control: MidiControl::Cc { number: 21 },
        }
    );
    assert_eq!(
        (m.target.clone(), m.min, m.max, m.mode),
        (volume(t), 0.0, 1.0, MidiMapMode::Absolute)
    );
    assert_eq!(patches(&out).len(), 1);
    let evs = midi_events(&out);
    assert!(evs.contains(&MidiMapEvent::Learned { mapping: m.id }));
    assert!(evs.contains(&MidiMapEvent::LearnChanged { target: None }));
    // The learning message itself is not applied.
    assert_eq!(h.volume(t), before);

    // Now the knob drives the volume.
    h.advance(10);
    h.midi("Knobs", &[[CC | 2, 21, 127]]);
    assert_eq!(h.volume(t), fader_db(1.0));

    // `List` replies the mapping.
    match h.ok(Command::MidiMap(MidiMapCommand::List)) {
        ReplyValue::MidiMappings { mappings } => assert_eq!(mappings, vec![m.clone()]),
        other => panic!("{other:?}"),
    }

    // Undo: the volume move, then the learn.
    h.advance(1_000);
    h.tick();
    h.undo();
    assert_eq!(h.volume(t), before);
    h.undo();
    assert!(h.project().midi_mappings.is_empty());
}

#[test]
fn learn_replaces_mappings_of_the_target_and_of_the_source() {
    let mut h = H::new();
    let a = h.track(TrackKind::Audio);
    let b = h.track(TrackKind::Audio);
    let old_target = h.map(
        cc(1),
        MidiMapTarget::TrackMute { track: a },
        MidiMapMode::Toggle,
    );
    let concrete = MidiSource {
        port: Some("P".into()),
        channel: Some(0),
        control: MidiControl::Cc { number: 9 },
    };
    let old_source = h.map(concrete.clone(), volume(b), MidiMapMode::Absolute);

    h.ok(Command::MidiMap(MidiMapCommand::Learn {
        target: Some(MidiMapTarget::TrackMute { track: a }),
    }));
    h.midi("P", &[[CC, 9, 127]]);
    let p = h.project();
    assert!(!p.midi_mappings.contains_key(&old_target));
    assert!(!p.midi_mappings.contains_key(&old_source));
    let m = p.midi_mappings.values().next().unwrap();
    assert_eq!(p.midi_mappings.len(), 1);
    assert_eq!(m.source, concrete);
    // On/off targets learn as toggles.
    assert_eq!(m.mode, MidiMapMode::Toggle);
}

#[test]
fn learn_can_be_cancelled_and_note_learns_toggle() {
    let mut h = H::new();
    let t = h.track(TrackKind::Audio);
    h.ok(Command::MidiMap(MidiMapCommand::Learn {
        target: Some(volume(t)),
    }));
    let out = h.send(Command::MidiMap(MidiMapCommand::Learn { target: None }));
    assert_eq!(
        midi_events(&out),
        vec![MidiMapEvent::LearnChanged { target: None }]
    );
    h.midi("K", &[[CC, 1, 1]]);
    assert!(h.project().midi_mappings.is_empty());

    h.ok(Command::MidiMap(MidiMapCommand::Learn {
        target: Some(volume(t)),
    }));
    h.midi("K", &[[NOTE_ON, 36, 90]]);
    let m = h.project().midi_mappings.values().next().unwrap();
    assert_eq!(m.source.control, MidiControl::Note { key: 36 });
    assert_eq!(m.mode, MidiMapMode::Toggle);
}

#[test]
fn learn_is_cancelled_when_its_target_disappears() {
    let mut h = H::new();
    let t = h.track(TrackKind::Audio);
    h.ok(Command::MidiMap(MidiMapCommand::Learn {
        target: Some(MidiMapTarget::TrackSolo { track: t }),
    }));
    assert!(midi_events(&h.tick()).is_empty());
    h.ok(Command::Track(TrackCommand::Delete { id: t }));
    assert_eq!(
        midi_events(&h.tick()),
        vec![MidiMapEvent::LearnChanged { target: None }]
    );
    h.midi("K", &[[CC, 1, 64]]);
    assert!(h.project().midi_mappings.is_empty());
}

// ─── Document commands ───────────────────────────────────────────────────────────────

#[test]
fn map_edit_unmap_are_undoable_and_validated() {
    let mut h = H::new();
    let t = h.track(TrackKind::Audio);
    let a = h.map(cc(1), volume(t), MidiMapMode::Absolute);
    // Same source: replaced in the same step.
    let b = h.map(
        cc(1),
        MidiMapTarget::TrackMute { track: t },
        MidiMapMode::Toggle,
    );
    assert_eq!(h.project().midi_mappings.len(), 1);
    assert!(h.project().midi_mappings.contains_key(&b));
    h.undo();
    assert!(h.project().midi_mappings.contains_key(&a));
    h.ok(Command::Edit(EditCommand::Redo));

    h.ok(Command::MidiMap(MidiMapCommand::Edit {
        id: b,
        min: Some(0.25),
        max: None,
        mode: Some(MidiMapMode::Relative {
            encoding: RelativeEncoding::BinaryOffset,
        }),
    }));
    let m = &h.project().midi_mappings[&b];
    assert_eq!((m.min, m.max), (0.25, 1.0));
    assert_eq!(
        m.mode,
        MidiMapMode::Relative {
            encoding: RelativeEncoding::BinaryOffset
        }
    );
    h.undo();
    assert_eq!(h.project().midi_mappings[&b].min, 0.0);

    // Out-of-range values and unknown ids are rejected without changes.
    let out = h.send(Command::MidiMap(MidiMapCommand::Edit {
        id: b,
        min: Some(1.5),
        max: None,
        mode: None,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    let ghost: MidiMappingId = h.id();
    let out = h.send(Command::MidiMap(MidiMapCommand::Unmap { ids: vec![ghost] }));
    assert_eq!(err(&out).code, ErrorCode::NotFound);
    let missing: TrackId = h.id();
    let out = h.send(Command::MidiMap(MidiMapCommand::Map {
        mapping: MidiMapping {
            id: ghost,
            source: cc(2),
            target: volume(missing),
            min: 0.0,
            max: 1.0,
            mode: MidiMapMode::Absolute,
        },
    }));
    assert_eq!(err(&out).code, ErrorCode::NotFound);

    h.ok(Command::MidiMap(MidiMapCommand::Unmap { ids: vec![b, b] }));
    assert!(h.project().midi_mappings.is_empty());
    h.undo();
    assert!(h.project().midi_mappings.contains_key(&b));

    // Deleting the track cascades its mappings.
    h.ok(Command::Track(TrackCommand::Delete { id: t }));
    assert!(h.project().midi_mappings.is_empty());
}

// ─── Modes ───────────────────────────────────────────────────────────────────────────

#[test]
fn absolute_moves_coalesce_into_one_undo_step_per_movement() {
    let mut h = H::new();
    let t = h.track(TrackKind::Audio);
    let before = h.volume(t);
    h.map(cc(7), volume(t), MidiMapMode::Absolute);

    // One knob turn over several ticks (< 300 ms apart).
    for v in [10u8, 40, 80, 100] {
        h.advance(20);
        h.midi("K", &[[CC, 7, v]]);
    }
    assert_eq!(h.volume(t), fader_db(100.0 / 127.0));
    // The engine got the live values.
    assert!(!h.ctl.bridge.inner.param_changes().is_empty());
    // Pause: the gesture ends; a second movement is a second undo step.
    h.advance(400);
    h.tick();
    h.advance(10);
    h.midi("K", &[[CC, 7, 127]]);
    assert_eq!(h.volume(t), fader_db(1.0));
    h.advance(400);
    h.tick();

    h.undo();
    assert_eq!(h.volume(t), fader_db(100.0 / 127.0));
    h.undo();
    assert_eq!(h.volume(t), before);
    // The next undo removes the mapping itself (no step per message).
    h.undo();
    assert!(h.project().midi_mappings.is_empty());
}

#[test]
fn ranges_scale_and_invert() {
    let mut h = H::new();
    let t = h.track(TrackKind::Audio);
    let pan = MidiMapTarget::Param {
        target: AutomationTarget::TrackPan { track: t },
    };
    h.map_range(cc(10), pan, MidiMapMode::Absolute, 0.75, 0.25);
    h.midi("K", &[[CC, 10, 127]]);
    // Pan is linear -1..1: normalized 0.25 → -0.5.
    assert!((h.project().tracks[&t].mixer.pan.0 + 0.5).abs() < 1e-6);
    h.advance(20);
    h.midi("K", &[[CC, 10, 0]]);
    assert!((h.project().tracks[&t].mixer.pan.0 - 0.5).abs() < 1e-6);
}

#[test]
fn relative_encoders_step_device_params() {
    let mut h = H::new();
    let t = h.track(TrackKind::Audio);
    let device: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id: device,
        track: t,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::new(BuiltinDeviceType::Delay),
        },
        before: None,
    }));
    let desc = ether_devices::descriptor(BuiltinDeviceType::Delay);
    let info = desc
        .params
        .iter()
        .find(|p| p.labels.is_none() && matches!(p.scale, ParamScale::Linear))
        .expect("a linear delay param")
        .clone();
    let target = MidiMapTarget::Param {
        target: AutomationTarget::DeviceParam {
            device,
            param: info.id,
        },
    };
    h.map(
        cc(20),
        target,
        MidiMapMode::Relative {
            encoding: RelativeEncoding::TwosComplement,
        },
    );
    let start = info.to_normalized(info.default);
    // +3, +3, -1 (two's complement 127 = -1).
    h.midi("K", &[[CC, 20, 3], [CC, 20, 3], [CC, 20, 127]]);
    let value = h.project().devices[&device].params[&info.id];
    let expected = info.to_plain((start + 5.0 / 127.0).min(1.0));
    assert!((value - expected).abs() < 1e-9, "{value} vs {expected}");
    // All three messages are one undo step.
    h.advance(400);
    h.tick();
    h.undo();
    assert!(
        !h.project().devices[&device].params.contains_key(&info.id)
            || (h.project().devices[&device].params[&info.id] - info.default).abs() < 1e-9
    );
}

#[test]
fn toggle_flips_on_each_press() {
    let mut h = H::new();
    let t = h.track(TrackKind::Audio);
    h.map(
        note(60),
        MidiMapTarget::TrackMute { track: t },
        MidiMapMode::Toggle,
    );
    let mute = |h: &H| h.project().tracks[&t].mixer.mute;
    h.midi("K", &[[NOTE_ON, 60, 100]]);
    assert!(mute(&h));
    // Release: nothing.
    h.midi("K", &[[NOTE_OFF, 60, 0]]);
    assert!(mute(&h));
    h.midi("K", &[[NOTE_ON, 60, 100]]);
    assert!(!mute(&h));
    // A held button that keeps sending "on" does not re-toggle.
    h.midi("K", &[[NOTE_ON, 60, 100], [NOTE_ON, 60, 110]]);
    assert!(!mute(&h));

    // Toggle on a continuous param flips between min and max.
    let vol = h.map_range(cc(64), volume(t), MidiMapMode::Toggle, 0.0, 0.5);
    h.midi("K", &[[CC, 64, 127]]);
    assert_eq!(h.volume(t), fader_db(0.0));
    h.midi("K", &[[CC, 64, 0], [CC, 64, 127]]);
    assert_eq!(h.volume(t), fader_db(0.5));
    let _ = vol;
}

#[test]
fn absolute_on_off_targets_follow_the_threshold() {
    let mut h = H::new();
    let t = h.track(TrackKind::Audio);
    h.map(
        cc(30),
        MidiMapTarget::TrackSolo { track: t },
        MidiMapMode::Absolute,
    );
    let solo = |h: &H| h.project().tracks[&t].mixer.solo;
    h.midi("K", &[[CC, 30, 127]]);
    assert!(solo(&h));
    h.midi("K", &[[CC, 30, 100]]);
    assert!(solo(&h));
    h.midi("K", &[[CC, 30, 0]]);
    assert!(!solo(&h));

    // Record-arm is runtime state.
    let midi = h.track(TrackKind::Midi);
    h.map(
        note(1),
        MidiMapTarget::TrackArm { track: midi },
        MidiMapMode::Toggle,
    );
    h.midi("K", &[[NOTE_ON, 1, 127]]);
    assert_eq!(h.ctl.armed(), vec![midi]);
    h.midi("K", &[[NOTE_OFF, 1, 0], [NOTE_ON, 1, 127]]);
    assert!(h.ctl.armed().is_empty());
}

#[test]
fn transport_actions_fire_on_press() {
    let mut h = H::new();
    h.map(
        note(10),
        MidiMapTarget::Transport {
            action: TransportAction::TogglePlay,
        },
        MidiMapMode::Toggle,
    );
    h.map(
        cc(11),
        MidiMapTarget::Transport {
            action: TransportAction::ToggleLoop,
        },
        MidiMapMode::Absolute,
    );
    h.map(
        cc(12),
        MidiMapTarget::Transport {
            action: TransportAction::NextMarker,
        },
        MidiMapMode::Absolute,
    );
    h.midi("K", &[[NOTE_ON, 10, 127]]);
    assert!(
        h.ctl
            .bridge
            .inner
            .calls
            .contains(&Call::Transport(TransportControl::Play))
    );
    h.midi("K", &[[NOTE_OFF, 10, 0], [NOTE_ON, 10, 127]]);
    assert!(
        h.ctl
            .bridge
            .inner
            .calls
            .contains(&Call::Transport(TransportControl::Stop))
    );

    assert!(!h.project().settings.loop_enabled);
    h.midi("K", &[[CC, 11, 127], [CC, 11, 120]]);
    assert!(h.project().settings.loop_enabled);
    h.midi("K", &[[CC, 11, 0], [CC, 11, 127]]);
    assert!(!h.project().settings.loop_enabled);

    // No markers: nothing happens (and nothing fails).
    let calls = h.ctl.bridge.inner.calls.len();
    h.midi("K", &[[CC, 12, 127]]);
    assert!(
        !h.ctl.bridge.inner.calls[calls..]
            .iter()
            .any(|c| matches!(c, Call::Transport(TransportControl::Locate { .. })))
    );
}

#[test]
fn the_most_specific_source_wins() {
    let mut h = H::new();
    let a = h.track(TrackKind::Audio);
    let b = h.track(TrackKind::Audio);
    h.map(cc(5), volume(a), MidiMapMode::Absolute);
    h.map(
        MidiSource {
            port: Some("Pads".into()),
            channel: Some(9),
            control: MidiControl::Cc { number: 5 },
        },
        volume(b),
        MidiMapMode::Absolute,
    );
    let va = h.volume(a);
    h.midi("Pads", &[[CC | 9, 5, 127]]);
    assert_eq!((h.volume(a), h.volume(b)), (va, fader_db(1.0)));
    h.midi("Pads", &[[CC | 3, 5, 127]]);
    assert_eq!(h.volume(a), fader_db(1.0));
    // Unmapped controls and other messages are ignored.
    let before = h.project().clone();
    h.midi("Pads", &[[CC, 99, 127], [0xc0, 1, 0], [0xd0, 5, 0]]);
    assert_eq!(h.project(), &before);
}

#[test]
fn activity_is_throttled() {
    let mut h = H::new();
    let out = h.midi("K", &[[CC, 1, 1], [CC, 1, 2], [CC, 2, 3]]);
    let acts: Vec<MidiMapEvent> = midi_events(&out);
    assert_eq!(
        acts,
        vec![MidiMapEvent::Activity {
            source: MidiSource {
                port: Some("K".into()),
                channel: Some(0),
                control: MidiControl::Cc { number: 2 },
            }
        }]
    );
    h.advance(10);
    assert!(midi_events(&h.midi("K", &[[CC, 4, 1]])).is_empty());
    h.advance(100);
    assert_eq!(midi_events(&h.tick()).len(), 1);
    assert!(midi_events(&h.tick()).is_empty());
}

#[test]
fn send_levels_are_mappable() {
    let mut h = H::new();
    let t = h.track(TrackKind::Audio);
    let ret = h.track(TrackKind::Return);
    let send: SendId = h.id();
    h.ok(Command::Mixer(MixerCommand::CreateSend {
        id: send,
        from: t,
        to: ret,
        level: Decibels(-144.0),
        pre_fader: false,
    }));
    h.map(
        cc(3),
        MidiMapTarget::Param {
            target: AutomationTarget::SendLevel { send },
        },
        MidiMapMode::Absolute,
    );
    h.midi("K", &[[CC, 3, 127]]);
    assert!((h.project().sends[&send].level.0 - 6.0).abs() < 1e-4);
}

// ─── Persistence ─────────────────────────────────────────────────────────────────────

#[test]
fn mappings_persist_in_the_project_file() {
    let mut h = H::new();
    let t = h.track(TrackKind::Audio);
    let a = h.map_range(
        MidiSource {
            port: Some("Knobs".into()),
            channel: Some(4),
            control: MidiControl::PitchBend,
        },
        volume(t),
        MidiMapMode::Relative {
            encoding: RelativeEncoding::SignMagnitude,
        },
        0.9,
        0.1,
    );
    h.map(
        note(3),
        MidiMapTarget::Transport {
            action: TransportAction::PreviousMarker,
        },
        MidiMapMode::Toggle,
    );
    let saved = h.project().midi_mappings.clone();
    assert!(saved.contains_key(&a));

    // Through the .ether serializer.
    let json = ether_core::protocol::model::file::save(h.project(), "test").unwrap();
    let loaded = ether_core::protocol::model::file::load(&json).unwrap();
    assert_eq!(loaded.midi_mappings, saved);

    // Through the controller: save, open another project, reopen.
    let id = h.project().id;
    h.ok(Command::Project(ProjectCommand::Save));
    let other = h.ids.next_project_id(T0);
    h.ok(Command::Project(ProjectCommand::Create {
        id: other,
        name: "Other".into(),
    }));
    assert!(h.project().midi_mappings.is_empty());
    h.ok(Command::Project(ProjectCommand::Open { id }));
    assert_eq!(h.project().midi_mappings, saved);
}

#[test]
fn learn_needs_an_open_project() {
    let mut store = MemoryStore::new();
    store.now_ms = T0;
    let mut ctl: MidiCtl = EtherController::with_config(
        MidiBridge::default(),
        FakeHost { now: T0 },
        store,
        MemoryLibrary::new(),
        ControllerConfig::default(),
    );
    let mut out = Vec::new();
    ctl.handle(
        ClientMessage {
            id: 1,
            gesture: None,
            command: Command::MidiMap(MidiMapCommand::List),
        },
        &mut out,
    );
    assert_eq!(err(&out).code, ErrorCode::InvalidState);
    // Input without a project is harmless.
    ctl.bridge.midi.push(MidiInputEvent {
        port: "K".into(),
        data: [CC, 1, 1],
        time_ms: 0.0,
    });
    let mut out = Vec::new();
    ctl.tick(T0, &mut out);
    let _ = HostServices::now_ms(&ctl.host);
}

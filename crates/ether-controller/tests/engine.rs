//! Engine side: graph compilation and publishing, node lifecycle, live params, transport,
//! playhead/meters, plugins.

mod common;

use std::collections::BTreeMap;

use common::*;
use ether_controller::compile::{
    PAN_MAPPING, SEND_LEVEL_MAPPING, TRACK_VOLUME_MAPPING, track_param_info,
};
use ether_core::graph::{ClipContentDesc, ResolvedTarget};
use ether_core::plugin::PluginNotification;
use ether_core::protocol::automation::{AutomationCommand, PointSpec};
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::devices::{DeviceCategory, DeviceCommand, DeviceSpec, ParamScale};
use ether_core::protocol::meters::TrackMeter;
use ether_core::protocol::mixer::MixerCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteSpec};
use ether_core::protocol::plugins::{PluginCommand, PluginEvent};
use ether_core::protocol::project::{EditCommand, ProjectCommand};
use ether_core::protocol::recording::{RecordingCommand, RecordingEvent};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::transport::TransportCommand;
use ether_core::protocol::*;
use ether_core::{ParamChange, ParamTarget, PlayheadState, TransportControl};

fn track(h: &mut Harness, kind: TrackKind, parent: Option<TrackId>) -> TrackId {
    let id = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id,
        kind,
        name: None,
        color: None,
        parent,
        before: None,
    }));
    id
}

fn device(h: &mut Harness, track: TrackId, device: BuiltinDevice) -> DeviceId {
    let id = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id,
        track,
        device: DeviceSpec::Builtin { device },
        before: None,
    }));
    id
}

fn lane(
    h: &mut Harness,
    owner: AutomationOwner,
    target: AutomationTarget,
    points: &[(f64, f64)],
) -> AutomationLaneId {
    let id = h.id();
    h.ok(Command::Automation(AutomationCommand::CreateLane {
        id,
        owner,
        target,
    }));
    let points = points
        .iter()
        .map(|(t, v)| PointSpec {
            id: h.id(),
            time: Beats(*t),
            value: *v,
            curve: CurveShape::Linear,
        })
        .collect();
    h.ok(Command::Automation(AutomationCommand::AddPoints {
        lane: id,
        points,
    }));
    id
}

fn node_of(h: &Harness, d: DeviceId) -> ether_core::NodeKey {
    *h.ctl
        .bridge
        .live
        .iter()
        .find(|(_, dev)| **dev == d)
        .expect("device has a live node")
        .0
}

#[test]
fn compiles_groups_sends_and_automation() {
    let mut h = Harness::with_project();
    let master = h.project().master_track().id;
    let group = track(&mut h, TrackKind::Group, None);
    let a = track(&mut h, TrackKind::Audio, Some(group));
    let ret = track(&mut h, TrackKind::Return, None);
    let comp = device(&mut h, a, BuiltinDevice::Compressor);
    let send: SendId = h.id();
    h.ok(Command::Mixer(MixerCommand::CreateSend {
        id: send,
        from: a,
        to: ret,
        level: Decibels(-6.0),
        pre_fader: true,
    }));
    h.ok(Command::Mixer(MixerCommand::SetSolo {
        track: a,
        solo: true,
        exclusive: true,
    }));
    lane(
        &mut h,
        AutomationOwner::Track { track: a },
        AutomationTarget::TrackVolume { track: a },
        &[(0.0, 0.5), (4.0, 1.0)],
    );
    lane(
        &mut h,
        AutomationOwner::Track { track: a },
        AutomationTarget::SendLevel { send },
        &[(0.0, 0.25)],
    );
    lane(
        &mut h,
        AutomationOwner::Track { track: a },
        AutomationTarget::DeviceParam {
            device: comp,
            param: ParamId(0),
        },
        &[(2.0, 0.75)],
    );
    let disabled = lane(
        &mut h,
        AutomationOwner::Track { track: a },
        AutomationTarget::TrackPan { track: a },
        &[(0.0, 0.0)],
    );
    h.ok(Command::Automation(AutomationCommand::SetLaneEnabled {
        id: disabled,
        enabled: false,
    }));
    h.tick();

    let g = h.ctl.bridge.last_graph().clone();
    let td = |id: TrackId| g.tracks.iter().find(|t| t.id == id).unwrap().clone();
    assert_eq!(g.tracks.len(), 4);
    assert_eq!(td(a).output, Some(group));
    assert_eq!(td(a).group, Some(group));
    assert_eq!(td(group).output, Some(master));
    assert_eq!(td(ret).output, Some(master));
    assert_eq!(td(master).output, None);
    assert!(td(a).solo);
    let s = &td(a).sends[0];
    assert_eq!((s.id, s.to, s.pre_fader), (send, ret, true));
    assert!((s.level - 10f32.powf(-6.0 / 20.0)).abs() < 1e-6);
    let node = node_of(&h, comp);
    assert_eq!(td(a).chain.len(), 1);
    assert_eq!(td(a).chain[0].node, node);

    let auto = td(a).automation;
    assert_eq!(auto.len(), 3, "disabled lane skipped: {auto:#?}");
    let vol = auto
        .iter()
        .find(|x| x.resolved == ResolvedTarget::TrackVolume)
        .unwrap();
    assert_eq!(vol.mapping, TRACK_VOLUME_MAPPING);
    assert_eq!(
        vol.points.iter().map(|p| (p.0, p.1)).collect::<Vec<_>>(),
        vec![(0.0, 0.5), (4.0, 1.0)]
    );
    let snd = auto
        .iter()
        .find(|x| x.resolved == ResolvedTarget::Send { send })
        .unwrap();
    assert_eq!(snd.mapping, SEND_LEVEL_MAPPING);
    let dev = auto
        .iter()
        .find(|x| {
            x.resolved
                == ResolvedTarget::Node {
                    node,
                    param: ParamId(0),
                }
        })
        .unwrap();
    let info = &ether_devices::descriptor(BuiltinDeviceType::Compressor).params[0];
    assert_eq!(
        (dev.mapping.min, dev.mapping.max, dev.mapping.scale),
        (info.min, info.max, info.scale)
    );

    // The documented mixer mapping (shared with the UI).
    let vi = track_param_info(&AutomationTarget::TrackVolume { track: a }).unwrap();
    assert_eq!((vi.min, vi.max, vi.scale), (-144.0, 6.0, ParamScale::Fader));
    let pi = track_param_info(&AutomationTarget::TrackPan { track: a }).unwrap();
    assert_eq!(
        (pi.min, pi.max, pi.scale),
        (PAN_MAPPING.min, PAN_MAPPING.max, ParamScale::Linear)
    );
}

#[test]
fn compiles_clips_notes_and_envelopes() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi, None);
    let c: ClipId = h.id();
    h.ok(Command::Clip(ClipCommand::CreateMidi {
        id: c,
        track: t,
        start: Beats(4.0),
        length: Beats(8.0),
        name: None,
    }));
    let (n1, n2): (NoteId, NoteId) = (h.id(), h.id());
    h.ok(Command::Note(NoteCommand::Add {
        clip: c,
        notes: vec![
            NoteSpec {
                id: n1,
                pitch: 60,
                velocity: 0.5,
                start: Beats(1.0),
                duration: Beats(1.0),
            },
            NoteSpec {
                id: n2,
                pitch: 62,
                velocity: 0.5,
                start: Beats(0.0),
                duration: Beats(1.0),
            },
        ],
    }));
    h.ok(Command::Note(NoteCommand::Edit {
        edits: vec![ether_core::protocol::notes::NoteEdit {
            id: n2,
            pitch: None,
            velocity: None,
            start: None,
            duration: None,
            muted: Some(true),
        }],
    }));
    lane(
        &mut h,
        AutomationOwner::Clip { clip: c },
        AutomationTarget::TrackPan { track: t },
        &[(0.0, 0.0), (8.0, 1.0)],
    );
    h.tick();
    let g = h.ctl.bridge.last_graph();
    let td = g.tracks.iter().find(|x| x.id == t).unwrap();
    let cd = &td.clips[0];
    assert_eq!(
        (cd.start, cd.length, cd.offset, cd.looping),
        (4.0, 8.0, 0.0, None)
    );
    match &cd.content {
        ClipContentDesc::Midi { notes } => {
            assert_eq!(notes.len(), 1, "muted notes are not played");
            assert_eq!(notes[0].key, 60);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(cd.envelopes.len(), 1);
    assert_eq!(cd.envelopes[0].resolved, ResolvedTarget::TrackPan);
    assert_eq!(g.tempo[0].bpm, 120.0);
}

#[test]
fn continuous_controls_push_params_without_republish() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi, None);
    let synth = device(&mut h, t, BuiltinDevice::Synth);
    h.tick();
    let publishes = h.ctl.bridge.publishes();
    h.ok(Command::Mixer(MixerCommand::SetVolume {
        track: t,
        volume: Decibels(-6.0),
    }));
    h.ok(Command::Mixer(MixerCommand::SetPan {
        track: t,
        pan: Pan(-2.0),
    }));
    h.ok(Command::Device(DeviceCommand::SetParam {
        device: synth,
        param: ParamId(0),
        value: 1e9,
    }));
    h.advance(100);
    h.tick();
    assert_eq!(
        h.ctl.bridge.publishes(),
        publishes,
        "no republish for continuous controls"
    );
    let params = h.ctl.bridge.param_changes();
    assert!(
        matches!(params[0], ParamChange { target: ParamTarget::TrackVolume { track }, value } if track == t && (value - 0.501187).abs() < 1e-5)
    );
    assert!(
        matches!(params[1], ParamChange { target: ParamTarget::TrackPan { .. }, value } if value == -1.0)
    );
    let info = &ether_devices::descriptor(BuiltinDeviceType::Synth).params[0];
    let node = node_of(&h, synth);
    assert_eq!(
        params[2],
        ParamChange {
            target: ParamTarget::Node {
                node,
                param: ParamId(0)
            },
            value: info.max.max(info.min)
        }
    );
    // Undo also goes through the param queue.
    h.ok(Command::Edit(EditCommand::Undo));
    let last = *h.ctl.bridge.param_changes().last().unwrap();
    assert_eq!(
        last.target,
        ParamTarget::Node {
            node,
            param: ParamId(0)
        }
    );
    assert_eq!(last.value, info.default);
}

#[test]
fn node_lifecycle_follows_the_document() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi, None);
    let sampler = device(
        &mut h,
        t,
        BuiltinDevice::Sampler {
            sample: None,
            slices: Default::default(),
        },
    );
    let delay = device(&mut h, t, BuiltinDevice::Delay);
    h.tick();
    assert_eq!(h.ctl.bridge.live.len(), 2);
    let old = node_of(&h, delay);

    // Remove: destroyed only after a graph without it was published (asserted by the fake).
    h.ok(Command::Device(DeviceCommand::Remove { id: delay }));
    h.tick();
    assert!(!h.ctl.bridge.live.contains_key(&old));
    assert!(h.ctl.bridge.calls.contains(&Call::Destroy(old)));

    // Undo brings a fresh node back.
    h.ok(Command::Edit(EditCommand::Undo));
    h.tick();
    assert_eq!(h.ctl.bridge.live.len(), 2);

    // Disabling bypasses in the chain (no new node).
    h.ok(Command::Device(DeviceCommand::SetEnabled {
        id: delay,
        enabled: false,
    }));
    h.tick();
    let chain = &h
        .ctl
        .bridge
        .last_graph()
        .tracks
        .iter()
        .find(|x| x.id == t)
        .unwrap()
        .chain;
    assert_eq!(chain.len(), 2);
    assert!(!chain[1].enabled);

    // Changing a sampler's sample rebuilds it (no in-place swap).
    let before = node_of(&h, sampler);
    let media: MediaId = h.id();
    let out = h.send(Command::Device(DeviceCommand::SetSample {
        device: sampler,
        media: Some(media),
    }));
    assert_eq!(err(&out).code, ErrorCode::NotFound);
    h.ok(Command::Device(DeviceCommand::SetSample {
        device: sampler,
        media: None,
    }));
    h.tick();
    assert_eq!(node_of(&h, sampler), before, "same kind: node kept");

    // Deleting the track removes every node.
    h.ok(Command::Track(TrackCommand::Delete { id: t }));
    h.tick();
    assert!(h.ctl.bridge.live.is_empty());
}

#[test]
fn publishes_are_coalesced() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi, None);
    h.tick();
    let n = h.ctl.bridge.publishes();
    for i in 0..10 {
        h.ok(Command::Track(TrackCommand::Rename {
            id: t,
            name: format!("T{i}"),
        }));
    }
    assert_eq!(
        h.ctl.bridge.publishes(),
        n,
        "burst within the interval: deferred"
    );
    h.tick();
    assert_eq!(h.ctl.bridge.publishes(), n + 1);
    h.tick();
    assert_eq!(h.ctl.bridge.publishes(), n + 1, "nothing pending");
    // After the interval a command publishes right away.
    h.advance(1_000);
    h.ok(Command::Track(TrackCommand::Rename {
        id: t,
        name: "X".into(),
    }));
    assert_eq!(h.ctl.bridge.publishes(), n + 2);
}

#[test]
fn transport_playhead_and_meters() {
    let mut h = Harness::with_project();
    let out = h.send(Command::Transport(TransportCommand::Play));
    assert!(
        events(&out)
            .iter()
            .any(|e| matches!(e, Event::Transport { state } if state.playing))
    );
    assert!(
        h.ctl
            .bridge
            .calls
            .contains(&Call::Transport(TransportControl::Play))
    );

    let t = h.project().master_track().id;
    h.ctl.bridge.playhead = Some(PlayheadState {
        playing: true,
        recording: false,
        position: Beats(2.0),
        seconds: 1.0,
        bpm: 120.0,
        sample_time: 48_000,
    });
    h.ctl.bridge.meters = vec![TrackMeter {
        track: t,
        peak: [0.5, 0.5],
        rms: [0.2, 0.2],
        clipped: false,
    }];
    let out = h.tick();
    assert!(out.iter().any(|m| matches!(m, ServerMessage::Playhead(f) if f.transport.position == Beats(2.0) && f.transport.playing)));
    assert!(
        out.iter()
            .any(|m| matches!(m, ServerMessage::Meters(f) if f.tracks.len() == 1))
    );
    // Unchanged playhead: no new frame.
    let out = h.tick();
    assert!(!out.iter().any(|m| matches!(m, ServerMessage::Playhead(_))));

    // Stop, then Stop again returns to the start position.
    h.ok(Command::Transport(TransportCommand::Stop));
    h.ok(Command::Transport(TransportCommand::Stop));
    assert_eq!(
        h.ctl.bridge.calls.last(),
        Some(&Call::Transport(TransportControl::Locate {
            position: Beats(0.0)
        }))
    );
    // The engine stopping on its own is adopted.
    h.ok(Command::Transport(TransportCommand::Play));
    h.ctl.bridge.playhead.as_mut().unwrap().playing = false;
    let out = h.tick();
    assert!(
        events(&out)
            .iter()
            .any(|e| matches!(e, Event::Transport { state } if !state.playing))
    );
}

#[test]
fn tap_tempo_merges_into_one_step() {
    let mut h = Harness::with_project();
    for _ in 0..4 {
        h.ok(Command::Transport(TransportCommand::TapTempo));
        h.advance(500);
    }
    assert!((h.project().tempo_map().bpm_at(Beats(0.0)) - 120.0).abs() < 1e-9);
    for _ in 0..3 {
        h.ok(Command::Transport(TransportCommand::TapTempo));
        h.advance(400);
    }
    // New sequence after a pause? No: 400 ms gaps continue the same sequence.
    assert!(h.project().tempo_map().bpm_at(Beats(0.0)) > 120.0);
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(h.project().tempo_map().bpm_at(Beats(0.0)), 120.0);
}

#[test]
fn record_arm_is_runtime_state() {
    let mut h = Harness::with_project();
    let a = track(&mut h, TrackKind::Audio, None);
    let b = track(&mut h, TrackKind::Audio, None);
    h.tick();
    let out = h.send(Command::Recording(RecordingCommand::Arm {
        track: a,
        armed: true,
        exclusive: false,
    }));
    assert!(patches(&out).is_empty(), "arm is not a document edit");
    assert!(events(&out).contains(&Event::Recording {
        event: RecordingEvent::ArmChanged { armed: vec![a] }
    }));
    let out = h.send(Command::Recording(RecordingCommand::Arm {
        track: b,
        armed: true,
        exclusive: true,
    }));
    assert!(events(&out).contains(&Event::Recording {
        event: RecordingEvent::ArmChanged { armed: vec![b] }
    }));
    h.advance(100);
    h.tick();
    let td = h
        .ctl
        .bridge
        .last_graph()
        .tracks
        .iter()
        .find(|x| x.id == b)
        .unwrap()
        .clone();
    assert!(td.armed && td.monitor, "Auto monitoring follows arm");
    assert_eq!(td.audio_input, Some((0, 2)));
    // Deleting an armed track disarms it.
    let out = h.send(Command::Track(TrackCommand::Delete { id: b }));
    assert!(events(&out).contains(&Event::Recording {
        event: RecordingEvent::ArmChanged { armed: vec![] }
    }));
}

fn plugin_harness() -> Harness {
    let bridge = FakeBridge {
        plugins: Some(BTreeMap::from([(
            "com.test.Verb".to_string(),
            plugin_descriptor("Verb", DeviceCategory::AudioEffect),
        )])),
        ..Default::default()
    };
    let mut h = Harness::with(bridge, Default::default(), Default::default());
    h.create_project("Plugins");
    h
}

fn insert_plugin(h: &mut Harness, t: TrackId) -> DeviceId {
    let d: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id: d,
        track: t,
        device: DeviceSpec::Plugin {
            plugin_id: "com.test.Verb".into(),
            sandboxed: None,
            format: None,
        },
        before: None,
    }));
    d
}

#[test]
fn plugin_format_is_recorded_and_defaults_to_clap() {
    let mut h = plugin_harness();
    let t = track(&mut h, TrackKind::Audio, None);
    let clap = insert_plugin(&mut h, t);
    let vst3: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id: vst3,
        track: t,
        device: DeviceSpec::Plugin {
            plugin_id: "com.test.Verb".into(),
            sandboxed: None,
            format: Some(PluginFormat::Vst3),
        },
        before: None,
    }));
    let format = |h: &Harness, d: DeviceId| match &h.project().devices[&d].kind {
        DeviceKind::Plugin { plugin } => plugin.format,
        other => panic!("{other:?}"),
    };
    assert_eq!(format(&h, clap), PluginFormat::Clap);
    assert_eq!(format(&h, vst3), PluginFormat::Vst3);
}

#[test]
fn plugins_gui_edits_latency_crash_and_state() {
    let mut h = plugin_harness();
    let t = track(&mut h, TrackKind::Audio, None);
    let d = insert_plugin(&mut h, t);
    assert_eq!(h.project().devices[&d].name, "Verb");
    assert!(
        matches!(h.ctl.bridge.calls.iter().find(|c| matches!(c, Call::CreatePlugin(..))), Some(Call::CreatePlugin(dev, _, None)) if *dev == d)
    );
    h.tick();
    let creates = h
        .ctl
        .bridge
        .calls
        .iter()
        .filter(|c| matches!(c, Call::CreatePlugin(..)))
        .count();
    assert_eq!(creates, 1, "instantiated once (at insert)");

    // GUI drag = one undo step.
    let p = ParamId(7);
    h.ctl.bridge.plugin_notes = vec![
        (d, PluginNotification::GestureBegin { param: p }),
        (
            d,
            PluginNotification::ParamEdited {
                param: p,
                value: 10.0,
            },
        ),
        (
            d,
            PluginNotification::ParamEdited {
                param: p,
                value: 20.0,
            },
        ),
        (d, PluginNotification::GestureEnd { param: p }),
        (d, PluginNotification::LatencyChanged { samples: 64 }),
        (
            d,
            PluginNotification::Crashed {
                message: "boom".into(),
            },
        ),
    ];
    let publishes = h.ctl.bridge.publishes();
    let out = h.tick();
    assert_eq!(h.project().devices[&d].params[&p], 20.0);
    let evs = events(&out);
    assert!(evs.contains(&Event::Plugin {
        event: PluginEvent::LatencyChanged {
            device: d,
            samples: 64
        }
    }));
    assert!(evs.contains(&Event::Plugin {
        event: PluginEvent::Crashed {
            device: d,
            message: "boom".into()
        }
    }));
    assert_eq!(
        h.ctl.bridge.publishes(),
        publishes + 1,
        "latency change republishes"
    );
    assert!(
        h.ctl.bridge.param_changes().is_empty(),
        "GUI edits are not echoed back to the plugin"
    );
    h.ok(Command::Edit(EditCommand::Undo));
    assert!(!h.project().devices[&d].params.contains_key(&p));
    // Undo does reach the plugin (its node has no mirror entry: the default is sent).
    assert!(matches!(
        h.ctl.bridge.param_changes().last(),
        Some(ParamChange { target: ParamTarget::Node { param, .. }, value }) if *param == p && *value == 50.0
    ));

    // Descriptor query.
    let v = h.ok(Command::Device(DeviceCommand::GetDescriptor { device: d }));
    assert!(matches!(v, ReplyValue::Descriptor { descriptor } if descriptor.name == "Verb"));

    // Save reads live plugin state.
    h.ctl
        .bridge
        .plugin_states
        .insert(d, Base64Bytes(vec![1, 2, 3]));
    h.ok(Command::Project(ProjectCommand::Save));
    let id = h.project().id;
    let saved = std::str::from_utf8(h.ctl.store.file(id, "project.ether").unwrap())
        .unwrap()
        .to_string();
    let loaded = ether_core::protocol::model::file::load(&saved).unwrap();
    match &loaded.devices[&d].kind {
        DeviceKind::Plugin { plugin } => assert_eq!(plugin.state, Some(Base64Bytes(vec![1, 2, 3]))),
        other => panic!("{other:?}"),
    }
    // The in-memory document is not changed by saving.
    assert!(
        matches!(&h.project().devices[&d].kind, DeviceKind::Plugin { plugin } if plugin.state.is_none())
    );

    // Reload re-creates the node with the live state.
    h.ok(Command::Plugin(PluginCommand::Reload { device: d }));
    assert!(
        matches!(h.ctl.bridge.calls.iter().rev().find(|c| matches!(c, Call::CreatePlugin(..))), Some(Call::CreatePlugin(_, _, Some(s))) if s.0 == vec![1, 2, 3])
    );

    // Sandbox toggle is an undoable edit that rebuilds the node.
    h.ok(Command::Plugin(PluginCommand::SetSandboxed {
        device: d,
        sandboxed: true,
    }));
    assert!(
        matches!(&h.project().devices[&d].kind, DeviceKind::Plugin { plugin } if plugin.sandboxed)
    );
    h.tick();
    let creates = h
        .ctl
        .bridge
        .calls
        .iter()
        .filter(|c| matches!(c, Call::CreatePlugin(..)))
        .count();
    assert_eq!(creates, 3);
}

#[test]
fn plugins_unsupported_on_web() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Audio, None);
    let d: DeviceId = h.id();
    let out = h.send(Command::Device(DeviceCommand::Insert {
        id: d,
        track: t,
        device: DeviceSpec::Plugin {
            plugin_id: "com.test.Verb".into(),
            sandboxed: None,
            format: None,
        },
        before: None,
    }));
    assert_eq!(err(&out).code, ErrorCode::Unsupported);
    assert!(h.project().devices.is_empty());
}

#[test]
fn list_builtin_devices() {
    let mut h = Harness::with_project();
    let v = h.ok(Command::Device(DeviceCommand::ListBuiltin));
    assert!(
        matches!(v, ReplyValue::DeviceTypes { devices } if devices.len() == BuiltinDeviceType::ALL.len())
    );
}

fn midi_clip(h: &mut Harness, t: TrackId) -> ClipId {
    let c: ClipId = h.id();
    h.ok(Command::Clip(ClipCommand::CreateMidi {
        id: c,
        track: t,
        start: Beats(0.0),
        length: Beats(4.0),
        name: Some("C".into()),
    }));
    c
}

fn envelopes_of(h: &Harness, t: TrackId) -> Vec<ResolvedTarget> {
    let g = h.ctl.bridge.last_graph();
    let td = g.tracks.iter().find(|x| x.id == t).unwrap();
    td.clips
        .iter()
        .flat_map(|c| c.envelopes.iter().map(|e| e.resolved))
        .collect()
}

#[test]
fn clip_envelopes_follow_track_duplicates() {
    let mut h = Harness::with_project();
    let t = track(&mut h, TrackKind::Midi, None);
    let d = device(&mut h, t, BuiltinDevice::Synth);
    let c = midi_clip(&mut h, t);
    let owner = AutomationOwner::Clip { clip: c };
    lane(
        &mut h,
        owner,
        AutomationTarget::TrackVolume { track: t },
        &[(0.0, 0.5)],
    );
    lane(
        &mut h,
        owner,
        AutomationTarget::DeviceParam {
            device: d,
            param: ParamId(0),
        },
        &[(0.0, 0.5)],
    );
    let copy: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Duplicate {
        id: t,
        new_id: copy,
    }));
    h.tick();
    let new_dev = h.project().devices_of(copy)[0].id;
    let node = node_of(&h, new_dev);
    let env = envelopes_of(&h, copy);
    assert_eq!(env.len(), 2, "{env:?}");
    assert!(env.contains(&ResolvedTarget::TrackVolume));
    assert!(env.contains(&ResolvedTarget::Node {
        node,
        param: ParamId(0)
    }));
    assert_eq!(envelopes_of(&h, t).len(), 2);
}

#[test]
fn clip_envelopes_follow_cross_track_moves() {
    let mut h = Harness::with_project();
    let ret = track(&mut h, TrackKind::Return, None);
    let a = track(&mut h, TrackKind::Midi, None);
    let b = track(&mut h, TrackKind::Midi, None);
    let d = device(&mut h, a, BuiltinDevice::Synth);
    let (sa, sb): (SendId, SendId) = (h.id(), h.id());
    for (s, from) in [(sa, a), (sb, b)] {
        h.ok(Command::Mixer(MixerCommand::CreateSend {
            id: s,
            from,
            to: ret,
            level: Decibels(0.0),
            pre_fader: false,
        }));
    }
    let c = midi_clip(&mut h, a);
    let owner = AutomationOwner::Clip { clip: c };
    let pan = lane(
        &mut h,
        owner,
        AutomationTarget::TrackPan { track: a },
        &[(0.0, 0.2), (2.0, 0.8)],
    );
    lane(
        &mut h,
        owner,
        AutomationTarget::SendLevel { send: sa },
        &[(0.0, 0.5)],
    );
    lane(
        &mut h,
        owner,
        AutomationTarget::DeviceParam {
            device: d,
            param: ParamId(0),
        },
        &[(0.0, 0.5)],
    );
    // A clip envelope may only target its own track.
    let bad: AutomationLaneId = h.id();
    let out = h.send(Command::Automation(AutomationCommand::CreateLane {
        id: bad,
        owner,
        target: AutomationTarget::TrackVolume { track: b },
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);

    let before = h.project().clone();
    let out = h.send(Command::Clip(ClipCommand::Move {
        moves: vec![ether_core::protocol::clips::ClipMove {
            id: c,
            track: b,
            start: Beats(0.0),
        }],
    }));
    ok(&out);
    assert!(events(&out).iter().any(|e| matches!(
        e,
        Event::Notification {
            level: NotificationLevel::Warning,
            ..
        }
    )));
    let p = h.project();
    assert_eq!(
        p.automation_lanes[&pan].target,
        AutomationTarget::TrackPan { track: b }
    );
    assert_eq!(p.points_of(pan).len(), 2, "points kept");
    h.tick();
    let env = envelopes_of(&h, b);
    assert_eq!(env.len(), 2, "{env:?}");
    assert!(env.contains(&ResolvedTarget::TrackPan));
    assert!(env.contains(&ResolvedTarget::Send { send: sb }));
    // Undo restores the original envelopes exactly.
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(h.project(), &before);
}

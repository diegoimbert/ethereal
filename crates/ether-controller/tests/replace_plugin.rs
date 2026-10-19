//! `Resolve::Replace` (owner request: "swap plugins even if no error, with an option to
//! retain automations and parameters"): any plugin device switches plugin in one undo step,
//! keeping its id, position, sidechain source and enabled state.
//!
//! - retain, another plugin: values, lanes, modulation and MIDI mappings follow the
//!   parameters by normalized name (values through their normalized range); what has no
//!   match is dropped and reported; the new instance never gets the old plugin's state,
//!   and undo brings the old plugin back with its live state;
//! - retain, the same plugin: the live state and every parameter id are kept;
//! - no retain: defaults; parameter lanes and mappings go (the on/off lane stays).

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::*;
use ether_core::protocol::automation::{AutomationCommand, PointSpec};
use ether_core::protocol::devices::{
    DeviceCategory, DeviceCommand, DeviceDescriptor, DeviceSpec, DeviceTypeRef, ParamInfo,
    ParamScale, ParamUnit,
};
use ether_core::protocol::midi_map::MidiMapCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::racks::ModulationCommand;
use ether_core::protocol::resolver::{
    ParamMatch, ReplaceReport, ReplacementPlugin, ResolveCommand,
};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::{Command, ErrorCode, ReplyValue};

const ALPHA: &str = "com.test.Alpha";
const BETA: &str = "com.test.Beta";

fn param(id: u32, name: &str, min: f64, max: f64, default: f64) -> ParamInfo {
    ParamInfo {
        step: None,
        remote: None,
        id: ParamId(id),
        name: name.into(),
        group: None,
        unit: ParamUnit::None,
        min,
        max,
        default,
        scale: ParamScale::Linear,
        labels: None,
        automatable: true,
        hidden: false,
    }
}

fn descriptor(id: &str, name: &str, params: Vec<ParamInfo>) -> DeviceDescriptor {
    DeviceDescriptor {
        layout: None,
        device_type: DeviceTypeRef::Plugin {
            plugin_id: id.into(),
        },
        name: name.into(),
        category: DeviceCategory::AudioEffect,
        params,
        audio_inputs: 2,
        audio_outputs: 2,
        midi_input: false,
        sidechain_inputs: 2,
        implemented: true,
    }
}

struct Setup {
    h: Harness,
    device: DeviceId,
    other: DeviceId,
    sidechain: TrackId,
    cutoff_lane: AutomationLaneId,
    drive_lane: AutomationLaneId,
    switch_lane: AutomationLaneId,
    to_device: ModMappingId,
    midi: MidiMappingId,
}

fn track(h: &mut Harness) -> TrackId {
    let id = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id,
        kind: TrackKind::Audio,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    id
}

fn lane(h: &mut Harness, track: TrackId, target: AutomationTarget, value: f64) -> AutomationLaneId {
    let lane: AutomationLaneId = h.id();
    h.ok(Command::Automation(AutomationCommand::CreateLane {
        id: lane,
        owner: AutomationOwner::Track { track },
        target,
    }));
    let point = h.id();
    h.ok(Command::Automation(AutomationCommand::AddPoints {
        lane,
        points: vec![PointSpec {
            id: point,
            time: Beats(1.0),
            value,
            curve: CurveShape::Linear,
        }],
    }));
    lane
}

/// Alpha (Cutoff 0..100, Resonance 0..1, Drive 0..10) between two built-ins, bypassed, with
/// a sidechain, set values, lanes (two params and its on/off switch), a modulator mapped to
/// its Cutoff, and a MIDI mapping on Resonance. Beta has
/// "cutoff" (0..1000) and "RESONANCE" (0..100), no Drive.
fn setup() -> Setup {
    let bridge = FakeBridge {
        plugins: Some(BTreeMap::from([
            (
                ALPHA.to_string(),
                descriptor(
                    ALPHA,
                    "Alpha",
                    vec![
                        param(1, "Cutoff", 0.0, 100.0, 50.0),
                        param(2, "Resonance", 0.0, 1.0, 0.0),
                        param(3, "Drive", 0.0, 10.0, 0.0),
                    ],
                ),
            ),
            (
                BETA.to_string(),
                descriptor(
                    BETA,
                    "Beta",
                    vec![
                        param(10, "RESONANCE", 0.0, 100.0, 0.0),
                        param(11, "cutoff", 0.0, 1000.0, 500.0),
                        param(12, "Mix", 0.0, 1.0, 1.0),
                    ],
                ),
            ),
        ])),
        ..Default::default()
    };
    let mut h = Harness::with(bridge, Default::default(), Default::default());
    h.create_project("Replace plugin");
    let t = track(&mut h);
    let sidechain = track(&mut h);
    let builtin = |h: &mut Harness, ty: BuiltinDeviceType| {
        let id: DeviceId = h.id();
        h.ok(Command::Device(DeviceCommand::Insert {
            id,
            track: t,
            device: DeviceSpec::Builtin {
                device: BuiltinDevice::new(ty),
            },
            before: None,
        }));
        id
    };
    builtin(&mut h, BuiltinDeviceType::Compressor);
    let device: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id: device,
        track: t,
        device: DeviceSpec::Plugin {
            plugin_id: ALPHA.into(),
            sandboxed: None,
            format: Some(PluginFormat::Clap),
            name: None,
        },
        before: None,
    }));
    let other = builtin(&mut h, BuiltinDeviceType::Delay);
    h.tick();
    for (p, v) in [(1, 25.0), (2, 0.5), (3, 5.0)] {
        h.ok(Command::Device(DeviceCommand::SetParam {
            device,
            param: ParamId(p),
            value: v,
        }));
    }
    h.ok(Command::Device(DeviceCommand::SetSidechain {
        tap: None,
        device,
        source: Some(sidechain),
    }));
    h.ok(Command::Device(DeviceCommand::SetEnabled {
        id: device,
        enabled: false,
    }));
    let cutoff_lane = lane(
        &mut h,
        t,
        AutomationTarget::DeviceParam {
            device,
            param: ParamId(1),
        },
        0.75,
    );
    let drive_lane = lane(
        &mut h,
        t,
        AutomationTarget::DeviceParam {
            device,
            param: ParamId(3),
        },
        0.2,
    );
    let switch_lane = lane(&mut h, t, AutomationTarget::DeviceEnabled { device }, 1.0);
    let lfo: ModulatorId = h.id();
    h.ok(Command::Modulation(ModulationCommand::AddModulator {
        id: lfo,
        device,
        kind: ModulatorKind::Lfo,
        name: None,
    }));
    let map = |h: &mut Harness, target: DeviceId, param: ParamId| {
        let id: ModMappingId = h.id();
        h.ok(Command::Modulation(ModulationCommand::Map {
            id,
            source: ModSource::Modulator { modulator: lfo },
            device: target,
            param,
            depth: 0.5,
        }));
        id
    };
    let to_device = map(&mut h, device, ParamId(1));
    let midi: MidiMappingId = h.id();
    h.ok(Command::MidiMap(MidiMapCommand::Map {
        mapping: MidiMapping {
            id: midi,
            source: MidiSource {
                port: None,
                channel: None,
                control: MidiControl::Cc { number: 20 },
            },
            target: MidiMapTarget::Param {
                target: AutomationTarget::DeviceParam {
                    device,
                    param: ParamId(2),
                },
            },
            min: 0.0,
            max: 1.0,
            mode: MidiMapMode::Absolute,
        },
    }));
    // The running instance has a state the document hasn't saved yet.
    h.ctl
        .bridge
        .plugin_states
        .insert(device, Base64Bytes(b"alpha-live".to_vec()));
    Setup {
        h,
        device,
        other,
        sidechain,
        cutoff_lane,
        drive_lane,
        switch_lane,
        to_device,
        midi,
    }
}

fn beta() -> ReplacementPlugin {
    ReplacementPlugin {
        format: PluginFormat::Clap,
        plugin_id: BETA.into(),
        name: "Beta".into(),
        vendor: String::new(),
        version: "2.0".into(),
    }
}

fn replace(
    h: &mut Harness,
    device: DeviceId,
    to: ReplacementPlugin,
    retain: bool,
    preview: bool,
) -> ReplaceReport {
    match h.ok(Command::Resolve(ResolveCommand::Replace {
        device,
        plugin: to,
        retain,
        preview,
    })) {
        ReplyValue::PluginReplaced { report } => report,
        r => panic!("{r:?}"),
    }
}

fn plugin_of(h: &Harness, device: DeviceId) -> PluginInstance {
    match &h.project().devices[&device].kind {
        DeviceKind::Plugin { plugin } => plugin.clone(),
        k => panic!("{k:?}"),
    }
}

fn lane_ids(h: &Harness) -> BTreeSet<AutomationLaneId> {
    h.project().automation_lanes.keys().copied().collect()
}

fn created_with(h: &Harness, device: DeviceId) -> Option<Option<Base64Bytes>> {
    h.ctl.bridge.calls.iter().rev().find_map(|c| match c {
        Call::CreatePlugin(d, _, state) if *d == device => Some(state.clone()),
        _ => None,
    })
}

#[test]
fn retain_moves_values_lanes_and_mappings_by_name_in_one_undo_step() {
    let mut s = setup();
    let h = &mut s.h;
    let before = h.project().clone();

    // The preview reports and changes nothing.
    let preview = replace(h, s.device, beta(), true, true);
    assert_eq!(h.project(), &before);
    assert_eq!(
        preview,
        ReplaceReport {
            matched: ParamMatch::ByName,
            state_kept: false,
            kept: 2,
            lost: vec![
                "Plugin state (presets, loaded samples)".into(),
                "Drive".into()
            ],
            lanes_removed: 1,
        }
    );

    let report = replace(h, s.device, beta(), true, false);
    assert_eq!(report, preview);
    h.tick();
    let d = &h.project().devices[&s.device];
    let old = &before.devices[&s.device];
    assert_eq!(
        (d.track, &d.order, d.sidechain, d.enabled),
        (old.track, &old.order, Some(s.sidechain), false),
        "same place, sidechain and on/off state"
    );
    assert_eq!(
        d.name, "Beta",
        "named after the new plugin, as it was after the old one"
    );
    let p = plugin_of(h, s.device);
    assert_eq!((p.plugin_id.as_str(), p.state.clone()), (BETA, None));
    // Cutoff 25 of 0..100 → 250 of 0..1000; Resonance 0.5 of 0..1 → 50 of 0..100.
    assert_eq!(
        d.params,
        BTreeMap::from([(ParamId(11), 250.0), (ParamId(10), 50.0)])
    );
    // Lanes: Cutoff's follows (points unchanged), Drive's is gone, the switch stays.
    assert_eq!(lane_ids(h), BTreeSet::from([s.cutoff_lane, s.switch_lane]));
    assert_eq!(
        h.project().automation_lanes[&s.cutoff_lane].target,
        AutomationTarget::DeviceParam {
            device: s.device,
            param: ParamId(11)
        }
    );
    assert_eq!(h.project().automation_points.len(), 2);
    // Mappings follow.
    assert_eq!(h.project().mod_mappings[&s.to_device].param, ParamId(11));
    assert_eq!(h.project().modulators, before.modulators);
    assert_eq!(
        h.project().midi_mappings[&s.midi].target,
        MidiMapTarget::Param {
            target: AutomationTarget::DeviceParam {
                device: s.device,
                param: ParamId(10)
            }
        }
    );
    // The new instance starts from the document (no state), never Alpha's.
    assert_eq!(created_with(h, s.device), Some(None));
    assert_eq!(h.ctl.bridge.plugin_of[&s.device], BETA);

    // One undo step: everything back, Alpha with its live state.
    h.ok(Command::Edit(EditCommand::Undo));
    h.tick();
    let after_undo = h.project();
    assert_eq!(after_undo.automation_lanes, before.automation_lanes);
    assert_eq!(after_undo.automation_points, before.automation_points);
    assert_eq!(after_undo.mod_mappings, before.mod_mappings);
    assert_eq!(after_undo.midi_mappings, before.midi_mappings);
    assert_eq!(after_undo.modulators, before.modulators);
    let mut expected = before.devices[&s.device].clone();
    if let DeviceKind::Plugin { plugin } = &mut expected.kind {
        plugin.state = Some(Base64Bytes(b"alpha-live".to_vec()));
    }
    assert_eq!(after_undo.devices[&s.device], expected);
    assert_eq!(
        created_with(h, s.device),
        Some(Some(Base64Bytes(b"alpha-live".to_vec())))
    );
    assert_eq!(h.ctl.bridge.plugin_of[&s.device], ALPHA);
    // Redo applies it again.
    h.ok(Command::Edit(EditCommand::Redo));
    assert_eq!(plugin_of(h, s.device).plugin_id, BETA);
    let _ = s.drive_lane;
}

#[test]
fn without_retain_the_new_plugin_starts_at_defaults() {
    let mut s = setup();
    let h = &mut s.h;
    let before = h.project().clone();
    let report = replace(h, s.device, beta(), false, false);
    assert_eq!(
        report,
        ReplaceReport {
            matched: ParamMatch::Reset,
            state_kept: false,
            kept: 0,
            lost: vec![],
            lanes_removed: 2,
        }
    );
    let d = &h.project().devices[&s.device];
    assert!(d.params.is_empty(), "{:?}", d.params);
    assert_eq!((d.sidechain, d.enabled), (Some(s.sidechain), false));
    // Only the on/off lane stays (with its point); the modulator stays; mappings to the
    // device's params go.
    assert_eq!(lane_ids(h), BTreeSet::from([s.switch_lane]));
    assert_eq!(h.project().automation_points.len(), 1);
    assert_eq!(h.project().modulators, before.modulators);
    assert!(h.project().mod_mappings.is_empty());
    assert!(h.project().midi_mappings.is_empty());

    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(h.project().automation_lanes, before.automation_lanes);
    assert_eq!(h.project().automation_points, before.automation_points);
    assert_eq!(h.project().mod_mappings, before.mod_mappings);
    assert_eq!(h.project().midi_mappings, before.midi_mappings);
    assert_eq!(
        h.project().devices[&s.device].params,
        before.devices[&s.device].params
    );
}

#[test]
fn the_same_plugin_keeps_its_live_state_and_ids() {
    let mut s = setup();
    let h = &mut s.h;
    let before = h.project().clone();
    let report = replace(
        h,
        s.device,
        ReplacementPlugin {
            format: PluginFormat::Clap,
            plugin_id: ALPHA.into(),
            name: "Alpha".into(),
            vendor: String::new(),
            version: "1.1".into(),
        },
        true,
        false,
    );
    assert_eq!(report.matched, ParamMatch::SameIds);
    assert!(report.state_kept);
    assert_eq!(report.lost, Vec::<String>::new());
    assert_eq!(report.lanes_removed, 0);
    h.tick();
    assert_eq!(
        plugin_of(h, s.device).state,
        Some(Base64Bytes(b"alpha-live".to_vec()))
    );
    assert_eq!(
        h.project().devices[&s.device].params,
        before.devices[&s.device].params
    );
    assert_eq!(h.project().automation_lanes, before.automation_lanes);
    assert_eq!(h.project().mod_mappings, before.mod_mappings);
    assert_eq!(h.project().midi_mappings, before.midi_mappings);
    assert_eq!(
        created_with(h, s.device),
        Some(Some(Base64Bytes(b"alpha-live".to_vec())))
    );
}

#[test]
fn only_plugin_devices_are_replaced() {
    let mut s = setup();
    let e = err(&s.h.send(Command::Resolve(ResolveCommand::Replace {
        device: s.other,
        plugin: beta(),
        retain: true,
        preview: false,
    })));
    assert_eq!(e.code, ErrorCode::InvalidArgument);
    assert!(e.message.contains("is a built-in device"), "{}", e.message);
}

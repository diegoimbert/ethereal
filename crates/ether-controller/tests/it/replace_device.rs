//! `Resolve::Replace` for any device (owner request: "it should rather just be replace
//! device and not limited to plugins. And we should also be able to swap factory devices
//! for other plugins or devices"): a built-in or a plugin becomes another built-in or
//! plugin of the same role, in one undo step, keeping its id, place and on/off state.
//!
//! - roles: audio effect ↔ audio effect, instrument ↔ instrument, MIDI effect ↔ MIDI
//!   effect; racks and drum racks are never replaced nor replacements;
//! - retain: values, lanes, modulation, macro and MIDI mappings follow the parameters by
//!   normalized name (the same built-in type keeps everything); the rest is dropped and
//!   reported;
//! - the sidechain source stays when the new device has a sidechain input, else it goes
//!   (reported; undo brings it back).

use crate::common;

use std::collections::BTreeMap;

use common::*;
use ether_core::protocol::automation::{AutomationCommand, PointSpec};
use ether_core::protocol::devices::{DeviceCategory, DeviceCommand, DeviceSpec, ParamInfo};
use ether_core::protocol::midi_map::MidiMapCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::racks::{ModulationCommand, RackCommand};
use ether_core::protocol::resolver::{
    ParamMatch, ReplaceReport, ReplacementPlugin, ResolveCommand,
};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::{Command, CommandError, ErrorCode, ReplyValue};

const VERB: &str = "com.test.Verb";
const KEYS: &str = "com.test.Keys";

/// A built-in parameter by name.
fn info(ty: BuiltinDeviceType, name: &str) -> ParamInfo {
    ether_devices::descriptor(ty)
        .params
        .into_iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("{ty:?} has no {name}"))
}

fn pid(ty: BuiltinDeviceType, name: &str) -> ParamId {
    info(ty, name).id
}

/// Plain `value` of `from` moved to `to` through the normalized range.
fn moved(value: f64, from: &ParamInfo, to: &ParamInfo) -> f64 {
    to.snap(to.to_plain(from.to_normalized(value).clamp(0.0, 1.0)))
}

/// "Verb" (an audio effect plugin whose parameters share names with the built-in reverb's
/// first two, no sidechain input) and "Keys" (an instrument plugin).
fn harness() -> Harness {
    let mut verb = plugin_descriptor("Verb", DeviceCategory::AudioEffect);
    verb.device_type = ether_core::protocol::devices::DeviceTypeRef::Plugin {
        plugin_id: VERB.into(),
    };
    verb.sidechain_inputs = 0;
    verb.params = ether_devices::descriptor(BuiltinDeviceType::Reverb)
        .params
        .into_iter()
        .take(2)
        .enumerate()
        .map(|(i, p)| ParamInfo {
            id: ParamId(100 + i as u32),
            ..p
        })
        .collect();
    let mut keys = plugin_descriptor("Keys", DeviceCategory::Instrument);
    keys.device_type = ether_core::protocol::devices::DeviceTypeRef::Plugin {
        plugin_id: KEYS.into(),
    };
    let bridge = FakeBridge {
        plugins: Some(BTreeMap::from([
            (VERB.to_string(), verb),
            (KEYS.to_string(), keys),
        ])),
        ..Default::default()
    };
    let mut h = Harness::with(bridge, Default::default(), Default::default());
    h.create_project("Replace device");
    h
}

fn track(h: &mut Harness, kind: TrackKind) -> TrackId {
    let id = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id,
        kind,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    id
}

fn insert(h: &mut Harness, track: TrackId, device: DeviceSpec) -> DeviceId {
    let id: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id,
        track,
        device,
        before: None,
    }));
    id
}

fn builtin(ty: BuiltinDeviceType) -> DeviceSpec {
    DeviceSpec::Builtin {
        device: BuiltinDevice::new(ty),
    }
}

fn plugin_spec(id: &str) -> DeviceSpec {
    DeviceSpec::Plugin {
        plugin_id: id.into(),
        sandboxed: None,
        format: Some(PluginFormat::Clap),
        name: None,
    }
}

fn plugin(id: &str, name: &str) -> ReplacementPlugin {
    ReplacementPlugin {
        format: PluginFormat::Clap,
        plugin_id: id.into(),
        name: name.into(),
        vendor: "Test".into(),
        version: "1".into(),
    }
}

fn set(h: &mut Harness, device: DeviceId, param: ParamId, value: f64) {
    h.ok(Command::Device(DeviceCommand::SetParam {
        device,
        param,
        value,
    }));
}

fn lane(h: &mut Harness, track: TrackId, target: AutomationTarget) -> AutomationLaneId {
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
            value: 0.5,
            curve: CurveShape::Linear,
        }],
    }));
    lane
}

fn param_lane(
    h: &mut Harness,
    track: TrackId,
    device: DeviceId,
    param: ParamId,
) -> AutomationLaneId {
    lane(h, track, AutomationTarget::DeviceParam { device, param })
}

fn send_replace(
    h: &mut Harness,
    device: DeviceId,
    plugin: Option<ReplacementPlugin>,
    builtin: Option<BuiltinDeviceType>,
    retain: bool,
) -> Vec<ether_core::protocol::ServerMessage> {
    h.send(Command::Resolve(ResolveCommand::Replace {
        device,
        plugin,
        builtin,
        retain,
        preview: false,
    }))
}

fn with_builtin(
    h: &mut Harness,
    device: DeviceId,
    ty: BuiltinDeviceType,
    retain: bool,
) -> ReplaceReport {
    match ok(&send_replace(h, device, None, Some(ty), retain)) {
        ReplyValue::PluginReplaced { report } => report,
        r => panic!("{r:?}"),
    }
}

fn with_plugin(
    h: &mut Harness,
    device: DeviceId,
    to: ReplacementPlugin,
    retain: bool,
) -> ReplaceReport {
    match ok(&send_replace(h, device, Some(to), None, retain)) {
        ReplyValue::PluginReplaced { report } => report,
        r => panic!("{r:?}"),
    }
}

fn rejected(
    h: &mut Harness,
    device: DeviceId,
    plugin: Option<ReplacementPlugin>,
    builtin: Option<BuiltinDeviceType>,
) -> CommandError {
    let before = h.project().clone();
    let e = err(&send_replace(h, device, plugin, builtin, true));
    assert_eq!(e.code, ErrorCode::InvalidArgument, "{e:?}");
    assert_eq!(
        h.project(),
        &before,
        "a refused replacement changes nothing"
    );
    e
}

fn builtin_type(h: &Harness, device: DeviceId) -> Option<BuiltinDeviceType> {
    match &h.project().devices[&device].kind {
        DeviceKind::Builtin { device } => Some(device.device_type()),
        _ => None,
    }
}

#[test]
fn factory_to_factory_moves_values_lanes_and_mappings_by_name_in_one_undo_step() {
    use BuiltinDeviceType::{Compressor, Gate};
    let mut h = harness();
    let t = track(&mut h, TrackKind::Audio);
    let key = track(&mut h, TrackKind::Audio);
    insert(&mut h, t, builtin(BuiltinDeviceType::Eq));
    let comp = insert(&mut h, t, builtin(Compressor));
    insert(&mut h, t, builtin(BuiltinDeviceType::Delay));
    h.tick();
    let threshold = info(Compressor, "Threshold");
    let knee = info(Compressor, "Knee");
    set(&mut h, comp, threshold.id, -30.0);
    set(&mut h, comp, knee.id, (knee.min + knee.max) / 2.0);
    h.ok(Command::Device(DeviceCommand::SetSidechain {
        tap: None,
        device: comp,
        source: Some(key),
    }));
    h.ok(Command::Device(DeviceCommand::SetEnabled {
        id: comp,
        enabled: false,
    }));
    let threshold_lane = param_lane(&mut h, t, comp, threshold.id);
    let knee_lane = param_lane(&mut h, t, comp, knee.id);
    let switch_lane = lane(&mut h, t, AutomationTarget::DeviceEnabled { device: comp });
    let lfo: ModulatorId = h.id();
    h.ok(Command::Modulation(ModulationCommand::AddModulator {
        id: lfo,
        device: comp,
        kind: ModulatorKind::Lfo,
        name: None,
    }));
    let to_threshold: ModMappingId = h.id();
    h.ok(Command::Modulation(ModulationCommand::Map {
        id: to_threshold,
        source: ModSource::Modulator { modulator: lfo },
        device: comp,
        param: threshold.id,
        depth: 0.5,
    }));
    let midi: MidiMappingId = h.id();
    h.ok(Command::MidiMap(MidiMapCommand::Map {
        mapping: MidiMapping {
            id: midi,
            source: MidiSource {
                port: None,
                channel: None,
                control: MidiControl::Cc { number: 21 },
            },
            target: MidiMapTarget::Param {
                target: AutomationTarget::DeviceParam {
                    device: comp,
                    param: knee.id,
                },
            },
            min: 0.0,
            max: 1.0,
            mode: MidiMapMode::Absolute,
        },
    }));
    let before = h.project().clone();

    let report = with_builtin(&mut h, comp, Gate, true);
    assert_eq!(report.matched, ParamMatch::ByName);
    assert!(!report.state_kept);
    assert!(report.lost.iter().any(|l| l == "Knee"), "{report:?}");
    assert!(!report.lost.iter().any(|l| l == "Threshold"), "{report:?}");
    h.tick();

    let d = &h.project().devices[&comp];
    let old = &before.devices[&comp];
    assert_eq!(builtin_type(&h, comp), Some(Gate));
    assert_eq!(
        (d.track, &d.order, d.enabled, d.sidechain),
        (old.track, &old.order, false, Some(key)),
        "same place and on/off state; the gate has a sidechain input"
    );
    assert_eq!(
        d.name, "gate",
        "named after the new device, as it was after the old one"
    );
    let gate_threshold = info(Gate, "Threshold");
    assert_eq!(
        d.params[&gate_threshold.id],
        moved(-30.0, &threshold, &gate_threshold)
    );
    // Lanes: Threshold's follows, Knee's goes, the on/off lane stays.
    let lanes = &h.project().automation_lanes;
    assert_eq!(
        lanes[&threshold_lane].target,
        AutomationTarget::DeviceParam {
            device: comp,
            param: gate_threshold.id
        }
    );
    assert!(!lanes.contains_key(&knee_lane));
    assert!(lanes.contains_key(&switch_lane));
    assert_eq!(report.lanes_removed, 1);
    // The modulator stays and its mapping follows; the MIDI mapping on Knee goes.
    assert_eq!(h.project().modulators, before.modulators);
    assert_eq!(
        h.project().mod_mappings[&to_threshold].param,
        gate_threshold.id
    );
    assert!(!h.project().midi_mappings.contains_key(&midi));

    // One undo step puts the compressor back exactly; redo replaces it again.
    h.ok(Command::Edit(EditCommand::Undo));
    let p = h.project();
    assert_eq!(p.devices, before.devices);
    assert_eq!(p.automation_lanes, before.automation_lanes);
    assert_eq!(p.automation_points, before.automation_points);
    assert_eq!(p.mod_mappings, before.mod_mappings);
    assert_eq!(p.midi_mappings, before.midi_mappings);
    assert_eq!(p.modulators, before.modulators);
    h.ok(Command::Edit(EditCommand::Redo));
    assert_eq!(builtin_type(&h, comp), Some(Gate));
    assert_eq!(
        h.project().mod_mappings[&to_threshold].param,
        gate_threshold.id
    );
}

#[test]
fn a_device_without_a_sidechain_input_drops_the_source_and_says_so() {
    let mut h = harness();
    let t = track(&mut h, TrackKind::Audio);
    let key = track(&mut h, TrackKind::Audio);
    let comp = insert(&mut h, t, builtin(BuiltinDeviceType::Compressor));
    h.ok(Command::Device(DeviceCommand::SetSidechain {
        tap: None,
        device: comp,
        source: Some(key),
    }));
    let before = h.project().clone();
    let report = with_builtin(&mut h, comp, BuiltinDeviceType::Reverb, true);
    assert!(
        report.lost.iter().any(|l| l == "sidechain input"),
        "{report:?}"
    );
    let d = &h.project().devices[&comp];
    assert_eq!((d.sidechain, d.sidechain_tap), (None, None));
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(h.project().devices, before.devices);

    // A plugin without one (probed): the same, even without retaining.
    let report = with_plugin(&mut h, comp, plugin(VERB, "Verb"), false);
    assert_eq!(report.matched, ParamMatch::Reset);
    assert_eq!(report.lost, vec!["sidechain input".to_string()]);
    assert_eq!(h.project().devices[&comp].sidechain, None);
}

#[test]
fn factory_to_plugin_and_back() {
    use BuiltinDeviceType::Reverb;
    let mut h = harness();
    let t = track(&mut h, TrackKind::Audio);
    let verb = insert(&mut h, t, builtin(Reverb));
    let desc = ether_devices::descriptor(Reverb);
    let (first, second) = (desc.params[0].clone(), desc.params[1].clone());
    let v = (first.min + first.max) / 2.0;
    set(&mut h, verb, first.id, v);
    let l = param_lane(&mut h, t, verb, second.id);
    h.tick();
    let before = h.project().clone();

    // Built-in reverb → "Verb": the two shared parameters follow by name.
    let report = with_plugin(&mut h, verb, plugin(VERB, "Verb"), true);
    assert_eq!(report.matched, ParamMatch::ByName);
    h.tick();
    let d = &h.project().devices[&verb];
    match &d.kind {
        DeviceKind::Plugin { plugin } => {
            assert_eq!(
                (plugin.plugin_id.as_str(), plugin.state.clone()),
                (VERB, None)
            );
        }
        k => panic!("{k:?}"),
    }
    assert_eq!(d.name, "Verb");
    // Both shared parameters carry their values (an inserted built-in has them all).
    assert_eq!(
        d.params.keys().copied().collect::<Vec<_>>(),
        vec![ParamId(100), ParamId(101)]
    );
    assert!((d.params[&ParamId(100)] - v).abs() < 1e-9, "{:?}", d.params);
    assert_eq!(
        h.project().automation_lanes[&l].target,
        AutomationTarget::DeviceParam {
            device: verb,
            param: ParamId(101)
        }
    );
    assert_eq!(
        h.ctl.bridge.plugin_of[&verb], VERB,
        "the plugin instance runs"
    );

    // "Verb" → the built-in reverb again: its parameters come back by name.
    let report = with_builtin(&mut h, verb, Reverb, true);
    assert_eq!(report.matched, ParamMatch::ByName);
    let d = &h.project().devices[&verb];
    assert_eq!(builtin_type(&h, verb), Some(Reverb));
    assert_eq!(d.name, "reverb");
    assert!((d.params[&first.id] - v).abs() < 1e-9, "{:?}", d.params);
    assert_eq!(
        h.project().automation_lanes[&l].target,
        AutomationTarget::DeviceParam {
            device: verb,
            param: second.id
        }
    );

    // Two undo steps: back to the built-in as it was.
    h.ok(Command::Edit(EditCommand::Undo));
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(h.project().devices, before.devices);
    assert_eq!(h.project().automation_lanes, before.automation_lanes);
}

#[test]
fn instruments_are_replaced_by_instruments() {
    use BuiltinDeviceType::{PolySynth, Synth};
    let mut h = harness();
    let t = track(&mut h, TrackKind::Midi);
    let synth = insert(&mut h, t, builtin(Synth));
    let report = with_builtin(&mut h, synth, PolySynth, true);
    assert_ne!(report.matched, ParamMatch::Reset);
    assert_eq!(builtin_type(&h, synth), Some(PolySynth));
    let report = with_plugin(&mut h, synth, plugin(KEYS, "Keys"), false);
    assert_eq!(report.matched, ParamMatch::Reset);
    assert!(matches!(
        &h.project().devices[&synth].kind,
        DeviceKind::Plugin { plugin } if plugin.plugin_id == KEYS
    ));
    assert_eq!(h.project().devices[&synth].name, "Keys");
    with_builtin(&mut h, synth, Synth, false);
    assert_eq!(builtin_type(&h, synth), Some(Synth));
    assert_eq!(h.project().devices[&synth].name, "synth");
}

#[test]
fn a_renamed_device_keeps_its_name_and_the_same_type_keeps_everything() {
    use BuiltinDeviceType::Compressor;
    let mut h = harness();
    let t = track(&mut h, TrackKind::Audio);
    let comp = insert(&mut h, t, builtin(Compressor));
    h.ok(Command::Device(DeviceCommand::Rename {
        id: comp,
        name: "glue".into(),
    }));
    set(&mut h, comp, pid(Compressor, "Ratio"), 6.0);
    let before = h.project().devices[&comp].clone();
    let report = with_builtin(&mut h, comp, Compressor, true);
    assert_eq!(report.matched, ParamMatch::SameIds);
    assert!(report.state_kept);
    assert_eq!(report.lost, Vec::<String>::new());
    assert_eq!(h.project().devices[&comp], before);
    with_builtin(&mut h, comp, BuiltinDeviceType::Gate, true);
    assert_eq!(h.project().devices[&comp].name, "glue");
}

#[test]
fn macro_mappings_inside_a_rack_follow_or_go() {
    use BuiltinDeviceType::{AudioEffectRack, Compressor, Gate};
    let mut h = harness();
    let t = track(&mut h, TrackKind::Audio);
    let rack = insert(&mut h, t, builtin(AudioEffectRack));
    let chain: RackChainId = h.id();
    h.ok(Command::Rack(RackCommand::AddChain {
        id: chain,
        rack,
        name: None,
        before: None,
    }));
    let comp: DeviceId = h.id();
    h.ok(Command::Rack(RackCommand::InsertDevice {
        id: comp,
        chain,
        device: builtin(Compressor),
        before: None,
    }));
    let map = |h: &mut Harness, index: u8, param: ParamId| {
        let id: ModMappingId = h.id();
        h.ok(Command::Modulation(ModulationCommand::Map {
            id,
            source: ModSource::Macro { rack, index },
            device: comp,
            param,
            depth: 1.0,
        }));
        id
    };
    let to_threshold = map(&mut h, 0, pid(Compressor, "Threshold"));
    let to_knee = map(&mut h, 1, pid(Compressor, "Knee"));
    with_builtin(&mut h, comp, Gate, true);
    let d = &h.project().devices[&comp];
    assert_eq!(d.chain, Some(chain), "still on the rack chain");
    let maps = &h.project().mod_mappings;
    assert_eq!(maps[&to_threshold].param, pid(Gate, "Threshold"));
    assert!(!maps.contains_key(&to_knee));
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(
        h.project().mod_mappings[&to_knee].param,
        pid(Compressor, "Knee")
    );
}

#[test]
fn incompatible_replacements_are_refused() {
    use BuiltinDeviceType::*;
    let mut h = harness();
    let audio = track(&mut h, TrackKind::Audio);
    let midi = track(&mut h, TrackKind::Midi);
    let comp = insert(&mut h, audio, builtin(Compressor));
    let arp = insert(&mut h, midi, builtin(Arpeggiator));
    let synth = insert(&mut h, midi, builtin(Synth));
    let rack = insert(&mut h, audio, builtin(AudioEffectRack));
    let drums = insert(&mut h, midi, builtin(DrumRack));
    let verb = insert(&mut h, audio, plugin_spec(VERB));
    h.tick();

    // Roles.
    let e = rejected(&mut h, comp, None, Some(Synth));
    assert!(
        e.message.contains("an instrument can't replace it"),
        "{}",
        e.message
    );
    rejected(&mut h, comp, Some(plugin(KEYS, "Keys")), None);
    rejected(&mut h, synth, None, Some(Reverb));
    rejected(&mut h, synth, None, Some(Chord));
    rejected(&mut h, arp, None, Some(PolySynth));
    rejected(&mut h, arp, Some(plugin(VERB, "Verb")), None);
    rejected(&mut h, verb, None, Some(Synth));
    // Racks: never replaced, never a replacement.
    let e = rejected(&mut h, rack, None, Some(Reverb));
    assert!(e.message.contains("rack"), "{}", e.message);
    rejected(&mut h, drums, None, Some(Synth));
    rejected(&mut h, comp, None, Some(AudioEffectRack));
    rejected(&mut h, synth, None, Some(DrumRack));
    // Exactly one replacement.
    rejected(&mut h, comp, None, None);
    rejected(&mut h, comp, Some(plugin(VERB, "Verb")), Some(Reverb));
}

#[test]
fn the_preview_changes_nothing() {
    let mut h = harness();
    let t = track(&mut h, TrackKind::Audio);
    let comp = insert(&mut h, t, builtin(BuiltinDeviceType::Compressor));
    let before = h.project().clone();
    let r = h.ok(Command::Resolve(ResolveCommand::Replace {
        device: comp,
        plugin: None,
        builtin: Some(BuiltinDeviceType::Gate),
        retain: true,
        preview: true,
    }));
    assert!(
        matches!(r, ReplyValue::PluginReplaced { report } if report.matched == ParamMatch::ByName)
    );
    assert_eq!(h.project(), &before);
}

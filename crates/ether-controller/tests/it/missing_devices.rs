//! Missing devices (owner request: "automation of a device that was not imported" must not be
//! lost): a project holding a placeholder (`DeviceKind::Missing`, what an import leaves for a
//! device Ethereal has no equivalent for) opens without a node or an error for it, lists it
//! in `Resolve::Propose`, and `Resolve::Apply` / `Resolve::Replace` replace it with a
//! built-in device or a plugin, carrying its values and automation over by parameter name,
//! in one undo step.

use crate::common;

use std::collections::BTreeMap;

use common::*;
use ether_controller::store::ProjectStore;
use ether_core::graph::ResolvedTarget;
use ether_core::protocol::devices::DeviceCategory;
use ether_core::protocol::model::file;
use ether_core::protocol::model::*;
use ether_core::protocol::project::{EditCommand, ProjectCommand};
use ether_core::protocol::resolver::*;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::{Command, Event, NotificationLevel, ReplyValue};

struct Setup {
    h: Harness,
    track: TrackId,
    device: DeviceId,
    lane: AutomationLaneId,
    switch: AutomationLaneId,
}

fn missing() -> MissingDevice {
    MissingDevice {
        description: "Live's Hybrid Reverb".into(),
        source_id: "ableton:Hybrid".into(),
        origin: "Ableton Live 12.1".into(),
        role: MissingRole::AudioEffect,
        params: vec![
            MissingParam {
                id: ParamId(0),
                name: "DryWet".into(),
                min: 0.0,
                max: 1.0,
            },
            MissingParam {
                id: ParamId(1),
                name: "Vintage".into(),
                min: 0.0,
                max: 1.0,
            },
        ],
    }
}

/// An audio track with a missing device, a lane on its "DryWet" (0 → 0.5), one on its
/// "Vintage" and one on its on/off switch; stored and opened like any project.
fn setup(bridge: FakeBridge) -> Setup {
    let mut h = Harness::with(
        bridge,
        ether_controller::memory::MemoryLibrary::new(),
        Default::default(),
    );
    h.create_project("Missing");
    let track = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id: track,
        kind: TrackKind::Audio,
        name: Some("Vox".into()),
        color: None,
        parent: None,
        before: None,
    }));
    let mut p = h.project().clone();
    let device: DeviceId = h.id();
    p.devices.insert(
        device,
        Device {
            id: device,
            track,
            order: OrderKey::between(None, None),
            name: "Live's Hybrid Reverb".into(),
            enabled: true,
            kind: DeviceKind::Missing { missing: missing() },
            params: BTreeMap::from([(ParamId(0), 0.25), (ParamId(1), 0.5)]),
            sidechain: None,
            sidechain_tap: None,
            midi_sidechain: None,
            pad: None,
            chain: None,
        },
    );
    let mut lane_on = |h: &mut Harness, target: AutomationTarget, values: &[(f64, f64)]| {
        let lane: AutomationLaneId = h.id();
        p.automation_lanes.insert(
            lane,
            AutomationLane {
                id: lane,
                owner: AutomationOwner::Track { track },
                target,
                enabled: true,
            },
        );
        for (time, value) in values {
            let id: AutomationPointId = h.id();
            p.automation_points.insert(
                id,
                AutomationPoint {
                    id,
                    lane,
                    time: Beats(*time),
                    value: *value,
                    curve: CurveShape::Linear,
                },
            );
        }
        lane
    };
    let lane = lane_on(
        &mut h,
        AutomationTarget::DeviceParam {
            device,
            param: ParamId(0),
        },
        &[(0.0, 0.0), (4.0, 0.5)],
    );
    lane_on(
        &mut h,
        AutomationTarget::DeviceParam {
            device,
            param: ParamId(1),
        },
        &[(0.0, 1.0)],
    );
    let switch = lane_on(
        &mut h,
        AutomationTarget::DeviceEnabled { device },
        &[(2.0, 0.0)],
    );
    p.validate().unwrap();
    h.ok(Command::Project(ProjectCommand::Save { autosave: false }));
    let json = file::save(&p, "0.4.0").unwrap();
    h.ctl.store.save(p.id, &json).unwrap();
    let out = h.send(Command::Project(ProjectCommand::Open { id: p.id }));
    ok(&out);
    let out = h.tick();
    // No node, no "could not create" error for the placeholder.
    assert!(
        !events(&out).iter().any(|e| matches!(
            e,
            Event::Notification {
                level: NotificationLevel::Error,
                ..
            }
        )),
        "{out:?}"
    );
    Setup {
        h,
        track,
        device,
        lane,
        switch,
    }
}

fn chain_len(h: &mut Harness, track: TrackId) -> usize {
    h.tick();
    let g = h.ctl.bridge.last_graph();
    g.tracks.iter().find(|t| t.id == track).unwrap().chain.len()
}

fn propose(h: &mut Harness) -> ResolveProposal {
    match h.ok(Command::Resolve(ResolveCommand::Propose {
        installed: vec![],
        params: vec![],
        plugins: true,
    })) {
        ReplyValue::ResolveProposal { proposal } => proposal,
        r => panic!("{r:?}"),
    }
}

fn lanes_of(p: &Project, device: DeviceId) -> Vec<AutomationTarget> {
    let mut v: Vec<_> = p
        .automation_lanes
        .values()
        .filter(|l| l.target.device() == Some(device))
        .map(|l| l.target)
        .collect();
    v.sort_by_key(|t| format!("{t:?}"));
    v
}

#[test]
fn a_missing_device_opens_as_a_pass_through_and_is_proposed() {
    let mut s = setup(FakeBridge::default());
    assert_eq!(
        chain_len(&mut s.h, s.track),
        0,
        "no node for the placeholder"
    );
    let p = propose(&mut s.h);
    assert_eq!(p.devices.len(), 1);
    let g = &p.devices[0];
    assert_eq!(g.devices, vec![s.device]);
    assert_eq!(g.source_id, "ableton:Hybrid");
    assert_eq!(g.origin, "Ableton Live 12.1");
    assert_eq!(g.params.len(), 2);
    assert_eq!(g.params[0].name.as_deref(), Some("DryWet"));
    assert_eq!(
        g.candidates[0].replacement,
        Replacement::Builtin {
            device: BuiltinDeviceType::Reverb
        }
    );
}

#[test]
fn replacing_with_a_builtin_carries_values_and_automation_by_name() {
    let mut s = setup(FakeBridge::default());
    let before = s.h.project().clone();
    let out = s.h.send(Command::Resolve(ResolveCommand::Apply {
        plugins: vec![],
        media: vec![],
        devices: vec![DeviceResolution {
            devices: vec![s.device],
            replacement: Replacement::Builtin {
                device: BuiltinDeviceType::Reverb,
            },
        }],
    }));
    ok(&out);
    // "Vintage" has no Reverb parameter: reported.
    let warned = events(&out).iter().any(|e| {
        matches!(e, Event::Notification { level: NotificationLevel::Warning, message, .. }
            if message.contains("Vintage"))
    });
    assert!(warned, "{out:?}");
    let mix = ether_devices::reverb::params::MIX;
    let d = &s.h.project().devices[&s.device];
    assert!(matches!(
        d.kind,
        DeviceKind::Builtin {
            device: BuiltinDevice::Reverb
        }
    ));
    // Default names are lowercase.
    assert_eq!(d.name, "reverb", "named after its replacement");
    // DryWet 0.25 (0..1) → Mix 25 % (0..100).
    assert_eq!(d.params, BTreeMap::from([(mix, 25.0)]));
    // The DryWet lane follows to Mix with its points; the Vintage lane goes; the switch
    // lane stays.
    assert_eq!(
        lanes_of(s.h.project(), s.device),
        vec![
            AutomationTarget::DeviceEnabled { device: s.device },
            AutomationTarget::DeviceParam {
                device: s.device,
                param: mix
            },
        ]
    );
    let lane = &s.h.project().automation_lanes[&s.lane];
    assert_eq!(
        lane.target,
        AutomationTarget::DeviceParam {
            device: s.device,
            param: mix
        }
    );
    let points: Vec<f64> =
        s.h.project()
            .points_of(s.lane)
            .into_iter()
            .map(|p| p.value)
            .collect();
    assert_eq!(points, vec![0.0, 0.5]);
    assert!(s.h.project().automation_lanes.contains_key(&s.switch));

    // The automation plays on the new device: its node and its Mix param.
    assert_eq!(chain_len(&mut s.h, s.track), 1);
    let g = s.h.ctl.bridge.last_graph();
    let t = g.tracks.iter().find(|t| t.id == s.track).unwrap();
    let node = t.chain[0].node;
    assert!(t.automation.iter().any(|a| a.resolved
        == ResolvedTarget::Node { node, param: mix }
        && a.points.len() == 2));
    assert!(
        t.automation
            .iter()
            .any(|a| a.resolved == ResolvedTarget::NodeEnabled { node })
    );

    // One undo step puts the placeholder back with everything.
    s.h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(s.h.project().devices, before.devices);
    assert_eq!(s.h.project().automation_lanes, before.automation_lanes);
    assert_eq!(s.h.project().automation_points, before.automation_points);
    assert_eq!(chain_len(&mut s.h, s.track), 0);
    // And a saved, reopened project still has it to resolve.
    assert_eq!(propose(&mut s.h).devices.len(), 1);
}

#[test]
fn a_plugin_can_replace_a_missing_device() {
    let mut bridge = FakeBridge::default();
    let mut plugins = BTreeMap::new();
    // Its only parameter is "Mix" (id 7, 0..100 %).
    plugins.insert(
        "com.test.verb".to_string(),
        plugin_descriptor("Verb", DeviceCategory::AudioEffect),
    );
    bridge.plugins = Some(plugins);
    let mut s = setup(bridge);
    let to = ReplacementPlugin {
        format: PluginFormat::Clap,
        plugin_id: "com.test.verb".into(),
        name: "Verb".into(),
        vendor: "Test".into(),
        version: "1".into(),
    };
    let report = match s.h.ok(Command::Resolve(ResolveCommand::Replace {
        device: s.device,
        plugin: Some(to),
        builtin: None,
        retain: true,
        preview: false,
    })) {
        ReplyValue::PluginReplaced { report } => report,
        r => panic!("{r:?}"),
    };
    assert_eq!(report.matched, ParamMatch::ByName);
    assert_eq!(report.lanes_removed, 1);
    assert!(report.lost.iter().any(|l| l == "Vintage"), "{report:?}");
    let d = &s.h.project().devices[&s.device];
    let DeviceKind::Plugin { plugin } = &d.kind else {
        panic!("{:?}", d.kind)
    };
    assert_eq!(plugin.plugin_id, "com.test.verb");
    assert_eq!(d.params, BTreeMap::from([(ParamId(7), 25.0)]));
    assert_eq!(
        s.h.project().automation_lanes[&s.lane].target,
        AutomationTarget::DeviceParam {
            device: s.device,
            param: ParamId(7)
        }
    );
    assert_eq!(chain_len(&mut s.h, s.track), 1);
}

#[test]
fn an_instrument_cannot_replace_an_effect_on_an_audio_track() {
    let mut s = setup(FakeBridge::default());
    let out = s.h.send(Command::Resolve(ResolveCommand::Apply {
        plugins: vec![],
        media: vec![],
        devices: vec![DeviceResolution {
            devices: vec![s.device],
            replacement: Replacement::Builtin {
                device: BuiltinDeviceType::PolySynth,
            },
        }],
    }));
    let e = err(&out);
    assert!(e.message.contains("instrument"), "{}", e.message);
    assert!(matches!(
        s.h.project().devices[&s.device].kind,
        DeviceKind::Missing { .. }
    ));
}

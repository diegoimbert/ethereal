//! perf-plugins: plugin param edits on the wire (audit #4) and plugin-GUI edit coalescing
//! (audit #6). The `eprintln!`ed sizes are the numbers quoted in the PR.

use crate::common;

use std::collections::BTreeMap;

use common::*;
use ether_core::plugin::PluginNotification;
use ether_core::protocol::devices::{DeviceCategory, DeviceCommand, DeviceSpec, ParamInfo};
use ether_core::protocol::model::*;
use ether_core::protocol::plugins::PluginCommand;
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::*;

const PARAMS: u32 = 500;
const STATE_BYTES: usize = 512 * 1024;

/// A 500-param plugin, like a mid-size synth.
fn harness() -> Harness {
    let mut desc = plugin_descriptor("Synth", DeviceCategory::Instrument);
    let template = desc.params[0].clone();
    desc.params = (0..PARAMS)
        .map(|i| ParamInfo {
            id: ParamId(i),
            name: format!("P{i}"),
            ..template.clone()
        })
        .collect();
    let bridge = FakeBridge {
        plugins: Some(BTreeMap::from([("com.test.Synth".to_string(), desc)])),
        ..Default::default()
    };
    let mut h = Harness::with(bridge, Default::default(), Default::default());
    h.create_project("Plugins");
    h
}

/// A plugin device whose document holds every param and a 512 KiB state blob (as after a
/// project load, a sandbox toggle or a preset).
fn plugin_with_state(h: &mut Harness) -> DeviceId {
    let t: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id: t,
        kind: TrackKind::Midi,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    let d: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id: d,
        track: t,
        device: DeviceSpec::Plugin {
            plugin_id: "com.test.Synth".into(),
            sandboxed: None,
            format: None,
            name: None,
        },
        before: None,
    }));
    for p in 0..PARAMS {
        h.ok(Command::Device(DeviceCommand::SetParam {
            device: d,
            param: ParamId(p),
            value: 25.0,
        }));
    }
    h.ctl
        .bridge
        .plugin_states
        .insert(d, Base64Bytes(vec![0x5a; STATE_BYTES]));
    // Captures the live state into `PluginInstance.state`.
    h.ok(Command::Plugin(PluginCommand::SetSandboxed {
        device: d,
        sandboxed: true,
    }));
    match &h.project().devices[&d].kind {
        DeviceKind::Plugin { plugin } => {
            assert_eq!(plugin.state.as_ref().unwrap().0.len(), STATE_BYTES)
        }
        other => panic!("{other:?}"),
    }
    d
}

macro_rules! json_len {
    ($v:expr) => {
        serde_json::to_string($v).unwrap().len()
    };
}

#[test]
fn param_edits_patch_one_field_and_never_carry_plugin_state() {
    let mut h = harness();
    let d = plugin_with_state(&mut h);
    let whole = json_len!(&Entity::Device(h.project().devices[&d].clone()));

    let out = h.send(Command::Device(DeviceCommand::SetParam {
        device: d,
        param: ParamId(7),
        value: 30.0,
    }));
    let p = patches(&out);
    assert_eq!(p.len(), 1);
    assert_eq!(
        p[0].changes,
        vec![PatchChange::DeviceParam {
            id: d,
            param: ParamId(7),
            value: Some(30.0)
        }]
    );
    let bytes = json_len!(&p[0]);
    eprintln!("SetParam patch: {bytes} B (whole device with state: {whole} B)");
    assert!(whole > STATE_BYTES && bytes < 400, "{bytes} B");
    assert_eq!(h.project().devices[&d].params[&ParamId(7)], 30.0);

    // Undo of a param edit is field-level too.
    let p = patches(&h.send(Command::Edit(EditCommand::Undo)));
    assert!(matches!(
        &p[0].changes[..],
        [PatchChange::DeviceParam { id, param: ParamId(7), value: Some(v) }] if *id == d && *v == 25.0
    ));

    // Other device edits still upsert the device, without its state.
    let out = h.send(Command::Device(DeviceCommand::Rename {
        id: d,
        name: "Lead".into(),
    }));
    let p = patches(&out);
    let bytes = json_len!(&p[0]);
    eprintln!("Rename patch (500 params, state stripped): {bytes} B");
    assert!(bytes < 32 * 1024, "{bytes} B");
    match &p[0].changes[..] {
        [
            PatchChange::Upsert {
                entity: Entity::Device(dev),
            },
        ] => {
            assert_eq!(dev.name, "Lead");
            assert!(matches!(&dev.kind, DeviceKind::Plugin { plugin } if plugin.state.is_none()));
        }
        other => panic!("{other:?}"),
    }
    // The document keeps the state (saves and Project::Get still have it).
    assert!(
        matches!(&h.project().devices[&d].kind, DeviceKind::Plugin { plugin } if plugin.state.is_some())
    );

    // A connected client that predates DeviceParam: whole (stripped) devices again.
    let legacy = h.ctl.config().patch_compat.legacy_client();
    let p = patches(&h.send(Command::Device(DeviceCommand::SetParam {
        device: d,
        param: ParamId(8),
        value: 40.0,
    })));
    let bytes = json_len!(&p[0]);
    eprintln!("SetParam patch for a legacy client: {bytes} B");
    match &p[0].changes[..] {
        [
            PatchChange::Upsert {
                entity: Entity::Device(dev),
            },
        ] => {
            assert_eq!(dev.params[&ParamId(8)], 40.0);
            assert!(matches!(&dev.kind, DeviceKind::Plugin { plugin } if plugin.state.is_none()));
        }
        other => panic!("{other:?}"),
    }
    drop(legacy);
    let p = patches(&h.send(Command::Device(DeviceCommand::SetParam {
        device: d,
        param: ParamId(8),
        value: 41.0,
    })));
    assert!(matches!(
        &p[0].changes[..],
        [PatchChange::DeviceParam { .. }]
    ));
}

fn edited(d: DeviceId, param: u32, value: f64) -> (DeviceId, PluginNotification) {
    (
        d,
        PluginNotification::ParamEdited {
            param: ParamId(param),
            value,
        },
    )
}

fn undo_label(h: &mut Harness) -> Option<String> {
    // A no-op command's reply carries no patch; read the label through an undo/redo pair.
    let p = patches(&h.send(Command::Edit(EditCommand::Undo)));
    let label = p.last().and_then(|p| p.history.redo_label.clone());
    h.ok(Command::Edit(EditCommand::Redo));
    label
}

#[test]
fn plugin_gui_edits_coalesce_per_tick_and_group_into_one_undo_step() {
    let mut h = harness();
    let d = plugin_with_state(&mut h);
    let before_label = undo_label(&mut h);

    // A GUI preset switch without gestures: every param, some twice.
    h.ctl.bridge.calls.clear();
    let mut notes: Vec<_> = (0..PARAMS).map(|p| edited(d, p, 60.0)).collect();
    notes.extend((0..10).map(|p| edited(d, p, 70.0)));
    h.ctl.bridge.plugin_notes = notes;
    let out = h.tick();
    let p = patches(&out);
    assert_eq!(p.len(), 1, "one transaction for the poll");
    assert_eq!(p[0].changes.len(), PARAMS as usize, "one change per param");
    assert!(
        p[0].changes
            .iter()
            .all(|c| matches!(c, PatchChange::DeviceParam { .. }))
    );
    eprintln!(
        "preset switch ({PARAMS} params): 1 patch, {} B",
        json_len!(&p[0])
    );
    assert_eq!(h.project().devices[&d].params[&ParamId(3)], 70.0);
    assert_eq!(h.project().devices[&d].params[&ParamId(300)], 60.0);
    assert_eq!(p[0].history.undo_label.as_deref(), Some("plugin edit"));
    assert!(
        h.ctl.bridge.param_changes().is_empty(),
        "GUI edits are not echoed back to the plugin"
    );

    // More gesture-less edits within 250 ms join the same step...
    h.ctl.host.now += 100;
    h.ctl.bridge.plugin_notes = vec![edited(d, 400, 61.0)];
    h.tick();
    // ...and a pause closes it.
    h.ctl.host.now += 300;
    h.tick();
    h.ctl.host.now += 10;
    h.ctl.bridge.plugin_notes = vec![edited(d, 401, 62.0)];
    h.tick();

    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(h.project().devices[&d].params[&ParamId(401)], 60.0);
    assert_eq!(h.project().devices[&d].params[&ParamId(400)], 61.0);
    let p = patches(&h.send(Command::Edit(EditCommand::Undo)));
    // One undo reverts the whole preset switch (plus the edit that joined it).
    for (param, v) in [(3, 25.0), (300, 25.0), (400, 25.0)] {
        assert_eq!(h.project().devices[&d].params[&ParamId(param)], v);
    }
    assert_eq!(p[0].history.undo_label, before_label);

    // Edits inside a plugin gesture keep their own step and order around begin/end.
    h.ok(Command::Edit(EditCommand::Redo));
    h.ctl.host.now += 1_000;
    h.ctl.bridge.plugin_notes = vec![
        edited(d, 5, 1.0),
        (d, PluginNotification::GestureBegin { param: ParamId(6) }),
        edited(d, 6, 2.0),
        edited(d, 6, 3.0),
        (d, PluginNotification::GestureEnd { param: ParamId(6) }),
    ];
    let p = patches(&h.tick());
    assert_eq!(p.len(), 2);
    assert_eq!(p[0].history.undo_label.as_deref(), Some("plugin edit"));
    assert_eq!(p[1].history.undo_label.as_deref(), Some("set p6"));
    assert_eq!(
        p[1].changes,
        vec![PatchChange::DeviceParam {
            id: d,
            param: ParamId(6),
            value: Some(3.0)
        }]
    );
}

//! Plugin sidechains in the controller (`plugin-sidechain`, CONTRACTS §12.14): a plugin
//! device whose instance descriptor reports `sidechain_inputs > 0` takes a sidechain source
//! exactly like a built-in (validation, one undo step, compiled into `ChainEntry::
//! sidechain`, kept by save/reopen, cut when the source track is deleted); one without an
//! aux bus is refused.

mod common;

use std::collections::BTreeMap;

use common::*;
use ether_core::protocol::devices::{DeviceCategory, DeviceCommand, DeviceSpec};
use ether_core::protocol::model::*;
use ether_core::protocol::project::{EditCommand, ProjectCommand};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::{Command, ErrorCode};

const PLUGIN: &str = "com.test.Ducker";

fn harness(sidechain_inputs: u16) -> Harness {
    let mut desc = plugin_descriptor("Ducker", DeviceCategory::AudioEffect);
    desc.sidechain_inputs = sidechain_inputs;
    let bridge = FakeBridge {
        plugins: Some(BTreeMap::from([(PLUGIN.to_string(), desc)])),
        ..Default::default()
    };
    let mut h = Harness::with(bridge, Default::default(), Default::default());
    h.create_project("Plugin sidechain");
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

fn plugin(h: &mut Harness, track: TrackId, format: PluginFormat) -> DeviceId {
    let d: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id: d,
        track,
        device: DeviceSpec::Plugin {
            plugin_id: PLUGIN.into(),
            sandboxed: Some(format == PluginFormat::Vst3),
            format: Some(format),
        },
        before: None,
    }));
    d
}

fn set(device: DeviceId, source: Option<TrackId>) -> Command {
    Command::Device(DeviceCommand::SetSidechain { device, source })
}

fn compiled(h: &mut Harness, track: TrackId, index: usize) -> Option<TrackId> {
    h.tick();
    let g = h.ctl.bridge.last_graph();
    g.tracks.iter().find(|t| t.id == track).unwrap().chain[index].sidechain
}

#[test]
fn a_plugin_with_an_aux_bus_takes_a_sidechain_like_a_builtin() {
    let mut h = harness(2);
    let kick = track(&mut h, TrackKind::Audio);
    let bass = track(&mut h, TrackKind::Audio);
    // Every format and the sandboxed flag go through the same path.
    let clap = plugin(&mut h, bass, PluginFormat::Clap);
    let vst3 = plugin(&mut h, bass, PluginFormat::Vst3);
    let au = plugin(&mut h, bass, PluginFormat::Au);

    for (i, d) in [clap, vst3, au].into_iter().enumerate() {
        let out = h.send(set(d, Some(kick)));
        ok(&out);
        assert_eq!(patches(&out).len(), 1, "one patch");
        assert_eq!(h.project().devices[&d].sidechain, Some(kick));
        assert_eq!(compiled(&mut h, bass, i), Some(kick));
    }

    // One undo step per edit.
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(h.project().devices[&au].sidechain, None);
    assert_eq!(compiled(&mut h, bass, 2), None);
    h.ok(Command::Edit(EditCommand::Redo));
    assert_eq!(compiled(&mut h, bass, 2), Some(kick));

    // Its own track and the master are refused.
    let e = err(&h.send(set(clap, Some(bass))));
    assert_eq!(e.code, ErrorCode::InvalidArgument);

    // Save / reopen keeps it.
    let pid = h.project().id;
    h.ok(Command::Project(ProjectCommand::Save));
    h.ok(Command::Project(ProjectCommand::Open { id: pid }));
    assert_eq!(h.project().devices[&clap].sidechain, Some(kick));
    assert_eq!(compiled(&mut h, bass, 0), Some(kick));

    // Deleting the source cuts it.
    h.ok(Command::Track(TrackCommand::Delete { id: kick }));
    assert_eq!(h.project().devices[&clap].sidechain, None);
    assert_eq!(compiled(&mut h, bass, 0), None);
}

#[test]
fn a_plugin_without_an_aux_bus_is_refused() {
    let mut h = harness(0);
    let kick = track(&mut h, TrackKind::Audio);
    let bass = track(&mut h, TrackKind::Audio);
    let d = plugin(&mut h, bass, PluginFormat::Clap);
    let e = err(&h.send(set(d, Some(kick))));
    assert_eq!(e.code, ErrorCode::InvalidArgument);
    assert!(e.message.contains("no sidechain input"), "{}", e.message);
    // Clearing is always allowed.
    ok(&h.send(set(d, None)));
    assert_eq!(compiled(&mut h, bass, 0), None);
}

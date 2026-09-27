//! Roadmap v2 routing (contracts-2): every new command reaches its feature module and, until
//! the feature node implements it, replies `Unsupported` without changing the document.
//! Feature nodes replace the relevant assertions with real behaviour tests.

mod common;

use common::*;
use ether_core::protocol::collab::CollabCommand;
use ether_core::protocol::devices::{DeviceCommand, DeviceSpec};
use ether_core::protocol::model::*;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::{Command, ErrorCode};

/// Sends `c` and asserts it replies `Unsupported` without changing the document.
fn assert_unsupported(h: &mut Harness, c: Command) {
    let before = h.project().clone();
    let out = h.send(c.clone());
    assert_eq!(err(&out).code, ErrorCode::Unsupported, "{c:?}");
    assert!(patches(&out).is_empty(), "{c:?}");
    assert_eq!(h.project(), &before, "{c:?}");
}

// One test per feature node, so each node deletes only its own function when it lands
// (parallel removals from one shared list kept conflicting).

#[test]
fn collab_unsupported_until_implemented() {
    let mut h = Harness::with_project();
    assert_unsupported(&mut h, Command::Collab(CollabCommand::Leave));
}

#[test]
fn new_builtins_insert_and_compile_as_placeholders() {
    let mut h = Harness::with_project();
    let track: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id: track,
        kind: TrackKind::Midi,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    for ty in [
        BuiltinDeviceType::DrumRack,
        BuiltinDeviceType::Eq,
        BuiltinDeviceType::Reverb,
        BuiltinDeviceType::Limiter,
        BuiltinDeviceType::Utility,
    ] {
        let id: DeviceId = h.id();
        h.ok(Command::Device(DeviceCommand::Insert {
            id,
            track,
            device: DeviceSpec::Builtin {
                device: BuiltinDevice::new(ty),
            },
            before: None,
        }));
        let d = &h.project().devices[&id];
        assert_eq!((d.sidechain, d.pad), (None, None));
    }
    h.tick();
    let graph = h.ctl.bridge.last_graph();
    let t = graph.tracks.iter().find(|t| t.id == track).unwrap();
    assert_eq!(t.chain.len(), 5);
    assert!(t.chain.iter().all(|e| e.sidechain.is_none()));
    // The drum rack compiles to a rack desc (no pads yet).
    assert_eq!(t.racks.len(), 1);
    assert!(t.racks[0].pads.is_empty());
    // Click settings follow the project settings.
    let s = &h.project().settings;
    assert!((graph.click.volume - s.metronome_volume.to_linear()).abs() < 1e-6);
}

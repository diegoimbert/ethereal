//! Note Blocker (issue #192) through the controller: it goes before the instrument like every
//! MIDI effect, compiles into the track chain, and its `Block Everything` option survives a
//! save and reopen.

use crate::common;

use common::*;
use ether_core::protocol::devices::{DeviceCommand, DeviceSpec};
use ether_core::protocol::model::*;
use ether_core::protocol::project::ProjectCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::*;
use ether_devices::midi_fx::note_blocker;

fn insert(
    h: &mut Harness,
    track: TrackId,
    ty: BuiltinDeviceType,
    before: Option<DeviceId>,
) -> DeviceId {
    let id: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id,
        track,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::new(ty),
        },
        before,
    }));
    id
}

#[test]
fn note_blocker_goes_before_the_instrument_and_survives_save_and_reopen() {
    let mut h = Harness::with_project();
    let t: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id: t,
        kind: TrackKind::Midi,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    let synth = insert(&mut h, t, BuiltinDeviceType::Synth, None);
    let blocker = insert(&mut h, t, BuiltinDeviceType::NoteBlocker, Some(synth));
    // After the instrument: refused (MIDI effects precede it).
    let id: DeviceId = h.id();
    let out = h.send(Command::Device(DeviceCommand::Insert {
        id,
        track: t,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::NoteBlocker,
        },
        before: None,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);

    assert_eq!(
        h.project().devices[&blocker]
            .params
            .get(&note_blocker::BLOCK_EVERYTHING),
        Some(&0.0),
        "notes only by default"
    );
    h.ok(Command::Device(DeviceCommand::SetParam {
        device: blocker,
        param: note_blocker::BLOCK_EVERYTHING,
        value: 1.0,
    }));
    h.tick();
    let graph = h.ctl.bridge.last_graph();
    let track = graph.tracks.iter().find(|tr| tr.id == t).unwrap();
    assert_eq!(track.chain.len(), 2);

    let doc = h.project().clone();
    h.ok(Command::Project(ProjectCommand::Save { autosave: false }));
    h.create_project("Other");
    h.ok(Command::Project(ProjectCommand::Open { id: doc.id }));
    assert_eq!(h.project(), &doc);
    let d = &h.project().devices[&blocker];
    assert_eq!(
        d.kind,
        DeviceKind::Builtin {
            device: BuiltinDevice::NoteBlocker
        }
    );
    assert_eq!(d.params.get(&note_blocker::BLOCK_EVERYTHING), Some(&1.0));
}

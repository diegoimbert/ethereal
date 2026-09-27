//! Roadmap v2 routing (contracts-2): every new command reaches its feature module and, until
//! the feature node implements it, replies `Unsupported` without changing the document.
//! Feature nodes replace the relevant assertions with real behaviour tests.

mod common;

use common::*;
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::collab::CollabCommand;
use ether_core::protocol::devices::{DeviceCommand, DeviceSpec};
use ether_core::protocol::drum_rack::{DrumRackCommand, SliceCommand};
use ether_core::protocol::export::ExportCommand;
use ether_core::protocol::markers::MarkerCommand;
use ether_core::protocol::media::MediaCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::tempo::TempoCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::{Command, ErrorCode};

#[test]
fn new_domains_reply_unsupported_until_implemented() {
    let mut h = Harness::with_project();
    let before = h.project().clone();
    let marker: MarkerId = h.id();
    let pad: DrumPadId = h.id();
    let commands = vec![
        Command::Export(ExportCommand::Cancel { job: "j".into() }),
        Command::Tempo(TempoCommand::RemoveTempoPoints { ids: vec![] }),
        Command::Marker(MarkerCommand::Add {
            id: marker,
            position: Beats(4.0),
            name: None,
            color: None,
        }),
        Command::DrumRack(DrumRackCommand::RemovePad { id: pad }),
        Command::Slice(SliceCommand::Remove {
            device: h.id(),
            indices: vec![],
        }),
        Command::Collab(CollabCommand::Leave),
        Command::Media(MediaCommand::CancelUpload { upload: "u".into() }),
        Command::Clip(ClipCommand::SetReversed {
            id: h.id(),
            reversed: true,
        }),
        Command::Device(DeviceCommand::SetSidechain {
            device: h.id(),
            source: None,
        }),
    ];
    for c in commands {
        let out = h.send(c.clone());
        assert_eq!(err(&out).code, ErrorCode::Unsupported, "{c:?}");
        assert!(patches(&out).is_empty(), "{c:?}");
    }
    assert_eq!(h.project(), &before);
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
    assert!(t.racks.is_empty());
    // Click settings follow the project settings.
    let s = &h.project().settings;
    assert!((graph.click.volume - s.metronome_volume.to_linear()).abs() < 1e-6);
}

//! `DeviceCommand::SetMidiSidechain`: validation (devices without a note input, non-MIDI
//! sources, cycles), one undo step, compilation into `TrackDesc::midi_sidechains` (pre FX =
//! the chain's input, post FX = the notes reaching the first device that isn't a MIDI
//! effect, or the end of a chain of MIDI effects), the own track, the source-track delete
//! cascade and save/reopen.

use crate::common;

use common::*;
use ether_core::protocol::devices::{DeviceCommand, DeviceSpec};
use ether_core::protocol::model::*;
use ether_core::protocol::project::{EditCommand, ProjectCommand};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::{Command, ErrorCode};
use ether_core::{MidiPoint, MidiSidechainDesc, NodeKey};

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

fn device(h: &mut Harness, track: TrackId, ty: BuiltinDeviceType) -> DeviceId {
    let id = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id,
        track,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::new(ty),
        },
        before: None,
    }));
    id
}

fn set(device: DeviceId, source: Option<(TrackId, MidiTap)>) -> Command {
    Command::Device(DeviceCommand::SetMidiSidechain {
        device,
        source: source.map(|(track, tap)| MidiSidechain { track, tap }),
    })
}

/// The compiled MIDI sidechains of `track` and the nodes of its chain.
fn compiled(h: &mut Harness, track: TrackId) -> (Vec<MidiSidechainDesc>, Vec<NodeKey>) {
    h.tick();
    let g = h.ctl.bridge.last_graph();
    let t = g.tracks.iter().find(|t| t.id == track).unwrap();
    (
        t.midi_sidechains.clone(),
        t.chain.iter().map(|e| e.node).collect(),
    )
}

fn nodes(h: &mut Harness, track: TrackId) -> Vec<NodeKey> {
    compiled(h, track).1
}

#[test]
fn set_compiles_undoes_and_survives_a_reopen() {
    let mut h = Harness::with_project();
    let keys = track(&mut h, TrackKind::Midi);
    let arp = device(&mut h, keys, BuiltinDeviceType::Arpeggiator);
    let synth = device(&mut h, keys, BuiltinDeviceType::Synth);
    let vox = track(&mut h, TrackKind::Audio);
    let shifter = device(&mut h, vox, BuiltinDeviceType::Shifter);

    let out = h.send(set(shifter, Some((keys, MidiTap::PostFx))));
    ok(&out);
    assert_eq!(patches(&out).len(), 1, "one patch");
    let k = nodes(&mut h, keys);
    let (m, v) = compiled(&mut h, vox);
    // Post FX: after the arpeggiator, what the synth gets.
    assert_eq!(
        m,
        vec![MidiSidechainDesc {
            node: v[0],
            source: keys,
            point: MidiPoint::Before(k[1]),
        }]
    );
    let _ = (arp, synth);

    // Pre FX: the clips and the input.
    h.ok(set(shifter, Some((keys, MidiTap::PreFx))));
    assert_eq!(compiled(&mut h, vox).0[0].point, MidiPoint::Input);

    // Setting it again is no step; undo goes back one change at a time.
    assert!(patches(&h.send(set(shifter, Some((keys, MidiTap::PreFx))))).is_empty());
    h.ok(Command::Edit(EditCommand::Undo));
    assert_eq!(compiled(&mut h, vox).0[0].point, MidiPoint::Before(k[1]));
    h.ok(Command::Edit(EditCommand::Undo));
    assert!(compiled(&mut h, vox).0.is_empty());
    assert_eq!(h.project().devices[&shifter].midi_sidechain, None);
    h.ok(Command::Edit(EditCommand::Redo));

    // Save / reopen keeps it.
    let pid = h.project().id;
    h.ok(Command::Project(ProjectCommand::Save { autosave: false }));
    h.ok(Command::Project(ProjectCommand::Open { id: pid }));
    assert_eq!(
        h.project().devices[&shifter].midi_sidechain,
        Some(MidiSidechain {
            track: keys,
            tap: MidiTap::PostFx
        })
    );
    assert_eq!(compiled(&mut h, vox).0.len(), 1);

    // Deleting the source cuts it.
    h.ok(Command::Track(TrackCommand::Delete { id: keys }));
    assert_eq!(h.project().devices[&shifter].midi_sidechain, None);
    assert!(compiled(&mut h, vox).0.is_empty());
}

#[test]
fn post_fx_of_midi_effects_only_or_nothing_and_the_own_track() {
    let mut h = Harness::with_project();
    let arps = track(&mut h, TrackKind::Midi);
    device(&mut h, arps, BuiltinDeviceType::Arpeggiator);
    let empty = track(&mut h, TrackKind::Midi);
    let lead = track(&mut h, TrackKind::Midi);
    device(&mut h, lead, BuiltinDeviceType::Synth);
    let shifter = device(&mut h, lead, BuiltinDeviceType::Shifter);

    h.ok(set(shifter, Some((arps, MidiTap::PostFx))));
    assert_eq!(compiled(&mut h, lead).0[0].point, MidiPoint::End);
    h.ok(set(shifter, Some((empty, MidiTap::PostFx))));
    assert_eq!(compiled(&mut h, lead).0[0].point, MidiPoint::Input);
    // Its own track, after the synth: the notes the synth plays.
    h.ok(set(shifter, Some((lead, MidiTap::PostFx))));
    let (m, n) = compiled(&mut h, lead);
    assert_eq!(m[0].point, MidiPoint::Before(n[0]));
    assert_eq!(m[0].source, lead);
}

#[test]
fn refusals() {
    let mut h = Harness::with_project();
    let keys = track(&mut h, TrackKind::Midi);
    let audio = track(&mut h, TrackKind::Audio);
    let shifter = device(&mut h, audio, BuiltinDeviceType::Shifter);
    let comp = device(&mut h, audio, BuiltinDeviceType::Compressor);
    let synth = device(&mut h, keys, BuiltinDeviceType::Synth);

    // No note input: an audio effect that doesn't take notes, an instrument.
    for d in [comp, synth] {
        let e = err(&h.send(set(d, Some((keys, MidiTap::PostFx)))));
        assert_eq!(e.code, ErrorCode::InvalidArgument);
        assert!(e.message.contains("no note input"), "{}", e.message);
    }
    // Not a MIDI track.
    let other = track(&mut h, TrackKind::Audio);
    let e = err(&h.send(set(shifter, Some((other, MidiTap::PostFx)))));
    assert_eq!(e.code, ErrorCode::InvalidArgument);

    // A cycle: keys' shifter hears a track whose shifter hears keys.
    let b = track(&mut h, TrackKind::Midi);
    let sb = device(&mut h, b, BuiltinDeviceType::Shifter);
    let sk = device(&mut h, keys, BuiltinDeviceType::Shifter);
    h.ok(set(sb, Some((keys, MidiTap::PreFx))));
    let e = err(&h.send(set(sk, Some((b, MidiTap::PreFx)))));
    assert_eq!(e.code, ErrorCode::InvalidArgument, "{}", e.message);
    assert_eq!(h.project().devices[&sk].midi_sidechain, None);
    // Clearing is always allowed.
    ok(&h.send(set(comp, None)));
}

//! Drum rack and slicing commands (`drum-rack`): pads, pad chains, choke/mix/solo compiled
//! into `TrackDesc::racks`, slice editing, auto-slicing (transients on a synthetic signal,
//! grid, equal), `ToDrumRack` with client-chosen ids, one undo step each, save/reopen.

mod common;

use common::*;
use ether_controller::memory::MemoryLibrary;
use ether_core::graph::TrackDesc;
use ether_core::protocol::devices::{DeviceCommand, DeviceSpec};
use ether_core::protocol::drum_rack::{AutoSlice, DrumRackCommand, SliceCommand, SlicePadIds};
use ether_core::protocol::media::*;
use ether_core::protocol::model::*;
use ether_core::protocol::project::{EditCommand, ProjectCommand};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::*;
use ether_devices::sampler;

const SR: u32 = 48_000;
/// Onsets of the synthetic loop (seconds).
const HITS: [f64; 4] = [0.0, 0.25, 0.5, 0.875];
const LOOP_SECS: f64 = 1.2;

/// Decaying noise bursts at `HITS` on a quiet noise floor.
fn drum_loop() -> Vec<f32> {
    let n = (LOOP_SECS * SR as f64) as usize;
    let mut seed = 0x9e37_79b9u32;
    let mut noise = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32 * 2.0 - 1.0
    };
    let mut x: Vec<f32> = (0..n).map(|_| 0.001 * noise()).collect();
    for (i, t) in HITS.iter().enumerate() {
        let start = (t * SR as f64) as usize;
        let g = [0.9, 0.5, 0.7, 0.6][i];
        for (k, v) in x[start..].iter_mut().enumerate() {
            *v += g * (-(k as f32) / (0.03 * SR as f32)).exp() * noise();
        }
    }
    x
}

struct Kit {
    h: Harness,
    track: TrackId,
    rack: DeviceId,
    media: MediaId,
}

fn kit() -> Kit {
    let mut lib = MemoryLibrary::new();
    lib.add_root("lib", "Library");
    let x = drum_loop();
    lib.add_file("lib", "loop.wav", wav(SR, &[x.clone(), x]));
    let mut h = Harness::with(FakeBridge::default(), lib, Default::default());
    h.create_project("Kit");
    let track: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id: track,
        kind: TrackKind::Midi,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    let rack: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id: rack,
        track,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::DrumRack,
        },
        before: None,
    }));
    let media: MediaId = h.id();
    h.ok(Command::Media(MediaCommand::Import {
        id: media,
        source: MediaSource::Location {
            location: BrowseLocation::Library { id: "lib".into() },
            path: "loop.wav".into(),
        },
    }));
    h.drain_media();
    Kit {
        h,
        track,
        rack,
        media,
    }
}

fn rack_cmd(h: &mut Harness, c: DrumRackCommand) -> ReplyValue {
    h.ok(Command::DrumRack(c))
}

fn slice(h: &mut Harness, c: SliceCommand) -> ReplyValue {
    h.ok(Command::Slice(c))
}

fn undo(h: &mut Harness) {
    h.ok(Command::Edit(EditCommand::Undo));
}

fn graph_track(h: &mut Harness, track: TrackId) -> TrackDesc {
    h.tick();
    h.ctl
        .bridge
        .last_graph()
        .tracks
        .iter()
        .find(|t| t.id == track)
        .cloned()
        .unwrap()
}

fn node_of(h: &Harness, device: DeviceId) -> ether_core::NodeKey {
    *h.ctl
        .bridge
        .live
        .iter()
        .find(|(_, d)| **d == device)
        .expect("device has a node")
        .0
}

fn markers(h: &Harness, device: DeviceId) -> Vec<f64> {
    match &h.project().devices[&device].kind {
        DeviceKind::Builtin {
            device: BuiltinDevice::Sampler { slices, .. },
        } => slices.markers.iter().map(|m| m.0).collect(),
        other => panic!("not a sampler: {other:?}"),
    }
}

fn sampler_on_track(k: &mut Kit) -> DeviceId {
    let s: DeviceId = k.h.id();
    k.h.ok(Command::Device(DeviceCommand::Insert {
        id: s,
        track: k.track,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::Sampler {
                sample: Some(k.media),
                slices: SliceSettings::default(),
            },
        },
        before: None,
    }));
    s
}

#[test]
fn pads_and_pad_chains_compile_into_the_rack_desc() {
    let Kit {
        mut h,
        track,
        rack,
        media,
    } = kit();
    let (kick, snare): (DrumPadId, DrumPadId) = (h.id(), h.id());
    let (kick_dev, synth, fx): (DeviceId, DeviceId, DeviceId) = (h.id(), h.id(), h.id());
    rack_cmd(
        &mut h,
        DrumRackCommand::AddSamplePad {
            pad: kick,
            device: kick_dev,
            rack,
            note: 36,
            media,
        },
    );
    rack_cmd(
        &mut h,
        DrumRackCommand::AddPad {
            id: snare,
            rack,
            note: 38,
            name: None,
        },
    );
    rack_cmd(
        &mut h,
        DrumRackCommand::InsertDevice {
            id: synth,
            pad: snare,
            device: DeviceSpec::Builtin {
                device: BuiltinDevice::Synth,
            },
            before: None,
        },
    );
    rack_cmd(
        &mut h,
        DrumRackCommand::InsertDevice {
            id: fx,
            pad: snare,
            device: DeviceSpec::Builtin {
                device: BuiltinDevice::Delay,
            },
            before: None,
        },
    );
    // Names: the sample's file stem, the note name (C3 = 60).
    assert_eq!(h.project().drum_pads[&kick].name, "loop");
    assert_eq!(h.project().drum_pads[&snare].name, "D1");
    // Pad devices are not on the track chain.
    assert_eq!(h.project().devices_of(track).len(), 1);

    rack_cmd(
        &mut h,
        DrumRackCommand::SetChokeGroup {
            id: kick,
            group: Some(2),
        },
    );
    rack_cmd(
        &mut h,
        DrumRackCommand::SetPadVolume {
            id: snare,
            volume: Decibels(-6.0),
        },
    );
    rack_cmd(
        &mut h,
        DrumRackCommand::SetPadPan {
            id: snare,
            pan: Pan(-0.5),
        },
    );
    let t = graph_track(&mut h, track);
    assert_eq!(t.chain.len(), 1);
    assert_eq!(t.racks.len(), 1);
    let r = &t.racks[0];
    assert_eq!(r.rack, node_of(&h, rack));
    assert_eq!(r.pads.len(), 2);
    let (k, s) = (&r.pads[0], &r.pads[1]);
    assert_eq!((k.pad, k.note, k.choke_group), (kick, 36, Some(2)));
    assert_eq!(k.chain.len(), 1);
    assert_eq!(k.chain[0].node, node_of(&h, kick_dev));
    assert_eq!((s.pad, s.note), (snare, 38));
    assert_eq!(
        s.chain.iter().map(|c| c.node).collect::<Vec<_>>(),
        vec![node_of(&h, synth), node_of(&h, fx)]
    );
    assert!((s.volume - Decibels(-6.0).to_linear()).abs() < 1e-6);
    assert_eq!(s.pan, -0.5);

    // Invalid edits.
    let out = h.send(Command::DrumRack(DrumRackCommand::SetChokeGroup {
        id: kick,
        group: Some(17),
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    let other: DrumPadId = h.id();
    let out = h.send(Command::DrumRack(DrumRackCommand::AddPad {
        id: other,
        rack,
        note: 36,
        name: None,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument, "note taken");
    let nested: DeviceId = h.id();
    let out = h.send(Command::DrumRack(DrumRackCommand::InsertDevice {
        id: nested,
        pad: snare,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::DrumRack,
        },
        before: None,
    }));
    assert_eq!(
        err(&out).code,
        ErrorCode::InvalidArgument,
        "no nested racks"
    );

    // Removing the rack removes its pads and their chains.
    h.ok(Command::Device(DeviceCommand::Remove { id: rack }));
    assert!(h.project().drum_pads.is_empty());
    assert!(h.project().devices.is_empty());
    undo(&mut h);
    assert_eq!(h.project().drum_pads.len(), 2);
    assert_eq!(h.project().devices.len(), 4);
}

#[test]
fn pad_notes_swap_and_every_command_is_one_undo_step() {
    let Kit {
        mut h, rack, media, ..
    } = kit();
    let (a, b): (DrumPadId, DrumPadId) = (h.id(), h.id());
    let dev: DeviceId = h.id();
    rack_cmd(
        &mut h,
        DrumRackCommand::AddPad {
            id: a,
            rack,
            note: 36,
            name: Some("A".into()),
        },
    );
    rack_cmd(
        &mut h,
        DrumRackCommand::AddPad {
            id: b,
            rack,
            note: 38,
            name: None,
        },
    );
    // Idempotent create.
    rack_cmd(
        &mut h,
        DrumRackCommand::AddPad {
            id: a,
            rack,
            note: 40,
            name: None,
        },
    );
    assert_eq!(h.project().drum_pads[&a].note, 36);

    rack_cmd(&mut h, DrumRackCommand::SetPadNote { id: a, note: 38 });
    assert_eq!(h.project().drum_pads[&a].note, 38);
    assert_eq!(h.project().drum_pads[&b].note, 36, "swapped");
    undo(&mut h);
    assert_eq!(h.project().drum_pads[&a].note, 36);
    assert_eq!(h.project().drum_pads[&b].note, 38);

    let before = h.project().clone();
    let steps: Vec<DrumRackCommand> = vec![
        DrumRackCommand::RenamePad {
            id: a,
            name: "Kick".into(),
        },
        DrumRackCommand::SetPadColor {
            id: a,
            color: Some(Color(0xff0000)),
        },
        DrumRackCommand::SetChokeGroup {
            id: a,
            group: Some(1),
        },
        DrumRackCommand::SetPadVolume {
            id: a,
            volume: Decibels(-3.0),
        },
        DrumRackCommand::SetPadPan {
            id: a,
            pan: Pan(0.25),
        },
        DrumRackCommand::SetPadMute { id: a, mute: true },
        DrumRackCommand::AddSamplePad {
            pad: h.id(),
            device: dev,
            rack,
            note: 40,
            media,
        },
        DrumRackCommand::RemovePad { id: b },
    ];
    for c in steps {
        let pre = h.project().clone();
        rack_cmd(&mut h, c.clone());
        assert_ne!(h.project(), &pre, "{c:?} changed the document");
        undo(&mut h);
        assert_eq!(h.project(), &pre, "{c:?} undone in one step");
        h.ok(Command::Edit(EditCommand::Redo));
    }
    // Volume/pan are clamped.
    rack_cmd(
        &mut h,
        DrumRackCommand::SetPadVolume {
            id: a,
            volume: Decibels(40.0),
        },
    );
    assert_eq!(h.project().drum_pads[&a].volume, Decibels(6.0));
    for _ in 0..9 {
        undo(&mut h);
    }
    assert_eq!(h.project(), &before);
}

#[test]
fn solo_is_runtime_and_compiles_as_mute_of_the_other_pads() {
    let Kit {
        mut h, track, rack, ..
    } = kit();
    let (a, b, c): (DrumPadId, DrumPadId, DrumPadId) = (h.id(), h.id(), h.id());
    for (id, note) in [(a, 36), (b, 38), (c, 40)] {
        rack_cmd(
            &mut h,
            DrumRackCommand::AddPad {
                id,
                rack,
                note,
                name: None,
            },
        );
    }
    let before = h.project().clone();
    let out = h.send(Command::DrumRack(DrumRackCommand::SetPadSolo {
        id: b,
        solo: true,
    }));
    ok(&out);
    assert!(patches(&out).is_empty(), "no document change");
    assert_eq!(h.project(), &before);
    let t = graph_track(&mut h, track);
    let mutes: Vec<bool> = t.racks[0].pads.iter().map(|p| p.mute).collect();
    assert_eq!(mutes, vec![true, false, true]);
    // Not an undo step: undo reverts the last pad creation instead.
    undo(&mut h);
    assert!(!h.project().drum_pads.contains_key(&c));
    h.ok(Command::Edit(EditCommand::Redo));

    rack_cmd(&mut h, DrumRackCommand::SetPadSolo { id: c, solo: true });
    let t = graph_track(&mut h, track);
    let mutes: Vec<bool> = t.racks[0].pads.iter().map(|p| p.mute).collect();
    assert_eq!(mutes, vec![true, false, false]);
    rack_cmd(&mut h, DrumRackCommand::SetPadSolo { id: b, solo: false });
    rack_cmd(&mut h, DrumRackCommand::SetPadSolo { id: c, solo: false });
    let t = graph_track(&mut h, track);
    assert!(t.racks[0].pads.iter().all(|p| !p.mute));
    let missing: DrumPadId = h.id();
    let out = h.send(Command::DrumRack(DrumRackCommand::SetPadSolo {
        id: missing,
        solo: true,
    }));
    assert_eq!(err(&out).code, ErrorCode::NotFound);
}

#[test]
fn devices_move_between_track_and_pad_chains() {
    let Kit {
        mut h, track, rack, ..
    } = kit();
    let (a, b): (DrumPadId, DrumPadId) = (h.id(), h.id());
    rack_cmd(
        &mut h,
        DrumRackCommand::AddPad {
            id: a,
            rack,
            note: 36,
            name: None,
        },
    );
    rack_cmd(
        &mut h,
        DrumRackCommand::AddPad {
            id: b,
            rack,
            note: 37,
            name: None,
        },
    );
    let fx: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id: fx,
        track,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::Compressor,
        },
        before: None,
    }));
    // Into pad A, then pad B, then back to the track chain before the rack.
    rack_cmd(
        &mut h,
        DrumRackCommand::MoveDevice {
            id: fx,
            pad: Some(a),
            before: None,
        },
    );
    assert_eq!(h.project().devices[&fx].pad, Some(a));
    assert_eq!(h.project().devices_of(track).len(), 1);
    undo(&mut h);
    assert_eq!(h.project().devices[&fx].pad, None, "one undo step");
    rack_cmd(
        &mut h,
        DrumRackCommand::MoveDevice {
            id: fx,
            pad: Some(a),
            before: None,
        },
    );
    rack_cmd(
        &mut h,
        DrumRackCommand::MoveDevice {
            id: fx,
            pad: Some(b),
            before: None,
        },
    );
    assert_eq!(h.project().pad_devices_of(b).len(), 1);
    rack_cmd(
        &mut h,
        DrumRackCommand::MoveDevice {
            id: fx,
            pad: None,
            before: Some(rack),
        },
    );
    let chain: Vec<DeviceId> = h.project().devices_of(track).iter().map(|d| d.id).collect();
    assert_eq!(chain, vec![fx, rack]);
    // Plain Device::Move refuses pad devices; the rack can't go into its own pad.
    rack_cmd(
        &mut h,
        DrumRackCommand::MoveDevice {
            id: fx,
            pad: Some(a),
            before: None,
        },
    );
    let out = h.send(Command::Device(DeviceCommand::Move {
        id: fx,
        track,
        before: None,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    let out = h.send(Command::DrumRack(DrumRackCommand::MoveDevice {
        id: rack,
        pad: Some(a),
        before: None,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
}

#[test]
fn slice_markers_edit_by_sorted_index() {
    let mut k = kit();
    let s = sampler_on_track(&mut k);
    let h = &mut k.h;
    slice(
        h,
        SliceCommand::Add {
            device: s,
            positions: vec![Seconds(0.5), Seconds(0.1), Seconds(0.1004)],
        },
    );
    assert_eq!(
        markers(h, s),
        vec![0.1, 0.5],
        "sorted, 1 ms duplicates dropped"
    );
    slice(
        h,
        SliceCommand::Move {
            device: s,
            index: 0,
            position: Seconds(0.7),
        },
    );
    assert_eq!(markers(h, s), vec![0.5, 0.7]);
    slice(
        h,
        SliceCommand::Remove {
            device: s,
            indices: vec![1],
        },
    );
    assert_eq!(markers(h, s), vec![0.5]);
    slice(
        h,
        SliceCommand::SetEnabled {
            device: s,
            enabled: true,
        },
    );
    slice(
        h,
        SliceCommand::SetBaseNote {
            device: s,
            note: 48,
        },
    );
    let DeviceKind::Builtin {
        device: BuiltinDevice::Sampler { slices, .. },
    } = &h.project().devices[&s].kind
    else {
        panic!()
    };
    assert!(slices.enabled);
    assert_eq!(slices.base_note, 48);
    undo(h);
    undo(h);
    undo(h);
    assert_eq!(markers(h, s), vec![0.5, 0.7], "one undo step each");
    let out = h.send(Command::Slice(SliceCommand::Move {
        device: s,
        index: 5,
        position: Seconds(0.0),
    }));
    assert_eq!(err(&out).code, ErrorCode::NotFound);
    let out = h.send(Command::Slice(SliceCommand::Add {
        device: k.rack,
        positions: vec![],
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument, "not a sampler");
}

#[test]
fn auto_slice_finds_transients_grid_and_equal() {
    let mut k = kit();
    let s = sampler_on_track(&mut k);
    let h = &mut k.h;
    slice(
        h,
        SliceCommand::Auto {
            device: s,
            mode: AutoSlice::Transients { sensitivity: 0.5 },
        },
    );
    let found = markers(h, s);
    assert_eq!(found.len(), HITS.len(), "{found:?}");
    for (f, t) in found.iter().zip(HITS) {
        assert!((f - t).abs() < 0.001, "{f} vs {t}");
    }
    // Grid: 1 beat at 120 BPM = 0.5 s over 1.2 s.
    slice(
        h,
        SliceCommand::Auto {
            device: s,
            mode: AutoSlice::Grid { beats: 1.0 },
        },
    );
    assert_eq!(markers(h, s), vec![0.0, 0.5, 1.0]);
    slice(
        h,
        SliceCommand::Auto {
            device: s,
            mode: AutoSlice::Equal { count: 4 },
        },
    );
    let m = markers(h, s);
    assert_eq!(m.len(), 4);
    assert!((m[1] - 0.3).abs() < 1e-9);
    let out = h.send(Command::Slice(SliceCommand::Auto {
        device: s,
        mode: AutoSlice::Equal { count: 0 },
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
}

#[test]
fn slice_to_drum_rack_uses_client_ids_and_undoes_in_one_step() {
    let mut k = kit();
    let s = sampler_on_track(&mut k);
    let fx: DeviceId = k.h.id();
    let h = &mut k.h;
    h.ok(Command::Device(DeviceCommand::Insert {
        id: fx,
        track: k.track,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::Delay,
        },
        before: None,
    }));
    h.ok(Command::Device(DeviceCommand::SetParam {
        device: s,
        param: sampler::params::ATTACK,
        value: 5.0,
    }));
    slice(
        h,
        SliceCommand::Add {
            device: s,
            positions: HITS.iter().map(|&t| Seconds(t)).collect(),
        },
    );
    slice(
        h,
        SliceCommand::SetEnabled {
            device: s,
            enabled: true,
        },
    );
    let before = h.project().clone();
    let rack: DeviceId = h.id();
    let ids: Vec<SlicePadIds> = (0..4)
        .map(|_| SlicePadIds {
            pad: h.id(),
            device: h.id(),
        })
        .collect();

    // Too few ids.
    let out = h.send(Command::Slice(SliceCommand::ToDrumRack {
        device: s,
        rack,
        pads: ids[..3].to_vec(),
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    assert_eq!(h.project(), &before);

    slice(
        h,
        SliceCommand::ToDrumRack {
            device: s,
            rack,
            pads: ids.clone(),
        },
    );
    let p = h.project().clone();
    assert!(!p.devices.contains_key(&s), "the sampler is replaced");
    let chain: Vec<DeviceId> = p.devices_of(k.track).iter().map(|d| d.id).collect();
    assert_eq!(
        chain,
        vec![k.rack, rack, fx],
        "the rack takes the sampler's place"
    );
    let pads = p.pads_of(rack);
    assert_eq!(pads.len(), 4);
    for (i, (pad, id)) in pads.iter().zip(&ids).enumerate() {
        assert_eq!(pad.id, id.pad);
        assert_eq!(pad.note, 36 + i as u8);
        assert_eq!(pad.name, format!("Slice {}", i + 1));
        let chain = p.pad_devices_of(pad.id);
        assert_eq!(chain.len(), 1);
        let d = chain[0];
        assert_eq!(d.id, id.device);
        let DeviceKind::Builtin {
            device: BuiltinDevice::Sampler { sample, slices },
        } = &d.kind
        else {
            panic!()
        };
        assert_eq!(*sample, Some(k.media));
        assert!(!slices.enabled, "the pad sampler's slice mode is off");
        let start = d.params[&sampler::params::START];
        let end = d.params[&sampler::params::END];
        let len = LOOP_SECS;
        assert!((start - HITS[i] / len * 100.0).abs() < 1e-6);
        let next = HITS.get(i + 1).copied().unwrap_or(len);
        assert!((end - next / len * 100.0).abs() < 1e-6);
        assert_eq!(
            d.params[&sampler::params::ATTACK],
            5.0,
            "keeps the sampler's sound"
        );
        assert_eq!(d.params[&sampler::params::ROOT_KEY], PAD_PLAY_NOTE as f64);
    }
    // A retry is harmless.
    slice(
        h,
        SliceCommand::ToDrumRack {
            device: s,
            rack,
            pads: ids.clone(),
        },
    );
    assert_eq!(h.project(), &p);
    // It compiles into the rack desc.
    let t = graph_track(h, k.track);
    assert_eq!(t.racks[1].pads.len(), 4);
    undo(h);
    assert_eq!(h.project(), &before, "one undo step");
}

#[test]
fn racks_and_slices_survive_save_and_reopen() {
    let mut k = kit();
    let s = sampler_on_track(&mut k);
    let h = &mut k.h;
    let pad: DrumPadId = h.id();
    let dev: DeviceId = h.id();
    rack_cmd(
        h,
        DrumRackCommand::AddSamplePad {
            pad,
            device: dev,
            rack: k.rack,
            note: 36,
            media: k.media,
        },
    );
    rack_cmd(
        h,
        DrumRackCommand::SetChokeGroup {
            id: pad,
            group: Some(4),
        },
    );
    slice(
        h,
        SliceCommand::Auto {
            device: s,
            mode: AutoSlice::Equal { count: 3 },
        },
    );
    let doc = h.project().clone();
    h.ok(Command::Project(ProjectCommand::Save));
    h.create_project("Other");
    h.ok(Command::Project(ProjectCommand::Open { id: doc.id }));
    assert_eq!(h.project(), &doc);
    let t = graph_track(h, k.track);
    assert_eq!(t.racks[0].pads[0].choke_group, Some(4));
}

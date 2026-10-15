//! `groups-buses` (CONTRACTS.md §12.10): `Track::{GroupSelected, Ungroup, SetVca}`, VCA
//! compilation (`RenderGraphDesc::vcas`, VCA solo folded into `TrackDesc::solo`), input taps
//! and live VCA params.

mod common;

use common::*;
use ether_core::protocol::automation::{AutomationCommand, PointSpec};
use ether_core::protocol::devices::{DeviceCommand, DeviceSpec};
use ether_core::protocol::mixer::MixerCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::recording::RecordingCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::{Command, ErrorCode};
use ether_core::{ParamChange, ParamTarget};

fn track(h: &mut Harness, kind: TrackKind) -> TrackId {
    let id: TrackId = h.id();
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

fn child(h: &mut Harness, parent: TrackId) -> TrackId {
    let id: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id,
        kind: TrackKind::Audio,
        name: None,
        color: None,
        parent: Some(parent),
        before: None,
    }));
    id
}

fn group(h: &mut Harness, ids: &[TrackId]) -> TrackId {
    let group: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::GroupSelected {
        ids: ids.to_vec(),
        group,
        name: None,
    }));
    group
}

/// Top-level (or `parent`'s) track ids in order.
fn order(h: &Harness, parent: Option<TrackId>) -> Vec<TrackId> {
    let p = h.project();
    match parent {
        None => p.tracks_ordered().iter().map(|t| t.id).collect(),
        Some(g) => p.child_tracks(g).iter().map(|t| t.id).collect(),
    }
}

fn undo(h: &mut Harness) {
    h.ok(Command::Edit(EditCommand::Undo));
}

#[test]
fn group_selected_wraps_siblings_in_place_as_one_undo_step() {
    let mut h = Harness::with_project();
    let a = track(&mut h, TrackKind::Audio);
    let b = track(&mut h, TrackKind::Midi);
    let c = track(&mut h, TrackKind::Audio);
    let d = track(&mut h, TrackKind::Audio);
    let before = h.project().clone();
    // Selection order does not matter: children keep the track order.
    let g = group(&mut h, &[d, b]);
    let top = order(&h, None);
    let pos = |id| top.iter().position(|t| *t == id).unwrap();
    // The group takes b's place: a, G, c, then returns/master.
    assert!(pos(a) < pos(g) && pos(g) < pos(c), "{top:?}");
    assert_eq!(order(&h, Some(g)), vec![b, d]);
    let gt = &h.project().tracks[&g];
    assert_eq!(gt.kind, TrackKind::Group);
    assert_eq!(gt.name, "Group");
    assert_eq!(gt.output, TrackOutput::Default);
    // Children now route into the group bus.
    h.tick();
    let graph = h.ctl.bridge.last_graph();
    let desc = |id| graph.tracks.iter().find(|t| t.id == id).unwrap();
    assert_eq!(desc(b).output, Some(g));
    assert_eq!(desc(d).group, Some(g));
    undo(&mut h);
    assert_eq!(h.project(), &before);
}

#[test]
fn nested_groups_and_validation() {
    let mut h = Harness::with_project();
    let a = track(&mut h, TrackKind::Audio);
    let b = track(&mut h, TrackKind::Audio);
    let g = group(&mut h, &[a, b]);
    // Nesting: group a child inside the group.
    let inner = group(&mut h, &[a]);
    assert_eq!(h.project().tracks[&inner].parent, Some(g));
    assert_eq!(order(&h, Some(g)), vec![inner, b]);
    // Groups of groups.
    let c = track(&mut h, TrackKind::Audio);
    let outer = group(&mut h, &[g, c]);
    assert_eq!(h.project().tracks[&g].parent, Some(outer));

    // Different parents, master/return/VCA and empty selections are rejected.
    let bad: TrackId = h.id();
    let out = h.send(Command::Track(TrackCommand::GroupSelected {
        ids: vec![a, c],
        group: bad,
        name: None,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    let ret = track(&mut h, TrackKind::Return);
    let vca = track(&mut h, TrackKind::Vca);
    let master = h.project().master_track().id;
    for ids in [vec![ret], vec![vca], vec![master], vec![]] {
        let out = h.send(Command::Track(TrackCommand::GroupSelected {
            ids,
            group: bad,
            name: Some("X".into()),
        }));
        assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    }
    assert!(!h.project().tracks.contains_key(&bad));
}

#[test]
fn ungroup_restores_children_in_place() {
    let mut h = Harness::with_project();
    let a = track(&mut h, TrackKind::Audio);
    let b = track(&mut h, TrackKind::Audio);
    let c = track(&mut h, TrackKind::Audio);
    let d = track(&mut h, TrackKind::Audio);
    let g = group(&mut h, &[b, c]);
    let before = h.project().clone();
    h.ok(Command::Track(TrackCommand::Ungroup {
        group: g,
        force: false,
    }));
    assert!(!h.project().tracks.contains_key(&g));
    let top = order(&h, None);
    let pos = |id| top.iter().position(|t| *t == id).unwrap();
    assert!(
        pos(a) < pos(b) && pos(b) < pos(c) && pos(c) < pos(d),
        "{top:?}"
    );
    assert!(h.project().tracks[&b].parent.is_none());
    undo(&mut h);
    assert_eq!(h.project(), &before);
}

#[test]
fn ungroup_of_a_nested_group_goes_to_its_parent() {
    let mut h = Harness::with_project();
    let outer = track(&mut h, TrackKind::Group);
    let x = child(&mut h, outer);
    let y = child(&mut h, outer);
    let z = child(&mut h, outer);
    let inner = group(&mut h, &[y]);
    h.ok(Command::Track(TrackCommand::Ungroup {
        group: inner,
        force: false,
    }));
    assert_eq!(order(&h, Some(outer)), vec![x, y, z]);
}

#[test]
fn ungroup_with_losses_needs_force() {
    let mut h = Harness::with_project();
    let a = track(&mut h, TrackKind::Audio);
    let g = group(&mut h, &[a]);
    let fx: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id: fx,
        track: g,
        device: DeviceSpec::Builtin {
            device: BuiltinDevice::new(BuiltinDeviceType::Delay),
        },
        before: None,
    }));
    // A track outside routed explicitly to the group.
    let other = track(&mut h, TrackKind::Audio);
    h.ok(Command::Mixer(MixerCommand::SetOutput {
        track: other,
        output: TrackOutput::Track { track: g },
    }));
    let before = h.project().clone();
    let out = h.send(Command::Track(TrackCommand::Ungroup {
        group: g,
        force: false,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidState);
    assert!(
        err(&out).message.contains("device"),
        "{}",
        err(&out).message
    );
    assert_eq!(h.project(), &before);
    h.ok(Command::Track(TrackCommand::Ungroup {
        group: g,
        force: true,
    }));
    assert!(!h.project().devices.contains_key(&fx));
    assert_eq!(h.project().tracks[&other].output, TrackOutput::Default);
    assert!(h.project().tracks[&a].parent.is_none());
    // Not a group.
    let out = h.send(Command::Track(TrackCommand::Ungroup {
        group: a,
        force: true,
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
}

#[test]
fn vcas_compile_with_parents_mute_and_automation() {
    let mut h = Harness::with_project();
    let a = track(&mut h, TrackKind::Audio);
    let v1 = track(&mut h, TrackKind::Vca);
    let v2 = track(&mut h, TrackKind::Vca);
    h.ok(Command::Track(TrackCommand::SetVca {
        id: a,
        vca: Some(v1),
    }));
    h.ok(Command::Track(TrackCommand::SetVca {
        id: v1,
        vca: Some(v2),
    }));
    h.ok(Command::Mixer(MixerCommand::SetVolume {
        track: v2,
        volume: Decibels(-6.0),
    }));
    h.ok(Command::Mixer(MixerCommand::SetMute {
        track: v1,
        mute: true,
    }));
    let lane: AutomationLaneId = h.id();
    h.ok(Command::Automation(AutomationCommand::CreateLane {
        id: lane,
        owner: AutomationOwner::Track { track: v1 },
        target: AutomationTarget::TrackVolume { track: v1 },
    }));
    let pt: AutomationPointId = h.id();
    h.ok(Command::Automation(AutomationCommand::AddPoints {
        lane,
        points: vec![PointSpec {
            id: pt,
            time: Beats(0.0),
            value: 0.5,
            curve: CurveShape::Linear,
        }],
    }));
    h.tick();
    let graph = h.ctl.bridge.last_graph();
    assert!(graph.tracks.iter().all(|t| t.id != v1 && t.id != v2));
    assert_eq!(
        graph.tracks.iter().find(|t| t.id == a).unwrap().vca,
        Some(v1)
    );
    let d1 = graph.vcas.iter().find(|v| v.id == v1).unwrap();
    let d2 = graph.vcas.iter().find(|v| v.id == v2).unwrap();
    assert_eq!(d1.parent, Some(v2));
    assert!(d1.mute && !d2.mute);
    assert!((d2.volume - Decibels(-6.0).to_linear()).abs() < 1e-6);
    assert_eq!(d1.automation.len(), 1);
    assert!(d2.automation.is_empty());

    // Cycles and non-VCA targets are rejected.
    let out = h.send(Command::Track(TrackCommand::SetVca {
        id: v2,
        vca: Some(v1),
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);
    let out = h.send(Command::Track(TrackCommand::SetVca {
        id: v2,
        vca: Some(a),
    }));
    assert_eq!(err(&out).code, ErrorCode::InvalidArgument);

    // Deleting a VCA unassigns its tracks.
    h.ok(Command::Track(TrackCommand::Delete { id: v1 }));
    assert_eq!(h.project().tracks[&a].vca, None);
}

#[test]
fn vca_fader_and_mute_are_live_params() {
    let mut h = Harness::with_project();
    let a = track(&mut h, TrackKind::Audio);
    let v = track(&mut h, TrackKind::Vca);
    h.ok(Command::Track(TrackCommand::SetVca {
        id: a,
        vca: Some(v),
    }));
    h.tick();
    h.ok(Command::Mixer(MixerCommand::SetVolume {
        track: v,
        volume: Decibels(-6.0),
    }));
    h.ok(Command::Mixer(MixerCommand::SetMute {
        track: v,
        mute: true,
    }));
    let params = h.ctl.bridge.param_changes();
    assert!(params.contains(&ParamChange {
        target: ParamTarget::TrackMute { track: v },
        value: 1.0,
    }));
    assert!(params.iter().any(|p| matches!(
        p.target,
        ParamTarget::TrackVolume { track } if track == v
    )));
}

#[test]
fn soloing_a_vca_solos_its_tracks() {
    let mut h = Harness::with_project();
    let a = track(&mut h, TrackKind::Audio);
    let b = track(&mut h, TrackKind::Audio);
    let c = track(&mut h, TrackKind::Audio);
    let v1 = track(&mut h, TrackKind::Vca);
    let v2 = track(&mut h, TrackKind::Vca);
    for (id, vca) in [(a, v1), (b, v2), (v2, v1)] {
        h.ok(Command::Track(TrackCommand::SetVca { id, vca: Some(vca) }));
    }
    h.ok(Command::Mixer(MixerCommand::SetSolo {
        track: v1,
        solo: true,
        exclusive: true,
    }));
    h.tick();
    let graph = h.ctl.bridge.last_graph();
    let solo = |id| graph.tracks.iter().find(|t| t.id == id).unwrap().solo;
    assert!(solo(a) && solo(b), "nested VCAs follow their parent's solo");
    assert!(!solo(c));
    // The document keeps the per-user solo on the VCA only.
    assert!(!h.project().tracks[&a].mixer.solo);
}

#[test]
fn track_input_taps_compile_and_reject_cycles() {
    let mut h = Harness::with_project();
    let a = track(&mut h, TrackKind::Audio);
    let b = track(&mut h, TrackKind::Audio);
    h.ok(Command::Recording(RecordingCommand::SetInput {
        track: b,
        input: TrackInput::Track {
            track: a,
            tap: InputTap::PreFx,
        },
    }));
    h.ok(Command::Recording(RecordingCommand::SetMonitor {
        track: b,
        monitor: MonitorMode::In,
    }));
    h.tick();
    let graph = h.ctl.bridge.last_graph();
    let desc = graph.tracks.iter().find(|t| t.id == b).unwrap();
    assert_eq!(
        desc.input_tap,
        Some(ether_core::InputTapDesc {
            track: a,
            point: InputTap::PreFx,
        })
    );
    assert!(desc.monitor);
    // a ← b would be a cycle.
    let out = h.send(Command::Recording(RecordingCommand::SetInput {
        track: a,
        input: TrackInput::Track {
            track: b,
            tap: InputTap::PostFader,
        },
    }));
    assert!(matches!(
        err(&out).code,
        ErrorCode::InvalidArgument | ErrorCode::InvalidState
    ));
}

#[test]
fn group_selected_is_idempotent_for_a_retry() {
    let mut h = Harness::with_project();
    let a = track(&mut h, TrackKind::Audio);
    let g: TrackId = h.id();
    let cmd = Command::Track(TrackCommand::GroupSelected {
        ids: vec![a],
        group: g,
        name: Some("Drums".into()),
    });
    h.ok(cmd.clone());
    let after = h.project().clone();
    h.ok(cmd);
    assert_eq!(h.project(), &after);
    assert_eq!(h.project().tracks[&g].name, "Drums");
}

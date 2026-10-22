//! Solo overrides the soloed track's own mute (CONTRACTS.md §12.10): a muted track that is
//! soloed compiles (and gets live mute params) unmuted, its document mute is untouched, and
//! it is muted again once unsoloed. Not recursive: group / VCA mutes still apply, and a
//! soloed group doesn't unmute its muted children.

use crate::common;

use common::*;
use ether_core::graph::TrackDesc;
use ether_core::protocol::Command;
use ether_core::protocol::mixer::MixerCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::{ParamChange, ParamTarget};

fn create(h: &mut Harness, kind: TrackKind, parent: Option<TrackId>) -> TrackId {
    let id: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id,
        kind,
        name: None,
        color: None,
        parent,
        before: None,
    }));
    id
}

fn mute(h: &mut Harness, track: TrackId, mute: bool) {
    h.ok(Command::Mixer(MixerCommand::SetMute { track, mute }));
}

fn solo(h: &mut Harness, track: TrackId, solo: bool) {
    h.ok(Command::Mixer(MixerCommand::SetSolo {
        track,
        solo,
        exclusive: false,
    }));
}

fn desc(h: &mut Harness, id: TrackId) -> TrackDesc {
    h.tick();
    h.ctl
        .bridge
        .last_graph()
        .tracks
        .iter()
        .find(|t| t.id == id)
        .cloned()
        .unwrap()
}

#[test]
fn a_soloed_muted_track_is_heard_until_unsoloed() {
    let mut h = Harness::with_project();
    let t = create(&mut h, TrackKind::Audio, None);
    let ret = create(&mut h, TrackKind::Return, None);
    let send: SendId = h.id();
    h.ok(Command::Mixer(MixerCommand::CreateSend {
        id: send,
        from: t,
        to: ret,
        level: Decibels(0.0),
        pre_fader: false,
    }));
    mute(&mut h, t, true);
    assert!(desc(&mut h, t).mute);

    solo(&mut h, t, true);
    let d = desc(&mut h, t);
    assert!(d.solo && !d.mute, "soloed: its own mute is overridden");
    assert_eq!(
        d.sends.len(),
        1,
        "its sends stay (the return is always audible)"
    );
    assert!(
        h.project().tracks[&t].mixer.mute,
        "the document mute is untouched"
    );

    // A live mute flip while soloed keeps it audible (the param says unmuted)...
    mute(&mut h, t, false);
    mute(&mut h, t, true);
    let last = h
        .ctl
        .bridge
        .param_changes()
        .into_iter()
        .rfind(|p| p.target == ParamTarget::TrackMute { track: t });
    assert_eq!(
        last,
        Some(ParamChange {
            target: ParamTarget::TrackMute { track: t },
            value: 0.0,
        })
    );

    // ...and unsoloing mutes it again.
    solo(&mut h, t, false);
    let d = desc(&mut h, t);
    assert!(!d.solo && d.mute, "unsoloed: muted again");
}

#[test]
fn only_the_soloed_track_itself_overrides_its_mute() {
    let mut h = Harness::with_project();
    let g = create(&mut h, TrackKind::Group, None);
    let child = create(&mut h, TrackKind::Audio, Some(g));

    // Soloed group, muted child: the child stays muted (the group itself is heard).
    mute(&mut h, child, true);
    solo(&mut h, g, true);
    assert!(desc(&mut h, child).mute);
    assert!(!desc(&mut h, g).mute);

    // Muted (and soloed: overridden) group: audible.
    mute(&mut h, g, true);
    assert!(!desc(&mut h, g).mute);
    solo(&mut h, g, false);
    mute(&mut h, child, false);

    // Muted group, soloed child: the group's mute still applies (the engine silences
    // everything inside a muted group).
    solo(&mut h, child, true);
    assert!(desc(&mut h, g).mute);
    assert!(!desc(&mut h, child).mute && desc(&mut h, child).solo);
}

#[test]
fn a_soloed_muted_vca_is_heard_but_its_tracks_keep_their_own_mute() {
    let mut h = Harness::with_project();
    let a = create(&mut h, TrackKind::Audio, None);
    let b = create(&mut h, TrackKind::Audio, None);
    let v = create(&mut h, TrackKind::Vca, None);
    for id in [a, b] {
        h.ok(Command::Track(TrackCommand::SetVca { id, vca: Some(v) }));
    }
    mute(&mut h, v, true);
    mute(&mut h, b, true);
    solo(&mut h, v, true);
    h.tick();
    let graph = h.ctl.bridge.last_graph();
    assert!(!graph.vcas.iter().find(|x| x.id == v).unwrap().mute);
    let td = |id| graph.tracks.iter().find(|t| t.id == id).unwrap();
    // Solo folded from the VCA doesn't override a track's own mute.
    assert!(td(a).solo && !td(a).mute);
    assert!(td(b).solo && td(b).mute);

    solo(&mut h, v, false);
    h.tick();
    let graph = h.ctl.bridge.last_graph();
    assert!(graph.vcas.iter().find(|x| x.id == v).unwrap().mute);
}

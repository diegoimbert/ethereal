//! Collaboration (docs/COLLAB.md): several controllers ("sites") through the in-memory hub
//! (the real relay state machine, with controlled delivery).

#[path = "collab_support.rs"]
mod support;

use ether_collab::memory::Hub;
use ether_controller::memory::MemoryLibrary;
use ether_controller::store::ProjectStore;
use ether_core::protocol::clips::{ClipCommand, ClipMove};
use ether_core::protocol::collab::{CollabCommand, CollabStatus, PresenceState};
use ether_core::protocol::devices::{DeviceCategory, DeviceCommand, DeviceSpec};
use ether_core::protocol::media::{BrowseLocation, MediaCommand, MediaSource};
use ether_core::protocol::mixer::MixerCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::notes::{NoteCommand, NoteEdit, NoteSpec};
use ether_core::protocol::project::{EditCommand, ProjectCommand};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::transport::TransportCommand;
use ether_core::protocol::*;
use proptest::prelude::*;
use support::*;

fn add_track(s: &mut Site, kind: TrackKind) -> TrackId {
    let id = s.id();
    s.ok(Command::Track(TrackCommand::Create {
        id,
        kind,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    id
}

fn add_clip(s: &mut Site, track: TrackId, start: f64) -> ClipId {
    let id = s.id();
    s.ok(Command::Clip(ClipCommand::CreateMidi {
        id,
        track,
        start: Beats(start),
        length: Beats(4.0),
        name: None,
    }));
    id
}

fn set_volume(s: &mut Site, track: TrackId, db: f32) -> Vec<ServerMessage> {
    s.send(Command::Mixer(MixerCommand::SetVolume {
        track,
        volume: Decibels(db),
    }))
}

fn volume(s: &Site, track: TrackId) -> f32 {
    s.project().tracks[&track].mixer.volume.0
}

fn undo(s: &mut Site) -> Vec<ServerMessage> {
    s.send(Command::Edit(EditCommand::Undo))
}

#[test]
fn late_joiner_gets_the_project_and_edits_flow_both_ways() {
    let hub = Hub::default();
    let mut a = Site::on_hub(11, &hub);
    let pid = a.create_project("Jam");
    let t = add_track(&mut a, TrackKind::Midi);
    let c = add_clip(&mut a, t, 0.0);
    a.join("ws://hub", "jam", "Ada", None);
    settle(&mut [&mut a], &hub);
    assert_eq!(
        a.status(),
        Some(CollabStatus::Online {
            session: "jam".into(),
            site: a.ctl.collab_site()
        })
    );
    // Log activity before B joins.
    set_volume(&mut a, t, -3.0);
    settle(&mut [&mut a], &hub);

    let mut b = Site::on_hub(22, &hub);
    b.join("ws://hub", "jam", "Bob", None);
    settle(&mut [&mut a, &mut b], &hub);
    assert!(b.online());
    assert_eq!(b.project().id, pid, "same ProjectId");
    assert!(b.project().clips.contains_key(&c));
    assert_eq!(volume(&b, t), -3.0);
    assert!(
        b.ctl.store.contains(pid),
        "the session project is stored on B"
    );
    assert_converged(&[&a, &b]);

    // B edits: A gets a patch marked with B's origin.
    let n: NoteId = b.id();
    b.ok(Command::Note(NoteCommand::Add {
        clip: c,
        notes: vec![NoteSpec {
            id: n,
            pitch: 60,
            velocity: 0.8,
            start: Beats(0.0),
            duration: Beats(1.0),
        }],
    }));
    a.log.clear();
    settle(&mut [&mut a, &mut b], &hub);
    assert!(a.project().notes.contains_key(&n));
    let remote = patches(&a.log)
        .into_iter()
        .find(|p| p.origin.is_some())
        .expect("remote patch");
    assert_eq!(remote.origin.unwrap().site, b.ctl.collab_site());
    assert_converged(&[&a, &b]);

    // Presence: names and distinct colors (`Get` re-emits them).
    a.ok(Command::Collab(CollabCommand::Get));
    b.ok(Command::Collab(CollabCommand::Get));
    let pa = a.peers();
    let pb = b.peers();
    assert_eq!(pa.len(), 1);
    assert_eq!(pa[0].name, "Bob");
    assert_eq!(pb[0].name, "Ada");
    assert_ne!(pa[0].color, pb[0].color);
    b.ok(Command::Collab(CollabCommand::SetPresence {
        presence: PresenceState {
            selected_tracks: vec![t],
            cursor: Some(Beats(2.0)),
            ..PresenceState::default()
        },
    }));
    b.advance(200);
    settle(&mut [&mut a, &mut b], &hub);
    assert_eq!(a.peers()[0].state.selected_tracks, vec![t]);

    // `Get` re-emits the state; leaving empties the peer list on the other side.
    a.log.clear();
    a.ok(Command::Collab(CollabCommand::Get));
    assert!(a.online());
    b.ok(Command::Collab(CollabCommand::Leave));
    assert_eq!(b.status(), Some(CollabStatus::Offline));
    settle(&mut [&mut a], &hub);
    assert!(a.peers().is_empty());
    // B keeps a normal local project.
    set_volume(&mut b, t, -9.0);
    assert_eq!(volume(&b, t), -9.0);
}

#[test]
fn concurrent_edits_converge_with_last_sequenced_writer() {
    let hub = Hub::default();
    let mut sites = session(&hub, 2);
    let t = add_track(&mut sites[0], TrackKind::Midi);
    {
        let [a, b] = sites.as_mut_slice() else {
            unreachable!()
        };
        settle(&mut [a, b], &hub);
    }
    let [a, b] = sites.as_mut_slice() else {
        unreachable!()
    };
    // Concurrent writes to one field: B's reaches the relay first, A's wins (later).
    set_volume(b, t, -6.0);
    set_volume(a, t, -12.0);
    hub.deliver_from(hub.links()[1]);
    hub.deliver_from(hub.links()[0]);
    settle(&mut [a, b], &hub);
    assert_eq!(volume(a, t), -12.0);
    assert_converged(&[a, b]);
}

#[test]
fn delete_wins_over_a_concurrent_child_insert() {
    let hub = Hub::default();
    let mut sites = session(&hub, 2);
    let t = add_track(&mut sites[0], TrackKind::Midi);
    {
        let [a, b] = sites.as_mut_slice() else {
            unreachable!()
        };
        settle(&mut [a, b], &hub);
    }
    let [a, b] = sites.as_mut_slice() else {
        unreachable!()
    };
    // A adds a clip to T while B deletes T.
    let c = add_clip(a, t, 0.0);
    b.ok(Command::Track(TrackCommand::Delete { id: t }));
    // A's clip is sequenced first: B's delete then cascades over it.
    hub.deliver_from(hub.links()[0]);
    hub.deliver_from(hub.links()[1]);
    settle(&mut [a, b], &hub);
    assert!(!a.project().tracks.contains_key(&t));
    assert!(!a.project().clips.contains_key(&c));
    assert_converged(&[a, b]);

    // The other order: the delete first, the insert is dropped.
    let t2 = add_track(a, TrackKind::Midi);
    settle(&mut [a, b], &hub);
    b.ok(Command::Track(TrackCommand::Delete { id: t2 }));
    let c2 = add_clip(a, t2, 0.0);
    hub.deliver_from(hub.links()[1]);
    hub.deliver_from(hub.links()[0]);
    settle(&mut [a, b], &hub);
    assert!(!a.project().clips.contains_key(&c2));
    assert_converged(&[a, b]);
}

#[test]
fn per_site_undo_skips_what_peers_changed() {
    let hub = Hub::default();
    let mut sites = session(&hub, 2);
    let t = add_track(&mut sites[0], TrackKind::Midi);
    let u = add_track(&mut sites[0], TrackKind::Midi);
    {
        let [a, b] = sites.as_mut_slice() else {
            unreachable!()
        };
        settle(&mut [a, b], &hub);
    }
    let [a, b] = sites.as_mut_slice() else {
        unreachable!()
    };
    // A sets T to -6 and U to -6; B then sets T to -1 (after A, in sequence order).
    set_volume(a, t, -6.0);
    set_volume(a, u, -6.0);
    settle(&mut [a, b], &hub);
    set_volume(b, t, -1.0);
    settle(&mut [a, b], &hub);
    // A undoes its U edit: fine. Then its T edit: B changed T since: skipped.
    undo(a);
    assert_eq!(volume(a, u), 0.0);
    undo(a);
    assert_eq!(volume(a, t), -1.0, "the peer's later value wins");
    settle(&mut [a, b], &hub);
    assert_eq!(volume(b, u), 0.0, "the undo reached B");
    assert_eq!(volume(b, t), -1.0);
    // B's undo only reverts B's own edit.
    undo(b);
    assert_eq!(volume(b, t), -6.0);
    settle(&mut [a, b], &hub);
    assert_converged(&[a, b]);
    // Redo works on the recorded result.
    b.send(Command::Edit(EditCommand::Redo));
    settle(&mut [a, b], &hub);
    assert_eq!(volume(a, t), -1.0);
    // Undoing A's track creation while B added a clip to it: delete wins.
    let v = add_track(a, TrackKind::Midi);
    settle(&mut [a, b], &hub);
    let c = add_clip(b, v, 0.0);
    settle(&mut [a, b], &hub);
    undo(a);
    settle(&mut [a, b], &hub);
    assert!(!b.project().tracks.contains_key(&v));
    assert!(!b.project().clips.contains_key(&c));
    assert_converged(&[a, b]);
}

#[test]
fn undo_interleaved_with_concurrent_remote_edits() {
    let hub = Hub::default();
    let mut sites = session(&hub, 2);
    let t = add_track(&mut sites[0], TrackKind::Midi);
    {
        let [a, b] = sites.as_mut_slice() else {
            unreachable!()
        };
        settle(&mut [a, b], &hub);
    }
    let [a, b] = sites.as_mut_slice() else {
        unreachable!()
    };
    let c = add_clip(a, t, 0.0);
    set_volume(a, t, -4.0);
    // B edits concurrently (not delivered yet), A undoes while its edits are pending.
    let d = add_clip(b, t, 8.0);
    set_volume(b, t, -8.0);
    undo(a);
    hub.deliver_from(hub.links()[1]);
    settle(&mut [a, b], &hub);
    assert_converged(&[a, b]);
    // A undid before it saw B's concurrent volume: the undo is an ordinary write,
    // sequenced last, so it wins everywhere (LWW). B's clip is untouched.
    assert_eq!(volume(a, t), 0.0);
    assert!(a.project().clips.contains_key(&c) && a.project().clips.contains_key(&d));
}

#[test]
fn site_local_state_is_not_shared() {
    let hub = Hub::default();
    let mut sites = session(&hub, 2);
    let t = add_track(&mut sites[0], TrackKind::Midi);
    {
        let [a, b] = sites.as_mut_slice() else {
            unreachable!()
        };
        settle(&mut [a, b], &hub);
    }
    let [a, b] = sites.as_mut_slice() else {
        unreachable!()
    };
    a.ok(Command::Mixer(MixerCommand::SetSolo {
        track: t,
        solo: true,
        exclusive: false,
    }));
    a.ok(Command::Transport(TransportCommand::SetMetronome {
        enabled: true,
    }));
    a.ok(Command::Transport(TransportCommand::Play));
    settle(&mut [a, b], &hub);
    assert!(a.project().tracks[&t].mixer.solo);
    assert!(!b.project().tracks[&t].mixer.solo);
    assert!(a.project().settings.metronome);
    assert!(!b.project().settings.metronome);
    // A local-only undo stays local too.
    undo(a);
    settle(&mut [a, b], &hub);
    assert_converged(&[a, b]);
}

fn plugin_state(s: &Site, d: DeviceId) -> Option<Base64Bytes> {
    match &s.project().devices[&d].kind {
        DeviceKind::Plugin { plugin } => plugin.state.clone(),
        DeviceKind::Builtin { .. } => None,
    }
}

#[test]
fn save_replicates_changed_plugin_state_once_and_missing_plugins_stay_local() {
    let hub = Hub::default();
    let mut sites = session(&hub, 2);
    let [a, b] = sites.as_mut_slice() else {
        unreachable!()
    };
    // `a` has the plugin, `b` does not (it is bypassed there: runtime state only).
    a.ctl.bridge.plugins = Some(std::collections::BTreeMap::from([(
        "com.test.Verb".to_string(),
        plugin_descriptor("Verb", DeviceCategory::AudioEffect),
    )]));
    let t = add_track(a, TrackKind::Audio);
    let d: DeviceId = a.id();
    a.ok(Command::Device(DeviceCommand::Insert {
        id: d,
        track: t,
        device: DeviceSpec::Plugin {
            plugin_id: "com.test.Verb".into(),
            sandboxed: None,
            format: None,
        },
        before: None,
    }));
    settle(&mut [a, b], &hub);
    assert_converged(&[a, b]);

    // Opaque state changed in the plugin GUI: the next save replicates it, once.
    a.ctl
        .bridge
        .plugin_states
        .insert(d, Base64Bytes(vec![1, 2, 3]));
    a.ok(Command::Project(ProjectCommand::Save));
    assert_eq!(a.ctl.collab_pending(), 1, "one stamped transaction");
    settle(&mut [a, b], &hub);
    assert_eq!(plugin_state(b, d), Some(Base64Bytes(vec![1, 2, 3])));
    assert_converged(&[a, b]);
    a.ok(Command::Project(ProjectCommand::Save));
    assert_eq!(a.ctl.collab_pending(), 0, "unchanged state: nothing sent");

    // `b` misses the plugin: no live state, its saves send nothing and keep `a`'s state.
    b.ok(Command::Project(ProjectCommand::Save));
    assert_eq!(b.ctl.collab_pending(), 0);

    // `b` gets the plugin, running with its own live state. `a` changes the state again:
    // `b`'s instance is re-created from the replicated state (not its live one), and a
    // peer's state is not `b`'s change, so its next save doesn't send it back.
    b.ctl.bridge.plugins = a.ctl.bridge.plugins.clone();
    b.ctl.bridge.plugin_states.insert(d, Base64Bytes(vec![9]));
    a.ctl.bridge.plugin_states.insert(d, Base64Bytes(vec![4]));
    a.ok(Command::Project(ProjectCommand::Save));
    settle(&mut [a, b], &hub);
    assert!(
        b.ctl.bridge.calls.iter().any(|c| matches!(
            c,
            Call::CreatePlugin(dev, _, Some(st)) if *dev == d && st.0 == vec![4]
        )),
        "b's plugin reloaded from the replicated state"
    );
    // (The fake instance doesn't read its state back: do it for it.)
    b.ctl.bridge.plugin_states.insert(d, Base64Bytes(vec![4]));
    b.ok(Command::Project(ProjectCommand::Save));
    assert_eq!(b.ctl.collab_pending(), 0);
    settle(&mut [a, b], &hub);
    assert_eq!(plugin_state(a, d), Some(Base64Bytes(vec![4])));
    assert_eq!(plugin_state(b, d), Some(Base64Bytes(vec![4])));
    assert_converged(&[a, b]);
    // Captures are not undo steps: undo reverts the plugin insert.
    undo(a);
    settle(&mut [a, b], &hub);
    assert!(!b.project().devices.contains_key(&d));
    assert_converged(&[a, b]);
}

#[test]
fn media_is_transferred_before_the_import() {
    let hub = Hub::default();
    let mut lib = MemoryLibrary::new();
    lib.add_root("lib", "Library");
    let bytes = wav(48_000, &[vec![0.25; 4_800], vec![-0.25; 4_800]]);
    lib.add_file("lib", "kick.wav", bytes.clone());
    let mut a = Site::new(31, hub.connector(), lib);
    let pid = a.create_project("Media");
    a.join("ws://hub", "m", "A", None);
    settle(&mut [&mut a], &hub);
    let mut b = Site::on_hub(32, &hub);
    b.join("ws://hub", "m", "B", None);
    settle(&mut [&mut a, &mut b], &hub);
    let media: MediaId = a.id();
    a.ok(Command::Media(MediaCommand::Import {
        id: media,
        source: MediaSource::Location {
            location: BrowseLocation::Library { id: "lib".into() },
            path: "kick.wav".into(),
        },
    }));
    let t = add_track(&mut a, TrackKind::Audio);
    let c: ClipId = a.id();
    a.ok(Command::Clip(ClipCommand::CreateAudio {
        id: c,
        track: t,
        start: Beats(0.0),
        media,
    }));
    settle(&mut [&mut a, &mut b], &hub);
    for _ in 0..200 {
        b.tick();
    }
    let m = b.project().media[&media].clone();
    assert_eq!(b.ctl.store.file(pid, &m.file), Some(&bytes[..]));
    assert!(!b.ctl.media_pending(), "loaded on B");
    assert!(
        !b.log.iter().any(|e| matches!(
            e,
            ServerMessage::Event(Event::Media {
                event: ether_core::protocol::media::MediaEvent::Missing { .. }
            })
        )),
        "never missing on B"
    );
    assert_converged(&[&a, &b]);
    // A late joiner gets the file too (relay cache).
    let mut c3 = Site::on_hub(33, &hub);
    c3.join("ws://hub", "m", "C", None);
    settle(&mut [&mut a, &mut b, &mut c3], &hub);
    assert_eq!(c3.ctl.store.file(pid, &m.file), Some(&bytes[..]));
}

#[test]
fn reconnect_resends_pending_edits() {
    let hub = Hub::default();
    let mut sites = session(&hub, 2);
    let t = add_track(&mut sites[0], TrackKind::Midi);
    {
        let [a, b] = sites.as_mut_slice() else {
            unreachable!()
        };
        settle(&mut [a, b], &hub);
    }
    let [a, b] = sites.as_mut_slice() else {
        unreachable!()
    };
    let a_link = hub.links()[0];
    // A edits; the first edit reaches the relay, then the link dies before its ack.
    set_volume(a, t, -2.0);
    hub.deliver_from(a_link);
    set_volume(a, t, -5.0);
    let c = add_clip(a, t, 4.0);
    hub.kill(a_link);
    // Offline edits are kept too.
    a.tick();
    assert!(!a.online());
    a.ok(Command::Track(TrackCommand::Rename {
        id: t,
        name: "offline".into(),
    }));
    // B edits meanwhile.
    let d = add_clip(b, t, 8.0);
    for _ in 0..100 {
        a.advance(100);
        settle(&mut [a, b], &hub);
        if a.online() {
            break;
        }
    }
    assert!(a.online(), "reconnected: {:?}", a.status());
    settle(&mut [a, b], &hub);
    for s in [&*a, &*b] {
        assert_eq!(volume(s, t), -5.0);
        assert!(s.project().clips.contains_key(&c));
        assert!(s.project().clips.contains_key(&d));
        assert_eq!(s.project().tracks[&t].name, "offline");
    }
    assert_converged(&[a, b]);
    assert_eq!(a.ctl.collab_pending(), 0);
}

#[test]
fn leave_and_rejoin_on_the_same_controller_converges() {
    let hub = Hub::default();
    let mut sites = session(&hub, 2);
    let [a, b] = sites.as_mut_slice() else {
        unreachable!()
    };
    let pid = b.project().id;
    let tb = add_track(b, TrackKind::Midi);
    settle(&mut [a, b], &hub);
    b.ok(Command::Collab(CollabCommand::Leave));
    let ta = add_track(a, TrackKind::Audio);
    settle(&mut [a], &hub);
    b.join("ws://hub", "jam", "B again", None);
    settle(&mut [a, b], &hub);
    assert!(b.online());
    // B's own transaction from before the re-join is applied from the log, not taken
    // for an echo.
    assert!(b.project().tracks.contains_key(&tb), "B kept its own track");
    assert!(b.project().tracks.contains_key(&ta));
    assert_converged(&[a, b]);
    // B's new edits get fresh seqs: sequenced, not dropped as resends.
    let tc = add_track(b, TrackKind::Midi);
    settle(&mut [a, b], &hub);
    assert_eq!(b.ctl.collab_pending(), 0);
    assert!(a.project().tracks.contains_key(&tc));
    assert_converged(&[a, b]);
    // No offline work: the stored copy was a state of the log, so no "(local copy)".
    let list = b.ctl.store.list().unwrap();
    assert!(list.iter().all(|p| p.id == pid), "{list:?}");
}

#[test]
fn rebase_keeps_derived_media_length_of_a_pending_import() {
    let hub = Hub::default();
    let mut lib = MemoryLibrary::new();
    lib.add_root("lib", "Library");
    lib.add_file(
        "lib",
        "kick.wav",
        wav(48_000, &[vec![0.25; 4_800], vec![-0.25; 4_800]]),
    );
    let mut a = Site::new(41, hub.connector(), lib);
    a.create_project("Media");
    a.join("ws://hub", "m", "A", None);
    settle(&mut [&mut a], &hub);
    let mut b = Site::on_hub(42, &hub);
    b.join("ws://hub", "m", "B", None);
    settle(&mut [&mut a, &mut b], &hub);
    let a_link = hub.links()[0];
    let b_link = hub.links()[1];
    let media: MediaId = a.id();
    a.ok(Command::Media(MediaCommand::Import {
        id: media,
        source: MediaSource::Location {
            location: BrowseLocation::Library { id: "lib".into() },
            path: "kick.wav".into(),
        },
    }));
    // A decodes (local, derived length) while its import is still pending.
    for _ in 0..100 {
        a.tick();
    }
    let frames = a.project().media[&media].frames;
    assert!(frames > 0);
    assert_eq!(a.ctl.collab_pending(), 1);
    // A peer's edit is sequenced first: A rebases its pending import over it.
    add_track(&mut b, TrackKind::Midi);
    while hub.deliver_from(b_link) {}
    a.tick();
    assert_eq!(a.ctl.collab_pending(), 1, "still pending");
    assert_eq!(a.project().media[&media].frames, frames, "length kept");
    while hub.deliver_from(a_link) {}
    settle(&mut [&mut a, &mut b], &hub);
    assert_eq!(a.project().media[&media].frames, frames);
}

#[test]
fn rejoin_keeps_a_differing_local_copy() {
    let hub = Hub::default();
    let mut sites = session(&hub, 2);
    let t = add_track(&mut sites[0], TrackKind::Midi);
    {
        let [a, b] = sites.as_mut_slice() else {
            unreachable!()
        };
        settle(&mut [a, b], &hub);
    }
    let [a, b] = sites.as_mut_slice() else {
        unreachable!()
    };
    let pid = b.project().id;
    b.ok(Command::Collab(CollabCommand::Leave));
    // Offline work on B, saved.
    b.ok(Command::Track(TrackCommand::Rename {
        id: t,
        name: "mine".into(),
    }));
    b.ok(Command::Project(ProjectCommand::Save));
    // A keeps going.
    a.ok(Command::Track(TrackCommand::Rename {
        id: t,
        name: "theirs".into(),
    }));
    settle(&mut [a], &hub);
    b.join("ws://hub", "jam", "B", None);
    settle(&mut [a, b], &hub);
    assert_eq!(b.project().tracks[&t].name, "theirs");
    let list = b.ctl.store.list().unwrap();
    let backup = list.iter().find(|p| p.id != pid).expect("a backup project");
    assert!(backup.name.ends_with("(local copy)"), "{}", backup.name);
    let json = b.ctl.store.load(backup.id).unwrap();
    let copy = file::load(&json).unwrap();
    assert_eq!(copy.tracks[&t].name, "mine");
}

#[test]
fn opening_another_project_leaves_and_bad_joins_are_refused() {
    let hub = Hub::default();
    let mut sites = session(&hub, 2);
    let b = &mut sites[1];
    let out = b.send(Command::Collab(CollabCommand::Join {
        server: "http://x".into(),
        session: "jam".into(),
        token: None,
        name: "B".into(),
    }));
    assert!(matches!(
        out.last(),
        Some(ServerMessage::Reply(Reply {
            result: ReplyResult::Err { .. },
            ..
        }))
    ));
    b.create_project("Other");
    b.tick();
    assert_eq!(b.status(), Some(CollabStatus::Offline));
}

#[test]
fn unsupported_test_removed_collab_replies() {
    // Commands no longer reply Unsupported.
    let hub = Hub::default();
    let mut s = Site::on_hub(5, &hub);
    s.ok(Command::Collab(CollabCommand::Get));
    s.ok(Command::Collab(CollabCommand::Leave));
}

// ─── Convergence property test ──────────────────────────────────────────────────────────

/// One step of the simulation: (site, action, two random words).
type Step = (usize, u8, u64, u64);

fn pick<T: Copy>(v: &[T], r: u64) -> Option<T> {
    (!v.is_empty()).then(|| v[(r % v.len() as u64) as usize])
}

/// A random local command on `s`'s current document (errors are fine: they change nothing).
fn random_edit(s: &mut Site, action: u8, r1: u64, r2: u64) {
    let p = s.project().clone();
    let tracks: Vec<TrackId> = p
        .tracks
        .values()
        .filter(|t| t.kind != TrackKind::Master)
        .map(|t| t.id)
        .collect();
    let midi: Vec<TrackId> = p
        .tracks
        .values()
        .filter(|t| t.kind == TrackKind::Midi)
        .map(|t| t.id)
        .collect();
    let groups: Vec<TrackId> = p
        .tracks
        .values()
        .filter(|t| t.kind == TrackKind::Group)
        .map(|t| t.id)
        .collect();
    let clips: Vec<ClipId> = p.clips.keys().copied().collect();
    let notes: Vec<NoteId> = p.notes.keys().copied().collect();
    let devices: Vec<DeviceId> = p.devices.keys().copied().collect();
    let cmd = match action % 16 {
        0 | 1 => Command::Track(TrackCommand::Create {
            id: s.id(),
            kind: if r1.is_multiple_of(4) {
                TrackKind::Group
            } else {
                TrackKind::Midi
            },
            name: None,
            color: None,
            parent: if r2.is_multiple_of(3) {
                pick(&groups, r1)
            } else {
                None
            },
            before: None,
        }),
        2 => match pick(&tracks, r1) {
            Some(id) => Command::Track(TrackCommand::Delete { id }),
            None => return,
        },
        3 | 4 => match pick(&midi, r1) {
            Some(track) => Command::Clip(ClipCommand::CreateMidi {
                id: s.id(),
                track,
                start: Beats((r2 % 16) as f64),
                length: Beats(2.0),
                name: None,
            }),
            None => return,
        },
        5 => match pick(&clips, r1) {
            Some(id) => Command::Clip(ClipCommand::Delete { ids: vec![id] }),
            None => return,
        },
        6 => match (pick(&clips, r1), pick(&midi, r2)) {
            (Some(id), Some(track)) => Command::Clip(ClipCommand::Move {
                moves: vec![ClipMove {
                    id,
                    track,
                    start: Beats((r2 % 8) as f64),
                }],
            }),
            _ => return,
        },
        7 => match pick(&tracks, r1) {
            Some(track) => Command::Mixer(MixerCommand::SetVolume {
                track,
                volume: Decibels(-((r2 % 24) as f32)),
            }),
            None => return,
        },
        8 => match pick(&clips, r1) {
            Some(clip) => Command::Note(NoteCommand::Add {
                clip,
                notes: vec![NoteSpec {
                    id: s.id(),
                    pitch: (r2 % 128) as u8,
                    velocity: 0.5,
                    start: Beats((r2 % 4) as f64 * 0.5),
                    duration: Beats(0.5),
                }],
            }),
            None => return,
        },
        9 => match pick(&notes, r1) {
            Some(id) => Command::Note(NoteCommand::Edit {
                edits: vec![NoteEdit {
                    id,
                    pitch: Some((r2 % 128) as u8),
                    velocity: None,
                    start: None,
                    duration: None,
                    muted: None,
                }],
            }),
            None => return,
        },
        10 => match (pick(&tracks, r1), pick(&groups, r2)) {
            (Some(id), parent) => Command::Track(TrackCommand::Move {
                id,
                parent: if r2.is_multiple_of(2) { parent } else { None },
                before: None,
            }),
            _ => return,
        },
        11 | 12 => Command::Edit(EditCommand::Undo),
        13 => Command::Edit(EditCommand::Redo),
        14 => match pick(&tracks, r1) {
            Some(track) => Command::Device(DeviceCommand::Insert {
                id: s.id(),
                track,
                device: DeviceSpec::Builtin {
                    device: BuiltinDevice::new(BuiltinDeviceType::Eq),
                },
                before: None,
            }),
            None => return,
        },
        _ => match pick(&devices, r1) {
            Some(id) => Command::Device(DeviceCommand::Remove { id }),
            None => return,
        },
    };
    s.send(cmd);
}

/// Tick (with time passing, so dropped links reconnect) and deliver until every site is
/// online with nothing pending and nothing in flight.
fn settle_with_reconnects(sites: &mut [Site], hub: &Hub) {
    for _ in 0..200 {
        for s in sites.iter_mut() {
            s.advance(1_000);
        }
        let mut refs: Vec<&mut Site> = sites.iter_mut().collect();
        for s in refs.iter_mut() {
            s.tick();
        }
        hub.deliver();
        if sites.iter().all(|s| s.online()) {
            let mut refs: Vec<&mut Site> = sites.iter_mut().collect();
            settle(&mut refs, hub);
            return;
        }
    }
    panic!("sites did not come back online");
}

fn run_simulation(steps: &[Step]) {
    // A small log so the relay compacts (through a peer's snapshot) during the run.
    let hub = Hub::new(ether_collab::relay::RelayConfig {
        compact_after: 6,
        max_log: 60,
        ..Default::default()
    });
    let mut sites = session(&hub, 3);
    for &(site, action, r1, r2) in steps {
        let site = site % sites.len();
        match action % 24 {
            // Deliver one message of one site to the relay.
            16 | 17 | 23 => {
                if let Some(c) = pick(&hub.links(), r1) {
                    hub.deliver_from(c);
                }
            }
            // A site processes what the relay sent it (time passes: reconnects happen).
            18 | 19 => {
                sites[site].advance(300);
                sites[site].tick();
            }
            // A link drops (pending transactions are resent on reconnect).
            20 if r2.is_multiple_of(3) => {
                if let Some(c) = pick(&hub.links(), r1) {
                    hub.kill(c);
                }
            }
            // A site leaves and joins again on the same controller.
            21 if r2.is_multiple_of(4) => {
                sites[site].ok(Command::Collab(CollabCommand::Leave));
                sites[site].join("ws://hub", "jam", "again", None);
            }
            20..=22 => {
                for s in sites.iter_mut() {
                    s.advance(300);
                    s.tick();
                }
            }
            a => random_edit(&mut sites[site], a % 16, r1, r2),
        }
        for s in &sites {
            s.project()
                .validate()
                .expect("every intermediate state validates");
        }
    }
    settle_with_reconnects(&mut sites, &hub);
    // The convergence invariant (COLLAB.md §2): a fresh site that only replays the relay's
    // snapshot + log (resolve fold, no optimistic state) reaches the same document.
    let mut replay = Site::on_hub(0x7777, &hub);
    replay.join("ws://hub", "jam", "Replay", None);
    sites.push(replay);
    let mut refs: Vec<&mut Site> = sites.iter_mut().collect();
    settle(&mut refs, &hub);
    let refs: Vec<&Site> = sites.iter().collect();
    assert_converged(&refs);
    let p = sites[0].project();
    let remote: usize = sites
        .iter()
        .map(|s| {
            patches(&s.log)
                .iter()
                .filter(|p| p.origin.is_some())
                .count()
        })
        .sum();
    eprintln!(
        "sim: {} steps, {} tracks, {} clips, {} notes, {} devices, {remote} remote patches, {} relay msgs, relay log (base, len) {:?}",
        steps.len(),
        p.tracks.len(),
        p.clips.len(),
        p.notes.len(),
        p.devices.len(),
        hub.delivered(),
        hub.with_relay(|r| r.log_len("jam"))
    );
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    #[test]
    fn replicas_converge(steps in proptest::collection::vec(
        (0usize..3, 0u8..24, any::<u64>(), any::<u64>()), 10..80)) {
        run_simulation(&steps);
    }
}

#[test]
fn convergence_long_run() {
    // A longer deterministic run (xorshift) to cover deep interleavings.
    let mut x: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let steps: Vec<Step> = (0..600)
        .map(|_| {
            let a = next();
            ((a % 3) as usize, ((a >> 8) % 24) as u8, next(), next())
        })
        .collect();
    run_simulation(&steps);
}

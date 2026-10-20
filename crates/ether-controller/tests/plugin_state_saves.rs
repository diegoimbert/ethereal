//! Saving plugin states: `StateDirty` is not a document edit (no quick autosave), plugin
//! state changes alone are saved by a slow timer, routine autosaves only re-read the
//! plugins that changed, and explicit saves / project switches read every plugin.

mod common;

use std::collections::BTreeMap;

use common::*;
use ether_core::plugin::PluginNotification;
use ether_core::protocol::devices::{DeviceCategory, DeviceCommand, DeviceSpec};
use ether_core::protocol::mixer::MixerCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::plugins::PluginCommand;
use ether_core::protocol::project::{EditCommand, ProjectCommand, ProjectEvent};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::*;

/// `saving::STATE_SAVE_INTERVAL_MS`.
const STATE_SAVE_INTERVAL_MS: u64 = 30_000;

fn plugin_harness() -> Harness {
    let bridge = FakeBridge {
        plugins: Some(BTreeMap::from([(
            "com.test.Verb".to_string(),
            plugin_descriptor("Verb", DeviceCategory::AudioEffect),
        )])),
        ..Default::default()
    };
    let mut h = Harness::with(bridge, Default::default(), Default::default());
    h.create_project("Plugins");
    h
}

fn track(h: &mut Harness) -> TrackId {
    let id = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id,
        kind: TrackKind::Audio,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    id
}

fn insert_plugin(h: &mut Harness, t: TrackId) -> DeviceId {
    let d: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id: d,
        track: t,
        device: DeviceSpec::Plugin {
            plugin_id: "com.test.Verb".into(),
            sandboxed: None,
            format: None,
            name: None,
        },
        before: None,
    }));
    d
}

/// A project with two plugins whose live states are `[1]` and `[2]`, saved (clean).
fn two_plugins() -> (Harness, TrackId, DeviceId, DeviceId) {
    let mut h = plugin_harness();
    let t = track(&mut h);
    let (a, b) = (insert_plugin(&mut h, t), insert_plugin(&mut h, t));
    h.ctl.bridge.plugin_states.insert(a, Base64Bytes(vec![1]));
    h.ctl.bridge.plugin_states.insert(b, Base64Bytes(vec![2]));
    h.tick();
    h.ok(Command::Project(ProjectCommand::Save { autosave: false }));
    assert!(!h.ctl.is_dirty());
    h.ctl.bridge.plugin_state_reads.clear();
    (h, t, a, b)
}

fn state_dirty(h: &mut Harness, d: DeviceId, state: u8) {
    h.ctl
        .bridge
        .plugin_states
        .insert(d, Base64Bytes(vec![state]));
    h.ctl
        .bridge
        .plugin_notes
        .push((d, PluginNotification::StateDirty));
}

/// The plugin state stored in project `id`'s file.
fn stored_state(h: &Harness, id: ProjectId, d: DeviceId) -> Option<Vec<u8>> {
    let json = std::str::from_utf8(h.ctl.store.file(id, "project.ether").unwrap()).unwrap();
    let project = file::load(json).unwrap();
    match &project.devices[&d].kind {
        DeviceKind::Plugin { plugin } => plugin.state.as_ref().map(|s| s.0.clone()),
        other => panic!("{other:?}"),
    }
}

fn saved(out: &[ServerMessage]) -> bool {
    events(out).iter().any(|e| {
        matches!(
            e,
            Event::Project {
                event: ProjectEvent::Saved { .. }
            }
        )
    })
}

fn dirty_changed(out: &[ServerMessage]) -> bool {
    events(out).iter().any(|e| {
        matches!(
            e,
            Event::Project {
                event: ProjectEvent::DirtyChanged { .. }
            }
        )
    })
}

fn sorted(mut v: Vec<DeviceId>) -> Vec<DeviceId> {
    v.sort();
    v
}

#[test]
fn state_dirty_does_not_autosave_at_once_and_the_slow_timer_saves_it() {
    let (mut h, _, a, b) = two_plugins();
    let id = h.project().id;
    state_dirty(&mut h, a, 5);
    let out = h.tick();
    // Not a document edit: no `DirtyChanged`, so the UI's 1 s autosave doesn't fire...
    assert!(!dirty_changed(&out) && !saved(&out));
    // ...but it is unsaved work (quit and project switches save it).
    assert!(h.ctl.is_dirty());
    assert!(
        h.ctl.bridge.plugin_state_reads.is_empty(),
        "nothing read yet"
    );

    h.advance(STATE_SAVE_INTERVAL_MS - 1);
    assert!(!saved(&h.tick()));
    assert_eq!(stored_state(&h, id, a), Some(vec![1]));

    h.advance(1);
    let out = h.tick();
    assert!(saved(&out), "the slow timer saved the plugin state");
    assert_eq!(stored_state(&h, id, a), Some(vec![5]));
    assert_eq!(stored_state(&h, id, b), Some(vec![2]));
    assert_eq!(
        h.ctl.bridge.plugin_state_reads,
        vec![a],
        "only the changed plugin read"
    );
    assert!(!h.ctl.is_dirty());
    // Once.
    h.advance(STATE_SAVE_INTERVAL_MS * 2);
    assert!(!saved(&h.tick()));
}

#[test]
fn the_timer_waits_while_a_plugin_keeps_reporting_changes() {
    let (mut h, _, a, _) = two_plugins();
    state_dirty(&mut h, a, 5);
    h.tick();
    h.advance(STATE_SAVE_INTERVAL_MS);
    state_dirty(&mut h, a, 6);
    assert!(!saved(&h.tick()), "a change just now: wait for a pause");
    h.advance(2_000);
    assert!(saved(&h.tick()));
    assert_eq!(stored_state(&h, h.project().id, a), Some(vec![6]));
}

#[test]
fn a_reported_change_to_the_same_state_writes_nothing() {
    let (mut h, _, a, _) = two_plugins();
    state_dirty(&mut h, a, 1);
    h.tick();
    assert!(h.ctl.is_dirty());
    h.advance(STATE_SAVE_INTERVAL_MS);
    let out = h.tick();
    assert!(!saved(&out), "same bytes as saved: no write");
    assert_eq!(h.ctl.bridge.plugin_state_reads, vec![a]);
    assert!(!h.ctl.is_dirty());
}

#[test]
fn routine_autosaves_read_only_changed_plugins_and_explicit_saves_read_all() {
    let (mut h, t, a, b) = two_plugins();
    let id = h.project().id;
    // `a` reports its change; `b` changes silently (VST2 plugins may not report).
    state_dirty(&mut h, a, 5);
    h.ctl.bridge.plugin_states.insert(b, Base64Bytes(vec![9]));
    h.tick();
    // A document edit, then the UI's autosave.
    h.ok(Command::Mixer(MixerCommand::SetVolume {
        track: t,
        volume: Decibels(-6.0),
    }));
    let out = h.send(Command::Project(ProjectCommand::Save { autosave: true }));
    ok(&out);
    assert!(saved(&out));
    assert_eq!(h.ctl.bridge.plugin_state_reads, vec![a]);
    assert_eq!(stored_state(&h, id, a), Some(vec![5]));
    assert_eq!(stored_state(&h, id, b), Some(vec![2]), "cached");
    assert!(
        !h.ctl.is_dirty(),
        "the autosave also saved the plugin change"
    );
    // No re-read of the whole project list on routine saves.
    assert!(!events(&out).iter().any(|e| matches!(
        e,
        Event::Project {
            event: ProjectEvent::ListChanged { .. }
        }
    )));

    // Ctrl+S: every plugin's live state.
    h.ctl.bridge.plugin_state_reads.clear();
    h.ok(Command::Project(ProjectCommand::Save { autosave: false }));
    assert_eq!(
        sorted(h.ctl.bridge.plugin_state_reads.clone()),
        sorted(vec![a, b])
    );
    assert_eq!(stored_state(&h, id, b), Some(vec![9]));
}

#[test]
fn recreated_or_edited_plugins_are_read_again_by_routine_saves() {
    let (mut h, _, a, b) = two_plugins();
    // Re-created instance (its state may differ from the cache).
    h.ok(Command::Plugin(PluginCommand::Reload { device: b }));
    h.tick();
    h.ctl.bridge.plugin_state_reads.clear();
    h.ok(Command::Project(ProjectCommand::Save { autosave: true }));
    assert_eq!(h.ctl.bridge.plugin_state_reads, vec![b]);
    // A document edit of the device (a param from the device panel).
    h.ok(Command::Device(DeviceCommand::SetParam {
        device: a,
        param: ParamId(7),
        value: 0.25,
    }));
    h.ctl.bridge.plugin_state_reads.clear();
    h.ok(Command::Project(ProjectCommand::Save { autosave: true }));
    assert_eq!(h.ctl.bridge.plugin_state_reads, vec![a]);
}

#[test]
fn switching_projects_saves_every_live_plugin_state() {
    let (mut h, _, a, b) = two_plugins();
    let first = h.project().id;
    state_dirty(&mut h, a, 7);
    h.ctl.bridge.plugin_states.insert(b, Base64Bytes(vec![8]));
    h.tick();
    // Only plugin state changed (the document is clean): still saved before switching.
    let other = h.project_id();
    let out = h.send(Command::Project(ProjectCommand::Create {
        id: other,
        name: "Other".into(),
    }));
    ok(&out);
    assert!(events(&out).iter().any(|e| matches!(
        e,
        Event::Project { event: ProjectEvent::Saved { project } } if project.id == first
    )));
    assert_eq!(
        sorted(h.ctl.bridge.plugin_state_reads.clone()),
        sorted(vec![a, b])
    );
    assert_eq!(stored_state(&h, first, a), Some(vec![7]));
    assert_eq!(stored_state(&h, first, b), Some(vec![8]));
}

#[test]
fn saving_on_quit_includes_unsaved_plugin_state() {
    let (mut h, _, a, _) = two_plugins();
    state_dirty(&mut h, a, 4);
    h.tick();
    assert!(h.ctl.is_dirty());
    let mut out = Vec::new();
    h.ctl.save_now(&mut out).unwrap();
    assert_eq!(stored_state(&h, h.project().id, a), Some(vec![4]));
    assert!(!h.ctl.is_dirty());
}

fn doc_state(h: &Harness, d: DeviceId) -> Option<Vec<u8>> {
    match &h.project().devices[&d].kind {
        DeviceKind::Plugin { plugin } => plugin.state.as_ref().map(|s| s.0.clone()),
        other => panic!("{other:?}"),
    }
}

/// The state the last instance of `d` was created with.
fn last_create(h: &Harness, d: DeviceId) -> Option<Option<Vec<u8>>> {
    h.ctl.bridge.calls.iter().rev().find_map(|c| match c {
        Call::CreatePlugin(id, _, state) if *id == d => Some(state.as_ref().map(|s| s.0.clone())),
        _ => None,
    })
}

/// Undoing a plugin delete (on its own, or with its track) brings the device back with
/// what was running, not the last saved state; redo/undo again keeps it.
#[test]
fn undoing_a_plugin_delete_restores_the_running_state() {
    let mut h = plugin_harness();
    let t = track(&mut h);
    let d = insert_plugin(&mut h, t);
    h.tick();
    // Tweaked in its GUI, never saved: the document has no state.
    h.ctl.bridge.plugin_states.insert(d, Base64Bytes(vec![9]));
    assert_eq!(doc_state(&h, d), None);

    h.ok(Command::Device(DeviceCommand::Remove { id: d }));
    h.tick();
    assert!(!h.project().devices.contains_key(&d));
    h.ok(Command::Edit(EditCommand::Undo));
    h.tick();
    assert_eq!(doc_state(&h, d), Some(vec![9]));
    assert_eq!(
        last_create(&h, d),
        Some(Some(vec![9])),
        "re-created running it"
    );
    h.ok(Command::Edit(EditCommand::Redo));
    h.ok(Command::Edit(EditCommand::Undo));
    h.tick();
    assert_eq!(doc_state(&h, d), Some(vec![9]));

    // The same through a track delete.
    h.ctl.bridge.plugin_states.insert(d, Base64Bytes(vec![10]));
    h.ok(Command::Track(TrackCommand::Delete { id: t }));
    h.tick();
    h.ok(Command::Edit(EditCommand::Undo));
    h.tick();
    assert_eq!(doc_state(&h, d), Some(vec![10]));
    assert_eq!(last_create(&h, d), Some(Some(vec![10])));
}

/// `EtherController::load_plugin_state` (an extension's new state for a plugin): loaded in
/// place when the host can, else the instance is re-created from the document.
#[test]
fn a_new_state_loads_into_the_running_instance_when_it_can() {
    let mut h = plugin_harness();
    let t = track(&mut h);
    let d = insert_plugin(&mut h, t);
    h.tick();
    let state = Base64Bytes(vec![7, 7]);
    assert_eq!(h.ctl.load_plugin_state(d, &state), Ok(true));
    assert!(
        h.ctl
            .bridge
            .calls
            .iter()
            .any(|c| matches!(c, Call::SetPluginState(x, s) if *x == d && *s == state))
    );
    // A host that can't: re-created, the error says why.
    h.ctl.bridge.live_state_fails = true;
    assert!(h.ctl.load_plugin_state(d, &Base64Bytes(vec![8])).is_err());
    // No running instance: nothing to load into.
    let none: DeviceId = h.id();
    assert_eq!(h.ctl.load_plugin_state(none, &state), Ok(false));
}

//! base-131: launch safety. Safe open (`Project::OpenSafe`: plugin devices held as bypassed
//! placeholders until `Project::LoadPlugins`, the saved document untouched) and the
//! previous session's status (`Version::SessionStatus`) that the UI's "Reopen last project
//! on launch" and crash dialog read.

use crate::common;

use std::collections::BTreeMap;

use common::*;
use ether_controller::ControllerConfig;
use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::store::ProjectStore;
use ether_core::protocol::devices::{DeviceCategory, DeviceCommand, DeviceSpec};
use ether_core::protocol::model::*;
use ether_core::protocol::project::{ProjectCommand, ProjectEvent};
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::versions::VersionCommand;
use ether_core::protocol::*;

const PLUGIN: &str = "com.test.Crashy";
const MARKER: &str = "versions/.session";
const PROJECT_FILE: &str = "project.ether";

fn plugin_bridge() -> FakeBridge {
    FakeBridge {
        plugins: Some(BTreeMap::from([(
            PLUGIN.to_string(),
            plugin_descriptor("Crashy", DeviceCategory::AudioEffect),
        )])),
        ..Default::default()
    }
}

fn harness() -> Harness {
    Harness::with(
        plugin_bridge(),
        MemoryLibrary::new(),
        ControllerConfig {
            autosave_after_ms: None,
            ..ControllerConfig::default()
        },
    )
}

/// A new controller over `store`, as after a restart (a later start: another session id).
fn restart(store: MemoryStore) -> Harness {
    let mut h = harness();
    let now = store.now_ms + 1_000;
    h.ctl.store = store;
    h.ctl.host.now = now;
    h.ctl.store.now_ms = now;
    h
}

/// A clean quit: the open project's marker is removed, as `Drop` does (versions.rs
/// `a_clean_close_removes_the_marker`), keeping the store.
fn quit(mut h: Harness) -> MemoryStore {
    let ids: Vec<ProjectId> = h
        .ctl
        .store
        .list()
        .unwrap()
        .into_iter()
        .map(|s| s.id)
        .collect();
    for id in ids {
        let _ = h.ctl.store.remove(id, MARKER);
    }
    crash(h)
}

/// A crash: the controller never drops.
fn crash(h: Harness) -> MemoryStore {
    let store = h.ctl.store.clone();
    std::mem::forget(h);
    store
}

/// A saved project with one audio track holding the built-in Gain and the plugin.
fn project_with_plugin(h: &mut Harness) -> (ProjectId, DeviceId) {
    let id = h.create_project("Song");
    let track: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id: track,
        kind: TrackKind::Audio,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    let device: DeviceId = h.id();
    h.ok(Command::Device(DeviceCommand::Insert {
        id: device,
        track,
        device: DeviceSpec::Plugin {
            plugin_id: PLUGIN.into(),
            sandboxed: Some(false),
            format: Some(PluginFormat::Clap),
            name: None,
        },
        before: None,
    }));
    h.ok(Command::Project(ProjectCommand::Save { autosave: false }));
    (id, device)
}

fn plugin_creations(h: &Harness, device: DeviceId) -> usize {
    h.ctl
        .bridge
        .calls
        .iter()
        .filter(|c| matches!(c, Call::CreatePlugin(d, ..) if *d == device))
        .count()
}

fn safe_mode_events(out: &[ServerMessage]) -> Vec<(bool, Vec<DeviceId>)> {
    events(out)
        .into_iter()
        .filter_map(|e| match e {
            Event::Project {
                event: ProjectEvent::SafeMode { active, devices },
            } => Some((active, devices)),
            _ => None,
        })
        .collect()
}

fn unclean(h: &mut Harness) -> Vec<ProjectId> {
    match h.ok(Command::Version(VersionCommand::SessionStatus)) {
        ReplyValue::SessionStatus { unclean } => unclean,
        r => panic!("{r:?}"),
    }
}

#[test]
fn safe_open_holds_plugins_as_placeholders_until_loaded() {
    let mut h = harness();
    let (id, device) = project_with_plugin(&mut h);
    let saved = h.ctl.store.file(id, PROJECT_FILE).unwrap().to_vec();
    let mut h = restart(quit(h));

    let out = h.send(Command::Project(ProjectCommand::OpenSafe { id }));
    ok(&out);
    assert_eq!(safe_mode_events(&out), vec![(true, vec![device])]);
    h.tick();
    assert_eq!(plugin_creations(&h, device), 0, "no plugin is instantiated");
    // Bypassed: the track's chain skips it (the built-in stays).
    let g = h.ctl.bridge.last_graph();
    assert!(g.tracks.iter().all(|t| t.chain.is_empty()), "{g:?}");
    // The device, its state and params are in the document as saved.
    let dev = &h.project().devices[&device];
    assert!(matches!(&dev.kind, DeviceKind::Plugin { plugin } if plugin.plugin_id == PLUGIN));

    // Saving (explicitly or on switch) leaves the stored document as it was.
    h.ok(Command::Project(ProjectCommand::Save { autosave: false }));
    assert_eq!(
        file::load(std::str::from_utf8(h.ctl.store.file(id, PROJECT_FILE).unwrap()).unwrap())
            .unwrap()
            .devices,
        file::load(std::str::from_utf8(&saved).unwrap())
            .unwrap()
            .devices
    );

    // Load plugins: instantiated once, safe mode ends.
    let out = h.send(Command::Project(ProjectCommand::LoadPlugins));
    ok(&out);
    assert_eq!(
        safe_mode_events(&out),
        vec![(false, Vec::<DeviceId>::new())]
    );
    h.tick();
    assert_eq!(plugin_creations(&h, device), 1);
    assert_eq!(
        h.ctl
            .bridge
            .last_graph()
            .tracks
            .iter()
            .map(|t| t.chain.len())
            .sum::<usize>(),
        1
    );
    // Again: nothing to do, no event.
    let out = h.send(Command::Project(ProjectCommand::LoadPlugins));
    assert!(safe_mode_events(&out).is_empty());
}

/// #588: save as keeps safe mode (the plugins stay placeholders), and says so after its
/// `ProjectLoaded` (clients leave safe mode on it).
#[test]
fn save_as_in_safe_mode_says_safe_mode_again() {
    let mut h = harness();
    let (id, device) = project_with_plugin(&mut h);
    let mut h = restart(quit(h));
    h.ok(Command::Project(ProjectCommand::OpenSafe { id }));
    // (The restarted harness's ids start over: skip the open project's.)
    let new_id = std::iter::repeat_with(|| h.project_id())
        .find(|p| *p != id)
        .unwrap();
    let out = h.send(Command::Project(ProjectCommand::SaveAs {
        new_id,
        name: "Copy".into(),
    }));
    ok(&out);
    let evs = events(&out);
    let loaded = evs
        .iter()
        .position(|e| matches!(e, Event::ProjectLoaded { .. }))
        .expect("ProjectLoaded");
    let safe = evs
        .iter()
        .position(|e| {
            matches!(
                e,
                Event::Project {
                    event: ProjectEvent::SafeMode { active: true, .. }
                }
            )
        })
        .expect("SafeMode after save as");
    assert!(safe > loaded, "SafeMode must follow ProjectLoaded");
    assert_eq!(safe_mode_events(&out), vec![(true, vec![device])]);
    h.tick();
    assert_eq!(plugin_creations(&h, device), 0, "still placeholders");

    // Outside safe mode, save as says nothing about it.
    let other = std::iter::repeat_with(|| h.project_id())
        .find(|p| *p != id && *p != new_id)
        .unwrap();
    h.ok(Command::Project(ProjectCommand::LoadPlugins));
    let out = h.send(Command::Project(ProjectCommand::SaveAs {
        new_id: other,
        name: "Copy 2".into(),
    }));
    assert!(safe_mode_events(&out).is_empty());
}

#[test]
fn reloading_one_placeholder_loads_only_that_plugin() {
    let mut h = harness();
    let (id, device) = project_with_plugin(&mut h);
    let mut h = restart(quit(h));
    h.ok(Command::Project(ProjectCommand::OpenSafe { id }));
    h.tick();
    assert_eq!(plugin_creations(&h, device), 0);
    h.ok(Command::Plugin(
        ether_core::protocol::plugins::PluginCommand::Reload { device },
    ));
    h.tick();
    assert_eq!(plugin_creations(&h, device), 1);
}

#[test]
fn a_normal_open_loads_plugins_and_leaves_safe_mode() {
    let mut h = harness();
    let (id, device) = project_with_plugin(&mut h);
    let mut h = restart(quit(h));
    h.ok(Command::Project(ProjectCommand::OpenSafe { id }));
    let out = h.send(Command::Project(ProjectCommand::Open { id }));
    ok(&out);
    assert!(safe_mode_events(&out).is_empty());
    h.tick();
    assert_eq!(plugin_creations(&h, device), 1);
}

#[test]
fn safe_mode_without_plugins_is_still_reported() {
    let mut h = harness();
    let id = h.create_project("No plugins");
    let mut h = restart(quit(h));
    let out = h.send(Command::Project(ProjectCommand::OpenSafe { id }));
    assert_eq!(safe_mode_events(&out), vec![(true, Vec::new())]);
    let out = h.send(Command::Project(ProjectCommand::LoadPlugins));
    assert_eq!(safe_mode_events(&out), vec![(false, Vec::new())]);
}

#[test]
fn load_plugins_needs_an_open_project() {
    let mut h = harness();
    let e = err(&h.send(Command::Project(ProjectCommand::LoadPlugins)));
    assert_eq!(e.code, ErrorCode::InvalidState);
}

#[test]
fn session_status_tells_a_clean_close_from_a_crash() {
    // Nothing stored: clean.
    let mut h = harness();
    assert!(unclean(&mut h).is_empty());
    let (id, _) = project_with_plugin(&mut h);

    // A clean quit: clean.
    let mut h = restart(quit(h));
    assert!(unclean(&mut h).is_empty());

    // Opened, then a crash (even with nothing unsaved): unclean, also once reopened.
    h.ok(Command::Project(ProjectCommand::Open { id }));
    let mut h = restart(crash(h));
    assert_eq!(unclean(&mut h), vec![id]);
    h.ok(Command::Project(ProjectCommand::OpenSafe { id }));
    assert_eq!(unclean(&mut h), vec![id]);

    // Dismissed in the crash dialog, then this session quits cleanly: clean next time.
    h.ok(Command::Version(VersionCommand::DiscardRecovery {
        project: id,
    }));
    let mut h = restart(quit(h));
    assert!(unclean(&mut h).is_empty());

    // Dismissing a crashed project that is not open removes its stale marker.
    h.ok(Command::Project(ProjectCommand::Open { id }));
    let mut h = restart(crash(h));
    assert_eq!(unclean(&mut h), vec![id]);
    h.ok(Command::Version(VersionCommand::DiscardRecovery {
        project: id,
    }));
    assert!(unclean(&mut h).is_empty());
}

#[test]
fn a_plugin_crashing_while_the_project_opens_counts_as_a_crash() {
    let mut h = harness();
    let (id, _) = project_with_plugin(&mut h);
    let mut h = restart(quit(h));
    // The plugin takes the process down while it is instantiated.
    h.ctl.bridge.panic_on_plugin = true;
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.send(Command::Project(ProjectCommand::Open { id }))
    }));
    assert!(r.is_err());
    // The session marker was written before the plugins: the next launch sees a crash.
    assert!(h.ctl.store.file(id, MARKER).is_some());
    let mut h = restart(crash(h));
    assert_eq!(unclean(&mut h), vec![id]);
}

/// A project opened while its plugin is missing keeps the device (bypassed, no instance)
/// and its saved state; once a rescan lists the plugin, `Reload` (the UI's "found ·
/// reload") instantiates it with that state.
#[test]
fn a_missing_plugin_found_by_a_rescan_reloads_with_its_saved_state() {
    let mut h = harness();
    let (id, device) = project_with_plugin(&mut h);
    let state = Base64Bytes(vec![4, 2]);
    h.ctl.bridge.plugin_states.insert(device, state.clone());
    h.ok(Command::Project(ProjectCommand::Save { autosave: false }));

    // Uninstalled: the project opens, the device has no instance.
    let mut h = restart(quit(h));
    let found = h
        .ctl
        .bridge
        .plugins
        .as_mut()
        .unwrap()
        .remove(PLUGIN)
        .unwrap();
    h.ok(Command::Project(ProjectCommand::Open { id }));
    h.tick();
    assert_eq!(plugin_creations(&h, device), 0);
    assert!(h.ctl.bridge.live.values().all(|d| *d != device));
    // Saving meanwhile keeps the state it had.
    h.ok(Command::Project(ProjectCommand::Save { autosave: false }));
    assert!(
        matches!(&h.project().devices[&device].kind, DeviceKind::Plugin { plugin } if plugin.state.as_ref() == Some(&state))
    );

    // A rescan finds it; reloading creates the instance from the saved state.
    h.ctl
        .bridge
        .plugins
        .as_mut()
        .unwrap()
        .insert(PLUGIN.into(), found);
    h.ok(Command::Plugin(
        ether_core::protocol::plugins::PluginCommand::Reload { device },
    ));
    h.tick();
    let created: Vec<_> = h
        .ctl
        .bridge
        .calls
        .iter()
        .filter_map(|c| match c {
            Call::CreatePlugin(d, _, s) if *d == device => Some(s.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(created, vec![Some(state)]);
    assert!(h.ctl.bridge.live.values().any(|d| *d == device));
}

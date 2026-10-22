//! A project load suspends the engine until every plugin and media is in (`loading`):
//! the transport is refused meanwhile, whoever asks, the progress is in the transport
//! state, and the project stays explorable and editable.

use crate::common;

use std::collections::BTreeMap;

use common::*;
use ether_controller::memory::MemoryLibrary;
use ether_core::TransportControl;
use ether_core::protocol::agent::AgentCommand;
use ether_core::protocol::devices::{DeviceCategory, DeviceCommand, DeviceSpec};
use ether_core::protocol::media::{BrowseLocation, MediaCommand, MediaSource};
use ether_core::protocol::model::*;
use ether_core::protocol::project::ProjectCommand;
use ether_core::protocol::recording::RecordingCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::transport::{LoadProgress, TransportCommand};
use ether_core::protocol::*;

const PLUGINS: usize = 5;

fn harness() -> Harness {
    let bridge = FakeBridge {
        plugins: Some(BTreeMap::from([(
            "com.test.Verb".to_string(),
            plugin_descriptor("Verb", DeviceCategory::AudioEffect),
        )])),
        ..Default::default()
    };
    let mut lib = MemoryLibrary::new();
    lib.add_root("lib", "Library");
    let tone = sine(44_100, 440.0, 441_000, 0.5);
    lib.add_file("lib", "tone.wav", wav(44_100, &[tone]));
    let mut h = Harness::with(bridge, lib, Default::default());
    h.create_project("Big");
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

/// A saved project with [`PLUGINS`] plugins (each loading for ~60 ms: 2 per tick), and
/// optionally a 10 s media file. Returns its id.
fn big_project(h: &mut Harness, media: bool) -> ProjectId {
    let t = track(h);
    for _ in 0..PLUGINS {
        insert_plugin(h, t);
    }
    if media {
        let id: MediaId = h.id();
        h.ok(Command::Media(MediaCommand::Import {
            id,
            source: MediaSource::Location {
                location: BrowseLocation::Library { id: "lib".into() },
                path: "tone.wav".into(),
            },
        }));
        h.drain_media();
    }
    h.ok(Command::Project(ProjectCommand::Save { autosave: false }));
    h.ctl.host.step_per_read = 60;
    // The calls of what comes next only.
    h.ctl.bridge.calls.clear();
    h.project().id
}

fn suspends(h: &Harness) -> Vec<bool> {
    h.ctl
        .bridge
        .calls
        .iter()
        .filter_map(|c| match c {
            Call::Suspend(s) => Some(*s),
            _ => None,
        })
        .collect()
}

fn loading(h: &Harness) -> Option<LoadProgress> {
    h.ctl.transport_state().and_then(|s| s.loading)
}

fn refused(h: &mut Harness, command: Command) {
    let out = h.send(command.clone());
    let e = err(&out);
    assert_eq!(e.code, ErrorCode::InvalidState, "{command:?}");
    assert_eq!(
        e.key.as_ref().map(|k| k.id.as_str()),
        Some("error.project.loading")
    );
    assert!(e.message.contains("still loading"), "{}", e.message);
}

fn play() -> Command {
    Command::Transport(TransportCommand::Play)
}

#[test]
fn the_engine_is_suspended_until_every_plugin_is_in() {
    let mut h = harness();
    let id = big_project(&mut h, false);
    let before = h.ctl.bridge.calls.len();
    let out = h.send(Command::Project(ProjectCommand::Open { id }));
    ok(&out);
    assert_eq!(suspends(&h), vec![true], "suspended for the load");
    // The load starts silent: the suspension comes before any of the new project's nodes.
    let first_create = h.ctl.bridge.calls[before..]
        .iter()
        .position(|c| matches!(c, Call::CreatePlugin(..)))
        .unwrap();
    let suspend = h.ctl.bridge.calls[before..]
        .iter()
        .position(|c| matches!(c, Call::Suspend(true)))
        .unwrap();
    assert!(suspend < first_create);
    // The progress is in the transport state (and so in its event).
    let progress = LoadProgress {
        plugins_loaded: 2,
        plugins_total: PLUGINS as u32,
        media_loaded: 0,
        media_total: 0,
    };
    assert_eq!(loading(&h), Some(progress));
    assert!(events(&out).iter().any(|e| matches!(e,
        Event::Transport { state } if state.loading == Some(progress))));

    h.tick();
    assert_eq!(loading(&h).map(|l| l.plugins_loaded), Some(4));
    assert_eq!(suspends(&h), vec![true]);

    // The last plugin: the final graph is published, then the engine resumes.
    let at = h.ctl.bridge.calls.len();
    let out = h.tick();
    assert_eq!(loading(&h), None);
    assert_eq!(suspends(&h), vec![true, false]);
    let calls = &h.ctl.bridge.calls[at..];
    let publish = calls
        .iter()
        .rposition(|c| matches!(c, Call::Publish(_)))
        .unwrap();
    let resume = calls
        .iter()
        .position(|c| matches!(c, Call::Suspend(false)))
        .unwrap();
    assert!(publish < resume, "resumed after the final graph: {calls:?}");
    assert!(events(&out).iter().any(|e| matches!(e,
        Event::Transport { state } if state.loading.is_none())));

    // Ready: play works.
    h.ok(play());
    assert!(matches!(
        h.ctl.bridge.calls.last(),
        Some(Call::Transport(TransportControl::Play))
    ));
}

#[test]
fn transport_commands_are_refused_while_loading_and_the_project_stays_editable() {
    let mut h = harness();
    let id = big_project(&mut h, false);
    h.ok(Command::Project(ProjectCommand::Open { id }));
    assert!(loading(&h).is_some());
    let transport_calls = |h: &Harness| {
        h.ctl
            .bridge
            .calls
            .iter()
            .filter(|c| {
                matches!(
                    c,
                    Call::Transport(TransportControl::Play | TransportControl::SetRecording { .. })
                )
            })
            .count()
    };
    let before = transport_calls(&h);

    refused(&mut h, play());
    refused(&mut h, Command::Transport(TransportCommand::TogglePlay));
    refused(&mut h, Command::Transport(TransportCommand::Stop));
    refused(
        &mut h,
        Command::Recording(RecordingCommand::SetRecording { enabled: true }),
    );
    // The agent tools too (and so the MCP server and the API).
    let out = h.send(Command::Agent(AgentCommand::CallTool {
        name: "transport".into(),
        input: r#"{"action":"play"}"#.into(),
    }));
    let ReplyValue::AgentToolResult { content, is_error } = ok(&out) else {
        panic!("expected AgentToolResult");
    };
    assert!(is_error && content.contains("still loading"), "{content}");
    assert_eq!(transport_calls(&h), before, "nothing reached the engine");

    // Exploring and editing work: the playhead moves, edits apply at once.
    h.ok(Command::Transport(TransportCommand::Locate {
        position: Beats(8.0),
    }));
    let t = track(&mut h);
    assert!(h.project().tracks.contains_key(&t));
    h.ok(Command::Transport(TransportCommand::SetTempo {
        bpm: 100.0,
    }));

    // Once loaded, the edits are in the graph and the transport works again.
    h.tick();
    h.tick();
    assert_eq!(loading(&h), None);
    assert!(h.ctl.bridge.last_graph().tracks.iter().any(|x| x.id == t));
    h.ok(play());
    h.ok(Command::Transport(TransportCommand::Stop));
}

#[test]
fn the_load_waits_for_the_media() {
    let mut h = harness();
    let id = big_project(&mut h, true);
    h.ctl.host.step_per_read = 0;
    h.ok(Command::Project(ProjectCommand::Open { id }));
    // All plugins fit in the budget now; the media is still decoding.
    let l = loading(&h).expect("loading");
    assert_eq!(l.media_total, 1);
    let mut ticks = 0;
    while loading(&h).is_some() {
        let l = loading(&h).unwrap();
        assert_eq!((l.plugins_loaded, l.plugins_total), (5, 5));
        refused(&mut h, play());
        h.tick();
        ticks += 1;
        assert!(ticks < 1_000, "the load ends");
    }
    assert!(ticks > 1, "the media takes several ticks");
    assert!(!h.ctl.media_pending());
    assert_eq!(suspends(&h), vec![true, false]);
    h.ok(play());
}

#[test]
fn safe_mode_loads_its_plugins_suspended_too() {
    let mut h = harness();
    let id = big_project(&mut h, false);
    h.ok(Command::Project(ProjectCommand::OpenSafe { id }));
    // Nothing to load: the placeholders aren't instantiated.
    assert_eq!(loading(&h), None);
    assert_eq!(suspends(&h), vec![true, false]);
    h.ok(play());

    // Loading them stops the playback and suspends until they are in.
    h.ok(Command::Project(ProjectCommand::LoadPlugins));
    assert!(!h.ctl.transport.playing);
    assert_eq!(loading(&h).map(|l| l.plugins_loaded), Some(2));
    refused(&mut h, play());
    h.tick();
    h.tick();
    assert_eq!(loading(&h), None);
    assert_eq!(suspends(&h), vec![true, false, true, false]);
    h.ok(play());
}

#[test]
fn a_small_project_is_ready_at_once() {
    let mut h = harness();
    let t = track(&mut h);
    let _ = t;
    h.ok(Command::Project(ProjectCommand::Save { autosave: false }));
    let id = h.project().id;
    let out = h.send(Command::Project(ProjectCommand::Open { id }));
    ok(&out);
    assert_eq!(loading(&h), None);
    assert_eq!(suspends(&h).last(), Some(&false));
    h.ok(play());
}

fn clock(h: &mut Harness, sample_time: u64) {
    h.ctl.bridge.playhead = Some(ether_core::PlayheadState {
        sample_time,
        bpm: 120.0,
        ..Default::default()
    });
}

#[test]
fn after_the_resume_the_engine_primes_before_the_transport_is_available() {
    let mut h = harness();
    let id = big_project(&mut h, false);
    h.ctl.host.step_per_read = 0;
    clock(&mut h, 10_000);
    h.tick();
    h.ok(Command::Project(ProjectCommand::Open { id }));
    h.tick();
    // Everything is in and the engine resumed, but its plugins' first blocks run first.
    assert_eq!(suspends(&h), vec![true, false]);
    let l = loading(&h).expect("still priming");
    assert_eq!((l.plugins_loaded, l.plugins_total), (5, 5));
    refused(&mut h, play());
    // 100 ms of audio: not yet.
    clock(&mut h, 10_000 + 4_800);
    h.tick();
    assert!(loading(&h).is_some());
    // PRIME_MS (150 ms) of audio rendered since: ready.
    clock(&mut h, 10_000 + 7_200);
    h.tick();
    assert_eq!(loading(&h), None);
    h.ok(play());
}

#[test]
fn priming_ends_anyway_when_the_engine_clock_stands_still() {
    let mut h = harness();
    let id = big_project(&mut h, false);
    h.ctl.host.step_per_read = 0;
    clock(&mut h, 10_000);
    h.tick();
    h.ok(Command::Project(ProjectCommand::Open { id }));
    h.tick();
    assert!(loading(&h).is_some());
    h.advance(1_000);
    h.tick();
    assert!(loading(&h).is_some());
    h.advance(1_000);
    h.tick();
    assert_eq!(
        loading(&h),
        None,
        "no device: the load ends after the timeout"
    );
}

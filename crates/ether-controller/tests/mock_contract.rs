//! Mock/controller contract (#596): the UI's tests run on `MockTransport`
//! (ui/src/transport/mock), so it must behave as the controller does. Each scenario here has
//! a twin in `ui/src/transport/mock/contract/contract.test.ts`; the transport one is shared
//! data (`transport.json`).

mod common;

use common::*;
use ether_core::protocol::automation::AutomationCommand;
use ether_core::protocol::clips::ClipCommand;
use ether_core::protocol::model::*;
use ether_core::protocol::project::ProjectCommand;
use ether_core::protocol::project::ProjectEvent;
use ether_core::protocol::recording::RecordingCommand;
use ether_core::protocol::tracks::TrackCommand;
use ether_core::protocol::transport::{TransportCommand, TransportState};
use ether_core::protocol::undo_history::HistoryCommand;
use ether_core::protocol::*;
use serde_json::Value;

struct Step {
    command: Command,
    playing: bool,
    start_position: f64,
}

/// The shared steps (`transport.json`): a command and the transport state it leads to.
fn steps() -> Vec<Step> {
    let json: Value = serde_json::from_str(include_str!(
        "../../../ui/src/transport/mock/contract/transport.json"
    ))
    .unwrap();
    json["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| Step {
            command: serde_json::from_value(s["command"].clone()).unwrap(),
            playing: s["playing"].as_bool().unwrap(),
            start_position: s["start_position"].as_f64().unwrap(),
        })
        .collect()
}

fn transports(out: &[ServerMessage]) -> Vec<TransportState> {
    events(out)
        .into_iter()
        .filter_map(|e| match e {
            Event::Transport { state } => Some(state),
            _ => None,
        })
        .collect()
}

fn midi_track(h: &mut Harness) -> TrackId {
    let id: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id,
        kind: TrackKind::Midi,
        name: None,
        color: None,
        parent: None,
        before: None,
    }));
    id
}

#[test]
fn play_resumes_and_locate_moves_the_start_only_while_stopped() {
    let mut h = Harness::with_project();
    let mut state: Option<TransportState> = None;
    for (i, step) in steps().into_iter().enumerate() {
        let mut out = h.send(step.command.clone());
        ok(&out);
        out.extend(h.tick());
        if let Some(s) = transports(&out).pop() {
            state = Some(s);
        }
        let s = state.clone().expect("a transport state");
        assert_eq!(
            (i, s.playing, s.start_position.0),
            (i, step.playing, step.start_position),
            "after {:?}",
            step.command
        );
    }
}

#[test]
fn save_as_renames_the_open_document_in_place() {
    let mut h = Harness::with_project();
    let track = midi_track(&mut h);
    h.ok(Command::Recording(RecordingCommand::Arm {
        track,
        armed: true,
        exclusive: false,
    }));
    h.ok(Command::Transport(TransportCommand::Locate {
        position: Beats(8.0),
    }));
    h.ok(Command::Transport(TransportCommand::Play));
    h.ok(Command::Track(TrackCommand::Rename {
        id: track,
        name: "Lead".into(),
    }));
    h.tick();
    let id = h.project_id();
    let mut out = h.send(Command::Project(ProjectCommand::SaveAs {
        new_id: id,
        name: "Copy".into(),
    }));
    ok(&out);
    out.extend(h.tick());
    let evs = events(&out);
    assert!(evs.iter().any(
        |e| matches!(e, Event::ProjectLoaded { project } if project.id == id && project.settings.name == "Copy")
    ));
    assert!(evs.iter().any(|e| matches!(
        e,
        Event::Project {
            event: ProjectEvent::Saved { project }
        } if project.id == id
    )));
    // Nothing stopped, rewound or disarmed; the copy has the unsaved rename.
    assert!(!evs.iter().any(|e| matches!(e, Event::Recording { .. })));
    assert!(transports(&out).iter().all(|t| t.playing));
    assert_eq!(h.project().tracks[&track].name, "Lead");
    match h.ok(Command::History(HistoryCommand::State)) {
        ReplyValue::HistoryState { history } => assert!(!history.can_undo),
        r => panic!("{r:?}"),
    }
}

#[test]
fn duplicating_the_open_project_copies_its_current_state() {
    let mut h = Harness::with_project();
    let p = h.project().id;
    h.ok(Command::Project(ProjectCommand::Save { autosave: false }));
    let track: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Create {
        id: track,
        kind: TrackKind::Midi,
        name: Some("Unsaved".into()),
        color: None,
        parent: None,
        before: None,
    }));
    let copy = h.project_id();
    h.ok(Command::Project(ProjectCommand::Duplicate {
        id: p,
        new_id: copy,
        name: "Dup".into(),
    }));
    h.ok(Command::Project(ProjectCommand::Open { id: copy }));
    assert!(h.project().tracks.values().any(|t| t.name == "Unsaved"));
}

#[test]
fn a_duplicated_tracks_clip_envelopes_target_the_copy() {
    let mut h = Harness::with_project();
    let track = midi_track(&mut h);
    let clip: ClipId = h.id();
    h.ok(Command::Clip(ClipCommand::CreateMidi {
        id: clip,
        track,
        start: Beats(0.0),
        length: Beats(4.0),
        name: Some("C".into()),
    }));
    let lane: AutomationLaneId = h.id();
    h.ok(Command::Automation(AutomationCommand::CreateLane {
        id: lane,
        owner: AutomationOwner::Clip { clip },
        target: AutomationTarget::TrackVolume { track },
    }));
    let copy: TrackId = h.id();
    h.ok(Command::Track(TrackCommand::Duplicate {
        id: track,
        new_id: copy,
    }));
    let p = h.project();
    let copied: Vec<ClipId> = p
        .clips
        .values()
        .filter(|c| c.track == copy)
        .map(|c| c.id)
        .collect();
    let targets: Vec<AutomationTarget> = p
        .automation_lanes
        .values()
        .filter(|l| matches!(l.owner, AutomationOwner::Clip { clip } if copied.contains(&clip)))
        .map(|l| l.target)
        .collect();
    assert_eq!(targets, vec![AutomationTarget::TrackVolume { track: copy }]);
}

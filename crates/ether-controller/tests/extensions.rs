//! Controller extensions (`ether_controller::ext`, docs/EXTENSIONS.md): an extension answers
//! its own domain and the app commands it takes over, and runs with the tick; without one
//! those reply `Unsupported`.

mod common;

use common::*;
use ether_controller::ext::{ControllerExt, api};
use ether_controller::memory::{MemoryLibrary, MemoryStore};
use ether_controller::{Controller, MessageSink};
use ether_core::protocol::ext::ExtCommand;
use ether_core::protocol::model::ProjectId;
use ether_core::protocol::project::{BundleSource, ProjectCommand};
use ether_core::protocol::{Command, ErrorCode, ReplyValue};

/// Answers the `Jam` domain and `Project::Import`, counts ticks.
#[derive(Default)]
struct Jam {
    ticks: std::rc::Rc<std::cell::Cell<u32>>,
}

impl ControllerExt<FakeBridge, FakeHost, MemoryStore, MemoryLibrary> for Jam {
    fn command(
        &mut self,
        ctl: &mut Ctl,
        command: &Command,
        _now: u64,
        _out: &mut dyn MessageSink,
    ) -> Option<api::CmdResult<ReplyValue>> {
        match command {
            Command::Ext(c) if c.domain == "Jam" => Some(Ok(ReplyValue::Unit)),
            Command::Project(ProjectCommand::Import { .. }) => Some(if ctl.project().is_some() {
                Ok(ReplyValue::Unit)
            } else {
                Err(api::invalid("no project"))
            }),
            _ => None,
        }
    }

    fn tick(&mut self, _ctl: &mut Ctl, _now: u64, _out: &mut dyn MessageSink) {
        self.ticks.set(self.ticks.get() + 1);
    }
}

fn jam() -> Command {
    Command::Ext(ExtCommand {
        domain: "Jam".into(),
        command: serde_json::json!({ "type": "Start" }),
    })
}

fn import(id: ProjectId) -> Command {
    Command::Project(ProjectCommand::Import {
        new_id: id,
        source: BundleSource::Upload {
            upload: "u1".into(),
        },
        name: None,
    })
}

fn code(out: &[ether_core::protocol::ServerMessage]) -> ErrorCode {
    err(out).code
}

#[test]
fn without_an_extension_its_commands_are_unsupported() {
    let mut h = Harness::with_project();
    assert_eq!(code(&h.send(jam())), ErrorCode::Unsupported);
    let id = h.project_id();
    assert_eq!(code(&h.send(import(id))), ErrorCode::Unsupported);
}

#[test]
fn an_extension_answers_its_domain_and_the_commands_it_takes_over() {
    let mut h = Harness::with_project();
    let ext = Jam::default();
    let ticks = ext.ticks.clone();
    h.ctl.add_extension(ext);
    assert_eq!(ok(&h.send(jam())), ReplyValue::Unit);
    let id = h.project_id();
    assert_eq!(ok(&h.send(import(id))), ReplyValue::Unit);
    // The app's own commands still reach the app.
    assert!(matches!(
        ok(&h.send(Command::Project(ProjectCommand::List))),
        ReplyValue::Projects { .. }
    ));
    let before = ticks.get();
    h.tick();
    assert_eq!(ticks.get(), before + 1);
}

/// Watches commits and refuses edits / saves when told to.
#[derive(Default)]
struct Guard {
    refuse_edits: bool,
    refuse_saves: bool,
    seen: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
}

impl ControllerExt<FakeBridge, FakeHost, MemoryStore, MemoryLibrary> for Guard {
    fn before_command(&mut self, _ctl: &mut Ctl, command: &Command) {
        if let Command::Track(_) = command {
            self.seen.borrow_mut().push("track command".into());
        }
    }

    fn refuse_edit(
        &self,
        _ctl: &Ctl,
        _ops: &[ether_core::protocol::model::Op],
    ) -> Option<ether_core::protocol::CommandError> {
        self.refuse_edits
            .then(|| api::invalid_state("still arriving"))
    }

    fn save_refused(&self, _ctl: &Ctl) -> Option<ether_core::protocol::CommandError> {
        self.refuse_saves.then(|| api::invalid_state("not now"))
    }

    fn committed(
        &mut self,
        _ctl: &mut Ctl,
        label: &ether_controller::ext::UndoLabel,
        ops: &[ether_core::protocol::model::Op],
        _inverse: &[ether_core::protocol::model::Op],
    ) {
        self.seen
            .borrow_mut()
            .push(format!("{} ({} ops)", label.text, ops.len()));
    }
}

fn create_track(h: &mut Harness) -> Vec<ether_core::protocol::ServerMessage> {
    let id = h.id();
    h.send(Command::Track(
        ether_core::protocol::tracks::TrackCommand::Create {
            id,
            kind: ether_core::protocol::model::TrackKind::Midi,
            name: None,
            color: None,
            parent: None,
            before: None,
        },
    ))
}

#[test]
fn extensions_see_commits_and_may_refuse_edits_and_saves() {
    let mut h = Harness::with_project();
    let guard = Guard::default();
    let seen = guard.seen.clone();
    h.ctl.add_extension(guard);
    ok(&create_track(&mut h));
    assert_eq!(seen.borrow()[0], "track command");
    assert!(seen.borrow()[1].contains("ops)"), "{:?}", seen.borrow());

    let mut h = Harness::with_project();
    h.ctl.add_extension(Guard {
        refuse_edits: true,
        refuse_saves: true,
        ..Default::default()
    });
    assert_eq!(code(&create_track(&mut h)), ErrorCode::InvalidState);
    let out = h.send(Command::Project(ProjectCommand::Save { autosave: false }));
    assert_eq!(code(&out), ErrorCode::InvalidState);
}

/// Writes a sidecar with every save, and records copies.
#[derive(Default)]
struct Journal {
    log: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
}

impl ControllerExt<FakeBridge, FakeHost, MemoryStore, MemoryLibrary> for Journal {
    fn save_sidecar(
        &mut self,
        _ctl: &mut Ctl,
        _project: ProjectId,
    ) -> Option<ether_controller::store::RenderSidecar> {
        Some(Box::new(|json: &str| {
            Some((
                "journal.json".into(),
                format!("{}", json.len()).into_bytes(),
            ))
        }))
    }

    fn saved(&mut self, _ctl: &mut Ctl, _project: ProjectId, with_sidecar: bool) {
        self.log.borrow_mut().push(format!("saved {with_sidecar}"));
    }

    fn project_copied(&mut self, _ctl: &mut Ctl, _copy: ProjectId) {
        self.log.borrow_mut().push("copied".into());
    }
}

#[test]
fn extensions_add_files_to_saves_and_see_copies() {
    use ether_controller::store::ProjectStore;
    let mut h = Harness::with_project();
    let id = h.ctl.project().unwrap().id;
    let ext = Journal::default();
    let log = ext.log.clone();
    h.ctl.add_extension(ext);
    ok(&h.send(Command::Project(ProjectCommand::Save { autosave: false })));
    assert!(h.ctl.store.read(id, "journal.json").is_ok());
    assert_eq!(log.borrow().last().unwrap(), "saved true");
    let copy = h.project_id();
    ok(&h.send(Command::Project(ProjectCommand::Duplicate {
        id,
        new_id: copy,
        name: "Copy".into(),
    })));
    assert!(log.borrow().iter().any(|l| l == "copied"));
}

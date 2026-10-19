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
